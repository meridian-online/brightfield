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
//!   [`PreviewRefusal::TookTooLong`].
//!
//! A step is previewed through the steps before it: a sort on a filter reads
//! the filter, then the sort. A sort's page is read in its own order, then every
//! column by position, so ties fall the same way on every read. The page is
//! the first one and no more, and nothing here counts the step's rows: the page
//! is there to prompt the next question, and must not pass for the step's
//! rows or their range.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

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
/// tie-break, and in `ORDER BY ALL` otherwise, so the page is the same page on
/// every read.
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
    let bytes = std::fs::metadata(data)
        .map_err(|e| PreviewRefusal::Engine(format!("{}: {e}", data.display())))?
        .len();
    if bytes > file_limit_bytes {
        return Err(PreviewRefusal::TooLarge { bytes });
    }
    let conn = Connection::open_in_memory().map_err(|e| refusal(&e.to_string(), bytes))?;
    // Taken before any SQL runs, and dropped (stopping the watch) on every
    // way out of this function.
    let watch = Watchdog::start(&conn, deadline);
    let refused = |e: duckdb::Error| {
        if watch.fired() {
            PreviewRefusal::TookTooLong { deadline }
        } else {
            refusal(&e.to_string(), bytes)
        }
    };
    // The limit and the spill before the load, so the load is held to them.
    conn.execute_batch(&format!(
        "SET memory_limit='{PREVIEW_MEMORY_LIMIT}'; SET max_temp_directory_size='0B';"
    ))
    .map_err(refused)?;
    let path = data.to_string_lossy().replace('\'', "''");
    conn.execute_batch(&format!(
        "CREATE TABLE {} AS SELECT * FROM {}('{path}');",
        quote(root_table),
        reader.function()
    ))
    .map_err(refused)?;
    // Shut, and locked shut, before any step's text runs.
    conn.execute_batch("SET enable_external_access = false; SET lock_configuration = true;")
        .map_err(refused)?;

    let mut previous = root_table;
    for (name, op) in chain {
        let body = match op {
            PreviewOp::Where(condition) => {
                one_statement(crate::condition_statement_count(&format!(
                    "(\n{condition}\n)"
                )))?;
                format!("SELECT * FROM {} WHERE (\n{condition}\n)", quote(previous))
            }
            PreviewOp::OrderBy(order) => {
                one_statement(crate::order_statement_count(&format!("{order}\n, 1")))?;
                format!("SELECT * FROM {} ORDER BY \n{order}\n", quote(previous))
            }
        };
        conn.execute_batch(&format!("CREATE TABLE {} AS {body};", quote(name)))
            .map_err(refused)?;
        previous = name;
    }

    let rows_sql = format!("SELECT * FROM {}", quote(previous));
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
        _ => crate::unordered_window_sql(&rows_sql, 0, limit),
    };
    let mut statement = conn.prepare(&sql).map_err(refused)?;
    let batches = statement
        .query_arrow(duckdb::params![])
        .map_err(refused)?
        .collect();
    // An interrupt that landed between statements is not an error from any
    // of them, and the page it leaves is not to be trusted as complete.
    if watch.fired() {
        return Err(PreviewRefusal::TookTooLong { deadline });
    }
    Ok(batches)
}

/// **Interrupts a connection that is still working at its deadline.**
///
/// A thread waits out the deadline; if the read has not finished by then it
/// marks the watch fired and interrupts the connection, and keeps
/// interrupting it every few milliseconds until the read gives up, because an
/// interrupt that lands between two statements stops neither. Dropping the
/// watch tells the thread the read is over and waits for it to go.
struct Watchdog {
    fired: Arc<AtomicBool>,
    done: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watchdog {
    fn start(conn: &Connection, deadline: Duration) -> Self {
        let handle = conn.interrupt_handle();
        let fired = Arc::new(AtomicBool::new(false));
        let (done, over) = std::sync::mpsc::channel::<()>();
        let flag = fired.clone();
        let thread = std::thread::spawn(move || {
            if over.recv_timeout(deadline) != Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                return;
            }
            flag.store(true, Ordering::SeqCst);
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

    fn fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
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
