//! The engine-level suites: cold open, and interaction latency through the
//! coordinator seam — the exact seam the live shell blocks its frame on.

use std::path::Path;
use std::time::Instant;

use brightfield_engine::coordinator::{Coordinator, Interaction};
use brightfield_engine::{RecordBatch, SqlPredicate};
use brightfield_spec::analysis::{analyse_spec, build_brushable_bindings, ComponentPath};
use brightfield_spec::{parse_spec, Format};
use brightfield_sql::ir::ScalarValue;
use serde::Serialize;

use crate::stats::Stats;

/// The brushed interval for iteration `i` of a suite — a widening/narrowing
/// drag over `value_a`'s [0, 100) domain. The pairs stay distinct for 35
/// consecutive iterations, which matters: the engine's renderer-side SQL cache
/// short-circuits a repeated identical query, so re-using one interval would
/// time the cache, not the engine. A real drag never repeats a pixel either.
pub fn brush_interval(i: usize) -> (f64, f64) {
    let lo = 10.0 + (i % 5) as f64 * 4.0;
    let hi = 58.0 + (i % 7) as f64 * 5.0;
    (lo, hi)
}

/// The brush `Select` interaction for iteration `i`, against the binding's
/// selection and contributor — identical in shape to what the chart pane's
/// interval gesture produces.
pub fn brush_select(selection: &str, contributor: &ComponentPath, i: usize) -> Interaction {
    let (lo, hi) = brush_interval(i);
    Interaction::Select {
        name: selection.to_string(),
        contributor: contributor.clone(),
        predicate: SqlPredicate::Interval {
            column: "value_a".to_string(),
            lo: ScalarValue::Float(lo),
            hi: ScalarValue::Float(hi),
            meta: None,
        },
    }
}

/// Per-mark materialisation shape after the first full execute: how many rows
/// the query returned in total, and how many of them the first Arrow batch
/// holds. The presentation layer currently composes a mark's scene from the
/// FIRST batch only, so `first_batch_rows < materialised_rows` means the drawn
/// picture holds fewer rows than the query answered — recorded, not hidden.
#[derive(Debug, Clone, Serialize)]
pub struct MarkRows {
    /// Mark index in spec order.
    pub mark: usize,
    /// Total rows across all returned batches.
    pub materialised_rows: u64,
    /// Rows in the first batch (what the composed scene draws).
    pub first_batch_rows: u64,
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
    /// re-query of every affected mark. What the pre-aggregation layer is
    /// later measured against.
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
}

/// The parsed scenario inputs the suites share.
pub struct Scenario {
    /// Scenario id used in reports and spec filenames.
    pub name: &'static str,
    /// Fully substituted spec text.
    pub spec_text: String,
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

fn rows_of(batches: &[RecordBatch]) -> (u64, u64) {
    let total: usize = batches.iter().map(RecordBatch::num_rows).sum();
    let first = batches.first().map_or(0, RecordBatch::num_rows);
    (total as u64, first as u64)
}

/// Run the engine suites for one scenario over one dataset.
///
/// `iterations` timed brush applies run against BOTH seams (coordinator and
/// live-dashboard), each iteration with a distinct interval (see
/// [`brush_interval`]).
pub fn run_engine_suites(
    scenario: &Scenario,
    spec_dir: Option<&Path>,
    iterations: usize,
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
        let (materialised_rows, first_batch_rows) = rows_of(batches);
        marks.push(MarkRows {
            mark: i,
            materialised_rows,
            first_batch_rows,
        });
    }

    // Non-vacuity ground: the cross-filtered mark's step, unfiltered.
    // Mark 1 is plot B's mark in both scenarios (spec order).
    let unfiltered_step_rows = coord
        .session()
        .step_rows_count(1)
        .map_err(|e| format!("{}: step count: {e}", scenario.name))?;

    // --- Coordinator seam: per-interaction apply latency. ------------------
    let mut apply_ms = Vec::with_capacity(iterations);
    for i in 0..iterations {
        let interaction = brush_select(&selection, &contributor, i);
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
    drop(coord);

    // --- Live seam: apply + re-composite, on a fresh session. --------------
    let mut dash =
        brightfield_shell::pipeline::LiveDashboard::load_str(&scenario.spec_text, spec_dir)
            .map_err(|e| format!("{}: live load error: {e}", scenario.name))?;
    dash.present()
        .map_err(|e| format!("{}: first present: {e}", scenario.name))?;
    let mut live_ms = Vec::with_capacity(iterations);
    for i in 0..iterations {
        let interaction = brush_select(&selection, &contributor, i);
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brush_intervals_stay_distinct_for_a_full_suite() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..35 {
            let (lo, hi) = brush_interval(i);
            assert!(lo < hi, "interval {i} is ordered");
            assert!(
                seen.insert(format!("{lo:.3}-{hi:.3}")),
                "interval {i} repeats — a repeated interval would hit the SQL cache"
            );
        }
    }
}
