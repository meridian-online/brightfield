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
use brightfield_shell::data_file::{self, OpenOptions};

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
/// taken it away. This is the branch that keeps it: nothing is copied, the
/// view is still a view on `read_parquet`, and the dashboard composes.
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
