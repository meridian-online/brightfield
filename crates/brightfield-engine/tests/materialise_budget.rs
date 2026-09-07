//! The copy that makes a `file:` source cheap to draw is bounded by memory
//! DuckDB enforces, and these drive the branches of that bound.
//!
//! [`Session::materialise_source`] reads a source once into a session-scoped
//! table so that later statements scan memory instead of re-parsing a file.
//! What it costs is the table's width in memory, and **the source does not say
//! what that will be** — a four-column ZSTD Parquet of 123,260 bytes on disk
//! materialises to a 511,031,296-byte table on this build. So the size is not
//! predicted from the file. A `memory_limit` is imposed for the duration of
//! the copy, spilling is shut off beside it, and a table that does not fit
//! comes back as an error.
//!
//! Three properties, one test each:
//!
//! 1. **an over-budget copy is refused, and the refusal costs nothing** — no
//!    table left behind, the view still reading the file, the same rows;
//! 2. **a budget below the floor is raised to it**, because a small enough
//!    `memory_limit` stops DuckDB being able to lift it again and takes the
//!    session down with it;
//! 3. **a copy invalidates what was computed before it**, so no cached answer
//!    outlives the source it was computed from.

use brightfield_engine::{Engine, RecordBatch, Session, MATERIALISE_BUDGET_FLOOR_BYTES};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::{parse_spec, Format};
use duckdb::arrow::array::{Array, Int64Array};

/// A generous budget: far above anything these fixtures cost, so a refusal
/// under it would mean the mechanism refuses everything.
const GENEROUS: u64 = 512 * 1024 * 1024;

/// A query that cannot run inside [`MATERIALISE_BUDGET_FLOOR_BYTES`]: a
/// `GROUP BY` over half a million distinct 120-character strings.
///
/// **This exists because a small query cannot detect a budget left in force.**
/// The restore is the part of `materialise_source` most able to fail
/// invisibly — on this build's DuckDB, `RESET memory_limit` reports success
/// and reads back correctly while the buffer pool goes on enforcing the
/// narrowed limit — and what notices is work that does not fit in it.
const HEAVY_QUERY: &str = "SELECT count(*) FROM (SELECT a, count(*) FROM \
     (SELECT repeat('x', 120) || (i % 400000)::VARCHAR AS a \
      FROM range(500000) r(i)) GROUP BY 1) AS counted";

/// A session over `spec`, loaded the way an application loads one.
fn session(spec: &str) -> Session {
    let parsed = parse_spec(spec, Format::Yaml).expect("the spec parses");
    let analysis = analyse_spec(&parsed.spec).expect("the spec analyses");
    Engine::new()
        .load_spec(parsed.spec, analysis, None)
        .expect("the spec loads")
        .session
}

/// A source of `rows` rows whose table in memory is far larger than the
/// budget a refusing test sets: two 120-character strings per row.
///
/// Generated in SQL rather than written to a file because what is under test
/// is the copy, not the reader. `query:` sources bind a view exactly as a
/// `file:` source does, and the copy sees a view either way.
fn wide_rows(rows: u64) -> String {
    format!(
        "data:\n  t:\n    query: \"SELECT repeat('x', 120) || i::VARCHAR AS a, \
         repeat('z', 120) || i::VARCHAR AS b, i AS n FROM range({rows}) r(i)\"\n"
    )
}

/// The scalar in the first column of the first row.
///
/// `execute_uncached` rather than the mark path: what is being read is the
/// state of the session, and a cached answer would report the state it was in
/// when the cache was filled.
fn count_of(session: &mut Session, sql: &str) -> i64 {
    let batches: Vec<RecordBatch> = session.execute_uncached(sql).expect("the query runs");
    let batch = batches
        .into_iter()
        .find(|b| b.num_rows() > 0)
        .unwrap_or_else(|| panic!("no rows from {sql}"));
    let column = batch.column(0);
    let array = column
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap_or_else(|| panic!("column 0 of {sql} is {:?}", column.data_type()));
    array.value(0)
}

/// **A copy that does not fit the budget is refused, the source is exactly
/// where it was, and the budget is gone.**
///
/// This is the branch the larger-than-memory claim rests on. Before the budget
/// existed the `Err` arm this drives was unreachable: a `memory_limit` on its
/// own does not refuse an over-budget copy, it spills it to `temp_directory`
/// and returns success, which is a copy that "worked" by writing hundreds of
/// megabytes into a directory beside the process.
///
/// The vacuity guard is the second half — the same source under a generous
/// budget must be copied — because a `materialise_source` that returned `Err`
/// unconditionally would pass the first half alone.
#[test]
fn a_refused_copy_leaves_the_session_able_to_run_a_query_far_larger_than_the_budget() {
    let mut over = session(&wide_rows(400_000));
    let rows_before = count_of(&mut over, "SELECT count(*) FROM \"t\"");

    let refused = over.materialise_source("t", MATERIALISE_BUDGET_FLOOR_BYTES);
    assert!(
        refused.is_err(),
        "a table far wider than the budget was copied anyway — the budget is \
         not being imposed, or it is being spilled to disk rather than refused"
    );

    // The refusal costs nothing: no half-built table, the view still reads.
    assert_eq!(
        count_of(
            &mut over,
            "SELECT count(*) FROM duckdb_tables() \
                         WHERE table_name = 't__bf_materialised'"
        ),
        0,
        "the refused copy left its backing table behind"
    );
    assert_eq!(
        count_of(&mut over, "SELECT count(*) FROM \"t\""),
        rows_before,
        "the view stopped serving the same rows after a refused copy"
    );
    // ...and the session is usable FOR WORK LARGER THAN THE BUDGET, which is
    // the assertion this test had wrong at first and the reason it is spelled
    // out. `SELECT count(*) FROM range(7)` fits inside 8 MiB, so it passes
    // whether or not the budget was lifted; the first version of this test
    // asked exactly that and was green over a session pinned at 8 MiB.
    // `heavy_query` does not fit, so it answers the question actually being
    // asked.
    assert!(
        over.execute_uncached(HEAVY_QUERY).is_ok(),
        "after a refused copy the session could not run a query needing far \
         more than the {MATERIALISE_BUDGET_FLOOR_BYTES}-byte budget — the \
         budget is still in force, whatever `current_setting` reports"
    );

    // The guard against a mechanism that refuses everything.
    let mut under = session(&wide_rows(400_000));
    under
        .materialise_source("t", GENEROUS)
        .expect("the same source under a generous budget is copied");
    assert_eq!(
        count_of(
            &mut under,
            "SELECT count(*) FROM duckdb_tables() \
                          WHERE table_name = 't__bf_materialised'"
        ),
        1,
        "a generous budget did not produce a backing table, so the refusal \
         above is not evidence that the budget decided anything"
    );
    assert_eq!(
        count_of(&mut under, "SELECT count(*) FROM \"t\""),
        rows_before,
        "the copied source serves different rows from the view it replaced"
    );
}

/// **A budget below the floor is raised to it rather than passed through.**
///
/// Measured on this build's DuckDB: `SET memory_limit='1B'` leaves the
/// connection answering `Out of Memory Error` to `RESET memory_limit` itself,
/// so a budget of one byte does not produce a strict guard, it produces a
/// session that cannot run the next query. A copy that comfortably fits the
/// floor is therefore expected to succeed however little was asked for.
#[test]
fn a_budget_below_the_floor_is_raised_to_it_rather_than_breaking_the_session() {
    let mut small = session(&wide_rows(200));
    small
        .materialise_source("t", 1)
        .expect("a budget of one byte was passed through to DuckDB");
    assert_eq!(
        count_of(&mut small, "SELECT count(*) FROM \"t\""),
        200,
        "the source did not survive a copy asked for with a one-byte budget"
    );
    assert_eq!(
        count_of(&mut small, "SELECT count(*) FROM range(7) r(i)"),
        7,
        "the session could not run an ordinary query afterwards"
    );
}

/// **What was computed before the copy does not outlive it.**
///
/// `materialise_source` calls `invalidate_derived_state`, and dropping that
/// call leaves the SQL cache holding answers computed against the view. The
/// mark path reads that cache, so this asks the same mark twice across a copy
/// and reads DuckDB's own execute counter: a cached answer would be served
/// without a second execute.
#[test]
fn a_materialised_source_serves_no_answer_computed_before_the_copy() {
    let mut live = session(
        "data:\n  t:\n    query: \"SELECT (i % 7) AS a, i AS b FROM range(500) r(i)\"\nplot:\n  - mark: dot\n    data: { from: t }\n    x: a\n    y: b\n",
    );
    live.execute_mark(0).expect("the mark executes");
    let before = live.duckdb_execute_count();
    live.execute_mark(0).expect("the mark executes again");
    assert_eq!(
        live.duckdb_execute_count(),
        before,
        "the second execution of an unchanged mark was not served from cache, \
         so this test cannot tell a cleared cache from a warm one"
    );

    live.materialise_source("t", GENEROUS)
        .expect("the copy fits a generous budget");
    live.execute_mark(0)
        .expect("the mark executes after the copy");
    assert!(
        live.duckdb_execute_count() > before,
        "the mark was served from a cache filled before the copy — its answer \
         was computed against the view, not the table"
    );
}
