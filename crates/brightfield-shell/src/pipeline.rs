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
//! golden `dashboard.yaml` and the simple examples use none of them). Each mark
//! draws EVERY materialised chunk — its result batches are assembled into one
//! drawable batch via [`assemble_batches`], so a row-per-mark chart wider than a
//! single ~2048-row chunk draws all its rows, and an assembly that cannot
//! proceed fails loudly by name rather than silently drawing the first chunk.

use std::path::{Path, PathBuf};

use arrow::record_batch::RecordBatch;
use brightfield_conformance::LoadDiagnostics;
use brightfield_engine::coordinator::{Coordinator, Interaction};
use brightfield_engine::error::EngineError;
use brightfield_engine::facts::MarkFacts;
use brightfield_engine::{assemble_batches, Engine, Session};
use brightfield_render::channel::ChannelMap;
use brightfield_render::inset::{resolve_insets_for_marks, DEFAULT_SCALE_INSET};
use brightfield_render::layout::{ChartLayout, Margins};
use brightfield_render::mark::{default_renderers, find_renderer, MarkRenderer};
use brightfield_render::sample_notice::{sample_band_margins, SampleFact};
use brightfield_render::scale::ScaleSet;
use brightfield_render::scene::{
    build_multi_mark_scene_with_domains, compose_dashboard, ChartData, UnsampledDomains,
};
use brightfield_render::{grow_margins, resolve_titles};
use brightfield_spec::analysis::{
    analyse_spec, build_brushable_bindings, BrushKind, ComponentPath,
};
use brightfield_spec::layout::{collect_plot_nodes, placed_plots, resolve_plot_insets, Rect};
use brightfield_spec::vocab::MarkKind;
use brightfield_spec::{parse_spec, parse_spec_path, Format, ParseOutput, Spec};
use brightfield_sql::ir::SampleRate;
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
    /// `Some` when this plot drew a pushed-down sample — the mirror of
    /// [`ChartData::sample`](brightfield_render::scene::ChartData::sample), so a
    /// surface reading plot handles (chrome, a future export caption) can tell
    /// a sampled plot from a complete one without re-deriving it.
    pub sample: Option<SampleFact>,
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
    /// What the load of this spec had to SAY: the parts of it brightfield
    /// cannot draw, and every warning the parse and the analysis produced.
    ///
    /// It rides on the composition because the composition is what the window
    /// receives, and a diagnostic that arrives separately from the picture it
    /// is about is a diagnostic that arrives at the wrong time. Empty for a
    /// spec that renders whole — which is most of them, and is why a surface
    /// can draw this unconditionally without becoming noise.
    pub diagnostics: LoadDiagnostics,
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
            diagnostics: LoadDiagnostics::default(),
            run_state: None,
        }
    }

    /// Attach what the load of this spec had to say. Consumes and returns
    /// `self` for the reason [`Composed::with_run_state`] does: the
    /// attachment happens at the compose call site rather than as a mutation
    /// something else can forget to make.
    #[must_use]
    pub fn with_diagnostics(mut self, diagnostics: LoadDiagnostics) -> Self {
        self.diagnostics = diagnostics;
        self
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
    compose_spec_sampled(spec_path, None)
}

/// [`compose_spec`] at an explicit pushed-down sample rate.
///
/// `None` is [`compose_spec`] exactly. `Some(rate)` makes every row-level mark
/// draw one row in `rate.modulus()` and say so in its own ink — the switch
/// `--force-sample` turns, so that ONE spec can produce a complete PNG and a
/// sampled PNG over the SAME rows. That comparison is the point: judging
/// whether a sampled render reads as sampled against a different, denser
/// dataset would confound the treatment with the density.
///
/// # Errors
///
/// As [`compose_spec`].
pub fn compose_spec_sampled(
    spec_path: &str,
    sample: Option<SampleRate>,
) -> Result<Composed, String> {
    let parsed = parse_spec_path(spec_path).map_err(|e| format!("parse error: {e}"))?;
    compose(
        parsed,
        Some(source_name(spec_path)),
        Path::new(spec_path).parent(),
        sample,
    )
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
    live_spec_sampled(spec_path, None)
}

/// [`live_spec`] at an explicit pushed-down sample rate — the live window's
/// half of `--force-sample`, so the window and the captures are judged at the
/// same rate rather than one of them being taken on trust.
///
/// # Errors
///
/// As [`live_spec`].
pub fn live_spec_sampled(
    spec_path: &str,
    sample: Option<SampleRate>,
) -> Result<(LiveDashboard, Composed), String> {
    let parsed = parse_spec_path(spec_path).map_err(|e| format!("parse error: {e}"))?;
    let mut dash = LiveDashboard::load_parsed(
        parsed,
        Some(source_name(spec_path)),
        Path::new(spec_path).parent(),
    )?;
    dash.set_sample(sample);
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
    compose(parsed, None, base_dir, None)
}

/// The name a diagnostic cites for a spec loaded from a path: its file name,
/// which is what the reader has open, not the absolute path they did not type.
fn source_name(spec_path: &str) -> String {
    Path::new(spec_path).file_name().map_or_else(
        || spec_path.to_string(),
        |n| n.to_string_lossy().to_string(),
    )
}

/// Everything after the parse, shared by both entry points above.
///
/// Takes the whole [`ParseOutput`] rather than `.spec` out of it, and that is
/// the point: the previous signature took a `Spec`, so `.warnings` had to be
/// dropped at each call site to satisfy it, and all four of them did. A
/// function that cannot be called without the warnings is the only version of
/// this that stays fixed.
fn compose(
    parsed: ParseOutput,
    source: Option<String>,
    spec_dir: Option<&Path>,
    sample: Option<SampleRate>,
) -> Result<Composed, String> {
    let spec = parsed.spec;
    let analysis = analyse_spec(&spec).map_err(|e| format!("analysis error: {e}"))?;
    let diagnostics = LoadDiagnostics::collect(source, &spec, &parsed.warnings, &analysis.warnings);

    let engine = Engine::new();
    let load = engine
        .load_spec(spec.clone(), analysis, spec_dir)
        .map_err(|e| format!("engine error: {e}"))?;
    let mut session = load.session;
    session.set_sample(sample);

    // Execute every mark; assemble all its result chunks into one drawable batch.
    let results = session.execute_all();
    let facts = unsampled_facts(&session, results.len());
    Ok(compose_from_results(&spec, results, &facts)?.with_diagnostics(diagnostics))
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
    diagnostics: LoadDiagnostics,
}

impl LiveDashboard {
    /// Load a spec and hold its session live for interaction. `spec_dir` is
    /// where relative `file:` paths resolve (`None` for inline-data specs).
    ///
    /// The caller here holds an already-parsed [`Spec`] and therefore no
    /// parse warnings, so the diagnostics this dashboard reports cover the
    /// preflight walk and the analysis only. A caller that still has its
    /// `ParseOutput` should use [`LiveDashboard::load_parsed`] and keep the
    /// whole picture.
    ///
    /// # Errors
    ///
    /// As [`compose_spec`]: a human-readable message on analyse / load failure.
    pub fn load(spec: Spec, spec_dir: Option<&Path>) -> Result<Self, String> {
        let analysis = analyse_spec(&spec).map_err(|e| format!("analysis error: {e}"))?;
        let diagnostics = LoadDiagnostics::collect(None, &spec, &[], &analysis.warnings);
        let coordinator = Coordinator::load(spec.clone(), analysis, spec_dir)
            .map_err(|e| format!("engine error: {e}"))?;
        Ok(Self {
            coordinator,
            spec,
            diagnostics,
        })
    }

    /// Load from a whole [`ParseOutput`], keeping its warnings.
    ///
    /// # Errors
    ///
    /// As [`LiveDashboard::load`].
    pub fn load_parsed(
        parsed: ParseOutput,
        source: Option<String>,
        spec_dir: Option<&Path>,
    ) -> Result<Self, String> {
        let spec = parsed.spec;
        let analysis = analyse_spec(&spec).map_err(|e| format!("analysis error: {e}"))?;
        let diagnostics =
            LoadDiagnostics::collect(source, &spec, &parsed.warnings, &analysis.warnings);
        let coordinator = Coordinator::load(spec.clone(), analysis, spec_dir)
            .map_err(|e| format!("engine error: {e}"))?;
        Ok(Self {
            coordinator,
            spec,
            diagnostics,
        })
    }

    /// Load from spec text (mirrors [`compose_spec_str`]).
    ///
    /// # Errors
    ///
    /// As [`LiveDashboard::load`].
    pub fn load_str(source: &str, spec_dir: Option<&Path>) -> Result<Self, String> {
        let parsed = parse_spec(source, Format::Yaml).map_err(|e| format!("parse error: {e}"))?;
        Self::load_parsed(parsed, None, spec_dir)
    }

    /// What this dashboard's load had to say.
    #[must_use]
    pub fn diagnostics(&self) -> &LoadDiagnostics {
        &self.diagnostics
    }

    /// Composite the CURRENT materialisation into a dashboard scene — the first
    /// paint and every post-interaction re-paint go through here.
    ///
    /// Every composite carries the load's diagnostics, including the ones
    /// after an interaction: the spec did not become renderable because
    /// someone dragged a brush, so a re-present that dropped the warnings
    /// would let a single gesture silence them.
    ///
    /// # Errors
    ///
    /// As [`compose_spec`] (returns `Err` when nothing renders).
    pub fn present(&mut self) -> Result<Composed, String> {
        let results = self.coordinator.session_mut().execute_all();
        // Gathered on the SAME path as the first paint, not only there: a
        // re-present after a brush that dropped the facts would erase the
        // notice on the first gesture and leave a sampled picture looking
        // complete.
        let facts = unsampled_facts(self.coordinator.session_mut(), results.len());
        Ok(compose_from_results(&self.spec, results, &facts)?
            .with_diagnostics(self.diagnostics.clone()))
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

    /// Set the pushed-down sample rate every later query carries — including
    /// every re-query a brush, a click or a slider triggers, which is the
    /// point of holding it on the session rather than on the call.
    pub fn set_sample(&mut self, sample: Option<SampleRate>) {
        self.coordinator.session_mut().set_sample(sample);
    }

    /// The live coordinator, for surfaces that read the session directly (a grid
    /// at a step, distinct-value option lists) or hold the interrupt handle.
    pub fn coordinator(&mut self) -> &mut Coordinator {
        &mut self.coordinator
    }

    /// The local files this dashboard's spec reads through `file:` data
    /// sources — see [`spec_data_files`], which this delegates to over the
    /// held spec.
    #[must_use]
    pub fn data_files(&self, spec_dir: Option<&Path>) -> Vec<PathBuf> {
        spec_data_files(&self.spec, spec_dir)
    }
}

/// The local files `spec` reads through `file:` data sources, resolved
/// against `spec_dir` — the list the document's file watcher watches. URLs
/// are not files and are skipped; whether a listed path exists is the
/// watcher's business (a file appearing where the spec expects one is a
/// change worth noticing).
#[must_use]
pub fn spec_data_files(spec: &Spec, spec_dir: Option<&Path>) -> Vec<PathBuf> {
    use brightfield_spec::ast::DataSourceKind;
    spec.data
        .values()
        .filter_map(|source| match &source.kind {
            DataSourceKind::File(f) if !f.contains("://") => Some(match spec_dir {
                Some(dir) => dir.join(f),
                None => PathBuf::from(f),
            }),
            _ => None,
        })
        .collect()
}

/// Build the composited dashboard from a spec and its per-mark execution
/// results. Shared by the one-shot [`compose`] path and the live
/// [`LiveDashboard`] re-query seam, so a re-composite after an interaction takes
/// the identical layout and scene path as the first paint.

/// The unsampled facts for every mark, or `None` per mark when the session is
/// not sampling.
///
/// One query per SAMPLED mark and none at all otherwise, which is what keeps
/// an unsampled chart's query count byte-unchanged by this feature's
/// existence.
fn unsampled_facts(session: &Session, marks: usize) -> Vec<Option<MarkFacts>> {
    (0..marks)
        .map(|i| match session.unsampled_mark_facts(i) {
            None => None,
            Some(Ok(f)) => Some(f),
            Some(Err(e)) => {
                // A failed facts query must not take the picture down with it:
                // the chart still draws, it just cannot say what it is a
                // sample of, and says so by not claiming.
                eprintln!("warning: unsampled facts for mark {i}: {e}");
                None
            }
        })
        .collect()
}

fn compose_from_results(
    spec: &Spec,
    results: Vec<Result<Vec<RecordBatch>, EngineError>>,
    facts: &[Option<MarkFacts>],
) -> Result<Composed, String> {
    let marks = collect_marks(spec);
    let mut batches: Vec<Option<RecordBatch>> = Vec::with_capacity(marks.len());
    let mut channel_maps: Vec<ChannelMap> = Vec::with_capacity(marks.len());
    let mut kinds = Vec::with_capacity(marks.len());
    for (i, result) in results.into_iter().enumerate() {
        // Assemble EVERY materialised chunk into the one batch this mark draws,
        // not just the first ~2048-row chunk. A row-per-mark chart wider than one
        // chunk must draw all its rows; taking `bs.into_iter().next()` silently
        // dropped the rest. Assembly failure is a loud, NAMED error, surfaced
        // here rather than masked by drawing a partial batch.
        let batch = match result {
            Ok(bs) => assemble_batches(bs).map_err(|e| format!("mark {i}: {e}"))?,
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
        let mut plot_domains = UnsampledDomains::default();
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
            // A mark is sampled exactly when the session produced unsampled
            // facts for it; `drawn` is what actually arrived, `of` is what the
            // same query returns without the clause.
            let sample = facts.get(mi).copied().flatten().map(|f| SampleFact {
                drawn: batch.num_rows() as u64,
                of: f.rows,
            });
            if let Some(f) = facts.get(mi).copied().flatten() {
                plot_domains.x = plot_domains.x.or(f.x_domain);
                plot_domains.y = plot_domains.y.or(f.y_domain);
            }
            chart_data.push(ChartData {
                batch,
                channel_map: &channel_maps[mi],
                renderer,
                layout: ChartLayout::new(plot.rect.width, plot.rect.height),
                view_extent: None,
                highlight: None,
                sample,
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

        // Two bands, reserved the same way: one for the axis titles, one for
        // the sampling notice. Growing the margin is what makes the device
        // removable later without disturbing anything else's geometry.
        let plot_sample = chart_data.iter().find_map(|d| d.sample);
        let margins = sample_band_margins(
            grow_margins(Margins::default(), &titles),
            plot_sample.is_some(),
        );
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
        let (scene, scales) =
            build_multi_mark_scene_with_domains(&refs, false, &titles, plot_domains);
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
            sample: plot_sample,
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
        // Attached by the load path, which is the only place that holds the
        // ParseOutput. `compose_from_results` is also reached on every
        // re-present after an interaction, where re-deriving diagnostics from
        // the spec alone would silently lose the parse warnings.
        diagnostics: LoadDiagnostics::default(),
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

    // --- Drawing every materialised chunk (the row-per-mark fix) -----------

    /// A one-mark dot spec whose inline data is irrelevant — these tests feed
    /// the execution results directly, so `compose_from_results` never queries.
    const DOT_SPEC: &str = r#"
data:
  t:
    - { x: 0, y: 0 }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
"#;

    fn xy_batch(xs: Vec<f64>, ys: Vec<f64>) -> RecordBatch {
        use arrow::array::Float64Array;
        use arrow::datatypes::{DataType, Field, Schema};
        use std::sync::Arc;
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(xs)),
                Arc::new(Float64Array::from(ys)),
            ],
        )
        .unwrap()
    }

    /// Two chunks that, between them, span more than one DuckDB vector
    /// (>2048 rows). The FIRST chunk already carries the domain endpoints (0
    /// and 100 on both axes), so the inferred scales — and therefore the grid
    /// and axis ink — are byte-identical whether the composite sees only the
    /// first chunk or both. That pins every non-mark path constant, so a
    /// path-count DIFFERENCE between the two composites is exactly the extra
    /// dots drawn.
    fn two_chunk_dot_results(
        first_rows: usize,
        second_rows: usize,
    ) -> (Vec<Result<Vec<RecordBatch>, EngineError>>, usize) {
        let first_x: Vec<f64> = (0..first_rows).map(|i| (i % 101) as f64).collect();
        let first_y: Vec<f64> = (0..first_rows).map(|i| (i % 101) as f64).collect();
        let first = xy_batch(first_x, first_y);
        // Second chunk stays strictly inside [0, 100] so it cannot widen the
        // domain — every value is the midpoint.
        let second = xy_batch(vec![50.0; second_rows], vec![50.0; second_rows]);
        let total = first_rows + second_rows;
        (vec![Ok(vec![first, second])], total)
    }

    /// A row-per-mark chart over more than one ~2048-row chunk draws EVERY
    /// materialised row, not just the first chunk. Proven two ways: the whole
    /// materialisation composites to strictly more mark ink than the first
    /// chunk alone, by exactly the dropped-row count; and chunking is
    /// invisible — one 3052-row batch draws identically to 2000 + 1052.
    #[test]
    fn a_chart_over_2048_rows_draws_every_materialised_row() {
        use brightfield_render::mark::count_scene_paths;
        let spec = parse_spec(DOT_SPEC, Format::Yaml).expect("parse").spec;

        let first_rows = 2000;
        let second_rows = 1052;
        let materialised = first_rows + second_rows;
        assert!(
            materialised > 2048,
            "the fixture must exceed one DuckDB vector"
        );

        // The whole materialisation vs the first chunk only (what the old code
        // drew). Domains are pinned equal, so the frame ink is identical and
        // the path-count delta is purely the extra dots.
        let (full_results, total) = two_chunk_dot_results(first_rows, second_rows);
        assert_eq!(total, materialised);
        let full = compose_from_results(&spec, full_results, &[]).expect("compose full");

        let first_only: Vec<Result<Vec<RecordBatch>, EngineError>> = vec![Ok(vec![xy_batch(
            (0..first_rows).map(|i| (i % 101) as f64).collect(),
            (0..first_rows).map(|i| (i % 101) as f64).collect(),
        )])];
        let first = compose_from_results(&spec, first_only, &[]).expect("compose first");

        let drawn_delta = count_scene_paths(&full.scene) - count_scene_paths(&first.scene);
        assert_eq!(
            drawn_delta, second_rows,
            "the composite drew exactly the rows the first-chunk cap used to drop"
        );

        // Chunking is invisible: one batch of all rows draws the same as two.
        let single: Vec<Result<Vec<RecordBatch>, EngineError>> = vec![Ok(vec![xy_batch(
            (0..materialised).map(|i| (i % 101) as f64).collect(),
            (0..materialised).map(|i| (i % 101) as f64).collect(),
        )])];
        let single = compose_from_results(&spec, single, &[]).expect("compose single");
        assert_eq!(
            count_scene_paths(&full.scene),
            count_scene_paths(&single.scene),
            "a chart draws the same whether its rows arrive in one chunk or many"
        );
    }

    /// Hitting a real assembly limit — a mark's chunks whose schemas disagree —
    /// fails the compose loudly, by NAME, naming the mark, instead of silently
    /// drawing only the first chunk.
    #[test]
    fn an_unassemblable_mark_fails_loudly_by_name() {
        use arrow::array::{Float64Array, Int64Array};
        use arrow::datatypes::{DataType, Field, Schema};
        use std::sync::Arc;
        let parsed = parse_spec(DOT_SPEC, Format::Yaml).expect("parse");
        let spec = parsed.spec;

        let int_schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Int64, false),
            Field::new("y", DataType::Int64, false),
        ]));
        let a = RecordBatch::try_new(
            int_schema,
            vec![
                Arc::new(Int64Array::from(vec![1_i64, 2])),
                Arc::new(Int64Array::from(vec![1_i64, 2])),
            ],
        )
        .unwrap();
        // Same column names, a different x type — the chunks cannot concatenate.
        let drift_schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Int64, false),
        ]));
        let b = RecordBatch::try_new(
            drift_schema,
            vec![
                Arc::new(Float64Array::from(vec![3.0_f64])),
                Arc::new(Int64Array::from(vec![3_i64])),
            ],
        )
        .unwrap();

        let results: Vec<Result<Vec<RecordBatch>, EngineError>> = vec![Ok(vec![a, b])];
        // `.err()` rather than `.expect_err()` — `Composed` is deliberately not
        // `Debug` (it holds a Vello scene), so we inspect the error side directly.
        let err = compose_from_results(&spec, results, &[])
            .err()
            .expect("must fail loudly");
        assert!(err.contains("mark 0"), "the failure names the mark: {err}");
        assert!(
            err.contains("batch-assembly limit"),
            "the failure names the limit: {err}"
        );
    }
}
