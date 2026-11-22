//! **The first page of a step nobody has run**, read on a connection of its
//! own.
//!
//! A step arc generated has no table until the Protocol runs, and the grid
//! still has something honest to show for it: the step's effect on the data
//! file, recomputed here, one page deep. The read is kept apart from the
//! session for three reasons, each a rule this module holds.
//!
//! - **Its own connection.** A fresh in-memory database, so nothing the
//!   session holds is reachable from a condition and nothing the preview makes
//!   lands in the session.
//! - **Shut before any step runs.** The data file is loaded into a table
//!   first; then `enable_external_access` is switched off and the
//!   configuration locked, and after that the steps' conditions and orders
//!   run. A condition that reads another file fails
//!   (`a_condition_that_reads_another_file_fails` in `tests/preview_read.rs`),
//!   and cannot switch the setting back.
//! - **Bounded.** The file's size on disk is checked against
//!   [`PREVIEW_FILE_LIMIT_BYTES`] before the file is opened, and the connection
//!   runs under a memory limit with spilling to disk shut off, because a
//!   compressed Parquet can take several times its size once loaded (the
//!   measurement is on `Session::materialise_source`). Running out of memory is
//!   [`PreviewRefusal::TooLarge`], the same answer as a file over the limit,
//!   and not an engine error. A memory limit does not bound time, so the read
//!   also runs against [`PREVIEW_DEADLINE`]: a condition whose subquery counts
//!   to a quadrillion holds almost no memory, and past the deadline the
//!   connection is interrupted and the read ends as
//!   [`PreviewRefusal::TookTooLong`]. A read the caller has moved past can be
//!   stopped the same way through a [`PreviewStop`], and ends as
//!   [`PreviewRefusal::Superseded`].
//!
//! A step is previewed through the steps before it: a sort on a filter reads
//! the filter, then the sort. A sort's page is read in its own order, then every
//! column by position, so ties fall the same way on every read. A typed
//! statement's page is read in the order its rows were made, which is the
//! statement's own `ORDER BY` when it has one; without one, or between rows
//! its order ties, DuckDB may make them in another order on another read,
//! so that page can differ from read to read. The page is the first one and
//! no more: it is there to prompt the next question, and must not pass for the
//! step's rows or their range. Counting the last step's rows is a read of its
//! own, [`preview_row_count_stoppable`], on the same terms.
//!
//! **A generated step runs as arc wrote it.** For the operations Brightfield
//! does not rebuild as a condition or an order, the step's model is run whole
//! ([`PreviewOp::Model`]): several statements for some operations, which make
//! the step's table and then alter it inside a transaction. A model is not held
//! to one read-only query as a typed statement is, and it does not need to be.
//! It runs once the connection is shut and locked, so whatever it makes, alters
//! or drops is in the preview's own in-memory database and goes with it, and a
//! statement that reaches for a file is refused
//! (`a_model_cannot_reach_outside_the_preview` in `tests/preview_model.rs`).
//! The read then asks that a table of the step's name exists, and refuses a
//! model that left no such table. A model's page is read in the order its rows were
//! made, as a statement's is.
//!
//! **A step being made is read on a connection kept open** ([`WarmPreview`]).
//! A cold read loads the file and runs the chain again for each page it reads,
//! which is right for a page read once and too slow for a read at each
//! keystroke. The warm connection is opened once for a draft, on the same
//! terms, and each edit then runs the draft alone. It holds the table whole, as
//! a cache of sampled rows would not, so a preview of an aggregate is exact.
//!
//! **A second file is a further root** ([`PreviewOp::ReadFile`]): a step that
//! reads a local file is loaded with the data file, before the connection is
//! shut, and held to the same size limit.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use duckdb::Connection;

use crate::RecordBatch;

/// The largest data file a preview reads, in bytes on disk: 256 MiB.
pub const PREVIEW_FILE_LIMIT_BYTES: u64 = 256 * 1024 * 1024;

/// The memory a preview's connection may use, with spilling shut off beside
/// it so the limit refuses rather than writes to disk.
const PREVIEW_MEMORY_LIMIT: &str = "2GB";

/// The longest a preview read may take, load and steps together, before its
/// connection is interrupted: 30 seconds.
pub const PREVIEW_DEADLINE: Duration = Duration::from_secs(30);

/// The DuckDB reader a data file is loaded with, by its extension: the same
/// dispatch the load a data file opens as writes into its model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileReader {
    /// `read_csv`, for `.csv` and `.tsv`.
    Csv,
    /// `read_parquet`, for `.parquet`.
    Parquet,
}

impl FileReader {
    /// The reader for `path`'s extension, or `None` for one neither reads.
    #[must_use]
    pub fn for_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "parquet" => Some(Self::Parquet),
            "csv" | "tsv" => Some(Self::Csv),
            _ => None,
        }
    }

    /// The DuckDB table function's name.
    #[must_use]
    pub const fn function(self) -> &'static str {
        match self {
            Self::Csv => "read_csv",
            Self::Parquet => "read_parquet",
        }
    }
}

/// One step of a previewed chain: what it does to the rows of the step before
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewOp {
    /// Keep the rows a SQL condition keeps.
    Where(String),
    /// Sort the rows by a SQL `ORDER BY` list.
    OrderBy(String),
    /// Make the step's table from a typed query, which reads the root table
    /// or an earlier step's table by name. [`crate::read_only_statement`]
    /// is asked first, and text it refuses does not run
    /// (`a_statement_that_is_not_one_read_only_query_is_refused_before_anything_runs`
    /// in `tests/preview_read.rs`).
    Statement(String),
    /// arc's model for a generated step, run as written once the preview's
    /// access is shut; it makes the step's table.
    Model(String),
    /// A further file loaded as the step's table, by `reader`. It is read
    /// while the connection may still reach outside itself, with the data
    /// file and before any other step runs, wherever it stands in the chain,
    /// and is held to the same size limit as the data file.
    ReadFile {
        /// The file to load.
        path: PathBuf,
        /// The reader it loads with.
        reader: FileReader,
    },
    /// A step the preview does not make, with the sentence saying why. A read
    /// through it is refused in those words
    /// (`a_step_that_is_not_previewed_refuses_in_its_own_words` in
    /// `tests/preview_model.rs`).
    NotPreviewed(String),
}

/// **A switch that stops a stoppable preview read**, shared between the read
/// and whoever may move past it.
///
/// Clones share one switch. Once stopped it stays stopped, so a read started
/// with it afterwards ends at once.
#[derive(Clone, Default, Debug)]
pub struct PreviewStop(Arc<AtomicBool>);

impl PreviewStop {
    /// Stop each read this switch was given.
    pub fn stop(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether [`PreviewStop::stop`] has been called on this switch or a
    /// clone of it.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Why a preview has no rows to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewRefusal {
    /// The data file is over [`PREVIEW_FILE_LIMIT_BYTES`] on disk, or the read
    /// ran out of the memory the preview may use. `bytes` is the file's size.
    TooLarge {
        /// The data file's size on disk.
        bytes: u64,
    },
    /// The read ran past its deadline ([`PREVIEW_DEADLINE`] outside a test)
    /// and was interrupted.
    TookTooLong {
        /// The deadline it ran past.
        deadline: Duration,
    },
    /// DuckDB refused a statement, in its own words.
    Engine(String),
    /// The read's [`PreviewStop`] was stopped, because the text it previews
    /// changed, and its connection was interrupted.
    Superseded,
}

impl std::fmt::Display for PreviewRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { bytes } => {
                write!(f, "the data file is too large to preview ({bytes} bytes)")
            }
            Self::TookTooLong { deadline } => write!(
                f,
                "the preview took longer than {} seconds and was stopped",
                deadline.as_secs()
            ),
            Self::Engine(msg) => f.write_str(msg),
            Self::Superseded => f.write_str("the preview was stopped because the text changed"),
        }
    }
}

/// **The first `limit` rows of the last step of `chain`**, recomputed from the
/// data file on a connection of their own.
///
/// `data` is loaded with `reader` into a table named `root_table`; each entry
/// of `chain` then reads the one before it (the first reads `root_table`) and
/// is made as a table under its own name. The last is read in its own order
/// when it is a sort, its order followed by each column by position as the
/// tie-break, and in `ORDER BY ALL` when it is a filter or the chain is empty,
/// so for those the page is the same on every read. A statement's page is
/// read in the order its rows were made: its own `ORDER BY` fixes that order
/// as far as the keys do not tie, and without one the page can differ
/// between reads.
///
/// # Errors
///
/// [`PreviewRefusal::TooLarge`] for a file over the limit or a read that ran
/// out of memory; [`PreviewRefusal::TookTooLong`] for a read still running at
/// [`PREVIEW_DEADLINE`]; otherwise [`PreviewRefusal::Engine`] with DuckDB's words,
/// which for a condition that reaches outside the database say that external
/// access is off.
pub fn preview_first_page(
    data: &Path,
    reader: FileReader,
    root_table: &str,
    chain: &[(String, PreviewOp)],
    limit: u64,
) -> Result<Vec<RecordBatch>, PreviewRefusal> {
    preview_first_page_with_limit(
        data,
        reader,
        root_table,
        chain,
        limit,
        PREVIEW_FILE_LIMIT_BYTES,
    )
}

/// [`preview_first_page`] under a file-size limit of `file_limit_bytes`, so a
/// test can reach the refusal without a file a quarter of a gigabyte long.
///
/// # Errors
///
/// As [`preview_first_page`].
#[doc(hidden)]
pub fn preview_first_page_with_limit(
    data: &Path,
    reader: FileReader,
    root_table: &str,
    chain: &[(String, PreviewOp)],
    limit: u64,
    file_limit_bytes: u64,
) -> Result<Vec<RecordBatch>, PreviewRefusal> {
    preview_first_page_within(
        data,
        reader,
        root_table,
        chain,
        limit,
        file_limit_bytes,
        PREVIEW_DEADLINE,
    )
}

/// [`preview_first_page_with_limit`] against a deadline of `deadline`, so a
/// test can reach [`PreviewRefusal::TookTooLong`] without waiting out
/// [`PREVIEW_DEADLINE`].
///
/// # Errors
///
/// As [`preview_first_page`], with `deadline` in place of the deadline.
#[doc(hidden)]
pub fn preview_first_page_within(
    data: &Path,
    reader: FileReader,
    root_table: &str,
    chain: &[(String, PreviewOp)],
    limit: u64,
    file_limit_bytes: u64,
    deadline: Duration,
) -> Result<Vec<RecordBatch>, PreviewRefusal> {
    preview_first_page_stoppable(
        data,
        reader,
        root_table,
        chain,
        limit,
        file_limit_bytes,
        deadline,
        &PreviewStop::default(),
    )
}

/// [`preview_first_page_within`] that also ends when `stop` is stopped: the
/// connection is interrupted within about ten milliseconds of the stop, as it
/// is at the deadline, and the read ends as [`PreviewRefusal::Superseded`].
///
/// # Errors
///
/// As [`preview_first_page_within`], and [`PreviewRefusal::Superseded`] for a
/// read stopped through `stop`.
#[allow(clippy::too_many_arguments)]
pub fn preview_first_page_stoppable(
    data: &Path,
    reader: FileReader,
    root_table: &str,
    chain: &[(String, PreviewOp)],
    limit: u64,
    file_limit_bytes: u64,
    deadline: Duration,
    stop: &PreviewStop,
) -> Result<Vec<RecordBatch>, PreviewRefusal> {
    with_chain(
        data,
        reader,
        root_table,
        chain,
        file_limit_bytes,
        deadline,
        stop,
        |conn, last, refused| {
            let rows_sql = format!("SELECT * FROM {}", quote(last));
            let sql = match chain.last() {
                Some((_, PreviewOp::OrderBy(order))) => {
                    let width: i64 = conn
                        .query_row(
                            &format!("SELECT count(*) FROM (DESCRIBE {rows_sql})"),
                            [],
                            |r| r.get(0),
                        )
                        .map_err(refused)?;
                    crate::ordered_window_sql(&rows_sql, order, width, 0, limit)
                }
                // The table keeps the order the statement made its rows in.
                Some((_, PreviewOp::Statement(_) | PreviewOp::Model(_))) => {
                    format!("{rows_sql} LIMIT {limit}")
                }
                _ => crate::unordered_window_sql(&rows_sql, 0, limit),
            };
            let mut statement = conn.prepare(&sql).map_err(refused)?;
            Ok(statement
                .query_arrow(duckdb::params![])
                .map_err(refused)?
                .collect())
        },
    )
}

/// **The number of rows in the last step of `chain`**, recomputed from the
/// data file on a connection of their own, on the same terms as
/// [`preview_first_page_stoppable`].
///
/// # Errors
///
/// As [`preview_first_page_stoppable`].
pub fn preview_row_count_stoppable(
    data: &Path,
    reader: FileReader,
    root_table: &str,
    chain: &[(String, PreviewOp)],
    file_limit_bytes: u64,
    deadline: Duration,
    stop: &PreviewStop,
) -> Result<u64, PreviewRefusal> {
    with_chain(
        data,
        reader,
        root_table,
        chain,
        file_limit_bytes,
        deadline,
        stop,
        |conn, last, refused| {
            let count: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {}", quote(last)), [], |r| {
                    r.get(0)
                })
                .map_err(refused)?;
            Ok(u64::try_from(count).unwrap_or(0))
        },
    )
}

/// Loads `data`, shuts and locks the connection, makes each step of `chain`
/// as a table, and hands `read` the connection, the last table's name and the
/// map from a DuckDB error to a refusal. One watch of `deadline` and `stop`
/// covers the load, the steps and `read`.
#[allow(clippy::too_many_arguments)]
fn with_chain<T>(
    data: &Path,
    reader: FileReader,
    root_table: &str,
    chain: &[(String, PreviewOp)],
    file_limit_bytes: u64,
    deadline: Duration,
    stop: &PreviewStop,
    read: impl FnOnce(
        &Connection,
        &str,
        &dyn Fn(duckdb::Error) -> PreviewRefusal,
    ) -> Result<T, PreviewRefusal>,
) -> Result<T, PreviewRefusal> {
    let bytes = size_within(data, file_limit_bytes)?;
    let conn = Connection::open_in_memory().map_err(|e| refusal(&e.to_string(), bytes))?;
    // Taken before any SQL runs, and dropped (stopping the watch) on every
    // way out of this function.
    let watch = Watchdog::start(&conn, deadline, stop.clone());
    let refused = |e: duckdb::Error| {
        watch
            .refusal(deadline)
            .unwrap_or_else(|| refusal(&e.to_string(), bytes))
    };
    load_and_shut(
        &conn,
        data,
        reader,
        root_table,
        chain,
        file_limit_bytes,
        &refused,
    )?;

    let mut previous = root_table;
    for (name, op) in chain {
        make_step(&conn, previous, name, op, &refused)?;
        previous = name;
    }

    let out = read(&conn, previous, &refused)?;
    // An interrupt that landed between statements is not an error from any
    // of them, and the result it leaves is not to be trusted as complete.
    if let Some(why) = watch.refusal(deadline) {
        return Err(why);
    }
    Ok(out)
}

/// The size of `file` on disk, when it is within `limit` bytes.
fn size_within(file: &Path, limit: u64) -> Result<u64, PreviewRefusal> {
    let bytes = std::fs::metadata(file)
        .map_err(|e| PreviewRefusal::Engine(format!("{}: {e}", file.display())))?
        .len();
    if bytes > limit {
        return Err(PreviewRefusal::TooLarge { bytes });
    }
    Ok(bytes)
}

/// Sets the connection's limits, loads `data` as `root_table` and each
/// [`PreviewOp::ReadFile`] of `steps` as its step's table, then shuts the
/// connection to what is outside it and locks it shut. No step's text has
/// run when this returns.
fn load_and_shut(
    conn: &Connection,
    data: &Path,
    reader: FileReader,
    root_table: &str,
    steps: &[(String, PreviewOp)],
    file_limit_bytes: u64,
    refused: &dyn Fn(duckdb::Error) -> PreviewRefusal,
) -> Result<(), PreviewRefusal> {
    // The limit and the spill before the load, so the load is held to them.
    conn.execute_batch(&format!(
        "SET memory_limit='{PREVIEW_MEMORY_LIMIT}'; SET max_temp_directory_size='0B';"
    ))
    .map_err(refused)?;
    let load = |table: &str, file: &Path, reader: FileReader| {
        let path = file.to_string_lossy().replace('\'', "''");
        conn.execute_batch(&format!(
            "CREATE TABLE {} AS SELECT * FROM {}('{path}');",
            quote(table),
            reader.function()
        ))
        .map_err(refused)
    };
    load(root_table, data, reader)?;
    for (name, op) in steps {
        if let PreviewOp::ReadFile { path, reader } = op {
            size_within(path, file_limit_bytes)?;
            load(name, path, *reader)?;
        }
    }
    // Shut, and locked shut, before any step's text runs.
    conn.execute_batch("SET enable_external_access = false; SET lock_configuration = true;")
        .map_err(refused)
}

/// Makes the step `name` as a table: `op` over `previous`, the table made just
/// before it. A [`PreviewOp::ReadFile`] was loaded by [`load_and_shut`] and is
/// left as it is.
fn make_step(
    conn: &Connection,
    previous: &str,
    name: &str,
    op: &PreviewOp,
    refused: &dyn Fn(duckdb::Error) -> PreviewRefusal,
) -> Result<(), PreviewRefusal> {
    let create = match op {
        PreviewOp::Where(condition) => {
            one_statement(crate::condition_statement_count(&format!(
                "(\n{condition}\n)"
            )))?;
            format!(
                "CREATE TABLE {} AS SELECT * FROM {} WHERE (\n{condition}\n);",
                quote(name),
                quote(previous)
            )
        }
        PreviewOp::OrderBy(order) => {
            one_statement(crate::order_statement_count(&format!("{order}\n, 1")))?;
            format!(
                "CREATE TABLE {} AS SELECT * FROM {} ORDER BY \n{order}\n;",
                quote(name),
                quote(previous)
            )
        }
        PreviewOp::Statement(sql) => {
            crate::read_only_statement(sql).map_err(PreviewRefusal::Engine)?;
            // No `;` after the text: it may end in one, or in a `--`
            // comment that would swallow it.
            format!("CREATE TABLE {} AS\n{sql}", quote(name))
        }
        // arc's model names the table it makes and the tables it reads.
        PreviewOp::Model(model) => model.clone(),
        PreviewOp::ReadFile { .. } => return Ok(()),
        PreviewOp::NotPreviewed(why) => return Err(PreviewRefusal::Engine(why.clone())),
    };
    conn.execute_batch(&create).map_err(refused)?;
    if matches!(op, PreviewOp::Model(_)) {
        made_its_table(conn, name).map_err(|e| match e {
            Some(e) => refused(e),
            None => PreviewRefusal::Engine(no_table_of_its_name(name)),
        })?;
    }
    Ok(())
}

/// What one read of a [`WarmPreview`] found.
#[derive(Debug)]
pub struct WarmPage {
    /// The step's first rows, in the order they were made.
    pub batches: Vec<RecordBatch>,
    /// How many rows the step's table holds.
    pub rows: u64,
    /// How many rows the table the step is on holds; 0 for a step on no table.
    pub rows_in: u64,
    /// Each column of the step's table with how many of its values are
    /// missing, in the table's order.
    pub missing: Vec<(String, u64)>,
}

/// **A preview connection kept open while one step is being made**, so each
/// edit of that step runs the step alone.
///
/// [`WarmPreview::open`] loads the data file and makes the steps the draft
/// reads once, on the terms the cold reads hold to: a connection of its own,
/// the files held to the size limit, the memory limit with spilling off, and
/// external access shut and locked before a step's text runs. Each
/// [`WarmPreview::read`] then makes the draft's table from what is there and
/// reads it, under the deadline and a [`PreviewStop`] of its own.
///
/// The connection holds the table whole, so a draft that aggregates is
/// computed over its rows and not over a sample of them: the counts a preview
/// shows are the ones a run would give for the file as it was when the draft
/// was started. A change to the file after that is not seen
/// (`the_table_is_loaded_once_and_each_read_runs_only_the_draft` in
/// `tests/warm_preview.rs`).
pub struct WarmPreview {
    conn: Connection,
    root_table: String,
    /// The tables the open made, case folded: the root and each step of the
    /// upstream chain. A draft may read them and may not be named for one.
    made: Vec<String>,
    bytes: u64,
    /// The table the last read made, dropped before the next.
    drafted: Option<String>,
}

impl std::fmt::Debug for WarmPreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WarmPreview")
            .field("root_table", &self.root_table)
            .finish_non_exhaustive()
    }
}

impl WarmPreview {
    /// Load `data` as `root_table`, and each [`PreviewOp::ReadFile`] of
    /// `upstream`; shut the connection and lock it; then make each other step
    /// of `upstream`, in order, each over the one before it.
    ///
    /// # Errors
    ///
    /// As [`preview_first_page`], for a file over `file_limit` bytes, a load or
    /// a step DuckDB refuses, or an open still running at [`PREVIEW_DEADLINE`].
    pub fn open(
        data: &Path,
        reader: FileReader,
        root_table: &str,
        upstream: &[(String, PreviewOp)],
        file_limit: u64,
    ) -> Result<Self, PreviewRefusal> {
        Self::open_stoppable(
            data,
            reader,
            root_table,
            upstream,
            file_limit,
            &PreviewStop::default(),
        )
    }

    /// [`WarmPreview::open`] that also ends when `stop` is stopped, as
    /// [`PreviewRefusal::Superseded`]: a draft dropped while its table is
    /// still loading lets go of the load
    /// (`an_open_that_is_stopped_ends_as_superseded` in
    /// `tests/warm_preview.rs`).
    ///
    /// # Errors
    ///
    /// As [`WarmPreview::open`], and [`PreviewRefusal::Superseded`] for an open
    /// stopped through `stop`.
    pub fn open_stoppable(
        data: &Path,
        reader: FileReader,
        root_table: &str,
        upstream: &[(String, PreviewOp)],
        file_limit: u64,
        stop: &PreviewStop,
    ) -> Result<Self, PreviewRefusal> {
        let bytes = size_within(data, file_limit)?;
        let conn = Connection::open_in_memory().map_err(|e| refusal(&e.to_string(), bytes))?;
        {
            let watch = Watchdog::start(&conn, PREVIEW_DEADLINE, stop.clone());
            let refused = |e: duckdb::Error| {
                watch
                    .refusal(PREVIEW_DEADLINE)
                    .unwrap_or_else(|| refusal(&e.to_string(), bytes))
            };
            load_and_shut(
                &conn, data, reader, root_table, upstream, file_limit, &refused,
            )?;
            let mut previous = root_table;
            for (name, op) in upstream {
                make_step(&conn, previous, name, op, &refused)?;
                previous = name;
            }
            if let Some(why) = watch.refusal(PREVIEW_DEADLINE) {
                return Err(why);
            }
        }
        let made = std::iter::once(root_table)
            .chain(upstream.iter().map(|(name, _)| name.as_str()))
            .map(str::to_lowercase)
            .collect();
        Ok(Self {
            conn,
            root_table: root_table.to_owned(),
            made,
            bytes,
            drafted: None,
        })
    }

    /// Make the step `name` by running `op` over what [`WarmPreview::open`]
    /// made, with the table the last read made dropped first, and read its
    /// first `limit` rows, its row count, the row count of `on`, the table it
    /// is on, and each column's count of missing values.
    ///
    /// An `op` that reads the table before it, a condition or an order, reads
    /// `on`, or the root table when `on` is `None`. A read that fails inside a
    /// transaction arc's model opened has that transaction rolled back, so the
    /// next read starts clean.
    ///
    /// A step named for a table the open made, in any case, is refused before
    /// a table is dropped: making it would replace a table the draft's later
    /// reads stand on.
    ///
    /// # Errors
    ///
    /// As [`preview_first_page_stoppable`], against [`PREVIEW_DEADLINE`].
    pub fn read(
        &mut self,
        name: &str,
        on: Option<&str>,
        op: &PreviewOp,
        limit: u64,
        stop: &PreviewStop,
    ) -> Result<WarmPage, PreviewRefusal> {
        let bytes = self.bytes;
        let watch = Watchdog::start(&self.conn, PREVIEW_DEADLINE, stop.clone());
        let refused = |e: duckdb::Error| {
            watch
                .refusal(PREVIEW_DEADLINE)
                .unwrap_or_else(|| refusal(&e.to_string(), bytes))
        };
        let page = self.read_watched(name, on, op, limit, &refused);
        if page.is_err() {
            // Not in a transaction is the usual case, and DuckDB's refusal of
            // the rollback then is of no interest.
            let _ = self.conn.execute_batch("ROLLBACK;");
        }
        let page = page?;
        if let Some(why) = watch.refusal(PREVIEW_DEADLINE) {
            return Err(why);
        }
        Ok(page)
    }

    fn read_watched(
        &mut self,
        name: &str,
        on: Option<&str>,
        op: &PreviewOp,
        limit: u64,
        refused: &dyn Fn(duckdb::Error) -> PreviewRefusal,
    ) -> Result<WarmPage, PreviewRefusal> {
        if matches!(op, PreviewOp::ReadFile { .. }) {
            return Err(PreviewRefusal::Engine(
                "a file is read when the preview opens, and not after".to_owned(),
            ));
        }
        if self.made.contains(&name.to_lowercase()) {
            return Err(PreviewRefusal::Engine(format!(
                "{name} is a table this step reads from, so the step cannot be previewed under that name"
            )));
        }
        if let Some(last) = self.drafted.take() {
            self.conn
                .execute_batch(&format!("DROP TABLE IF EXISTS {};", quote(&last)))
                .map_err(refused)?;
        }
        // Noted before the step runs, so a table a failed read left is
        // dropped by the next.
        self.drafted = Some(name.to_owned());
        self.conn
            .execute_batch(&format!("DROP TABLE IF EXISTS {};", quote(name)))
            .map_err(refused)?;
        let previous = on.unwrap_or(&self.root_table).to_owned();
        make_step(&self.conn, &previous, name, op, refused)?;

        let count = |table: &str| -> Result<u64, PreviewRefusal> {
            let rows: i64 = self
                .conn
                .query_row(&format!("SELECT count(*) FROM {}", quote(table)), [], |r| {
                    r.get(0)
                })
                .map_err(refused)?;
            Ok(u64::try_from(rows).unwrap_or(0))
        };
        let rows = count(name)?;
        let rows_in = match on {
            Some(on) => count(on)?,
            None => 0,
        };
        let columns: Vec<String> = {
            let mut described = self
                .conn
                .prepare(&format!(
                    "SELECT column_name FROM (DESCRIBE SELECT * FROM {})",
                    quote(name)
                ))
                .map_err(refused)?;
            let names = described
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(refused)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(refused)?;
            names
        };
        let mut missing = Vec::with_capacity(columns.len());
        if !columns.is_empty() {
            let counts: Vec<String> = columns
                .iter()
                .map(|c| format!("count(*) FILTER (WHERE {} IS NULL)", quote(c)))
                .collect();
            let sql = format!("SELECT {} FROM {}", counts.join(", "), quote(name));
            let nulls: Vec<i64> = self
                .conn
                .query_row(&sql, [], |r| {
                    (0..columns.len()).map(|i| r.get::<_, i64>(i)).collect()
                })
                .map_err(refused)?;
            for (column, nulls) in columns.into_iter().zip(nulls) {
                missing.push((column, u64::try_from(nulls).unwrap_or(0)));
            }
        }
        let mut statement = self
            .conn
            .prepare(&format!("SELECT * FROM {} LIMIT {limit}", quote(name)))
            .map_err(refused)?;
        let batches = statement
            .query_arrow(duckdb::params![])
            .map_err(refused)?
            .collect();
        Ok(WarmPage {
            batches,
            rows,
            rows_in,
            missing,
        })
    }
}

/// The longest the watch waits before looking at its stop again, so a stop is
/// heard within about this long.
const STOP_POLL: Duration = Duration::from_millis(10);

/// The watch has not fired.
const NOT_FIRED: u8 = 0;
/// The watch fired at the deadline.
const FIRED_AT_DEADLINE: u8 = 1;
/// The watch fired because its stop was stopped.
const FIRED_BY_STOP: u8 = 2;

/// **Interrupts a connection that is still working at its deadline, or once
/// its stop is stopped.**
///
/// A thread waits out the deadline in slices of at most [`STOP_POLL`],
/// looking at the stop between them; at whichever comes first, if the read
/// has not finished, it records which one fired and interrupts the
/// connection, and keeps interrupting it every few milliseconds until the
/// read gives up, because an interrupt that lands between two statements
/// stops neither. Dropping the watch tells the thread the read is over and
/// waits for it to go.
struct Watchdog {
    fired: Arc<AtomicU8>,
    done: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watchdog {
    fn start(conn: &Connection, deadline: Duration, stop: PreviewStop) -> Self {
        let handle = conn.interrupt_handle();
        let fired = Arc::new(AtomicU8::new(NOT_FIRED));
        let (done, over) = std::sync::mpsc::channel::<()>();
        let flag = fired.clone();
        let thread = std::thread::spawn(move || {
            let started = Instant::now();
            let why = loop {
                if stop.is_stopped() {
                    break FIRED_BY_STOP;
                }
                let left = deadline.saturating_sub(started.elapsed());
                if left.is_zero() {
                    break FIRED_AT_DEADLINE;
                }
                if over.recv_timeout(left.min(STOP_POLL))
                    != Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                {
                    return;
                }
            };
            flag.store(why, Ordering::SeqCst);
            loop {
                handle.interrupt();
                if over.recv_timeout(Duration::from_millis(5))
                    != Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                {
                    return;
                }
            }
        });
        Self {
            fired,
            done: Some(done),
            thread: Some(thread),
        }
    }

    /// The refusal the watch's firing stands for, or `None` while it has not
    /// fired.
    fn refusal(&self, deadline: Duration) -> Option<PreviewRefusal> {
        match self.fired.load(Ordering::SeqCst) {
            FIRED_AT_DEADLINE => Some(PreviewRefusal::TookTooLong { deadline }),
            FIRED_BY_STOP => Some(PreviewRefusal::Superseded),
            _ => None,
        }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        drop(self.done.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// What a read says of a model that ran and left no table named for its step.
fn no_table_of_its_name(step: &str) -> String {
    format!("{step}: arc's model made no table of its name")
}

/// Whether the connection holds a table named `step`: `Err(None)` when it
/// holds no such table, and `Err(Some(_))` when DuckDB could not be asked.
fn made_its_table(conn: &Connection, step: &str) -> Result<(), Option<duckdb::Error>> {
    let held: i64 = conn
        .query_row(
            "SELECT count(*) FROM duckdb_tables() WHERE NOT internal AND table_name = ?",
            [step],
            |r| r.get(0),
        )
        .map_err(Some)?;
    if held > 0 {
        Ok(())
    } else {
        Err(None)
    }
}

/// `name` as a double-quoted identifier, an embedded quote doubled.
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// A statement count that is not exactly one, as a refusal: the step's text
/// would end the statement it is put in and start another.
fn one_statement(count: Result<usize, String>) -> Result<(), PreviewRefusal> {
    match count {
        Ok(1) => Ok(()),
        Ok(n) => Err(PreviewRefusal::Engine(format!(
            "the step's text reads as {n} statements, not one"
        ))),
        Err(e) => Err(PreviewRefusal::Engine(e)),
    }
}

/// DuckDB's message `msg` as a refusal: running out of memory is the file
/// being too large to preview, and a statement refused for reaching outside
/// the database says that external access is off, beside DuckDB's own words.
fn refusal(msg: &str, bytes: u64) -> PreviewRefusal {
    if msg.contains("Out of Memory") {
        return PreviewRefusal::TooLarge { bytes };
    }
    if msg.contains("Permission Error") {
        return PreviewRefusal::Engine(format!(
            "a preview runs with external access off, so a step may read only \
             the data file: {msg}"
        ));
    }
    PreviewRefusal::Engine(msg.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_out_of_memory_is_too_large_to_preview() {
        let msg = "Out of Memory Error: failed to allocate data of size 64.0 MiB \
                   (1.9 GiB/2.0 GiB used)";
        assert_eq!(
            refusal(msg, 1_234),
            PreviewRefusal::TooLarge { bytes: 1_234 }
        );
    }

    #[test]
    fn another_failure_keeps_duckdbs_words() {
        let msg = "Binder Error: Referenced column \"nope\" not found";
        assert_eq!(refusal(msg, 1), PreviewRefusal::Engine(msg.to_string()));
    }
}
