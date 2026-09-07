//! Reading the sources a start's spec names over `https://`, off the frame.
//!
//! # Why this exists at all
//!
//! DuckDB binds a view over an `https://` Parquet **eagerly** — the plan is
//! made, the footer is read, and for a table of any size that is a fetch. The
//! bind happens inside [`crate::pipeline::LiveDashboard::load_str`], which the
//! front door used to call straight out of the click handler, so the whole
//! download sat inside one frame: the window stopped answering the operating
//! system and the click that most wanted to look responsive was the one that
//! produced the spinning cursor.
//!
//! What moves here is the **bytes**, not the engine. [`crate::pipeline`]'s
//! `Session` holds a `duckdb::Connection` beside a cache of prepared
//! statements borrowed from it, so it is self-referential and cannot be sent
//! to a worker; moving it would be a change to the engine's shape rather than
//! to this one start. Instead the worker writes the source to a local file,
//! and the engine — still on the UI thread, still doing exactly what it did
//! before — binds a `file:` source it can read at local speed.
//!
//! # What the reader is told while it happens
//!
//! [`Fetch::readout`] is the card's foot while a fetch is outstanding: the
//! bytes received against the length the server declared, and the bytes
//! received **alone** where the server declared no length. A denominator this
//! module does not have is not invented — a bar that fills at an unmeasured
//! rate is worse than a number that counts up.
//! `the_card_reads_the_count_alone_when_no_length_was_declared` in
//! `crates/brightfield-shell/tests/remote_start.rs` holds the second case
//! against a server that sends no `Content-Length`.
//!
//! `Content-Length` is read only where the response arrives unencoded. Under
//! `Content-Encoding: gzip` the header measures the compressed body while the
//! reader below yields the decompressed one, so the two are counts of
//! different things and pairing them would draw a readout that overshoots its
//! own total.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use brightfield_spec::ast::{DataSourceKind, SpecValue};
use brightfield_spec::{parse_spec, Format, ParseOutput};

/// The scheme this module fetches. A source naming any other scheme is left
/// alone for the engine to resolve as it always has.
pub const HTTPS: &str = "https://";

/// How much of the body is moved between the socket and the file at a time.
///
/// The meter is bumped once per chunk, so this is also the granularity of the
/// readout: at 64 KiB a slow connection still ticks several times a second and
/// a fast one does not spend the frame budget on `request_repaint`.
const CHUNK: usize = 64 * 1024;

/// How long a connection may take to establish before the fetch gives up.
///
/// A read timeout is deliberately **not** set beside it: a large Parquet over
/// a slow link is a long read and cutting it off at any fixed duration would
/// refuse the case this module exists to serve. A host that accepts and then
/// says nothing is the residual, and the window stays up under it — which is
/// the whole difference from the frame-blocking bind this replaced.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// How far one fetch has got: what has arrived, and what was promised.
///
/// Written by the worker and read by the frame, so the fields are atomics read
/// and written at [`Ordering::Relaxed`]. Relaxed is right rather than merely
/// cheap: what these numbers order is a string on a card, and the *completion*
/// of the fetch travels on the channel in [`Fetch`], which carries its own
/// ordering.
#[derive(Debug, Default)]
pub struct Meter {
    received: AtomicU64,
    declared: AtomicU64,
    length_declared: AtomicBool,
}

impl Meter {
    /// How many bytes have been written to disk so far.
    #[must_use]
    pub fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }

    /// How many bytes the server said there would be, where it said so.
    ///
    /// Two fields rather than a sentinel value in one, because every `u64` is
    /// a length a server can legitimately declare and a reader should not have
    /// to know which one this module reserved.
    #[must_use]
    pub fn declared(&self) -> Option<u64> {
        self.length_declared
            .load(Ordering::Relaxed)
            .then(|| self.declared.load(Ordering::Relaxed))
    }

    fn add_received(&self, n: u64) {
        self.received.fetch_add(n, Ordering::Relaxed);
    }

    fn declare(&self, total: u64) {
        self.declared.store(total, Ordering::Relaxed);
        self.length_declared.store(true, Ordering::Relaxed);
    }
}

/// The files one fetch wrote, and the directory holding them.
///
/// **Dropping this deletes the directory.** The engine binds a view over these
/// paths rather than copying them into DuckDB, so a query re-reads the file:
/// the value has to outlive the session that reads it, which is why the window
/// holds it beside the document rather than letting it fall at the end of the
/// open.
#[derive(Debug)]
pub struct Fetched {
    dir: PathBuf,
    local: Vec<(String, PathBuf)>,
}

impl Fetched {
    /// The local path standing in for `url`, if this fetch wrote one.
    #[must_use]
    pub fn local_path(&self, url: &str) -> Option<&Path> {
        self.local
            .iter()
            .find(|(from, _)| from == url)
            .map(|(_, to)| to.as_path())
    }

    /// Every source this fetch resolved, as `(url, local path)`.
    #[must_use]
    pub fn pairs(&self) -> &[(String, PathBuf)] {
        &self.local
    }

    /// The directory the files were written into.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Fetched {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// A fetch running on a worker thread.
///
/// The window holds one of these while a remote start is opening. It polls
/// [`Fetch::take`] once a frame and draws [`Fetch::readout`] on the card in
/// between; the worker wakes the event loop through the callback handed to
/// [`Fetch::begin`], which is what keeps the readout moving on a window nobody
/// is typing into.
pub struct Fetch {
    urls: Vec<String>,
    meter: Arc<Meter>,
    outcome: Receiver<Result<Fetched, String>>,
    done: bool,
}

impl Fetch {
    /// Start fetching `urls` into a fresh directory under the system temporary
    /// directory, on a thread of its own.
    ///
    /// `wake` is called after every chunk and once at the end. The live window
    /// passes `egui::Context::request_repaint`; a headless caller driving its
    /// own frames passes a no-op. Taking a callback rather than a context is
    /// what keeps this module free of the UI framework, which is what lets the
    /// tests below drive it with no window at all.
    #[must_use]
    pub fn begin(urls: Vec<String>, wake: impl Fn() + Send + 'static) -> Self {
        let meter = Arc::new(Meter::default());
        let (tx, outcome) = mpsc::channel();
        let worker_meter = Arc::clone(&meter);
        let worker_urls = urls.clone();
        std::thread::spawn(move || {
            let result = fetch_all(&worker_urls, &worker_meter, &wake);
            // A failed send means the window has already dropped this
            // `Fetch` — someone went Home, or opened something else, while the
            // bytes were still moving. There is no one to report to and no
            // clean-up to do: `Fetched`'s own `Drop` removes the directory when
            // the undelivered message is dropped with the channel.
            tx.send(result).ok();
            wake();
        });
        Self {
            urls,
            meter,
            outcome,
            done: false,
        }
    }

    /// The URLs this fetch is moving.
    #[must_use]
    pub fn urls(&self) -> &[String] {
        &self.urls
    }

    /// What has arrived so far.
    #[must_use]
    pub fn meter(&self) -> &Meter {
        &self.meter
    }

    /// The fetch's result, once there is one — `None` while it is still
    /// outstanding.
    ///
    /// A disconnected channel with nothing on it means the worker panicked,
    /// which is reported as a failure rather than left as a fetch that never
    /// finishes: a card stuck on a readout that has stopped moving is the
    /// worst of the three outcomes.
    pub fn take(&mut self) -> Option<Result<Fetched, String>> {
        if self.done {
            return None;
        }
        match self.outcome.try_recv() {
            Ok(result) => {
                self.done = true;
                Some(result)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.done = true;
                Some(Err(format!(
                    "reading {} over the network stopped without an answer",
                    self.urls.join(", ")
                )))
            }
        }
    }

    /// Block until this fetch answers.
    ///
    /// The entry [`crate::starts::load`] takes, which is the blocking one: a
    /// caller with no frames to draw — a network-gated test, the thumbnail
    /// regeneration, a launch restoring a recorded start — has nothing to do
    /// with a readout and everything to gain from one line instead of a poll
    /// loop. The window takes [`Fetch::take`] instead.
    ///
    /// # Errors
    ///
    /// The fetch's own failure, or a message naming the URLs if the worker
    /// went away without sending one.
    pub fn wait(self) -> Result<Fetched, String> {
        self.outcome.recv().unwrap_or_else(|_| {
            Err(format!(
                "reading {} over the network stopped without an answer",
                self.urls.join(", ")
            ))
        })
    }

    /// What the card says while this is outstanding: the bytes received
    /// against the declared length, or the bytes received alone where the
    /// server declared none.
    #[must_use]
    pub fn readout(&self) -> String {
        readout(self.meter.received(), self.meter.declared())
    }
}

/// [`Fetch::readout`]'s words, over numbers rather than over a live fetch —
/// which is what lets both halves of the readout be pinned without a server.
#[must_use]
pub fn readout(received: u64, declared: Option<u64>) -> String {
    match declared {
        Some(total) => format!(
            "fetching \u{b7} {} of {}",
            crate::protocol::human_bytes(received),
            crate::protocol::human_bytes(total)
        ),
        None => format!("fetching \u{b7} {}", crate::protocol::human_bytes(received)),
    }
}

/// Every `https://` source `spec` declares, in declaration order.
///
/// Both shapes a spec can name a file in are read: the `file:` key that parses
/// to [`DataSourceKind::File`], and the `file:` that rides in `extras` under a
/// `type:`-discriminated source. `brightfield_sql::source::emit_file_typed`
/// resolves the second the way it resolves the first, so a fetch that read the
/// first and stopped would leave a typed remote source to be bound eagerly on
/// the UI thread — the freeze this module exists to end, one source over.
///
/// # Errors
///
/// If `spec` does not parse.
pub fn https_sources(spec: &str) -> Result<Vec<String>, String> {
    let parsed = parse_spec(spec, Format::Yaml).map_err(|e| format!("parse error: {e}"))?;
    let mut out = Vec::new();
    for source in parsed.spec.data.values() {
        let named = match &source.kind {
            DataSourceKind::File(path) => Some(path.clone()),
            _ => match source.extras.get("file") {
                Some(SpecValue::String(path)) => Some(path.clone()),
                _ => None,
            },
        };
        if let Some(path) = named {
            if path.starts_with(HTTPS) && !out.contains(&path) {
                out.push(path);
            }
        }
    }
    Ok(out)
}

/// `spec` with every source in `fetched` repointed at the local file that
/// stands in for it.
///
/// Rewritten through the parsed spec rather than by substituting text: the
/// thing that has to change is the value of one `file:` key, and a string
/// replacement over the document would also hit the same URL written in a
/// comment — which `examples/remote/edgar-gleif-crosswalk.yaml` does, twice.
///
/// # Errors
///
/// If `spec` does not parse.
pub fn repointed(spec: &str, fetched: &Fetched) -> Result<ParseOutput, String> {
    let mut parsed = parse_spec(spec, Format::Yaml).map_err(|e| format!("parse error: {e}"))?;
    for source in parsed.spec.data.values_mut() {
        match &mut source.kind {
            DataSourceKind::File(path) => {
                if let Some(local) = fetched.local_path(path) {
                    *path = local.display().to_string();
                }
            }
            _ => {
                if let Some(SpecValue::String(path)) = source.extras.get("file") {
                    if let Some(local) = fetched.local_path(path) {
                        let local = SpecValue::String(local.display().to_string());
                        source.extras.insert("file".to_string(), local);
                    }
                }
            }
        }
    }
    Ok(parsed)
}

/// Fetch the URLs into a fresh directory, bumping `meter` as the bytes land.
fn fetch_all(
    urls: &[String],
    meter: &Meter,
    wake: &(impl Fn() + Send + 'static),
) -> Result<Fetched, String> {
    let dir = std::env::temp_dir().join(format!(
        "brightfield-remote-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not make a place to fetch into: {e}"))?;
    // Built before the loop so a `Fetched` exists from the first failure
    // onwards: returning `Err` out of the middle of a multi-source fetch would
    // otherwise leave the files written so far on disk with no owner to remove
    // them.
    let mut fetched = Fetched {
        dir: dir.clone(),
        local: Vec::new(),
    };
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .build();
    for (i, url) in urls.iter().enumerate() {
        let target = dir.join(format!("{i}-{}", file_name_of(url)));
        fetch_one(&agent, url, &target, meter, wake)?;
        fetched.local.push((url.clone(), target));
    }
    Ok(fetched)
}

/// One URL to one file.
///
/// A failure here names the network and the URL, because that is what the
/// eager bind this replaced said and it is what the window raises as a banner:
/// `brightfield_engine::EngineError::RemoteSourceFailed` renders to a sentence
/// carrying both, and a fetch that failed with a bare io error would have moved
/// the work off the frame and taken the diagnosis with it.
fn fetch_one(
    agent: &ureq::Agent,
    url: &str,
    target: &Path,
    meter: &Meter,
    wake: &(impl Fn() + Send + 'static),
) -> Result<(), String> {
    let response = agent
        .get(url)
        .call()
        .map_err(|e| format!("could not read {url} over the network: {e}"))?;
    // Only where the body arrives as it was measured — see the module header.
    let encoded = response
        .header("Content-Encoding")
        .is_some_and(|e| !e.trim().eq_ignore_ascii_case("identity"));
    if !encoded {
        if let Some(total) = response
            .header("Content-Length")
            .and_then(|v| v.trim().parse::<u64>().ok())
        {
            meter.declare(total);
        }
    }
    let mut body = response.into_reader();
    let mut file = std::fs::File::create(target)
        .map_err(|e| format!("could not read {url} over the network: {e}"))?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = body
            .read(&mut buf)
            .map_err(|e| format!("could not read {url} over the network: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("could not read {url} over the network: {e}"))?;
        meter.add_received(n as u64);
        wake();
    }
    file.flush()
        .map_err(|e| format!("could not read {url} over the network: {e}"))?;
    Ok(())
}

/// The file name to write `url` under.
///
/// The URL's last path segment, kept for its **extension**: DuckDB's DDL is
/// chosen by it — `brightfield_sql::source::emit_file` dispatches `parquet` /
/// `csv` / `json` off the value's tail — so a fetch that wrote to a name
/// without one would turn a readable Parquet into `UnknownFormat`. Anything
/// outside a conservative set is replaced, because the segment comes off a URL
/// and the result is a path this process creates.
fn file_name_of(url: &str) -> String {
    let tail = url
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or("source");
    // Query strings and fragments are not part of the name.
    let tail = tail.split(['?', '#']).next().unwrap_or("source");
    let cleaned: String = tail
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        "source".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The readout says both numbers when the server declared a length, and
    /// only the one it has when it did not.
    #[test]
    fn the_readout_names_a_denominator_only_when_there_is_one() {
        assert_eq!(
            readout(1024 * 1024, Some(4 * 1024 * 1024)),
            "fetching \u{b7} 1.0 MB of 4.0 MB"
        );
        assert_eq!(readout(1024 * 1024, None), "fetching \u{b7} 1.0 MB");
    }

    /// The name a URL is written under keeps the extension the engine's DDL
    /// dispatch reads, and nothing else survives that a path should not hold.
    #[test]
    fn the_local_name_keeps_the_extension_and_nothing_dangerous() {
        assert_eq!(
            file_name_of("https://example.invalid/edgar_gleif.parquet"),
            "edgar_gleif.parquet"
        );
        assert_eq!(
            file_name_of("https://example.invalid/a/b/table.csv?v=2#top"),
            "table.csv"
        );
        assert_eq!(file_name_of("https://example.invalid/"), "example.invalid");
        assert_eq!(file_name_of("https://example.invalid/.."), "source");
    }
}
