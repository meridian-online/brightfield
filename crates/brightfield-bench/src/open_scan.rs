//! What opening a data file costs: how many times it is read, and how long
//! the wait is.
//!
//! # Why this is measured separately from the interaction baseline
//!
//! Everything else in this harness times a gesture on a table that is already
//! open. This times the wait before the first picture — the one an analyst
//! meets first — and it is dominated by a different thing. A `file:` source is
//! a DuckDB view over `read_csv`, so **a statement issued over it reads and
//! re-parses the file**; the cost of an open is therefore a count of
//! statements times the cost of one read, and the count is what this module
//! reports.
//!
//! # The two numbers
//!
//! **The scan count** is what
//! [`Session::profile_sources_counting_scans`][counting] reports: the leaves of
//! DuckDB's physical plan for each statement the profile pass issues, summed.
//! It does not vary with the row count, so a test can read it off a small
//! fixture in a second and hold it to
//! [`brightfield_engine::profile::SCANS_PER_SOURCE`].
//!
//! **The wall time** is the whole of `brightfield_shell::data_file::open` —
//! the profile pass, the dashboard the profile chooses, and the first
//! composition over it — and it needs the full row count to be worth reading.
//!
//! [counting]: brightfield_engine::Session::profile_sources_counting_scans

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use brightfield_engine::{profile, Engine, LoadOptions, ProfileOutcome, ScanTally};
use brightfield_shell::data_file;
use brightfield_shell::pipeline;
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::{parse_spec, Format};

use crate::stats::Stats;

/// A table to open: how many rows, and how the columns divide by type.
///
/// The types are separated because the profile pass treats them differently —
/// only a numeric column carries moments and therefore a distribution, so a
/// numeric column is the one that used to add a read.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Shape {
    /// The fixture's name, and the stem of the CSV written for it.
    pub name: &'static str,
    /// Rows in the file.
    pub rows: u64,
    /// Numeric columns. The first takes [`BOUNDED_DISTINCT`] distinct values
    /// and no more, so both branches of the distribution are counted —
    /// `the_fixture_carries_a_bounded_column_and_a_wide_one` reads that back.
    pub numeric: usize,
    /// VARCHAR columns.
    pub text: usize,
    /// TIMESTAMP columns. These carry bounds and no moments, so they are the
    /// case that must NOT add a distribution.
    pub timestamps: usize,
}

impl Shape {
    /// Columns in the file.
    #[must_use]
    pub fn columns(&self) -> usize {
        self.numeric + self.text + self.timestamps
    }
}

/// A narrow table: two numeric columns, one of them bounded, and a label.
pub const NARROW: Shape = Shape {
    name: "narrow",
    rows: 14_133,
    numeric: 2,
    text: 1,
    timestamps: 0,
};

/// The wide table, at the row and column count of the file that motivated
/// this measurement: fourteen thousand rows and twenty-two columns.
///
/// **The row and column counts are the motivating file's; the type split is
/// not, and nothing here can check that it is.** Fourteen numeric, six text
/// and two temporal is a plausible split for a public measurement feed and it
/// is chosen for what it exercises: the numeric count is what the distribution
/// pass scales with, and the two temporal columns are the case that carries
/// bounds and must add no distribution at all. The fixture's size on disk is
/// not stated here either — [`Measured::bytes`] carries what the generator
/// actually wrote, so the record answers it and this sentence cannot go stale
/// against it.
pub const WIDE: Shape = Shape {
    name: "wide",
    rows: 14_133,
    numeric: 14,
    text: 6,
    timestamps: 2,
};

/// The shapes the harness reports, in report order.
pub const SHAPES: &[Shape] = &[NARROW, WIDE];

/// How many distinct values the bounded numeric column takes.
///
/// Under [`profile::VALUE_BAR_LIMIT`] on purpose: the distribution has two
/// branches, and `the_fixture_carries_a_bounded_column_and_a_wide_one` reads
/// back that this fixture reaches both of them. Both branches ride the same
/// statement now, which is why one that reached the binned branch alone would
/// leave half of it unexercised.
const BOUNDED_DISTINCT: u64 = 12;

/// One statement the profile pass issued, as it goes into the record.
#[derive(Debug, Clone, Serialize)]
pub struct StatementRecord {
    /// The statement's leaf count, or `null` where DuckDB declined to explain
    /// it.
    pub scans: Option<u32>,
    /// The statement, truncated for the record — the shape is the evidence,
    /// and a wide table's aggregate SELECT is thousands of characters of it.
    pub sql: String,
}

/// How far into a statement the record keeps.
const SQL_RECORDED: usize = 160;

/// One shape, opened and measured.
#[derive(Debug, Clone, Serialize)]
pub struct Measured {
    /// The shape this row is for.
    pub shape: Shape,
    /// Columns in the file.
    pub columns: usize,
    /// The CSV's size on disk, bytes.
    pub bytes: u64,
    /// Scan leaves summed over every statement the profile pass issued, or
    /// `null` where any one of them went unexplained.
    pub scans: Option<u32>,
    /// The bound `scans` is held to — [`profile::SCANS_PER_SOURCE`], carried
    /// into the record so a reader is not comparing against a number they have
    /// to go and look up.
    pub scan_bound: u32,
    /// Statements the pass issued, in order.
    pub statements: Vec<StatementRecord>,
    /// `Session::profile_sources` alone, milliseconds.
    pub profile: Option<Stats>,
    /// The whole of `data_file::open`, milliseconds.
    pub open: Option<Stats>,
    /// Tiles the dashboard chose for the file.
    pub tiles: usize,
    /// DuckDB executes the first composition performed — the queries marks
    /// are drawn from, which is **not** the number of times the table was
    /// read. See [`Measured::composition_scans`].
    pub composition_queries: usize,
    /// Scan leaves summed over every statement the first composition issued,
    /// or `null` where any one of them went unexplained.
    ///
    /// The counterpart of [`Measured::scans`] for the other term of the wait,
    /// taken through the same `EXPLAIN` and counted the same way. It is the
    /// larger of the two numbers on this row and the one the wait tracks: a
    /// mark query is one execute and its plan may read the table more than
    /// once, and the queries the composition issues *beside* the marks — the
    /// status band's two counts, the sample facts — are not executes at all.
    pub composition_scans: Option<u32>,
    /// Of those leaves, how many read the **file** rather than a relation the
    /// session holds in memory.
    ///
    /// **This is the number the claim about opening a data file is made in,
    /// and it is the one held to a bound.** A leaf is a leaf whatever it
    /// scans, and after [`brightfield_shell::data_file::open`] has read the
    /// file into a session-scoped table the composition's leaves are scans of
    /// that table — the same count, three orders of magnitude apart in cost.
    pub composition_file_reads: Option<u32>,
    /// The bound `composition_file_reads` is held to —
    /// [`brightfield_shell::pipeline::COMPOSITION_FILE_READS`], carried into
    /// the record so a reader is not comparing against a number they have to
    /// go and look up.
    pub composition_file_read_bound: u32,
    /// Whether the file was read into memory before composing — `false` above
    /// [`brightfield_shell::data_file::MATERIALISE_UNDER_BYTES`] on disk and
    /// `false` again where the copy did not fit
    /// [`brightfield_shell::data_file::MATERIALISE_BUDGET_BYTES`], and then
    /// `composition_file_reads` is under no bound.
    pub materialised: bool,
    /// The one read of the file, milliseconds — the term
    /// [`Measured::composition`] no longer carries. `null` when the file was
    /// not materialised.
    pub materialise: Option<Stats>,
    /// **What the copy cost in memory**, bytes, as DuckDB accounts for it —
    /// the figure the budget is denominated in, so this record says what an
    /// ordinary open of this shape spends rather than leaving it to be
    /// inferred from `bytes`, which is the file on disk and a different unit.
    /// `null` when the file was not materialised, or where DuckDB declined the
    /// question.
    pub materialise_bytes: Option<u64>,
    /// Statements the first composition issued, in order.
    pub composition_statements: Vec<StatementRecord>,
    /// `LiveDashboard::present` alone, milliseconds — the composition's own
    /// clock, taken from the same uncounted open [`Measured::open`] is.
    pub composition: Option<Stats>,
}

/// Write (or reuse) the CSV for `shape` under `dir`.
///
/// Each column is a pure function of the row index — the numeric and text
/// ones through DuckDB's `hash()`, as the interaction harness's datasets are,
/// and the temporal ones as the index counted in seconds from a fixed
/// timestamp. So a present file IS the fixture, and regeneration could only
/// reproduce it.
///
/// # Errors
///
/// The directory could not be created, or DuckDB would not write the file.
pub fn ensure_csv(conn: &duckdb::Connection, dir: &Path, shape: &Shape) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let path = dir.join(format!(
        "open_{}_{}x{}.csv",
        shape.name,
        shape.rows,
        shape.columns()
    ));
    if path.exists() {
        return Ok(path);
    }
    let mut selects: Vec<String> = Vec::with_capacity(shape.columns());
    for c in 0..shape.numeric {
        if c == 0 {
            // The bounded column: few enough distinct values to take the
            // per-value branch.
            selects.push(format!(
                "(hash(i * 11 + 7) % {BOUNDED_DISTINCT})::DOUBLE AS measure_{c}"
            ));
        } else {
            selects.push(format!(
                "((hash(i * {} + {c}) % 1000000) / 1000.0)::DOUBLE AS measure_{c}",
                c * 7 + 3
            ));
        }
    }
    for c in 0..shape.text {
        selects.push(format!(
            "('label{c}_' || (hash(i * {} + {c}) % 500))::VARCHAR AS label_{c}",
            c * 13 + 5
        ));
    }
    for c in 0..shape.timestamps {
        selects.push(format!(
            "(TIMESTAMP '2020-01-01' + INTERVAL (i + {c}) SECOND) AS at_{c}"
        ));
    }
    let sql = format!(
        "COPY (SELECT {} FROM range({}) AS r(i)) TO '{}' (FORMAT CSV, HEADER)",
        selects.join(", "),
        shape.rows,
        path.display()
    );
    conn.execute_batch(&sql)
        .map_err(|e| format!("generate {}: {e}", path.display()))?;
    Ok(path)
}

/// The open this run is measuring: the app's own, or the one a file too large
/// to copy takes.
///
/// **The second is here so the before-and-after is one binary run twice.**
/// Reading the file into memory is what the composition figures are about, and
/// comparing them against a number measured on another day, on another build,
/// under another machine's load is how a speed-up gets claimed that the change
/// did not produce. `--open-scan-no-materialise` measures the other branch,
/// minutes apart, on the same machine.
fn options(materialise: bool) -> data_file::OpenOptions {
    if materialise {
        data_file::OpenOptions::default()
    } else {
        data_file::OpenOptions {
            materialise_under_bytes: 0,
            ..data_file::OpenOptions::default()
        }
    }
}

/// The scan tally for one file, and the columns the profile found.
///
/// Runs the profile pass with counting on, which asks DuckDB to explain each
/// statement before issuing it. Timing anything from this run would be timing
/// the explaining, which is why the timed runs below are separate.
fn tally(path: &Path) -> Result<(ScanTally, usize), String> {
    let spec = data_file::source_spec(path);
    let parsed = parse_spec(&spec, Format::Yaml).map_err(|e| format!("parse: {e}"))?;
    let analysis = analyse_spec(&parsed.spec).map_err(|e| format!("analysis: {e}"))?;
    let load = Engine::new()
        .load_spec_with(parsed.spec, analysis, None, &LoadOptions::packaged())
        .map_err(|e| format!("load: {e}"))?;
    let (profiles, tally) = load.session.profile_sources_counting_scans();
    let columns = profiles
        .iter()
        .find(|p| p.name == data_file::SOURCE)
        .map_or(0, |p| match &p.outcome {
            ProfileOutcome::Profiled { columns, .. } => columns.len(),
            _ => 0,
        });
    Ok((tally, columns))
}

/// One timed `Session::profile_sources`, over its own freshly loaded session.
///
/// A new session per sample because the point is what an open costs, and an
/// open builds one.
fn time_profile(path: &Path) -> Result<f64, String> {
    let spec = data_file::source_spec(path);
    let parsed = parse_spec(&spec, Format::Yaml).map_err(|e| format!("parse: {e}"))?;
    let analysis = analyse_spec(&parsed.spec).map_err(|e| format!("analysis: {e}"))?;
    let load = Engine::new()
        .load_spec_with(parsed.spec, analysis, None, &LoadOptions::packaged())
        .map_err(|e| format!("load: {e}"))?;
    let at = Instant::now();
    let profiles = load.session.profile_sources();
    let elapsed = at.elapsed();
    if profiles.is_empty() {
        return Err("the profile pass reported no source".to_string());
    }
    Ok(elapsed.as_secs_f64() * 1000.0)
}

/// Open `shape` and report what it cost.
///
/// `repeats` timed samples of each of the two timed quantities; the scan tally
/// is taken once whatever `repeats` says, because it is a property of the plan
/// and not of the clock. **`repeats: 0` takes the tally and skips the clock
/// entirely**, which is what the tests below want: the scan count is the same
/// number on a 240-row fixture as on a 14,133-row one, and opening the wide
/// shape for real is seconds of tile queries that say nothing about it.
///
/// `materialise` is the open being measured — see [`options`]. `false` is the
/// branch a file too large to copy takes, and it is what the before-and-after
/// of reading the file into memory is measured against.
///
/// # Errors
///
/// The fixture could not be written, or the file would not open.
pub fn measure(
    conn: &duckdb::Connection,
    dir: &Path,
    shape: &Shape,
    repeats: usize,
    materialise: bool,
) -> Result<Measured, String> {
    let path = ensure_csv(conn, dir, shape)?;
    let bytes = std::fs::metadata(&path)
        .map_err(|e| format!("stat {}: {e}", path.display()))?
        .len();
    let chosen = path.to_str().ok_or("fixture path is not UTF-8")?;

    let (tally, columns) = tally(&path)?;
    if columns != shape.columns() {
        return Err(format!(
            "{}: the profile found {columns} columns and the fixture has {}",
            shape.name,
            shape.columns()
        ));
    }

    // The composition's count, taken once and untimed for the same reason the
    // profile pass's is: the `EXPLAIN` before each statement is what makes the
    // count readable and is not what an open pays.
    let counting = data_file::OpenOptions {
        count_scans: true,
        ..options(materialise)
    };
    let (counted, composition_tally) =
        data_file::open_traced(chosen, &counting).map_err(|e| format!("{}: {e}", shape.name))?;
    let mut tiles = counted.dashboard.tiles().len();
    let mut composition_queries = counted.live.executes();
    drop(counted);

    let mut profile_ms = Vec::with_capacity(repeats);
    let mut open_ms = Vec::with_capacity(repeats);
    let mut composition_ms = Vec::with_capacity(repeats);
    let mut materialise_ms = Vec::with_capacity(repeats);
    for _ in 0..repeats {
        profile_ms.push(time_profile(&path)?);
        let at = Instant::now();
        let (opened, trace) = data_file::open_traced(chosen, &options(materialise))
            .map_err(|e| format!("{}: {e}", shape.name))?;
        open_ms.push(at.elapsed().as_secs_f64() * 1000.0);
        composition_ms.push(trace.composition_ms);
        if trace.materialised {
            materialise_ms.push(trace.materialise_ms);
        }
        tiles = opened.dashboard.tiles().len();
        composition_queries = opened.live.executes();
    }

    Ok(Measured {
        shape: *shape,
        columns,
        bytes,
        scans: tally.scans(),
        scan_bound: profile::SCANS_PER_SOURCE,
        statements: tally
            .statements
            .iter()
            .map(|s| StatementRecord {
                scans: s.scans,
                sql: s.sql.chars().take(SQL_RECORDED).collect(),
            })
            .collect(),
        profile: Stats::from_ms(profile_ms),
        open: Stats::from_ms(open_ms),
        tiles,
        composition_queries,
        composition_scans: composition_tally.composition.scans(),
        composition_file_reads: composition_tally.composition.file_reads(),
        composition_file_read_bound: pipeline::COMPOSITION_FILE_READS,
        materialised: composition_tally.materialised,
        materialise: Stats::from_ms(materialise_ms),
        // Taken from the counted open rather than from a timed repeat, for two
        // reasons: it is a size and not a duration, so the `EXPLAIN`ing that
        // makes the counted open the wrong place to read a clock does not
        // touch it; and `--repeats 0` is a legitimate way to run this suite
        // for its counts alone, and the loop above does not execute then.
        materialise_bytes: composition_tally.materialise_bytes,
        composition_statements: composition_tally
            .composition
            .statements
            .iter()
            .map(|s| StatementRecord {
                scans: s.scans,
                sql: s.sql.chars().take(SQL_RECORDED).collect(),
            })
            .collect(),
        composition: Stats::from_ms(composition_ms),
    })
}

/// The measured shapes, as the lines the run prints.
#[must_use]
pub fn report(rows: &[Measured]) -> String {
    let mut out = String::new();
    out.push_str(
        "shape   rows    cols  numeric  bytes      tiles  profile      profile p50  \
         compose  reads/bound  in-memory  compose p50  open p50\n",
    );
    for m in rows {
        let p50 = |s: &Option<Stats>| {
            s.as_ref()
                .map_or_else(|| "?".to_string(), |s| format!("{:.1} ms", s.p50_ms))
        };
        let bounded = |scans: Option<u32>, bound: u32| {
            format!(
                "{}/{bound}",
                scans.map_or_else(|| "?".to_string(), |s| s.to_string())
            )
        };
        out.push_str(&format!(
            "{:<7} {:<7} {:<5} {:<8} {:<10} {:<6} {:<12} {:<12} {:<8} {:<12} {:<10} {:<12} {}\n",
            m.shape.name,
            m.shape.rows,
            m.columns,
            m.shape.numeric,
            m.bytes,
            m.tiles,
            bounded(m.scans, m.scan_bound),
            p50(&m.profile),
            m.composition_scans
                .map_or_else(|| "?".to_string(), |s| s.to_string()),
            bounded(m.composition_file_reads, m.composition_file_read_bound),
            if m.materialised { "yes" } else { "no" },
            p50(&m.composition),
            p50(&m.open)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixture small enough to open inside a test, at each shape's column
    /// split.
    ///
    /// **The row count is dropped and the column split is not**, because the
    /// scan count is a property of the statement the pass writes and the
    /// statement is written from the columns. A test that needed the full
    /// fourteen thousand rows to see the defect would be a test nobody runs.
    /// **The declared shapes are fixed here, against literals, and this is
    /// the only place they are.**
    ///
    /// Every other test in this module is written in terms of `shape.numeric`
    /// and `shape.columns()` — `moments.len() == shape.numeric`,
    /// `binned == shape.numeric - 1` — so each of them holds the profiled
    /// output against the DECLARED shape and would pass at any declaration.
    /// Checking a declaration against itself is not a pin, and a test that
    /// re-derives a number from the number is one an author can cite as
    /// though it were.
    ///
    /// What the pin is for: `WIDE` is not an arbitrary big table. Its row and
    /// column counts are the ones from the file this whole measurement
    /// started at, and the committed record, the harness's printed table and
    /// the pull request all quote them. A shape that drifted would leave all
    /// of those describing a file nobody opened, with nothing red.
    ///
    /// `NARROW` is pinned for a nearer reason: the guard's vacuity check
    /// compares the two shapes' numeric counts, and raising the narrow shape
    /// is the cheapest wrong way to quiet a red guard.
    #[test]
    fn the_two_shapes_are_fixed_at_the_counts_they_were_chosen_for() {
        assert_eq!(
            WIDE.rows, 14_133,
            "the wide fixture's row count is the motivating file's and is \
             quoted in the committed record"
        );
        assert_eq!(
            WIDE.numeric, 14,
            "the wide fixture's numeric column count is what the distribution \
             pass scales with, so it is the count every figure about this \
             measurement rests on"
        );
        assert_eq!(WIDE.text, 6, "the wide fixture's VARCHAR column count");
        assert_eq!(
            WIDE.timestamps, 2,
            "the wide fixture's temporal column count — the columns that carry \
             bounds and must add no distribution"
        );
        assert_eq!(
            WIDE.columns(),
            22,
            "the wide fixture's column count is the motivating file's, and the \
             three counts above have to add up to it"
        );
        assert_eq!(
            NARROW.numeric, 2,
            "the narrow fixture's numeric column count, which the guard's \
             vacuity check is stated against"
        );
        assert_eq!(NARROW.columns(), 3, "the narrow fixture's column count");
    }

    fn small(shape: &Shape) -> Shape {
        Shape {
            rows: 240,
            ..*shape
        }
    }

    /// The tally for a shape, with the clock left out — see [`measure`] on
    /// why `repeats: 0` is the right call for a scan assertion.
    fn counted(shape: &Shape) -> Measured {
        let dir = std::env::temp_dir().join(format!(
            "bf-open-scan-{}-{}",
            std::process::id(),
            shape.name
        ));
        let conn = duckdb::Connection::open_in_memory().expect("duckdb");
        measure(&conn, &dir, shape, 0, true).expect("measure")
    }

    /// **Opening a file reads it a bounded number of times, and the bound does
    /// not move with the column count.**
    ///
    /// Two assertions, and they fail for different reasons. The first is the
    /// bound: [`profile::SCANS_PER_SOURCE`] reads, whatever the table. The
    /// second is the shape of the old defect — a read per numeric column —
    /// and it is stated as the two shapes agreeing rather than as a number,
    /// because a bound alone cannot tell a constant from a count that happens
    /// to fit under it today.
    ///
    /// The vacuity guard is the third assertion: the wide shape has to carry
    /// materially more numeric columns than the narrow one, or "they agree"
    /// is a sentence about one table written twice.
    #[test]
    fn a_wide_table_is_read_no_more_often_than_a_narrow_one() {
        let narrow = counted(&small(&NARROW));
        let wide = counted(&small(&WIDE));

        assert!(
            wide.shape.numeric >= narrow.shape.numeric * 5,
            "the wide shape carries {} numeric columns against the narrow \
             shape's {} — too close for their agreeing to mean anything",
            wide.shape.numeric,
            narrow.shape.numeric
        );

        // The class first, the number second. This one holds however the
        // bound is written, so raising the bound to cover a count that has
        // gone proportional again does not buy the raise anything.
        assert_eq!(
            wide.scans, narrow.scans,
            "the {}-numeric-column table is read {:?} times and the \
             {}-numeric-column one {:?}, so the count tracks the columns: \
             {:#?}",
            wide.shape.numeric, wide.scans, narrow.shape.numeric, narrow.scans, wide.statements
        );
        assert_eq!(
            narrow.scans,
            Some(profile::SCANS_PER_SOURCE),
            "the narrow table's open read it {:?} times against a bound of {}: \
             {:#?}",
            narrow.scans,
            profile::SCANS_PER_SOURCE,
            narrow.statements
        );
        assert_eq!(
            wide.scans,
            Some(profile::SCANS_PER_SOURCE),
            "the {}-column table's open read it {:?} times against a bound of \
             {} — a read per numeric column is what this bound exists to \
             catch: {:#?}",
            wide.columns,
            wide.scans,
            profile::SCANS_PER_SOURCE,
            wide.statements
        );
    }

    /// **Composing the first screen reads the data file the same number of
    /// times whatever the tile count, and that number is
    /// [`pipeline::COMPOSITION_FILE_READS`].**
    ///
    /// Three assertions in a deliberate order, and they fail for different
    /// reasons.
    ///
    /// The **vacuity guard** comes first: the wide shape has to draw
    /// materially more tiles than the narrow one, or "they agree" is a
    /// sentence about one dashboard written twice. It is stated on the tiles
    /// rather than on the columns because tiles are what the composition
    /// issues statements for — a table of twenty columns that drew two tiles
    /// would satisfy a column-count guard and prove nothing.
    ///
    /// The **class** comes second: the two shapes agree. This holds however
    /// the bound is written, so raising the bound to cover a count that has
    /// gone proportional again does not buy the raise anything. It is the
    /// assertion that catches a composition reading the file once per tile,
    /// which is the defect the whole measurement started at.
    ///
    /// The **number** comes last. Restoring the per-tile shape reddens the
    /// assertion above it even if this literal is raised to fit.
    ///
    /// The second witness is the one that keeps the first honest:
    /// `composition_scans` — the leaf count, rather than the file-read subset
    /// of it — is asserted to be *larger* on the wide shape than on the
    /// narrow one. The
    /// composition still issues a statement per tile and their plans still
    /// have leaves; what changed is what those leaves read. Without this the
    /// file-read count could read zero because the attribution had stopped
    /// working, and a test that cannot tell "reads nothing" from "counts
    /// nothing" is testing nothing.
    #[test]
    fn composing_a_wide_dashboard_reads_the_file_no_more_often_than_a_narrow_one() {
        let narrow = counted(&small(&NARROW));
        let wide = counted(&small(&WIDE));

        assert!(
            wide.materialised && narrow.materialised,
            "the fixtures were not read into memory (narrow {}, wide {}), so              the bound below is not the thing this test is about",
            narrow.materialised,
            wide.materialised
        );
        assert!(
            wide.tiles >= narrow.tiles * 5,
            "the wide dashboard draws {} tiles against the narrow one's {} —              too close for their agreeing to mean anything",
            wide.tiles,
            narrow.tiles
        );

        // The class first, the number second.
        assert_eq!(
            wide.composition_file_reads, narrow.composition_file_reads,
            "composing {} tiles read the file {:?} times and composing {} read              it {:?}, so the count tracks the tiles: {:#?}",
            wide.tiles,
            wide.composition_file_reads,
            narrow.tiles,
            narrow.composition_file_reads,
            wide.composition_statements
        );
        assert_eq!(
            narrow.composition_file_reads,
            Some(pipeline::COMPOSITION_FILE_READS),
            "the narrow dashboard's composition read the file {:?} times              against a bound of {}: {:#?}",
            narrow.composition_file_reads,
            pipeline::COMPOSITION_FILE_READS,
            narrow.composition_statements
        );
        assert_eq!(
            wide.composition_file_reads,
            Some(pipeline::COMPOSITION_FILE_READS),
            "the {}-tile dashboard's composition read the file {:?} times              against a bound of {} — a read per tile is what this bound exists              to catch: {:#?}",
            wide.tiles,
            wide.composition_file_reads,
            pipeline::COMPOSITION_FILE_READS,
            wide.composition_statements
        );

        // The witness. The statements are still there and still have leaves;
        // a file-read count of zero that came from counting nothing would sit
        // beside a leaf count of zero, and this is what tells them apart.
        let (Some(wide_scans), Some(narrow_scans)) =
            (wide.composition_scans, narrow.composition_scans)
        else {
            panic!(
                "a composition statement went unexplained, so the file-read                  count above is reading past a hole: {:#?}",
                wide.composition_statements
            );
        };
        assert!(
            wide_scans > narrow_scans,
            "the wide composition planned {wide_scans} leaves and the narrow              one {narrow_scans}. The composition still issues a statement per              tile, so the wide one must plan more leaves than the narrow one —              equal counts here mean the leaf counter has stopped counting, and              then the file-read count above is zero for the wrong reason"
        );
        assert!(
            narrow_scans > 0,
            "the narrow composition planned no leaves at all, so both counts              above are about a composition that issued nothing"
        );
    }

    /// **The file-read count is a real subset and not a constant zero.**
    ///
    /// The bound above is zero, and a counter hard-wired to zero would pass
    /// it on both shapes. This is the case that must NOT be zero: the profile
    /// pass runs before the source has been materialised, over a view on
    /// `read_csv`, so its leaves are reads of the file and the two counts have
    /// to agree.
    ///
    /// It is also what pins the exclusion rule. If DuckDB stopped carrying a
    /// `Table` key, every leaf would count as a file read and this test would
    /// still pass — that is the safe direction. If the rule inverted, or the
    /// walk stopped finding leaves, this is where it goes red.
    #[test]
    fn the_profile_pass_reads_the_file_at_every_leaf_it_plans() {
        let shape = small(&WIDE);
        let dir = std::env::temp_dir().join(format!("bf-open-attrib-{}", std::process::id()));
        let conn = duckdb::Connection::open_in_memory().expect("duckdb");
        let path = ensure_csv(&conn, &dir, &shape).expect("fixture");
        let (tally, _) = tally(&path).expect("tally");

        assert_eq!(
            tally.file_reads(),
            tally.scans(),
            "the profile pass runs before anything is materialised, so every              leaf it plans is a read of the file — {:?} of {:?} were counted              as one: {:#?}",
            tally.file_reads(),
            tally.scans(),
            tally.statements
        );
        assert_eq!(
            tally.file_reads(),
            Some(profile::SCANS_PER_SOURCE),
            "the profile pass read the file {:?} times against its own bound              of {}",
            tally.file_reads(),
            profile::SCANS_PER_SOURCE
        );
    }

    /// **A statement DuckDB would not explain is not a statement that cost
    /// nothing.**
    ///
    /// The tally carries an unexplained statement out as an absence rather
    /// than as a zero, and the bound above compares against `Some(_)` so the
    /// absence fails it. This is the assertion that says the absence is
    /// reachable at all — if every statement is explained, as it is here, the
    /// list is empty and the bound above is measuring something.
    #[test]
    fn every_statement_the_profile_pass_issues_is_explained() {
        let wide = counted(&small(&WIDE));
        let unexplained: Vec<&str> = wide
            .statements
            .iter()
            .filter(|s| s.scans.is_none())
            .map(|s| s.sql.as_str())
            .collect();
        assert!(
            unexplained.is_empty(),
            "DuckDB declined to explain {} of the {} statements, so the scan \
             count above is reading past a hole: {unexplained:#?}",
            unexplained.len(),
            wide.statements.len()
        );
        // Every statement carries at least one leaf, so the statement count
        // can never exceed the scan count — `<=` rather than `==` because
        // folding two of these statements into one would be a good change and
        // this is not the test that should refuse it.
        assert!(
            wide.statements.len() <= profile::SCANS_PER_SOURCE as usize,
            "the pass issued {} statements against a bound of {} reads: {:#?}",
            wide.statements.len(),
            profile::SCANS_PER_SOURCE,
            wide.statements
        );
    }

    /// **The wide fixture reaches both branches of the distribution.**
    ///
    /// One statement now carries the per-value branch and the binned branch
    /// together, so a fixture that reached only one of them would leave half
    /// the statement unexercised — and the half it left out is the half a
    /// mutation could break silently.
    #[test]
    fn the_fixture_carries_a_bounded_column_and_a_wide_one() {
        let shape = small(&WIDE);
        let dir = std::env::temp_dir().join(format!("bf-open-branches-{}", std::process::id()));
        let conn = duckdb::Connection::open_in_memory().expect("duckdb");
        let path = ensure_csv(&conn, &dir, &shape).expect("fixture");

        let spec = data_file::source_spec(&path);
        let parsed = parse_spec(&spec, Format::Yaml).expect("parse");
        let analysis = analyse_spec(&parsed.spec).expect("analysis");
        let load = Engine::new()
            .load_spec_with(parsed.spec, analysis, None, &LoadOptions::packaged())
            .expect("load");
        let profiles = load.session.profile_sources();
        let columns = match &profiles
            .iter()
            .find(|p| p.name == data_file::SOURCE)
            .expect("the source")
            .outcome
        {
            ProfileOutcome::Profiled { columns, .. } => columns.clone(),
            other => panic!("the fixture did not profile: {other:?}"),
        };

        let moments: Vec<_> = columns.iter().filter_map(|c| c.moments.as_ref()).collect();
        assert_eq!(
            moments.len(),
            shape.numeric,
            "{} of the fixture's {} numeric columns carry moments",
            moments.len(),
            shape.numeric
        );

        let per_value = moments
            .iter()
            .filter(|m| matches!(m.distribution, brightfield_engine::Distribution::Values(_)))
            .count();
        let binned = moments
            .iter()
            .filter(|m| matches!(m.distribution, brightfield_engine::Distribution::Bins(_)))
            .count();
        assert_eq!(
            per_value, 1,
            "the bounded column should be the one taking the per-value branch"
        );
        assert_eq!(
            binned,
            shape.numeric - 1,
            "every other numeric column should be binned"
        );

        // Every row is counted exactly once, on both branches — the property
        // a combined statement could break by dropping a column's entries or
        // by counting one column's rows against another's slot.
        for (column, m) in columns.iter().filter(|c| c.moments.is_some()).zip(&moments) {
            let counted: u64 = match &m.distribution {
                brightfield_engine::Distribution::Values(v) => v.iter().map(|(_, n)| *n).sum(),
                brightfield_engine::Distribution::Bins(b) => b.iter().sum(),
            };
            assert_eq!(
                counted, column.non_null,
                "{}: the distribution counts {counted} rows and the column has \
                 {} non-null",
                column.name, column.non_null
            );
        }
    }

    /// **The harness itself opens a file and comes back with figures.**
    ///
    /// The two tests above take `repeats: 0`, which never calls
    /// `data_file::open` — so without this one the timed half of [`measure`]
    /// could be broken outright and the suite would stay green. The narrow
    /// shape, because what is proved here is that the path runs, and the
    /// narrow shape runs it with the fewest tiles.
    #[test]
    fn the_timed_half_of_the_harness_opens_a_file_and_reports() {
        let dir = std::env::temp_dir().join(format!("bf-open-timed-{}", std::process::id()));
        let conn = duckdb::Connection::open_in_memory().expect("duckdb");
        let m = measure(&conn, &dir, &small(&NARROW), 1, true).expect("measure");

        let profile = m.profile.as_ref().expect("a timed profile sample");
        let open = m.open.as_ref().expect("a timed open sample");
        assert_eq!(profile.n, 1);
        assert_eq!(open.n, 1);
        assert!(
            open.min_ms >= profile.min_ms,
            "the whole open ({} ms) came in under the profile pass inside it \
             ({} ms), so one of the two is not timing what it says",
            open.min_ms,
            profile.min_ms
        );
        assert!(
            m.tiles > 0,
            "the dashboard chose no tile, so `open` timed a composition of \
             nothing"
        );
        assert_eq!(
            m.composition_queries, m.tiles,
            "{} tiles issued {} queries on the first composition",
            m.tiles, m.composition_queries
        );
        assert!(
            m.bytes > 0,
            "the fixture is empty, so every figure above is about no file"
        );
        assert!(
            report(&[m]).contains("narrow"),
            "the printed report does not name the shape it measured"
        );
    }
}
