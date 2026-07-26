//! The sampling clause, pinned where it has to sit.
//!
//! Every assertion here is on the **emitted SQL string**, never on rows that
//! come back from running it. `hash()`'s stability across DuckDB versions is
//! unverified, and pinning a snapshot to sampled output would make a version
//! bump look like a sampling bug. What DuckDB actually promises about the
//! clause is measured separately, against the linked library, in
//! `brightfield-engine`'s `sample_determinism` tests.

use brightfield_spec::{parse_spec, Format};
use brightfield_sql::cube::derive_cube;
use brightfield_sql::emit::{emit_query, emit_query_sampled, lower_mark_plan};
use brightfield_sql::ir::{Predicate, QueryPlan, SampleRate, SelectionPredicate};

fn spec_of(src: &str) -> brightfield_spec::ast::Spec {
    parse_spec(src, Format::Yaml).expect("parse").spec
}

/// A row-level scatter over a named source, with a brush the plot honours.
const SCATTER: &str = "params:\n  brush:\n    select: intersect\ndata:\n  t:\n    file: rows.parquet\nplot:\n  - mark: dot\n    data: { from: t, filterBy: $brush }\n    x: xcol\n    y: ycol\n";

/// The same scatter with a PARAM-valued channel, which is what makes the
/// lowerer emit a projection at all — a plain `x`/`y` dot mark lowers to a bare
/// `SELECT *`, so it is the wrong fixture for a question about projections.
const SCATTER_PROJECTED: &str = "params:\n  k: 3\ndata:\n  t:\n    file: rows.parquet\nplot:\n  - mark: dot\n    data: { from: t }\n    x: xcol\n    y: $k\n";

/// An aggregating mark over the same source.
const BINNED: &str = "params:\n  brush:\n    select: intersect\ndata:\n  t:\n    file: rows.parquet\nplot:\n  - mark: densityX\n    data: { from: t, filterBy: $brush }\n    x: xcol\n";

fn no_passes() -> Vec<Box<dyn brightfield_sql::passes::Pass>> {
    vec![]
}

/// The feature must be invisible to every chart that is not sampled — same
/// SQL, therefore the same plan hash, therefore the same cache behaviour and
/// the same query count.
#[test]
fn sample_leaves_unsampled_emission_byte_identical() {
    for src in [SCATTER, BINNED] {
        let spec = spec_of(src);
        let baseline = emit_query(&spec, 0, None, None).expect("baseline");
        let threaded =
            emit_query_sampled(&spec, 0, None, None, &no_passes(), None).expect("threaded");
        assert_eq!(
            baseline.sql, threaded.sql,
            "threading an absent sample rate changed the emitted SQL"
        );
        assert_eq!(
            baseline.plan_hash, threaded.plan_hash,
            "threading an absent sample rate changed the plan hash"
        );
    }
}

/// The clause itself: a power-of-two modulus on a hash of the whole row.
#[test]
fn sample_renders_a_power_of_two_hash_modulus_clause() {
    let spec = spec_of(SCATTER);
    let rate = SampleRate::from_modulus(16).expect("16 is a power of two");
    let emitted =
        emit_query_sampled(&spec, 0, None, None, &no_passes(), Some(rate)).expect("emit sampled");
    assert!(
        emitted.sql.contains("AS _s WHERE hash(_s) % 16 = 0"),
        "expected the sampling clause, got: {}",
        emitted.sql
    );
}

/// The clause wraps the mark's projection rather than sitting under it. Under
/// it, `hash(_s)` would hash the SOURCE row — which DuckDB expands to a hash of
/// every column and which therefore reads every column, taking parquet
/// projection pushdown off exactly the query that needs it.
#[test]
fn sample_sits_above_the_marks_projection() {
    let spec = spec_of(SCATTER_PROJECTED);
    let rate = SampleRate::from_modulus(8).expect("power of two");
    let sql = emit_query_sampled(&spec, 0, None, None, &no_passes(), Some(rate))
        .expect("emit")
        .sql;

    let sample_at = sql.find("AS _s WHERE hash(_s)").expect("sampling clause");
    let projection_at = sql.find("AS _p").expect("the mark's channel projection");
    assert!(
        projection_at < sample_at,
        "the projection must be INSIDE the sampled subquery (so the hashed tuple is \
         the projected one), got: {sql}"
    );
}

/// The one that is silent when it breaks. The highlight pass appends a
/// `__bf_selected` column; if the sample sat above it that flag would join the
/// hashed tuple, and every selection change would redraw a different subset of
/// points while looking like data movement.
#[test]
fn sample_sits_below_the_highlight_projection_so_a_brush_cannot_reshuffle_it() {
    let spec = spec_of(HIGHLIGHTED);
    // Modulus 32, and not for taste. The consequence this guards is measured
    // over real rows in `brightfield-engine`'s `sample_determinism`, and there
    // DuckDB's `hash(struct_pack(…))` turns out to be BLIND to an appended
    // boolean below modulus 32 — a runtime check at 8 or 16 passes in both
    // nestings. Using the same rate here keeps the two files talking about the
    // same thing.
    let rate = SampleRate::from_modulus(32).expect("power of two");

    let sql = emit_query_sampled(
        &spec,
        0,
        None,
        Some(&selection("\"xcol\" > 1")),
        &no_passes(),
        Some(rate),
    )
    .expect("emit")
    .sql;

    // The STRUCTURAL question, asked structurally. An earlier version of this
    // test compared the two substrings' positions, which was vacuous: the
    // marker it searched for is the sampled subquery's TRAILING `WHERE`, so it
    // lands after the whole inner query text in either nesting and the
    // assertion held under the very mutation it existed to catch. What is
    // actually being asked is whether `__bf_selected` is one of the columns
    // `_s` projects — i.e. whether it is INSIDE the sampled subquery's own
    // text — so that is what is extracted and searched.
    let hashed = hashed_subquery(&sql);
    assert!(
        !hashed.contains("__bf_selected"),
        "__bf_selected is inside the hashed tuple. `hash(_s)` hashes whatever `_s` \
         projects, so the sample would be re-drawn on every selection change and the \
         plot would silently redraw a different subset of points under a brush.\n  \
         hashed subquery: {hashed}\n  full: {sql}"
    );
    // Not vacuous in the other direction either: the flag IS emitted, just
    // outside. Without this, deleting the highlight pass would make the check
    // above pass for the wrong reason.
    assert!(
        sql.contains("__bf_selected"),
        "fixture check: this spec must produce a highlight projection at all, got: {sql}"
    );

    // A second, weaker check, kept and labelled as weak: the two emissions
    // differ ONLY in the highlight predicate's own text. This is NECESSARY but
    // NOT SUFFICIENT — verified by mutation, it holds in both nestings, because
    // relocating the Sample node moves the predicate's text without changing
    // it. It guards against unrelated drift inside the sampled subquery, not
    // against the placement.
    let sql_a = sql;
    let sql_b = emit_query_sampled(
        &spec,
        0,
        None,
        Some(&selection("\"xcol\" > 900")),
        &no_passes(),
        Some(rate),
    )
    .expect("emit b")
    .sql;
    assert_eq!(
        sql_a.replace("\"xcol\" > 1", "<PRED>"),
        sql_b.replace("\"xcol\" > 900", "<PRED>"),
        "changing the selection changed something other than the highlight predicate\n  \
         a: {sql_a}\n  b: {sql_b}"
    );
}

/// A dot scatter whose plot carries a highlight interactor — the one shape in
/// which the sample's placement relative to `__bf_selected` is observable.
const HIGHLIGHTED: &str = "data:\n  t:\n    file: rows.parquet\nplot:\n  - mark: dot\n    data: { from: t }\n    x: xcol\n    y: ycol\n  - select: intervalXY\n    as: $range\n  - select: highlight\n    by: $range\n    opacity: 0.15\n";

/// One live contributor to `$range`, carrying `expr` as its predicate.
fn selection(expr: &str) -> Vec<SelectionPredicate> {
    vec![(
        "range".to_string(),
        vec![("root".to_string(), Predicate::Expr(expr.to_string()))],
    )]
}

/// The text of the subquery `hash(_s)` hashes — i.e. everything between the
/// parentheses of `FROM (…) AS _s WHERE hash(_s)`.
///
/// Found by matching parentheses backwards from the closer, so it answers the
/// NESTING question rather than a question about where two substrings happen to
/// land in the rendered string. (The rendered IR puts no parentheses inside
/// string literals, so a naive depth count is exact here.)
fn hashed_subquery(sql: &str) -> &str {
    const MARK: &str = " AS _s WHERE hash(_s)";
    let mark_at = sql.find(MARK).expect("the sampling clause");
    let close = mark_at - 1;
    assert_eq!(
        &sql[close..=close],
        ")",
        "the sampling clause is not shaped `(…) AS _s WHERE hash(_s)`: {sql}"
    );
    let bytes = sql.as_bytes();
    let mut depth = 1_i32;
    let mut i = close;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b')' => depth += 1,
            b'(' => {
                depth -= 1;
                if depth == 0 {
                    return &sql[i + 1..close];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced parentheses around the sampling clause: {sql}");
}

/// A sample is never applied to an aggregating plan, and the pre-aggregation
/// layer never sees one. Both halves are asserted rather than argued: the first
/// by emitting a sampled aggregating mark, the second by handing `derive_cube`
/// a chain that contains a `Sample` and requiring it to decline.
#[test]
fn sampling_and_the_cube_stay_disjoint() {
    // Half one: an aggregating mark ignores the sample rate.
    let spec = spec_of(BINNED);
    let rate = SampleRate::from_modulus(8).expect("power of two");
    let unsampled = emit_query(&spec, 0, None, None).expect("unsampled").sql;
    let sampled = emit_query_sampled(&spec, 0, None, None, &no_passes(), Some(rate))
        .expect("sampled")
        .sql;
    assert_eq!(
        unsampled, sampled,
        "an aggregating mark must emit the same SQL sampled or not — its rows are bins"
    );

    // Half two: the cube's chain check rejects a Sample node through its
    // catch-all, so a chain carrying one declines the cube rather than being
    // served a rewrite that silently ignores the clause. Derived from the REAL
    // lowered plan, and asserted in both directions so the negative is not
    // vacuous.
    let (plan, _) = lower_mark_plan(&spec, 0).expect("lower the aggregating mark");
    let active = Predicate::Interval {
        column: "\"xcol\"".to_string(),
        lo: brightfield_sql::ir::ScalarValue::Float(0.0),
        hi: brightfield_sql::ir::ScalarValue::Float(10.0),
        meta: None,
    };
    assert!(
        derive_cube(&plan, &active, None).is_some(),
        "fixture check: the unsampled aggregating plan must be cube-eligible"
    );

    let sampled_plan = with_sample_under_the_aggregation(plan, 8);
    assert!(
        derive_cube(&sampled_plan, &active, None).is_none(),
        "the cube must decline a chain containing a Sample node"
    );
}

/// Wrap an aggregating plan's INPUT in a sample, leaving the aggregation
/// itself alone — the shape the cube's chain check has to reject.
fn with_sample_under_the_aggregation(plan: QueryPlan, modulus: u32) -> QueryPlan {
    match plan {
        QueryPlan::Order { input, keys } => QueryPlan::Order {
            input: Box::new(with_sample_under_the_aggregation(*input, modulus)),
            keys,
        },
        QueryPlan::Aggregation {
            input,
            group_by,
            aggregates,
        } => QueryPlan::Aggregation {
            input: Box::new(QueryPlan::Sample { input, modulus }),
            group_by,
            aggregates,
        },
        QueryPlan::AggregateScalar { input, aggregates } => QueryPlan::AggregateScalar {
            input: Box::new(QueryPlan::Sample { input, modulus }),
            aggregates,
        },
        other => QueryPlan::Sample {
            input: Box::new(other),
            modulus,
        },
    }
}

/// The nesting guarantee is only real if a non-power-of-two rate is
/// unrepresentable. Rounding one would keep nesting true while making the
/// stated rate a lie.
#[test]
fn sample_rate_admits_only_powers_of_two() {
    assert!(SampleRate::from_modulus(0).is_none());
    assert!(SampleRate::from_modulus(3).is_none());
    assert!(SampleRate::from_modulus(10).is_none());
    assert!(SampleRate::from_modulus(1000).is_none());
    for m in [1u32, 2, 4, 8, 1024, 1 << 31] {
        let r = SampleRate::from_modulus(m).expect("power of two accepted");
        assert_eq!(r.modulus(), m);
    }
    // Halving the modulus is one step down the exponent, which is what makes a
    // rate change densify instead of reshuffle.
    let fine = SampleRate::from_modulus(64).unwrap();
    let coarse = SampleRate::from_exponent(fine.exponent() - 1).unwrap();
    assert_eq!(coarse.modulus(), 32);
}
