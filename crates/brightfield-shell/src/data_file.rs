//! Opening a data file the user chose: a CSV or a Parquet, straight to a
//! queryable table.
//!
//! Until this existed the app could be pointed at a chart spec or at a
//! protocol manifest and at nothing else. There was no file dialog anywhere in
//! the tree and no dependency that could raise one, so the only way to see your
//! own data was to write a spec describing it first — which is asking someone
//! to author a document about a file before they are allowed to look at the
//! file.
//!
//! # What opening a file means here
//!
//! The file becomes a **DuckDB view in a live session**, exactly as a `file:`
//! source in a hand-written spec would: [`open`] synthesises a spec whose one
//! `data:` entry is the chosen path, loads it live, and hands back the session
//! and its first composition. The rows do not come back into Rust on the way —
//! the Data pane beside the chart reads `SELECT * FROM <view>` through the
//! engine's windowed row seam, a page at a time.
//!
//! A small file is then copied into DuckDB before the first picture is drawn,
//! and the section below is why. A file whose table does not fit the memory
//! budget is not copied, and it opens off the view exactly as it did before
//! any of this existed.
//!
//! # Why a small file is copied into DuckDB first, and a large one is not
//!
//! A view over `read_csv` is re-read and re-parsed by **each statement issued
//! over it**, so the wait before the first picture is a count of statements
//! times the cost of one read — and the dashboard [`crate::dashboard`] chooses
//! is a tile per column, which makes that count grow with the table's width.
//! Measured on an Apple M1 Pro against this build's DuckDB v1.5.2, on a
//! 2,967,637-byte CSV of 14,133 rows and 22 columns: the first composition read
//! the file 46 times and took 8,199.1 ms of a 9,033.6 ms open — the record is
//! `benchmarks/results/open-scan/2026-09-07-apple-m1-pro-from-the-file.json`,
//! and `--open-scan-no-materialise` is how the harness measures that branch
//! again. Fourteen of those tiles are histograms and each costs three reads:
//! one for the rows and two more for the bin extent, which is a `min` and a
//! `max` the profile pass has already taken.
//!
//! So [`open`] reads the file **once**, into a session-scoped table, and points
//! the view at that ([`brightfield_engine::Session::materialise_source`]). A
//! tile's query is byte-identical and scans memory — the rows it draws are
//! compared down both branches by
//! `every_mark_draws_the_same_rows_down_both_branches_of_the_threshold`. The
//! composition then reads the file no times, which is what
//! [`crate::pipeline::COMPOSITION_FILE_READS`] states,
//! `composing_a_wide_dashboard_reads_the_file_no_more_often_than_a_narrow_one`
//! holds, and the open-scan harness measures — the same file, the same
//! machine, eight seconds earlier: 52.7 ms of a 998.5 ms open, in
//! `benchmarks/results/open-scan/2026-09-07-apple-m1-pro.json`. Both records
//! carry the load average they were taken under, because these are wall times
//! and the machine was not idle.
//!
//! # What decides whether a file is copied, and in which unit
//!
//! Two things decide it, and they are in two different units because they are
//! answering two different questions.
//!
//! [`MATERIALISE_BUDGET_BYTES`] is the **memory** the copy may cost, and it is
//! the guard. It is not a prediction: it is handed to DuckDB as a
//! `memory_limit` for the duration of the copy, with spilling shut off, so a
//! table that does not fit it comes back as an error rather than as a swap
//! storm — [`brightfield_engine::Session::materialise_source`] carries the
//! measurement, and `data_file::open` then leaves the source as a view and
//! composes off the file. That branch is what a file too big to copy takes,
//! and `a_file_whose_table_exceeds_the_budget_opens_off_the_view` drives it
//! through a real open.
//!
//! [`MATERIALISE_UNDER_BYTES`] is the file's size **on disk**, and what it
//! decides is whether the copy is worth attempting. It makes no claim about
//! memory, and it could not: measured on this build's DuckDB, a four-column ZSTD
//! Parquet of 123,260 bytes on disk becomes a 511,031,296-byte table, which is
//! 4,146 times its size on disk. What it does buy is the reading a doomed
//! attempt would have done before the budget stopped it.
//!
//! The two errors are not symmetric, which is why one of these is generous and
//! the other is not. Refusing to copy a file that would have fitted costs a
//! slower open of a big table, which is what happened before and is
//! survivable; copying one that does not fit costs the open entirely, on the
//! table where a `file:` source was the whole point.
//!
//! # A `file:` session is a snapshot taken at open
//!
//! This is a real consequence of the copy and it is worth stating plainly. A
//! file that was copied is read once, at open. Editing that file on disk
//! afterwards changes nothing on screen — before the copy, a brush or a page
//! turn re-read the view and picked the edit up (torn against the SQL cache,
//! but picked it up). Reopening the file is what shows an edit now.
//!
//! [`crate::watch`] still raises its standing *data changed on disk* notice
//! when the file's fingerprint moves, and that notice offers no reload action;
//! it tells the reader the thing on screen is now behind the file, which is
//! true, and reopening is the way forward.
//!
//! # The other half: what was opened, as a Protocol
//!
//! [`open`] also builds the **spec** that says what was opened — one SQL step
//! reading the file, one table, and the file's columns — so the navigator,
//! Steps and inspector rails have something to draw. The shape of that spec
//! is [`crate::one_step`]'s business rather than this module's; what is here
//! is the route.
//!
//! **What is drawn on it is [`crate::dashboard`]'s decision, not this
//! module's**: a tile per column, each chosen from what that column means. This
//! module owns the route — what may be opened, how the schema is read, and
//! where the generated spec is written for the reader — and holds no opinion
//! about pictures.
//!
//! # Why the schema is read before the spec is written
//!
//! A spec has to name columns, and nobody knows the columns of a file they have
//! not opened. So [`open`] loads twice: once over a **root-less** spec that
//! declares only the source (no plot, no marks — nothing executes), which is
//! what makes `profile_sources` able to answer what the columns are and what
//! they hold; and once over the spec that first answer lets it write. The cost
//! is a profile pass over the table before the first picture, and it buys a
//! dashboard chosen from the data rather than guessed at.
//!
//! # Why a URL is refused
//!
//! This opens a file on this machine. DuckDB will happily bind a view over an
//! `https://` Parquet through `httpfs`, so a path box that passed the string on
//! would fetch instead — and succeed, with nothing red anywhere. [`accept`]
//! refuses anything carrying a URL scheme, before the engine is involved at
//! all, and says so.
//!
//! # Why a name the reader would take as a pattern is refused
//!
//! A chosen path crosses two languages on its way to DuckDB: it is written into
//! a YAML scalar, and it is handed to `read_csv` / `read_parquet`, which resolve
//! it as a **glob**. Each has characters that mean something other than
//! themselves, and in both the failure is the same and it is silent — the file
//! that opens is not the file that was picked, under a window titled with the
//! picked name.
//!
//! Measured against this build's DuckDB (`SELECT version()` reports `v1.5.2`),
//! with `sales1.csv` sitting beside `sales[1].csv`: `read_csv` on the second
//! path returns the first file's rows. `glob()` is what decides it, and the
//! danger is exactly *`glob(p)` matching a non-empty set that is not `{p}`* —
//! when nothing matches, DuckDB falls back to the literal path, which is why
//! most punctuation is harmless. There is no way to turn this off. Measured on
//! the same build: no reader keyword disables it, the list form
//! `read_csv([…])` still globs, a `file://` prefix still globs, and `\[` is not
//! an escape — it matches nothing rather than matching the bracket.
//!
//! So [`accept`] **refuses** these names rather than escaping them, and the
//! reason is that the escape is not ours to rely on. The one spelling that does
//! work today, rewriting `[` as the character class `[[]`, is undocumented
//! dialect: if a DuckDB bump changed it the app would go back to opening the
//! wrong file silently, which is the one outcome that is not allowed here.
//! Escaping in the emitter was rejected for a second reason — `brightfield-sql`
//! is shared, and a glob in a hand-written spec's `file:` is a feature there,
//! so that seam cannot tell a pattern from a name. A refusal naming the file
//! and the character is the answer that cannot silently be wrong, and it is the
//! same answer a URL gets.
//!
//! The control-character half is the YAML side: a line break inside a
//! single-quoted scalar is **folded to a space** by the parser, so
//! `sales<LF>2026.csv` would come back as `sales 2026.csv` and open that file
//! instead. It is refused in the same place and for the same reason.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

use brightfield_engine::{ColumnProfile, Engine, LoadOptions, ProfileOutcome, ScanTally};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::{parse_spec, Format};

use crate::dashboard::Dashboard;
use crate::pipeline::{Composed, LiveDashboard};

/// The file extensions this action opens, lowercased and without the dot.
///
/// The engine's source emitter dispatches on exactly these spellings for the
/// two formats a data file arrives in; `tsv` rides the same reader as `csv`.
/// JSON and the DuckDB/DuckLake attach forms are deliberately absent: they are
/// emitted by the same dispatch but they are not what "open a data file" means
/// to someone with a table in front of them, and each carries its own
/// behaviour (a whole catalog attaches, a database file has many tables) that
/// this one-view path has no answer for.
pub const OPENABLE_EXTENSIONS: &[&str] = &["csv", "tsv", "parquet"];

/// **The largest file [`open`] will attempt to read into memory before
/// composing**, measured on disk. Above it the file stays a DuckDB view over
/// `read_csv` / `read_parquet` and each statement re-reads it —
/// `a_parquet_over_the_threshold_opens_without_being_copied` holds that branch
/// open and `the_shipped_threshold_is_what_decides_an_ordinary_open` is what
/// reads this number.
///
/// 64 MiB. **This is a judgement about time, not about memory**, and the
/// distinction is the whole reason there are two constants. On-disk size does
/// not predict a table's width in memory and cannot be made to: measured on
/// this build's DuckDB, a four-column ZSTD Parquet of 123,260 bytes on disk
/// becomes a 511,031,296-byte table. Bounding memory is
/// [`MATERIALISE_BUDGET_BYTES`]'s job, and it is enforced rather than
/// estimated.
///
/// What this bounds is the reading a copy that will be refused does before it
/// is refused. The budget stops an over-large copy once the copy has grown
/// past it, so the wasted work is real but paid in file reads; declining to
/// start on a file this size keeps that off the biggest files, where it is
/// most expensive and least likely to be worth it.
///
/// The lower end is not in doubt. A dashboard over the 2,967,637-byte CSV in
/// the open-scan harness issued 46 reads of it and spent 8,199.1 ms doing so —
/// about 178 ms a read — so anything in that range is worth copying many times
/// over. The figures are the committed
/// `benchmarks/results/open-scan/2026-09-07-apple-m1-pro-from-the-file.json`.
///
/// It is deliberately not derived from the machine's free memory: a threshold
/// that moved with the host would make an open fast on one machine and slow on
/// another with nothing to read that off.
pub const MATERIALISE_UNDER_BYTES: u64 = 64 * 1024 * 1024;

/// **The memory one copy may cost.** A file whose table does not fit this is
/// not copied: the source stays a view on the file, the open costs what it
/// cost before any of this existed, and the rows served are the same rows.
///
/// 512 MiB, and unlike [`MATERIALISE_UNDER_BYTES`] this one is a bound and not
/// a guess. It is imposed on DuckDB as a `memory_limit` around the copy, with
/// spilling shut off so that exceeding it is an error instead of a write to
/// disk — [`brightfield_engine::Session::materialise_source`] holds the
/// mechanism and the measurements behind it, and
/// `a_file_whose_table_exceeds_the_budget_opens_off_the_view` drives the
/// refusal through a real open.
///
/// **Why 512 MiB rather than a number tied to the host.** It has to be far
/// enough above what an ordinary open costs that no ordinary open is refused,
/// and far enough below a laptop's memory that spending it is unremarkable.
/// The first half is measured rather than asserted: the open-scan harness
/// records the copy's footprint per shape as `materialise_bytes`, so
/// `benchmarks/results/open-scan/` says what an ordinary open of its widest
/// shape actually costs, and
/// `the_wide_shapes_table_is_far_inside_the_shipped_budget` reads that
/// footprint back out of a live open and asserts it is under a thirty-second
/// of this constant. The second half is the judgement, and it is the same one
/// the old prose here made about a threshold that could not keep it.
pub const MATERIALISE_BUDGET_BYTES: u64 = 512 * 1024 * 1024;

/// The `data:` key the chosen file is declared under, and therefore the name of
/// the DuckDB view it becomes.
///
/// A fixed name rather than one derived from the file: a view name is a SQL
/// identifier that the synthesised spec, the emitted DDL and every mark query
/// have to agree on, and deriving it from a user-chosen file name means
/// deriving it from arbitrary bytes. The user never sees this string — the
/// window is titled from the file name.
pub const SOURCE: &str = "opened";

/// Everything opening a file produced: the live session over it, the first
/// composition, the dashboard that was chosen for it, and where the generated
/// spec was written for the reader.
///
/// A struct rather than a tuple because the fourth thing is optional and the
/// third is what a caller asks questions of — which tiles, which columns were
/// left out, and why.
pub struct OpenedFile {
    /// The session holding the file as a DuckDB view.
    pub live: LiveDashboard,
    /// The **Protocol** this file opened as: one SQL step reading it, one
    /// table, and the columns the rails list under that table. The spec is in
    /// memory and unsaved — see [`crate::one_step`].
    pub protocol: crate::one_step::OneStepProtocol,
    /// The first composition over it.
    pub composed: Composed,
    /// The dashboard [`crate::dashboard::Dashboard::of`] chose: a tile per
    /// column, and the columns it declined.
    pub dashboard: Dashboard,
    /// Where the generated spec was written so the editor pane can open it —
    /// `None` when the scratch write failed, which is not a reason to refuse a
    /// file that opened.
    pub spec_file: Option<PathBuf>,
}

/// What the wait inside [`open_traced`] was spent on: the first composition's
/// clock, and how many times it read the table.
///
/// **The two terms of an open are the profile pass and the composition, and
/// they are reported apart on purpose.** A single number for the whole open
/// hides which of them moved, and the two are bounded by different things —
/// the profile pass by [`brightfield_engine::profile::SCANS_PER_SOURCE`] and
/// the composition by [`crate::pipeline::COMPOSITION_FILE_READS`].
pub struct OpenTrace {
    /// [`crate::pipeline::LiveDashboard::present`] alone, milliseconds — the
    /// term the profile pass is not.
    ///
    /// **Inflated when [`open_traced`] was asked to count**, because counting
    /// `EXPLAIN`s each statement before running it.
    pub composition_ms: f64,
    /// Leaves of DuckDB's physical plan for each statement the first
    /// composition issued. Empty when [`open_traced`] was not asked to count.
    pub composition: ScanTally,
    /// Whether the file was read into a session-scoped table before composing.
    ///
    /// `false` down two branches: a file over [`MATERIALISE_UNDER_BYTES`] on
    /// disk, where no copy was attempted, and a copy DuckDB refused because
    /// the table did not fit [`MATERIALISE_BUDGET_BYTES`]. Both compose off
    /// the view exactly as they did before this existed.
    pub materialised: bool,
    /// The one read of the file, milliseconds — the clock around the copy, and
    /// `0.0` when there was no copy to time.
    ///
    /// It is a measurement and not a placeholder in the materialised case:
    /// `a_materialised_open_reports_the_time_the_copy_took` reads it back and
    /// reddens on a hard-wired zero.
    pub materialise_ms: f64,
    /// The chosen file's size on disk, bytes — what the threshold was applied
    /// to, carried out so a reader is not re-stat-ing the file to find out
    /// which side of it this open fell.
    pub bytes: u64,
    /// **What the copy actually cost in memory**, bytes, read back out of
    /// DuckDB after the fact — see
    /// [`brightfield_engine::Session::in_memory_table_bytes`].
    ///
    /// `None` when there was no copy. This is the figure
    /// [`MATERIALISE_BUDGET_BYTES`] is denominated in, so an open reports what
    /// it spent beside what it was allowed to spend, and neither number has to
    /// be inferred from the file's size on disk.
    pub materialise_bytes: Option<u64>,
}

// ---------------------------------------------------------------------------
// What may be opened
// ---------------------------------------------------------------------------

/// The characters this build refuses in a chosen path because DuckDB's file
/// readers resolve a path as a **glob**, where each of them means something
/// other than itself: `*` and `?` are wildcards, `[…]` is a character class and
/// `{…}` is an alternation.
///
/// Wider than what this DuckDB was measured mis-resolving, deliberately. On
/// `v1.5.2` only `*`, `?` and a `[` that closes into a class were observed
/// selecting a different file; `]`, `{` and `}` resolved to themselves. They
/// are refused anyway because they are the closing halves of the same two
/// constructs, and the cost of the two errors is not symmetric — refusing a
/// name that would have worked is a sentence the user can act on, while
/// accepting one that stops working on a DuckDB bump is a wrong table nobody
/// sees. `every_character_this_duckdb_reads_as_a_pattern_is_refused` in
/// `tests/data_file.rs` asks DuckDB itself, so a bump that widens the dialect
/// past this list reddens rather than shipping.
const PATTERN_CHARACTERS: &[char] = &['*', '?', '[', ']', '{', '}'];

/// The chosen location as a path this build will open, or the words to show
/// the user for refusing it.
///
/// Five refusals, each with its own sentence, and all five happen **before**
/// the engine exists — a refusal that arrived as a DuckDB binder error would
/// name a SQL view rather than the thing the user picked.
///
/// What the last two defend is the invariant the rest of this module rests on:
/// **the path that reaches DuckDB names the file the user chose.** See the
/// module docs for the two measured ways it can stop being true.
///
/// # Errors
///
/// A URL rather than a local path; a path with no extension or an extension
/// this build does not read; a path the reader would resolve as a pattern
/// (see `PATTERN_CHARACTERS`) or one carrying a control character; or a path
/// naming something that is not a readable file.
pub fn accept(chosen: &str) -> Result<PathBuf, String> {
    if let Some(scheme) = url_scheme(chosen) {
        return Err(format!(
            "{chosen}: Brightfield opens files on this machine, and `{scheme}:` \
             is a URL. Download the file and open it from disk."
        ));
    }
    let path = PathBuf::from(chosen);
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match extension {
        Some(ext) if OPENABLE_EXTENSIONS.contains(&ext.as_str()) => {}
        Some(ext) => {
            return Err(format!(
                "{chosen}: Brightfield opens {}; this is a .{ext}.",
                openable_prose()
            ))
        }
        None => {
            return Err(format!(
                "{chosen}: Brightfield opens {}, and this path names no file \
                 type at all.",
                openable_prose()
            ))
        }
    }
    if let Some(found) = chosen.chars().find(|c| PATTERN_CHARACTERS.contains(c)) {
        return Err(format!(
            "{chosen}: Brightfield cannot open this path, because `{found}` is \
             pattern syntax to the reader underneath — it would match the name \
             rather than read it, and could open a different file. Rename the \
             file without it, or move it to a folder whose name has none."
        ));
    }
    if chosen.chars().any(char::is_control) {
        return Err(format!(
            "{}: Brightfield cannot open this path, because it contains a \
             control character — a line break or a similar invisible one — and \
             there is no way to write that into a spec without it naming a \
             different file. Rename the file without it.",
            chosen.escape_debug()
        ));
    }
    if !path.is_file() {
        return Err(format!("{chosen}: there is no file there to open."));
    }
    Ok(path)
}

/// `.csv, .tsv and .parquet files`, written from the one list so the sentence
/// cannot describe a set this build does not read.
fn openable_prose() -> String {
    let dotted: Vec<String> = OPENABLE_EXTENSIONS
        .iter()
        .map(|e| format!(".{e}"))
        .collect();
    match dotted.split_last() {
        Some((last, rest)) if !rest.is_empty() => {
            format!("{} and {last} files", rest.join(", "))
        }
        Some((last, _)) => format!("{last} files"),
        None => "no files".to_string(),
    }
}

/// The URL scheme `chosen` carries, if it carries one.
///
/// A scheme is `[A-Za-z][A-Za-z0-9+.-]*` before a `:`, per RFC 3986 — with one
/// narrowing that is not pedantry: a **single-letter** scheme is not treated as
/// one, because `C:\Users\…` is a Windows path and refusing it as a URL would
/// be a worse answer than opening it. Two letters is the shortest real scheme
/// in use, and no drive letter is two characters.
///
/// Both the `scheme://host/…` and the bare `scheme:…` forms are caught: the
/// second is how DuckDB spells a DuckLake catalog, and it reaches the network
/// exactly as the first does.
fn url_scheme(chosen: &str) -> Option<&str> {
    let colon = chosen.find(':')?;
    let scheme = &chosen[..colon];
    if scheme.len() < 2 {
        return None;
    }
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')) {
        return None;
    }
    Some(scheme)
}

// ---------------------------------------------------------------------------
// The synthesised spec
// ---------------------------------------------------------------------------

/// The root-less spec that declares `path` as a source and nothing else.
///
/// No `plot:`, so no mark and no query executes when it loads — the source's
/// DDL is a `CREATE OR REPLACE VIEW`, which for both a CSV and a Parquet reads
/// metadata rather than the table. This is what the schema is read through.
#[must_use]
pub fn source_spec(path: &Path) -> String {
    source_spec_deriving(path, &[])
}

/// The `data:` key the rows are read into when a column has to be **derived**
/// beside them, and therefore the name of the first of the two views.
///
/// A second name rather than a second `data:` block on the same one: a source
/// entry is one view, and [`SOURCE`] has to stay the name every tile reads or
/// the tiles would not share a table to cross-filter over. The test over the
/// kinds is `every_kind_reads_the_files_own_view` below, and the one over the
/// tiles is `every_tile_reads_the_one_table_however_many_columns_are_derived`
/// in [`crate::dashboard`].
pub const ROWS: &str = "opened_rows";

/// The root-less spec that declares `path` as a source, with `derived` extra
/// columns projected beside its own.
///
/// With nothing to derive this is [`source_spec`] exactly — one entry, the file
/// under [`SOURCE`] — and that is the shape a table of measures, categories and
/// dates opens as, which the test
/// `a_table_with_nothing_to_derive_declares_the_file_and_nothing_else` in
/// [`crate::dashboard`] reads off the emitted source.
///
/// With something to derive, the file is read under [`ROWS`] and [`SOURCE`]
/// becomes a `SELECT *` over it carrying the extra projections. Two views
/// rather than one because the `file:` entry is what dispatches on the
/// extension, and writing the `read_csv`/`read_parquet` call here would be a
/// second copy of `emit_file` in `brightfield-sql`.
///
/// **Every tile still reads one table**, which is the property the crossfilter
/// rests on: a clause published by one tile is applied to every other, so a
/// tile drawing a column its siblings' table lacks would take their queries
/// down with it.
#[must_use]
pub fn source_spec_deriving(path: &Path, derived: &[String]) -> String {
    let file = yaml_quoted(&path.to_string_lossy());
    if derived.is_empty() {
        return format!("data:\n  {SOURCE}:\n    file: {file}\n");
    }
    let sql = format!("SELECT *, {} FROM \"{ROWS}\"", derived.join(", "));
    format!(
        "data:\n  {ROWS}:\n    file: {file}\n  {SOURCE}: {}\n",
        yaml_quoted(&sql)
    )
}

/// `value` as a YAML single-quoted scalar.
///
/// Single-quoted rather than double-quoted because the only escape a
/// single-quoted YAML scalar has is a doubled quote — no backslashes, no
/// interpretation — so a Windows path or a column name full of punctuation
/// survives verbatim.
///
/// What single-quoting cannot carry is a **line break**: YAML folds one inside
/// a quoted scalar to a space, so a value holding one would parse back as a
/// different string. Two guards keep such a value from reaching here, and this
/// function is correct only because both do — `accept`'s control-character
/// refusal for a path, and `chart_kinds::fields_of`'s name check for a column,
/// which is what stops an unwritable name reaching a chart kind's builder.
fn yaml_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// What the window is titled for an opened file: its file name, which is what
/// the person who picked it was looking at — not the absolute path they never
/// typed.
pub(crate) fn file_label(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |n| n.to_string_lossy().into_owned(),
    )
}

// ---------------------------------------------------------------------------
// Opening
// ---------------------------------------------------------------------------

/// Open `chosen` as a live table: the session holding the file as a DuckDB
/// view, the first composition over it, **the dashboard that was chosen for
/// it** — a tile per column — and the file the generated spec was written to.
///
/// The dashboard is [`crate::dashboard::Dashboard::of`]'s, which is where the
/// per-column choice and the layout live. This function's own job is the two
/// loads either side of it: the root-less one that answers what the columns
/// are, and the real one over the spec that answer let it write.
///
/// # Errors
///
/// Every failure carries the chosen path and the reason, because the window
/// raises this string as a banner and a banner that says only "could not open"
/// is a blank frame with a sentence on it. The reasons are: the location was
/// refused (see `accept`); DuckDB would not read the file, in which case its
/// own words come through; or no column in the table admits a tile — and then
/// the sentence names each column and why, because "nothing to draw" about a
/// file the user can see the contents of is not an answer.
pub fn open(chosen: &str) -> Result<OpenedFile, String> {
    open_traced(chosen, &OpenOptions::default()).map(|(opened, _)| opened)
}

/// What [`open_traced`] may do differently from [`open`].
///
/// [`OpenOptions::default`] **is** what [`open`] does, so a caller that wants
/// the app's own open and a trace of it has no field to set.
#[derive(Debug, Clone, Copy)]
pub struct OpenOptions {
    /// Ask DuckDB to explain each statement the first composition issues
    /// before running it, and count the leaves of those plans into
    /// [`OpenTrace::composition`].
    ///
    /// Counting roughly doubles the statements issued and so inflates
    /// [`OpenTrace::composition_ms`]. Take the clock from a run with this off
    /// and the count from a run with it on, as
    /// [`brightfield_engine::Session::profile_sources_counting_scans`] and the
    /// harness's `time_profile` already do for the profile pass.
    pub count_scans: bool,
    /// The largest size on disk a file may have for a copy to be attempted —
    /// [`MATERIALISE_UNDER_BYTES`] by default. A file of exactly this many
    /// bytes is still copied, which is the `<=` in [`open_traced`]; the
    /// constant's name reads as the rule rather than as the boundary.
    ///
    /// **It is a parameter and not only a constant so that the other branch
    /// is reachable.** A file over the threshold composes off the view, one
    /// read per statement; a test that could not ask for that would be left
    /// asserting in prose that it still works. Passing `0` takes it on any
    /// file.
    pub materialise_under_bytes: u64,
    /// The memory one copy may cost — [`MATERIALISE_BUDGET_BYTES`] by
    /// default, and enforced by DuckDB rather than estimated here.
    ///
    /// A parameter for the same reason as the field above and a sharper one:
    /// the refusal branch is the one the larger-than-memory claim rests on,
    /// and a fixture big enough to exceed the shipped 512 MiB would make the
    /// suite pay half a gigabyte to find out. A small budget and a small
    /// fixture drive the same code.
    ///
    /// Values under [`brightfield_engine::MATERIALISE_BUDGET_FLOOR_BYTES`] are
    /// raised to it, because a small enough `memory_limit` stops DuckDB being
    /// able to lift it again.
    pub materialise_budget_bytes: u64,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            count_scans: false,
            materialise_under_bytes: MATERIALISE_UNDER_BYTES,
            materialise_budget_bytes: MATERIALISE_BUDGET_BYTES,
        }
    }
}

/// [`open`], and what the wait inside it was spent on.
///
/// One route, not a second one — this **is** [`open`], and [`open`] is this
/// under [`OpenOptions::default`]. A harness that reproduced the open
/// sequence to time it would be timing a sequence the app does not perform,
/// which is the way an open-cost measurement goes quietly wrong.
///
/// # Errors
///
/// As [`open`].
pub fn open_traced(chosen: &str, options: &OpenOptions) -> Result<(OpenedFile, OpenTrace), String> {
    let path = accept(chosen)?;
    let columns = columns_of(&path)?;
    let dashboard = Dashboard::of(&path, &columns);
    if dashboard.tiles().is_empty() {
        let left: Vec<String> = dashboard
            .omitted()
            .iter()
            .map(|o| format!("{}: {}", o.column, o.because))
            .collect();
        return Err(format!(
            "{}: opened, but there is nothing here to draw — no column in it \
             admits a picture this build can make. {}",
            path.display(),
            if left.is_empty() {
                "It declares no columns at all.".to_string()
            } else {
                left.join("; ")
            }
        ));
    }
    let spec = dashboard.to_spec();
    let mut live =
        LiveDashboard::load_str(&spec, None).map_err(|e| format!("{}: {e}", path.display()))?;
    // The one read. What the composition issues after this scans a table, so
    // the tile count stops multiplying the cost of a read — see the module
    // header for the measurement that decided it.
    //
    // TWO conditions, in two units, and they are not interchangeable.
    // `materialise_under_bytes` is on disk and decides whether the copy is
    // worth attempting; `materialise_budget_bytes` is the memory the copy may
    // cost and is enforced inside `materialise_source`, which returns `Err`
    // when the table does not fit it.
    //
    // A file DuckDB will not copy is not a file that failed to open: the view
    // is still there and still correct, so a refusal here costs the old slow
    // open and nothing else. That is the branch a table larger than the budget
    // takes, and `a_file_whose_table_exceeds_the_budget_opens_off_the_view`
    // drives it.
    //
    // A path that cannot be stat-ed reads as `u64::MAX`, which is over any
    // threshold, so an unreadable size declines the copy rather than assuming
    // one is safe.
    let bytes = std::fs::metadata(&path)
        .map(|m| m.len())
        .unwrap_or(u64::MAX);
    let mut materialised = false;
    let mut materialise_ms = 0.0;
    let mut materialise_bytes = None;
    if bytes <= options.materialise_under_bytes {
        let at = std::time::Instant::now();
        match live.materialise_source(SOURCE, options.materialise_budget_bytes) {
            Ok(()) => {
                materialised = true;
                materialise_ms = at.elapsed().as_secs_f64() * 1000.0;
                materialise_bytes = live.coordinator().session().in_memory_table_bytes();
            }
            Err(e) => eprintln!(
                "warning: {}: composing from the file directly, because it \
                 could not be read into memory within {} bytes: {e}",
                path.display(),
                options.materialise_budget_bytes
            ),
        }
    }

    let at = std::time::Instant::now();
    let (composed, composition) = if options.count_scans {
        live.present_counting_scans()
    } else {
        live.present().map(|c| (c, ScanTally::default()))
    }
    .map_err(|e| format!("{}: {e}", path.display()))?;
    let composition_ms = at.elapsed().as_secs_f64() * 1000.0;
    let spec_file = write_spec_file(&path, &spec);
    // The other document. Built from the same profile the dashboard was chosen
    // from, so the columns the rails list and the columns the tiles draw are
    // one walk over one table rather than two that could disagree.
    let protocol = crate::one_step::OneStepProtocol::of(&path, &columns, &dashboard);
    Ok((
        OpenedFile {
            live,
            composed,
            dashboard,
            spec_file,
            protocol,
        },
        OpenTrace {
            composition_ms,
            composition,
            materialised,
            materialise_ms,
            bytes,
            materialise_bytes,
        },
    ))
}

/// The directory generated specs are written to, per process.
///
/// The temp directory rather than beside the data file: opening a file to look
/// at it must not write into the folder it came from. Per process rather than a
/// fixed name, so two runs cannot edit each other's copy and nothing inherits a
/// directory another user created.
///
/// Both of those separate one run from another run, and the specs from the
/// data. The third axis is inside a single run and neither reaches it: one
/// process opens several data files, and two of them can be named the same. It
/// is [`spec_scratch_path`] that keys those apart — this directory is where it
/// puts what it names, and is deliberately a part of the key rather than the
/// whole of it.
fn spec_scratch_dir() -> PathBuf {
    std::env::temp_dir().join(format!("brightfield-generated-{}", std::process::id()))
}

/// Where `data`'s generated spec is written: one path per data file, and the
/// same path each time that file is opened, however the caller spelled it.
///
/// **Keyed by the file's whole path rather than by its name.** A stem is not an
/// identity — an analyst comparing two months opens `jan/readings.csv` and then
/// `feb/readings.csv`, and under a name-keyed path the second write landed on
/// the first file's spec and replaced it with no warning, leaving one document
/// drawing January's chart beside February's source. The digest separates two
/// files a stem cannot, and the pair is held apart by
/// `two_data_files_with_one_name_keep_their_own_generated_specs` in
/// `crates/brightfield-shell/tests/data_file.rs`.
///
/// A **digest rather than a counter**, because re-opening one file has to land
/// back on its own spec instead of accumulating a second copy beside it — see
/// `a_scratch_path_is_per_data_file_and_stable_across_re_opens` below. It is
/// taken over the canonicalized path, so two spellings of one file are one key
/// — `two_spellings_of_one_data_file_are_one_scratch_path` below; over the path
/// as given when canonicalizing fails, which is what a file that has gone
/// missing since it was opened gets.
///
/// The digest names the **directory** and the resolved path's own stem names
/// the file — the same canonicalized path the digest is taken over, not the
/// caller's typed path, so a file opened under two spellings that a
/// case-folding filesystem treats as one lands on one filename too, not two
/// — `two_case_spellings_of_one_data_file_produce_one_generated_spec_file`
/// below. The editor pane's tab and header title itself from that file name —
/// see `describe` in [`crate::editor`] — so a reader sees `readings.yaml`
/// there, not a hex string; the breadcrumb ahead of it names the digest
/// directory too, and that crumb is the hex string.
fn spec_scratch_path(data: &Path) -> PathBuf {
    let resolved = std::fs::canonicalize(data).unwrap_or_else(|_| data.to_path_buf());
    let mut hasher = DefaultHasher::new();
    resolved.hash(&mut hasher);
    let stem = resolved.file_stem().unwrap_or(resolved.as_os_str());
    spec_scratch_dir()
        .join(format!("{:016x}", hasher.finish()))
        .join(stem)
        .with_extension("yaml")
}

/// Write `spec` where the editor pane can open it, and hand back the path.
///
/// **The bytes are the bytes that composed the picture**, not a re-rendering of
/// them — which is what makes the file evidence rather than documentation: a
/// reader comparing the chart to the spec is comparing it to what ran.
///
/// `None` on any failure. A scratch write that did not land is a spec the reader
/// cannot open; it is not a reason to refuse a file that opened, so the caller
/// carries on without one.
fn write_spec_file(data: &Path, spec: &str) -> Option<PathBuf> {
    let path = spec_scratch_path(data);
    std::fs::create_dir_all(path.parent()?).ok()?;
    std::fs::write(&path, spec).ok()?;
    Some(path)
}

/// The profiled columns of `path`, read through a root-less spec.
///
/// # Errors
///
/// The engine's own words, prefixed with the path — this is where a file that
/// is not really a Parquet, or a CSV whose rows do not line up, is caught, and
/// DuckDB's message is the whole of what the reader needs.
fn columns_of(path: &Path) -> Result<Vec<ColumnProfile>, String> {
    let spec = source_spec(path);
    let parsed = parse_spec(&spec, Format::Yaml)
        .map_err(|e| format!("{}: parse error: {e}", path.display()))?;
    let analysis = analyse_spec(&parsed.spec)
        .map_err(|e| format!("{}: analysis error: {e}", path.display()))?;
    let load = Engine::new()
        .load_spec_with(parsed.spec, analysis, None, &LoadOptions::packaged())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let profile = load
        .session
        .profile_sources()
        .into_iter()
        .find(|p| p.name == SOURCE)
        .ok_or_else(|| {
            format!(
                "{}: the engine loaded the file but reported no source to \
                 profile.",
                path.display()
            )
        })?;
    match profile.outcome {
        ProfileOutcome::Profiled { columns, .. } => Ok(columns),
        ProfileOutcome::Failed(reason) => Err(format!("{}: {reason}", path.display())),
        ProfileOutcome::Unsupported => Err(format!(
            "{}: this build cannot read the schema of that source.",
            path.display()
        )),
    }
}

/// Ask the operating system for a data file, blocking until the user chooses
/// or cancels.
///
/// The one call in this crate that opens a window nobody laid out, and the one
/// thing here no headless test drives — everything a test needs to know about
/// opening a file is decided by `accept`, `open` and the dashboard `open`
/// chooses, none of which needs a dialog. The dialog's own job is to return a
/// path.
///
/// Filtered to the extensions `accept` will take, so the common refusal never
/// has to be shown: a picker that offers a `.txt` and then declines it is a
/// picker arguing with itself.
#[must_use]
pub fn pick() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open a data file")
        .add_filter("Data files", OPENABLE_EXTENSIONS)
        .pick_file()
}

// ---------------------------------------------------------------------------
// Unit tests — the pure decisions. The engine-backed path is exercised in
// `tests/data_file.rs` against real files on disk.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_workbench::registry::Field;

    /// Every URL form a path box realistically receives is refused, and the
    /// refusal says the word — what this defends is that the box opens a file
    /// on this machine, so "it happens to fail later" is not the same answer.
    #[test]
    fn a_url_is_refused_by_scheme_not_by_hostname() {
        for url in [
            "https://openlake.meridian.online/edgar_gleif.parquet",
            "http://example.com/data.csv",
            "s3://bucket/key.parquet",
            "gs://bucket/key.parquet",
            "ducklake:https://example.com/catalog.ducklake",
            "file:///Users/someone/data.csv",
        ] {
            let refusal = accept(url).expect_err("a URL is not a local file");
            assert!(
                refusal.contains("URL"),
                "the refusal of {url} has to say what it refused: {refusal}"
            );
            assert!(
                refusal.contains(url),
                "the refusal of {url} has to name it: {refusal}"
            );
        }
    }

    /// …and a Windows drive letter is not a URL scheme. The single-letter
    /// narrowing is the whole of what stops `C:\data.csv` reading as one.
    #[test]
    fn a_drive_letter_is_not_a_scheme() {
        assert_eq!(url_scheme("C:\\Users\\hugh\\data.csv"), None);
        assert_eq!(url_scheme("/Users/hugh/data.csv"), None);
        assert_eq!(url_scheme("data.csv"), None);
        assert_eq!(url_scheme("https://x/y.csv"), Some("https"));
        assert_eq!(url_scheme("ducklake:x"), Some("ducklake"));
    }

    /// An extension this build does not read is refused by name, and the
    /// sentence lists what it does read — written from the one list, so it
    /// cannot describe a set that does not exist.
    #[test]
    fn an_unreadable_extension_is_refused_with_the_list_it_is_not_on() {
        let refusal = accept("/tmp/notes.txt").expect_err(".txt is not openable");
        assert!(refusal.contains(".txt"), "{refusal}");
        for ext in OPENABLE_EXTENSIONS {
            assert!(
                refusal.contains(&format!(".{ext}")),
                "the refusal has to name .{ext} as openable: {refusal}"
            );
        }
    }

    /// A path the reader would resolve as a pattern is refused by name and by
    /// character, and the refusal happens without touching the disk — these
    /// paths are not written anywhere, so a check that needed the file to exist
    /// would be gating the wrong thing.
    ///
    /// The sentence has to carry the character, because "rename it" is useless
    /// advice when the user cannot see which byte is the problem.
    #[test]
    fn a_path_the_reader_would_take_as_a_pattern_is_refused_by_character() {
        for (path, offender) in [
            ("/tmp/sales[1].csv", '['),
            ("/tmp/sales].csv", ']'),
            ("/tmp/sales*.csv", '*'),
            ("/tmp/sales?.csv", '?'),
            ("/tmp/sales{a,b}.csv", '{'),
            ("/tmp/sales}.csv", '}'),
            // A folder is resolved by the same glob the file name is.
            ("/tmp/quarter[1]/sales.csv", '['),
        ] {
            let refusal = accept(path).unwrap_err_or_else_message(path);
            assert!(
                refusal.contains(path),
                "the refusal must name it: {refusal}"
            );
            assert!(
                refusal.contains(offender),
                "the refusal of {path} must name `{offender}`: {refusal}"
            );
            assert!(
                refusal.contains("pattern"),
                "the refusal of {path} must say why: {refusal}"
            );
        }
    }

    /// `expect_err` with a sentence naming the path, so a regression reads as
    /// "this path opened" rather than as an `unwrap` backtrace.
    trait RefusalExt {
        fn unwrap_err_or_else_message(self, path: &str) -> String;
    }

    impl RefusalExt for Result<PathBuf, String> {
        fn unwrap_err_or_else_message(self, path: &str) -> String {
            match self {
                Err(message) => message,
                Ok(accepted) => panic!(
                    "{path} is pattern syntax to the reader underneath and must \
                     not be accepted — it was, as {}",
                    accepted.display()
                ),
            }
        }
    }

    /// …and a path carrying a control character is refused too, with the
    /// character shown escaped rather than raw — a banner holding a real line
    /// break is a banner that does not read as one line.
    #[test]
    fn a_path_with_a_control_character_is_refused_and_shown_escaped() {
        let refusal = accept("/tmp/sales\n2026.csv").expect_err("a line break is not openable");
        assert!(
            refusal.contains("/tmp/sales\\n2026.csv"),
            "the refusal names the path with the control character escaped: {refusal}"
        );
        assert!(
            !refusal.contains('\n'),
            "…and holds no raw line break of its own: {refusal:?}"
        );
        assert!(refusal.contains("control character"), "{refusal}");
        assert!(accept("/tmp/sales\r2026.csv").is_err());
        assert!(accept("/tmp/sales\t2026.csv").is_err());
    }

    /// An ordinary path with punctuation that means nothing to either language
    /// is NOT refused by these two checks — they are narrow on purpose, and a
    /// guard that turned away `Hugh's data (final).csv` would be a worse answer
    /// than the defect it prevents.
    ///
    /// Asserted by the message, because a path that does not exist is refused
    /// by `is_file` at the end regardless: what is gated is *which* sentence.
    #[test]
    fn ordinary_punctuation_is_not_mistaken_for_a_pattern() {
        for path in [
            "/tmp/Hugh's data (final).csv",
            "/tmp/sales+2026.csv",
            "/tmp/sales@2026 #2.csv",
            "/tmp/100% of sales, £ & $.csv",
            "C:\\Users\\hugh\\sales.csv",
        ] {
            let refusal = accept(path).expect_err("none of these exist on disk");
            assert!(
                refusal.contains("there is no file there to open"),
                "{path} must reach the last refusal, not a pattern one: {refusal}"
            );
        }
    }

    /// **Every kind the registry ships** parses under this module's header and
    /// names the file's own view as its source — not the two that happened to
    /// exist when this was written.
    ///
    /// Stated over the registry rather than over a list of shapes because that
    /// is the only form of the claim a kind added later has to satisfy: the
    /// Data pane tabulates `SELECT * FROM <the first mark's source>`, so a kind
    /// pointing its mark at a rolled-up view would put summary rows in the grid
    /// where the user is entitled to their file, and no assertion written
    /// against `rectY` and `cell` would notice.
    #[test]
    fn every_kind_reads_the_files_own_view() {
        let path = PathBuf::from("/tmp/t.parquet");
        for kind in crate::chart_kinds::registry().kinds() {
            let fields: Vec<Field> = kind
                .slots
                .iter()
                .filter(|slot| slot.required)
                .enumerate()
                .map(|(i, slot)| Field::new(format!("c{i}"), slot.accepts[0]))
                .collect();
            let binding = kind.bind(&fields).expect("a kind binds its own slots");
            let block = kind
                .spec(&binding, &kind.options())
                .expect("a kind builds from its own binding");
            let spec = format!("{}{block}", source_spec(&path));
            let parsed = parse_spec(&spec, Format::Yaml)
                .unwrap_or_else(|e| panic!("{}: does not parse: {e}\n{spec}", kind.id));
            let marks = brightfield_sql::collect_marks(&parsed.spec);
            assert!(!marks.is_empty(), "{}: declares no mark", kind.id);
            for mark in marks {
                let from = format!("{:?}", mark.data);
                assert!(
                    from.contains(SOURCE),
                    "{}: a mark must read the file's own view, not a rolled-up \
                     one: {from}",
                    kind.id
                );
            }
        }
    }

    /// **The spec the reader opens is the spec that ran**, byte for byte.
    ///
    /// The whole of what makes a generated dashboard inspectable rather than
    /// opaque is that the file is a copy of the source the picture was composed
    /// from. A writer that re-rendered the spec — reordered a key, dropped the
    /// comments that say why each tile is the tile it is — would produce a
    /// document that reads plausibly and is not evidence of anything.
    #[test]
    fn the_written_spec_is_the_source_that_composed_the_picture() {
        let data = PathBuf::from("/tmp/bf-written-spec/readings.csv");
        let spec = "meta:\n  title: 'readings.csv'\n# why this tile\nplot: []\n";
        let written = write_spec_file(&data, spec).expect("the scratch write lands");
        assert_eq!(
            std::fs::read_to_string(&written).expect("the file reads back"),
            spec
        );
        assert_eq!(
            written.extension().and_then(|e| e.to_str()),
            Some("yaml"),
            "the editor opens it as a spec, so it has to be named like one: {}",
            written.display()
        );
        assert!(
            written.starts_with(std::env::temp_dir()),
            "a file opened to be looked at must not write beside itself: {}",
            written.display()
        );
        std::fs::remove_file(&written).ok();
    }

    /// **A data file's scratch path is its own**, and it is the same path each
    /// time that file is opened.
    ///
    /// The two halves of one choice, so one test: keying on the whole path is
    /// what gives two files a stem cannot tell apart two specs, and keying on
    /// something stable is what stops re-opening one file leaving a second copy
    /// beside the first. A counter would buy the first and lose the second.
    ///
    /// Neither path exists on disk, which is the point — this is the branch
    /// where canonicalizing fails and the path as given is the key.
    #[test]
    fn a_scratch_path_is_per_data_file_and_stable_across_re_opens() {
        let jan = PathBuf::from("/tmp/bf-scratch-key/jan/readings.csv");
        let feb = PathBuf::from("/tmp/bf-scratch-key/feb/readings.csv");
        assert_ne!(
            spec_scratch_path(&jan),
            spec_scratch_path(&feb),
            "two data files with one name share a generated spec, so the \
             second to be opened overwrites the first"
        );
        assert_eq!(
            spec_scratch_path(&jan),
            spec_scratch_path(&jan),
            "re-opening one data file writes a second copy of its spec instead \
             of rewriting the one it already has"
        );
        assert_eq!(
            spec_scratch_path(&jan).file_name().and_then(|n| n.to_str()),
            Some("readings.yaml"),
            "the editor pane titles itself from the file name, so the name has \
             to stay the data file's own: {}",
            spec_scratch_path(&jan).display()
        );
        assert!(
            spec_scratch_path(&jan).starts_with(spec_scratch_dir()),
            "a generated spec belongs to this process's scratch directory: {}",
            spec_scratch_path(&jan).display()
        );
    }

    /// **Two spellings of one data file are one scratch path.** The key is the
    /// file, not the string the file arrived as.
    ///
    /// A real file, because this is the branch `std::fs::canonicalize`
    /// succeeds on: the detour through `..` is a different string that names
    /// the same file, and a reader who opened it twice should find their spec
    /// where they left it rather than in a second place.
    #[test]
    fn two_spellings_of_one_data_file_are_one_scratch_path() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "bf-scratch-spelling-{}-{nanos}",
            std::process::id()
        ));
        let inner = dir.join("jan");
        std::fs::create_dir_all(&inner).expect("a temp directory for the fixture");
        let direct = inner.join("readings.csv");
        std::fs::write(&direct, "region,reading\nnorth,12\n").expect("the fixture writes");
        let detour = inner.join("..").join("jan").join("readings.csv");

        assert_eq!(
            spec_scratch_path(&direct),
            spec_scratch_path(&detour),
            "one file named two ways got two generated specs: {} and {}",
            direct.display(),
            detour.display()
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// **Two case spellings of one on-disk data file are one generated spec
    /// file, not two.** The regression this closes: the digest directory was
    /// keyed on the canonicalized path, so it was already shared across
    /// spellings, but the filename was taken from the caller's typed path, so
    /// opening `readings.csv` and later `READINGS.CSV` for the same file wrote
    /// `readings.yaml` beside `READINGS.yaml` in that one directory.
    ///
    /// macOS only — this needs a case-folding filesystem to mean anything.
    /// That holds by default on APFS, which is what `test.yml`'s `macos-15`
    /// runner gives it. In-repo precedent for a filesystem-conditioned test
    /// rather than an unconditional one: `spec_save.rs:399`, `:420`,
    /// `protocol_degrade_channel.rs:28`, `mem.rs:98`/`:103`.
    #[cfg(target_os = "macos")]
    #[test]
    fn two_case_spellings_of_one_data_file_produce_one_generated_spec_file() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "bf-scratch-case-fold-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        let lower = dir.join("readings.csv");
        std::fs::write(&lower, "region,reading\nnorth,12\n").expect("the fixture writes");
        let upper = dir.join("READINGS.CSV");
        assert!(
            std::fs::canonicalize(&upper).is_ok(),
            "this filesystem does not fold case — {} does not resolve",
            upper.display()
        );

        let lower_generated = spec_scratch_path(&lower);
        let upper_generated = spec_scratch_path(&upper);

        assert_eq!(
            lower_generated,
            upper_generated,
            "one on-disk file opened as {} and as {} produced two generated \
             specs, {} and {}, instead of one",
            lower.display(),
            upper.display(),
            lower_generated.display(),
            upper_generated.display()
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
