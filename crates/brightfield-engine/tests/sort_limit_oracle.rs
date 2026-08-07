//! The arithmetic oracle for `sort:` and its `limit:`.
//!
//! `sort: {y: -x, limit: n}` becomes an `ORDER BY` on the aggregate's output
//! column and a `LIMIT` above it. A SQL-text assertion cannot tell that apart
//! from the several plausible wrong queries — ordering by the BAND under a
//! descending direction, limiting the rows that feed the aggregation rather
//! than the bars it produces, ordering by the aggregate's ARGUMENT (a column
//! that also exists, and whose per-row values rank differently from the
//! group totals) — so every case here **executes** the emitted query against
//! the bundled DuckDB and compares the bars that come back.
//!
//! The fixture is built so each of those returns a different answer. Its four
//! bands come out in four different sequences under band order, descending
//! total, ascending total and first-seen row order; the largest single row
//! belongs to the band ranked THIRD by total, so ordering by the aggregate's
//! argument is distinguishable from ordering by the aggregate; and every band
//! but one spans several rows, so a limit applied beneath the aggregation
//! changes the totals as well as the number of bars.

use brightfield_spec::{parse_spec, Format};
use brightfield_sql::cube::derive_cube;
use brightfield_sql::emit::{emit_all_queries, emit_sources, lower_mark_plan};
use brightfield_sql::ir::{Predicate, QueryPlan, ScalarValue};
use brightfield_sql::render::render_query;
use duckdb::Connection;

/// One output bar: `(band value, aggregate)`.
type Bar = (String, f64);

/// Eight rows over four categories.
///
/// | cat | rows    | total |
/// |-----|---------|-------|
/// | a   | 1, 2    |     3 |
/// | b   | 4, 4, 4 |    12 |
/// | c   | 6       |     6 |
/// | q   | 5, 4    |     9 |
///
/// The four sequences this fixture has to keep apart:
///
/// - band order — `a b c q`
/// - descending total — `b q c a`
/// - ascending total — `a c q b`
/// - first-seen row order — `c a b q`
///
/// No two are the same, and no two share a first element, so a query taking
/// the wrong one cannot agree with the right one on the head of the list or on
/// what a `limit: 2` keeps. The largest single ROW is `c: 6` while `c` is
/// third by total, so ordering by `v` rather than by `sum(v)` also shows.
const ROWS: &str = "  - { cat: c, v: 6 }\n  \
                    - { cat: a, v: 1 }\n  \
                    - { cat: b, v: 4 }\n  \
                    - { cat: q, v: 5 }\n  \
                    - { cat: b, v: 4 }\n  \
                    - { cat: a, v: 2 }\n  \
                    - { cat: q, v: 4 }\n  \
                    - { cat: b, v: 4 }";

/// A `barX` summing `v` over the band `cat`, with `sort` written as given.
fn spec(sort: Option<&str>) -> String {
    let line = sort.map_or(String::new(), |s| format!("  sort: {s}\n"));
    format!(
        "data:\n  t:\n{ROWS}\nplot:\n- mark: barX\n  data: {{ from: t }}\n  \
         x: {{ sum: v }}\n  y: cat\n{line}"
    )
}

/// Emit mark 0, run it against an in-memory DuckDB, and return its
/// `(band, aggregate)` rows in emitted order.
fn bars(spec_yaml: &str) -> Vec<Bar> {
    let spec = parse_spec(spec_yaml, Format::Yaml)
        .expect("spec parses")
        .spec;
    let conn = Connection::open_in_memory().expect("duckdb opens");
    for ddl in emit_sources(&spec, None).expect("sources emit").statements {
        conn.execute_batch(&ddl.sql).expect("ddl runs");
    }
    let query = emit_all_queries(&spec, None)
        .into_iter()
        .next()
        .expect("one mark")
        .expect("mark lowers");
    let mut stmt = conn.prepare(&query.sql).expect("query prepares");
    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
        .expect("query runs")
        .collect::<Result<Vec<Bar>, _>>()
        .expect("rows read")
}

/// The bands only, in the order the query returned them — the order the
/// renderer draws the bars in.
fn order(spec_yaml: &str) -> Vec<String> {
    bars(spec_yaml).into_iter().map(|(band, _)| band).collect()
}

/// **AC1.** `sort: {y: -x}` orders the band by the value channel.
///
/// The control is the same spec with the `sort:` line removed: it comes back
/// in band order, which is what every ranked bar in the corpus drew before
/// this. All three sequences below are different, and the fixture's table says
/// why none of them can coincide.
#[test]
fn a_sort_orders_the_band_by_the_value() {
    assert_eq!(
        order(&spec(None)),
        vec!["a", "b", "c", "q"],
        "with no `sort:` the bars are in band order — the deterministic \
         ordering the aggregating lowerer emits"
    );
    assert_eq!(
        order(&spec(Some("{ y: -x }"))),
        vec!["b", "q", "c", "a"],
        "descending by total: 12, 9, 6, 3"
    );
    assert_eq!(
        order(&spec(Some("{ y: x }"))),
        vec!["a", "c", "q", "b"],
        "ascending is the reverse"
    );
}

/// The totals themselves are untouched by the ordering. A query that ranked
/// the bars by re-aggregating something else would show here.
#[test]
fn ordering_does_not_change_what_the_bars_measure() {
    let mut sorted = bars(&spec(Some("{ y: -x }")));
    let mut unsorted = bars(&spec(None));
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    unsorted.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(sorted, unsorted);
    assert_eq!(
        unsorted,
        vec![
            ("a".to_string(), 3.0),
            ("b".to_string(), 12.0),
            ("c".to_string(), 6.0),
            ("q".to_string(), 9.0),
        ],
        "the hand-computed totals"
    );
}

/// **AC2.** `limit: n` truncates to the top n **after** ordering.
///
/// The two bands a `limit: 2` keeps differ under every ordering this fixture
/// can be given — descending total keeps `b q`, ascending keeps `a c`, band
/// order would keep `a b` and row order `c a` — so the cut is being made on
/// the order rather than on anything the rows already had.
#[test]
fn a_limit_keeps_the_top_n_of_the_order() {
    assert_eq!(order(&spec(Some("{ y: -x, limit: 2 }"))), vec!["b", "q"]);
    assert_eq!(order(&spec(Some("{ y: x, limit: 2 }"))), vec!["a", "c"]);
    assert_eq!(
        bars(&spec(Some("{ y: -x, limit: 1 }"))),
        vec![("b".to_string(), 12.0)],
        "one bar, and it still carries the whole category's total — a limit \
         pushed beneath the aggregation would return a smaller number"
    );
}

/// A limit wider than the result set is not an error and does not pad.
#[test]
fn a_limit_wider_than_the_bars_returns_them_all() {
    assert_eq!(
        order(&spec(Some("{ y: -x, limit: 99 }"))),
        vec!["b", "q", "c", "a"]
    );
}

/// The tiebreak. Two categories on the same total have no order of their own —
/// `GROUP BY` output order is unspecified in DuckDB — and the renderer draws
/// in row order, so without a second key the picture is free to move between
/// runs and a `limit:` at the cut is a coin toss.
///
/// `b` and `c` are tied at 6 here; the band ascending is the tiebreak, so `b`
/// precedes `c` and `limit: 2` takes `b`.
#[test]
fn tied_totals_fall_back_to_the_band_order() {
    let tied = "data:\n  t:\n  - { cat: c, v: 6 }\n  - { cat: b, v: 6 }\n  \
                - { cat: a, v: 9 }\nplot:\n- mark: barX\n  data: { from: t }\n  \
                x: { sum: v }\n  y: cat\n  sort: { y: -x }\n";
    assert_eq!(order(tied), vec!["a", "b", "c"]);
    let limited = tied.replace("{ y: -x }", "{ y: -x, limit: 2 }");
    assert_eq!(order(&limited), vec!["a", "b"]);
}

/// A pre-aggregated bar ranks by its value COLUMN, with no `GROUP BY`
/// invented for it. The same wrapper over `SimpleLowerer`'s pass-through rows.
#[test]
fn a_pre_aggregated_bar_ranks_and_truncates_too() {
    let src = "data:\n  t:\n  - { cat: a, v: 2 }\n  - { cat: b, v: 9 }\n  \
               - { cat: c, v: 5 }\nplot:\n- mark: barX\n  data: { from: t }\n  \
               x: v\n  y: cat\n  sort: { y: -x, limit: 2 }\n";
    assert_eq!(
        bars(src)
            .into_iter()
            .map(|(band, _)| band)
            .collect::<Vec<_>>(),
        vec!["b", "c"]
    );
}

/// The pre-aggregation layer serves the same ranked, truncated bars the direct
/// query does.
///
/// `derive_cube` unwraps the structural wrappers above an aggregation looking
/// for one it can decompose. It knew `ORDER BY` and not `LIMIT`, so a ranked
/// bar chart with a `limit:` — the one this card exists to make possible — was
/// the single mark shape that fell back to the direct query on every brush
/// step. The cube's contract is that its result set EQUALS the direct query's,
/// so the claim to prove is not that a cube is built but that the two agree,
/// row for row and in order, with the truncation applied to the bars rather
/// than to the cube rows they are re-aggregated from.
///
/// The brush is what makes that non-trivial: unbrushed the top two bands are
/// `a` and `b`, brushed to `g = 1` they are `b` and `q`. A cube that ignored
/// the clause, or truncated before applying it, cannot return the second.
#[test]
fn the_cube_serves_the_same_ranked_bars_as_the_direct_query() {
    const SRC: &str = "params:\n  brush:\n    select: intersect\ndata:\n  t:\n  \
                       - { cat: a, v: 1, g: 1 }\n  - { cat: b, v: 4, g: 1 }\n  \
                       - { cat: c, v: 2, g: 2 }\n  - { cat: q, v: 3, g: 1 }\n  \
                       - { cat: a, v: 9, g: 2 }\nplot:\n- mark: barX\n  \
                       data: { from: t, filterBy: $brush }\n  x: { sum: v }\n  \
                       y: cat\n  sort: { y: -x, limit: 2 }\n";

    let spec = parse_spec(SRC, Format::Yaml).expect("spec parses").spec;
    let conn = Connection::open_in_memory().expect("duckdb opens");
    for ddl in emit_sources(&spec, None).expect("sources emit").statements {
        conn.execute_batch(&ddl.sql).expect("ddl runs");
    }

    // The plan without any live selection — what the pre-aggregation layer's
    // selection path hands `derive_cube`.
    let (plan, _) = lower_mark_plan(&spec, 0).expect("mark lowers");
    let active = Predicate::Interval {
        column: "\"g\"".to_string(),
        lo: ScalarValue::Int(1),
        hi: ScalarValue::Int(1),
        meta: None,
    };

    // The direct query: the clause beneath the aggregation, where emission
    // threads it, leaving the ORDER BY and the LIMIT above.
    let direct = sql_with_clause_under_aggregation(&plan, &active);
    let direct_rows = rows(&conn, &direct);
    assert_eq!(
        direct_rows,
        vec![("b".to_string(), 4.0), ("q".to_string(), 3.0)],
        "with `g = 1` the top two bands are b and q, not the a and b an \
         unbrushed query returns: {direct}"
    );

    // The served query, over a materialised cube.
    let derivation = derive_cube(&plan, &active, None)
        .expect("a ranked, limited bar over one source must decompose");
    let sqls = derivation.finalize(&[]).expect("no centering needed");
    conn.execute_batch(&format!(
        "CREATE TEMP TABLE ranked_cube AS {}",
        sqls.build_select
    ))
    .expect("cube builds");
    let served = sqls.query_select("ranked_cube").expect("re-query renders");
    assert!(
        !served.contains("\"t\""),
        "the re-query is served from the cube and never touches the source: {served}"
    );
    assert!(served.contains("LIMIT 2"), "{served}");

    assert_eq!(
        rows(&conn, &served),
        direct_rows,
        "the cube and the direct query must return the same bars in the same \
         order: {served}"
    );
}

/// Thread `predicate` onto the aggregation's input, descending through the
/// `LIMIT` and `ORDER BY` above it — the placement `emit`'s
/// `apply_selection_filter` produces.
fn sql_with_clause_under_aggregation(plan: &QueryPlan, predicate: &Predicate) -> String {
    fn thread(plan: QueryPlan, predicate: &Predicate) -> QueryPlan {
        match plan {
            QueryPlan::Limit {
                input,
                limit,
                offset,
            } => QueryPlan::Limit {
                input: Box::new(thread(*input, predicate)),
                limit,
                offset,
            },
            QueryPlan::Order { input, keys } => QueryPlan::Order {
                input: Box::new(thread(*input, predicate)),
                keys,
            },
            QueryPlan::Aggregation {
                input,
                group_by,
                aggregates,
            } => QueryPlan::Aggregation {
                input: Box::new(QueryPlan::Filter {
                    input,
                    predicate: predicate.clone(),
                }),
                group_by,
                aggregates,
            },
            other => other,
        }
    }
    let mut bindings = Vec::new();
    let sql = render_query(&thread(plan.clone(), predicate), &mut bindings);
    assert!(bindings.is_empty(), "this plan carries no parameters");
    sql
}

/// Execute `sql` and read it as `(band, aggregate)` rows.
fn rows(conn: &Connection, sql: &str) -> Vec<Bar> {
    let mut stmt = conn.prepare(sql).expect("query prepares");
    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
        .expect("query runs")
        .collect::<Result<Vec<Bar>, _>>()
        .expect("rows read")
}
