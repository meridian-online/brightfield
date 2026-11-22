//! The reads of a draft as it is edited: arc's model for it, run at each edit
//! on a preview connection kept open, the latest answer winning.
//!
//! **One worker, one connection.** [`DraftReads::start`] starts a worker that
//! opens a [`WarmPreview`] over the data file and the steps the draft reads,
//! and holds it until the reads are dropped. Opening is the slow part, the
//! file's load, and it happens once, when the draft starts.
//!
//! **An edit is read at once.** [`DraftReads::edit`] sends the request to the
//! worker with no pause. A read has two halves: arc is asked for the draft's
//! model ([`draft_model`], a child process), and then DuckDB runs that model
//! on the warm connection. The worker takes one request at a time, so one arc
//! child runs at a time. When it comes back for the next request it takes the
//! newest one waiting and drops the ones before it, which is why the last edit
//! of a burst is read however fast the burst was
//! (`the_last_edit_of_a_burst_is_always_read` in `tests/operation_reads.rs`).
//! An edit also stops the DuckDB half of the read before it, through that
//! read's [`PreviewStop`].
//!
//! **The answer shown is for the text as it stands.** Each request carries a
//! number, and an answer is taken when its number is the newest sent. An
//! answer for a request since changed is dropped where it arrives
//! (`an_answer_for_text_since_changed_is_never_shown`).
//!
//! **A refused edit keeps the last good rows.** When arc or DuckDB refuses the
//! request as it stands, [`DraftReads::refusal`] holds the reason and
//! [`DraftReads::shown`] still holds the last answer that read, which the grid
//! draws faded ([`DraftReads::faded`]). While a newer request is being read
//! and has not been refused, the last good rows are drawn as they were: a
//! fade at each keystroke would flicker for the tenth of a second arc takes.
//!
//! **A table too large or too slow to preview still answers.** arc's model is
//! in the answer, for the Editor and for keeping the draft, with an empty page
//! and [`DraftAnswer::unread`] saying why there are no rows.

use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;

use brightfield_engine::{
    PreviewOp, PreviewRefusal, PreviewStop, WarmPreview, PREVIEW_FILE_LIMIT_BYTES,
};

use super::draft::{draft_model, DraftModel};
use crate::data_grid::{GridPage, PreviewHooks, PREVIEW_ROWS};
use crate::operation::StepRequest;
use crate::typed_sql::TypedSource;

/// What a read of a draft answered.
#[derive(Clone, Debug)]
pub struct DraftAnswer {
    /// The request that was read.
    pub request: StepRequest,
    /// The step's name the request was read under.
    pub name: String,
    /// arc's model for it, with arc's notes.
    pub model: DraftModel,
    /// The first page of the rows the model makes.
    pub page: GridPage,
    /// How many rows the model makes.
    pub rows: u64,
    /// How many rows the table the draft is on holds.
    pub rows_in: u64,
    /// Each column the model makes with its count of missing values.
    pub missing: Vec<(String, u64)>,
    /// Why the page is empty, when the rows were not read: the table is too
    /// large to preview, or the read ran past its deadline.
    pub unread: Option<String>,
}

/// One request sent to the worker.
struct Ask {
    number: u64,
    name: String,
    request: StepRequest,
    stop: PreviewStop,
}

/// What the worker sends back for the request `number`: the answer, or why it
/// was refused. A read that was stopped sends no answer.
type Told = (u64, Result<Box<DraftAnswer>, String>);

/// **The reads of one draft**, from its start to its being kept or dropped.
/// Dropping it ends the worker and closes the preview connection.
pub struct DraftReads {
    asks: Sender<Ask>,
    told: Receiver<Told>,
    /// The number of the newest request sent; 0 before the first.
    newest: u64,
    /// The newest request sent, with its name and the switch that stops it.
    asked: Option<(String, StepRequest, PreviewStop)>,
    current: Option<DraftAnswer>,
    shown: Option<DraftAnswer>,
    refusal: Option<String>,
    read_count: Arc<std::sync::atomic::AtomicUsize>,
}

impl std::fmt::Debug for DraftReads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DraftReads")
            .field("newest", &self.newest)
            .field("answered", &self.current.is_some())
            .field("refusal", &self.refusal)
            .finish_non_exhaustive()
    }
}

/// The warm connection, once it has been asked for: open, or why it is not.
enum Warm {
    Open(WarmPreview),
    /// The table cannot be previewed, and a read answers with no rows and
    /// this reason.
    Unread(String),
    /// The connection could not be opened, in DuckDB's words.
    Refused(String),
}

impl DraftReads {
    /// Start the reads of a draft against `source`. `upstream_of` names the
    /// tables the draft reads, so the worker makes the steps those come from,
    /// and theirs, once. `wake` is called from the worker when an answer is
    /// ready.
    pub fn start(
        source: TypedSource,
        upstream_of: Vec<String>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self::start_under(source, upstream_of, PREVIEW_FILE_LIMIT_BYTES, wake)
    }

    /// [`Self::start`] under a file-size limit of `file_limit` bytes, so a test
    /// can reach a table too large to preview without a large file.
    #[doc(hidden)]
    pub fn start_under(
        source: TypedSource,
        upstream_of: Vec<String>,
        file_limit: u64,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (asks, asked) = channel::<Ask>();
        let (tell, told) = channel::<Told>();
        let hooks = PreviewHooks::default();
        let read_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = read_count.clone();
        std::thread::spawn(move || {
            let mut warm = hooks.read_inside(|| open(&source, &upstream_of, file_limit));
            while let Ok(mut ask) = asked.recv() {
                // The newest request waiting is the one read; the ones before
                // it are for text since changed.
                loop {
                    match asked.try_recv() {
                        Ok(newer) => ask = newer,
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let answer = hooks.read_inside(|| read(&source, &mut warm, &ask));
                if let Some(answer) = answer {
                    if tell.send((ask.number, answer)).is_err() {
                        return;
                    }
                    wake();
                }
            }
        });
        Self {
            asks,
            told,
            newest: 0,
            asked: None,
            current: None,
            shown: None,
            refusal: None,
            read_count,
        }
    }

    /// Note that the draft is now `request`, to be recorded as `name`, and
    /// read it at once. The same request under the same name as the last is
    /// not read again. A read under way for the request before it is stopped,
    /// and its answer is dropped when it comes.
    pub fn edit(&mut self, name: &str, request: StepRequest) {
        if let Some((last_name, last, stop)) = &self.asked {
            if last_name == name && *last == request {
                return;
            }
            stop.stop();
        }
        self.newest += 1;
        self.current = None;
        self.refusal = None;
        let stop = PreviewStop::default();
        self.asked = Some((name.to_string(), request.clone(), stop.clone()));
        // A worker that has gone leaves the request unanswered, which the
        // window shows as a read still under way.
        let _ = self.asks.send(Ask {
            number: self.newest,
            name: name.to_string(),
            request,
            stop,
        });
    }

    /// Take what the worker has answered. `true` when what [`Self::current`],
    /// [`Self::shown`] or [`Self::refusal`] answers has changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok((number, answer)) = self.told.try_recv() {
            if number != self.newest {
                continue;
            }
            changed = true;
            match answer {
                Ok(answer) => {
                    self.shown = Some((*answer).clone());
                    self.current = Some(*answer);
                    self.refusal = None;
                }
                Err(why) => {
                    self.current = None;
                    self.refusal = Some(why);
                }
            }
        }
        changed
    }

    /// The answer for the request as it stands, once it has come.
    #[must_use]
    pub fn current(&self) -> Option<&DraftAnswer> {
        self.current.as_ref()
    }

    /// The last answer that read, which may be for an earlier request.
    #[must_use]
    pub fn shown(&self) -> Option<&DraftAnswer> {
        self.shown.as_ref()
    }

    /// arc's or DuckDB's words for refusing the request as it stands.
    #[must_use]
    pub fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }

    /// Whether the grid draws [`Self::shown`] faded: the request as it stands
    /// was refused, and the rows are an earlier request's.
    #[must_use]
    pub fn faded(&self) -> bool {
        self.refusal.is_some() && self.shown.is_some()
    }

    /// Whether the request as it stands has been answered or refused.
    #[must_use]
    pub fn settled(&self) -> bool {
        self.asked.is_none() || self.current.is_some() || self.refusal.is_some()
    }

    /// How many requests the worker has started to read: fewer than the edits
    /// sent, when a burst of them arrived while one was being read.
    #[doc(hidden)]
    #[must_use]
    pub fn reads_started(&self) -> usize {
        self.read_count.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Drop for DraftReads {
    fn drop(&mut self) {
        // The read under way is of no more use; the worker ends when it finds
        // the channel shut.
        if let Some((_, _, stop)) = &self.asked {
            stop.stop();
        }
    }
}

/// Open the warm connection over `source` and the steps that make `tables`.
fn open(source: &TypedSource, tables: &[String], file_limit: u64) -> Warm {
    let (root, upstream) = source.read_over(source.chain_reading(tables));
    match WarmPreview::open(&source.data, source.reader, &root, &upstream, file_limit) {
        Ok(warm) => Warm::Open(warm),
        Err(why @ (PreviewRefusal::TooLarge { .. } | PreviewRefusal::TookTooLong { .. })) => {
            Warm::Unread(why.to_string())
        }
        Err(why) => Warm::Refused(why.to_string()),
    }
}

/// Read `ask`: arc's model for it, then that model run on `warm`. `None` when
/// the read was stopped, since its request has changed.
fn read(
    source: &TypedSource,
    warm: &mut Warm,
    ask: &Ask,
) -> Option<Result<Box<DraftAnswer>, String>> {
    let Some(runner) = &source.runner else {
        return Some(Err(
            "An operation is recorded through arc, and this window was started without it."
                .to_string(),
        ));
    };
    let model = match draft_model(runner, &ask.name, &ask.request) {
        Ok(model) => model,
        Err(why) => return Some(Err(why)),
    };
    if ask.stop.is_stopped() {
        return None;
    }
    let answer = |page: GridPage, rows, rows_in, missing, unread| {
        Some(Ok(Box::new(DraftAnswer {
            request: ask.request.clone(),
            name: ask.name.clone(),
            model: model.clone(),
            page,
            rows,
            rows_in,
            missing,
            unread,
        })))
    };
    let empty = || GridPage {
        window: 0..0,
        columns: Vec::new(),
        rows: Vec::new(),
    };
    let warm = match warm {
        Warm::Open(warm) => warm,
        Warm::Unread(why) => return answer(empty(), 0, 0, Vec::new(), Some(why.clone())),
        Warm::Refused(why) => return Some(Err(why.clone())),
    };
    let op = PreviewOp::Model(model.model.clone());
    match warm.read(
        &ask.name,
        ask.request.on.as_deref(),
        &op,
        PREVIEW_ROWS,
        &ask.stop,
    ) {
        Ok(page) => {
            let hidden = source.raw_columns();
            let missing = page
                .missing
                .into_iter()
                .filter(|(column, _)| !hidden.contains(column))
                .collect();
            answer(
                crate::data_grid::page_of_hiding(&page.batches, &source.shown, &hidden),
                page.rows,
                page.rows_in,
                missing,
                None,
            )
        }
        Err(PreviewRefusal::Superseded) => None,
        Err(why @ (PreviewRefusal::TooLarge { .. } | PreviewRefusal::TookTooLong { .. })) => {
            answer(empty(), 0, 0, Vec::new(), Some(why.to_string()))
        }
        Err(why) => Some(Err(why.to_string())),
    }
}
