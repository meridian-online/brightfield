//! The engine-level suites: cold open, and interaction latency through the
//! coordinator seam — the exact seam the live shell blocks its frame on.

use std::path::Path;
use std::time::Instant;

use brightfield_engine::coordinator::{Coordinator, Interaction};
use brightfield_engine::{
    assemble_batches, AxisExtent, NavigationExtent, RecordBatch, SqlPredicate,
};
use brightfield_spec::analysis::{
    analyse_spec, build_brushable_bindings, plot_node_path, ComponentPath,
};
use brightfield_spec::{parse_spec, Format};
use brightfield_sql::ir::ScalarValue;
use serde::Serialize;

use crate::mem::{ComposeMemory, RssSampler};
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
/// the "every timed step is distinct" methodology guarantee.
///
/// TWO independent step budgets are bounded by this value in `Args::parse`,
/// because the harness has two: the engine suites' `--iterations`, and the
/// frame suites' `--warmup-frames + --frames` (the interaction frame suite
/// indexes its drag step by the frame counter). Validating only the first left
/// the second free to wrap — with the shipped defaults summing to exactly this
/// value, `--frames 31` was all it took.
pub const DISTINCT_BRUSH_INTERVALS: usize = 35;

/// The number of consecutive slider steps [`slider_interval`] yields before it
/// wraps. Held equal to [`DISTINCT_BRUSH_INTERVALS`] (proved by test) so one
/// period bounds BOTH drag shapes — a longer slider period would let a brush
/// suite silently re-issue an interval under a bound sized for the slider.
pub const DISTINCT_SLIDER_STEPS: usize = 35;

/// Which gesture a scenario's timed steps imitate.
///
/// The two shapes are not interchangeable. A brush moves BOTH interval
/// endpoints as a rectangle is dragged over a chart; a slider moves ONE handle
/// across fixed stops with its other end pinned. They land different SQL, put
/// different pressure on the cube, and answer different questions — so the
/// record names which one produced each row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Drag {
    /// A rectangular brush dragged inside a chart: both endpoints move.
    Brush,
    /// A range slider's upper handle dragged across its stops: the lower end
    /// stays pinned at the domain floor and the upper end advances one stop
    /// per step.
    Slider,
}

impl Drag {
    /// This gesture's interval for step `i`, in the unit domain [0, 100).
    #[must_use]
    pub fn interval(self, i: usize) -> (f64, f64) {
        match self {
            Self::Brush => brush_interval(i),
            Self::Slider => slider_interval(i),
        }
    }

    /// [`Drag::interval`] mapped onto a scenario's brushed-column domain: the
    /// unit-domain endpoints scale linearly onto `[domain.0, domain.1)`, so a
    /// dataset whose brushable axis is not [0, 100) still receives distinct,
    /// inside-the-data intervals.
    #[must_use]
    pub fn interval_in(self, domain: (f64, f64), i: usize) -> (f64, f64) {
        let (lo, hi) = self.interval(i);
        let span = (domain.1 - domain.0) / 100.0;
        (domain.0 + lo * span, domain.0 + hi * span)
    }
}

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

/// The slider interval for step `i` of a drag, in the unit domain [0, 100):
/// the lower end pinned at the floor, the upper handle advancing one stop per
/// step. `2.5` units per stop is one fortieth of the domain, so on a
/// forty-value axis every step lands exactly on a stop — a slider's positions
/// are discrete, and a step that fell between two of them would select the
/// same rows as its neighbour while still emitting different SQL.
///
/// Monotone in `i`, so the steps are distinct by construction; the modulus
/// wraps at [`DISTINCT_SLIDER_STEPS`] to keep the upper handle inside the
/// domain rather than running off its end.
pub fn slider_interval(i: usize) -> (f64, f64) {
    (0.0, 12.5 + (i % DISTINCT_SLIDER_STEPS) as f64 * 2.5)
}

/// The `Select` interaction for step `i` of `drag`: a structured interval over
/// `column` within `domain`, against the binding's selection and contributor —
/// identical in shape to what the chart pane's interval gesture (and, upstream,
/// a range slider's `select: interval`) produces. Structured clauses are what
/// let the pre-aggregation layer engage; a scalar param would arrive as a
/// substituted expression predicate and decompose into no cube at all.
pub fn drag_select(
    drag: Drag,
    column: &str,
    domain: (f64, f64),
    selection: &str,
    contributor: &ComponentPath,
    i: usize,
) -> Interaction {
    let (lo, hi) = drag.interval_in(domain, i);
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
    /// Arrow chunks the query returned for this mark. The compose holds them
    /// all at once, so the count is what turns a row total into a memory
    /// question.
    pub chunks: u64,
    /// Exact Arrow bytes across those chunks.
    pub chunk_bytes: u64,
    /// Exact Arrow bytes of the assembled drawable batch. A single-chunk mark
    /// assembles by pass-through, so this is the SAME allocation as
    /// `chunk_bytes`, not a second copy.
    pub assembled_bytes: u64,
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
    /// MARK re-queries served from a cube instead of the base table — the
    /// engine increments this once per mark it serves, not once per drag
    /// step. A scenario whose selection filters two subscribing marks records
    /// two hits per step, so `cube_hits` is a multiple of the step count and
    /// must never be reported as one. (Engine counterpart:
    /// `PreAggStats::cube_hits`.)
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
    /// Per-gesture `Coordinator::apply` latency for a **settled navigation** —
    /// the pan/zoom extent pushed at the aggregating mark's plot and every mark
    /// of that plot re-queried.
    ///
    /// Measured beside the brush rather than instead of it because they take
    /// different paths: a brush goes through selection propagation and a
    /// navigation resolves to an extent on the session. This column is why the
    /// difference stopped being invisible — it recorded a zoom taking the
    /// direct query in BOTH configurations while the brush beside it was served
    /// from a cube, which is what showed that the layer had one trigger and
    /// navigation reached it by no path.
    ///
    /// It is now measured under both configurations for the same reason the
    /// brush is: the engine keys a cube off an extent as well, so this figure
    /// has a delta to report, and quoting one arm would hide it.
    pub navigation_apply: Stats,
    /// Row count of the cross-filtered mark's step under the final brush —
    /// the non-vacuity evidence that the brush actually filtered.
    pub brushed_step_rows: u64,
    /// The same step's row count with no brush active.
    pub unfiltered_step_rows: u64,
    /// What the pre-aggregation layer did (coordinator-seam session).
    pub preagg: PreAggSummary,
    /// What the client held while the first full scene was composed.
    pub compose_memory: ComposeMemory,
}

/// The parsed scenario inputs the suites share.
pub struct Scenario {
    /// Scenario id used in reports and spec filenames.
    pub name: String,
    /// Fully substituted spec text.
    pub spec_text: String,
    /// Which gesture the timed steps imitate.
    pub drag: Drag,
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

/// The row and byte shape of one mark's query result, measured through the
/// SAME [`assemble_batches`] path the compose step uses — so `drawn_rows`
/// against `materialised_rows` is a genuine cross-check, not a tautology, and
/// the byte figures are the compose's real working set rather than an
/// estimate. An assembly failure is surfaced as `Err`, never silently reported
/// as a smaller drawn count.
fn shape_of(mark: usize, batches: &[RecordBatch]) -> Result<MarkRows, String> {
    let total: usize = batches.iter().map(RecordBatch::num_rows).sum();
    let chunk_bytes: usize = batches.iter().map(RecordBatch::get_array_memory_size).sum();
    let assembled = assemble_batches(batches.to_vec()).map_err(|e| e.to_string())?;
    Ok(MarkRows {
        mark,
        materialised_rows: total as u64,
        drawn_rows: assembled.as_ref().map_or(0, RecordBatch::num_rows) as u64,
        chunks: batches.len() as u64,
        chunk_bytes: chunk_bytes as u64,
        assembled_bytes: assembled
            .as_ref()
            .map_or(0, RecordBatch::get_array_memory_size) as u64,
    })
}

/// The `[min, max]` a column spans across a mark's drawn batches, or `None`
/// when the mark has no such column or no numeric rows in it.
fn column_domain(batches: &[RecordBatch], column: &str) -> Option<(f64, f64)> {
    use duckdb::arrow::array::{Array, Float64Array};
    use duckdb::arrow::compute::cast;
    use duckdb::arrow::datatypes::DataType;
    let mut domain: Option<(f64, f64)> = None;
    for batch in batches {
        let Ok(idx) = batch.schema().index_of(column) else {
            continue;
        };
        let Ok(col) = cast(batch.column(idx), &DataType::Float64) else {
            continue;
        };
        let Some(arr) = col.as_any().downcast_ref::<Float64Array>() else {
            continue;
        };
        for i in 0..arr.len() {
            if arr.is_null(i) {
                continue;
            }
            let v = arr.value(i);
            domain = Some(match domain {
                None => (v, v),
                Some((lo, hi)) => (lo.min(v), hi.max(v)),
            });
        }
    }
    domain.filter(|(lo, hi)| hi > lo)
}

/// Everything the record needs from one phase's complete result set — and
/// nothing that keeps the batches alive.
#[derive(Debug, Clone)]
struct PhaseShapes {
    /// Per-mark row and byte shape, in spec order.
    marks: Vec<MarkRows>,
    /// Exact Arrow bytes across every chunk of every mark.
    chunk_bytes: u64,
    /// Exact Arrow bytes across every mark's assembled drawable batch.
    assembled_bytes: u64,
}

/// Measure the per-mark shape of a phase's complete result set, CONSUMING it.
///
/// Taking ownership is the measurement, not a style preference. At ten million
/// row-level rows this result set is hundreds of mebibytes of Arrow, and the
/// compose-memory window further down reports the process's resident size as
/// the COMPOSE's client cost. A caller that still held these batches when that
/// window opened would have two complete result sets resident and publish
/// their sum under the compose's name — which is precisely what an earlier
/// revision of this harness did, inflating the largest cell by the whole
/// coordinator phase. Moving the batches in here makes that unrepresentable:
/// they are dropped before this function returns, and the caller has no
/// binding left to hold.
fn consume_phase_shapes<E: std::fmt::Display>(
    scenario: &str,
    results: Vec<Result<Vec<RecordBatch>, E>>,
) -> Result<PhaseShapes, String> {
    let mut marks = Vec::with_capacity(results.len());
    for (i, r) in results.into_iter().enumerate() {
        let batches =
            r.map_err(|e| format!("{scenario}: mark {i} failed on first materialise: {e}"))?;
        marks.push(
            shape_of(i, &batches).map_err(|e| format!("{scenario}: mark {i} assembly: {e}"))?,
        );
        // `batches` drops here, at the end of this iteration.
    }
    Ok(PhaseShapes {
        chunk_bytes: marks.iter().map(|m| m.chunk_bytes).sum(),
        assembled_bytes: marks.iter().map(|m| m.assembled_bytes).sum(),
        marks,
    })
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

    // The coordinator phase's whole result set is measured and DROPPED here —
    // `consume_phase_shapes` takes it by value, so no binding survives this
    // line. Nothing below holds Arrow from this phase, which is what lets the
    // compose-memory window further down describe the compose alone.
    let shapes = consume_phase_shapes(&scenario.name, results)?;
    let marks = shapes.marks;
    let arrow_chunk_bytes = shapes.chunk_bytes;
    let arrow_assembled_bytes = shapes.assembled_bytes;

    // Non-vacuity ground: the cross-filtered mark's step, unfiltered.
    // Mark 1 is plot B's mark in every scenario (spec order).
    let unfiltered_step_rows = coord
        .session()
        .step_rows_count(1)
        .map_err(|e| format!("{}: step count: {e}", scenario.name))?;

    // --- Coordinator seam: per-interaction apply latency. ------------------
    let mut apply_ms = Vec::with_capacity(iterations);
    for i in 0..iterations {
        let interaction = drag_select(
            scenario.drag,
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

    // --- Navigation seam: per-SETTLED-GESTURE apply latency. ---------------
    //
    // Navigated on mark 1 (plot B in every scenario) along ITS OWN x channel,
    // over the range that mark's rows actually span. Reusing the brush column
    // and the brush domain would have been shorter and was wrong: two of the
    // four scenarios brush a column plot B does not draw, so the extent matched
    // no mark, the SQL came back byte-identical, the cache served it and the
    // record printed 0.0 ms for a gesture that did nothing. The non-vacuity
    // check below is what makes that unrepeatable.
    let (nav_plot, nav_column) = {
        let marks = brightfield_sql::emit::collect_marks_with_paths(&spec);
        let (mark, path) = marks
            .get(1)
            .ok_or_else(|| format!("{}: no second mark to navigate", scenario.name))?;
        let column = match mark.options.get("x") {
            Some(brightfield_spec::ast::ValueOrParamRef::Value(v)) => v
                .as_str()
                .ok_or_else(|| format!("{}: mark 1's x is not a column", scenario.name))?
                .to_string(),
            _ => return Err(format!("{}: mark 1 declares no x column", scenario.name)),
        };
        (plot_node_path(path).to_string(), column)
    };
    // The range that column actually spans, read off the mark's own drawn
    // batch — no assumption about how the dataset was generated.
    let nav_domain = {
        let batches = coord
            .session_mut()
            .execute_mark(1)
            .map_err(|e| format!("{}: reading mark 1 for its domain: {e}", scenario.name))?;
        column_domain(&batches, &nav_column).ok_or_else(|| {
            format!(
                "{}: mark 1 draws no {nav_column} to navigate",
                scenario.name
            )
        })?
    };
    let unnavigated_rows: usize = coord
        .session_mut()
        .execute_mark(1)
        .map_err(|e| format!("{}: mark 1 at full extent: {e}", scenario.name))?
        .iter()
        .map(RecordBatch::num_rows)
        .sum();

    let mut nav_ms = Vec::with_capacity(iterations);
    let mut navigated_rows = unnavigated_rows;
    for i in 0..iterations {
        let (lo, hi) = scenario.drag.interval_in(nav_domain, i);
        let interaction = Interaction::Navigate {
            plot: ComponentPath(nav_plot.clone()),
            extent: NavigationExtent {
                x: Some(AxisExtent::new(nav_column.clone(), lo, hi)),
                y: None,
            },
        };
        let t = Instant::now();
        let requery = coord.apply(interaction);
        nav_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        if requery.affected.is_empty() {
            return Err(format!(
                "{}: navigation {i} affected no marks — the measurement would be vacuous",
                scenario.name
            ));
        }
        for (mark_index, r) in &requery.affected {
            let batches = r.as_ref().map_err(|e| {
                format!(
                    "{}: navigation {i} failed re-querying mark {mark_index}: {e}",
                    scenario.name
                )
            })?;
            if *mark_index == 1 {
                navigated_rows = batches.iter().map(RecordBatch::num_rows).sum();
            }
        }
    }
    // Non-vacuity: the last extent must have scoped the mark. A navigation that
    // changed nothing costs a cache lookup, and reporting that as the price of
    // a zoom is exactly the borrowed-number failure this directory exists to
    // prevent.
    if navigated_rows >= unnavigated_rows {
        return Err(format!(
            "{}: the navigation extent filtered nothing on mark 1 ({navigated_rows} rows \
             against {unnavigated_rows} unnavigated) — the measured cost would be a cache hit",
            scenario.name
        ));
    }

    // Put the frame back before anything else is measured: a leftover extent
    // would scope every figure below under a gesture the record does not name.
    coord.apply(Interaction::Navigate {
        plot: ComponentPath(nav_plot),
        extent: NavigationExtent::default(),
    });

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
    // The compose-time memory window: `present` executes every mark and hands
    // the whole result set to `compose_from_results`, which assembles every
    // chunk of every mark and holds them while it builds the scene. Sampling
    // wraps THIS call and stops before the timed applies below, so the poller
    // cannot perturb a reported latency.
    //
    // Two things must already be gone when this opens, or the window measures
    // more than one compose: the coordinator (dropped above, taking its
    // session and its query cache) and the coordinator phase's result set
    // (moved into `consume_phase_shapes`, which drops it — see the note
    // there). Resident size is a whole-PROCESS quantity; anything still held
    // is reported as the compose's cost.
    let sampler = RssSampler::start();
    let present = dash.present();
    let compose_memory = sampler.finish(arrow_chunk_bytes, arrow_assembled_bytes);
    present.map_err(|e| format!("{}: first present: {e}", scenario.name))?;
    let mut live_ms = Vec::with_capacity(iterations);
    for i in 0..iterations {
        let interaction = drag_select(
            scenario.drag,
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
        navigation_apply: Stats::from_ms(nav_ms).expect("iterations > 0"),
        live_apply: Stats::from_ms(live_ms).expect("iterations > 0"),
        brushed_step_rows,
        unfiltered_step_rows,
        preagg: preagg_summary,
        compose_memory,
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
            let (lo, hi) = Drag::Brush.interval_in(domain, i);
            assert!(lo < hi, "interval {i} is ordered");
            assert!(lo >= domain.0 && hi <= domain.1, "inside the domain");
            assert!(
                seen.insert(format!("{lo:.6}-{hi:.6}")),
                "interval {i} repeats in the mapped domain"
            );
        }
        // The identity domain reproduces the unit intervals exactly.
        assert_eq!(Drag::Brush.interval_in((0.0, 100.0), 3), brush_interval(3));
    }

    #[test]
    fn slider_steps_stay_distinct_for_a_full_suite() {
        let mut seen = std::collections::HashSet::new();
        let mut last_hi = f64::NEG_INFINITY;
        for i in 0..DISTINCT_SLIDER_STEPS {
            let (lo, hi) = slider_interval(i);
            assert!(lo < hi, "step {i} is ordered");
            assert!(hi < 100.0, "step {i} keeps the handle inside the domain");
            assert!(hi > last_hi, "step {i} advances the handle");
            last_hi = hi;
            assert!(
                seen.insert(format!("{lo:.3}-{hi:.3}")),
                "step {i} repeats — a repeated step would hit the SQL cache"
            );
        }
    }

    #[test]
    fn slider_is_one_handle_and_the_brush_is_two() {
        // The distinction the scenario exists to measure: the slider pins its
        // lower end and only the upper handle moves; the brush moves both.
        let los: std::collections::HashSet<String> = (0..8)
            .map(|i| format!("{:.3}", slider_interval(i).0))
            .collect();
        assert_eq!(los.len(), 1, "a slider drag pins its lower end");
        let brush_los: std::collections::HashSet<String> = (0..8)
            .map(|i| format!("{:.3}", brush_interval(i).0))
            .collect();
        assert!(brush_los.len() > 1, "a brush drag moves both endpoints");
    }

    #[test]
    fn one_period_covers_both_drag_shapes() {
        // The harness bounds TWO step budgets against this one constant: the
        // `--iterations` count of the engine suites, and the
        // `--warmup-frames + --frames` budget of the frame suites (both in
        // `main`, each with its own validator — a single validator never
        // covered both, which is how the frame budget went unbounded).
        // Sharing one constant is only sound while no generator repeats
        // earlier than it does; otherwise a run inside the bound could still
        // re-issue a step and time the SQL cache.
        assert_eq!(
            DISTINCT_SLIDER_STEPS, DISTINCT_BRUSH_INTERVALS,
            "the two generators must share one period for one cap to cover both"
        );
        assert_eq!(
            slider_interval(DISTINCT_SLIDER_STEPS),
            slider_interval(0),
            "step {DISTINCT_SLIDER_STEPS} must collide with step 0"
        );
    }

    #[test]
    fn slider_steps_land_on_a_forty_stop_axis() {
        // The bench slider sweeps a forty-value column. Every step must land
        // exactly on a stop: a step BETWEEN two stops emits different SQL but
        // selects the identical rows, which would report a re-query that did
        // no new work as though it had.
        for i in 0..DISTINCT_SLIDER_STEPS {
            let (lo, hi) = Drag::Slider.interval_in((0.0, 40.0), i);
            assert!((lo - 0.0).abs() < 1e-9, "step {i} pins the floor");
            assert!(
                (hi - hi.round()).abs() < 1e-9,
                "step {i} lands between stops: {hi}"
            );
            assert!((5.0..40.0).contains(&hi), "step {i} stays in [0,40): {hi}");
        }
    }

    /// One int column, three rows — enough to be measured, small enough to
    /// build inline. The returned `ArrayRef` is a second handle on the batch's
    /// only buffer, so its reference count says whether the batch is still
    /// alive anywhere.
    fn one_batch() -> (RecordBatch, std::sync::Arc<dyn duckdb::arrow::array::Array>) {
        use duckdb::arrow::array::{ArrayRef, Int32Array};
        use std::sync::Arc;
        let col: ArrayRef = Arc::new(Int32Array::from(vec![1, 2, 3]));
        let batch =
            RecordBatch::try_from_iter([("v", col.clone())]).expect("a one-column batch builds");
        (batch, col)
    }

    #[test]
    fn the_shape_pass_drops_the_batches_it_measured() {
        // The compose-memory window opened later in a run reports the whole
        // PROCESS's resident size. If the coordinator phase's result set were
        // still alive then, the published figure would be two result sets, not
        // one — the contamination this consuming signature exists to prevent.
        use std::sync::Arc;
        let (batch, col) = one_batch();
        assert_eq!(
            Arc::strong_count(&col),
            2,
            "the batch holds the only other handle on the column"
        );

        let shapes = consume_phase_shapes::<String>("t", vec![Ok(vec![batch])])
            .expect("a well-formed result set measures");
        assert_eq!(shapes.marks.len(), 1);
        assert_eq!(shapes.marks[0].materialised_rows, 3);
        assert_eq!(shapes.marks[0].drawn_rows, 3);
        assert!(shapes.chunk_bytes > 0, "a measured batch has bytes");

        assert_eq!(
            Arc::strong_count(&col),
            1,
            "the measured batches must be dropped by the shape pass — a caller \
             still holding them would put TWO complete result sets inside the \
             compose-memory window and publish their sum as the compose's cost"
        );
    }

    #[test]
    fn the_shape_pass_drops_the_batches_even_when_a_mark_failed() {
        // The error path drops the rest of the result set too: a run that
        // fails still must not leave hundreds of megabytes of Arrow alive.
        use std::sync::Arc;
        let (batch, col) = one_batch();
        let err =
            consume_phase_shapes::<String>("t", vec![Err("boom".to_string()), Ok(vec![batch])])
                .expect_err("a failed mark fails the phase");
        assert!(err.contains("mark 0 failed on first materialise"), "{err}");
        assert_eq!(
            Arc::strong_count(&col),
            1,
            "the un-measured remainder is dropped with the iterator"
        );
    }

    #[test]
    fn drag_dispatch_matches_the_generators() {
        assert_eq!(Drag::Brush.interval(4), brush_interval(4));
        assert_eq!(Drag::Slider.interval(4), slider_interval(4));
        assert_ne!(Drag::Brush.interval(4), Drag::Slider.interval(4));
    }
}
