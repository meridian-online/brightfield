//! Reading the file into memory before composing must change what an open
//! **costs** and nothing about what it **draws**.
//!
//! `data_file::open` reads a file under
//! [`data_file::MATERIALISE_UNDER_BYTES`] into a session-scoped table and
//! points the view at it, so a tile's query scans memory instead of
//! re-reading and re-parsing the file. Every emitted query is byte-identical
//! either way — the view keeps its name and its columns — which is a claim
//! about the SQL and not yet a claim about the rows that come back.
//!
//! These tests make it a claim about the rows. Each opens the same fixture
//! twice, once down each branch of the threshold, and compares what every
//! mark actually drew. Prose asserting "the picture is unchanged" is what
//! this replaces.

use std::path::{Path, PathBuf};

use arrow::util::pretty::pretty_format_batches;
use brightfield_engine::MATERIALISE_BUDGET_FLOOR_BYTES;
use brightfield_shell::data_file::{self, OpenOptions, MATERIALISE_UNDER_BYTES};

/// A fixture with one column of each kind the dashboard draws a tile from: two
/// measures (one bounded, so both branches of the histogram's own device are
/// exercised), a category, and a timestamp.
///
/// Row count is small on purpose — what is compared here is equality of two
/// result sets, and a difference in a binned count shows up in two hundred
/// rows exactly as it does in fourteen thousand.
fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bf-materialise-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join(format!("{name}.csv"));
    if path.exists() {
        return path;
    }
    let conn = duckdb::Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(&format!(
        "COPY (SELECT (hash(i * 11 + 7) % 12)::DOUBLE AS bounded, \
         ((hash(i * 17 + 3) % 1000000) / 1000.0)::DOUBLE AS wide, \
         ('label_' || (hash(i * 13 + 5) % 6))::VARCHAR AS label, \
         (TIMESTAMP '2020-01-01' + INTERVAL (i) SECOND) AS at \
         FROM range(240) AS r(i)) TO '{}' (FORMAT CSV, HEADER)",
        path.display()
    ))
    .expect("write fixture");
    path
}

/// A Parquet of the same rows, written by the same DuckDB the engine reads it
/// back with.
fn parquet_fixture() -> PathBuf {
    let csv = fixture("rows");
    let path = csv.with_extension("parquet");
    if path.exists() {
        return path;
    }
    let conn = duckdb::Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(&format!(
        "COPY (SELECT * FROM read_csv('{}')) TO '{}' (FORMAT PARQUET)",
        csv.display(),
        path.display()
    ))
    .expect("write parquet");
    path
}

/// Every mark's result set, as text, in mark order — the rows each tile drew.
///
/// Rendered rather than compared as `RecordBatch`es because a difference has
/// to be readable: a bin edge that moved by one step is a diff a reader can
/// see, and a failed `assert_eq!` on two vectors of Arrow batches is not.
fn drawn_rows(path: &Path, options: &OpenOptions) -> (Vec<String>, bool, usize) {
    let (mut opened, trace) =
        data_file::open_traced(&path.to_string_lossy(), options).expect("the fixture opens");
    let tiles = opened.dashboard.tiles().len();
    let marks = opened.live.coordinator().session().mark_count();
    let mut out = Vec::with_capacity(marks);
    for index in 0..marks {
        let batches = opened
            .live
            .coordinator()
            .session_mut()
            .execute_mark(index)
            .unwrap_or_else(|e| panic!("mark {index}: {e}"));
        out.push(
            pretty_format_batches(&batches)
                .unwrap_or_else(|e| panic!("mark {index}: {e}"))
                .to_string(),
        );
    }
    (out, trace.materialised, tiles)
}

/// **Every tile draws the same rows whether or not the file was read into
/// memory first.**
///
/// The two calls differ in exactly one thing: the threshold. One is the app's
/// own open; the other is the branch a file too large to copy takes. If the
/// materialised source were subtly not the file — a column re-typed by the
/// copy, a NULL that became an empty string, rows in a different order under
/// a `LIMIT` — this is where it shows, per mark, as a diff of the rows.
#[test]
fn every_mark_draws_the_same_rows_down_both_branches_of_the_threshold() {
    let path = fixture("rows");

    let (materialised, was_materialised, tiles) = drawn_rows(&path, &OpenOptions::default());
    let (direct, was_direct, direct_tiles) = drawn_rows(
        &path,
        &OpenOptions {
            materialise_under_bytes: 0,
            ..OpenOptions::default()
        },
    );

    // The vacuity guard: the two calls have to have taken different branches,
    // or this compares one route with itself.
    assert!(
        was_materialised,
        "the default threshold did not materialise a {}-byte fixture, so both \
         sides of this comparison are the same route",
        std::fs::metadata(&path).expect("stat").len()
    );
    assert!(
        !was_direct,
        "a threshold of zero still materialised, so both sides of this \
         comparison are the same route"
    );

    assert_eq!(
        tiles, direct_tiles,
        "the dashboard chose {tiles} tiles one way and {direct_tiles} the \
         other, so the marks below are not the same marks"
    );
    assert!(
        tiles >= 4,
        "the fixture drew {tiles} tiles, too few to cover the histogram, the \
         ranked bars and the time bars this comparison is for"
    );
    assert_eq!(
        materialised.len(),
        direct.len(),
        "{} marks executed one way and {} the other",
        materialised.len(),
        direct.len()
    );
    assert!(
        !materialised.is_empty(),
        "no mark executed at all, so this test compared two empty lists"
    );

    for (index, (from_memory, from_file)) in materialised.iter().zip(&direct).enumerate() {
        assert_eq!(
            from_memory, from_file,
            "mark {index} drew different rows once the file was read into \
             memory.\n--- from the materialised table ---\n{from_memory}\n\
             --- from the file ---\n{from_file}"
        );
    }
}

/// **A file over the threshold still opens, and it opens off the file.**
///
/// The property the module header sells is that a Parquet larger than memory
/// opens as readily as a small CSV, and the copy above is what could have
/// taken it away. This is the branch that keeps it: no copy is made, the view
/// is still a view on `read_parquet`, and the dashboard composes.
///
/// A Parquet rather than a CSV because that is the format the claim is made
/// about, and it is the one whose on-disk size says least about its size in
/// memory.
#[test]
fn a_parquet_over_the_threshold_opens_without_being_copied() {
    let path = parquet_fixture();

    let (opened, trace) = data_file::open_traced(
        &path.to_string_lossy(),
        &OpenOptions {
            materialise_under_bytes: 0,
            ..OpenOptions::default()
        },
    )
    .expect("a Parquet over the threshold opens");

    assert!(
        !trace.materialised,
        "the file was copied into memory despite being over the threshold, so \
         this test is not about the branch it names"
    );
    assert!(
        !opened.dashboard.tiles().is_empty(),
        "the dashboard drew no tile, so nothing was composed to be unaffected"
    );
    assert!(
        opened.composed.mark_faults.is_empty(),
        "the composition reported faults: {:?}",
        opened.composed.mark_faults
    );
    assert!(
        trace.bytes > 0,
        "the fixture is empty, so the threshold was applied to no file"
    );
}

// ---------------------------------------------------------------------------
// The two conditions, in the two units they are stated in
// ---------------------------------------------------------------------------

/// A CSV of exactly `rows` fixed-width rows, so its size on disk is arithmetic
/// rather than a thing to measure afterwards.
///
/// Every row is a 200-character label and a three-digit number: 205 bytes with
/// the comma and the newline, under a 12-byte header. That exactness is what
/// lets a test put one file on each side of a threshold one row apart.
fn sized_csv(name: &str, rows: u64) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bf-materialise-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join(format!("{name}.csv"));
    if path.exists() {
        return path;
    }
    let conn = duckdb::Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(&format!(
        "COPY (SELECT lpad(('x' || (i % 7))::VARCHAR, 200, 'y') AS label, \
         lpad((i % 97)::VARCHAR, 3, '0') AS value FROM range({rows}) AS r(i)) \
         TO '{}' (FORMAT CSV, HEADER)",
        path.display()
    ))
    .expect("write fixture");
    path
}

/// Bytes a [`sized_csv`] of `rows` rows occupies: `label,value\n` and then
/// 200 + 1 + 3 + 1 per row.
fn sized_csv_bytes(rows: u64) -> u64 {
    12 + 205 * rows
}

/// **The threshold the build ships is the one an ordinary open applies.**
///
/// Every other test on this branch reaches it by passing
/// `materialise_under_bytes` explicitly, which is what makes the branch
/// reachable and also what makes the shipped constant unread: setting
/// `MATERIALISE_UNDER_BYTES` to `u64::MAX` left all of them green. This one
/// goes through `OpenOptions::default()` on two files one row apart, either
/// side of the constant, so the number itself decides the outcome.
#[test]
fn the_shipped_threshold_is_what_decides_an_ordinary_open() {
    // The bounds are the test's own affordability, and they redden loudly for
    // the two mutations that would otherwise make this test hang or write a
    // file nobody can hold: a threshold of `u64::MAX` or of zero.
    assert!(
        (1024 * 1024..=256 * 1024 * 1024).contains(&MATERIALISE_UNDER_BYTES),
        "the shipped threshold is {MATERIALISE_UNDER_BYTES} bytes; this test \
         writes a file either side of it and will not write that much, nor \
         open an empty one"
    );

    let over_rows = MATERIALISE_UNDER_BYTES.div_ceil(205);
    let under_rows = over_rows - 1;
    assert!(
        sized_csv_bytes(under_rows) <= MATERIALISE_UNDER_BYTES
            && sized_csv_bytes(over_rows) > MATERIALISE_UNDER_BYTES,
        "the two row counts do not straddle the threshold: {} and {} against \
         {MATERIALISE_UNDER_BYTES}",
        sized_csv_bytes(under_rows),
        sized_csv_bytes(over_rows)
    );

    let under = sized_csv("just-under", under_rows);
    let over = sized_csv("just-over", over_rows);
    assert_eq!(
        std::fs::metadata(&under).expect("stat").len(),
        sized_csv_bytes(under_rows),
        "the fixture is not the size the arithmetic above says it is, so the \
         two files may not straddle the threshold at all"
    );
    assert_eq!(
        std::fs::metadata(&over).expect("stat").len(),
        sized_csv_bytes(over_rows),
        "the fixture is not the size the arithmetic above says it is"
    );

    let (_, was_copied, _) = drawn_rows(&under, &OpenOptions::default());
    assert!(
        was_copied,
        "a file of {} bytes, one row under the shipped {MATERIALISE_UNDER_BYTES}, \
         was not read into memory by an ordinary open",
        sized_csv_bytes(under_rows)
    );

    let (_, was_copied, _) = drawn_rows(&over, &OpenOptions::default());
    assert!(
        !was_copied,
        "a file of {} bytes, over the shipped {MATERIALISE_UNDER_BYTES}, was \
         read into memory by an ordinary open",
        sized_csv_bytes(over_rows)
    );
}

/// A Parquet that is tiny on disk and large in memory: four low-cardinality
/// columns over half a million rows, ZSTD.
///
/// **This is the fixture the whole size question turns on.** Its size on disk
/// says nothing about the memory the copy costs, which is why the copy is not
/// bounded by its size on disk.
fn widening_parquet() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bf-materialise-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join("widening.parquet");
    if path.exists() {
        return path;
    }
    let conn = duckdb::Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(&format!(
        "COPY (SELECT ('cat_' || (i % 8))::VARCHAR AS label, \
         ('grp_' || (i % 5))::VARCHAR AS kind, (i % 97)::BIGINT AS value, \
         (i % 13)::BIGINT AS other FROM range(500000) AS r(i)) \
         TO '{}' (FORMAT PARQUET, COMPRESSION ZSTD)",
        path.display()
    ))
    .expect("write parquet");
    path
}

/// **A file whose table does not fit the budget opens off the view, and draws
/// the same rows.**
///
/// This is the branch the larger-than-memory property rests on, driven through
/// a real `data_file::open` rather than asserted in prose. The fixture is one
/// the old guard would have got wrong in the dangerous direction: comfortably
/// under `MATERIALISE_UNDER_BYTES` on disk, and far over the budget set here.
///
/// The budget is the test's rather than the shipped 512 MiB, because a fixture
/// big enough to exceed the shipped one would make the suite allocate half a
/// gigabyte to prove a branch a small one proves exactly as well. What the
/// shipped number decides is pinned separately, by
/// `every_recorded_open_spends_far_less_than_the_budget_allows` in the
/// open-scan harness.
#[test]
fn a_file_whose_table_exceeds_the_budget_opens_off_the_view() {
    let path = widening_parquet();
    let on_disk = std::fs::metadata(&path).expect("stat").len();
    assert!(
        on_disk < MATERIALISE_UNDER_BYTES,
        "the fixture is {on_disk} bytes on disk, over the {MATERIALISE_UNDER_BYTES} \
         threshold — the copy would be declined on size and this test would \
         not reach the budget at all"
    );

    let tight = OpenOptions {
        materialise_budget_bytes: MATERIALISE_BUDGET_FLOOR_BYTES,
        ..OpenOptions::default()
    };
    let (from_view, was_copied, tiles) = drawn_rows(&path, &tight);
    assert!(
        !was_copied,
        "a table far larger than the budget was copied anyway — the budget is \
         not being imposed, or DuckDB spilled it to disk rather than refusing"
    );

    // The vacuity guard, and it is a measurement rather than an assumption:
    // the same file under the shipped budget IS copied, and what that copy
    // cost is larger than the budget refused above.
    let (from_memory, was_copied, generous_tiles) = drawn_rows(&path, &OpenOptions::default());
    assert!(
        was_copied,
        "the fixture was not copied under the shipped budget either, so the \
         refusal above is not evidence the budget decided anything"
    );
    let spent = copy_cost(&path);
    assert!(
        spent > MATERIALISE_BUDGET_FLOOR_BYTES,
        "the copy cost {spent} bytes, which the {MATERIALISE_BUDGET_FLOOR_BYTES}-byte \
         budget above would have admitted — the refusal was not about size"
    );

    assert_eq!(
        tiles, generous_tiles,
        "the dashboard chose {tiles} tiles off the view and {generous_tiles} \
         off the table, so the marks below are not the same marks"
    );
    assert!(
        !from_view.is_empty(),
        "no mark executed, so this test compared two empty lists"
    );
    for (index, (view, memory)) in from_view.iter().zip(&from_memory).enumerate() {
        assert_eq!(
            view, memory,
            "mark {index} drew different rows on the branch where the copy was \
             refused.\n--- off the view ---\n{view}\n--- off the table ---\n{memory}"
        );
    }
}

/// What the copy cost, in bytes, on an ordinary open of `path`.
fn copy_cost(path: &Path) -> u64 {
    let (_, trace) = data_file::open_traced(&path.to_string_lossy(), &OpenOptions::default())
        .expect("the fixture opens");
    trace
        .materialise_bytes
        .expect("an open that copied reports what the copy cost")
}

/// **A materialised open reports the time the copy took, and an open with no
/// copy reports none.**
///
/// `OpenTrace::materialise_ms` is the term the composition's clock no longer
/// carries, and it is written into every committed record. Hard-wiring it to
/// zero left the suite green, so it is read back here on both branches.
#[test]
fn a_materialised_open_reports_the_time_the_copy_took() {
    let path = fixture("rows");

    let (_, copied) = data_file::open_traced(&path.to_string_lossy(), &OpenOptions::default())
        .expect("the fixture opens");
    assert!(copied.materialised, "the fixture was not copied");
    assert!(
        copied.materialise_ms > 0.0,
        "an open that copied the file reported {} ms for the copy",
        copied.materialise_ms
    );
    assert!(
        copied.materialise_ms < copied.composition_ms + copied.materialise_ms + 60_000.0,
        "the reported copy time is not a plausible clock: {} ms",
        copied.materialise_ms
    );

    let (_, direct) = data_file::open_traced(
        &path.to_string_lossy(),
        &OpenOptions {
            materialise_under_bytes: 0,
            ..OpenOptions::default()
        },
    )
    .expect("the fixture opens off the view");
    assert!(!direct.materialised, "the fixture was copied anyway");
    assert_eq!(
        direct.materialise_ms, 0.0,
        "an open that made no copy timed one"
    );
    assert_eq!(
        direct.materialise_bytes, None,
        "an open that made no copy reported what it cost"
    );
}
