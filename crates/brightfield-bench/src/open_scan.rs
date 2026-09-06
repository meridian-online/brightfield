//! What opening a data file costs: how many times it is read, and how long
//! the wait is.
//!
//! # Why this is measured separately from the interaction baseline
//!
//! Everything else in this harness times a gesture on a table that is already
//! open. This times the wait before the first picture — the one an analyst
//! meets first — and it is dominated by a different thing. A `file:` source is
//! a DuckDB view over `read_csv`, so **every statement issued over it reads and
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
//! composition over it — and it needs the full row count to mean anything.
//!
//! [counting]: brightfield_engine::Session::profile_sources_counting_scans

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use brightfield_engine::{profile, Engine, LoadOptions, ProfileOutcome, ScanTally};
use brightfield_shell::data_file;
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
    /// Numeric columns. The first is deliberately bounded — see
    /// [`Shape::columns`] — so both branches of the distribution are counted.
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

/// The wide table, at the shape of the file that motivated this measurement:
/// fourteen thousand rows and twenty-two columns, about 2.6 MB of CSV.
///
/// The type split is an earthquake feed's — fourteen measurements, six labels
/// and two times — because that is what a public data file of this shape holds
/// and because it is the numeric count that drives the pass.
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
/// branches and a fixture that reached only one of them would leave the other
/// unmeasured. Both branches now ride the same statement, which is precisely
/// why a fixture has to carry both.
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
    /// Tiles the dashboard chose for the file — one query each on the first
    /// composition, which is the rest of the wait.
    pub tiles: usize,
    /// DuckDB executes the first composition performed.
    pub composition_queries: usize,
}

/// Write (or reuse) the CSV for `shape` under `dir`.
///
/// Every column is a pure function of the row index through DuckDB's `hash()`,
/// as the interaction harness's datasets are, so a present file IS the fixture
/// and regeneration could only reproduce it.
///
/// # Errors
///
/// The directory could not be created, or DuckDB would not write the file.
pub fn ensure_csv(
    conn: &duckdb::Connection,
    dir: &Path,
    shape: &Shape,
) -> Result<PathBuf, String> {
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
/// # Errors
///
/// The fixture could not be written, or the file would not open.
pub fn measure(
    conn: &duckdb::Connection,
    dir: &Path,
    shape: &Shape,
    repeats: usize,
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

    let mut profile_ms = Vec::with_capacity(repeats);
    let mut open_ms = Vec::with_capacity(repeats);
    let mut tiles = 0;
    let mut composition_queries = 0;
    for _ in 0..repeats {
        profile_ms.push(time_profile(&path)?);
        let at = Instant::now();
        let opened = data_file::open(chosen).map_err(|e| format!("{}: {e}", shape.name))?;
        open_ms.push(at.elapsed().as_secs_f64() * 1000.0);
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
    })
}

/// The measured shapes, as the lines the run prints.
#[must_use]
pub fn report(rows: &[Measured]) -> String {
    let mut out = String::new();
    out.push_str(
        "shape   rows    cols  numeric  bytes      scans/bound  profile p50  open p50  tiles\n",
    );
    for m in rows {
        let scans = m
            .scans
            .map_or_else(|| "?".to_string(), |s| s.to_string());
        let profile = m
            .profile
            .as_ref()
            .map_or_else(|| "?".to_string(), |s| format!("{:.1} ms", s.p50_ms));
        let open = m
            .open
            .as_ref()
            .map_or_else(|| "?".to_string(), |s| format!("{:.1} ms", s.p50_ms));
        out.push_str(&format!(
            "{:<7} {:<7} {:<5} {:<8} {:<10} {:<12} {:<12} {:<9} {}\n",
            m.shape.name,
            m.shape.rows,
            m.columns,
            m.shape.numeric,
            m.bytes,
            format!("{scans}/{}", m.scan_bound),
            profile,
            open,
            m.tiles
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
        measure(&conn, &dir, shape, 0).expect("measure")
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
        assert_eq!(
            wide.scans, narrow.scans,
            "the wide table is read {:?} times and the narrow one {:?}, so the \
             count tracks the columns",
            wide.scans, narrow.scans
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
        assert_eq!(
            wide.statements.len(),
            profile::SCANS_PER_SOURCE as usize,
            "the pass issued {} statements: {:#?}",
            wide.statements.len(),
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
        let m = measure(&conn, &dir, &small(&NARROW), 1).expect("measure");

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
