//! Brightfield application entry point.
//!
//! Orchestrates the full spec-to-chart pipeline:
//! parse → analyse → engine → execute → render → display.
//!
//! The GPUI window requires a platform implementation (gpui_macos on macOS,
//! which needs full Xcode + Metal compiler). Without it, the pipeline runs
//! headlessly and prints a summary.

mod boot;
#[cfg(any(target_os = "macos", test))]
mod dock_state_file;
#[cfg(any(target_os = "macos", test))]
mod log_model;
#[cfg(any(target_os = "macos", test))]
mod reload_feedback;
#[cfg(any(target_os = "macos", test))]
mod shell;
#[cfg(any(target_os = "macos", test))]
mod profile_model;
#[cfg(any(target_os = "macos", test))]
mod shell_model;
#[cfg(any(target_os = "macos", test))]
mod spec_save;

use std::collections::{HashMap, HashSet};
use std::env;
use std::path::Path;
use std::process;

use brightfield_engine::{Engine, Session, SourceProfile};
use brightfield_render::channel::{Channel, ChannelMap};
use brightfield_render::layout::ChartLayout;
use brightfield_render::legend::{colour_legend_size, render_colour_legend_at};
use brightfield_render::mark::{configured_renderer, default_renderers, find_renderer, MarkRenderer};
use brightfield_render::scale::{Scale, ScaleSet, SequentialScheme};
use brightfield_render::scene::{build_multi_mark_scene, compose_dashboard, ChartData};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::layout::{
    collect_legend_nodes, collect_plot_nodes, placed_input_nodes, placed_legend_nodes,
    placed_legends, placed_plots, Rect,
};
use brightfield_spec::parse_spec_path;
use brightfield_spec::vocab::{InputKind, LegendChannel};
use brightfield_sql::{collect_marks, collect_plot_groups};
use brightfield_ui::chart_view::BrushBinding;
use brightfield_ui::{CrossfilterCoordinator, LivePlot, MarkInput, SliderBinding};

/// Concatenate the record batches from one mark's query into a single batch.
///
/// DuckDB streams results one batch per internal vector (~2048 rows), so a
/// query returning more than one chunk arrives as several batches. Rendering
/// only the first would silently drop the rest. Returns `None` for an empty
/// result. On the rare concat failure, falls back to the first batch (with a
/// warning) rather than dropping the mark entirely.
fn concat_result_batches(
    batches: Vec<arrow::record_batch::RecordBatch>,
) -> Option<arrow::record_batch::RecordBatch> {
    match batches.len() {
        0 => None,
        1 => batches.into_iter().next(),
        _ => {
            let schema = batches[0].schema();
            match arrow::compute::concat_batches(&schema, &batches) {
                Ok(batch) => Some(batch),
                Err(e) => {
                    eprintln!(
                        "warning: could not concatenate {} result batches ({e}); \
                         rendering the first chunk only",
                        batches.len()
                    );
                    batches.into_iter().next()
                }
            }
        }
    }
}

/// One rendered plot: its component-path identity, position/size in the
/// dashboard, and its own scene.
struct PlotRender {
    path: String,
    x: f64,
    y: f64,
    width: u32,
    height: u32,
    scene: vello::Scene,
}

/// A slider widget's placement (card 0005): its dashboard rect, the param binding
/// it drives, and the thumb's resting value. The window path turns each into a
/// hosted `SliderElement`; the headless/PNG path draws it into the composite.
struct SliderPlacement {
    rect: Rect,
    binding: SliderBinding,
    value: f64,
}

/// A standalone `legend:` node's placement (multi-view inc 6): its component
/// path, its dashboard rect, and the colour scale it displays, resolved from
/// the plot its `for:` names. The headless/PNG path draws it into the
/// composite; the window hosts it as a `LegendElement` at the same rect (card
/// 0016) — display-only unless the node is bound `as:` a selection, in which
/// case `path` joins it to its analysis [`LegendBinding`] (card 0009).
///
/// [`LegendBinding`]: brightfield_spec::analysis::LegendBinding
struct LegendPlacement {
    path: String,
    rect: Rect,
    scale: Scale,
}

/// A rendered dashboard: the bounding-box dimensions, one scene per plot, the
/// placed slider widgets, the standalone legends, and the spec's declared
/// `meta.title` (if any — the window title resolver consumes it). The
/// headless/PNG path composites these; the window hosts one element per plot +
/// one per slider + one per legend.
struct Dashboard {
    width: u32,
    height: u32,
    plots: Vec<PlotRender>,
    sliders: Vec<SliderPlacement>,
    legends: Vec<LegendPlacement>,
    meta_title: Option<String>,
}

/// Map each resolved [`LegendPlacement`] to its hosted window descriptor —
/// one [`brightfield_ui::PlacedLegend`] per placement, at the placement's
/// rect (card 0016). The live path and the fww_ac05/lcf_ac05 view-model
/// tests share this single mapping.
///
/// A placement whose node carries a producer binding (card 0009) — matched
/// by legend path against `bindings`, categorical [`Scale::Colour`] only —
/// is additionally wired to `coordinator` at its binding index, so a swatch
/// click commits through `commit_legend_click`. Sequential (gradient) and
/// unbound legends stay display-only, exactly as 0016 shipped.
#[cfg(any(target_os = "macos", test))]
fn placed_legend_views(
    legends: &[LegendPlacement],
    bindings: &[brightfield_spec::analysis::LegendBinding],
    coordinator: Option<&std::rc::Rc<std::cell::RefCell<brightfield_ui::CrossfilterCoordinator>>>,
) -> Vec<brightfield_ui::PlacedLegend> {
    legends
        .iter()
        .map(|l| {
            let placed = brightfield_ui::PlacedLegend::new(
                l.rect.x,
                l.rect.y,
                l.rect.width,
                l.rect.height,
                l.scale.clone(),
            );
            let bound = coordinator.and_then(|coord| {
                if !matches!(l.scale, Scale::Colour { .. }) {
                    return None;
                }
                bindings
                    .iter()
                    .position(|b| b.legend_path.0 == l.path)
                    .map(|index| (index, coord.clone()))
            });
            match bound {
                Some((index, coord)) => placed.with_binding(index, coord),
                None => placed,
            }
        })
        .collect()
}

/// The colour (fill/stroke) scale of a plot's [`ScaleSet`], if it has one — the
/// scale a standalone legend for that plot displays. Fill takes precedence over
/// stroke (a mark colour-encoded on both is unusual; fill is the common case).
/// Accepts both a categorical [`Scale::Colour`] (swatch legend) and a continuous
/// [`Scale::Sequential`] (gradient-bar legend, e.g. a raster's count ramp).
fn colour_scale_of(scales: &ScaleSet) -> Option<Scale> {
    // Filter each channel to a colour scale BEFORE falling back — otherwise a
    // present-but-non-colour Fill (e.g. a numeric fill inferred as Linear) would
    // short-circuit `or_else` and mask a real categorical Stroke colour scale.
    let is_colour = |s: &&Scale| matches!(s, Scale::Colour { .. } | Scale::Sequential { .. });
    scales
        .get(Channel::Fill)
        .filter(is_colour)
        .or_else(|| scales.get(Channel::Stroke).filter(is_colour))
        .cloned()
}

/// Resolve a raster plot's colour scheme from its `colorScheme` attribute,
/// defaulting to viridis and warning on an unrecognised name. `colorScheme` is a
/// plot-level attribute (Mosaic's colour scale is plot-scoped). The resolved
/// scheme is baked into each mark's `MarkInput::renderer_override`
/// (`configured_renderer`) at assembly, which is what carries scheme fidelity to
/// the live rebuild; it is ALSO recorded in [`LivePlotMeta::scheme`] for the
/// hot-reload chrome gate only (card 0016).
fn raster_scheme(color_scheme: Option<&brightfield_spec::ast::SpecValue>) -> SequentialScheme {
    use brightfield_spec::ast::SpecValue;
    match color_scheme {
        Some(SpecValue::String(name)) => SequentialScheme::from_wire(name).unwrap_or_else(|| {
            eprintln!("warning: unknown colorScheme {name:?} — falling back to viridis");
            SequentialScheme::default()
        }),
        _ => SequentialScheme::default(),
    }
}

/// Literal numeric mark attribute (e.g. `bandwidth: 15`), skipping params —
/// read at assembly time so a mark-level attribute reaches its per-mark
/// renderer override (card 0008, density marks).
fn mark_attr_f64(mark: &brightfield_spec::ast::Mark, key: &str) -> Option<f64> {
    use brightfield_spec::ast::{SpecValue, ValueOrParamRef};
    match mark.options.get(key)? {
        ValueOrParamRef::Value(SpecValue::Float(f)) => Some(*f),
        ValueOrParamRef::Value(SpecValue::Integer(i)) => Some(*i as f64),
        _ => None,
    }
}

/// How a standalone legend's `for:` attribute is authored.
enum LegendFor {
    /// No `for:` key — eligible for the sole-colour-plot fallback.
    Absent,
    /// `for: <literal-name>` — must resolve to that named plot or be skipped.
    Named(String),
    /// `for:` present but not a literal string (e.g. a param `$sel`) —
    /// unsupported, so skipped rather than silently borrowing another scale.
    Unresolvable,
}

/// Classify a standalone legend's `for:` attribute.
fn legend_for(node: &brightfield_spec::ast::LegendNode) -> LegendFor {
    use brightfield_spec::ast::{SpecValue, ValueOrParamRef};
    match node.options.get("for") {
        None => LegendFor::Absent,
        Some(ValueOrParamRef::Value(SpecValue::String(s))) => LegendFor::Named(s.clone()),
        Some(_) => LegendFor::Unresolvable,
    }
}

/// Resolve each standalone `legend:` node to a positioned colour scale.
///
/// A legend displays the colour scale of the plot its `for:` names (matched
/// against the plot's `name` attribute). When `for:` is absent or unmatched and
/// the dashboard has exactly one colour-encoded plot, that plot's scale is used
/// (the common single-legend case). A legend that resolves to no colour scale —
/// or names a non-`color` channel (opacity/symbol are unimplemented) — is
/// skipped with a diagnostic. Multi-colour-scale disambiguation and `for:`
/// validation errors are a follow-up.
fn resolve_legends(spec: &brightfield_spec::ast::Spec, live_plots: &[LivePlotMeta]) -> Vec<LegendPlacement> {
    // path → colour scale, for every colour-encoded plot.
    let mut by_path: HashMap<&str, Scale> = HashMap::new();
    for lp in live_plots {
        if let Some(cs) = colour_scale_of(&lp.scales) {
            by_path.insert(lp.path.as_str(), cs);
        }
    }
    // name → colour scale, for plots that carry a `name` attribute. A duplicate
    // name is an authoring error (the `for:` reference becomes ambiguous); warn
    // rather than silently resolving to whichever plot happens to be last.
    let mut by_name: HashMap<String, Scale> = HashMap::new();
    for (path, node) in collect_plot_nodes(spec) {
        if let (Some(cs), Some(brightfield_spec::ast::SpecValue::String(name))) =
            (by_path.get(path.as_str()), node.attributes.get("name"))
        {
            if by_name.insert(name.clone(), cs.clone()).is_some() {
                eprintln!(
                    "warning: two colour-encoded plots share name {name:?} — a legend `for: {name}` is ambiguous; using the last"
                );
            }
        }
    }
    // The dashboard's sole colour scale — the `for:`-absent convenience fallback.
    let sole = if by_path.len() == 1 {
        by_path.values().next().cloned()
    } else {
        None
    };

    let mut out = Vec::new();
    // The placed-rect ↔ AST-node join, path retained: the placement's path is
    // what joins a bound legend to its analysis LegendBinding (card 0009).
    let nodes = collect_legend_nodes(spec);
    for placed in placed_legends(spec, Rect::new(0.0, 0.0, 0.0, 0.0)) {
        let Some((_, node)) = nodes.iter().find(|(path, _)| path == &placed.path) else {
            continue;
        };
        let rect = placed.rect;
        if node.channel != LegendChannel::Color {
            eprintln!(
                "warning: standalone legend channel {:?} is unimplemented — skipping",
                node.channel
            );
            continue;
        }
        // An explicit `for:` must resolve to that named plot's scale — never
        // silently borrow another plot's. The sole-colour-plot convenience
        // applies ONLY when `for:` is genuinely absent; a present-but-
        // unresolvable `for:` (a typo'd name or a param) is skipped + warned.
        let scale = match legend_for(node) {
            LegendFor::Named(name) => match by_name.get(&name) {
                Some(scale) => Some(scale.clone()),
                None => {
                    eprintln!("warning: standalone legend `for: {name}` names no colour-encoded plot — skipping");
                    None
                }
            },
            LegendFor::Absent => {
                if sole.is_none() {
                    eprintln!("warning: standalone legend has no `for:` and the dashboard has no single colour-encoded plot — skipping");
                }
                sole.clone()
            }
            LegendFor::Unresolvable => {
                eprintln!("warning: standalone legend `for:` must be a literal plot name — skipping");
                None
            }
        };
        if let Some(scale) = scale {
            // Size the placement to the panel the renderer will actually draw
            // (content-sized), so the composite bounding-box fold reserves enough
            // room and the legend is never clipped off-canvas. The layout node's
            // fixed 120×24 reservation is only a placeholder (the scale is unknown
            // at layout time). Residual: a legend FOLLOWED by another element in a
            // concat can still overlap it — single-pass layout can't reflow.
            let (w, h) = colour_legend_size(&scale).unwrap_or((rect.width, rect.height));
            out.push(LegendPlacement {
                path: placed.path,
                rect: Rect::new(rect.x, rect.y, w, h),
                scale,
            });
        }
    }
    out
}

/// Reconcile the analysis-side legend producer bindings against the LIVE
/// legend placements (card 0009 F4, interim until the two population counts
/// unify). `build_legend_bindings` counts colour-encoded plots by STRING
/// `fill:`/`stroke:` option, while [`resolve_legends`] counts by live
/// Colour/Sequential scale — the populations diverge (a numeric fill counts
/// statically but infers Linear; a raster plot counts live via its Sequential
/// Fill but has no fill option), so their sole-plot fallbacks can disagree.
///
/// (a) A binding whose legend has NO placement is discarded — a phantom
/// binding would hold the coordinator open with no clickable surface.
/// (b) A PLACED legend carrying `as:` with no surviving binding gets a
/// diagnostic — its clicks would silently do nothing.
///
/// Returns the surviving bindings plus human-readable diagnostics (the caller
/// eprintlns them); pure so both halves are headlessly testable.
fn reconcile_legend_bindings(
    spec: &brightfield_spec::ast::Spec,
    placements: &[LegendPlacement],
    bindings: Vec<brightfield_spec::analysis::LegendBinding>,
) -> (Vec<brightfield_spec::analysis::LegendBinding>, Vec<String>) {
    use brightfield_spec::ast::ValueOrParamRef;

    let placed: HashSet<&str> = placements.iter().map(|l| l.path.as_str()).collect();
    let (retained, orphaned): (Vec<_>, Vec<_>) = bindings
        .into_iter()
        .partition(|b| placed.contains(b.legend_path.0.as_str()));

    let mut diagnostics: Vec<String> = orphaned
        .iter()
        .map(|b| {
            format!(
                "legend binding `as: ${}` at {} has no hosted legend — discarding \
                 (the legend did not resolve to a colour scale, so its clicks would \
                 have no surface)",
                b.selection, b.legend_path.0
            )
        })
        .collect();

    let nodes = collect_legend_nodes(spec);
    for l in placements {
        if retained.iter().any(|b| b.legend_path.0 == l.path) {
            continue;
        }
        let carries_as = nodes
            .iter()
            .find(|(path, _)| path == &l.path)
            .is_some_and(|(_, node)| {
                matches!(node.options.get("as"), Some(ValueOrParamRef::Param(_)))
            });
        if carries_as {
            diagnostics.push(format!(
                "legend at {} carries `as:` but no selection binding matched — \
                 clicks on it will not filter",
                l.path
            ));
        }
    }
    (retained, diagnostics)
}

/// Coerce a scalar `SpecValue` (a param default) to `f64` for a slider's resting
/// value; `None` for non-scalar params.
fn spec_value_as_f64(v: &brightfield_spec::ast::SpecValue) -> Option<f64> {
    use brightfield_spec::ast::SpecValue;
    match v {
        SpecValue::Integer(i) => Some(*i as f64),
        SpecValue::Float(f) => Some(*f),
        _ => None,
    }
}

/// Live engine + render state kept alive for in-window interaction (cross-filter).
/// Only the window path uses this; the headless/PNG path and the hot-reload
/// watcher drop it — dropping the non-`Send` [`Session`] is what lets the watcher
/// run the pipeline off the main thread.
struct LiveParts {
    session: Session,
    /// Per flat mark index (aligned with `collect_marks` order).
    marks: Vec<MarkInput>,
    /// Per plot, aligned 1:1 with `Dashboard.plots`.
    plots: Vec<LivePlotMeta>,
    /// Legend producer bindings (card 0009), in analysis order — the
    /// coordinator's legend index space; placements join by legend path.
    legend_bindings: Vec<brightfield_spec::analysis::LegendBinding>,
}

/// Per-plot live metadata captured during rendering, joined to its `ChartState`
/// entity in `main` to build the [`CrossfilterCoordinator`].
struct LivePlotMeta {
    path: String,
    mark_indices: Vec<usize>,
    layout: ChartLayout,
    bindings: Vec<BrushBinding>,
    scales: ScaleSet,
    /// Whether the plot draws its own inline colour legend (false when a
    /// standalone `legend:` node relocated it) — carried to the coordinator so a
    /// live re-render keeps the same suppression.
    draw_inline_legend: bool,
    /// The plot's resolved `colorScheme` (default viridis), for the hot-reload
    /// chrome gate ONLY (feeds `ChromeSnapshot::plot_render_meta`): a rebuild
    /// that changes a plot's scheme must fall back to "restart to apply", since
    /// the watcher never rebuilds the coordinator and a gesture would otherwise
    /// re-run the old scheme. Live-rebuild scheme fidelity does NOT ride this
    /// field — it rides each mark's `MarkInput::renderer_override`.
    scheme: SequentialScheme,
}

/// The launch-fixed chrome + render metadata the hot-reload gate compares
/// against each rebuilt spec. The watcher can hot-swap plot scenes, but the
/// window's chrome (titlebar/header title), its hosted legends, the legend
/// selection bindings (a bound legend's click wiring — `as:`/`for:` — is
/// captured into the coordinator at launch), and the coordinator's per-plot
/// render metadata (colorScheme, inline-legend suppression) are all captured
/// at launch — a rebuild that changes any of them must fall back to "restart
/// to apply" rather than silently reloading with stale chrome, stale click
/// wiring, or reverting on the next gesture-driven re-render.
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, PartialEq)]
struct ChromeSnapshot {
    /// Resolved display title (`meta.title` or the spec filename stem).
    title: String,
    /// Per hosted legend, in document order: layout rect plus a cheap
    /// structural key of the displayed scale (its `Debug` form).
    legends: Vec<(f64, f64, f64, f64, String)>,
    /// Per legend producer binding (card 0009), in analysis order: the click
    /// wiring keys (legend path, `for:`-plot path, selection name, colour
    /// column). An `as:`/`for:`-only edit changes these WITHOUT moving any
    /// legend rect or scale, so the gate needs them explicitly — otherwise a
    /// hot swap would keep the launch-time coordinator's stale click wiring.
    legend_bindings: Vec<(String, String, String, String)>,
    /// Per plot, in dashboard order: path, resolved colour scheme, and
    /// whether the plot draws its own inline legend.
    plot_render_meta: Vec<(String, SequentialScheme, bool)>,
}

#[cfg(any(target_os = "macos", test))]
impl ChromeSnapshot {
    /// Snapshot the chrome-relevant slice of a built dashboard.
    fn capture(
        title: String,
        legends: &[LegendPlacement],
        bindings: &[brightfield_spec::analysis::LegendBinding],
        plots: &[LivePlotMeta],
    ) -> Self {
        Self {
            title,
            legends: legends
                .iter()
                .map(|l| {
                    (
                        l.rect.x,
                        l.rect.y,
                        l.rect.width,
                        l.rect.height,
                        format!("{:?}", l.scale),
                    )
                })
                .collect(),
            legend_bindings: bindings
                .iter()
                .map(|b| {
                    (
                        b.legend_path.0.clone(),
                        b.plot_path.0.clone(),
                        b.selection.clone(),
                        b.colour_column.clone(),
                    )
                })
                .collect(),
            plot_render_meta: plots
                .iter()
                .map(|p| (p.path.clone(), p.scheme, p.draw_inline_legend))
                .collect(),
        }
    }
}

/// The first launch-vs-rebuilt chrome divergence a hot reload cannot apply in
/// place, if any — `None` means the edit is plots-only and safe to hot-swap.
/// Pure so the reload gate is headlessly testable.
#[cfg(any(target_os = "macos", test))]
fn chrome_divergence(launch: &ChromeSnapshot, rebuilt: &ChromeSnapshot) -> Option<&'static str> {
    if launch.title != rebuilt.title {
        return Some("dashboard title");
    }
    if launch.legends != rebuilt.legends {
        return Some("legend placement/scale");
    }
    if launch.legend_bindings != rebuilt.legend_bindings {
        return Some("legend selection binding (as:/for:)");
    }
    if launch.plot_render_meta != rebuilt.plot_render_meta {
        return Some("per-plot render metadata (colorScheme/inline legend)");
    }
    None
}

/// Thin wrapper for the hot-reload watcher: runs the full pipeline and returns
/// the renderable [`Dashboard`] plus the rebuilt [`ChromeSnapshot`] the reload
/// gate compares, dropping the live engine state. Dropping the non-`Send`
/// [`Session`] here is what lets the watcher run this off the main thread
/// (a `Dashboard` and a `ChromeSnapshot` are `Send`).
///
/// Returns `Err` (rather than exiting) on any failure, so callers can recover —
/// the hot-reload watcher keeps the last good chart when a mid-edit save is
/// momentarily invalid.
#[cfg(any(target_os = "macos", test))]
fn run_pipeline(spec_path: &str) -> Result<(Dashboard, ChromeSnapshot, Vec<SourceProfile>), String> {
    build_everything(spec_path).map(|(dashboard, live)| {
        let title = brightfield_ui::resolve_title(dashboard.meta_title.as_deref(), spec_path);
        let chrome = ChromeSnapshot::capture(
            title,
            &dashboard.legends,
            &live.legend_bindings,
            &live.plots,
        );
        // Sidebar profiles from the throwaway session BEFORE it drops (card
        // 0017): pure data, so it crosses the watcher's Send return boundary
        // where the non-Send Session cannot. The Session is created, profiled,
        // and dropped entirely on the background executor — never handed out.
        let profiles = live.session.profile_sources();
        (dashboard, chrome, profiles)
    })
}

/// Run the spec-to-scene pipeline, returning a [`Dashboard`] — one independently
/// rendered scene per plot (each with its own axes/scales), positioned per the
/// layout pass — AND the live engine/render state ([`LiveParts`]) the window
/// needs for in-window cross-filtering. A single plot is just a one-plot
/// dashboard.
/// Parse a `BRIGHTFIELD_PARAM_OVERRIDE` value into a `SpecValue` — integer,
/// then float, then string (card 0014 headless preview).
fn parse_override_value(raw: &str) -> brightfield_spec::ast::SpecValue {
    use brightfield_spec::ast::SpecValue;
    if let Ok(i) = raw.parse::<i64>() {
        SpecValue::Integer(i)
    } else if let Ok(f) = raw.parse::<f64>() {
        SpecValue::Float(f)
    } else {
        SpecValue::String(raw.to_string())
    }
}

fn build_everything(spec_path: &str) -> Result<(Dashboard, LiveParts), String> {
    // 1. Parse the spec.
    let parsed = parse_spec_path(spec_path).map_err(|e| format!("parse error: {e}"))?;
    for w in &parsed.warnings {
        eprintln!("parse warning: {w:?}");
    }

    // 2. Analyse. Convert the brush bindings (one per brushable interactor,
    //    each carrying its plot-node contributor identity) before `analysis` is
    //    moved into the engine.
    let analysis = analyse_spec(&parsed.spec).map_err(|e| format!("analysis error: {e}"))?;
    let brush_bindings: Vec<(String, BrushBinding)> = analysis
        .brushable_bindings
        .iter()
        .map(|bb| (bb.parent_plot.0.clone(), BrushBinding::from(bb)))
        .collect();
    // Legend producer bindings (card 0009), kept for the window path — a
    // bound legend's swatch click dispatches through the coordinator.
    let legend_bindings = analysis.legend_bindings.clone();

    // 3. Load into engine (creates DuckDB views).
    let engine = Engine::new();
    let spec_dir = Path::new(spec_path).parent();
    let load = engine
        .load_spec(parsed.spec.clone(), analysis, spec_dir)
        .map_err(|e| format!("engine error: {e}"))?;
    let mut session = load.session;

    // 3b. Optional headless param override (card 0014): apply
    //     BRIGHTFIELD_PARAM_OVERRIDE="name=value[,name=value]" before executing,
    //     so a PNG dump can preview the dashboard at a chosen param value — the
    //     same propagate_param path a slider will drive live once the widget lands.
    if let Ok(overrides) = env::var("BRIGHTFIELD_PARAM_OVERRIDE") {
        for pair in overrides.split(',') {
            if let Some((name, raw)) = pair.split_once('=') {
                let _ = session.propagate_param(name.trim(), parse_override_value(raw.trim()));
            }
        }
    }

    // 3c. Reconcile each slider's param to its declared [min, max] before
    //     executing (card 0005). A spec whose param default lies outside the
    //     slider's own domain (an authoring inconsistency) would otherwise render
    //     one value while the clamped thumb rests at another; clamping the param
    //     to the slider's range keeps the first render and the thumb in agreement.
    //     A no-op for well-formed specs (default already in range).
    for (_, input) in placed_input_nodes(&parsed.spec, Rect::new(0.0, 0.0, 0.0, 0.0)) {
        if input.kind != InputKind::Slider {
            continue;
        }
        if let Some(binding) = SliderBinding::from_input(input) {
            if let Some(v) = session
                .current_params()
                .get(&binding.param_name)
                .and_then(spec_value_as_f64)
            {
                let clamped = v.max(binding.min).min(binding.max);
                if clamped != v {
                    let _ = session.propagate_param(
                        &binding.param_name,
                        brightfield_spec::ast::SpecValue::Float(clamped),
                    );
                }
            }
        }
    }

    // 4. Execute all marks, building per-mark inputs indexed by the flat mark
    //    order (= execution order). A failed mark keeps `batch: None` and is
    //    skipped when rendering (AC-05: graceful failure); its channels/kind are
    //    still recorded so a later cross-filter re-execution can render it.
    //    Batches are concatenated so a >2048-row result isn't truncated.
    let results = session.execute_all();
    let marks = collect_marks(&parsed.spec);
    let mut mark_inputs: Vec<MarkInput> = Vec::with_capacity(marks.len());
    for (i, result) in results.into_iter().enumerate() {
        let mark = marks[i];
        let batch = match result {
            Ok(batches) => concat_result_batches(batches),
            Err(e) => {
                eprintln!("warning: skipping mark {i}: {e}");
                None
            }
        };
        mark_inputs.push(MarkInput {
            batch,
            channels: ChannelMap::from_mark(mark),
            kind: mark.kind,
            // Populated once per mark below, when its owning plot's colorScheme
            // is resolved (a mark belongs to exactly one plot).
            renderer_override: None,
        });
    }

    // 5. Lay the plots out, group each plot's marks, and build one scene per
    //    plot (its own axes/scales) at the position from the layout pass. Keep
    //    each plot's inferred scales (for pixel→data brush inversion) and the
    //    brush bindings it contributes, for the live cross-filter coordinator.
    let placed = placed_plots(&parsed.spec, Rect::new(0.0, 0.0, 0.0, 0.0));
    let groups = collect_plot_groups(&parsed.spec);
    let registry = default_renderers();

    // Plot paths whose inline colour legend an explicit standalone
    // `legend: color for: <name>` node relocates — the plot must NOT also draw its
    // own top-right legend, or the same scale appears twice. A bare `legend:`
    // (no `for:`) stays an addition, so only explicit `for:` targets suppress.
    let legend_suppressed: HashSet<String> = {
        let mut name_to_path: HashMap<String, String> = HashMap::new();
        for (path, node) in collect_plot_nodes(&parsed.spec) {
            if let Some(brightfield_spec::ast::SpecValue::String(name)) =
                node.attributes.get("name")
            {
                name_to_path.insert(name.clone(), path);
            }
        }
        let mut set = HashSet::new();
        for (_rect, node) in placed_legend_nodes(&parsed.spec, Rect::new(0.0, 0.0, 0.0, 0.0)) {
            if node.channel != LegendChannel::Color {
                continue;
            }
            if let LegendFor::Named(name) = legend_for(node) {
                if let Some(path) = name_to_path.get(&name) {
                    set.insert(path.clone());
                }
            }
        }
        set
    };

    // path → plot node, for reading plot-level attributes (colorScheme) during
    // assembly. Kept as a Vec (plot counts are tiny) to hold the paths alive.
    let plot_nodes = collect_plot_nodes(&parsed.spec);

    let mut plots: Vec<PlotRender> = Vec::new();
    let mut live_plots: Vec<LivePlotMeta> = Vec::new();
    for plot in &placed {
        let group = match groups.iter().find(|g| g.plot_path == plot.path) {
            Some(g) => g,
            None => continue,
        };
        let layout = ChartLayout::new(plot.rect.width, plot.rect.height);

        // The plot's colour scheme, applied to its raster marks.
        let scheme = plot_nodes
            .iter()
            .find(|(path, _)| *path == plot.path)
            .map(|(_, node)| raster_scheme(node.attributes.get("colorScheme")))
            .unwrap_or_default();

        // Populate each of this plot's marks' `renderer_override` ONCE, from
        // the plot's colorScheme plus the mark's attributes (`configured_renderer`
        // owns the raster/heatmap/cell scheme + heatmap/contour bandwidth +
        // contour thresholds match). This same override then drives BOTH the
        // first render below and every live cross-filter rebuild (it rides
        // `MarkInput` into the coordinator), so a mark renders identically each
        // time — one construction site, no drift. `thresholds` on contour is the
        // iso-level count (renderer-side; the lowerer registration shields it
        // from the SQL bin count).
        for &mi in &group.mark_indices {
            let Some(kind) = mark_inputs.get(mi).map(|m| m.kind) else {
                continue;
            };
            let bandwidth = marks.get(mi).and_then(|mk| mark_attr_f64(mk, "bandwidth"));
            let thresholds = marks
                .get(mi)
                .and_then(|mk| mark_attr_f64(mk, "thresholds"))
                .filter(|t| *t >= 1.0)
                .map(|t| t as usize);
            if let Some(m) = mark_inputs.get_mut(mi) {
                m.renderer_override = configured_renderer(kind, scheme, bandwidth, thresholds);
            }
        }

        let mut chart_data: Vec<ChartData<'_>> = Vec::new();
        for &mi in &group.mark_indices {
            let Some(m) = mark_inputs.get(mi) else { continue };
            let Some(batch) = m.batch.as_ref() else { continue };
            let renderer: &dyn MarkRenderer = match m.renderer_override.as_deref() {
                Some(r) => r,
                None => match find_renderer(&registry, m.kind) {
                    Some(r) => r,
                    None => {
                        eprintln!("warning: no renderer for mark kind {:?} — skipping", m.kind);
                        continue;
                    }
                },
            };
            chart_data.push(ChartData {
                batch,
                channel_map: &m.channels,
                renderer,
                layout: layout.clone(),
                view_extent: None,
                highlight: None,
            });
        }
        if chart_data.is_empty() {
            continue;
        }
        let refs: Vec<&ChartData<'_>> = chart_data.iter().collect();
        let draw_inline_legend = !legend_suppressed.contains(&plot.path);
        let (scene, scales) = build_multi_mark_scene(&refs, draw_inline_legend);
        drop(refs);
        drop(chart_data);

        // Bindings whose contributor identity is this plot's node path.
        let bindings: Vec<BrushBinding> = brush_bindings
            .iter()
            .filter(|(contributor, _)| *contributor == plot.path)
            .map(|(_, b)| b.clone())
            .collect();

        plots.push(PlotRender {
            path: plot.path.clone(),
            x: plot.rect.x,
            y: plot.rect.y,
            width: plot.rect.width.ceil() as u32,
            height: plot.rect.height.ceil() as u32,
            scene,
        });
        live_plots.push(LivePlotMeta {
            path: plot.path.clone(),
            mark_indices: group.mark_indices.clone(),
            layout,
            bindings,
            scales,
            draw_inline_legend,
            scheme,
        });
    }

    if plots.is_empty() {
        return Err("no marks rendered successfully".to_string());
    }

    // Slider placements (card 0005): each composition-level `input: slider` with
    // a param target + numeric bounds becomes a hosted widget at its layout rect,
    // its thumb resting at the param's current (default) value.
    let sliders: Vec<SliderPlacement> =
        placed_input_nodes(&parsed.spec, Rect::new(0.0, 0.0, 0.0, 0.0))
            .into_iter()
            .filter(|(_, input)| input.kind == InputKind::Slider)
            .filter_map(|(rect, input)| {
                let binding = SliderBinding::from_input(input)?;
                let value = session
                    .current_params()
                    .get(&binding.param_name)
                    .and_then(spec_value_as_f64)
                    .unwrap_or(binding.min)
                    .max(binding.min)
                    .min(binding.max);
                Some(SliderPlacement {
                    rect,
                    binding,
                    value,
                })
            })
            .collect();

    // Standalone legends (multi-view inc 6): resolve each `legend:` node to the
    // colour scale of the plot its `for:` names. Resolved before `live_plots` is
    // moved into `LiveParts` (it borrows the per-plot scales).
    let legends = resolve_legends(&parsed.spec, &live_plots);

    // Reconcile the producer bindings against the live placements (card 0009
    // F4): drop phantom bindings with no hosted legend, and diagnose placed
    // `as:` legends whose clicks would be dead.
    let (legend_bindings, legend_binding_diags) =
        reconcile_legend_bindings(&parsed.spec, &legends, legend_bindings);
    for d in &legend_binding_diags {
        eprintln!("warning: {d}");
    }

    // Fold slider + legend rects into the dashboard size so a widget beside/below
    // the plots reserves its space (the window is the bounding box).
    let width = placed
        .iter()
        .map(|p| p.rect.x + p.rect.width)
        .chain(sliders.iter().map(|s| s.rect.x + s.rect.width))
        .chain(legends.iter().map(|l| l.rect.x + l.rect.width))
        .fold(0.0_f64, f64::max)
        .ceil() as u32;
    let height = placed
        .iter()
        .map(|p| p.rect.y + p.rect.height)
        .chain(sliders.iter().map(|s| s.rect.y + s.rect.height))
        .chain(legends.iter().map(|l| l.rect.y + l.rect.height))
        .fold(0.0_f64, f64::max)
        .ceil() as u32;
    Ok((
        Dashboard {
            width,
            height,
            plots,
            sliders,
            legends,
            meta_title: parsed.spec.meta.as_ref().and_then(|m| m.title.clone()),
        },
        LiveParts {
            session,
            marks: mark_inputs,
            plots: live_plots,
            legend_bindings,
        },
    ))
}

/// Last-modified time of the spec file, for change detection. `None` if the
/// file is momentarily unreadable (e.g. mid-save), which the watcher treats as
/// a change and retries.
#[cfg(target_os = "macos")]
fn file_mtime(path: &str) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Spawn a background task that polls the spec file and swaps in a freshly
/// rendered scene when it changes — turning the edit→see loop interactive
/// without a restart. A save that fails to parse/execute keeps the last good
/// chart (the warning is printed) rather than killing the window.
/// A plot the watcher tracks: its stable component path, fixed geometry, and
/// reactive state. Hot-reload matches new plots to these by `path`.
#[cfg(target_os = "macos")]
struct WatchedPlot {
    path: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    state: gpui::Entity<brightfield_ui::ChartState>,
}

#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
fn spawn_spec_watcher(
    cx: &mut gpui::App,
    watched: Vec<WatchedPlot>,
    spec_path: String,
    launch_chrome: ChromeSnapshot,
    workspace_window: gpui::WindowHandle<gpui_component::Root>,
    editor: Option<gpui::Entity<shell::EditorPanel>>,
    sidebar: Option<gpui::Entity<shell::SidebarPanel>>,
    feedback_log: gpui::Entity<log_model::FeedbackLog>,
) {
    const POLL: std::time::Duration = std::time::Duration::from_millis(300);
    use reload_feedback::{reload_notification, ReloadOutcome};

    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        let mut last = file_mtime(&spec_path);
        loop {
            cx.background_executor().timer(POLL).await;
            let now = file_mtime(&spec_path);
            if now == last {
                continue;
            }
            last = now;

            // A PRISTINE editor buffer follows the file it mirrors: adopt
            // the changed contents before the reload runs (the decision is
            // spec_save::should_reseed — a dirty buffer is left alone and
            // our own save's echo is a no-op). A tap only: no reload branch
            // below is entered, skipped, or reordered by this.
            if let Some(editor) = editor.as_ref() {
                if let Ok(contents) = std::fs::read_to_string(&spec_path) {
                    let editor = editor.clone();
                    let _ = workspace_window.update(cx, |_root, window, cx| {
                        editor.update(cx, |editor, cx| {
                            editor.reseed_from_disk(&contents, window, cx);
                        });
                    });
                }
            }

            // Re-run the (blocking) pipeline off the main thread (Dashboard is
            // Send). catch_unwind contains a panicking pipeline (a degenerate
            // mid-edit spec) so a bad save keeps the last good chart rather than
            // crashing the window — the same guarantee the Err paths give.
            let path = spec_path.clone();
            let built = cx
                .background_executor()
                .spawn(async move {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_pipeline(&path)))
                        .unwrap_or_else(|_| Err("pipeline panicked".to_string()))
                })
                .await;

            match built {
                Ok((dashboard, rebuilt_chrome, rebuilt_profiles)) => {
                    // Only a data/visual change is hot-swappable: the window's
                    // plot layout is fixed at launch, so require exactly the same
                    // plots (by stable path) at the same geometry. Any structural
                    // change (count, which plots render, or any size/position)
                    // can't be absorbed in place — restart to apply.
                    let same_layout = dashboard.plots.len() == watched.len()
                        && dashboard.plots.iter().all(|p| {
                            watched.iter().any(|w| {
                                w.path == p.path
                                    && w.x == p.x
                                    && w.y == p.y
                                    && w.width == f64::from(p.width)
                                    && w.height == f64::from(p.height)
                            })
                        });
                    if !same_layout {
                        eprintln!("reload skipped: dashboard layout changed; restart to apply");
                        if let Some((severity, message)) =
                            reload_notification(&ReloadOutcome::LayoutChanged)
                        {
                            shell::notify_reload_rejection(
                                &workspace_window,
                                cx,
                                severity,
                                message,
                                &feedback_log,
                            );
                        }
                        continue;
                    }
                    // The chrome (title, hosted legends) and the coordinator's
                    // per-plot render metadata are equally launch-fixed: the
                    // swap below only replaces plot scenes, so an edit to any
                    // of them would otherwise reload with stale chrome — or
                    // revert on the next gesture-driven re-render (a changed
                    // colorScheme would snap back to the launch scheme).
                    if let Some(what) = chrome_divergence(&launch_chrome, &rebuilt_chrome) {
                        eprintln!("reload skipped: {what} changed; restart to apply");
                        if let Some((severity, message)) =
                            reload_notification(&ReloadOutcome::ChromeDiverged(what))
                        {
                            shell::notify_reload_rejection(
                                &workspace_window,
                                cx,
                                severity,
                                message,
                                &feedback_log,
                            );
                        }
                        continue;
                    }
                    // Swap each plot's new scene into its state (matched by path),
                    // then repaint once.
                    let mut scenes: std::collections::HashMap<String, vello::Scene> =
                        dashboard.plots.into_iter().map(|p| (p.path, p.scene)).collect();
                    cx.update(|app| {
                        for w in &watched {
                            if let Some(scene) = scenes.remove(&w.path) {
                                w.state.update(app, |s, c| {
                                    s.set_scene(scene);
                                    c.notify();
                                });
                            }
                        }
                        app.refresh_windows();
                    });
                    // Refresh the Data sidebar with the profiles computed on
                    // the throwaway session (sbp_ac04): the frozen-at-launch
                    // gap closes — a spec edit adding/removing a source is now
                    // reflected. Per-source profiling failures already folded
                    // into a Failed variant (the sidebar shows a muted row);
                    // surface each in the Log dock at Warning, no toast.
                    cx.update(|app| {
                        if let Some(sidebar) = sidebar.as_ref() {
                            sidebar.update(app, |panel, cx| {
                                panel.set_profiles(rebuilt_profiles.clone(), cx);
                            });
                        }
                        for profile in &rebuilt_profiles {
                            if let brightfield_engine::ProfileOutcome::Failed(reason) =
                                &profile.outcome
                            {
                                feedback_log.update(app, |log, _| {
                                    log.append(
                                        reload_feedback::Severity::Warning,
                                        profile_model::profile_warning(&profile.name, reason),
                                    );
                                });
                            }
                        }
                    });
                    eprintln!("reloaded {spec_path}");
                    // The routing decision is total: Applied maps to NO
                    // notification — successful reloads stay quiet
                    // (aws_ac05), through the same fn the rejections use.
                    if let Some((severity, message)) =
                        reload_notification(&ReloadOutcome::Applied)
                    {
                        shell::notify_reload_rejection(
                            &workspace_window,
                            cx,
                            severity,
                            message,
                            &feedback_log,
                        );
                    }
                    // Recovery is self-cleaning: a successful reload clears
                    // the sticky error a prior rejection left up.
                    if reload_feedback::clears_errors(&ReloadOutcome::Applied) {
                        shell::clear_reload_error(&workspace_window, cx);
                    }
                }
                Err(e) => {
                    eprintln!("reload skipped (keeping last good chart): {e}");
                    if let Some((severity, message)) =
                        reload_notification(&ReloadOutcome::PipelineFailed(&e))
                    {
                        shell::notify_reload_rejection(
                            &workspace_window,
                            cx,
                            severity,
                            message,
                            &feedback_log,
                        );
                    }
                }
            }
        }
    })
    .detach();
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: brightfield <spec.yaml>");
        process::exit(1);
    }
    let spec_path = &args[1];

    let (dashboard, live) = match build_everything(spec_path) {
        Ok(parts) => parts,
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    };

    eprintln!(
        "Pipeline complete: {}x{} dashboard, {} plot(s)",
        dashboard.width,
        dashboard.height,
        dashboard.plots.len()
    );

    // Debug path: composite the per-plot scenes and dump a PNG instead of
    // opening a window. Triggered by `BRIGHTFIELD_DUMP_PNG=<path>`. The
    // decision is the `boot` module's seam (aws_ac01): this arm RETURNS
    // before the workspace shell (DockArea/panels/editor) is reachable, so
    // shell state can never move a pixel in a dumped PNG.
    if let boot::BootMode::HeadlessDump(dump_path) =
        boot::boot_mode(env::var("BRIGHTFIELD_DUMP_PNG").ok())
    {
        // Optional supersampling for HiDPI verification: BRIGHTFIELD_DUMP_SCALE=2
        // renders at device resolution via the same scale-the-scene path the
        // window uses for crisp Retina output.
        let scale: f32 = env::var("BRIGHTFIELD_DUMP_SCALE")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|s: &f32| *s > 0.0)
            .unwrap_or(1.0);
        let placements: Vec<(f64, f64, &vello::Scene)> =
            dashboard.plots.iter().map(|p| (p.x, p.y, &p.scene)).collect();
        let mut composite = compose_dashboard(
            f64::from(dashboard.width),
            f64::from(dashboard.height),
            &placements,
        );
        // Draw the resting slider widgets into the composite so the PNG previews
        // them (card 0005). The thumb sits at the param's current value.
        for s in &dashboard.sliders {
            let span = s.binding.max - s.binding.min;
            let frac = if span > 0.0 {
                (s.value - s.binding.min) / span
            } else {
                0.0
            };
            brightfield_render::scene::render_slider(
                &mut composite,
                s.rect.x,
                s.rect.y,
                s.rect.width,
                s.rect.height,
                frac,
            );
        }
        // Draw the standalone legends into the composite at their layout rects
        // (multi-view inc 6). Each shows its resolved plot's colour scale.
        for l in &dashboard.legends {
            render_colour_legend_at(&mut composite, l.rect.x, l.rect.y, &l.scale);
        }

        let dev_w = ((dashboard.width as f32) * scale).round() as u32;
        let dev_h = ((dashboard.height as f32) * scale).round() as u32;
        let mut scaled = vello::Scene::new();
        scaled.append(&composite, Some(vello::kurbo::Affine::scale(f64::from(scale))));

        let renderer = brightfield_ui::VelloRenderer::new();
        let pixels = renderer
            .lock()
            .expect("renderer mutex poisoned")
            .render_to_pixels(&scaled, dev_w, dev_h);
        let img = image::RgbaImage::from_raw(dev_w, dev_h, pixels)
            .expect("pixel buffer size mismatch");
        img.save(&dump_path).expect("failed to write PNG");
        let non_zero = img.as_raw().iter().filter(|&&b| b != 0).count();
        let total = img.as_raw().len();
        eprintln!(
            "PNG dumped: {dump_path} ({dev_w}x{dev_h}, {non_zero}/{total} non-zero bytes, {:.1}% coverage)",
            100.0 * non_zero as f64 / total as f64
        );
        return;
    }

    // Open a native GPUI window: the docked authoring workspace (card 0017) —
    // a DockArea hosting the canvas panel (one ChartElement per plot,
    // positioned per the layout, each with its own ChartState so interaction
    // is per-plot), the YAML spec editor, and the data sidebar.
    #[cfg(target_os = "macos")]
    {
        use gpui::AppContext;
        use gpui::Focusable as _;
        use std::rc::Rc;

        let renderer = brightfield_ui::VelloRenderer::new();
        // The entrypoint keeps gpui_macos (the 0017 locked pick — migrating
        // to gpui_platform is an escalation, never a silent swap) and gains
        // gpui-component's bundled icon assets (aws_ac01).
        let app = gpui::Application::with_platform(Rc::new(gpui_macos::MacPlatform::new(false)))
            .with_assets(gpui_component_assets::Assets);
        let spec_path = spec_path.to_string();
        let Dashboard { width, height, plots, sliders, legends, meta_title } = dashboard;
        // The dashboard's display title — the ONE resolver call feeding both
        // the native titlebar and the canvas panel's tab title below.
        let title = brightfield_ui::resolve_title(meta_title.as_deref(), &spec_path);
        let LiveParts {
            session,
            marks,
            plots: live_plots_meta,
            legend_bindings,
        } = live;
        // Workspace shell inputs (card 0017), computed before `marks` moves
        // into the coordinator: the editor buffer seed (the spec file's
        // text) and the sidebar derivation (spec AST + the column names of
        // the batches the pipeline ALREADY executed — no new DuckDB
        // queries, aws_ac06). A failed seed read is passed through as None
        // — NOT an empty string, which the editor could later "save" over
        // the real file (the empty-seed truncation guard): an unseeded
        // editor refuses cmd-s until a pristine reseed lands.
        let editor_seed: Option<String> = match std::fs::read_to_string(&spec_path) {
            Ok(text) => Some(text),
            Err(e) => {
                eprintln!(
                    "spec editor: failed to read {spec_path} for the editor seed ({e}); \
                     the editor opens empty and will refuse to save until the file is readable"
                );
                None
            }
        };
        // Real per-source column profiles for the Data sidebar (card 0017),
        // the upgrade over the launch-frozen column-name approximation:
        // computed synchronously on the launch session BEFORE the window opens
        // (launch already runs every mark query; this adds one scan per
        // source). Read-only and non-&mut, so it runs here while `session` is
        // still owned — before it moves into the coordinator, whose live
        // session must never be borrowed for profiling.
        let sidebar_profiles: Vec<SourceProfile> = session.profile_sources();
        // Launch-time chrome snapshot for the hot-reload gate: the title,
        // hosted legends, legend selection bindings (click wiring), and
        // per-plot render metadata are fixed at launch, so the watcher
        // refuses to hot-swap when a rebuild diverges on any of them
        // ("restart to apply" — same contract as a layout change).
        let launch_chrome =
            ChromeSnapshot::capture(title.clone(), &legends, &legend_bindings, &live_plots_meta);
        app.run(move |cx| {
            // gpui-component globals — theme, dock/input/root registries —
            // before any of its views exist (aws_ac01).
            gpui_component::init(cx);

            // The shared feedback log (card 0017, wsc_ac02): the editor's
            // save outcomes and the watcher's reload rejections append to
            // it; the bottom-dock Log panel renders it. History — reload
            // recovery clears the sticky error toast, never this.
            let feedback_log = cx.new(|_| log_model::FeedbackLog::default());

            // A source whose launch profiling failed surfaces in the Log dock
            // at Warning (no toast — the sidebar already shows a muted row for
            // it); the same routing the watcher uses on reload (sbp_ac04).
            for profile in &sidebar_profiles {
                if let brightfield_engine::ProfileOutcome::Failed(reason) = &profile.outcome {
                    feedback_log.update(cx, |log, _| {
                        log.append(
                            reload_feedback::Severity::Warning,
                            profile_model::profile_warning(&profile.name, reason),
                        );
                    });
                }
            }

            // One ChartState per plot; the watcher tracks each by its stable
            // path + geometry for hot-reload.
            let mut watched: Vec<WatchedPlot> = Vec::with_capacity(plots.len());
            for p in plots {
                let (x, y, w, h) = (p.x, p.y, f64::from(p.width), f64::from(p.height));
                let state = cx.new(|_| {
                    brightfield_ui::ChartState::new(p.scene, p.width, p.height, renderer.clone())
                });
                watched.push(WatchedPlot { path: p.path, x, y, width: w, height: h, state });
            }

            // Build the live cross-filter coordinator, joining each plot's
            // metadata to its state entity (same order as the dashboard plots).
            // `None` when nothing brushes — the brush then stays purely visual.
            let live_plots: Vec<LivePlot> = live_plots_meta
                .into_iter()
                .zip(watched.iter())
                .map(|(meta, w)| LivePlot {
                    path: meta.path,
                    mark_indices: meta.mark_indices,
                    layout: meta.layout,
                    bindings: meta.bindings,
                    // Displayed and launch scales start equal (the launch
                    // inference); a rebuild folds a fresh inference against
                    // launch_scales and updates `scales` (widen-only).
                    scales: meta.scales.clone(),
                    launch_scales: meta.scales,
                    draw_inline_legend: meta.draw_inline_legend,
                    state: w.state.clone(),
                })
                .collect();
            // Coordinator slider bindings, in the same order as the hosted slider
            // widgets below (both derived from `sliders`), so a widget's index
            // matches its binding.
            let slider_bindings: Vec<SliderBinding> =
                sliders.iter().map(|s| s.binding.clone()).collect();
            // Coordinator legend bindings (card 0009), in analysis order — the
            // same slice `placed_legend_views` positions against, so a hosted
            // legend's index matches its binding.
            let legend_select_bindings: Vec<brightfield_ui::LegendSelectBinding> =
                legend_bindings.iter().map(Into::into).collect();
            let coordinator = CrossfilterCoordinator::new(
                session,
                marks,
                live_plots,
                slider_bindings,
                legend_select_bindings,
            );

            // One placed chart per plot, each wired to the shared coordinator.
            let charts: Vec<brightfield_ui::PlacedChart> = watched
                .iter()
                .map(|w| brightfield_ui::PlacedChart {
                    x: w.x,
                    y: w.y,
                    width: w.width,
                    height: w.height,
                    state: w.state.clone(),
                    coordinator: coordinator.clone(),
                })
                .collect();

            // One placed slider widget per input:slider, wired to the same
            // coordinator; index = position (matching the slider_bindings order).
            let placed_sliders: Vec<brightfield_ui::PlacedSlider> = sliders
                .iter()
                .map(|s| brightfield_ui::PlacedSlider {
                    x: s.rect.x,
                    y: s.rect.y,
                    width: s.rect.width,
                    height: s.rect.height,
                    binding: s.binding.clone(),
                    state: cx.new(|_| brightfield_ui::SliderWidget::new(s.value)),
                    coordinator: coordinator.clone(),
                })
                .collect();

            // One hosted legend descriptor per resolved placement, at its
            // layout rect beside the plots (card 0016) — a bound categorical
            // legend additionally carries the coordinator + its binding index,
            // arming click-to-filter (card 0009); the rest stay display-only.
            let hosted_legends =
                placed_legend_views(&legends, &legend_bindings, coordinator.as_ref());

            // The workspace key bindings, declared as data: bare `p` toggles
            // presentation mode inside the workspace key context (card 0016 —
            // Brightfield's first GPUI action; the binding is unchanged, its
            // handler now lives on the canvas panel), plus cmd-s → SaveSpec
            // scoped to the editor context (card 0017).
            cx.bind_keys(brightfield_ui::workspace_key_bindings());
            cx.bind_keys(shell::editor_key_bindings());

            // Size the initial window to the dashboard plus the 0016 chrome
            // margins and the default authoring dock widths (card 0017).
            // `window_bounds` is the CONTENT rect — the macOS titlebar is
            // added above it. Initial size ONLY: DockArea owns layout from
            // here (the 0016 toggle-resize invariant is superseded; recorded
            // in the 0017 tabletop).
            let (win_w, win_h) =
                shell_model::initial_window_size(f64::from(width), f64::from(height));
            // Clamp to the primary display's visible bounds (menu bar/dock
            // excluded): a dashboard plus both dock widths can exceed a
            // laptop display, and centring an oversized content rect would
            // push the titlebar off-screen. No display info → unclamped.
            let display_size = cx
                .primary_display()
                .map(|d| d.visible_bounds().size)
                .map(|s| (f64::from(s.width), f64::from(s.height)));
            let (win_w, win_h) = shell_model::clamp_to_display((win_w, win_h), display_size);
            let window_size = gpui::size(gpui::px(win_w as f32), gpui::px(win_h as f32));
            let window_opts = gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::centered(
                    None,
                    window_size,
                    cx,
                ))),
                titlebar: Some(gpui::TitlebarOptions {
                    // The resolved dashboard title (document-app convention);
                    // the canvas panel's title shows the same string.
                    title: Some(title.clone().into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            // The canvas panel entity, captured out of the window closure so
            // boot focus lands on it (bare `p` from the first keypress) —
            // and the editor panel, captured for the watcher's pristine
            // reseed tap below.
            let canvas_slot: Rc<std::cell::RefCell<Option<gpui::Entity<shell::CanvasPanel>>>> =
                Rc::new(std::cell::RefCell::new(None));
            let canvas_capture = canvas_slot.clone();
            let editor_slot: Rc<std::cell::RefCell<Option<gpui::Entity<shell::EditorPanel>>>> =
                Rc::new(std::cell::RefCell::new(None));
            let editor_capture = editor_slot.clone();
            // The sidebar panel, captured out of the window closure for the
            // watcher's profile-refresh tap (sbp_ac04) — like the editor
            // reseed tap.
            let sidebar_slot: Rc<std::cell::RefCell<Option<gpui::Entity<shell::SidebarPanel>>>> =
                Rc::new(std::cell::RefCell::new(None));
            let sidebar_capture = sidebar_slot.clone();
            let spec_path_for_editor = spec_path.clone();
            let feedback_log_for_editor = feedback_log.clone();
            let window = cx
                .open_window(window_opts, move |window, cx| {
                    let chart_view = cx.new(|_| {
                        brightfield_ui::ChartView::new(
                            f64::from(width),
                            f64::from(height),
                            charts,
                            placed_sliders,
                            hosted_legends,
                        )
                    });
                    // The docked workspace shell (card 0017): shared
                    // presentation state, the three panels, the DockArea
                    // root, all wrapped in gpui-component's Root (the
                    // notification/dialog layers live there).
                    let presentation = cx.new(|_| shell::PresentationState {
                        mode: brightfield_ui::PresentationMode::default(),
                    });
                    let canvas = cx.new(|cx| {
                        shell::CanvasPanel::new(chart_view, title, presentation.clone(), cx)
                    });
                    *canvas_capture.borrow_mut() = Some(canvas.clone());
                    let editor = cx.new(|cx| {
                        shell::EditorPanel::new(
                            std::path::PathBuf::from(&spec_path_for_editor),
                            editor_seed.as_deref(),
                            presentation.clone(),
                            feedback_log_for_editor.clone(),
                            window,
                            cx,
                        )
                    });
                    *editor_capture.borrow_mut() = Some(editor.clone());
                    let sidebar = cx.new(|cx| {
                        shell::SidebarPanel::new(sidebar_profiles, presentation.clone(), cx)
                    });
                    *sidebar_capture.borrow_mut() = Some(sidebar.clone());
                    // The bottom-dock Log panel over the shared feedback
                    // log (wsc_ac02).
                    let log = cx.new(|cx| {
                        shell::LogPanel::new(
                            feedback_log_for_editor.clone(),
                            presentation.clone(),
                            cx,
                        )
                    });
                    let workspace = cx.new(|cx| {
                        shell::WorkspaceRoot::new(
                            canvas,
                            editor,
                            sidebar,
                            log,
                            presentation,
                            window,
                            cx,
                        )
                    });
                    cx.new(|cx| gpui_component::Root::new(workspace, window, cx))
                })
                .expect("failed to open window");

            // Focus the canvas panel so the canvas-scoped `p` binding
            // receives key dispatch from the first keypress.
            window
                .update(cx, |_root, window, cx| {
                    if let Some(canvas) = canvas_slot.borrow().as_ref() {
                        window.focus(&canvas.focus_handle(cx), cx);
                    }
                })
                .expect("focus canvas panel");

            // Hot-reload: swap each plot's scene when the spec changes on
            // disk; rejections additionally surface as workspace
            // notifications (aws_ac05's tap — same outcomes, same stderr),
            // and a PRISTINE editor buffer reseeds from the changed file
            // (the watcher's second sanctioned tap — reload control flow
            // untouched).
            let editor = editor_slot.borrow().clone();
            let sidebar = sidebar_slot.borrow().clone();
            spawn_spec_watcher(
                cx,
                watched,
                spec_path,
                launch_chrome,
                window,
                editor,
                sidebar,
                feedback_log,
            );
        });
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (dashboard, live);
        eprintln!(
            "GPUI window display is currently macOS-only. \
             Re-run with BRIGHTFIELD_DUMP_PNG=out.png to render the chart to an image."
        );
    }
}

#[cfg(test)]
mod tests {
    use brightfield_engine::Engine;
    use brightfield_render::channel::{Channel, ChannelMap};
    use brightfield_render::layout::ChartLayout;
    use brightfield_render::mark::{DotRenderer, MarkRenderer};
    use brightfield_render::scene::{build_multi_mark_scene, ChartData};
    use brightfield_spec::analysis::analyse_spec;
    use brightfield_spec::{parse_spec, Format};

    #[test]
    fn concat_result_batches_combines_chunks_not_truncates() {
        use arrow::array::Int32Array;
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;
        use std::sync::Arc;

        let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Int32, false)]));
        let chunk1 =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(Int32Array::from(vec![1, 2]))])
                .unwrap();
        let chunk2 =
            RecordBatch::try_new(schema, vec![Arc::new(Int32Array::from(vec![3, 4, 5]))]).unwrap();

        // Empty result -> None (nothing to render).
        assert!(super::concat_result_batches(vec![]).is_none());

        // Single chunk -> passed through unchanged.
        let single = super::concat_result_batches(vec![chunk1.clone()]).unwrap();
        assert_eq!(single.num_rows(), 2);

        // Multiple chunks -> concatenated (2 + 3 = 5), NOT truncated to the first.
        let combined = super::concat_result_batches(vec![chunk1, chunk2]).unwrap();
        assert_eq!(combined.num_rows(), 5, "all chunks must be retained, not just the first");
    }

    #[test]
    fn colour_scale_of_prefers_a_colour_channel_over_a_present_non_colour_fill() {
        use brightfield_render::scale::{Scale, ScaleSet};
        // A numeric fill (Linear) must NOT mask a categorical stroke Colour scale:
        // filter-per-channel-before-or_else, not or_else-then-filter.
        let colour = Scale::Colour {
            categories: vec!["a".to_string(), "b".to_string()],
            palette: vec![[0.3, 0.4, 0.6, 1.0], [0.9, 0.5, 0.1, 1.0]],
        };
        let linear = Scale::Linear {
            domain_min: 0.0,
            domain_max: 1.0,
            range_start: 0.0,
            range_end: 1.0,
        };
        let mut scales = ScaleSet::new();
        scales.insert(Channel::Fill, linear);
        scales.insert(Channel::Stroke, colour);
        assert!(
            matches!(super::colour_scale_of(&scales), Some(Scale::Colour { .. })),
            "a categorical stroke colour scale must be found even when fill is a non-colour scale"
        );

        // Fill takes precedence when both are colour.
        let mut both = ScaleSet::new();
        both.insert(
            Channel::Fill,
            Scale::Colour { categories: vec!["f".into()], palette: vec![[0.1, 0.1, 0.1, 1.0]] },
        );
        both.insert(
            Channel::Stroke,
            Scale::Colour { categories: vec!["s".into()], palette: vec![[0.2, 0.2, 0.2, 1.0]] },
        );
        match super::colour_scale_of(&both) {
            Some(Scale::Colour { categories, .. }) => assert_eq!(categories, vec!["f".to_string()]),
            other => panic!("expected the fill colour scale, got {other:?}"),
        }
    }

    // scs_ac07: a raster plot's Fill Sequential resolves for a standalone legend,
    // sized as a gradient bar.
    #[test]
    fn scs_ac07_sequential_resolves_for_standalone_legend() {
        use brightfield_render::legend::sequential_legend_size;
        use brightfield_render::scale::{Scale, ScaleSet, SequentialScheme};
        use brightfield_render::ChartLayout;
        use brightfield_spec::ast::SpecValue;
        use brightfield_spec::{layout::collect_plot_nodes, parse_spec, Format};

        const SRC: &str = r#"
data:
  points:
    - { x: 1, y: 1 }
    - { x: 2, y: 2 }
hconcat:
  - plot:
    - mark: raster
      data: { from: points }
      x: x
      y: y
    name: heat
  - legend: color
    for: heat
"#;
        let spec = parse_spec(SRC, Format::Yaml).expect("parse").spec;
        let (path, _) = collect_plot_nodes(&spec)
            .into_iter()
            .find(|(_, n)| n.attributes.get("name") == Some(&SpecValue::String("heat".into())))
            .expect("named raster plot");

        // A raster plot's Fill scale is the count → colour ramp.
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::Fill,
            Scale::Sequential {
                domain_min: 0.0,
                domain_max: 10.0,
                stops: SequentialScheme::Viridis.stops(),
            },
        );
        assert!(
            matches!(super::colour_scale_of(&scales), Some(Scale::Sequential { .. })),
            "colour_scale_of surfaces the Fill Sequential"
        );

        let meta = super::LivePlotMeta {
            path,
            mark_indices: vec![],
            layout: ChartLayout::new(300.0, 200.0),
            bindings: vec![],
            scales,
            draw_inline_legend: true,
            scheme: SequentialScheme::default(),
        };
        let placements = super::resolve_legends(&spec, std::slice::from_ref(&meta));
        assert_eq!(placements.len(), 1, "one standalone legend resolves");
        let placement = &placements[0];
        let expected_size = match &placement.scale {
            Scale::Sequential { .. } => sequential_legend_size(&placement.scale).unwrap(),
            other => panic!("expected a Sequential legend scale, got {other:?}"),
        };
        assert!(
            (placement.rect.width - expected_size.0).abs() < 1e-9,
            "placement sized via the gradient-bar size"
        );
    }

    // scs_ac08: the assembly resolves a raster plot's colorScheme to a scheme,
    // and a RasterRenderer built with it produces the matching Fill ramp.
    #[test]
    fn scs_ac08_colorscheme_selects_the_ramp() {
        use arrow::array::Float64Array;
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;
        use brightfield_render::mark::{MarkRenderer, RasterRenderer};
        use brightfield_render::scale::{Scale, ScaleSet, SequentialScheme};
        use brightfield_spec::ast::SpecValue;
        use std::sync::Arc;

        // colorScheme resolution: known name → scheme; unknown / absent → viridis.
        let blues = SpecValue::String("blues".into());
        assert_eq!(super::raster_scheme(Some(&blues)), SequentialScheme::Blues);
        let bad = SpecValue::String("notascheme".into());
        assert_eq!(
            super::raster_scheme(Some(&bad)),
            SequentialScheme::Viridis,
            "unknown scheme falls back to viridis (warning path)"
        );
        assert_eq!(super::raster_scheme(None), SequentialScheme::Viridis);

        // A RasterRenderer built from `blues` produces a Fill Sequential whose
        // stops are the blues ramp.
        let scheme = super::raster_scheme(Some(&blues));
        let schema = Arc::new(Schema::new(vec![
            Field::new("x_bin", DataType::Float64, false),
            Field::new("y_bin", DataType::Float64, false),
            Field::new("__bf_count", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 1.0])),
                Arc::new(Float64Array::from(vec![0.0, 1.0])),
                Arc::new(Float64Array::from(vec![2.0, 9.0])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x_bin".to_string());
        cm.insert(Channel::Y, "y_bin".to_string());
        let mut scales = ScaleSet::new();
        RasterRenderer { scheme }.augment_scales(&mut scales, &batch, &cm, (0.0, 100.0), (100.0, 0.0));
        match scales.get(Channel::Fill) {
            Some(Scale::Sequential { stops, domain_max, .. }) => {
                assert_eq!(stops, &SequentialScheme::Blues.stops(), "blues ramp stops");
                assert!((domain_max - 9.0).abs() < f64::EPSILON, "domain_max == max count");
            }
            other => panic!("expected a blues Fill Sequential, got {other:?}"),
        }
    }

    // scs_ac08 (review strengthening): drive the REAL consumption seam end-to-end
    // — build_everything reads the plot's colorScheme, builds the per-mark
    // RasterRenderer override, and threads it through augment_scales — so a key
    // typo or raster_boxes index drift can't silently render everything viridis.
    #[test]
    fn scs_ac08_colorscheme_consumed_end_to_end() {
        use brightfield_render::scale::{Scale, SequentialScheme};

        const SRC: &str = r#"
data:
  points:
    - { x: 1, y: 1 }
    - { x: 2, y: 2 }
    - { x: 3, y: 3 }
plot:
  - mark: raster
    data: { from: points }
    x: x
    y: y
colorScheme: blues
"#;
        // Write the spec to a temp file — build_everything takes a path.
        let dir = std::env::temp_dir().join(format!("bf-scs-ac08-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("raster-blues.yaml");
        std::fs::write(&path, SRC).unwrap();

        let (_dashboard, live) =
            super::build_everything(path.to_str().unwrap()).expect("pipeline runs");
        // The single raster plot's post-assembly Fill scale is the blues ramp —
        // proving colorScheme flowed all the way through the assembly seam.
        let fill = live.plots[0]
            .scales
            .get(Channel::Fill)
            .expect("raster plot has a Fill scale");
        match fill {
            Scale::Sequential { stops, .. } => {
                assert_eq!(
                    stops,
                    &SequentialScheme::Blues.stops(),
                    "colorScheme: blues must reach the rendered Fill ramp (not the viridis default)"
                );
            }
            other => panic!("expected a blues Fill Sequential, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    // fww_ac05 (card 0016): the view-model mapping hosts one display-only
    // legend child per resolved placement, at the placement's rect — asserted
    // over the descriptor list (no GPUI tree), through the same
    // `placed_legend_views` the live window path calls.
    #[test]
    fn fww_ac05_one_legend_child_per_placement_at_its_rect() {
        use brightfield_render::scale::Scale;
        use brightfield_spec::layout::Rect;

        let placements = vec![
            super::LegendPlacement {
                path: "root/hconcat[1]".into(),
                rect: Rect::new(400.0, 20.0, 90.0, 66.0),
                scale: Scale::Colour {
                    categories: vec!["a".into(), "b".into()],
                    palette: vec![[0.3, 0.4, 0.6, 1.0], [0.9, 0.5, 0.1, 1.0]],
                },
            },
            super::LegendPlacement {
                path: "root/hconcat[2]".into(),
                rect: Rect::new(400.0, 120.0, 60.0, 108.0),
                scale: Scale::Sequential {
                    domain_min: 0.0,
                    domain_max: 9.0,
                    stops: brightfield_render::scale::SequentialScheme::Viridis.stops(),
                },
            },
        ];

        let views = super::placed_legend_views(&placements, &[], None);
        assert_eq!(views.len(), placements.len(), "one child per placement");
        for (view, placement) in views.iter().zip(&placements) {
            assert_eq!(view.x, placement.rect.x);
            assert_eq!(view.y, placement.rect.y);
            assert_eq!(view.width, placement.rect.width);
            assert_eq!(view.height, placement.rect.height);
            assert!(view.binding.is_none(), "no bindings, no coordinator: display-only");
        }
        assert!(
            matches!(views[0].scale, Scale::Colour { .. })
                && matches!(views[1].scale, Scale::Sequential { .. }),
            "each child carries its placement's scale"
        );
    }

    // lcf_ac05 (card 0009): the view-model mapping arms click-to-filter ONLY
    // for a bound categorical legend — it carries the coordinator + its
    // binding index (positioned by legend path against the analysis binding
    // list) — while Sequential and unbound placements stay display-only.
    #[test]
    fn lcf_ac05_only_bound_colour_legends_carry_coordinator_and_index() {
        use brightfield_render::scale::Scale;
        use brightfield_spec::analysis::{ComponentPath, LegendBinding};
        use brightfield_spec::layout::Rect;
        use brightfield_ui::{CrossfilterCoordinator, LegendSelectBinding, MarkInput};
        use brightfield_engine::Engine;
        use brightfield_spec::analysis::analyse_spec;

        // A real (legend-only) coordinator over a minimal live session.
        let yaml = r#"
params:
  sel: { select: crossfilter }
data:
  t:
    - { x: 1, y: 2, g: a }
plot:
  - mark: dot
    data: { from: t, filterBy: $sel }
    x: x
    y: y
    fill: g
"#;
        let parsed = parse_spec(yaml, Format::Yaml).expect("parse");
        let analysis = analyse_spec(&parsed.spec).expect("analyse");
        let session = Engine::new()
            .load_spec(parsed.spec, analysis, None)
            .expect("load")
            .session;
        let ui_binding = LegendSelectBinding {
            selection_name: "sel".into(),
            contributor: ComponentPath("root".into()),
            column: "g".into(),
        };
        let coordinator =
            CrossfilterCoordinator::new(session, Vec::<MarkInput>::new(), vec![], vec![], vec![ui_binding])
                .expect("legend-only liveness (lcf_ac03) keeps the coordinator");

        let colour = Scale::Colour {
            categories: vec!["a".into()],
            palette: vec![[0.3, 0.4, 0.6, 1.0]],
        };
        let placements = vec![
            // Bound categorical legend (path matches the binding below).
            super::LegendPlacement {
                path: "root/hconcat[1]".into(),
                rect: Rect::new(400.0, 20.0, 90.0, 30.0),
                scale: colour.clone(),
            },
            // Sequential legend — never clickable, even if a path matched.
            super::LegendPlacement {
                path: "root/hconcat[2]".into(),
                rect: Rect::new(400.0, 80.0, 60.0, 108.0),
                scale: Scale::Sequential {
                    domain_min: 0.0,
                    domain_max: 9.0,
                    stops: brightfield_render::scale::SequentialScheme::Viridis.stops(),
                },
            },
            // Unbound categorical legend (no binding carries its path).
            super::LegendPlacement {
                path: "root/hconcat[3]".into(),
                rect: Rect::new(400.0, 200.0, 90.0, 30.0),
                scale: colour,
            },
        ];
        let bindings = vec![LegendBinding {
            legend_path: ComponentPath("root/hconcat[1]".into()),
            plot_path: ComponentPath("root/hconcat[0]".into()),
            selection: "sel".into(),
            colour_column: "g".into(),
        }];

        let views = super::placed_legend_views(&placements, &bindings, Some(&coordinator));
        assert_eq!(views.len(), 3);
        match &views[0].binding {
            Some((index, _)) => assert_eq!(*index, 0, "bound legend carries its binding index"),
            None => panic!("the bound categorical legend must carry the coordinator"),
        }
        assert!(views[1].binding.is_none(), "Sequential legends stay display-only");
        assert!(views[2].binding.is_none(), "unbound legends stay display-only");
    }

    // fww_ac06 (card 0016): colorScheme reaches the LIVE path — build_everything
    // resolves the plot's declared scheme into LivePlotMeta, the field the
    // coordinator threads into every live rebuild. Unknown schemes fall back to
    // viridis (warning path). Render-only: no SQL / plan-hash involvement.
    #[test]
    fn fww_ac06_live_plot_meta_carries_declared_scheme() {
        use brightfield_render::scale::SequentialScheme;

        let build = |color_scheme: &str, file: &str| {
            let src = format!(
                r#"
data:
  points:
    - {{ x: 1, y: 1 }}
    - {{ x: 2, y: 2 }}
plot:
  - mark: raster
    data: {{ from: points }}
    x: x
    y: y
colorScheme: {color_scheme}
"#
            );
            let dir = std::env::temp_dir().join(format!("bf-fww-ac06-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(file);
            std::fs::write(&path, src).unwrap();
            let (_dashboard, live) =
                super::build_everything(path.to_str().unwrap()).expect("pipeline runs");
            live.plots[0].scheme
        };

        assert_eq!(
            build("blues", "blues.yaml"),
            SequentialScheme::Blues,
            "the declared scheme rides LivePlotMeta into the live rebuild path"
        );
        assert_eq!(
            build("notascheme", "unknown.yaml"),
            SequentialScheme::Viridis,
            "an unknown scheme warns and falls back to viridis"
        );
    }

    // dmk_ac02 (card 0008, density marks): a heatmap plot's colorScheme reaches
    // the rendered Fill ramp through the REAL assembly seam — build_everything
    // resolves the plot's scheme, builds the per-mark HeatmapRenderer override,
    // and threads it through augment_scales — and the same scheme is THREADED
    // into LivePlotMeta.scheme. Threading only: render_plot_scene consumes the
    // scheme for Raster alone, so a live rebuild does NOT yet apply it to a
    // heatmap (the live renderer-config seam, recorded as deferred in the
    // density-marks spec).
    #[test]
    fn dmk_ac02_heatmap_colorscheme_consumed_end_to_end() {
        use brightfield_render::scale::{Scale, SequentialScheme};

        const SRC: &str = r#"
data:
  points:
    - { x: 1, y: 1 }
    - { x: 2, y: 2 }
    - { x: 3, y: 3 }
plot:
  - mark: heatmap
    data: { from: points }
    x: x
    y: y
colorScheme: blues
"#;
        let dir = std::env::temp_dir().join(format!("bf-dmk-ac02-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("heatmap-blues.yaml");
        std::fs::write(&path, SRC).unwrap();

        let (_dashboard, live) =
            super::build_everything(path.to_str().unwrap()).expect("pipeline runs");
        // The heatmap plot's post-assembly Fill scale is the blues ramp,
        // zero-anchored on the smoothed density domain.
        let fill = live.plots[0]
            .scales
            .get(Channel::Fill)
            .expect("heatmap plot has a Fill scale");
        match fill {
            Scale::Sequential { domain_min, stops, .. } => {
                assert_eq!(
                    stops,
                    &SequentialScheme::Blues.stops(),
                    "colorScheme: blues must reach the rendered Fill ramp (not the viridis default)"
                );
                assert!((domain_min - 0.0).abs() < f64::EPSILON, "zero-anchored");
            }
            other => panic!("expected a blues Fill Sequential, got {other:?}"),
        }
        // The live-path THREADING seam: the resolved scheme is carried on
        // LivePlotMeta. Threading is all this pins — render_plot_scene does not
        // yet consume it for heatmap (deferred: live renderer-config seam).
        assert_eq!(
            live.plots[0].scheme,
            SequentialScheme::Blues,
            "the declared scheme is threaded into LivePlotMeta (live consumption \
             for heatmap is deferred — render_plot_scene scheme-configures raster only)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // dmk_ac03 (card 0008, density marks): a cell plot's colorScheme reaches the
    // numeric-fill Sequential through the same per-mark assembly seam — DuckDB
    // executes the pass-through query, augment_scales replaces the inferred
    // Linear with the anchored blues ramp. First-render/headless only: a live
    // rebuild renders cell through the registry default (deferred: live
    // renderer-config seam, recorded in the density-marks spec).
    #[test]
    fn dmk_ac03_cell_colorscheme_consumed_end_to_end() {
        use brightfield_render::scale::{Scale, SequentialScheme};

        const SRC: &str = r#"
data:
  grid:
    - { day: Mon, slot: am, value: 1 }
    - { day: Mon, slot: pm, value: 4 }
    - { day: Tue, slot: am, value: 2 }
    - { day: Tue, slot: pm, value: 8 }
plot:
  - mark: cell
    data: { from: grid }
    x: slot
    y: day
    fill: value
colorScheme: blues
"#;
        let dir = std::env::temp_dir().join(format!("bf-dmk-ac03-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cell-blues.yaml");
        std::fs::write(&path, SRC).unwrap();

        let (_dashboard, live) =
            super::build_everything(path.to_str().unwrap()).expect("pipeline runs");
        let fill = live.plots[0]
            .scales
            .get(Channel::Fill)
            .expect("cell plot has a Fill scale");
        match fill {
            Scale::Sequential { domain_min, domain_max, stops } => {
                assert_eq!(
                    stops,
                    &SequentialScheme::Blues.stops(),
                    "colorScheme: blues must reach the cell's numeric-fill ramp"
                );
                assert!((domain_min - 0.0).abs() < f64::EPSILON, "min >= 0 anchors at zero");
                assert!((domain_max - 8.0).abs() < f64::EPSILON);
            }
            other => panic!("expected a blues Fill Sequential, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    // dmk_ac04 (card 0008, density marks): the contour mark's `thresholds`
    // attribute reaches the per-mark ContourRenderer override through the REAL
    // assembly seam (build_everything's mark_boxes), pinning the override wiring
    // end-to-end: 5 iso-levels stroke strictly more scene paths than 2 over the
    // same data. If the attr never reached the renderer, both builds would draw
    // the registry default level count and the two path counts would tie. (The
    // SQL half of the shield — thresholds NOT changing the emitted bin count —
    // is pinned in brightfield-sql's dmk_ac04 regression test.)
    #[test]
    fn dmk_ac04_contour_thresholds_override_reaches_renderer_end_to_end() {
        use brightfield_render::mark::count_scene_paths;

        let dir = std::env::temp_dir().join(format!("bf-dmk-ac04-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let build = |thresholds: usize, file: &str| {
            // The dmk_ac04 unimodal fixture as raw points — corners 1, edges 4,
            // centre 16 — so equiwidth binning reconstructs the 3x3 histogram.
            let mut rows = String::new();
            for (x, y, n) in [
                (1, 1, 1), (2, 1, 4), (3, 1, 1),
                (1, 2, 4), (2, 2, 16), (3, 2, 4),
                (1, 3, 1), (2, 3, 4), (3, 3, 1),
            ] {
                for _ in 0..n {
                    rows.push_str(&format!("    - {{ x: {x}, y: {y} }}\n"));
                }
            }
            let src = format!(
                "data:\n  points:\n{rows}plot:\n  - mark: contour\n    data: {{ from: points }}\n    x: x\n    y: y\n    thresholds: {thresholds}\n"
            );
            let path = dir.join(file);
            std::fs::write(&path, src).unwrap();
            let (dashboard, _live) =
                super::build_everything(path.to_str().unwrap()).expect("pipeline runs");
            count_scene_paths(&dashboard.plots[0].scene)
        };

        let (two, five) = (build(2, "contour-2.yaml"), build(5, "contour-5.yaml"));
        assert!(
            five > two,
            "thresholds: 5 must stroke more iso-lines than thresholds: 2 through \
             the per-mark override seam (got {five} vs {two}) — a tie means the \
             attr never reached the ContourRenderer"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legend_for_classifies_absent_named_and_unresolvable() {
        use brightfield_spec::ast::{LegendNode, ParamRef, SpecValue, ValueOrParamRef};
        use brightfield_spec::vocab::{ImplStatus, LegendChannel};

        // `options` is an IndexMap; the field type drives the `collect` target,
        // so no direct indexmap dependency is needed here.
        let node = |opts: Vec<(&str, ValueOrParamRef<SpecValue>)>| LegendNode {
            channel: LegendChannel::Color,
            status: ImplStatus::Implemented,
            options: opts.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        };

        assert!(matches!(super::legend_for(&node(vec![])), super::LegendFor::Absent));
        assert!(matches!(
            super::legend_for(&node(vec![("for", ValueOrParamRef::Value(SpecValue::String("p".into())))])),
            super::LegendFor::Named(ref n) if n == "p"
        ));
        // A param-valued `for:` is unresolvable — it must NOT fall through to the
        // sole-plot fallback (that would silently borrow another plot's scale).
        assert!(matches!(
            super::legend_for(&node(vec![("for", ValueOrParamRef::Param(ParamRef::new("sel")))])),
            super::LegendFor::Unresolvable
        ));
    }

    #[test]
    fn run_pipeline_returns_err_on_bad_spec_instead_of_exiting() {
        // The pipeline must return Err (not process::exit) so the hot-reload
        // watcher can keep the last good chart when a save is momentarily bad.
        // If run_pipeline still exited, this test process would die here.
        let missing = super::run_pipeline("/nonexistent/brightfield/spec.yaml");
        assert!(missing.is_err(), "missing spec should return Err, not exit");
    }

    /// sbp_ac04 (hand-off is Send): the profile set the watcher carries back
    /// from its throwaway session must cross the background→main boundary — so
    /// the whole `run_pipeline` return tuple, profiles included, must be Send.
    /// (The non-Send `Session` never crosses; only this data does.)
    #[test]
    fn sbp_ac04_profile_handoff_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Vec<super::SourceProfile>>();
        assert_send::<(super::Dashboard, super::ChromeSnapshot, Vec<super::SourceProfile>)>();
    }

    /// sbp_ac04 (hand-off carries real profiles): `run_pipeline` — the exact
    /// fn the watcher runs on the background executor — computes and returns
    /// per-source profiles from its throwaway session, so the sidebar refresh
    /// has real data to apply. A headless probe of the hand-off; the live
    /// mtime-watcher loop is confirmed in-app (sbp_ac05).
    #[test]
    fn sbp_ac04_run_pipeline_returns_source_profiles() {
        use brightfield_engine::ProfileOutcome;
        let dir = std::env::temp_dir().join(format!(
            "bf_sbp_ac04_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let spec_path = dir.join("spec.yaml");
        std::fs::write(
            &spec_path,
            "data:\n  t:\n    - { x: 1, y: 10 }\n    - { x: 2, y: 20 }\n    - { x: 3, y: 30 }\nplot:\n  - mark: dot\n    data: { from: t }\n    x: x\n    y: y\n",
        )
        .unwrap();

        let (_, _, profiles) =
            super::run_pipeline(spec_path.to_str().unwrap()).expect("pipeline ok");
        let _ = std::fs::remove_dir_all(&dir);

        let t = profiles.iter().find(|p| p.name == "t").expect("source t profiled");
        match &t.outcome {
            ProfileOutcome::Profiled { row_count, columns } => {
                assert_eq!(*row_count, 3);
                assert_eq!(
                    columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
                    vec!["x", "y"]
                );
            }
            other => panic!("expected Profiled, got {other:?}"),
        }
    }

    /// Card 0016 review (F2): the hot-reload chrome gate is a pure comparison —
    /// a plots-only rebuild passes, while a title / legend / render-metadata
    /// divergence names what changed so the watcher prints "restart to apply"
    /// instead of silently hot-swapping stale chrome.
    #[test]
    fn reload_gate_blocks_chrome_and_render_meta_divergence() {
        use brightfield_render::scale::SequentialScheme;

        let snapshot =
            |title: &str, scheme: SequentialScheme, inline: bool| super::ChromeSnapshot {
                title: title.to_string(),
                legends: vec![(10.0, 20.0, 120.0, 24.0, "Colour".to_string())],
                legend_bindings: vec![(
                    "root/hconcat[1]".to_string(),
                    "root/hconcat[0]".to_string(),
                    "sel".to_string(),
                    "species".to_string(),
                )],
                plot_render_meta: vec![("/plot/0".to_string(), scheme, inline)],
            };
        let launch = snapshot("framed", SequentialScheme::Blues, true);

        // Plots-only edit: identical chrome → hot-swap allowed.
        assert_eq!(
            super::chrome_divergence(&launch, &snapshot("framed", SequentialScheme::Blues, true)),
            None
        );
        // Title edit reloads with a stale header/titlebar without the gate.
        assert_eq!(
            super::chrome_divergence(&launch, &snapshot("renamed", SequentialScheme::Blues, true)),
            Some("dashboard title")
        );
        // colorScheme edit: without the gate the swap renders the new scheme
        // once, then the NEXT brush/slider gesture reverts to the launch-time
        // scheme held by the coordinator.
        assert_eq!(
            super::chrome_divergence(
                &launch,
                &snapshot("framed", SequentialScheme::Viridis, true)
            ),
            Some("per-plot render metadata (colorScheme/inline legend)")
        );
        // Inline-legend suppression flip (a standalone legend gained/lost its
        // `for:` target).
        assert_eq!(
            super::chrome_divergence(&launch, &snapshot("framed", SequentialScheme::Blues, false)),
            Some("per-plot render metadata (colorScheme/inline legend)")
        );
        // Hosted-legend rect or scale change.
        let mut moved = snapshot("framed", SequentialScheme::Blues, true);
        moved.legends[0].0 = 99.0;
        assert_eq!(
            super::chrome_divergence(&launch, &moved),
            Some("legend placement/scale")
        );
        let mut recoloured = snapshot("framed", SequentialScheme::Blues, true);
        recoloured.legends[0].4 = "Sequential".to_string();
        assert_eq!(
            super::chrome_divergence(&launch, &recoloured),
            Some("legend placement/scale")
        );

        // Binding-only divergence (card 0009 F7): an `as:`/`for:`-only edit
        // moves NO rect and changes NO scale — the legend still sits at the
        // same place drawing the same swatches — but its click wiring
        // (selection name here; equally plot path or colour column) differs,
        // so a hot swap would dispatch through the stale launch bindings.
        let mut rebound = snapshot("framed", SequentialScheme::Blues, true);
        rebound.legend_bindings[0].2 = "other".to_string();
        assert_eq!(
            super::chrome_divergence(&launch, &rebound),
            Some("legend selection binding (as:/for:)")
        );
        // Removing the binding altogether (as: deleted) equally gates.
        let mut unbound = snapshot("framed", SequentialScheme::Blues, true);
        unbound.legend_bindings.clear();
        assert_eq!(
            super::chrome_divergence(&launch, &unbound),
            Some("legend selection binding (as:/for:)")
        );
    }

    /// Card 0016 review (F2): `ChromeSnapshot::capture` maps the launch parts
    /// into the gate's comparison keys — rect + scale Debug key per legend,
    /// the click-wiring key tuple per legend binding (card 0009 F7), and
    /// path + scheme + inline flag per plot.
    #[test]
    fn reload_gate_snapshot_captures_comparison_keys() {
        use brightfield_render::scale::{Scale, ScaleSet, SequentialScheme};
        use brightfield_spec::analysis::{ComponentPath, LegendBinding};
        use brightfield_spec::layout::Rect;

        let scale = Scale::Colour {
            categories: vec!["a".to_string()],
            palette: vec![[0.3, 0.4, 0.6, 1.0]],
        };
        let legend = super::LegendPlacement {
            path: "root/hconcat[1]".to_string(),
            rect: Rect::new(1.0, 2.0, 3.0, 4.0),
            scale: scale.clone(),
        };
        let binding = LegendBinding {
            legend_path: ComponentPath("root/hconcat[1]".to_string()),
            plot_path: ComponentPath("root/hconcat[0]".to_string()),
            selection: "sel".to_string(),
            colour_column: "species".to_string(),
        };
        let meta = super::LivePlotMeta {
            path: "/plot/0".to_string(),
            mark_indices: vec![0],
            layout: ChartLayout::new(320.0, 240.0),
            bindings: Vec::new(),
            scales: ScaleSet::new(),
            draw_inline_legend: false,
            scheme: SequentialScheme::Blues,
        };

        let snap = super::ChromeSnapshot::capture(
            "framed".to_string(),
            &[legend],
            &[binding],
            &[meta],
        );
        assert_eq!(snap.title, "framed");
        assert_eq!(
            snap.legends,
            vec![(1.0, 2.0, 3.0, 4.0, format!("{scale:?}"))]
        );
        assert_eq!(
            snap.legend_bindings,
            vec![(
                "root/hconcat[1]".to_string(),
                "root/hconcat[0]".to_string(),
                "sel".to_string(),
                "species".to_string(),
            )],
            "the click-wiring keys ride the snapshot (card 0009 F7)"
        );
        assert_eq!(
            snap.plot_render_meta,
            vec![("/plot/0".to_string(), SequentialScheme::Blues, false)]
        );
    }

    /// Card 0009 F4a (static/live divergence, phantom binding): a dot plot
    /// with a string `fill:` plus a raster plot. `build_legend_bindings`
    /// counts colour plots by string fill option (dot only → sole → binding
    /// forms), but `resolve_legends` counts by LIVE scale (dot Colour AND
    /// raster Sequential → two → no sole → the bare legend gets NO
    /// placement). The reconcile must discard the phantom binding — else it
    /// holds the coordinator open with no clickable surface — and liveness
    /// must follow the placements.
    #[test]
    fn lcf_f4_orphan_binding_discarded_and_liveness_follows_placements() {
        use brightfield_ui::{CrossfilterCoordinator, LegendSelectBinding};

        const SRC: &str = r#"
params:
  sel: { select: crossfilter }
data:
  t:
    - { x: 1, y: 2, g: a }
    - { x: 2, y: 3, g: b }
    - { x: 3, y: 4, g: a }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: g
  - plot:
    - mark: raster
      data: { from: t }
      x: x
      y: y
  - legend: color
    as: $sel
"#;
        let dir = std::env::temp_dir().join(format!("bf-lcf-f4a-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dot-raster.yaml");
        std::fs::write(&path, SRC).unwrap();

        let (dashboard, live) =
            super::build_everything(path.to_str().unwrap()).expect("pipeline runs");

        // The static analysis DID produce a binding (the divergence premise)…
        let parsed = parse_spec(SRC, Format::Yaml).expect("parse");
        let analysis = analyse_spec(&parsed.spec).expect("analyse");
        assert_eq!(
            analysis.legend_bindings.len(),
            1,
            "premise: the static sole-fallback binds (string-fill population = 1)"
        );
        // …but the live placements skipped the legend (live population = 2),
        // so the reconcile discards the phantom and diagnoses it.
        assert!(
            dashboard.legends.is_empty(),
            "premise: two live colour scales → the bare legend gets no placement"
        );
        assert!(
            live.legend_bindings.is_empty(),
            "the phantom binding is discarded (card 0009 F4a)"
        );
        let (retained, diags) = super::reconcile_legend_bindings(
            &parsed.spec,
            &dashboard.legends,
            analysis.legend_bindings,
        );
        assert!(retained.is_empty());
        assert!(
            diags.iter().any(|d| d.contains("no hosted legend")),
            "the discard is diagnosed: {diags:?}"
        );

        // Coordinator liveness follows the placements: with the phantom gone
        // (and no brushes/sliders) the dashboard is not live.
        let legend_select: Vec<LegendSelectBinding> =
            live.legend_bindings.iter().map(Into::into).collect();
        assert!(
            CrossfilterCoordinator::new(live.session, live.marks, vec![], vec![], legend_select)
                .is_none(),
            "no placement → no binding → no coordinator"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Card 0009 F4b (static/live divergence, dead clicks diagnosed): two
    /// plots whose `fill:` options are BOTH strings (one names a numeric
    /// column) — the static population is 2 (ambiguous, no binding) while
    /// the live population is 1 (the numeric fill infers Linear, not
    /// Colour), so the bare legend IS placed but carries `as:` with no
    /// binding. The reconcile diagnoses the dead clicks.
    #[test]
    fn lcf_f4_placed_as_legend_without_binding_is_diagnosed() {
        const SRC: &str = r#"
params:
  sel: { select: crossfilter }
data:
  t:
    - { x: 1, y: 2, g: a, n: 5 }
    - { x: 2, y: 3, g: b, n: 7 }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: g
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: n
  - legend: color
    as: $sel
"#;
        let dir = std::env::temp_dir().join(format!("bf-lcf-f4b-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("numeric-fill.yaml");
        std::fs::write(&path, SRC).unwrap();

        let (dashboard, live) =
            super::build_everything(path.to_str().unwrap()).expect("pipeline runs");

        let parsed = parse_spec(SRC, Format::Yaml).expect("parse");
        let analysis = analyse_spec(&parsed.spec).expect("analyse");
        assert!(
            analysis.legend_bindings.is_empty(),
            "premise: two string-fill plots → static sole-fallback is ambiguous"
        );
        assert_eq!(
            dashboard.legends.len(),
            1,
            "premise: one live Colour scale → the bare legend IS placed"
        );
        assert!(live.legend_bindings.is_empty());

        let (retained, diags) = super::reconcile_legend_bindings(
            &parsed.spec,
            &dashboard.legends,
            analysis.legend_bindings,
        );
        assert!(retained.is_empty());
        assert!(
            diags.iter().any(|d| d.contains("clicks on it will not filter")),
            "the placed-but-unbound `as:` legend is diagnosed (card 0009 F4b): {diags:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn msv_ac05_graceful_failure_skips_invalid_mark() {
        // Spec with one valid mark (dot, data.from) and one invalid (hexbin, unsupported).
        let yaml = r#"
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
  - mark: hexbin
    data: { from: t }
"#;
        let parsed = parse_spec(yaml, Format::Yaml).expect("parse failed");
        let analysis = analyse_spec(&parsed.spec).expect("analysis failed");

        let engine = Engine::new();
        let load = engine
            .load_spec(parsed.spec.clone(), analysis, None)
            .expect("load_spec failed");
        let mut session = load.session;

        // Execute all marks — dot should succeed, hexbin should fail.
        let results = session.execute_all();

        let mut successful = Vec::new();
        let mut skipped = 0_usize;
        for (i, result) in results.into_iter().enumerate() {
            match result {
                Ok(batches) => {
                    if let Some(batch) = batches.into_iter().next() {
                        successful.push((i, batch));
                    }
                }
                Err(_) => {
                    skipped += 1;
                }
            }
        }

        // Exactly one mark skipped (hexbin), one succeeded (dot).
        assert_eq!(skipped, 1, "expected 1 skipped mark");
        assert_eq!(successful.len(), 1, "expected 1 successful mark");

        // The successful batch should have the inline data (2 rows).
        let (_, batch) = &successful[0];
        assert_eq!(batch.num_rows(), 2);
        assert!(batch.schema().index_of("x").is_ok());
        assert!(batch.schema().index_of("y").is_ok());

        // Build scene from the valid mark — manually construct channel map
        // since we're testing the pipeline's graceful failure, not from_mark.
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let layout = ChartLayout::new(640.0, 480.0);
        let renderer: Box<dyn MarkRenderer> = Box::new(DotRenderer);

        let chart_data = vec![ChartData {
            batch,
            channel_map: &cm,
            renderer: renderer.as_ref(),
            layout,
            view_extent: None,
            highlight: None,
        }];
        let refs: Vec<&ChartData<'_>> = chart_data.iter().collect();
        let (scene, scales) = build_multi_mark_scene(&refs, true);

        // Scene should be non-empty (the valid dot mark rendered).
        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "scene should have content from the valid mark"
        );

        // Scales should exist for x and y from the dot mark.
        assert!(scales.get(Channel::X).is_some(), "x scale should exist");
        assert!(scales.get(Channel::Y).is_some(), "y scale should exist");
    }
}
