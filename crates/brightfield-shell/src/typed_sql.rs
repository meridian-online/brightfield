//! Typed SQL in the chart palette: text on the query line that reads as a
//! statement is previewed and, when kept, becomes a step. This module holds the
//! parts of that which need no window.
//!
//! [`TypedReads`] holds the text being typed, the read of it under way, and
//! the last answer whose rows read. A read loads the whole data file and runs
//! the statement, and arc is asked about it in a process of its own, so it runs
//! on a worker and a frame only starts, stops and collects reads.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use brightfield_engine::{
    preview_first_page_stoppable, preview_row_count_stoppable, statement_reading, FileReader,
    PreviewOp, PreviewRefusal, PreviewStop, StatementReading, PREVIEW_DEADLINE,
    PREVIEW_FILE_LIMIT_BYTES,
};

use crate::data_grid::{page_of, GridPage, PreviewHooks, PREVIEW_ROWS};
use crate::generated::sql_step_name_for;
use crate::operation::{recognise, Recognised};
use crate::run::Runner;

/// The name for a step made from the statement `reading` describes: the first
/// table it reads and the column it turns on, made free of `taken`.
#[must_use]
pub fn typed_step_name(reading: &StatementReading, taken: &BTreeSet<String>) -> String {
    sql_step_name_for(
        reading.tables.first().map(String::as_str),
        reading.name_column.as_deref(),
        taken,
    )
}

/// How long the text must rest after an edit before it is read.
pub const TYPED_PAUSE: Duration = Duration::from_millis(300);

/// What a typed statement is read against: the data file, the table it loads
/// as, and every step the statement may read, in order.
#[derive(Clone, Debug)]
pub struct TypedSource {
    /// The data file the load reads.
    pub data: PathBuf,
    /// The reader the data file loads with.
    pub reader: FileReader,
    /// The table the load makes.
    pub root_table: String,
    /// Each step the statement may read, in order, with what it does.
    pub chain: Vec<(String, PreviewOp)>,
    /// The program arc runs as, when there is one to ask.
    pub runner: Option<Runner>,
}

/// What a read of a typed statement found.
#[derive(Clone, Debug)]
pub struct TypedAnswer {
    /// The text that was read.
    pub text: String,
    /// The statement's first page of rows.
    pub page: GridPage,
    /// The tables it reads and the column it turns on.
    pub reading: StatementReading,
    /// What arc makes of it: [`Recognised::Other`] when there is no runner or
    /// arc could not say.
    pub recognised: Recognised,
    /// The rows of the root table the condition keeps, counted on their own,
    /// when arc reads the statement as a filter of the root table.
    pub filter_rows: Option<u64>,
}

/// What a worker sends back: the answer, or the text and why it was refused.
type TypedResult = Result<TypedAnswer, (String, String)>;

/// A wake shared by the reader and each worker it starts.
type Wake = Arc<dyn Fn() + Send + Sync>;

/// **The reads of the text typed on the query line.** One read is under way
/// at a time; a read started while another is under way stops the older one
/// and waits behind the preview gate until it has let go.
pub struct TypedReads {
    source: TypedSource,
    pause: Duration,
    wake: Wake,
    hooks: PreviewHooks,
    text: String,
    edited: Option<Instant>,
    in_flight: Option<(String, PreviewStop, Receiver<TypedResult>)>,
    shown: Option<TypedAnswer>,
    refused: Option<(String, String)>,
    started: usize,
    superseded: usize,
}

impl TypedReads {
    /// A reader of statements against `source` that reads once the text has
    /// rested for `pause`, calling `wake` from the worker when an answer is
    /// ready.
    pub fn new(
        source: TypedSource,
        pause: Duration,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            source,
            pause,
            wake: Arc::new(wake),
            hooks: PreviewHooks::default(),
            text: String::new(),
            edited: None,
            in_flight: None,
            shown: None,
            refused: None,
            started: 0,
            superseded: 0,
        }
    }

    /// This reader with its workers behind `hooks`: the gate they read inside
    /// and the switch that holds them before they read.
    #[must_use]
    #[expect(
        dead_code,
        reason = "the chart document hands its own hooks here once its palette reads statements"
    )]
    pub(crate) fn with_hooks(mut self, hooks: PreviewHooks) -> Self {
        self.hooks = hooks;
        self
    }

    /// Note that the text is now `text`, edited at `now`. Nothing is read.
    pub fn edit(&mut self, text: &str, now: Instant) {
        if text != self.text {
            text.clone_into(&mut self.text);
        }
        self.edited = Some(now);
    }

    /// Start a read of the text once `pause` has passed since the last edit,
    /// when it has no answer and is not being read.
    pub fn tick(&mut self, now: Instant) {
        if self
            .edited
            .is_some_and(|edited| now.saturating_duration_since(edited) >= self.pause)
        {
            self.read_now();
        }
    }

    /// Start a read of the text without waiting out the pause, when it has no
    /// answer and is not being read.
    pub fn read_now(&mut self) {
        let answered = self.shown.as_ref().is_some_and(|a| a.text == self.text)
            || self.refused.as_ref().is_some_and(|(t, _)| *t == self.text);
        let being_read = self
            .in_flight
            .as_ref()
            .is_some_and(|(t, _, _)| *t == self.text);
        if answered || being_read {
            return;
        }
        if let Some((_, stop, _)) = self.in_flight.take() {
            stop.stop();
            self.superseded += 1;
        }
        let stop = PreviewStop::default();
        let (send, answer) = channel();
        let (source, text, hooks, wake) = (
            self.source.clone(),
            self.text.clone(),
            self.hooks.clone(),
            self.wake.clone(),
        );
        let stopped = stop.clone();
        std::thread::spawn(move || {
            if let Some(result) = hooks.read_inside(|| read_typed(&source, &text, &stopped)) {
                let _ = send.send(result);
                wake();
            }
        });
        self.in_flight = Some((self.text.clone(), stop, answer));
        self.started += 1;
    }

    /// Take a finished read off the worker, if one has finished. True when
    /// what is shown or said changed.
    pub fn poll(&mut self) -> bool {
        let Some((text, _, answer)) = &self.in_flight else {
            return false;
        };
        let result = match answer.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err((
                text.clone(),
                "the preview's read stopped without an answer".to_string(),
            )),
        };
        self.in_flight = None;
        match result {
            Ok(answer) => {
                self.refused = None;
                self.shown = Some(answer);
            }
            Err(refused) => self.refused = Some(refused),
        }
        true
    }

    /// The last answer whose rows read, whatever the text is now.
    #[must_use]
    pub fn shown(&self) -> Option<&TypedAnswer> {
        self.shown.as_ref()
    }

    /// [`Self::shown`] when it answers the text as it is now.
    #[must_use]
    pub fn current(&self) -> Option<&TypedAnswer> {
        self.shown.as_ref().filter(|a| a.text == self.text)
    }

    /// DuckDB's reason, when the text as it is now was refused.
    #[must_use]
    pub fn refusal(&self) -> Option<&str> {
        self.refused
            .as_ref()
            .filter(|(text, _)| *text == self.text)
            .map(|(_, reason)| reason.as_str())
    }

    /// Whether a read is under way.
    #[must_use]
    pub const fn reading(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Stop the read under way and drop its answer; the text is not read
    /// again until it is edited or [`Self::read_now`] asks.
    pub fn stop(&mut self) {
        if let Some((_, stop, _)) = self.in_flight.take() {
            stop.stop();
        }
        self.edited = None;
    }

    /// How many reads this reader has started.
    #[must_use]
    pub const fn started(&self) -> usize {
        self.started
    }

    /// How many reads were stopped because the text moved past them.
    #[must_use]
    pub const fn superseded(&self) -> usize {
        self.superseded
    }
}

/// Count the rows of the last step of `source.chain` on a worker, behind the
/// preview gate, calling `wake` once the count is sent.
pub fn spawn_row_count(
    source: TypedSource,
    wake: impl Fn() + Send + 'static,
) -> Receiver<Result<u64, PreviewRefusal>> {
    let (send, answer) = channel();
    let hooks = PreviewHooks::default();
    std::thread::spawn(move || {
        let count = hooks.read_inside(|| {
            preview_row_count_stoppable(
                &source.data,
                source.reader,
                &source.root_table,
                &source.chain,
                PREVIEW_FILE_LIMIT_BYTES,
                PREVIEW_DEADLINE,
                &PreviewStop::default(),
            )
        });
        let _ = send.send(count);
        wake();
    });
    answer
}

/// One read of `text` against `source`: its page, its reading, arc's reading,
/// and the filter's count, checking `stop` between them. `None` when it was
/// stopped.
fn read_typed(source: &TypedSource, text: &str, stop: &PreviewStop) -> Option<TypedResult> {
    let mut chain = source.chain.clone();
    chain.push((
        free_step_name(&source.root_table, &chain),
        PreviewOp::Statement(text.to_string()),
    ));
    let page = match preview_first_page_stoppable(
        &source.data,
        source.reader,
        &source.root_table,
        &chain,
        PREVIEW_ROWS,
        PREVIEW_FILE_LIMIT_BYTES,
        PREVIEW_DEADLINE,
        stop,
    ) {
        Ok(batches) => page_of(&batches),
        Err(PreviewRefusal::Superseded) => return None,
        Err(refusal) => return Some(Err((text.to_string(), refusal.to_string()))),
    };
    if stop.is_stopped() {
        return None;
    }
    let reading = statement_reading(text).unwrap_or_default();
    if stop.is_stopped() {
        return None;
    }
    let recognised = source
        .runner
        .as_ref()
        .and_then(|runner| recognise(runner, text).ok())
        .unwrap_or(Recognised::Other);
    if stop.is_stopped() {
        return None;
    }
    let filter_rows = match &recognised {
        Recognised::FilterRows { on, condition } if on.eq_ignore_ascii_case(&source.root_table) => {
            match preview_row_count_stoppable(
                &source.data,
                source.reader,
                &source.root_table,
                &[(
                    free_step_name(&source.root_table, &[]),
                    PreviewOp::Where(condition.clone()),
                )],
                PREVIEW_FILE_LIMIT_BYTES,
                PREVIEW_DEADLINE,
                stop,
            ) {
                Ok(rows) => Some(rows),
                Err(PreviewRefusal::Superseded) => return None,
                Err(_) => None,
            }
        }
        _ => None,
    };
    Some(Ok(TypedAnswer {
        text: text.to_string(),
        page,
        reading,
        recognised,
        filter_rows,
    }))
}

/// A table name for the typed statement's own step that neither the root
/// table nor a step of `chain` already uses.
fn free_step_name(root_table: &str, chain: &[(String, PreviewOp)]) -> String {
    let taken = |name: &str| {
        name.eq_ignore_ascii_case(root_table)
            || chain
                .iter()
                .any(|(step, _)| step.eq_ignore_ascii_case(name))
    };
    let mut name = "typed_statement".to_string();
    let mut n = 2;
    while taken(&name) {
        name = format!("typed_statement_{n}");
        n += 1;
    }
    name
}
