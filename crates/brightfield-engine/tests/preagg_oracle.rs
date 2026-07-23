//! The pre-aggregation correctness oracle, translated from the upstream
//! Mosaic preaggregator test corpus.
//!
//! The cube layer's semantics are adopted from
//! [uwdata/mosaic](https://github.com/uwdata/mosaic) (BSD-3-Clause, © UW
//! Interactive Data Lab) — see the credit in `brightfield_sql::cube`. This
//! file carries that project's preaggregator test corpus as the correctness
//! oracle: the same five-row dataset, the same point selection (`dim = 'b'`),
//! and the same expected values, translated case by case onto this engine's
//! query-plan IR and cube deriver. No upstream code is vendored.
//!
//! Every case lands in one of two classes, and both are executed against
//! DuckDB, not just shape-checked:
//!
//! - **Served** — the plan decomposes: the cube is materialised, the re-query
//!   runs over it and never touches the base table, and its answer equals
//!   BOTH the direct query's and the corpus's expected value.
//! - **Fallback** — derivation bails (`None`) and the direct query still
//!   answers the expected value: the interaction never breaks, it only
//!   slows.
//!
//! The fallback class includes this implementation's deliberate divergences
//! from upstream, documented on each case: aggregates outside the set this
//! engine's lowerers emit (geomean, product, argmax/argmin, the variance
//! family, covariance, corr, regr_avgy, regr_r2 — their decompositions are
//! adopted spec, waiting on a lowerer that emits them), aggregate-valued
//! expressions, subqueries, positional column references, window / QUALIFY /
//! HAVING shapes, unaliased expression-valued group dimensions, and DISTINCT
//! aggregates (which upstream likewise refuses to optimise).

use brightfield_sql::cube::derive_cube;
use brightfield_sql::ir::{
    AggregateCall, AggregateExpr, AggregateFunction, Predicate, QueryPlan, ScalarValue, SortDir,
};
use brightfield_sql::render::render_query;
use duckdb::Connection;

/// The corpus dataset: five rows, two dims, `x`/`y` with one NULL pair.
fn conn() -> Connection {
    let c = Connection::open_in_memory().expect("duckdb");
    c.execute_batch(
        "CREATE TABLE testData AS SELECT * FROM (VALUES
            ('a', 'c', 1, 1, 9),
            ('a', 'c', 2, 2, 8),
            ('b', 'd', 1, 3, 7),
            ('b', 'd', 2, 4, 6),
            ('b', 'd', 3, NULL, NULL)
         ) AS t(dim, cat, \"order\", x, y)",
    )
    .expect("test data");
    c
}

/// The corpus's selection: a point clause on `dim = 'b'`.
fn point_dim_b() -> Predicate {
    Predicate::Point {
        column: "\"dim\"".to_string(),
        values: vec![ScalarValue::Text("b".to_string())],
        meta: None,
    }
}

fn agg(func: AggregateFunction, args: &[&str], filter: Option<&str>) -> AggregateExpr {
    AggregateExpr::Call(AggregateCall {
        func,
        args: args.iter().map(|s| (*s).to_string()).collect(),
        filter: filter.map(str::to_string),
        cast: None,
        alias: Some("measure".to_string()),
    })
}

fn raw(expr: &str) -> AggregateExpr {
    AggregateExpr::Raw(format!("{expr} AS measure"))
}

fn source() -> Box<QueryPlan> {
    Box::new(QueryPlan::Source {
        table: "testData".to_string(),
    })
}

fn scalar(aggregates: Vec<AggregateExpr>) -> QueryPlan {
    QueryPlan::AggregateScalar {
        input: source(),
        aggregates,
    }
}

/// Thread the selection predicate into a plan exactly where the engine's
/// emission does: onto an aggregation's input (a selection filters the base
/// rows that get aggregated), wrapping row-level plans whole.
fn with_selection(plan: &QueryPlan, predicate: &Predicate) -> QueryPlan {
    match plan.clone() {
        QueryPlan::Order { input, keys } => QueryPlan::Order {
            input: Box::new(with_selection(&input, predicate)),
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
        QueryPlan::AggregateScalar { input, aggregates } => QueryPlan::AggregateScalar {
            input: Box::new(QueryPlan::Filter {
                input,
                predicate: predicate.clone(),
            }),
            aggregates,
        },
        other => QueryPlan::Filter {
            input: Box::new(other),
            predicate: predicate.clone(),
        },
    }
}

fn sql_of(plan: &QueryPlan) -> String {
    let mut bindings = Vec::new();
    let sql = render_query(plan, &mut bindings);
    assert!(bindings.is_empty(), "oracle plans carry no parameters");
    sql
}

/// The `measure` column of the (single expected) result row, as DOUBLE.
/// `None` is a SQL NULL.
fn measure_f64(c: &Connection, sql: &str) -> Option<f64> {
    c.query_row(
        &format!("SELECT CAST(measure AS DOUBLE) FROM ({sql}) LIMIT 1"),
        [],
        |r| r.get(0),
    )
    .expect("measure query")
}

fn measure_text(c: &Connection, sql: &str) -> String {
    c.query_row(
        &format!("SELECT CAST(measure AS VARCHAR) FROM ({sql}) LIMIT 1"),
        [],
        |r| r.get(0),
    )
    .expect("measure query")
}

/// One row of DOUBLE columns (NULL → 0.0) — the centering probe.
fn probe_row(c: &Connection, sql: &str) -> Vec<f64> {
    use duckdb::arrow::array::{Array, Float64Array};
    use duckdb::arrow::compute::cast;
    use duckdb::arrow::datatypes::DataType;
    let mut stmt = c.prepare(sql).expect("probe prepare");
    let batches: Vec<_> = stmt.query_arrow([]).expect("probe run").collect();
    let batch = batches
        .iter()
        .find(|b| b.num_rows() > 0)
        .expect("probe row");
    (0..batch.num_columns())
        .map(|i| {
            let col = cast(batch.column(i), &DataType::Float64).expect("probe cast");
            let arr = col
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("probe f64");
            if arr.is_null(0) {
                0.0
            } else {
                arr.value(0)
            }
        })
        .collect()
}

/// Derive, materialise, and re-query the cube for `plan` under `active`;
/// return the served `measure`. Panics when derivation bails — a served case
/// asserting through here IS the "this plan decomposes" claim. The re-query
/// is also asserted to never touch the base table.
fn served_value(c: &Connection, plan: &QueryPlan, active: &Predicate) -> Option<f64> {
    let derivation = derive_cube(plan, active, None).expect("plan should decompose into a cube");
    let centers = derivation
        .probe_sql()
        .map(|p| probe_row(c, p))
        .unwrap_or_default();
    let sqls = derivation.finalize(&centers).expect("finalize");
    c.execute_batch(&format!(
        "DROP TABLE IF EXISTS oracle_cube; CREATE TEMP TABLE oracle_cube AS {}",
        sqls.build_select
    ))
    .expect("cube build");
    let q = sqls.query_select("oracle_cube").expect("re-query renders");
    assert!(
        !q.contains("testData"),
        "the re-query never touches the base table: {q}"
    );
    measure_f64(c, &q)
}

fn approx(actual: Option<f64>, expected: Option<f64>, what: &str) {
    match (actual, expected) {
        (None, None) => {}
        (Some(a), Some(e)) => assert!(
            (a - e).abs() <= 1e-9 * e.abs().max(1.0),
            "{what}: got {a}, expected {e}"
        ),
        (a, e) => panic!("{what}: got {a:?}, expected {e:?}"),
    }
}

/// A served case: the cube answer equals both the direct answer and the
/// corpus's expected value.
fn assert_served(plan: &QueryPlan, expected: Option<f64>) {
    let c = conn();
    let active = point_dim_b();
    let direct = measure_f64(&c, &sql_of(&with_selection(plan, &active)));
    approx(direct, expected, "direct");
    let served = served_value(&c, plan, &active);
    approx(served, direct, "served vs direct");
    approx(served, expected, "served vs corpus");
}

/// A fallback case: derivation bails, and the direct query still answers the
/// corpus's expected value — nothing breaks, it only slows.
fn assert_fallback(plan: &QueryPlan, expected: Option<f64>) {
    let c = conn();
    let active = point_dim_b();
    assert!(
        derive_cube(plan, &active, None).is_none(),
        "expected derivation to bail to the direct query"
    );
    approx(
        measure_f64(&c, &sql_of(&with_selection(plan, &active))),
        expected,
        "direct under fallback",
    );
}

// --- the corpus, case by case -------------------------------------------

#[test]
fn supports_count_aggregate() {
    assert_served(
        &scalar(vec![agg(AggregateFunction::Count, &["*"], None)]),
        Some(3.0),
    );
    assert_served(
        &scalar(vec![agg(AggregateFunction::Count, &["\"x\""], None)]),
        Some(2.0),
    );
}

#[test]
fn supports_empty_count_aggregate() {
    // The client's own WHERE eliminates every row. Direct COUNT over zero
    // rows is 0 — and so is the cube's reassembly (the coalesce is what this
    // case exists to force).
    let plan = QueryPlan::AggregateScalar {
        input: Box::new(QueryPlan::Filter {
            input: source(),
            predicate: Predicate::Expr("false".to_string()),
        }),
        aggregates: vec![agg(AggregateFunction::Count, &["*"], None)],
    };
    assert_served(&plan, Some(0.0));
}

#[test]
fn supports_sum_aggregate() {
    assert_served(
        &scalar(vec![agg(AggregateFunction::Sum, &["\"x\""], None)]),
        Some(7.0),
    );
}

#[test]
fn supports_avg_aggregate() {
    assert_served(
        &scalar(vec![agg(AggregateFunction::Avg, &["\"x\""], None)]),
        Some(3.5),
    );
}

#[test]
fn supports_geomean_aggregate() {
    // DIVERGENCE: upstream decomposes geomean; no lowerer here emits it, so
    // it arrives raw and falls back — sqrt(12) either way.
    assert_fallback(&scalar(vec![raw("geomean(\"x\")")]), Some(12.0_f64.sqrt()));
}

#[test]
fn supports_min_aggregate() {
    assert_served(
        &scalar(vec![agg(AggregateFunction::Min, &["\"x\""], None)]),
        Some(3.0),
    );
}

#[test]
fn supports_max_aggregate() {
    assert_served(
        &scalar(vec![agg(AggregateFunction::Max, &["\"x\""], None)]),
        Some(4.0),
    );
}

#[test]
fn supports_product_aggregate() {
    // DIVERGENCE: outside the emitted surface — falls back.
    assert_fallback(&scalar(vec![raw("product(\"x\")")]), Some(12.0));
}

#[test]
fn supports_argmax_argmin_aggregates() {
    // DIVERGENCE: outside the emitted surface — falls back; the direct
    // answer still matches the corpus.
    let c = conn();
    let active = point_dim_b();
    for expr in ["arg_max(\"dim\", \"x\")", "arg_min(\"dim\", \"x\")"] {
        let plan = scalar(vec![raw(expr)]);
        assert!(derive_cube(&plan, &active, None).is_none());
        assert_eq!(
            measure_text(&c, &sql_of(&with_selection(&plan, &active))),
            "b",
            "{expr}"
        );
    }
}

#[test]
fn supports_variance_family_aggregates() {
    // DIVERGENCE: upstream decomposes the variance family; outside the
    // emitted surface here — all fall back, all stay correct.
    assert_fallback(&scalar(vec![raw("var_samp(\"x\")")]), Some(0.5));
    assert_fallback(&scalar(vec![raw("var_pop(\"x\")")]), Some(0.25));
    assert_fallback(
        &scalar(vec![raw("stddev_samp(\"x\")")]),
        Some(0.5_f64.sqrt()),
    );
    assert_fallback(
        &scalar(vec![raw("stddev_pop(\"x\")")]),
        Some(0.25_f64.sqrt()),
    );
}

#[test]
fn supports_covariance_and_corr_aggregates() {
    // DIVERGENCE: outside the emitted surface — falls back, both argument
    // orders.
    assert_fallback(&scalar(vec![raw("covar_samp(\"x\", \"y\")")]), Some(-0.5));
    assert_fallback(&scalar(vec![raw("covar_samp(\"y\", \"x\")")]), Some(-0.5));
    assert_fallback(&scalar(vec![raw("covar_pop(\"x\", \"y\")")]), Some(-0.25));
    assert_fallback(&scalar(vec![raw("covar_pop(\"y\", \"x\")")]), Some(-0.25));
    assert_fallback(&scalar(vec![raw("corr(\"x\", \"y\")")]), Some(-1.0));
    assert_fallback(&scalar(vec![raw("corr(\"y\", \"x\")")]), Some(-1.0));
}

#[test]
fn supports_regression_aggregates() {
    let args: &[&str] = &["\"y\"", "\"x\""];
    let served: &[(AggregateFunction, f64)] = &[
        (AggregateFunction::RegrCount, 2.0),
        (AggregateFunction::RegrAvgx, 3.5),
        (AggregateFunction::RegrSxx, 0.5),
        (AggregateFunction::RegrSyy, 0.5),
        (AggregateFunction::RegrSxy, -0.5),
        (AggregateFunction::RegrSlope, -1.0),
        (AggregateFunction::RegrIntercept, 10.0),
    ];
    for &(func, expected) in served {
        assert_served(&scalar(vec![agg(func, args, None)]), Some(expected));
    }
    // DIVERGENCE: regr_avgy / regr_r2 are not in the emitted surface — they
    // fall back.
    assert_fallback(&scalar(vec![raw("regr_avgy(\"y\", \"x\")")]), Some(6.5));
    assert_fallback(&scalar(vec![raw("regr_r2(\"y\", \"x\")")]), Some(1.0));
}

#[test]
fn supports_multi_aggregate_expressions() {
    // DIVERGENCE: upstream rewrites expressions over aggregates; a typed
    // call here is a single function, so the expression arrives raw and
    // falls back.
    assert_fallback(
        &scalar(vec![raw("(sum(\"x\") + product(\"x\"))")]),
        Some(19.0),
    );
}

#[test]
fn supports_aggregate_filter_clause() {
    for (filter, expected) in [
        ("\"x\" > 2", Some(7.0)),
        ("\"x\" > 3", Some(4.0)),
        ("\"x\" > 4", None),
    ] {
        assert_served(
            &scalar(vec![agg(AggregateFunction::Sum, &["\"x\""], Some(filter))]),
            expected,
        );
    }
}

#[test]
fn does_not_support_distinct_aggregates() {
    // Same stance as upstream: a DISTINCT aggregate does not decompose (a
    // sum of per-cell distinct counts is not a distinct count) — served
    // through the non-optimised route.
    assert_fallback(
        &scalar(vec![agg(
            AggregateFunction::Count,
            &["DISTINCT \"x\""],
            None,
        )]),
        Some(2.0),
    );
}

#[test]
fn supports_subqueries_with_aggregates() {
    // DIVERGENCE: upstream's analyzer walks through CTE subqueries; the
    // deriver here requires a plain chain over one source and bails on the
    // nested aggregation. The direct query — with the selection pushed to
    // the INNER aggregation's input, where the engine places it — still
    // answers the corpus value.
    let inner = |predicate: Option<&Predicate>| -> QueryPlan {
        let input = match predicate {
            Some(p) => Box::new(QueryPlan::Filter {
                input: source(),
                predicate: p.clone(),
            }),
            None => source(),
        };
        QueryPlan::Aggregation {
            input,
            group_by: vec!["\"x\"".to_string()],
            aggregates: vec![AggregateExpr::Raw("COUNT(*) AS \"freq\"".to_string())],
        }
    };
    let outer = |inner: QueryPlan| QueryPlan::AggregateScalar {
        input: Box::new(inner),
        aggregates: vec![raw("sum(2 * \"freq\")")],
    };

    let active = point_dim_b();
    assert!(
        derive_cube(&outer(inner(None)), &active, None).is_none(),
        "nested aggregation bails"
    );
    let c = conn();
    approx(
        measure_f64(&c, &sql_of(&outer(inner(Some(&active))))),
        Some(6.0),
        "subquery direct",
    );
}

#[test]
fn supports_queries_with_column_index_references() {
    // DIVERGENCE: upstream resolves positional references; a positional
    // group dimension is not a named output column here, so derivation
    // bails. (The plan groups by the constant 2 — one group — matching the
    // corpus's groupby of a literal.)
    let plan = QueryPlan::Order {
        input: Box::new(QueryPlan::Aggregation {
            input: source(),
            group_by: vec!["2".to_string()],
            aggregates: vec![agg(AggregateFunction::Avg, &["\"x\""], None)],
        }),
        keys: vec![("1".to_string(), SortDir::Desc)],
    };
    assert_fallback(&plan, Some(3.5));
}

#[test]
fn supports_queries_with_aggregate_order_by_expressions() {
    // DIVERGENCE, structural: upstream orders by an aggregate expression
    // inside the same SELECT. This IR renders ORDER BY in a wrapper over the
    // aggregation's output, where `sum("y")` cannot bind — the shape is
    // inexpressible as an executable plan here, direct or served. The
    // deriver's order-key guard bails on it for the same reason (a base
    // expression is absent from the cube), keeping cube behaviour aligned
    // with the direct path: neither side can serve this plan.
    let plan = QueryPlan::Order {
        input: Box::new(QueryPlan::Aggregation {
            input: source(),
            group_by: vec!["\"dim\"".to_string()],
            aggregates: vec![agg(AggregateFunction::Avg, &["\"x\""], None)],
        }),
        keys: vec![("sum(\"y\")".to_string(), SortDir::Asc)],
    };
    let active = point_dim_b();
    assert!(
        derive_cube(&plan, &active, None).is_none(),
        "an aggregate-expression order key bails"
    );
    let c = conn();
    let direct = c.prepare(&format!(
        "SELECT CAST(measure AS DOUBLE) FROM ({}) LIMIT 1",
        sql_of(&with_selection(&plan, &active))
    ));
    assert!(
        direct.is_err(),
        "the direct rendering cannot bind an aggregate order expression either"
    );
}

#[test]
fn window_qualify_and_having_shapes_fall_back() {
    // Upstream optimises through HAVING / QUALIFY / windows-over-aggregates.
    // This IR expresses those as (or reduces them to) a predicate ABOVE the
    // aggregation — a shape the deriver rejects outright, because filtering
    // aggregated output is not filtering base rows. (A window function has
    // no IR expression at all: no plan can even reach the deriver with one.)
    // The corpus's HAVING expectation holds on the direct path: sum(y) for
    // the selected dim, kept by the post-aggregation predicate.
    let grouped = |predicate: Option<&Predicate>| -> QueryPlan {
        let input = match predicate {
            Some(p) => Box::new(QueryPlan::Filter {
                input: source(),
                predicate: p.clone(),
            }),
            None => source(),
        };
        QueryPlan::Aggregation {
            input,
            group_by: vec!["\"dim\"".to_string()],
            aggregates: vec![agg(AggregateFunction::Sum, &["\"y\""], None)],
        }
    };
    let post_filtered = |inner: QueryPlan| QueryPlan::Filter {
        input: Box::new(inner),
        predicate: Predicate::Expr("\"measure\" > 7".to_string()),
    };

    let active = point_dim_b();
    assert!(
        derive_cube(&post_filtered(grouped(None)), &active, None).is_none(),
        "a post-aggregation predicate bails"
    );
    let c = conn();
    approx(
        measure_f64(&c, &sql_of(&post_filtered(grouped(Some(&active))))),
        Some(13.0),
        "post-aggregation direct",
    );
}

#[test]
fn supports_queries_with_filter_pushdown_applied() {
    // A non-selective static filter inside the plan (the corpus's pushed
    // `dim != 'c'`) coexists with derivation: it lands in the cube's build
    // WHERE.
    let plan = QueryPlan::Aggregation {
        input: Box::new(QueryPlan::Filter {
            input: source(),
            predicate: Predicate::Expr("\"dim\" != 'c'".to_string()),
        }),
        group_by: vec!["\"dim\"".to_string()],
        aggregates: vec![agg(AggregateFunction::Avg, &["\"x\""], None)],
    };
    assert_served(&plan, Some(3.5));
}

#[test]
fn supports_queries_with_renamed_groupby_dimensions() {
    // A renamed dimension (`cat AS "Cat"`) keeps its output alias through
    // the cube; ordering by the aggregate ALIAS is an output column and
    // stays servable.
    let plan = QueryPlan::Order {
        input: Box::new(QueryPlan::Aggregation {
            input: source(),
            group_by: vec!["\"cat\" AS \"Cat\"".to_string()],
            aggregates: vec![agg(AggregateFunction::Avg, &["\"x\""], None)],
        }),
        keys: vec![("\"measure\"".to_string(), SortDir::Desc)],
    };
    assert_served(&plan, Some(3.5));
}

#[test]
fn supports_queries_with_expression_valued_groupby_dimensions() {
    // DIVERGENCE: upstream derives cube dimensions for unaliased expression
    // groupbys; here a dimension must carry an output alias (or BE a plain
    // column) so the re-query can reproduce the name — `upper("cat")`
    // without an alias bails.
    let plan = QueryPlan::Order {
        input: Box::new(QueryPlan::Aggregation {
            input: source(),
            group_by: vec![
                "\"cat\" AS \"Cat\"".to_string(),
                "upper(\"cat\")".to_string(),
            ],
            aggregates: vec![agg(AggregateFunction::Avg, &["\"x\""], None)],
        }),
        keys: vec![("\"measure\"".to_string(), SortDir::Desc)],
    };
    assert_fallback(&plan, Some(3.5));
}

#[test]
fn supports_expression_valued_selection_fields() {
    // The active clause's column is an arbitrary expression; it becomes a
    // cube dimension. One cube row per expression value — the corpus asserts
    // exactly two ('big', 'small'; a NULL x makes the CASE fall to 'small').
    let case = "CASE WHEN \"x\" > 2 THEN 'big' ELSE 'small' END";
    let active = Predicate::Point {
        column: case.to_string(),
        values: vec![ScalarValue::Text("big".to_string())],
        meta: None,
    };
    let plan = scalar(vec![agg(AggregateFunction::Sum, &["\"x\""], None)]);

    let c = conn();
    let direct = measure_f64(&c, &sql_of(&with_selection(&plan, &active)));
    approx(direct, Some(7.0), "direct");
    let served = served_value(&c, &plan, &active);
    approx(served, Some(7.0), "served");

    let cube_rows: i64 = c
        .query_row("SELECT COUNT(*) FROM oracle_cube", [], |r| r.get(0))
        .expect("cube rows");
    assert_eq!(cube_rows, 2, "one cube row per expression value");
}

#[test]
fn supports_case_insensitive_collisions_among_groupby_dimensions() {
    // DIVERGENCE NOTE: upstream dedupes `cat`/`CAT` to one cube column; here
    // both dimensions are materialised (a slightly wider cube), and the
    // answers still agree with the direct query and the corpus.
    let plan = QueryPlan::Aggregation {
        input: source(),
        group_by: vec!["\"cat\"".to_string(), "\"CAT\"".to_string()],
        aggregates: vec![agg(AggregateFunction::Avg, &["\"x\""], None)],
    };
    assert_served(&plan, Some(3.5));
}

#[test]
fn supports_duplicate_groupby_dimensions() {
    // Same stance as the case-insensitive collision: not deduped, still
    // correct.
    let plan = QueryPlan::Aggregation {
        input: source(),
        group_by: vec!["\"cat\"".to_string(), "\"cat\"".to_string()],
        aggregates: vec![agg(AggregateFunction::Avg, &["\"x\""], None)],
    };
    assert_served(&plan, Some(3.5));
}
