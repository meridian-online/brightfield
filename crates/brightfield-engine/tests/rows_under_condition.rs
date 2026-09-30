//! A step's rows read under a condition, and the conditions that read refuses.
//!
//! **The two reads a grid pages with take a condition, and the condition is a
//! view, not state.** `step_rows_count_where` and
//! `execute_step_rows_window_where` narrow the step's rows to the ones a SQL
//! condition keeps; nothing about the session changes, so a read without a
//! condition afterwards, and a chart drawn from the same step, see every row.
//!
//! **The refusal is a guarantee against running a second statement on the
//! session's connection.** The duckdb crate's `prepare` runs each statement in
//! its text before the last, so a condition carrying `; CREATE TABLE …` would
//! create the table. The reads count `SELECT 1 WHERE <condition>` with DuckDB's
//! own parser, as `arc` does before it records a condition, and compose only a
//! condition that is exactly one statement.
//!
//! Every count below was taken with the DuckDB CLI over the same file:
//! `SELECT count(*) FROM '<file>' WHERE <condition>`.

use brightfield_engine::error::{ConditionRefusal, EngineError};
use brightfield_engine::{Engine, RecordBatch, RowsAudience, Session};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::{parse_spec, Format};
use duckdb::arrow::array::{Array, Float64Array, Int64Array};

/// The housing file the shell's first start opens.
const HOUSING: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../brightfield-shell/assets/starts/california_housing.parquet"
);

/// Every row in [`HOUSING`].
const ALL_ROWS: u64 = 16_640;

/// The table a refused condition would create if any of it ran.
const PROBE_TABLE: &str = "brightfield_probe";

/// A session over one `dot` mark drawn straight from [`HOUSING`], loaded the
/// way an application loads one. Mark `0` is the step every test reads.
fn session() -> Session {
    let spec = format!(
        "data:\n  housing: {{ file: \"{HOUSING}\" }}\n\
         plot:\n  - mark: dot\n    data: {{ from: housing }}\n    x: median_income\n    y: house_age\n"
    );
    let parsed = parse_spec(&spec, Format::Yaml).expect("the spec parses");
    let analysis = analyse_spec(&parsed.spec).expect("the spec analyses");
    Engine::new()
        .load_spec(parsed.spec, analysis, None)
        .expect("the spec loads")
        .session
}

fn rows(batches: &[RecordBatch]) -> u64 {
    batches.iter().map(|b| b.num_rows() as u64).sum()
}

/// The values of a `DOUBLE` column across `batches`.
fn doubles(batches: &[RecordBatch], column: &str) -> Vec<f64> {
    let mut out = Vec::new();
    for batch in batches {
        let index = batch
            .schema()
            .index_of(column)
            .unwrap_or_else(|_| panic!("the page has a `{column}` column"));
        let values = batch
            .column(index)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap_or_else(|| panic!("`{column}` is a DOUBLE"));
        out.extend((0..values.len()).map(|i| values.value(i)));
    }
    out
}

/// Every page of the step under `condition`, `limit` rows at a time, until a
/// page comes back short. Asserts that no page holds more rows than it asked
/// for, which is what a comment in the condition swallowing the `LIMIT` would
/// break.
fn pages_under(session: &Session, condition: &str, limit: u64) -> Vec<RecordBatch> {
    let mut all = Vec::new();
    let mut offset = 0;
    loop {
        let page = session
            .execute_step_rows_window_where(0, offset, limit, RowsAudience::Reader, condition)
            .unwrap_or_else(|e| panic!("the page at {offset} under `{condition}` reads: {e}"));
        let held = rows(&page);
        assert!(
            held <= limit,
            "the page at {offset} under `{condition}` asked for {limit} rows and holds {held}"
        );
        all.extend(page);
        if held < limit {
            return all;
        }
        offset += limit;
    }
}

/// Whether the session's database holds a table named [`PROBE_TABLE`].
fn probe_table_exists(session: &mut Session) -> bool {
    let batches = session
        .execute_uncached(&format!(
            "SELECT count(*) FROM duckdb_tables() WHERE table_name = '{PROBE_TABLE}'"
        ))
        .expect("the catalog reads");
    let counts = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("count(*) is a BIGINT");
    counts.value(0) > 0
}

/// The refusal a read of `condition` returned, from both reads, asserted to
/// be the same and to name the condition as written.
///
/// **Whether anything ran is asserted first**, before either result is read:
/// a condition that got a statement onto the connection has failed the one
/// guarantee this file is for, whatever either read then returned.
fn refusal_of(session: &mut Session, condition: &str) -> ConditionRefusal {
    let count = session.step_rows_count_where(0, RowsAudience::Reader, condition);
    let window = session.execute_step_rows_window_where(0, 0, 100, RowsAudience::Reader, condition);
    assert!(
        !probe_table_exists(session),
        "a read under `{condition}` created `{PROBE_TABLE}`"
    );
    let from_count = match count {
        Err(EngineError::ConditionRefused {
            condition: named,
            refusal,
        }) => {
            assert_eq!(named, condition, "the count's refusal names the condition");
            refusal
        }
        other => panic!("the count under `{condition}` was not refused: {other:?}"),
    };
    let from_window = match window {
        Err(EngineError::ConditionRefused {
            condition: named,
            refusal,
        }) => {
            assert_eq!(named, condition, "the window's refusal names the condition");
            refusal
        }
        other => panic!("the window under `{condition}` was not refused: {other:?}"),
    };
    assert_eq!(
        from_count, from_window,
        "both reads refuse `{condition}` alike"
    );
    let shown = EngineError::ConditionRefused {
        condition: condition.to_string(),
        refusal: from_count.clone(),
    }
    .to_string();
    assert!(
        shown.contains(condition),
        "the refusal's message names the condition: {shown}"
    );
    from_count
}

/// **AC1.** The rows a condition keeps, paged, are the rows the file holds
/// under it, and the count agrees.
#[test]
fn a_condition_reads_the_rows_it_keeps_across_pages_and_counts_them() {
    let session = session();
    for (condition, expected, limit, holds) in [
        (
            "house_age > 40",
            3_402_u64,
            500_u64,
            (|v: f64| v > 40.0) as fn(f64) -> bool,
        ),
        ("house_age = 28", 375, 100, |v: f64| v == 28.0),
    ] {
        let pages = pages_under(&session, condition, limit);
        assert_eq!(rows(&pages), expected, "the pages under `{condition}`");
        let ages = doubles(&pages, "house_age");
        assert!(
            ages.iter().all(|&age| holds(age)),
            "every row paged under `{condition}` satisfies it"
        );
        assert_eq!(
            session
                .step_rows_count_where(0, RowsAudience::Reader, condition)
                .expect("the count reads"),
            expected,
            "the count under `{condition}`"
        );
    }
}

/// **AC2.** A read under a condition leaves nothing behind: the step read
/// without one, and the chart drawn from it, still hold every row.
#[test]
fn a_read_without_a_condition_after_one_gives_every_row_and_so_does_the_chart() {
    let mut session = session();
    assert_eq!(
        session
            .step_rows_count_where(0, RowsAudience::Reader, "house_age > 40")
            .expect("the count reads"),
        3_402
    );
    assert_eq!(rows(&pages_under(&session, "house_age > 40", 1_000)), 3_402);

    for audience in [RowsAudience::Reader, RowsAudience::Plot] {
        assert_eq!(
            session
                .step_rows_count(0, audience)
                .expect("the count reads"),
            ALL_ROWS,
            "the step's count at {audience:?}"
        );
        let page = session
            .execute_step_rows_window(0, 0, ALL_ROWS + 1, audience)
            .expect("the window reads");
        assert_eq!(rows(&page), ALL_ROWS, "the step's window at {audience:?}");
        let all = session
            .execute_step_rows(0, audience)
            .expect("the step reads");
        assert_eq!(rows(&all), ALL_ROWS, "the step's rows at {audience:?}");
    }
    let chart = session.execute_mark(0).expect("the chart draws");
    assert_eq!(rows(&chart), ALL_ROWS, "the chart drawn from the step");
}

/// **AC3.** A condition carrying a second statement is refused, by the probe
/// and not by DuckDB failing the composed query, and nothing in it ran.
#[test]
fn a_condition_carrying_a_second_statement_is_refused_and_runs_nothing() {
    let mut session = session();
    let plain = format!("house_age > 1; CREATE TABLE {PROBE_TABLE} AS SELECT 1");
    assert_eq!(
        refusal_of(&mut session, &plain),
        ConditionRefusal::SecondStatement { statements: 2 }
    );

    // An unparseable tail fails the whole probe, so DuckDB counts no
    // statements at all and gives its own reason; nothing before the tail ran.
    let tailed = format!("house_age > 1; CREATE TABLE {PROBE_TABLE} AS SELECT 1; zzz");
    assert!(
        matches!(
            refusal_of(&mut session, &tailed),
            ConditionRefusal::Unparseable { .. }
        ),
        "`{tailed}` is refused as unparseable"
    );
}

/// **AC3, the case only the probe stops.** The two conditions above would
/// also fail as composed, because the read puts the condition inside
/// parentheses and a `;` inside parentheses does not parse. This one closes
/// those parentheses itself, creates the table as a statement of its own, and
/// opens a query the read's closing text completes — so the composed text is
/// three statements that each parse, and without the probe the table exists.
/// The probe opens no parentheses, so the same text does not parse as one.
#[test]
fn a_condition_that_closes_the_reads_parentheses_is_refused_and_runs_nothing() {
    let mut session = session();
    let escaping = format!(
        "true)) AS escaped; CREATE TABLE {PROBE_TABLE} AS SELECT 1; \
         SELECT * FROM (SELECT 1 AS one WHERE (true"
    );
    assert!(
        matches!(
            refusal_of(&mut session, &escaping),
            ConditionRefusal::Unparseable { .. }
        ),
        "`{escaping}` is refused as unparseable"
    );
}

/// **AC3a.** A trailing comment in the condition ends at the condition's own
/// line and swallows nothing the read wraps after it.
#[test]
fn a_trailing_comment_swallows_neither_the_limit_nor_the_offset() {
    let session = session();
    let condition = "house_age > 40 -- note";
    let pages = pages_under(&session, condition, 1_000);
    assert_eq!(rows(&pages), 3_402);
    assert_eq!(
        session
            .step_rows_count_where(0, RowsAudience::Reader, condition)
            .expect("the count reads"),
        3_402
    );
}

/// **AC4.** A condition DuckDB cannot parse is refused with DuckDB's own
/// message, read back here from the duckdb crate's own `prepare` over the
/// same probe; a condition of two clauses is read as written.
#[test]
fn an_unparseable_condition_carries_duckdbs_message_and_a_compound_one_reads_as_written() {
    let mut session = session();
    let condition = "house_age >";
    let duckdb_says = match duckdb::Connection::open_in_memory()
        .expect("an in-memory database opens")
        .prepare(&format!("SELECT 1 WHERE {condition}"))
    {
        Err(duckdb::Error::DuckDBFailure(_, Some(message))) => message,
        Err(other) => panic!("DuckDB refused the probe without a message: {other:?}"),
        Ok(_) => panic!("DuckDB parsed `{condition}`"),
    };
    assert!(
        duckdb_says.contains("syntax error"),
        "DuckDB's message is a syntax error: {duckdb_says}"
    );
    assert_eq!(
        refusal_of(&mut session, condition),
        ConditionRefusal::Unparseable {
            message: duckdb_says
        }
    );

    let compound = "house_age > 40 and median_income < 3";
    let pages = pages_under(&session, compound, 500);
    assert_eq!(rows(&pages), 1_512, "the pages under `{compound}`");
    let ages = doubles(&pages, "house_age");
    let incomes = doubles(&pages, "median_income");
    assert!(
        ages.iter()
            .zip(&incomes)
            .all(|(&a, &i)| a > 40.0 && i < 3.0),
        "every row under `{compound}` satisfies both clauses"
    );
    assert_eq!(
        session
            .step_rows_count_where(0, RowsAudience::Reader, compound)
            .expect("the count reads"),
        1_512
    );
}
