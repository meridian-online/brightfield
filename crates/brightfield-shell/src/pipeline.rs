//! Spec → composited Vello scene (framework-free).
//!
//! A focused port of the app's `build_everything` plot-composition path, using
//! only the framework-free crates (`brightfield-spec` / `-engine` / `-sql` /
//! `-render`). It parses a Mosaic spec, executes each mark's query on the
//! engine, builds one Vello scene per plot (its own axes/scales/legend, titles
//! and axis insets resolved via the same public helpers the app uses), and
//! composites them into a single dashboard scene the egui host presents.
//!
//! Scope for the loop-first phase: colour-scheme / projection / highlight /
//! explicit colorDomain and standalone-legend relocation are NOT ported (the
//! golden `dashboard.yaml` and the simple examples use none of them). Marks are
//! taken from their first result batch (examples fit one ~2048-row chunk); the
//! app's multi-chunk concat is the production path.

use std::path::Path;

use arrow::record_batch::RecordBatch;
use brightfield_engine::coordinator::{Coordinator, Interaction};
use brightfield_engine::error::EngineError;
use brightfield_engine::Engine;
use brightfield_render::channel::ChannelMap;
use brightfield_render::inset::{resolve_insets_for_marks, DEFAULT_SCALE_INSET};
use brightfield_render::layout::{ChartLayout, Margins};
use brightfield_render::mark::{default_renderers, find_renderer, MarkRenderer};
use brightfield_render::scale::ScaleSet;
use brightfield_render::scene::{build_multi_mark_scene, compose_dashboard, ChartData};
use brightfield_render::{grow_margins, resolve_titles};
use brightfield_spec::analysis::{
    analyse_spec, build_brushable_bindings, BrushKind, ComponentPath,
};
use brightfield_spec::layout::{collect_plot_nodes, placed_plots, resolve_plot_insets, Rect};
use brightfield_spec::vocab::MarkKind;
use brightfield_spec::{parse_spec, parse_spec_path, Format, Spec};
use brightfield_sql::{collect_marks, collect_plot_groups};
use brightfield_workbench::subject::RunState;
use vello::Scene;

/// One placed plot of the composed dashboard, carried beside the scene so the
/// shell can act on the chart rather than merely picture it: the margin
/// legend reads the *displayed* scales, and a gesture inverts its pixels
/// through the same set — which is the only way the predicate a brush pushes
/// can mean the rectangle the user drew.
///
/// Everything here is a by-product of the composition that already happened;
/// nothing is recomputed, so a handle cannot disagree with the scene beside it.
pub struct PlotHandle {
    /// The plot node's component path (`root`, `root/hconcat[0]`, …) — the
    /// same join key `collect_plot_groups` and the brushable bindings use.
    pub path: String,
    /// The placed rect on the dashboard plane, in logical pixels.
    pub rect: Rect,
    /// The scale set this plot was drawn against. Pixel↔data inversion for
    /// gestures, and the series the margin legend is accurate to.
    pub scales: ScaleSet,
    /// The layout (margins + insets) the scales' pixel ranges live in.
    pub layout: ChartLayout,
    /// The mark kinds drawn on this plot, in declaration order. The first is
    /// the plot's presenting kind — the parameter the chart item reads.
    pub marks: Vec<MarkKind>,
    /// The plot's brush/point gesture binding, when its spec declares one.
    pub gesture: Option<GestureBinding>,
}

/// A plot's declared interaction, resolved from the spec's brushable-interactor
/// analysis to exactly what the coordinator seam consumes: which selection the
/// gesture writes, as which contributor, over which columns.
#[derive(Clone, Debug)]
pub struct GestureBinding {
    /// The selection name the gesture writes to (`as: $brush` → `"brush"`).
    pub selection: String,
    /// The contributor identity (the parent plot's node path) — crossfilter
    /// self-exclusion compares this, so it is carried, never re-derived.
    pub contributor: ComponentPath,
    /// Which gesture the interactor declared (interval axes / point toggle).
    pub kind: BrushKind,
    /// The x channel's column expression, when the first mark names one.
    pub x_column: Option<String>,
    /// The y channel's column expression, when the first mark names one.
    pub y_column: Option<String>,
}

/// One spec-declared scalar parameter with a slider widget behind it: what the
/// controls rail binds instead of its worked example, when the spec declares
/// anything to bind.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamControl {
    /// The parameter name (`$threshold` → `"threshold"`).
    pub name: String,
    /// Its current value.
    pub value: f64,
    /// Slider minimum, from the input widget's `min:` (0 when unstated).
    pub min: f64,
    /// Slider maximum, from the input widget's `max:` (1 when unstated).
    pub max: f64,
    /// Slider step, from the input widget's `step:` (`None` = continuous).
    pub step: Option<f64>,
}

/// One composited dashboard ready to present: the merged Vello scene, its
/// logical bounding size, and the spec's declared title (for the window chrome).
pub struct Composed {
    /// The single composited Vello scene (all plots placed on the page plane).
    pub scene: Scene,
    /// Dashboard width in logical pixels.
    pub width: u32,
    /// Dashboard height in logical pixels.
    pub height: u32,
    /// The spec's `meta.title`, if declared.
    pub title: Option<String>,
    /// The placed plots, with the scales and gesture bindings each was
    /// composed against. Empty only for [`Composed::empty`].
    pub plots: Vec<PlotHandle>,
    /// The spec's slider-backed scalar params, for the controls rail.
    pub params: Vec<ParamControl>,
    /// The run-state of materialised data this preview shows, when it shows
    /// any — the honesty channel at the preview surface.
    ///
    /// `None` means the preview makes **no currency claim**: the compose
    /// paths in this module set `None` because they execute their queries
    /// live for this very composition (nothing previewed here outlived an
    /// edit). A caller whose spec reads output materialised by a pipeline
    /// run annotates via [`Composed::with_run_state`], **ingesting** the
    /// state from that run's contract — it is never computed here.
    ///
    /// The render is [`Composed::run_state_line`]: minimal but real, so an
    /// annotated stale preview is never presented bare. Fuller status chrome
    /// arrives with the chart-side status work and must consume this same
    /// vocabulary rather than define a second one.
    pub run_state: Option<RunState>,
}

impl Composed {
    /// A dashboard with no plots on it: an empty scene, no area, no title.
    ///
    /// [`compose_spec`] never produces this — it returns `Err` when nothing
    /// rendered, and a dashboard's size is the union of its placed plots' rects,
    /// so any success has area. This exists for
    /// [`brightfield_workbench::audit`], which constructs every pane of a view
    /// and asks it what it shows over a document with nothing in it, and so
    /// needs "nothing in it" to be a value that can be built without a spec, a
    /// device or a window.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            scene: Scene::new(),
            width: 0,
            height: 0,
            title: None,
            plots: Vec::new(),
            params: Vec::new(),
            run_state: None,
        }
    }

    /// Annotate this preview with the run-state of the materialised data it
    /// shows, read off the run's contract by the caller. Consumes and returns
    /// `self` so the annotation happens at the compose call site, not as a
    /// mutation something else can forget to make.
    #[must_use]
    pub fn with_run_state(mut self, state: RunState) -> Self {
        self.run_state = Some(state);
        self
    }

    /// The one-line run-state banner this preview draws, when it previews
    /// materialised run output at all. `None` for a live-queried dashboard —
    /// no claim is made, so no label is owed.
    ///
    /// The words and tone come from the workbench vocabulary
    /// ([`RunState::label`] / [`RunState::gloss`]), so a stale preview here
    /// and a stale step in the inspector say it the same way — and a preview
    /// annotated stale can never render the fresh line.
    #[must_use]
    pub fn run_state_line(&self) -> Option<String> {
        self.run_state
            .map(|s| format!("data {} — {}", s.label(), s.gloss()))
    }
}

/// Run a Mosaic spec at `spec_path` through parse → analyse → engine → execute →
/// per-plot scene → composite, returning the [`Composed`] dashboard.
///
/// # Errors
///
/// Returns a human-readable message if any pipeline stage fails or no mark
/// renders.
pub fn compose_spec(spec_path: &str) -> Result<Composed, String> {
    let parsed = parse_spec_path(spec_path).map_err(|e| format!("parse error: {e}"))?;
    compose(parsed.spec, Path::new(spec_path).parent())
}

/// [`compose_spec`], with the session **kept**: the live dashboard holding
/// the DuckDB session for interaction, plus its first composite. What the
/// window boots a command-line chart spec through, so a brush has something
/// to re-query; the one-shot [`compose_spec`] remains the capture tiers'
/// deterministic path.
///
/// # Errors
///
/// As [`compose_spec`].
pub fn live_spec(spec_path: &str) -> Result<(LiveDashboard, Composed), String> {
    let parsed = parse_spec_path(spec_path).map_err(|e| format!("parse error: {e}"))?;
    let mut dash = LiveDashboard::load(parsed.spec, Path::new(spec_path).parent())?;
    let composed = dash.present()?;
    Ok((dash, composed))
}

/// The same pipeline over spec **text** rather than a file.
///
/// What it exists for: the starting points in [`crate::starts`] are
/// `include_str!`-ed into the binary, so there is no path to hand
/// [`compose_spec`] — and inventing one by resolving `examples/` relative to
/// the working directory is exactly the decoy this replaces, since it works
/// from the repo root and nowhere else.
///
/// `base_dir` is where relative `file:` paths in the spec resolve; `None` for
/// a spec whose data is inline, which every embedded start's is.
///
/// # Errors
///
/// As [`compose_spec`].
pub fn compose_spec_str(source: &str, base_dir: Option<&Path>) -> Result<Composed, String> {
    let parsed = parse_spec(source, Format::Yaml).map_err(|e| format!("parse error: {e}"))?;
    compose(parsed.spec, base_dir)
}

/// Everything after the parse, shared by both entry points above.
fn compose(spec: Spec, spec_dir: Option<&Path>) -> Result<Composed, String> {
    let analysis = analyse_spec(&spec).map_err(|e| format!("analysis error: {e}"))?;

    let engine = Engine::new();
    let load = engine
        .load_spec(spec.clone(), analysis, spec_dir)
        .map_err(|e| format!("engine error: {e}"))?;
    let mut session = load.session;

    // Execute every mark; keep its first result batch (examples fit one chunk).
    let results = session.execute_all();
    compose_from_results(&spec, results)
}

/// A live, session-holding dashboard — the push-down seam at the presentation
/// layer (per the push-down architecture: interactions are queries).
///
/// The one-shot [`compose_spec`] path parses, executes once, composites, and
/// **drops the session**: there is no path for a later brush or slider to
/// re-query. [`LiveDashboard`] instead HOLDS a [`Coordinator`] — and therefore
/// the live DuckDB session — across frames. An interaction is handed to
/// [`LiveDashboard::apply`], which resolves it to a predicate/param the engine
/// pushes into DuckDB, re-queries the affected marks, and re-composites through
/// the identical layout/scene path the first paint took (`compose_from_results`).
/// No frame is ever built by filtering a materialised batch in Rust — the filter
/// is in the emitted SQL.
///
/// This is the synchronous handle a single-window presenter drives on its own
/// thread. The off-UI-thread interaction path (coalescing + interrupt +
/// generation-stamped supersession, forced by a sustained drag) is
/// [`brightfield_engine::coordinator::QueryLoop`]; wiring its channels into a
/// specific egui event loop is the chrome layer's concern, not this seam's.
pub struct LiveDashboard {
    coordinator: Coordinator,
    spec: Spec,
}

impl LiveDashboard {
    /// Load a spec and hold its session live for interaction. `spec_dir` is
    /// where relative `file:` paths resolve (`None` for inline-data specs).
    ///
    /// # Errors
    ///
    /// As [`compose_spec`]: a human-readable message on analyse / load failure.
    pub fn load(spec: Spec, spec_dir: Option<&Path>) -> Result<Self, String> {
        let analysis = analyse_spec(&spec).map_err(|e| format!("analysis error: {e}"))?;
        let coordinator = Coordinator::load(spec.clone(), analysis, spec_dir)
            .map_err(|e| format!("engine error: {e}"))?;
        Ok(Self { coordinator, spec })
    }

    /// Load from spec text (mirrors [`compose_spec_str`]).
    ///
    /// # Errors
    ///
    /// As [`LiveDashboard::load`].
    pub fn load_str(source: &str, spec_dir: Option<&Path>) -> Result<Self, String> {
        let parsed = parse_spec(source, Format::Yaml).map_err(|e| format!("parse error: {e}"))?;
        Self::load(parsed.spec, spec_dir)
    }

    /// Composite the CURRENT materialisation into a dashboard scene — the first
    /// paint and every post-interaction re-paint go through here.
    ///
    /// # Errors
    ///
    /// As [`compose_spec`] (returns `Err` when nothing renders).
    pub fn present(&mut self) -> Result<Composed, String> {
        let results = self.coordinator.session_mut().execute_all();
        compose_from_results(&self.spec, results)
    }

    /// Apply one interaction — push its predicate/param into DuckDB, re-query,
    /// and re-composite. This is the seam: an interaction resolves to a query.
    ///
    /// A [`Interaction::SetParam`] also lands in this handle's spec copy, so
    /// the [`ParamControl`]s the next composition surfaces carry the value the
    /// slider was just dragged to — otherwise every re-present would snap the
    /// control back to the spec's boot value while the query ran at the new
    /// one, a lie in whichever direction the reader trusted.
    ///
    /// # Errors
    ///
    /// As [`LiveDashboard::present`].
    pub fn apply(&mut self, interaction: Interaction) -> Result<Composed, String> {
        if let Interaction::SetParam { name, value } = &interaction {
            use brightfield_spec::ast::ParamNode;
            if let Some(node @ ParamNode::Value(_)) = self.spec.params.get_mut(name) {
                *node = ParamNode::Value(value.clone());
            }
        }
        self.coordinator.apply(interaction);
        self.present()
    }

    /// The live coordinator, for surfaces that read the session directly (a grid
    /// at a step, distinct-value option lists) or hold the interrupt handle.
    pub fn coordinator(&mut self) -> &mut Coordinator {
        &mut self.coordinator
    }
}

/// Build the composited dashboard from a spec and its per-mark execution
/// results. Shared by the one-shot [`compose`] path and the live
/// [`LiveDashboard`] re-query seam, so a re-composite after an interaction takes
/// the identical layout and scene path as the first paint.
fn compose_from_results(
    spec: &Spec,
    results: Vec<Result<Vec<RecordBatch>, EngineError>>,
) -> Result<Composed, String> {
    let marks = collect_marks(spec);
    let mut batches: Vec<Option<RecordBatch>> = Vec::with_capacity(marks.len());
    let mut channel_maps: Vec<ChannelMap> = Vec::with_capacity(marks.len());
    let mut kinds = Vec::with_capacity(marks.len());
    for (i, result) in results.into_iter().enumerate() {
        let batch = match result {
            Ok(bs) => bs.into_iter().next(),
            Err(e) => {
                eprintln!("warning: skipping mark {i}: {e}");
                None
            }
        };
        batches.push(batch);
        channel_maps.push(ChannelMap::from_mark(marks[i]));
        kinds.push(marks[i].kind);
    }

    let placed = placed_plots(spec, Rect::new(0.0, 0.0, 0.0, 0.0));
    let groups = collect_plot_groups(spec);
    let plot_nodes = collect_plot_nodes(spec);
    let registry = default_renderers();
    let brushable = build_brushable_bindings(spec);

    // Own each plot's scene; place them below.
    let mut placements: Vec<(f64, f64, Scene)> = Vec::new();
    let mut plots: Vec<PlotHandle> = Vec::new();
    for plot in &placed {
        let Some(group) = groups.iter().find(|g| g.plot_path == plot.path) else {
            continue;
        };

        let mut chart_data: Vec<ChartData<'_>> = Vec::new();
        let mut plot_marks: Vec<MarkKind> = Vec::new();
        for &mi in &group.mark_indices {
            let Some(batch) = batches.get(mi).and_then(|b| b.as_ref()) else {
                continue;
            };
            let renderer: &dyn MarkRenderer = match find_renderer(&registry, kinds[mi]) {
                Some(r) => r,
                None => {
                    eprintln!("warning: no renderer for mark {mi} — skipping");
                    continue;
                }
            };
            plot_marks.push(kinds[mi]);
            chart_data.push(ChartData {
                batch,
                channel_map: &channel_maps[mi],
                renderer,
                layout: ChartLayout::new(plot.rect.width, plot.rect.height),
                view_extent: None,
                highlight: None,
            });
        }
        if chart_data.is_empty() {
            continue;
        }

        // Axis + plot titles, then grow the margins to reserve their band.
        let title_maps: Vec<&ChannelMap> = chart_data.iter().map(|d| d.channel_map).collect();
        let titles = plot_nodes
            .iter()
            .find(|(p, _)| *p == plot.path)
            .map(|(_, node)| resolve_titles(node, &title_maps))
            .unwrap_or_default();
        drop(title_maps);

        // Axis insets so edge marks render whole inside the frame clip.
        let explicit_insets = plot_nodes
            .iter()
            .find(|(p, _)| *p == plot.path)
            .map(|(_, node)| resolve_plot_insets(node))
            .unwrap_or_default();
        let inset_entries: Vec<_> = chart_data
            .iter()
            .map(|d| (d.batch, d.channel_map, d.renderer))
            .collect();
        let insets = resolve_insets_for_marks(explicit_insets, &inset_entries, DEFAULT_SCALE_INSET);
        drop(inset_entries);

        let margins = grow_margins(Margins::default(), &titles);
        let layout = ChartLayout::with_margins_and_insets(
            plot.rect.width,
            plot.rect.height,
            margins,
            insets,
        );
        for d in &mut chart_data {
            d.layout = layout;
        }

        let refs: Vec<&ChartData<'_>> = chart_data.iter().collect();
        // `draw_inline_legend = false`: the legend is NOT baked into the data
        // scene. The shell draws it as a native margin panel outside the plot
        // rect, from the scales returned here — one legend per chart, one
        // source of truth, and no in-plot swatch block a margin copy could
        // drift from or that could sit on top of the marks.
        let (scene, scales) = build_multi_mark_scene(&refs, false, &titles);
        drop(refs);
        drop(chart_data);
        placements.push((plot.rect.x, plot.rect.y, scene));

        let gesture = brushable
            .iter()
            .find(|b| b.parent_plot.0 == plot.path)
            .map(|b| GestureBinding {
                selection: b.selection.clone(),
                contributor: b.parent_plot.clone(),
                kind: b.kind,
                x_column: b.channels.x.clone(),
                y_column: b.channels.y.clone(),
            });
        plots.push(PlotHandle {
            path: plot.path.clone(),
            rect: plot.rect,
            scales,
            layout,
            marks: plot_marks,
            gesture,
        });
    }

    if placements.is_empty() {
        return Err("no marks rendered successfully".to_string());
    }

    let width = placed
        .iter()
        .map(|p| p.rect.x + p.rect.width)
        .fold(0.0_f64, f64::max)
        .ceil() as u32;
    let height = placed
        .iter()
        .map(|p| p.rect.y + p.rect.height)
        .fold(0.0_f64, f64::max)
        .ceil() as u32;

    let refs2: Vec<(f64, f64, &Scene)> = placements.iter().map(|(x, y, s)| (*x, *y, s)).collect();
    let scene = compose_dashboard(f64::from(width), f64::from(height), &refs2);

    let title = spec.meta.as_ref().and_then(|m| m.title.clone());
    Ok(Composed {
        scene,
        width,
        height,
        title,
        plots,
        params: param_controls(spec),
        // Live-queried this very composition — no materialised run output is
        // being previewed, so no currency claim is made (or owed). A caller
        // previewing run output annotates with `with_run_state`, ingesting
        // from the run's contract.
        run_state: None,
    })
}

/// The spec's slider-backed scalar params: every `input: slider` widget bound
/// `as: $param` whose param currently holds a number, with the widget's own
/// `min:`/`max:`/`step:` range. Read off the spec, never invented — a rail
/// slider over a range the spec did not declare would be a control whose ends
/// mean nothing.
fn param_controls(spec: &Spec) -> Vec<ParamControl> {
    use brightfield_spec::ast::{Component, Input, ParamNode, SpecValue, ValueOrParamRef};
    use brightfield_spec::vocab::InputKind;

    fn numeric(v: &SpecValue) -> Option<f64> {
        match v {
            SpecValue::Integer(i) => Some(*i as f64),
            SpecValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    fn collect<'s>(component: &'s Component, out: &mut Vec<&'s Input>) {
        match component {
            Component::Input(input) => out.push(input),
            Component::Plot(node) => {
                for item in &node.items {
                    collect(item, out);
                }
            }
            Component::HConcat(node) | Component::VConcat(node) => {
                for item in &node.items {
                    collect(item, out);
                }
            }
            _ => {}
        }
    }

    let mut inputs = Vec::new();
    if let Some(root) = &spec.root {
        collect(root, &mut inputs);
    }

    let mut out = Vec::new();
    for input in inputs {
        if input.kind != InputKind::Slider {
            continue;
        }
        let Some(param) = &input.as_param else {
            continue;
        };
        let name = param.0.clone();
        let Some(ParamNode::Value(value)) = spec.params.get(&name) else {
            continue;
        };
        let Some(value) = numeric(value) else {
            continue;
        };
        let option = |key: &str| match input.options.get(key) {
            Some(ValueOrParamRef::Value(v)) => numeric(v),
            _ => None,
        };
        out.push(ParamControl {
            name,
            value,
            min: option("min").unwrap_or(0.0),
            max: option("max").unwrap_or(1.0),
            step: option("step"),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_engine::SqlPredicate;
    use brightfield_spec::analysis::ComponentPath;

    const SPEC: &str = r#"
params:
  brush:
    select: intersect
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
    - { x: 4, y: 40 }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
    y: y
"#;

    #[test]
    fn live_dashboard_holds_session_and_re_queries_on_interaction() {
        // The seam at the presentation layer: the session is held across frames
        // and a brush resolves to a pushed predicate + a re-composite, rather
        // than the one-shot compose_spec path that drops the session.
        let mut dash = LiveDashboard::load_str(SPEC, None).expect("load");
        let first = dash.present().expect("first paint");
        assert!(first.width > 0 && first.height > 0, "first paint has area");
        assert_eq!(dash.coordinator().generation(), 0);

        let after = dash
            .apply(Interaction::Select {
                name: "brush".to_string(),
                contributor: ComponentPath("root/plot[99]".to_string()),
                predicate: SqlPredicate::Expr("x > 2".to_string()),
            })
            .expect("re-paint after brush");
        assert!(after.width > 0 && after.height > 0, "re-paint has area");
        assert_eq!(
            dash.coordinator().generation(),
            1,
            "the interaction advanced the materialisation generation"
        );

        // The re-composite drew from a DuckDB-filtered batch: 2 rows kept, and
        // no Rust-side path filtered a materialised batch.
        let rows: usize = dash
            .coordinator()
            .chart_rows(0)
            .expect("chart rows")
            .iter()
            .map(RecordBatch::num_rows)
            .sum();
        assert_eq!(rows, 2, "brush kept x in {{3,4}} via a pushed predicate");
    }

    /// The same seam, driven with the STRUCTURED clause the chart gestures
    /// prefer: a `Predicate::Interval` keeps exactly the rows its hand-written
    /// string form would — the variants render byte-identical SQL — while the
    /// column and bounds stay machine-readable end to end.
    #[test]
    fn a_structured_interval_selects_the_same_rows_as_its_string_form() {
        use brightfield_sql::ir::ScalarValue;
        let mut dash = LiveDashboard::load_str(SPEC, None).expect("load");
        let _ = dash.present().expect("first paint");

        let interval = SqlPredicate::Interval {
            column: "x".to_string(),
            lo: ScalarValue::Float(2.0),
            hi: ScalarValue::Float(3.0),
            meta: None,
        };
        assert_eq!(
            interval.to_string(),
            "(x >= 2 AND x <= 3)",
            "the structured clause renders exactly the string form"
        );
        let after = dash
            .apply(Interaction::Select {
                name: "brush".to_string(),
                contributor: ComponentPath("root/plot[99]".to_string()),
                predicate: interval,
            })
            .expect("re-paint after structured brush");
        assert!(after.width > 0 && after.height > 0);
        let rows: usize = dash
            .coordinator()
            .chart_rows(0)
            .expect("chart rows")
            .iter()
            .map(RecordBatch::num_rows)
            .sum();
        assert_eq!(rows, 2, "the interval kept x in {{2,3}} in DuckDB");
    }

    /// A live-queried dashboard makes no currency claim — its queries ran for
    /// this very composition, so there is no materialised run output whose
    /// staleness could be misrepresented, and no banner is owed.
    #[test]
    fn a_live_queried_preview_makes_no_run_state_claim() {
        let mut dash = LiveDashboard::load_str(SPEC, None).expect("load");
        let composed = dash.present().expect("paint");
        assert_eq!(composed.run_state, None);
        assert_eq!(composed.run_state_line(), None);
        assert_eq!(Composed::empty().run_state, None, "empty claims nothing");
    }

    /// A preview annotated with run output's state renders that state's own
    /// words: a stale annotation can never produce the fresh line, and a
    /// never-run annotation is not the fresh line either — the preview
    /// surface cannot show materialised data as though it were current.
    #[test]
    fn an_annotated_preview_is_labelled_not_merely_rendered() {
        let stale = Composed::empty().with_run_state(RunState::StaleUpstream);
        let line = stale.run_state_line().expect("an annotated preview labels");
        assert!(line.contains("stale"), "the stale line says stale: {line}");
        assert!(
            !stale.run_state.expect("annotated").is_current(),
            "a stale preview may never claim current"
        );

        let fresh_line = Composed::empty()
            .with_run_state(RunState::Fresh)
            .run_state_line()
            .expect("labelled");
        let never_line = Composed::empty()
            .with_run_state(RunState::NeverRun)
            .run_state_line()
            .expect("labelled");
        assert_ne!(line, fresh_line, "stale and fresh are different words");
        assert_ne!(
            never_line, fresh_line,
            "never-run is visibly distinct from fresh"
        );
    }
}
