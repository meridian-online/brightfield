//! Brightfield application entry point.
//!
//! Orchestrates the full spec-to-chart pipeline:
//! parse → analyse → engine → execute → render → display.
//!
//! The GPUI window requires a platform implementation (gpui_macos on macOS,
//! which needs full Xcode + Metal compiler). Without it, the pipeline runs
//! headlessly and prints a summary.

use std::collections::{HashMap, HashSet};
use std::env;
use std::path::Path;
use std::process;

use brightfield_engine::{Engine, Session};
use brightfield_render::channel::{Channel, ChannelMap};
use brightfield_render::layout::ChartLayout;
use brightfield_render::legend::{colour_legend_size, render_colour_legend_at};
use brightfield_render::mark::{default_renderers, find_renderer, MarkRenderer, RasterRenderer};
use brightfield_render::scale::{Scale, ScaleSet, SequentialScheme};
use brightfield_render::scene::{build_multi_mark_scene, compose_dashboard, ChartData};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::layout::{
    collect_plot_nodes, placed_input_nodes, placed_legend_nodes, placed_plots, Rect,
};
use brightfield_spec::parse_spec_path;
use brightfield_spec::vocab::{InputKind, LegendChannel, MarkKind};
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

/// A standalone `legend:` node's placement (multi-view inc 6): its dashboard rect
/// and the colour scale it displays, resolved from the plot its `for:` names. The
/// headless/PNG path draws it into the composite; hosting it as a window element
/// is a follow-up (see the legends/spacers memo).
struct LegendPlacement {
    rect: Rect,
    scale: Scale,
}

/// A rendered dashboard: the bounding-box dimensions, one scene per plot, the
/// placed slider widgets, and the standalone legends. The headless/PNG path
/// composites these; the window hosts one element per plot + one per slider
/// (legend window-hosting is a follow-up).
struct Dashboard {
    width: u32,
    height: u32,
    plots: Vec<PlotRender>,
    sliders: Vec<SliderPlacement>,
    legends: Vec<LegendPlacement>,
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
/// plot-level attribute (Mosaic's colour scale is plot-scoped); it is consumed on
/// the headless authoring path. The live cross-filter path inherits the viridis
/// default (a recorded follow-up — see the spec's deferred list).
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
    for (rect, node) in placed_legend_nodes(spec, Rect::new(0.0, 0.0, 0.0, 0.0)) {
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
                rect: Rect::new(rect.x, rect.y, w, h),
                scale,
            });
        }
    }
    out
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
}

/// Thin wrapper for the headless/PNG path and the hot-reload watcher: runs the
/// full pipeline and returns just the renderable [`Dashboard`], dropping the live
/// engine state. Dropping the non-`Send` [`Session`] here is what lets the
/// watcher run this off the main thread (a `Dashboard` is `Send`).
///
/// Returns `Err` (rather than exiting) on any failure, so callers can recover —
/// the hot-reload watcher keeps the last good chart when a mid-edit save is
/// momentarily invalid. `main` turns the initial error into a clean exit.
fn run_pipeline(spec_path: &str) -> Result<Dashboard, String> {
    build_everything(spec_path).map(|(dashboard, _live)| dashboard)
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

        // The plot's colour scheme, applied to its raster marks (headless path).
        let scheme = plot_nodes
            .iter()
            .find(|(path, _)| *path == plot.path)
            .map(|(_, node)| raster_scheme(node.attributes.get("colorScheme")))
            .unwrap_or_default();

        // Owned per-mark renderer overrides. A raster mark uses a scheme-configured
        // RasterRenderer (built here so the plot's colorScheme is honoured); every
        // other mark borrows the shared registry. Declared before `chart_data` so
        // the boxes outlive the references into them.
        let raster_boxes: Vec<Option<Box<dyn MarkRenderer + Send + Sync>>> = group
            .mark_indices
            .iter()
            .map(|&mi| {
                let is_raster =
                    mark_inputs.get(mi).is_some_and(|m| m.kind == MarkKind::Raster);
                is_raster
                    .then(|| Box::new(RasterRenderer { scheme }) as Box<dyn MarkRenderer + Send + Sync>)
            })
            .collect();

        let mut chart_data: Vec<ChartData<'_>> = Vec::new();
        for (j, &mi) in group.mark_indices.iter().enumerate() {
            let Some(m) = mark_inputs.get(mi) else { continue };
            let Some(batch) = m.batch.as_ref() else { continue };
            let renderer: &dyn MarkRenderer = if let Some(b) = &raster_boxes[j] {
                b.as_ref()
            } else {
                match find_renderer(&registry, m.kind) {
                    Some(r) => r,
                    None => {
                        eprintln!("warning: no renderer for mark kind {:?} — skipping", m.kind);
                        continue;
                    }
                }
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
        },
        LiveParts {
            session,
            marks: mark_inputs,
            plots: live_plots,
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
fn spawn_spec_watcher(cx: &mut gpui::App, watched: Vec<WatchedPlot>, spec_path: String) {
    const POLL: std::time::Duration = std::time::Duration::from_millis(300);

    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        let mut last = file_mtime(&spec_path);
        loop {
            cx.background_executor().timer(POLL).await;
            let now = file_mtime(&spec_path);
            if now == last {
                continue;
            }
            last = now;

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
                Ok(dashboard) => {
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
                    eprintln!("reloaded {spec_path}");
                }
                Err(e) => {
                    eprintln!("reload skipped (keeping last good chart): {e}");
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
    // opening a window. Triggered by `BRIGHTFIELD_DUMP_PNG=<path>`.
    if let Ok(dump_path) = env::var("BRIGHTFIELD_DUMP_PNG") {
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

    // Open a native GPUI window: one ChartElement per plot, positioned per the
    // layout, each with its own ChartState (so interaction is per-plot).
    #[cfg(target_os = "macos")]
    {
        use gpui::AppContext;
        use std::rc::Rc;

        let renderer = brightfield_ui::VelloRenderer::new();
        let app = gpui::Application::with_platform(Rc::new(gpui_macos::MacPlatform::new(false)));
        let spec_path = spec_path.to_string();
        // `legends`: the standalone colour legends render in the headless/PNG
        // composite; hosting them as window elements is a follow-up (see the
        // legends/spacers memo), so the window path ignores them for now.
        let Dashboard { width, height, plots, sliders, legends: _ } = dashboard;
        let LiveParts {
            session,
            marks,
            plots: live_plots_meta,
        } = live;
        app.run(move |cx| {
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
                    scales: meta.scales,
                    draw_inline_legend: meta.draw_inline_legend,
                    state: w.state.clone(),
                })
                .collect();
            // Coordinator slider bindings, in the same order as the hosted slider
            // widgets below (both derived from `sliders`), so a widget's index
            // matches its binding.
            let slider_bindings: Vec<SliderBinding> =
                sliders.iter().map(|s| s.binding.clone()).collect();
            let coordinator =
                CrossfilterCoordinator::new(session, marks, live_plots, slider_bindings);

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

            // Size the window's content to the dashboard instead of
            // WindowOptions::default() (which opened a huge window with the chart
            // in a corner and a black void around it). `window_bounds` is the
            // CONTENT rect — the macOS titlebar is added above it — so use the
            // exact dashboard size. The window is resizable; ChartView fills it
            // with a white background and centres the plots, so enlarging shows a
            // clean margin rather than a void (chart-scaling reflow is inc 6).
            let window_size = gpui::size(gpui::px(width as f32), gpui::px(height as f32));
            let window_opts = gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::centered(
                    None,
                    window_size,
                    cx,
                ))),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Brightfield".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let _window = cx
                .open_window(window_opts, move |_window, cx| {
                    cx.new(|_| {
                        brightfield_ui::ChartView::new(
                            f64::from(width),
                            f64::from(height),
                            charts,
                            placed_sliders,
                        )
                    })
                })
                .expect("failed to open window");

            // Hot-reload: swap each plot's scene when the spec changes on disk.
            spawn_spec_watcher(cx, watched, spec_path);
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
