//! Reader for the live run stream — the `<run_id>.jsonl` file `arc run` writes
//! alongside the contract while a run is in flight.
//!
//! Each line is one JSON object, one of:
//! - a step event `{"step":"tally","state":"success","ts":"…"}`, or
//! - the terminal event `{"event":"run_complete","outcome":"success"}`.
//!
//! The file is append-only and *last-valid-line-wins per step*: folding it
//! collapses the sequence to the latest observed state for each step. Malformed
//! or partially-written lines are skipped, so the fold is safe to run against a
//! file still being appended to.
//!
//! The `.jsonl` is deliberately THINNER than the `.json` contract — a step that
//! was never reached emits no line — so the fold is an *overlay*: the `.json`
//! is the authoritative complete step set, and the stream only refreshes the
//! live state of the steps it mentions ([`crate::contract_graph::apply_stream`]).
//! `run_complete` is the reconcile signal that the run is done.
//!
//! [`StreamReader`] is a polling tailing reader: it remembers how far it has
//! read and, on each [`poll`](StreamReader::poll), consumes only the bytes
//! appended since — buffering a trailing partial line until its newline arrives.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::contract::Outcome;

/// The latest state observed for one step on the live stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepStreamEntry {
    /// The raw `state` string from the last event for this step (e.g.
    /// `"running"`, `"success"`). Kept verbatim because the live vocabulary is
    /// broader than the contract's terminal [`StepState`](crate::contract::StepState).
    pub state: String,
    /// The `ts` from that event, when present (second resolution).
    pub ts: Option<String>,
}

/// The folded state of a run's live stream: the latest entry per step plus the
/// terminal outcome once observed. Keyed by step name in a `BTreeMap` — the
/// fold is applied onto the authoritative `.json` steps by name, so order is
/// irrelevant and determinism is free.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamState {
    /// Latest entry per step, keyed by step name.
    pub steps: BTreeMap<String, StepStreamEntry>,
    /// Set once a `run_complete` event has been folded in.
    pub outcome: Option<Outcome>,
    /// `true` once the terminal `run_complete` event has been seen — the
    /// reconcile signal that the run is complete.
    pub complete: bool,
}

impl StreamState {
    /// An empty stream state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold every complete line of `content` into this state. Malformed and
    /// blank lines are skipped. Returns the number of events applied.
    pub fn fold_str(&mut self, content: &str) -> usize {
        let mut applied = 0;
        for line in content.lines() {
            if self.fold_line(line) {
                applied += 1;
            }
        }
        applied
    }

    /// Fold one line. Returns `true` if it parsed into a recognised event.
    fn fold_line(&mut self, line: &str) -> bool {
        let line = line.trim();
        if line.is_empty() {
            return false;
        }
        let Ok(raw) = serde_json::from_str::<RawLine>(line) else {
            return false;
        };
        if raw.event.as_deref() == Some("run_complete") {
            self.complete = true;
            self.outcome = raw
                .outcome
                .as_deref()
                .and_then(|o| serde_json::from_value(serde_json::Value::String(o.to_string())).ok());
            return true;
        }
        if let (Some(step), Some(state)) = (raw.step, raw.state) {
            self.steps.insert(step, StepStreamEntry { state, ts: raw.ts });
            return true;
        }
        false
    }
}

/// Convenience: fold a whole stream string into a fresh [`StreamState`].
#[must_use]
pub fn fold_stream(content: &str) -> StreamState {
    let mut state = StreamState::new();
    state.fold_str(content);
    state
}

/// Permissive row shape covering both event kinds; fields absent for the other
/// kind deserialize to `None`.
#[derive(Debug, Deserialize)]
struct RawLine {
    #[serde(default)]
    step: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    ts: Option<String>,
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    outcome: Option<String>,
}

/// A polling tailing reader over a run's `.jsonl` stream.
///
/// Construct with [`open`](StreamReader::open), then call
/// [`poll`](StreamReader::poll) on a timer. Each poll folds newly appended,
/// newline-terminated lines into the accumulated [`StreamState`]. A trailing
/// partial line (an append caught mid-write) is buffered until its newline
/// arrives. A not-yet-created file is treated as "no events yet".
#[derive(Debug)]
pub struct StreamReader {
    path: PathBuf,
    offset: u64,
    partial: String,
    state: StreamState,
}

impl StreamReader {
    /// Open a reader for the stream at `path`. Reads nothing yet.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            offset: 0,
            partial: String::new(),
            state: StreamState::new(),
        }
    }

    /// The stream file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The accumulated folded state.
    #[must_use]
    pub fn state(&self) -> &StreamState {
        &self.state
    }

    /// Read bytes appended since the last poll and fold their complete lines
    /// into the accumulated state, returning a reference to it.
    ///
    /// A missing file yields the current state unchanged. If the file has
    /// shrunk since the last poll (a rewrite), the reader resets and re-reads.
    ///
    /// # Errors
    /// Returns any I/O error other than "file not found" encountered while
    /// reading the stream.
    pub fn poll(&mut self) -> io::Result<&StreamState> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(&self.state),
            Err(e) => return Err(e),
        };
        let len = bytes.len() as u64;

        // A shrink means the file was rewritten; start over.
        if len < self.offset {
            self.offset = 0;
            self.partial.clear();
            self.state = StreamState::new();
        }

        let fresh = &bytes[self.offset as usize..];
        self.offset = len;
        // Lossily decode: the stream is UTF-8 JSON, but a byte-split mid-poll
        // must not error — malformed fragments buffer and are re-evaluated.
        self.partial.push_str(&String::from_utf8_lossy(fresh));

        while let Some(nl) = self.partial.find('\n') {
            let line: String = self.partial.drain(..=nl).collect();
            self.state.fold_line(line.trim_end_matches(['\n', '\r']));
        }
        Ok(&self.state)
    }
}
