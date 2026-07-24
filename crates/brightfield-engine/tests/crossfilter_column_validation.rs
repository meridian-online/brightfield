//! Cross-filter column-reachability validation — end-to-end acceptance.
//!
//! A cross-filter carries a column resolved from the producing plot's brush
//! channel; a subscriber mark that `filterBy`s the selection re-emits its query
//! with `WHERE <column> ...`. When the subscriber's source does not expose that
//! column, DuckDB's binder rejects the query at interaction time.
//!
//! `analyse_spec` now diagnoses that failure statically for the cases it can
//! prove (inline subscriber sources). These tests assert the three properties
//! the diagnosis must hold:
//!
//! 1. the diagnosis names the mark, the column, and the projected alternatives;
//! 2. a *validated* spec never reaches the runtime DuckDB binder path — the
//!    unbindable spec is rejected upstream of load/emit/execute, and an accepted
//!    spec's subscriber binds cleanly when brushed;
//! 3. the accept/reject decision and the brushed result are identical with the
//!    pre-aggregation cube layer on and off (validation is a pure function of
//!    the spec, upstream of the cube; the served and direct queries agree).

use brightfield_engine::{Engine, RecordBatch};
use brightfield_spec::analysis::{analyse_spec, ComponentPath};
use brightfield_spec::error::ParseError;
use brightfield_spec::{parse_spec, Format};
use brightfield_sql::ir::{Predicate, ScalarValue};

/// Producer plot brushes column `x` (from inline source `a`); subscriber reads
/// inline source `b`, which exposes only `p`/`q` — so `WHERE x ...` cannot bind.
const UNBINDABLE: &str = r#"
params:
  brush: { select: crossfilter }
data:
  a:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
  b:
    - { p: 1, q: 10 }
    - { p: 2, q: 20 }
hconcat:
  - plot:
    - mark: dot
      data: { from: a }
      x: x
      y: y
    - select: intervalX
      as: $brush
  - plot:
    - mark: dot
      data: { from: b, filterBy: $brush }
      x: p
      y: q
"#;

/// The corrected spec: both plots read one source that exposes the brushed
/// column. The producer brushes `b`; the subscriber is a `densityX` that bins
/// `a` and counts, so its plan decomposes into a data cube — the clause column
/// `b` differs from the binned dimension `a`, the shape the cube layer serves.
/// The source is a `query:` (schema not statically knowable), so the column
/// check leaves it to the runtime binder — and it binds, because `b` is real.
const BINDABLE: &str = r#"
params:
  brush: { select: crossfilter }
data:
  t: { query: "SELECT (i % 50) AS a, (i % 30) AS b FROM range(2000) t(i)" }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: b
      y: a
    - select: intervalX
      as: $brush
  - plot:
    - mark: densityX
      data: { from: t, filterBy: $brush }
      x: a
"#;

/// AC1 — the validation-time diagnosis names the mark, the column, and the
/// projected alternatives.
#[test]
fn diagnosis_names_mark_column_and_alternatives() {
    let parsed = parse_spec(UNBINDABLE, Format::Yaml).expect("parses");
    let err = analyse_spec(&parsed.spec).expect_err("must reject the unbindable cross-filter");
    match err {
        ParseError::CrossfilterColumnUnprojected {
            mark,
            column,
            alternatives,
            ..
        } => {
            assert_eq!(mark, "root/hconcat[1]/plot[0]/mark[dot]");
            assert_eq!(column, "x");
            assert_eq!(alternatives, vec!["p".to_string(), "q".to_string()]);
        }
        other => panic!("expected CrossfilterColumnUnprojected, got {other:?}"),
    }
}

/// AC2 — the runtime DuckDB binder path is unreachable for validated specs.
///
/// The unbindable spec is rejected at `analyse_spec`, so the standard
/// parse → analyse → load pipeline never constructs a `Session` for it: no
/// query is emitted, and the binder is never reached. The accepted spec, by
/// contrast, loads and binds cleanly when brushed — no binder error surfaces.
#[test]
fn validated_spec_never_reaches_the_binder() {
    // Rejected upstream of load: no analysis → no session → no emit → no binder.
    let parsed = parse_spec(UNBINDABLE, Format::Yaml).expect("parses");
    assert!(
        analyse_spec(&parsed.spec).is_err(),
        "the unbindable spec must be rejected before it can be loaded and executed"
    );

    // The accepted spec brushes cleanly: the subscriber's re-query binds.
    let parsed = parse_spec(BINDABLE, Format::Yaml).expect("parses");
    let analysis = analyse_spec(&parsed.spec).expect("bindable spec is accepted");
    let engine = Engine::new();
    let mut session = engine
        .load_spec(parsed.spec, analysis, None)
        .expect("loads")
        .session;

    let results = session.propagate_selection(
        "brush",
        ComponentPath("root/hconcat[0]".to_string()),
        interval_b(),
    );
    for (idx, r) in &results {
        assert!(
            r.is_ok(),
            "subscriber mark {idx} must bind (no runtime binder error): {:?}",
            r.as_ref().err()
        );
    }
}

/// AC3 — behaviour is identical with the cube layer on and off.
///
/// Validation is a pure function of the spec, upstream of the cube: the
/// unbindable spec is rejected regardless, and the accepted spec's brushed
/// subscriber result is byte-identical whether the cube serves the re-query or
/// the direct query runs.
#[test]
fn behaviour_identical_with_cube_on_and_off() {
    // Reject decision is cube-independent (validation precedes any Session).
    let parsed = parse_spec(UNBINDABLE, Format::Yaml).expect("parses");
    assert!(analyse_spec(&parsed.spec).is_err());

    let on = brushed_subscriber_rows(true);
    let off = brushed_subscriber_rows(false);
    assert_eq!(
        on.rows, off.rows,
        "subscriber result must be identical with the cube on and off"
    );
    // The cube must actually engage in the on-case, else the parity is vacuous.
    assert!(
        on.cube_hits > 0,
        "expected the cube to serve the aggregating re-query at least once"
    );
    assert_eq!(off.cube_hits, 0, "the cube is retired when disabled");
}

/// The subscriber's brushed result plus the cube-hit count, for a session with
/// the pre-aggregation layer enabled (`preagg = true`) or disabled.
struct Brushed {
    rows: Vec<Vec<Option<u64>>>,
    cube_hits: usize,
}

fn brushed_subscriber_rows(preagg: bool) -> Brushed {
    let parsed = parse_spec(BINDABLE, Format::Yaml).expect("parses");
    let analysis = analyse_spec(&parsed.spec).expect("accepted");
    let engine = Engine::new();
    let mut session = engine
        .load_spec(parsed.spec, analysis, None)
        .expect("loads")
        .session;
    session.set_preagg_enabled(preagg);

    let results = session.propagate_selection(
        "brush",
        ComponentPath("root/hconcat[0]".to_string()),
        interval_b(),
    );
    // The subscriber is the filterBy mark (index 1); index 0 is the producer.
    let batches = results
        .into_iter()
        .find(|(idx, _)| *idx == 1)
        .map(|(_, r)| r.expect("subscriber binds"))
        .expect("subscriber mark present");
    Brushed {
        rows: rows_as_sorted_cells(&batches),
        cube_hits: session.preagg_stats().cube_hits,
    }
}

/// A structured interval clause on `b ∈ [5, 15]` — the shape the live brush
/// dispatches (so the cube layer can derive a cube from it).
fn interval_b() -> Predicate {
    Predicate::Interval {
        column: "b".to_string(),
        lo: ScalarValue::Int(5),
        hi: ScalarValue::Int(15),
        meta: None,
    }
}

/// Every row of every batch as a Vec of per-column cells (each numeric cell as
/// its `f64` bit pattern, NULL as `None`), sorted — an order-independent,
/// type-agnostic canonical form for comparing two result sets. Every column in
/// the fixture is numeric, so casting each to `Float64` is lossless.
fn rows_as_sorted_cells(batches: &[RecordBatch]) -> Vec<Vec<Option<u64>>> {
    use duckdb::arrow::array::{Array, Float64Array};
    use duckdb::arrow::compute::cast;
    use duckdb::arrow::datatypes::DataType;

    let mut rows: Vec<Vec<Option<u64>>> = Vec::new();
    for batch in batches {
        let cols: Vec<Float64Array> = (0..batch.num_columns())
            .map(|c| {
                let arr = cast(batch.column(c), &DataType::Float64).expect("numeric column");
                arr.as_any()
                    .downcast_ref::<Float64Array>()
                    .expect("f64")
                    .clone()
            })
            .collect();
        for r in 0..batch.num_rows() {
            let cell: Vec<Option<u64>> = cols
                .iter()
                .map(|c| {
                    if c.is_null(r) {
                        None
                    } else {
                        Some(c.value(r).to_bits())
                    }
                })
                .collect();
            rows.push(cell);
        }
    }
    rows.sort();
    rows
}
