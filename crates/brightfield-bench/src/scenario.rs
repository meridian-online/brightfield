//! The engine-level suites: cold open, and interaction latency through the
//! coordinator seam — the exact seam the live shell blocks its frame on.

use std::path::Path;
use std::time::Instant;

use brightfield_engine::coordinator::{Coordinator, Interaction};
use brightfield_engine::{assemble_batches, RecordBatch, SqlPredicate};
use brightfield_spec::analysis::{analyse_spec, build_brushable_bindings, ComponentPath};
use brightfield_spec::{parse_spec, Format};
use brightfield_sql::ir::ScalarValue;
use serde::Serialize;

use crate::stats::Stats;

/// The number of consecutive iterations for which [`brush_interval`] yields
/// distinct `(lo, hi)` pairs before the sequence repeats. It is the least
/// common multiple of the two moduli in `brush_interval` (`i % 5` and
/// `i % 7`, coprime): once `i` reaches 35 the pair `(i % 5, i % 7)` — and
/// therefore the interval — collides with iteration 0.
///
/// Beyond this a suite would re-issue an interval it has already brushed, and
/// the engine's renderer-side SQL cache would short-circuit the repeat: the
/// measurement would time the cache rather than the engine, silently violating
/// the "every timed brush is distinct" methodology guarantee. The harness caps
/// its iteration count at this value (see `Args::parse`) so the guarantee holds
/// by construction rather than by the operator remembering to stay under it.
pub const DISTINCT_BRUSH_INTERVALS: usize = 35;

/// The brushed interval for iteration `i` of a suite, in the unit domain
/// [0, 100) — a widening/narrowing drag. The pairs stay distinct for
/// [`DISTINCT_BRUSH_INTERVALS`] consecutive iterations, which matters: the
/// engine's renderer-side SQL cache short-circuits a repeated identical query,
/// so re-using one interval would time the cache, not the engine. A real drag
/// never repeats a pixel either.
pub fn brush_interval(i: usize) -> (f64, f64) {
    let lo = 10.0 + (i % 5) as f64 * 4.0;
    let hi = 58.0 + (i % 7) as f64 * 5.0;
    (lo, hi)
}

/// [`brush_interval`] mapped onto a scenario's brushed-column domain: the
/// unit-domain endpoints scale linearly onto `[domain.0, domain.1)`, so a
/// dataset whose brushable axis is not [0, 100) still receives distinct,
/// inside-the-data intervals.
pub fn brush_interval_in(domain: (f64, f64), i: usize) -> (f64, f64) {
    let (lo, hi) = brush_interval(i);
    let span = (domain.1 - domain.0) / 100.0;
    (domain.0 + lo * span, domain.0 + hi * span)
}

/// The brush `Select` interaction for iteration `i`: a structured interval
/// over `column` within `domain`, against the binding's selection and
/// contributor — identical in shape to what the chart pane's interval gesture
/// produces (structured clauses are what let the pre-aggregation layer
/// engage).
pub fn brush_select(
    column: &str,
    domain: (f64, f64),
    selection: &str,
    contributor: &ComponentPath,
    i: usize,
) -> Interaction {
    let (lo, hi) = brush_interval_in(domain, i);
    Interaction::Select {
        name: selection.to_string(),
        contributor: contributor.clone(),
        predicate: SqlPredicate::Interval {
            column: column.to_string(),
            lo: ScalarValue::Float(lo),
            hi: ScalarValue::Float(hi),
            meta: None,
        },
    }
}

/// Per-mark materialisation shape after the first full execute: how many rows
/// the query returned in total, and how many of them the presentation layer
/// actually draws. `drawn_rows` is measured by running the SAME assembly the
/// presentation uses ([`assemble_batches`]) — so `drawn_rows < materialised_rows`
/// would mean the drawn picture holds fewer rows than the query answered. The
/// presentation now assembles every chunk, so the two are equal; a regression
/// that reintroduced a first-chunk cap would surface here as a discrepancy.
#[derive(Debug, Clone, Serialize)]
pub struct MarkRows {
    /// Mark index in spec order.
    pub mark: usize,
    /// Total rows across all returned batches.
    pub materialised_rows: u64,
    /// Rows in the assembled batch the composed scene draws — every chunk,
    /// via the presentation's own assembly path.
    pub drawn_rows: u64,
}

/// What the automatic pre-aggregation layer did during a suite — the
/// non-vacuity evidence beside the latencies. A comparison whose cube never
/// engaged (or engaged when it should not have) fails the run instead of
/// quietly reporting.
#[derive(Debug, Clone, Serialize)]
pub struct PreAggSummary {
    /// Whether the layer was enabled for this suite.
    pub enabled: bool,
    /// Cubes materialised (one CREATE TEMP TABLE each).
    pub cubes_built: usize,
    /// Brush-step re-queries served from a cube instead of the base table.
    pub cube_hits: usize,
    /// Cube builds that failed (each falls back to the direct query).
    pub build_failures: usize,
    /// Registered serves that failed at execution time and fell back.
    pub serve_failures: usize,
}

/// One scenario × row-count measurement at the engine seam.
#[derive(Debug, Clone, Serialize)]
pub struct EngineMeasurement {
    /// `Coordinator::load` — parse-to-ready session (DDL, no mark queries).
    pub load_ms: f64,
    /// First full materialisation of every mark (`execute_all`).
    pub first_materialise_ms: f64,
    /// Per-mark row shape after that first materialisation.
    pub marks: Vec<MarkRows>,
    /// Per-interaction `Coordinator::apply` latency: predicate push-down +
    /// re-query of every affected mark. The number the pre-aggregation layer
    /// is measured against (and, in the enabled run, measured with).
    pub coordinator_apply: Stats,
    /// Per-interaction `LiveDashboard::apply` latency: `Coordinator::apply`
    /// plus the re-composite into a Vello scene — the full cost the live
    /// window's frame blocks on for one committed brush step.
    pub live_apply: Stats,
    /// Row count of the cross-filtered mark's step under the final brush —
    /// the non-vacuity evidence that the brush actually filtered.
    pub brushed_step_rows: u64,
    /// The same step's row count with no brush active.
    pub unfiltered_step_rows: u64,
    /// What the pre-aggregation layer did (coordinator-seam session).
    pub preagg: PreAggSummary,
}

/// The parsed scenario inputs the suites share.
pub struct Scenario {
    /// Scenario id used in reports and spec filenames.
    pub name: String,
    /// Fully substituted spec text.
    pub spec_text: String,
    /// The column the interval brush sweeps.
    pub brush_column: &'static str,
    /// The brushed column's data domain, mapped from the unit drag.
    pub brush_domain: (f64, f64),
    /// Whether the pre-aggregation layer is expected to engage for this
    /// scenario's shape when enabled. Checked, both ways.
    pub expect_cube: bool,
}

/// The brushable binding the suites drive: first binding in spec order —
/// plot A's interval brush.
fn first_binding(spec: &brightfield_spec::ast::Spec) -> Result<(String, ComponentPath), String> {
    let bindings = build_brushable_bindings(spec);
    let b = bindings
        .first()
        .ok_or_else(|| "scenario spec declares no brushable interactor".to_string())?;
    Ok((b.selection.clone(), b.parent_plot.clone()))
}

/// `(materialised_rows, drawn_rows)` for a query's chunks. `materialised_rows`
/// is the raw total; `drawn_rows` is what the presentation actually draws,
/// measured through the SAME [`assemble_batches`] path the compose step uses —
/// so the pair is a genuine cross-check, not a tautology. An assembly failure is
/// surfaced as `Err`, never silently reported as a smaller drawn count.
fn rows_of(batches: &[RecordBatch]) -> Result<(u64, u64), String> {
    let total: usize = batches.iter().map(RecordBatch::num_rows).sum();
    let drawn = assemble_batches(batches.to_vec())
        .map_err(|e| e.to_string())?
        .map_or(0, |b| b.num_rows());
    Ok((total as u64, drawn as u64))
}

/// Run the engine suites for one scenario over one dataset, with the
/// automatic pre-aggregation layer switched on or off (`preagg`) — the two
/// runs measure identical code, so their delta is the layer's contribution.
///
/// `iterations` timed brush applies run against BOTH seams (coordinator and
/// live-dashboard), each iteration with a distinct interval (see
/// [`brush_interval`]).
pub fn run_engine_suites(
    scenario: &Scenario,
    spec_dir: Option<&Path>,
    iterations: usize,
    preagg: bool,
) -> Result<EngineMeasurement, String> {
    let parsed = parse_spec(&scenario.spec_text, Format::Yaml)
        .map_err(|e| format!("{}: parse error: {e}", scenario.name))?;
    let spec = parsed.spec;
    let analysis =
        analyse_spec(&spec).map_err(|e| format!("{}: analysis error: {e}", scenario.name))?;
    let (selection, contributor) = first_binding(&spec)?;

    // --- Cold open: load, then first full materialisation. -----------------
    let t = Instant::now();
    let mut coord = Coordinator::load(spec.clone(), analysis, spec_dir)
        .map_err(|e| format!("{}: load error: {e}", scenario.name))?;
    let load_ms = t.elapsed().as_secs_f64() * 1000.0;
    coord.session_mut().set_preagg_enabled(preagg);

    let t = Instant::now();
    let results = coord.session_mut().execute_all();
    let first_materialise_ms = t.elapsed().as_secs_f64() * 1000.0;

    let mut marks = Vec::new();
    for (i, r) in results.iter().enumerate() {
        let batches = r.as_ref().map_err(|e| {
            format!(
                "{}: mark {i} failed on first materialise: {e}",
                scenario.name
            )
        })?;
        let (materialised_rows, drawn_rows) =
            rows_of(batches).map_err(|e| format!("{}: mark {i} assembly: {e}", scenario.name))?;
        marks.push(MarkRows {
            mark: i,
            materialised_rows,
            drawn_rows,
        });
    }

    // Non-vacuity ground: the cross-filtered mark's step, unfiltered.
    // Mark 1 is plot B's mark in every scenario (spec order).
    let unfiltered_step_rows = coord
        .session()
        .step_rows_count(1)
        .map_err(|e| format!("{}: step count: {e}", scenario.name))?;

    // --- Coordinator seam: per-interaction apply latency. ------------------
    let mut apply_ms = Vec::with_capacity(iterations);
    for i in 0..iterations {
        let interaction = brush_select(
            scenario.brush_column,
            scenario.brush_domain,
            &selection,
            &contributor,
            i,
        );
        let t = Instant::now();
        let requery = coord.apply(interaction);
        apply_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        if requery.affected.is_empty() {
            return Err(format!(
                "{}: brush apply {i} affected no marks — the measurement would be vacuous",
                scenario.name
            ));
        }
        for (mark_index, r) in &requery.affected {
            r.as_ref().map_err(|e| {
                format!(
                    "{}: apply {i} failed re-querying mark {mark_index}: {e}",
                    scenario.name
                )
            })?;
        }
    }

    // Non-vacuity: under the final brush, the cross-filtered step must hold
    // fewer rows than unfiltered — otherwise the applies filtered nothing.
    let brushed_step_rows = coord
        .session()
        .step_rows_count(1)
        .map_err(|e| format!("{}: brushed step count: {e}", scenario.name))?;
    if brushed_step_rows >= unfiltered_step_rows {
        return Err(format!(
            "{}: brush did not filter (step rows {brushed_step_rows} >= {unfiltered_step_rows})",
            scenario.name
        ));
    }

    // Non-vacuity for the layer itself: engaged where expected, silent where
    // not. A cube comparison that never engaged would measure nothing.
    let stats = coord.session().preagg_stats().clone();
    let preagg_summary = PreAggSummary {
        enabled: preagg,
        cubes_built: stats.cubes_built,
        cube_hits: stats.cube_hits,
        build_failures: stats.build_failures,
        serve_failures: stats.serve_failures,
    };
    if preagg && scenario.expect_cube && (stats.cubes_built == 0 || stats.cube_hits == 0) {
        return Err(format!(
            "{}: the pre-aggregation layer never engaged (built {}, hits {}) — \
             the enabled run would be measuring the direct path",
            scenario.name, stats.cubes_built, stats.cube_hits
        ));
    }
    if (!preagg || !scenario.expect_cube) && stats.cube_hits != 0 {
        return Err(format!(
            "{}: unexpected cube serving (hits {}) in a run that promised none",
            scenario.name, stats.cube_hits
        ));
    }
    drop(coord);

    // --- Live seam: apply + re-composite, on a fresh session. --------------
    let mut dash =
        brightfield_shell::pipeline::LiveDashboard::load_str(&scenario.spec_text, spec_dir)
            .map_err(|e| format!("{}: live load error: {e}", scenario.name))?;
    dash.coordinator().session_mut().set_preagg_enabled(preagg);
    dash.present()
        .map_err(|e| format!("{}: first present: {e}", scenario.name))?;
    let mut live_ms = Vec::with_capacity(iterations);
    for i in 0..iterations {
        let interaction = brush_select(
            scenario.brush_column,
            scenario.brush_domain,
            &selection,
            &contributor,
            i,
        );
        let t = Instant::now();
        dash.apply(interaction)
            .map_err(|e| format!("{}: live apply {i}: {e}", scenario.name))?;
        live_ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    drop(dash);

    Ok(EngineMeasurement {
        load_ms: (load_ms * 1000.0).round() / 1000.0,
        first_materialise_ms: (first_materialise_ms * 1000.0).round() / 1000.0,
        marks,
        coordinator_apply: Stats::from_ms(apply_ms).expect("iterations > 0"),
        live_apply: Stats::from_ms(live_ms).expect("iterations > 0"),
        brushed_step_rows,
        unfiltered_step_rows,
        preagg: preagg_summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brush_intervals_stay_distinct_for_a_full_suite() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..DISTINCT_BRUSH_INTERVALS {
            let (lo, hi) = brush_interval(i);
            assert!(lo < hi, "interval {i} is ordered");
            assert!(
                seen.insert(format!("{lo:.3}-{hi:.3}")),
                "interval {i} repeats — a repeated interval would hit the SQL cache"
            );
        }
    }

    #[test]
    fn distinct_limit_is_exactly_the_repeat_period() {
        // The constant is the cap the harness enforces; it must be the FIRST
        // collision, not merely below it. One past the last distinct index
        // wraps back onto iteration 0, so the cap is tight: raising it by one
        // would admit a repeated interval and re-time the SQL cache.
        assert_eq!(
            brush_interval(DISTINCT_BRUSH_INTERVALS),
            brush_interval(0),
            "iteration {DISTINCT_BRUSH_INTERVALS} must collide with iteration 0"
        );
    }

    #[test]
    fn domain_mapping_preserves_distinctness_and_bounds() {
        let domain = (0.8, 1.0);
        let mut seen = std::collections::HashSet::new();
        for i in 0..DISTINCT_BRUSH_INTERVALS {
            let (lo, hi) = brush_interval_in(domain, i);
            assert!(lo < hi, "interval {i} is ordered");
            assert!(lo >= domain.0 && hi <= domain.1, "inside the domain");
            assert!(
                seen.insert(format!("{lo:.6}-{hi:.6}")),
                "interval {i} repeats in the mapped domain"
            );
        }
        // The identity domain reproduces the unit intervals exactly.
        assert_eq!(brush_interval_in((0.0, 100.0), 3), brush_interval(3));
    }
}
