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
use brightfield_engine::{RowsAudience, RowsView, MATERIALISE_BUDGET_FLOOR_BYTES};
use brightfield_shell::chart_kinds;
use brightfield_shell::column_record;
use brightfield_shell::data_file::{
    self, OpenOptions, OpenTrace, OpenedFile, MATERIALISE_BUDGET_BYTES, MATERIALISE_UNDER_BYTES,
};
use brightfield_shell::data_grid::fetch_page;
use brightfield_shell::pipeline::LiveDashboard;

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
    let writing = writing_path(&path);
    let conn = duckdb::Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(&format!(
        "COPY (SELECT (hash(i * 11 + 7) % 12)::DOUBLE AS bounded, \
         ((hash(i * 17 + 3) % 1000000) / 1000.0)::DOUBLE AS wide, \
         ('label_' || (hash(i * 13 + 5) % 6))::VARCHAR AS label, \
         (TIMESTAMP '2020-01-01' + INTERVAL (i) SECOND) AS at \
         FROM range(240) AS r(i)) TO '{}' (FORMAT CSV, HEADER)",
        writing.display()
    ))
    .expect("write fixture");
    std::fs::rename(&writing, &path).expect("the fixture moves into place");
    path
}

/// A name beside `path` that no other writer in this process uses, for a
/// fixture to be written under before it is renamed onto `path`.
///
/// Tests that run at once share these fixtures. Each one that finds a fixture
/// missing writes it, and DuckDB 2.0's `COPY ... TO` refuses a file another
/// writer has just created, or leaves it missing for a moment while it is
/// replaced. Writing under a private name and renaming it into place means a
/// reader only ever finds the whole file, whichever writer put it there.
fn writing_path(path: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let name = path.file_name().expect("a fixture names a file");
    path.with_file_name(format!(
        "writing-{}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed),
        name.to_string_lossy()
    ))
}

/// A Parquet of the same rows, written by the same DuckDB the engine reads it
/// back with.
fn parquet_fixture() -> PathBuf {
    let csv = fixture("rows");
    let path = csv.with_extension("parquet");
    if path.exists() {
        return path;
    }
    let writing = writing_path(&path);
    let conn = duckdb::Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(&format!(
        "COPY (SELECT * FROM read_csv('{}')) TO '{}' (FORMAT PARQUET)",
        csv.display(),
        writing.display()
    ))
    .expect("write parquet");
    std::fs::rename(&writing, &path).expect("the fixture moves into place");
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

    // Without a type source, so no column is labelled. A labelled column is cast in the copy and
    // only there, which the cast's own tests below cover; here the copy must change nothing.
    let unlabelled = OpenOptions {
        type_source: None,
        ..OpenOptions::default()
    };
    let (materialised, was_materialised, tiles) = drawn_rows(&path, &unlabelled);
    let (direct, was_direct, direct_tiles) = drawn_rows(
        &path,
        &OpenOptions {
            materialise_under_bytes: 0,
            ..unlabelled.clone()
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
    let writing = writing_path(&path);
    let conn = duckdb::Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(&format!(
        "COPY (SELECT lpad(('x' || (i % 7))::VARCHAR, 200, 'y') AS label, \
         lpad((i % 97)::VARCHAR, 3, '0') AS value FROM range({rows}) AS r(i)) \
         TO '{}' (FORMAT CSV, HEADER)",
        writing.display()
    ))
    .expect("write fixture");
    std::fs::rename(&writing, &path).expect("the fixture moves into place");
    path
}

/// Bytes a [`sized_csv`] of `rows` rows occupies: `label,value\n` and then
/// 200 + 1 + 3 + 1 per row.
fn sized_csv_bytes(rows: u64) -> u64 {
    12 + 205 * rows
}

/// **The threshold the build ships is the one an ordinary open applies.**
///
/// The other tests on this branch reach it by passing
/// `materialise_under_bytes` explicitly, which is what makes the branch
/// reachable and also what leaves the shipped constant unread: setting
/// `MATERIALISE_UNDER_BYTES` to `u64::MAX` left them green. This one goes
/// through `OpenOptions::default()` on two files one row apart, either side of
/// the constant, so the number itself decides the outcome.
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
    let writing = writing_path(&path);
    let conn = duckdb::Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(&format!(
        "COPY (SELECT ('cat_' || (i % 8))::VARCHAR AS label, \
         ('grp_' || (i % 5))::VARCHAR AS kind, (i % 97)::BIGINT AS value, \
         (i % 13)::BIGINT AS other FROM range(500000) AS r(i)) \
         TO '{}' (FORMAT PARQUET, COMPRESSION ZSTD)",
        writing.display()
    ))
    .expect("write parquet");
    std::fs::rename(&writing, &path).expect("the fixture moves into place");
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
/// carries, and it is what the committed records report as `materialise`.
/// Hard-wiring it to zero left the suite green, so it is read back here on
/// both branches.
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

// ---------------------------------------------------------------------------
// The shipped budget, and the boundary the threshold is stated at
// ---------------------------------------------------------------------------

/// The sum of `total_uncompressed_size` over a Parquet's row groups: the
/// decompressed size of its *encoded* columns, and the cheapest better
/// estimate of a copy's cost than the file's size on disk.
fn parquet_footer_bytes(path: &Path) -> u64 {
    let conn = duckdb::Connection::open_in_memory().expect("duckdb");
    conn.query_row(
        &format!(
            "SELECT sum(total_uncompressed_size)::BIGINT FROM parquet_metadata('{}')",
            path.display()
        ),
        [],
        |row| row.get::<_, i64>(0),
    )
    .ok()
    .and_then(|n| u64::try_from(n).ok())
    .expect("the fixture's Parquet footer reads")
}

/// **Neither the file's size on disk nor its Parquet footer says what the copy
/// will cost, and both understate it by orders of magnitude.**
///
/// This is the measurement the two constants are shaped around, and it is a
/// test rather than a sentence because both round 1 and round 2 carried the
/// figures in prose, from a fixture that was never committed. The fixture here
/// is [`widening_parquet`], which the refusal test already opens.
///
/// Measured on an Apple M1 Pro against this build's DuckDB v1.5.2: 12,419
/// bytes on disk, a footer of 1,139,201 bytes, and a 24,649,728-byte table —
/// 1,985 times the file and 21.6 times the footer. The assertions below are
/// loose bounds on those ratios rather than the figures themselves, because
/// the figures are a property of a platform and the ordering is the property
/// the design rests on.
#[test]
fn the_size_on_disk_and_the_footer_both_understate_what_the_copy_costs() {
    let path = widening_parquet();
    let on_disk = std::fs::metadata(&path).expect("stat").len();
    let footer = parquet_footer_bytes(&path);
    let in_memory = copy_cost(&path);

    assert!(
        footer > on_disk,
        "the footer ({footer}) is not larger than the file ({on_disk}), so \
         this fixture is not compressed and says nothing about either estimate"
    );
    assert!(
        in_memory > on_disk * 500,
        "the table is {in_memory} bytes against {on_disk} on disk, under 500 \
         times — the fixture no longer demonstrates that on-disk size cannot \
         carry a memory bound, and the prose citing it is now wrong"
    );
    assert!(
        in_memory > footer * 10,
        "the table is {in_memory} bytes against a footer of {footer}, under 10 \
         times — the Parquet footer would now be a usable estimate, and the \
         reason for rejecting it no longer holds"
    );
}

/// A Parquet whose table is far over [`MATERIALISE_BUDGET_BYTES`] and whose
/// file is far under [`MATERIALISE_UNDER_BYTES`].
///
/// **Both halves matter.** Over the budget is what makes it the refusing case;
/// under the on-disk threshold is what makes the *budget* the thing that
/// refused it, rather than the threshold declining to try.
///
/// The row count is set from a measurement of the same four columns at a size
/// that still completes: on an Apple M1 Pro, 12,000,000 rows copy to
/// 574,046,208 bytes, already over the 512-MiB budget. 16,000,000 is used
/// instead for margin, and measures 331,306 bytes on disk. Its own table is
/// never measured, because the budget refuses the copy before it finishes —
/// which is the behaviour under test.
fn widening_parquet_over_the_budget() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bf-materialise-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join("over-budget.parquet");
    if path.exists() {
        return path;
    }
    let writing = writing_path(&path);
    let conn = duckdb::Connection::open_in_memory().expect("duckdb");
    conn.execute_batch(&format!(
        "COPY (SELECT ('cat_' || (i % 8))::VARCHAR AS label, \
         ('grp_' || (i % 5))::VARCHAR AS kind, (i % 97)::BIGINT AS value, \
         (i % 13)::BIGINT AS other FROM range(16000000) AS r(i)) \
         TO '{}' (FORMAT PARQUET, COMPRESSION ZSTD)",
        writing.display()
    ))
    .expect("write parquet");
    std::fs::rename(&writing, &path).expect("the fixture moves into place");
    path
}

/// **The budget the build ships is the one an ordinary open applies.**
///
/// The twin of `the_shipped_threshold_is_what_decides_an_ordinary_open`, and
/// it exists for the same reason: every other test of the budget reaches its
/// branches by passing `materialise_budget_bytes` explicitly, so setting
/// `MATERIALISE_BUDGET_BYTES` to `u64::MAX` left them green — the constant the
/// whole round was about was read by nothing. Two Parquets either side of it,
/// both through [`OpenOptions::default`], put the number itself in the path.
#[test]
fn the_shipped_budget_is_what_decides_an_ordinary_open() {
    // The affordability guard, as on the threshold's twin: the over-budget
    // fixture below is sized for a budget in this range, and a constant
    // outside it reddens here rather than by writing a file nobody can hold.
    assert!(
        (64 * 1024 * 1024..=512 * 1024 * 1024).contains(&MATERIALISE_BUDGET_BYTES),
        "the shipped budget is {MATERIALISE_BUDGET_BYTES} bytes; \
         widening_parquet_over_the_budget is sized to exceed 512 MiB and no \
         more, so a budget outside that range needs the fixture resized with it"
    );

    let under = widening_parquet();
    let (_, was_copied, _) = drawn_rows(&under, &OpenOptions::default());
    assert!(
        was_copied,
        "a fixture whose table is well inside the shipped budget was not \
         copied by an ordinary open"
    );

    let over = widening_parquet_over_the_budget();
    let on_disk = std::fs::metadata(&over).expect("stat").len();
    assert!(
        on_disk < MATERIALISE_UNDER_BYTES,
        "the over-budget fixture is {on_disk} bytes on disk, over the \
         {MATERIALISE_UNDER_BYTES} threshold — the threshold would decline the \
         copy and this test would say nothing about the budget"
    );

    let (drawn, was_copied, tiles) = drawn_rows(&over, &OpenOptions::default());
    assert!(
        !was_copied,
        "a fixture whose table is far over the shipped budget of \
         {MATERIALISE_BUDGET_BYTES} bytes was copied by an ordinary open"
    );
    assert!(
        tiles > 0 && !drawn.is_empty(),
        "the refused fixture drew {tiles} tiles and {} marks, so the open did \
         not survive the refusal",
        drawn.len()
    );
}

/// **A file of exactly the threshold is copied**, which is the `<=` in
/// [`data_file::open_traced`] and what the option's own documentation claims.
///
/// Driven at the size of a small fixture rather than at the shipped 64 MiB,
/// because what is under test is the comparison and not the constant: the
/// threshold is passed as the file's own byte count, and then one byte less.
/// `the_shipped_budget_is_what_decides_an_ordinary_open` and its threshold
/// twin are where the shipped numbers are read.
#[test]
fn a_file_of_exactly_the_threshold_is_copied_and_one_byte_under_it_is_not() {
    let path = fixture("rows");
    let bytes = std::fs::metadata(&path).expect("stat").len();
    assert!(bytes > 1, "the fixture is empty");

    let (_, at_the_boundary, _) = drawn_rows(
        &path,
        &OpenOptions {
            materialise_under_bytes: bytes,
            ..OpenOptions::default()
        },
    );
    assert!(
        at_the_boundary,
        "a {bytes}-byte file was not copied under a threshold of exactly \
         {bytes} — the comparison is `<` where the documentation says `<=`"
    );

    let (_, one_byte_short, _) = drawn_rows(
        &path,
        &OpenOptions {
            materialise_under_bytes: bytes - 1,
            ..OpenOptions::default()
        },
    );
    assert!(
        !one_byte_short,
        "a {bytes}-byte file was copied under a threshold of {}, so the \
         comparison admits a file larger than the threshold",
        bytes - 1
    );
}

// ---------------------------------------------------------------------------
// The copy casts what FineType's projection casts
// ---------------------------------------------------------------------------
//
// A lone file's copy casts each column whose label FineType's projection casts, when every value
// casts. The labels here come from a column record that nominates them, as an analyst who accepted
// them would have, so no FineType bundle is needed; the casts come from the transforms vendored
// into `brightfield-protocol`.

/// The committed fixture: sterling amounts with accounting negatives, dollar prices, percentages,
/// rates, years, counts, ISO dates and free text, written by `data/typed_formats.py`.
const TYPED_FORMATS: &str = include_str!("data/typed_formats.csv");

/// The label each of the fixture's columns is given. `notes` has none.
const TYPED_LABELS: &[(&str, &str)] = &[
    ("revenue", "finance.currency.amount"),
    ("price_usd", "finance.currency.amount"),
    ("share", "representation.numeric.percentage"),
    ("yield", "finance.rate.yield"),
    ("year", "datetime.component.year"),
    ("downloads", "representation.numeric.integer_number"),
    ("day", "datetime.date.iso"),
];

/// Point the config folder, where a lone file's column record is kept, at a scratch folder of
/// this process's, once and before any test reads it.
fn scratch_config() {
    use std::sync::OnceLock;
    static CONFIG: OnceLock<PathBuf> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("bf-materialise-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch config folder");
        std::env::set_var(brightfield_shell::startup::CONFIG_DIR_VAR, &dir);
        dir
    });
}

/// `text` written as a CSV in a folder named `name`, with a record nominating [`TYPED_LABELS`].
fn typed(name: &str, text: &str) -> PathBuf {
    typed_with(name, text, TYPED_LABELS)
}

/// `text` written as a CSV in a folder named `name`, with a record nominating `labels`.
fn typed_with(name: &str, text: &str, labels: &[(&str, &str)]) -> PathBuf {
    scratch_config();
    let dir = std::env::temp_dir()
        .join(format!("bf-typed-formats-{}", std::process::id()))
        .join(name);
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join("typed_formats.csv");
    std::fs::write(&path, text).expect("write fixture");
    let mut record = column_record::SourceRecord::default();
    for (column, label) in labels {
        record.fields.insert(
            (*column).to_string(),
            column_record::FieldRecord {
                label: Some((*label).to_string()),
                nominated: true,
                ..column_record::FieldRecord::default()
            },
        );
    }
    let package = column_record::flat_record_path(&path).expect("a record path");
    column_record::write_record(&package, data_file::SOURCE, &record).expect("write record");
    path
}

/// The options these opens take: no type source, so a bundle beside the test binary changes
/// nothing, and the labels are the record's alone.
fn unbundled() -> OpenOptions {
    OpenOptions {
        type_source: None,
        ..OpenOptions::default()
    }
}

fn open_typed(path: &Path, options: &OpenOptions) -> (OpenedFile, OpenTrace) {
    data_file::open_traced(&path.to_string_lossy(), options).expect("the fixture opens")
}

/// Every row `sql` returns from the opened file's session, each cell as text.
fn rows_of(live: &mut LiveDashboard, sql: &str) -> Vec<Vec<Option<String>>> {
    use arrow::array::{Array, StringArray};
    let batches = live
        .coordinator()
        .session_mut()
        .execute_uncached(sql)
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    let mut out = Vec::new();
    for batch in &batches {
        let text: Vec<StringArray> = batch
            .columns()
            .iter()
            .map(|c| {
                arrow::compute::cast(c, &arrow::datatypes::DataType::Utf8)
                    .expect("a cell casts to text")
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .expect("text")
                    .clone()
            })
            .collect();
        for row in 0..batch.num_rows() {
            out.push(
                text.iter()
                    .map(|c| (!c.is_null(row)).then(|| c.value(row).to_string()))
                    .collect(),
            );
        }
    }
    out
}

/// The type the opened copy holds `column` as.
fn type_of(live: &mut LiveDashboard, column: &str) -> String {
    let rows = rows_of(
        live,
        &format!(
            "SELECT column_type FROM (DESCRIBE \"{}\") WHERE column_name = '{column}'",
            data_file::SOURCE
        ),
    );
    rows.first()
        .and_then(|r| r[0].clone())
        .unwrap_or_else(|| panic!("no column {column}"))
}

/// The kind of the tile drawn of `column`, or `None` when the dashboard left it out.
fn tile_kind(opened: &OpenedFile, column: &str) -> Option<String> {
    opened
        .dashboard
        .tiles()
        .iter()
        .find(|t| t.column() == column)
        .map(|t| t.kind().to_string())
}

/// The record kept for `path` after it opened.
fn record_of(path: &Path) -> column_record::SourceRecord {
    let package = column_record::flat_record_path(path).expect("a record path");
    column_record::read_record(&package, data_file::SOURCE)
        .expect("the record reads")
        .expect("there is a record")
}

#[test]
fn flat_open_casts_a_currency_column() {
    let path = typed("casts", TYPED_FORMATS);
    let (mut opened, trace) = open_typed(&path, &unbundled());
    assert!(trace.materialised, "the fixture was not copied");
    assert!(
        type_of(&mut opened.live, "revenue").starts_with("DECIMAL"),
        "revenue is {}",
        type_of(&mut opened.live, "revenue")
    );
    assert_eq!(
        tile_kind(&opened, "revenue").as_deref(),
        Some(chart_kinds::BINNED_HISTOGRAM.to_string().as_str())
    );
    // The profile the dashboard was chosen from is the cast column's.
    let revenue = opened
        .protocol
        .profiles
        .iter()
        .find(|c| c.name == "revenue")
        .expect("revenue is listed");
    assert!(revenue.type_name.starts_with("DECIMAL"), "{revenue:?}");
}

#[test]
fn raw_text_survives_the_cast() {
    let path = typed("raw", TYPED_FORMATS);
    let (mut opened, _) = open_typed(&path, &unbundled());
    let raw = format!("{}revenue", data_file::RAW_PREFIX);
    let rows = rows_of(
        &mut opened.live,
        &format!("SELECT \"{raw}\", \"revenue\" FROM \"opened\" WHERE \"{raw}\" LIKE '(%'"),
    );
    assert!(!rows.is_empty(), "no parenthesised raw value survived");
    assert_eq!(rows[0][0].as_deref(), Some("(£12.00)"));
    assert_eq!(rows[0][1].as_deref(), Some("-12.00"));

    let page = fetch_page(
        opened.live.coordinator().session(),
        0,
        0..5,
        RowsAudience::Reader,
        &RowsView::default(),
    )
    .expect("the grid reads");
    let names: Vec<&str> = page.columns.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"revenue"), "{names:?}");
    assert!(
        names.iter().all(|n| !n.starts_with(data_file::RAW_PREFIX)),
        "the grid lists a raw column: {names:?}"
    );
    assert_eq!(page.rows[0].len(), names.len());
}

#[test]
fn one_bad_value_keeps_the_column_text() {
    let text = TYPED_FORMATS.replacen("$12.50", "n/a", 1);
    assert_ne!(text, TYPED_FORMATS);
    let path = typed("bad-value", &text);
    let (mut cast, _) = open_typed(&path, &unbundled());
    assert_eq!(type_of(&mut cast.live, "price_usd"), "VARCHAR");
    // Its neighbours still cast.
    assert!(type_of(&mut cast.live, "revenue").starts_with("DECIMAL"));
    // The column is back in its place, read as it was.
    let rows = rows_of(
        &mut cast.live,
        "SELECT \"price_usd\" FROM \"opened\" LIMIT 1",
    );
    assert_eq!(rows[0][0].as_deref(), Some("n/a"));
    let names: Vec<String> = rows_of(
        &mut cast.live,
        "SELECT column_name FROM (DESCRIBE \"opened\")",
    )
    .into_iter()
    .filter_map(|r| r[0].clone())
    .filter(|n| !n.starts_with(data_file::RAW_PREFIX))
    .collect();
    assert_eq!(
        names,
        [
            "revenue",
            "price_usd",
            "share",
            "yield",
            "year",
            "downloads",
            "day",
            "notes"
        ]
    );

    // Its tile is the one it got before the flat route cast anything.
    let (mut uncast, _) = open_typed(
        &path,
        &OpenOptions {
            already_cast: true,
            ..unbundled()
        },
    );
    assert_eq!(
        tile_kind(&cast, "price_usd"),
        tile_kind(&uncast, "price_usd")
    );
    // An open taken as already cast casts nothing.
    assert_eq!(type_of(&mut uncast.live, "revenue"), "VARCHAR");
}

#[test]
fn mixed_symbols_record_none() {
    let text = TYPED_FORMATS.replacen("£200,000,000.00", "$200,000,000.00", 1);
    assert_ne!(text, TYPED_FORMATS);
    let path = typed("mixed-symbols", &text);
    let (mut opened, _) = open_typed(&path, &unbundled());
    assert!(type_of(&mut opened.live, "revenue").starts_with("DECIMAL"));
    let record = record_of(&path);
    assert_eq!(record.fields["revenue"].written.currency_symbol, None);
    assert_eq!(
        record.fields["price_usd"]
            .written
            .currency_symbol
            .as_deref(),
        Some("$")
    );
}

#[test]
fn parentheses_are_recorded() {
    let path = typed("parentheses", TYPED_FORMATS);
    let _ = open_typed(&path, &unbundled());
    let record = record_of(&path);
    let revenue = &record.fields["revenue"];
    assert_eq!(
        revenue.written.negatives,
        Some(column_record::Negatives::Parentheses)
    );
    assert_eq!(revenue.written.currency_symbol.as_deref(), Some("£"));
    // The accepted label is still the accepted label.
    assert!(revenue.nominated);
    assert_eq!(revenue.label.as_deref(), Some("finance.currency.amount"));
    assert_eq!(
        record.source.and_then(|s| s.path),
        Some(path.canonicalize().unwrap()),
        "the record is stamped with the file it was read from"
    );
}

#[test]
fn over_budget_file_is_not_cast() {
    let path = typed("not-copied", TYPED_FORMATS);
    let (mut opened, trace) = open_typed(
        &path,
        &OpenOptions {
            materialise_under_bytes: 0,
            ..unbundled()
        },
    );
    assert!(!trace.materialised);
    assert_eq!(type_of(&mut opened.live, "revenue"), "VARCHAR");
    assert!(trace.cast.statements.is_empty());
    let record = record_of(&path);
    assert_eq!(
        record.fields["revenue"].written,
        column_record::Written::default()
    );
}

#[test]
fn cast_open_reads_the_file_once() {
    let path = typed("read-once", TYPED_FORMATS);
    let (_, trace) = open_typed(
        &path,
        &OpenOptions {
            count_scans: true,
            ..unbundled()
        },
    );
    let statements = &trace.cast.statements;
    let copies: Vec<_> = statements
        .iter()
        .filter(|s| s.sql.starts_with("CREATE TEMP TABLE"))
        .collect();
    assert_eq!(copies.len(), 1, "{statements:#?}");
    assert_eq!(copies[0].file_reads, Some(1), "{:?}", copies[0]);
    // Every statement after the copy reads the copy. A `DESCRIBE` is left out of the count: the
    // plan of one is a column-data scan, which the tally counts as a file read whatever it
    // describes (see `plan_file_reads` in the engine's profile module).
    let after: Vec<_> = statements
        .iter()
        .filter(|s| !s.sql.starts_with("CREATE TEMP TABLE") && !s.sql.starts_with("DESCRIBE"))
        .collect();
    assert!(
        after.iter().any(|s| s.sql.contains("__raw__revenue")),
        "no aggregate over the copy was counted: {statements:#?}"
    );
    for s in after {
        assert_eq!(s.file_reads, Some(0), "{s:?}");
    }
}

#[test]
fn typed_dates_survive() {
    let path = typed("dates", TYPED_FORMATS);
    let (mut opened, _) = open_typed(&path, &unbundled());
    assert_eq!(type_of(&mut opened.live, "day"), "DATE");
    let raw = format!("{}day", data_file::RAW_PREFIX);
    let rows = rows_of(
        &mut opened.live,
        &format!("SELECT count(*) FROM (DESCRIBE \"opened\") WHERE column_name = '{raw}'"),
    );
    assert_eq!(
        rows[0][0].as_deref(),
        Some("0"),
        "a DATE column was cast again"
    );
}

/// A cast file whose timestamp is drawn through a derived bucket column still draws every mark,
/// off the copy.
#[test]
fn a_cast_file_still_derives_a_bucket_column() {
    let mut text = String::from("revenue,at\n");
    for i in 0..240 {
        text.push_str(&format!(
            "\"£{},{:03}.50\",2020-01-01 00:{:02}:{:02}\n",
            i + 1,
            (i * 7) % 1000,
            i / 60,
            i % 60
        ));
    }
    let path = typed("derived", &text);
    let (mut opened, trace) = open_typed(&path, &unbundled());
    assert!(trace.materialised);
    assert!(type_of(&mut opened.live, "revenue").starts_with("DECIMAL"));
    assert!(
        opened
            .dashboard
            .tiles()
            .iter()
            .any(|t| t.resampled().is_some()),
        "the timestamp was not resampled, so nothing was derived"
    );
    let marks = opened.live.coordinator().session().mark_count();
    for index in 0..marks {
        opened
            .live
            .coordinator()
            .session_mut()
            .execute_mark(index)
            .unwrap_or_else(|e| panic!("mark {index}: {e}"));
    }
}

/// The grid's column names for the first page `live` shows.
fn grid_names(live: &mut LiveDashboard) -> Vec<String> {
    fetch_page(
        live.coordinator().session(),
        0,
        0..5,
        RowsAudience::Reader,
        &RowsView::default(),
    )
    .expect("the grid reads")
    .columns
    .into_iter()
    .map(|c| c.name)
    .collect()
}

/// **A session loaded again from the dashboard's spec reads the columns as cast**: the spec
/// declares the cast, so a tile switch's or a shelf edit's rebuild, and the written spec opened
/// again, see `revenue` as the number the tiles were chosen for.
#[test]
fn a_spec_loaded_again_reads_the_cast_columns() {
    let path = typed("reloaded", TYPED_FORMATS);
    let (opened, _) = open_typed(&path, &unbundled());

    // The rebuild a tile switch and the shelf take: a new session over the live spec.
    let mut rebuilt = LiveDashboard::load(opened.live.spec().clone(), opened.live.base_dir())
        .expect("the spec loads again");
    assert!(
        type_of(&mut rebuilt, "revenue").starts_with("DECIMAL"),
        "a rebuild reads revenue as {}",
        type_of(&mut rebuilt, "revenue")
    );
    assert_eq!(type_of(&mut rebuilt, "price_usd").get(..7), Some("DECIMAL"));
    let names = grid_names(&mut rebuilt);
    assert!(names.contains(&"revenue".to_string()), "{names:?}");
    assert!(
        names.iter().all(|n| !n.starts_with(data_file::RAW_PREFIX)),
        "the rebuilt grid lists a raw column: {names:?}"
    );
    rebuilt.present().expect("the rebuilt dashboard composes");

    // The spec written for the editor, opened on its own.
    let written = opened.spec_file.as_ref().expect("the spec was written");
    let text = std::fs::read_to_string(written).expect("the spec reads");
    let mut reloaded = LiveDashboard::load_str(&text, None).expect("the written spec loads");
    assert!(type_of(&mut reloaded, "revenue").starts_with("DECIMAL"));
    reloaded.present().expect("the written spec composes");
}

/// **A column DuckDB already read as a number is not cast through its text**, which would turn
/// `1e16` into `116.00` and round `123.456`.
#[test]
fn a_stored_number_keeps_its_values() {
    let path = typed_with(
        "stored-number",
        "amount,region\n123.456,North\n1e16,South\n7.5,East\n",
        &[("amount", "finance.currency.amount")],
    );
    let (mut opened, _) = open_typed(&path, &unbundled());
    assert_eq!(type_of(&mut opened.live, "amount"), "DOUBLE");
    let rows = rows_of(
        &mut opened.live,
        "SELECT count(*) FROM \"opened\" WHERE \"amount\" IN (123.456, 1e16, 7.5)",
    );
    assert_eq!(rows[0][0].as_deref(), Some("3"));
}

/// **A file's own column is kept whatever it is called**: one named like an engine column survives
/// the cast copy, and one named like a raw column is listed by the grid.
#[test]
fn a_files_own_columns_survive_whatever_their_names() {
    let path = typed_with(
        "own-names",
        "revenue,__bf_extra,__raw__note\n\"£1,234.50\",a,x\n(£12.00),b,y\n£7.75,c,z\n",
        &[("revenue", "finance.currency.amount")],
    );
    let (mut opened, _) = open_typed(&path, &unbundled());
    assert!(type_of(&mut opened.live, "revenue").starts_with("DECIMAL"));
    assert_eq!(type_of(&mut opened.live, "__bf_extra"), "VARCHAR");
    let names = grid_names(&mut opened.live);
    assert!(names.contains(&"__raw__note".to_string()), "{names:?}");
    assert!(!names.contains(&"__raw__revenue".to_string()), "{names:?}");
}

/// **A changed file loses what was read from its old values**, cast or not: the record is checked
/// against the file on every open.
#[test]
fn a_changed_file_loses_what_was_read_from_it() {
    let path = typed("changed", TYPED_FORMATS);
    let _ = open_typed(&path, &unbundled());
    assert_eq!(
        record_of(&path).fields["revenue"].written.negatives,
        Some(column_record::Negatives::Parentheses)
    );
    let mut longer = TYPED_FORMATS.to_string();
    longer.push_str("\"£1.00\",$1.00,1%,4.1%,2001,1,2024-12-31,one more\n");
    std::fs::write(&path, longer).expect("the file changes");
    // Not copied, so not cast: only the check against the file can clear the record.
    let _ = open_typed(
        &path,
        &OpenOptions {
            materialise_under_bytes: 0,
            ..unbundled()
        },
    );
    let revenue = &record_of(&path).fields["revenue"];
    assert_eq!(revenue.written, column_record::Written::default());
    assert!(revenue.nominated, "the accepted label is kept");
}

/// **A transform that spans lines is written into the spec as it is**: the spec stays valid YAML
/// and its SQL reads back byte for byte, line breaks included.
#[test]
fn a_multi_line_transform_round_trips_through_the_written_spec() {
    const HEIGHT: &str = "identity.person.height";
    let path = typed_with(
        "multi-line",
        "height,name\n180cm,a\n175cm,b\n162cm,c\n",
        &[("height", HEIGHT)],
    );
    let (mut opened, _) = open_typed(&path, &unbundled());
    assert_eq!(type_of(&mut opened.live, "height"), "DOUBLE");

    let expr = brightfield_protocol::silver::cast_expr_over_text(
        "height",
        Some(HEIGHT),
        &brightfield_protocol::silver::FinetypeTransforms::vendored(),
    )
    .expect("FineType casts a height");
    assert!(expr.contains('\n'), "the transform no longer spans lines");

    let written = opened.spec_file.as_ref().expect("the spec was written");
    let text = std::fs::read_to_string(written).expect("the spec reads");
    let doc: serde_yaml::Value = serde_yaml::from_str(&text).expect("the spec is YAML");
    let sql = doc["data"][data_file::SOURCE]
        .as_str()
        .expect("the source is SQL");
    assert!(
        sql.contains(&format!("{expr} AS \"height\"")),
        "the transform did not read back as written:\n{sql}"
    );
    let mut reloaded = LiveDashboard::load_str(&text, None).expect("the written spec loads");
    assert_eq!(type_of(&mut reloaded, "height"), "DOUBLE");
}

/// **A label whose cast no format needs is not cast, so it cannot stop the others**: a JSON
/// column's cast may not even bind (a DuckDB without its JSON extension has no such type), and a
/// copy that fails to bind casts nothing. The JSON and UUID columns are read as they are, and
/// `revenue` beside them is still cast.
#[test]
fn a_label_no_format_needs_is_read_as_it_is() {
    let path = typed_with(
        "unformatted-labels",
        "revenue,payload,id\n\
         \"£1,234.50\",\"{\"\"a\"\": 1}\",0b6c1f8e-6f0e-4a7c-9a52-0c7f1a3f8e11\n\
         (£12.00),\"{\"\"a\"\": 2}\",5d3c2b1a-1111-4222-8333-944455556666\n\
         £7.75,\"{\"\"a\"\": 3}\",9e8d7c6b-aaaa-4bbb-8ccc-dddddddddddd\n",
        &[
            ("revenue", "finance.currency.amount"),
            ("payload", "container.object.json"),
            ("id", "representation.identifier.uuid"),
        ],
    );
    let (mut opened, _) = open_typed(&path, &unbundled());
    assert!(
        type_of(&mut opened.live, "revenue").starts_with("DECIMAL"),
        "revenue is {}",
        type_of(&mut opened.live, "revenue")
    );
    assert_eq!(type_of(&mut opened.live, "payload"), "VARCHAR");
    assert_eq!(type_of(&mut opened.live, "id"), "VARCHAR");
    let names = grid_names(&mut opened.live);
    assert!(
        names.iter().all(|n| !n.starts_with(data_file::RAW_PREFIX)),
        "a column no format needs kept a raw twin: {names:?}"
    );
    let rows = rows_of(
        &mut opened.live,
        "SELECT \"payload\" FROM \"opened\" LIMIT 1",
    );
    assert_eq!(rows[0][0].as_deref(), Some("{\"a\": 1}"));
}
