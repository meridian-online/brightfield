//! The arithmetic oracle for the positional `bin` + `count` histogram.
//!
//! `x: {bin: col}` with `y: {count:}` is lowered to SQL that transliterates
//! Mosaic's `binSpec` / `binStep` (`mosaic-sql`'s
//! `transforms/util/bin-step.ts`) and `binHistogram`
//! (`transforms/bin-histogram.ts`). A SQL-TEXT assertion cannot tell a correct
//! transliteration from a plausible-but-wrong one — every case below therefore
//! **executes** the emitted query against the bundled DuckDB and compares the
//! bins it produces to values computed by running the reference JavaScript.
//!
//! The expected rows were produced by porting `binSpec`/`binStep`/`binHistogram`
//! verbatim into a script and running it on the same inputs; they are not
//! derived from this implementation. Four shapes, each pinning something a
//! shortcut would get wrong:
//!
//! - **`skewed`** — the ordinary case. `steps` defaults to **25** and the nice
//!   step falls out as 5 over `[0, 100]`, so the answer is **20 bins**. The
//!   density lowerer's fixed 100 would give four times as many, with edges that
//!   are not round numbers.
//! - **`top_edge`** — the maximum sits exactly on a step boundary, so Mosaic
//!   puts it in a bin of its OWN, one past the last full bin. Folding it back
//!   (as `equiwidth_bin_centre` must, binning over a raw extent) would be a
//!   silent divergence, and this is the case that catches it either way.
//! - **`steps_is_a_hint`** — `protein-design.yaml`'s shape. A `steps: 60` over
//!   the extent `[67, 94.5]` yields **55** bins of width `0.5`, not 60.
//!   Honouring the hint as an exact bar count is its own kind of wrong.
//! - **`degenerate_extent`** — a single-valued column, where Mosaic's
//!   `binHistogram` short-circuits to a bare `floor(field)`.

use brightfield_spec::{parse_spec, Format};
use brightfield_sql::emit::{emit_all_queries, emit_query, emit_sources};
use brightfield_sql::ir::{Predicate, SelectionPredicate};
use duckdb::Connection;

/// One output bin: `(low edge, high edge, count)`.
type Bin = (f64, f64, i64);

/// Emit mark 0 of `spec_yaml`, run it against an in-memory DuckDB, and return
/// its `(x1, x2, count)` rows in emitted order.
fn histogram(spec_yaml: &str) -> Vec<Bin> {
    let parsed = parse_spec(spec_yaml, Format::Yaml).expect("spec parses");
    let spec = parsed.spec;
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
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get::<_, f64>(2)? as i64))
        })
        .expect("query runs")
        .collect::<Result<Vec<Bin>, _>>()
        .expect("rows read");
    rows
}

/// A one-column inline table and a `rectY` that bins it and counts.
fn binned_rect_spec(values: &[f64], steps: Option<i64>) -> String {
    let rows: Vec<String> = values.iter().map(|v| format!("  - {{ v: {v} }}")).collect();
    let steps = steps.map_or(String::new(), |n| format!(", steps: {n}"));
    format!(
        "data:\n  t:\n{}\nplot:\n- mark: rectY\n  data: {{ from: t }}\n  \
         x: {{ bin: v{steps} }}\n  y: {{ count: }}\n  fill: steelblue\n",
        rows.join("\n")
    )
}

/// Compare against the reference answer, tolerating only float representation.
fn assert_bins(found: &[Bin], expected: &[Bin], case: &str) {
    assert_eq!(
        found.len(),
        expected.len(),
        "{case}: bin COUNT differs — found {found:?}, expected {expected:?}"
    );
    for (i, ((flo, fhi, fc), (elo, ehi, ec))) in found.iter().zip(expected).enumerate() {
        assert!(
            (flo - elo).abs() < 1e-9 && (fhi - ehi).abs() < 1e-9,
            "{case}: bin {i} edges are ({flo}, {fhi}), reference says ({elo}, {ehi})"
        );
        assert_eq!(fc, ec, "{case}: bin {i} [{elo}, {ehi}) count");
    }
}

/// The ordinary case: 37 asymmetric, unimodal values over `[1, 97]`.
/// `binSpec(1, 97, {steps: 25})` snaps outward to `[0, 100]` with step 5.
#[test]
fn skewed_column_bins_as_the_reference_does() {
    let values = [
        1.0, 3.0, 4.0, 6.0, 7.0, 9.0, 11.0, 12.0, 12.0, 13.0, 14.0, 14.0, 15.0, 16.0, 16.0, 17.0,
        17.0, 18.0, 19.0, 19.0, 21.0, 22.0, 23.0, 24.0, 26.0, 27.0, 29.0, 31.0, 34.0, 38.0, 42.0,
        47.0, 53.0, 61.0, 72.0, 88.0, 97.0,
    ];
    let expected: &[Bin] = &[
        (0.0, 5.0, 3),
        (5.0, 10.0, 3),
        (10.0, 15.0, 6),
        (15.0, 20.0, 8),
        (20.0, 25.0, 4),
        (25.0, 30.0, 3),
        (30.0, 35.0, 2),
        (35.0, 40.0, 1),
        (40.0, 45.0, 1),
        (45.0, 50.0, 1),
        (50.0, 55.0, 1),
        (60.0, 65.0, 1),
        (70.0, 75.0, 1),
        (85.0, 90.0, 1),
        (95.0, 100.0, 1),
    ];
    let found = histogram(&binned_rect_spec(&values, None));
    assert_bins(&found, expected, "skewed");
    // The default is 25 steps and the answer is 20 bins over a span of 100 —
    // both facts, not preferences. A fixed 100-bin rule would put the first
    // bar at [1, 1.96) and no assertion on counts alone would notice.
    assert!(
        (found[0].1 - found[0].0 - 5.0).abs() < 1e-9,
        "the nice step over [0, 100] at 25 steps is 5, not {}",
        found[0].1 - found[0].0
    );
}

/// The maximum lands exactly on a step boundary. Mosaic's extent snaps
/// `max = ceil(max/step) * step`, so `b.max == 100` and the row at 100 floors
/// into bin index 20 — one past the last full bin, in a bin of its own.
///
/// This is the tier a SQL-text assertion cannot reach: a `least(idx, n-1)` fold
/// (which the density lowerer needs, binning over a RAW extent) would merge
/// those two bins and every other row would still agree.
#[test]
fn a_value_on_the_top_step_boundary_gets_its_own_bin() {
    let values = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 40.0, 41.0, 42.0, 99.0, 100.0];
    let expected: &[Bin] = &[
        (0.0, 5.0, 5),
        (5.0, 10.0, 1),
        (40.0, 45.0, 3),
        (95.0, 100.0, 1),
        (100.0, 105.0, 1),
    ];
    let found = histogram(&binned_rect_spec(&values, None));
    assert_bins(&found, expected, "top_edge");
    assert_eq!(
        found.last().map(|b| (b.0, b.2)),
        Some((100.0, 1)),
        "the row at the snapped maximum is NOT folded into [95, 100)"
    );
}

/// `steps` is documented upstream as a hint. `protein-design.yaml` asks for 60
/// over `[67, 94.5]`; the derived step is `0.5` and the extent already aligns,
/// so the answer is 55 bins. Honouring 60 exactly would be a divergence.
#[test]
fn steps_is_a_hint_not_a_bar_count() {
    let values = [67.0, 67.25, 70.0, 70.4, 80.0, 80.1, 80.2, 88.0, 94.5];
    let expected: &[Bin] = &[
        (67.0, 67.5, 2),
        (70.0, 70.5, 2),
        (80.0, 80.5, 3),
        (88.0, 88.5, 1),
        (94.5, 95.0, 1),
    ];
    let found = histogram(&binned_rect_spec(&values, Some(60)));
    assert_bins(&found, expected, "steps_is_a_hint");
    assert!(
        (found[0].1 - found[0].0 - 0.5).abs() < 1e-9,
        "a 60-step hint over [67, 94.5] gives a step of 0.5 — 55 bins, not 60"
    );
    // And the hint is READ: the same values at the default 25 steps take a
    // different step, so a lowerer that dropped `steps:` would fail here.
    let default_steps = histogram(&binned_rect_spec(&values, None));
    assert!(
        (default_steps[0].1 - default_steps[0].0 - 2.0).abs() < 1e-9,
        "at the default hint the step is 2, so `steps: 60` is not being ignored: \
         found {}",
        default_steps[0].1 - default_steps[0].0
    );
}

/// A single-valued column has no extent. `binHistogram` returns a bare
/// `floor(field)` in that case, which is the `[7, 8)` below.
#[test]
fn a_degenerate_extent_falls_back_to_floor() {
    let found = histogram(&binned_rect_spec(&[7.0, 7.0, 7.0, 7.0], None));
    assert_bins(&found, &[(7.0, 8.0, 4)], "degenerate_extent");
}

/// The transpose. `rectX` bins on `y` and counts on `x`, and the high edge
/// moves to the y-keyed reserved column — the same arithmetic, so the same
/// answer, read off a differently named pair.
#[test]
fn the_transposed_orientation_bins_identically() {
    let values = [
        1.0, 3.0, 4.0, 6.0, 7.0, 9.0, 11.0, 12.0, 12.0, 13.0, 14.0, 14.0, 15.0, 16.0, 16.0, 17.0,
        17.0, 18.0, 19.0, 19.0, 21.0, 22.0, 23.0, 24.0, 26.0, 27.0, 29.0, 31.0, 34.0, 38.0, 42.0,
        47.0, 53.0, 61.0, 72.0, 88.0, 97.0,
    ];
    let rows: Vec<String> = values.iter().map(|v| format!("  - {{ v: {v} }}")).collect();
    let spec = format!(
        "data:\n  t:\n{}\nplot:\n- mark: rectX\n  data: {{ from: t }}\n  \
         x: {{ count: }}\n  y: {{ bin: v }}\n  fill: steelblue\n",
        rows.join("\n")
    );
    let across = histogram(&spec);
    let down = histogram(&binned_rect_spec(&values, None));
    assert_eq!(
        across, down,
        "rectX and rectY must bin the same column the same way"
    );
}

/// A selection filters the ROWS THAT GET COUNTED, and leaves the bin edges
/// exactly where they were.
///
/// This is the reason binning lives in the lowerer rather than the renderer.
/// `apply_selection_filter` threads the predicate onto an aggregation's direct
/// input, which for a binned rect is the projection carrying the bin scheme —
/// so the scheme is resolved over the whole table above the filter, and only
/// the counts move. A histogram whose bars re-bin on every brush is not a
/// histogram, and the ghosted cross-filter this card exists for depends on the
/// filtered layer's bars lining up with the unfiltered one's.
///
/// Executed, not shape-checked: the placement is only correct if the emitted
/// SQL still resolves, and a predicate wrapped around the wrong side of the
/// projection would name a column that is no longer in scope.
#[test]
fn a_selection_narrows_the_counts_without_moving_the_bins() {
    let values = [
        1.0, 3.0, 4.0, 6.0, 7.0, 9.0, 11.0, 12.0, 12.0, 13.0, 14.0, 14.0, 15.0, 16.0, 16.0, 17.0,
        17.0, 18.0, 19.0, 19.0, 21.0, 22.0, 23.0, 24.0, 26.0, 27.0, 29.0, 31.0, 34.0, 38.0, 42.0,
        47.0, 53.0, 61.0, 72.0, 88.0, 97.0,
    ];
    let rows: Vec<String> = values.iter().map(|v| format!("  - {{ v: {v} }}")).collect();
    let spec_yaml = format!(
        "data:\n  t:\n{}\nparams:\n  brush: {{ select: intersect }}\n\
         plot:\n- mark: rectY\n  data: {{ from: t, filterBy: $brush }}\n  \
         x: {{ bin: v }}\n  y: {{ count: }}\n  fill: steelblue\n",
        rows.join("\n")
    );
    let spec = parse_spec(&spec_yaml, Format::Yaml)
        .expect("spec parses")
        .spec;

    let conn = Connection::open_in_memory().expect("duckdb opens");
    for ddl in emit_sources(&spec, None).expect("sources emit").statements {
        conn.execute_batch(&ddl.sql).expect("ddl runs");
    }
    let brushed: Vec<SelectionPredicate> = vec![(
        "brush".to_string(),
        vec![(
            "t".to_string(),
            Predicate::Expr("\"v\" BETWEEN 10 AND 20".to_string()),
        )],
    )];
    let query = emit_query(&spec, 0, None, Some(&brushed)).expect("mark lowers");
    let mut stmt = conn.prepare(&query.sql).expect("brushed query prepares");
    let found: Vec<Bin> = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get::<_, f64>(2)? as i64))
        })
        .expect("query runs")
        .collect::<Result<_, _>>()
        .expect("rows read");

    // The brush keeps 11 ≤ v ≤ 20, which is the [10, 15) and [15, 20) bins
    // ONLY — at the same edges and the same width as the unbrushed answer.
    assert_bins(
        &found,
        &[(10.0, 15.0, 6), (15.0, 20.0, 8)],
        "brushed_histogram",
    );
}

/// The reserved output columns, read off the executed statement rather than
/// off the SQL text: the low edge takes the SOURCE column's name (which is what
/// lets a navigation extent push under the GROUP BY, and what names the axis),
/// the high edge takes the axis-keyed reserved name, and the count takes the
/// same `__bf_count` the density and hexbin lowerers already use.
#[test]
fn the_emitted_columns_carry_the_reserved_names() {
    let spec = binned_rect_spec(&[1.0, 2.0, 3.0], None);
    let parsed = parse_spec(&spec, Format::Yaml).expect("spec parses");
    let conn = Connection::open_in_memory().expect("duckdb opens");
    for ddl in emit_sources(&parsed.spec, None)
        .expect("sources emit")
        .statements
    {
        conn.execute_batch(&ddl.sql).expect("ddl runs");
    }
    let query = emit_all_queries(&parsed.spec, None)
        .into_iter()
        .next()
        .expect("one mark")
        .expect("mark lowers");
    // `DESCRIBE` asks DuckDB itself what the statement produces, so the names
    // asserted here are the ones that reach the Arrow batch rather than the
    // ones the emitter believes it wrote.
    let mut stmt = conn
        .prepare(&format!("DESCRIBE {}", query.sql))
        .expect("describe prepares");
    let names: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .expect("describe runs")
        .collect::<Result<_, _>>()
        .expect("names read");
    assert_eq!(
        names,
        vec!["v", "__bf_bin_x2", "__bf_count"],
        "the statement's output columns are the renderer's contract: the low \
         edge under the SOURCE column's name, the high edge and the count \
         under the reserved names `ChannelMap::from_mark` synthesises"
    );
}
