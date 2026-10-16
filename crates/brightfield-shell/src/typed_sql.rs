//! **Typed SQL in the chart palette.** Text on the query line that reads as a
//! statement is previewed in the grid and, when kept, becomes a step of the
//! Protocol that Save records through arc. This module holds the parts of that
//! which need no window; the window drives them.
//!
//! **Detection.** Text is a statement when its first word, after leading
//! whitespace, is `select`, `with` or `from` in any case, followed by the end
//! of the text or a character that cannot continue an identifier
//! ([`crate::overlays::is_statement`]): `selection` is a search, and `select*`
//! is a statement. No key is added for it. The palette turns into the
//! statement form, a card with no scrim that stands over the map pane and
//! clear of the grid; where the canvas drew one pane, as over a node's grid,
//! it falls back to the trailing 40% of that pane, which covers the grid's
//! trailing columns. Enter adds a line, Cmd+Enter keeps and Esc closes and
//! keeps nothing. Opening another document closes it and drops what was
//! typed. This is the chart palette of a window whose Protocol
//! came from a data file; anywhere else the text stays a search.
//!
//! **The pause and the stop.** [`TypedReads`] holds the text being typed, the
//! read of it under way, and the last answer whose rows read. A read starts
//! once the text has rested for [`TYPED_PAUSE`], 300 ms. It parses the
//! statement, loads the data file, and runs the steps the statement reads,
//! and the steps those read, and then the statement, on
//! the isolated connection with external access off and under the preview's
//! file limit and deadline, and asks arc what it makes of the statement in a
//! process of its own. So it runs on a worker, and a frame starts, stops and
//! collects reads without waiting on one. An edit while a read is under way
//! stops that read, and so does closing the form. A statement that is not
//! exactly one query that reads rows is refused before it runs, and the row
//! under the query line gives DuckDB's reason.
//!
//! **Cmd+Enter has two outcomes.** It keeps the statement once a read has
//! answered the text as it stands; until then it starts that read without
//! waiting out the pause, and the form stays open.
//!
//! - A statement arc reads as a `filter-rows` of the loaded table lands in the
//!   grid's `where` field, as the filter key leaves it: applied and
//!   unrecorded, with no step of its own, and recorded at Save as the step the
//!   filter key records (see [`crate::grid_foot`]). While it is on the form,
//!   the row under the query line says `filter-rows on <table> · <n> rows` and
//!   the grid previews the condition with that count.
//! - Any other statement is kept as a SQL step. While it is on the form the
//!   row says `a SQL step`, or `replaces <name>` when it would correct the
//!   held step, and the grid previews its first page under the preview
//!   label; kept, it is laid out at the foot of the Protocol, its
//!   table is brought to the front in the grid with the cursor on its first
//!   cell, and its rows are counted on a worker. A palette opened over a kept
//!   statement's grid holds that statement, selected: kept again, it corrects
//!   the step in place under its name, unless it reads that step's own table,
//!   which makes it the step after it. When Save or Run recorded the held
//!   step while the form stood, the correction is kept as a step of its own
//!   under a fresh name, since recorded SQL is not edited.
//!
//! **The name rule** ([`typed_step_name`]): `<table it reads>_sql_by_<column>`
//! when the statement turns on one column, which is the single column of its
//! `GROUP BY`, else of a window's `PARTITION BY`, else the single column its
//! `WHERE` names; otherwise `<table it reads>_sql`. A name a step of the
//! Protocol already has, in any case, takes `_2`, `_3` and so on. The step's
//! table has the same name. A name is fixed when its statement is kept, and
//! Save does not change it.
//!
//! **Unsaved steps are in the Protocol graph.** A kept statement is a
//! [`TypedStep`]. The window lays the Protocol out again from its folder with
//! the kept steps after the folder's own
//! ([`crate::one_step::OneStepProtocol::inputs_with_typed`]), so the graph
//! draws each one's node and table, a later statement can read its table, and
//! a grid over it previews through the steps it reads. The graph is handed
//! `lineage_model` for each in place of the statement, so the node does not
//! depend on the graph's parser reading DuckDB's own syntax. In the spine the
//! step and its table carry the not-recorded mark, the foot row's filled dot,
//! and the table's row gives its count once the count is in. The unsaved mark
//! stays on while a step is kept, and the close question names it. A run's
//! record is matched against the steps less the kept ones, so a statement kept
//! after a run leaves that run's statuses on the steps it ran, and the Steps
//! sheet lists the kept step after them
//! (`a_run_record_leaves_the_typed_step_in_the_sheet` in
//! `tests/typed_sql_nodes.rs`).
//!
//! **Save records the work in the order it was made**
//! ([`crate::window::MeridianApp::record_pending`]): an applied grid view as
//! `filter-rows` and then `sort-rows`, and a kept statement through
//! `arc sql record` under the name it was kept with
//! ([`crate::operation::record_sql`]). arc writes a statement it reads as an
//! operation as that operation's generated step, and any other as
//! `models/NN_<name>.sql`, holding `CREATE OR REPLACE TABLE "<name>" AS`, a
//! newline, the statement as typed, and a final newline when it had none. A
//! refusal raises a banner in arc's words: what was recorded before it stays
//! recorded, and what it refused and what follows stay kept, under the unsaved
//! mark, for the next Save. A Run records the pending work first, as Save
//! does.
//!
//! **Reopening.** A folder holding SQL steps in the shape `arc sql record`
//! writes opens with them laid out after the load
//! ([`crate::generated::parse_sql_step`]). Each reads *not run* until a run's
//! record says otherwise, its grid previews its first page, and the palette
//! over it takes more statements, which read its table as they read a kept
//! step's.

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
use crate::generated::{sql_step_name_for, GeneratedStep, StepEffect};
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

/// **A statement kept from the palette and not yet saved**: a step of the
/// Protocol that no file records, with the tables it reads and, once counted,
/// the rows its table holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedStep {
    /// The step's name, which is also the table it makes.
    pub name: String,
    /// The statement, as typed.
    pub statement: String,
    /// The tables the statement reads, in the order it names them.
    pub reads: Vec<String>,
    /// The rows of its table, once a count has come back.
    pub rows: Option<u64>,
}

impl TypedStep {
    /// The step as one of the steps after the load: a SQL step on the first
    /// table it reads, with the statement as its argument.
    #[must_use]
    pub fn as_generated(&self) -> GeneratedStep {
        GeneratedStep {
            name: self.name.clone(),
            effect: StepEffect::Sql,
            on: self.reads.first().cloned().unwrap_or_default(),
            reads: self.reads.clone(),
            argument: self.statement.clone(),
        }
    }
}

/// **The model the lineage graph is handed for a SQL step**: a `CREATE` of the
/// step's table from each table it reads, `SELECT 1` when its reads are empty. The
/// graph's parser reads this rather than the statement, so the step's node
/// does not depend on that parser reading DuckDB's own syntax, such as a
/// statement that opens with `FROM`
/// (`a_from_first_statement_still_makes_a_node` in `tests/typed_sql_nodes.rs`).
pub(crate) fn lineage_model(step: &TypedStep) -> String {
    lineage_model_of(&step.name, &step.reads)
}

/// [`lineage_model`] for the step `name` reading `reads`.
pub(crate) fn lineage_model_of(name: &str, reads: &[String]) -> String {
    let quoted = |n: &str| format!("\"{}\"", n.replace('"', "\"\""));
    let from = if reads.is_empty() {
        "1".to_string()
    } else {
        let tables: Vec<String> = reads.iter().map(|r| quoted(r)).collect();
        format!("* FROM {}", tables.join(", "))
    };
    format!("CREATE OR REPLACE TABLE {} AS SELECT {from}", quoted(name))
}

/// How long the text must rest after an edit before it is read.
pub const TYPED_PAUSE: Duration = Duration::from_millis(300);

/// What a typed statement is read against: the data file, the table it loads
/// as, and the steps the statement may read, in order.
#[derive(Clone, Debug)]
pub struct TypedSource {
    /// The data file the load reads.
    pub data: PathBuf,
    /// The reader the data file loads with.
    pub reader: FileReader,
    /// The table the load makes.
    pub root_table: String,
    /// The steps the statement may read, in order. A read runs those of
    /// them the statement reads, and the steps those read, and no other
    /// ([`crate::generated::preview_chain_reading`]).
    pub steps: Vec<GeneratedStep>,
    /// The order the grid has applied and not recorded, which a filter of the
    /// root table is paged under, as the grid shows its `where` with it.
    pub order: Option<String>,
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
    /// The first page of those rows under [`TypedSource::order`], when an
    /// order is pending; otherwise the grid previews [`Self::page`].
    pub filter_page: Option<GridPage>,
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
    pub(crate) fn with_hooks(mut self, hooks: PreviewHooks) -> Self {
        self.hooks = hooks;
        self
    }

    /// Note that the text is now `text`, edited at `now`. Nothing is read,
    /// and a read under way of other text is stopped, even when the text
    /// returns to one already answered
    /// (`an_edit_back_to_answered_text_stops_the_read_in_between` in
    /// `tests/typed_sql_palette.rs`).
    pub fn edit(&mut self, text: &str, now: Instant) {
        if text != self.text {
            text.clone_into(&mut self.text);
            if self
                .in_flight
                .as_ref()
                .is_some_and(|(t, _, _)| *t != self.text)
            {
                if let Some((_, stop, _)) = self.in_flight.take() {
                    stop.stop();
                    self.superseded += 1;
                }
            }
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

    /// How long until the pause since the last edit is over, when the text is
    /// waiting out that pause for a read `tick` will start: what a frame asks
    /// to be woken after.
    #[must_use]
    pub fn pause_left(&self, now: Instant) -> Option<Duration> {
        let edited = self.edited?;
        if !self.wants_read() {
            return None;
        }
        Some(
            self.pause
                .saturating_sub(now.saturating_duration_since(edited)),
        )
    }

    /// Whether the text as it stands has neither an answer nor a refusal, and
    /// is not being read.
    fn wants_read(&self) -> bool {
        let answered = self.shown.as_ref().is_some_and(|a| a.text == self.text)
            || self.refused.as_ref().is_some_and(|(t, _)| *t == self.text);
        let being_read = self
            .in_flight
            .as_ref()
            .is_some_and(|(t, _, _)| *t == self.text);
        !(answered || being_read)
    }

    /// Start a read of the text without waiting out the pause, when it has no
    /// answer and is not being read.
    pub fn read_now(&mut self) {
        if !self.wants_read() {
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
                // `edit` stops the read of any text it leaves, so an answer is
                // to the line's text, and a refusal of earlier text goes: that
                // text is read again when the line comes back to it
                // (`text_refused_before_is_read_again_after_other_text_answers`
                // in `tests/typed_sql_reads.rs`).
                debug_assert_eq!(answer.text, self.text);
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

/// A reader dropped mid-read stops that read, so it lets go of the preview
/// gate at once rather than at its deadline
/// (`dropping_the_reader_stops_its_read` in `tests/typed_sql_reads.rs`).
impl Drop for TypedReads {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Count the rows of the last step of `chain`, made from `source`'s data
/// file, on a worker, behind the preview gate, calling `wake` once the count
/// is sent.
pub fn spawn_row_count(
    source: TypedSource,
    chain: Vec<(String, PreviewOp)>,
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
                &chain,
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

/// One read of `text` against `source`: its reading, its page, arc's
/// reading, and the filter's count, checking `stop` between them. `None` when
/// it was stopped.
///
/// The statement is parsed first, here on the worker, so its page is read
/// after only the steps it reads and theirs. A statement DuckDB cannot parse
/// is read after no step, and its preview refuses it in DuckDB's words.
fn read_typed(source: &TypedSource, text: &str, stop: &PreviewStop) -> Option<TypedResult> {
    let parsed = statement_reading(text);
    if stop.is_stopped() {
        return None;
    }
    let mut chain = match &parsed {
        Ok(reading) => crate::generated::preview_chain_reading(
            &source.root_table,
            &source.steps,
            &reading.tables,
        ),
        Err(_) => Vec::new(),
    };
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
    let reading = parsed.unwrap_or_default();
    let recognised = source
        .runner
        .as_ref()
        .and_then(|runner| recognise(runner, text).ok())
        .unwrap_or(Recognised::Other);
    if stop.is_stopped() {
        return None;
    }
    let (filter_rows, filter_page) = match &recognised {
        Recognised::FilterRows { on, condition } if on.eq_ignore_ascii_case(&source.root_table) => {
            let mut filter = vec![(
                free_step_name(&source.root_table, &[]),
                PreviewOp::Where(condition.clone()),
            )];
            let rows = match preview_row_count_stoppable(
                &source.data,
                source.reader,
                &source.root_table,
                &filter,
                PREVIEW_FILE_LIMIT_BYTES,
                PREVIEW_DEADLINE,
                stop,
            ) {
                Ok(rows) => Some(rows),
                Err(PreviewRefusal::Superseded) => return None,
                Err(_) => None,
            };
            let page = match &source.order {
                Some(order) => {
                    filter.push((
                        free_step_name(&source.root_table, &filter),
                        PreviewOp::OrderBy(order.clone()),
                    ));
                    match preview_first_page_stoppable(
                        &source.data,
                        source.reader,
                        &source.root_table,
                        &filter,
                        PREVIEW_ROWS,
                        PREVIEW_FILE_LIMIT_BYTES,
                        PREVIEW_DEADLINE,
                        stop,
                    ) {
                        Ok(batches) => Some(page_of(&batches)),
                        Err(PreviewRefusal::Superseded) => return None,
                        Err(_) => None,
                    }
                }
                None => None,
            };
            (rows, page)
        }
        _ => (None, None),
    };
    Some(Ok(TypedAnswer {
        text: text.to_string(),
        page,
        reading,
        recognised,
        filter_rows,
        filter_page,
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
