//! Live cross-filter coordinator.
//!
//! Keeps the engine [`Session`] and the per-mark / per-plot render metadata
//! alive past the initial render, so a brush committed in the window
//! re-executes the subscriber marks and swaps their scenes in place. The window
//! holds one coordinator wrapped in `Rc<RefCell<…>>`; each plot's
//! mouse-up handler calls [`CrossfilterCoordinator::commit_brush`].
//!
//! The chain — all but the final `set_scene` is exercised headlessly by
//! `tests/crossfilter_integration.rs` and the inversion is unit-tested below:
//!
//! ```text
//! pixel brush rect
//!   → invert to data coords via the plot's ScaleSet   (invert_pixel_brush)
//!     → commit_brush_release_multi into the live Session
//!       → re-executed subscriber batches
//!         → rebuild each affected plot's scene          (build_plot_scene)
//!           → set_scene on its ChartState
//! ```

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use kurbo::{Point, Rect};
use vello::Scene;

use brightfield_engine::DispatchResult;
use brightfield_engine::{concat_batches, RecordBatch, Session};
use brightfield_render::channel::{Channel, ChannelMap};
use brightfield_render::layout::ChartLayout;
use brightfield_render::mark::{
    build_highlight_state, configured_renderer, default_renderers, find_renderer, HighlightState,
    HighlightStyle, MarkRenderer, Projection,
};
use brightfield_render::nearest::SelectionValue;
use brightfield_render::scale::{Scale, ScaleSet, SequentialScheme};
use brightfield_render::scene::{
    build_multi_mark_scene, build_multi_mark_scene_anchored, ChartData,
};
use brightfield_render::title::ResolvedTitles;
use brightfield_spec::analysis::{
    mark_honours_highlight, ComponentPath, HighlightBinding, LegendBinding, SpecAnalysis,
};
use brightfield_spec::ast::{Component, Mark, PlotNode, Spec, SpecValue, ValueOrParamRef};
use brightfield_spec::edit::ChartEdit;
use brightfield_spec::layout::{collect_plot_nodes, resolve_projection};
use brightfield_spec::vocab::MarkKind;
use brightfield_sql::ir::{ClauseMeta, Predicate, ScaleDescriptor};

use crate::brush::{
    brush_rect_to_structured, commit_brush_release_multi_structured, commit_click_multi,
    point_predicate, BrushBinding, SelectionDispatcher, ZERO_AREA_EPSILON,
};
use crate::interaction::InteractionState;
use crate::menu::{commit_menu_release, MenuBinding, MenuState};
use crate::reactive::ReactiveHandle;
use crate::slider::{commit_slider_release, SliderBinding, SliderState};

/// Build the predicate for a legend's selected category set (legend
/// multi-select): a bare `Predicate::Expr` for a single category — byte-identical
/// to a single-select point predicate, so it shares the statement cache — or a
/// `Predicate::Or` of the per-category equalities for a union. `categories` MUST
/// be non-empty and canonically ordered so identical sets emit identical SQL.
fn legend_union_predicate(column: &str, categories: &[String]) -> Predicate {
    let mut members: Vec<Predicate> = categories
        .iter()
        .map(|cat| point_predicate(column, &SelectionValue::Text(cat.clone()).literal()))
        .collect();
    if members.len() == 1 {
        members.pop().expect("non-empty by contract")
    } else {
        Predicate::Or(members)
    }
}

/// Decompose a contributor-slot predicate into the OR members to test for
/// legend-category membership: an `Or` yields its members; anything else (a bare
/// `Expr`, a brush `And`, `True`) is a single opaque member that matches no
/// category point predicate — so a foreign slot contributes no selected
/// categories (the F1 display rule), and the shift-toggle build reuses this so
/// it can never fold a foreign predicate into the union.
fn predicate_members(pred: &Predicate) -> Vec<&Predicate> {
    match pred {
        Predicate::Or(members) => members.iter().collect(),
        other => vec![other],
    }
}

/// Per-mark render inputs, kept mutable so a re-executed subscriber's batch can
/// be swapped in before its plot's scene is rebuilt. `batch` is `None` for a
/// mark that failed initial execution or whose filtered result has no rows.
pub struct MarkInput {
    /// Latest data for the mark (initial execution, then cross-filter results).
    pub batch: Option<RecordBatch>,
    /// Encoding channels → column names.
    pub channels: ChannelMap,
    /// Mark kind (selects the renderer).
    pub kind: MarkKind,
    /// The mark's scheme/attribute-configured renderer, built ONCE during app
    /// assembly (`configured_renderer` from the owning plot's `colorScheme`
    /// plus the mark's `bandwidth`/`thresholds`), or `None` for a mark that
    /// renders through the shared registry. The SAME override the first render
    /// used drives every live rebuild, so a heatmap/cell/raster keeps its
    /// scheme and a heatmap/contour keeps its bandwidth/thresholds across a
    /// gesture (renderer seam). Render-only: no SQL / plan-hash
    /// involvement.
    pub renderer_override: Option<Box<dyn MarkRenderer + Send + Sync>>,
    /// Retained render-config inputs, parallel to `renderer_override`: the
    /// heatmap/contour KDE bandwidth, contour iso-level count, and hexgrid bin
    /// width computed once at app assembly. `cycle_scheme`
    /// rebuilds the override through `configured_renderer` with these, so a
    /// transient colour cycle re-colours WITHOUT reshaping the density surface,
    /// iso-lines, or hex mesh. `None` for a mark that carries none.
    pub bandwidth: Option<f64>,
    /// Contour iso-level count (see `bandwidth`).
    pub thresholds: Option<usize>,
    /// Hexgrid bin width (see `bandwidth`).
    pub bin_width: Option<f64>,
    /// Resolved `otherwise` deemphasis style when this mark's plot carries a
    /// `highlight, by: $sel` interactor AND the mark honours highlight
    /// (dot/bar/rect/text) — `None` otherwise. When present, a
    /// re-queried batch carrying the reserved `__bf_selected` boolean drives a
    /// `HighlightState` that dims the non-matching rows; an at-rest batch (empty
    /// selection → no `__bf_selected` column) renders every row normally.
    pub highlight_style: Option<HighlightStyle>,
}

/// One plot in the live dashboard: its identity, the marks it owns, its layout,
/// the brushes it contributes, its data scales (for inversion), and its state.
/// `H` is the host's [`ReactiveHandle`] to the plot's [`ChartState`](crate::chart_state::ChartState) cell.
pub struct LivePlot<H> {
    /// Stable component path (diagnostics / matching).
    pub path: String,
    /// Flat indices (into the coordinator's `marks`) of this plot's marks.
    pub mark_indices: Vec<usize>,
    /// Layout (size + margins).
    pub layout: ChartLayout,
    /// Brush bindings this plot contributes (its `intervalX/Y/XY` interactors).
    pub bindings: Vec<BrushBinding>,
    /// The plot's DISPLAYED ScaleSet — the launch-anchored scales the last
    /// rebuild rendered against. `build_plot_scene` re-infers fresh scales each
    /// gesture and folds them against `launch_scales` (widen-only), storing the
    /// result here; pixel-brush and click inversion read it, so inversion always
    /// matches what is on screen. At launch state (and under any subset filter)
    /// it equals `launch_scales` exactly (render fidelity).
    pub scales: ScaleSet,
    /// The plot's LAUNCH ScaleSet, captured when the coordinator was built and
    /// immutable thereafter — the widen-only anchor every rebuild folds the
    /// fresh inference against, so axes / colours / ramp anchoring hold still
    /// while a query-rewrite gesture can still widen the domain to keep new rows
    /// on-plot (F1). Never re-inferred: "launch" means "pinned at window launch".
    pub launch_scales: ScaleSet,
    /// The plot's CURRENT sequential colour scheme — the transient colour-cycle's
    /// running state. Seeded from the launch scheme; `c`
    /// advances it and `cycle_scheme` recolours `launch_scales`' Fill ramp
    /// accordingly. "Transient" = never written to the spec — NOT "reverts on
    /// reload": a hot reload swaps in a fresh scene but never rebuilds the
    /// coordinator (the known coordinator-staleness-on-reload limitation), so a
    /// cycled scheme persists here in-session until restart. `launch_scales` stays
    /// the widen-only render anchor.
    pub scheme: SequentialScheme,
    /// Whether this plot draws its own inline (top-right) colour legend. `false`
    /// when a standalone `legend: color for:` node has relocated it — resolved at
    /// the app layer and carried here so a live re-render honours the same
    /// suppression instead of resurrecting the inline legend.
    pub draw_inline_legend: bool,
    /// The plot's resolved axis + plot titles. Rides here — NOT the
    /// `Copy` `ChartLayout` (Strings would break `Copy`) — so every launch-
    /// anchored live rebuild re-emits the same titles the first render drew. The
    /// grown title margins are baked into `layout` (and the `ChartState`).
    pub titles: ResolvedTitles,
    /// The host's reactive handle to this plot's [`ChartState`](crate::chart_state::ChartState) — the cell
    /// whose scene we swap when this plot is re-filtered (see
    /// [`ReactiveHandle`]).
    pub state: H,
}

/// A legend's selection-producer binding, UI-side — the mirror of
/// the spec-side [`LegendBinding`], carrying what a swatch click dispatches:
/// the selection it writes, the contributor identity (the `for:` plot's node
/// path, giving self-exclusion by construction), and the colour column the
/// clicked category compares against.
#[derive(Debug, Clone)]
pub struct LegendSelectBinding {
    /// Name of the selection this legend contributes to (e.g. `sel`).
    pub selection_name: String,
    /// The `for:` plot's node path (for self-exclusion).
    pub contributor: ComponentPath,
    /// The colour column of the `for:` plot's colour encoding.
    pub column: String,
}

/// Convert a spec-side [`LegendBinding`] into a UI-side
/// [`LegendSelectBinding`]. Faithful field copy, mirroring
/// `BrushBinding::from(&BrushableBinding)`.
impl From<&LegendBinding> for LegendSelectBinding {
    fn from(b: &LegendBinding) -> Self {
        LegendSelectBinding {
            selection_name: b.selection.clone(),
            contributor: b.plot_path.clone(),
            column: b.colour_column.clone(),
        }
    }
}

/// Coordinates live cross-filtering (brushes), reactive params (sliders), and
/// legend point selections (swatch clicks) across a dashboard's
/// plots. All gestures re-execute subscriber marks through the same live
/// `Session` and rebuild only the affected plot scenes.
///
/// Generic over the host's [`ReactiveHandle`] `H` — the ONLY host-facing
/// thing the coordinator does is swap a rebuilt scene into a plot's
/// [`ChartState`](crate::chart_state::ChartState) cell and ask for a repaint, so the whole engine/render
/// chain here is host-free and a new shell adopts it by implementing that
/// one trait.
pub struct CrossfilterCoordinator<H: ReactiveHandle> {
    session: Session,
    marks: Vec<MarkInput>,
    plots: Vec<LivePlot<H>>,
    /// Dashboard-level slider bindings, indexed by hosted slider.
    /// A slider's subscribers may span multiple plots; the affected plots are
    /// resolved generically via `mark_to_plot` from the re-executed mark indices.
    slider_bindings: Vec<SliderBinding>,
    /// Dashboard-level menu-family bindings, indexed by hosted
    /// widget — the additive parallel lane beside `slider_bindings`. Options
    /// are resolved + reconciled at assembly (launch-fixed, slider parity).
    menu_bindings: Vec<MenuBinding>,
    /// Dashboard-level legend producer bindings, indexed by the
    /// bound legend's position in the analysis binding list — the index a
    /// hosted `LegendElement` carries. Toggle state is NOT mirrored here: the
    /// engine's `(selection, contributor)` slot is shared with the plot's
    /// brush/point interactors, so the toggle decision reads the slot itself
    /// via [`Session::contributor_predicate`] (a mirror desynchronises the
    /// moment any other gesture writes the slot).
    legend_bindings: Vec<LegendSelectBinding>,
    /// flat mark index → owning plot index (into `plots`).
    mark_to_plot: HashMap<usize, usize>,
    renderers: Vec<(MarkKind, Box<dyn MarkRenderer + Send + Sync>)>,
}

impl<H: ReactiveHandle> CrossfilterCoordinator<H> {
    /// Build a coordinator from the live engine session and the per-mark /
    /// per-plot / slider / menu / legend metadata assembled at startup. Returns
    /// `None` when there is nothing live to drive — no plot has a brush binding
    /// AND there are no sliders AND no menu widgets AND no bound legends AND no
    /// plot carries a sequential Fill ramp AND the editing grammar is not
    /// active — so a purely static PNG/embedding path skips the wiring entirely
    /// and behaves as before.
    /// A dashboard whose only interactive surface is a bound legend,
    /// or whose only live surface is a sequential colour scale (the `c`
    /// colour-cycle), stays live. `editing_active` is
    /// itself a live surface: the authoring window passes `true` so a structural
    /// edit (m/a/e/d/u) can re-render even an otherwise-static plot — without it
    /// a plain scatter's edits mutate the working spec but never re-render the
    /// canvas (the silent-no-op class). Keeping the
    /// coordinator costs only the retained non-Send Session in the window; the
    /// dump/watcher paths never build one and pass `false`.
    pub fn new(
        session: Session,
        marks: Vec<MarkInput>,
        plots: Vec<LivePlot<H>>,
        slider_bindings: Vec<SliderBinding>,
        menu_bindings: Vec<MenuBinding>,
        legend_bindings: Vec<LegendSelectBinding>,
        editing_active: bool,
    ) -> Option<Rc<RefCell<Self>>> {
        let any_brush = plots.iter().any(|p| !p.bindings.is_empty());
        let any_sequential_fill = plots.iter().any(|p| has_sequential_fill(&p.launch_scales));
        if !coordinator_has_live_surface(
            any_brush,
            !slider_bindings.is_empty(),
            !menu_bindings.is_empty(),
            !legend_bindings.is_empty(),
            any_sequential_fill,
            editing_active,
        ) {
            return None;
        }
        let mut mark_to_plot = HashMap::new();
        for (pi, plot) in plots.iter().enumerate() {
            for &mi in &plot.mark_indices {
                mark_to_plot.insert(mi, pi);
            }
        }
        Some(Rc::new(RefCell::new(Self {
            session,
            marks,
            plots,
            slider_bindings,
            menu_bindings,
            legend_bindings,
            mark_to_plot,
            renderers: default_renderers(),
        })))
    }

    /// Commit a brush gesture from plot `plot_index`: dispatch the (data-space)
    /// selection into the engine, then rebuild and swap the scenes of every plot
    /// whose marks re-executed. A click (zero-area gesture) on this plot clears
    /// its contributions instead.
    ///
    /// Only the plot actually gestured on responds. The mouse-up listener is
    /// window-level, so EVERY plot's listener fires on each release; a sibling
    /// whose interaction is `Idle` (it wasn't touched) must do nothing — else a
    /// release would wrongly clear every other plot's selection. So a
    /// non-`Brushing` interaction returns immediately.
    ///
    /// Returns `true` if this plot handled the gesture (the caller then refreshes
    /// the window once, repainting the swapped subscriber scenes); `false` for an
    /// untouched sibling, so siblings don't each trigger a redundant refresh.
    pub fn commit_brush(
        &mut self,
        plot_index: usize,
        interaction: &InteractionState,
        cx: &mut H::Cx,
    ) -> bool {
        let (start, current) = match interaction {
            InteractionState::Brushing { start, current } => (*start, *current),
            // Idle / Hovering: this plot wasn't the gesture target — do nothing.
            _ => return false,
        };
        let bindings = match self.plots.get(plot_index) {
            Some(p) if !p.bindings.is_empty() => p.bindings.clone(),
            _ => return false,
        };

        // Click vs drag is judged in pixels (matches commit_brush_clear).
        let is_drag = (start.x - current.x).abs() >= ZERO_AREA_EPSILON
            || (start.y - current.y).abs() >= ZERO_AREA_EPSILON;

        let mut to_rebuild: HashSet<usize> = HashSet::new();
        if is_drag {
            // Invert the pixel rect to data coordinates and dispatch as
            // STRUCTURED interval clauses (the live default) carrying each
            // axis's scale metadata — the machine-readable form the engine's
            // automatic pre-aggregation layer derives its cube from. Reuse the
            // shared helper by synthesising a Brushing state already in data
            // space.
            let data_rect = invert_pixel_brush(start, current, &self.plots[plot_index].scales);
            let x_meta = self.plots[plot_index]
                .scales
                .get(Channel::X)
                .map(clause_meta_for_scale);
            let y_meta = self.plots[plot_index]
                .scales
                .get(Channel::Y)
                .map(clause_meta_for_scale);
            let synthetic = InteractionState::Brushing {
                start: Point::new(data_rect.x0, data_rect.y0),
                current: Point::new(data_rect.x1, data_rect.y1),
            };
            let (_next, aggregated) = commit_brush_release_multi_structured(
                &synthetic,
                &bindings,
                x_meta,
                y_meta,
                &mut self.session,
            );
            for (_selection, results) in aggregated {
                self.absorb(results, &mut to_rebuild);
            }
        } else {
            // A click on this plot. Point selections (toggleX/Y) snap to the
            // nearest datum and dispatch its exact value; a click on empty space,
            // and any interval selection, clears — retracting this plot's
            // contribution so subscribers re-execute back toward unfiltered.
            // `start` is the click in element-local pixels (== `current`); the
            // plot's marks + scales let the shared helper resolve the datum.
            let aggregated = {
                let marks: Vec<(&RecordBatch, &ChannelMap)> = self.plots[plot_index]
                    .mark_indices
                    .iter()
                    .filter_map(|&mi| {
                        let m = self.marks.get(mi)?;
                        Some((m.batch.as_ref()?, &m.channels))
                    })
                    .collect();
                let (_next, aggregated) = commit_click_multi(
                    start,
                    &marks,
                    &self.plots[plot_index].scales,
                    &bindings,
                    &mut self.session,
                );
                aggregated
            };
            for (_selection, results) in aggregated {
                self.absorb(results, &mut to_rebuild);
            }
        }

        for pi in to_rebuild {
            let scene = self.build_plot_scene(pi);
            let state = self.plots[pi].state.clone();
            state.update(cx, |s| {
                s.set_scene(scene);
                true
            });
        }
        true
    }

    /// Clear the selection contributed by the plot at `plot_path` (keyboard Esc —
    /// retract each of its brush/point bindings, re-execute
    /// subscribers back toward unfiltered, and rebuild the affected scenes.
    /// Returns `true` if anything was actually cleared (caller then refreshes).
    pub fn clear_plot(&mut self, plot_path: &str, cx: &mut H::Cx) -> bool {
        let bindings = match self.plots.iter().find(|p| p.path == plot_path) {
            Some(p) => p.bindings.clone(),
            None => return false,
        };
        let filter = self.clear_bindings(&bindings, cx);
        // Also drop the persistent selection rectangle on the brushed plot itself.
        let overlay = self.clear_selection_overlay(plot_path, cx);
        filter || overlay
    }

    /// Clear EVERY plot's selection (Esc at the dashboard altitude).
    /// Returns `true` if anything was cleared.
    pub fn clear_all(&mut self, cx: &mut H::Cx) -> bool {
        let bindings: Vec<BrushBinding> =
            self.plots.iter().flat_map(|p| p.bindings.clone()).collect();
        let filter = self.clear_bindings(&bindings, cx);
        let paths: Vec<String> = self.plots.iter().map(|p| p.path.clone()).collect();
        let mut overlay = false;
        for path in &paths {
            if self.clear_selection_overlay(path, cx) {
                overlay = true;
            }
        }
        filter || overlay
    }

    /// Drop the persistent `Selected` rectangle on the plot at `plot_path` (the
    /// visual half of Esc-clear — the filter half is [`Self::clear_bindings`]).
    fn clear_selection_overlay(&mut self, plot_path: &str, cx: &mut H::Cx) -> bool {
        let Some(p) = self.plots.iter().find(|p| p.path == plot_path) else {
            return false;
        };
        let state = p.state.clone();
        let mut cleared = false;
        state.update(cx, |s| {
            cleared = s.clear_persistent_selection();
            cleared
        });
        cleared
    }

    /// Retract `bindings`' contributions, absorb the re-execution, and rebuild the
    /// affected scenes — the shared tail of [`Self::clear_plot`] / [`Self::clear_all`].
    fn clear_bindings(&mut self, bindings: &[BrushBinding], cx: &mut H::Cx) -> bool {
        let mut to_rebuild: HashSet<usize> = HashSet::new();
        let mut cleared = false;
        for b in bindings {
            let results = self.session.clear(&b.selection_name, b.contributor.clone());
            if !results.is_empty() {
                cleared = true;
            }
            self.absorb(results, &mut to_rebuild);
        }
        for pi in to_rebuild {
            let scene = self.build_plot_scene(pi);
            let state = self.plots[pi].state.clone();
            state.update(cx, |s| {
                s.set_scene(scene);
                true
            });
        }
        cleared
    }

    /// Cycle the focused plot's sequential colour scheme, transiently.
    /// Advances the plot's running `scheme`, rebuilds each of its marks'
    /// `renderer_override` through `configured_renderer` RETAINING bandwidth /
    /// thresholds / bin_width (so a mere colour cycle never reshapes a KDE
    /// surface, contour iso-lines, or a hex mesh), then — the load-bearing step —
    /// recolours the plot's LAUNCH Fill ramp: `anchor_scale` copies launch stops
    /// (scale.rs), so rewriting them is what actually moves the on-screen ramp; a
    /// renderer_override swap alone is inert. Finally re-renders that one plot's
    /// scene. No spec write (never durable to disk). A hot reload swaps a fresh
    /// scene but does NOT rebuild the coordinator, so the cycled scheme persists
    /// here until restart (the known coordinator-staleness-on-reload limitation).
    ///
    /// Returns `true` when the plot was cycled (the caller then refreshes the
    /// window); `false` for an unknown path, or a plot with no sequential Fill
    /// (a categorical/no-Fill plot has nothing to cycle — a clean no-op).
    pub fn cycle_scheme(&mut self, plot_path: &str, cx: &mut H::Cx) -> bool {
        let Some(pi) = self.plots.iter().position(|p| p.path == plot_path) else {
            return false;
        };
        if !has_sequential_fill(&self.plots[pi].launch_scales) {
            return false;
        }
        let next = self.plots[pi].scheme.next();
        self.plots[pi].scheme = next;

        // Rebuild each mark's override through the NEW scheme, retaining its
        // render-config attrs so only the colour changes (a mark whose kind has
        // no configured renderer — e.g. a dot overlay — keeps its override).
        for mi in self.plots[pi].mark_indices.clone() {
            if let Some(m) = self.marks.get_mut(mi) {
                if let Some(r) = recolour_override(m, next) {
                    m.renderer_override = Some(r);
                }
            }
        }

        // Load-bearing: recolour the launch Fill ramp (see the doc comment).
        recolour_fill(&mut self.plots[pi].launch_scales, next.stops());

        let scene = self.build_plot_scene(pi);
        let state = self.plots[pi].state.clone();
        state.update(cx, |s| {
            s.set_scene(scene);
            true
        });
        true
    }

    /// Apply a transient structural [`ChartEdit`] to the live coordinator
    /// — the load-bearing keyboard-edit mechanism, a DURABLE
    /// coordinator refresh (heavier than `cycle_scheme`: it re-queries).
    ///
    /// The app has already `apply`'d the edit to the working `Spec` and
    /// re-analysed; it hands the mutated `spec` + `analysis` here. This method
    /// (1) [`Session::reload_spec`]s so `execute_mark` re-emits the NEW SQL
    /// against the already-live views (no disk), (2) re-executes the affected
    /// plot's marks -> fresh batches, (3) installs the fresh mark inputs (a
    /// count-stable retype/rechannel is an in-place swap; a count-changing
    /// add/remove rebuilds the flat-index maps — coordinator.marks /
    /// mark_to_plot / every LivePlot.mark_indices), (4) RESETS the
    /// affected plot's launch anchor to the re-lowered FRESH inference so the
    /// axis is clean not widen-unioned, (5) re-scenes. The mutation is DURABLE:
    /// the edit PERSISTS across later gestures (like `c`).
    ///
    /// Returns `true` when a plot was refreshed (the caller then refreshes the
    /// window); `false` for an edit whose plot the coordinator doesn't track.
    pub fn apply_spec_edit(
        &mut self,
        edit: &ChartEdit,
        spec: Spec,
        analysis: SpecAnalysis,
        cx: &mut H::Cx,
    ) -> bool {
        let Some(pi) = self.apply_spec_edit_data(edit, spec, analysis) else {
            return false;
        };
        let scene = self.build_plot_scene(pi);
        let state = self.plots[pi].state.clone();
        state.update(cx, |s| {
            s.set_scene(scene);
            true
        });
        true
    }

    /// Reload the WHOLE working spec into the live session and rebuild EVERY
    /// plot fresh (the undo path). Unlike [`Self::apply_spec_edit`]
    /// (which touches one affected plot for a known edit), an undo restores an
    /// arbitrary earlier `Spec` and can revert any plot's kind / channel / mark
    /// count, so this rebuilds all plots' marks fresh from the restored spec,
    /// re-executes every mark, resets every launch anchor, and re-scenes every
    /// plot. Heavier than a per-edit apply, but undo is a deliberate, infrequent
    /// action. Returns `true` (the caller refreshes the window).
    pub fn reload_all_from_spec(
        &mut self,
        spec: Spec,
        analysis: SpecAnalysis,
        cx: &mut H::Cx,
    ) -> bool {
        // Capture the highlight bindings BEFORE reload_spec MOVES `analysis`, so a
        // rebuilt mark keeps its dimming (finding 1/2/4).
        let highlight_bindings = analysis.highlight_bindings.clone();
        self.session.reload_spec(spec.clone(), analysis);

        // Rebuild every plot's marks fresh, in the engine's depth-first flat
        // order, so coordinator.marks / mark_to_plot / mark_indices stay
        // consistent with the reloaded session's mark_index_map — re-deriving
        // each plot's highlight style + projection from its render context so an
        // undo never silently drops highlight dimming or a geo projection.
        let plot_meta: Vec<(String, SequentialScheme)> = self
            .plots
            .iter()
            .map(|p| (p.path.clone(), p.scheme))
            .collect();
        let plot_nodes = collect_plot_nodes(&spec);
        let mut new_marks: Vec<MarkInput> = Vec::new();
        let mut new_indices: HashMap<String, Vec<usize>> = HashMap::new();
        let mut mark_to_plot: HashMap<usize, usize> = HashMap::new();
        for (path, plot) in &plot_nodes {
            let Some(pi) = plot_meta.iter().position(|(p, _)| p == path) else {
                continue; // a plot the coordinator does not track
            };
            let scheme = plot_meta[pi].1;
            let (highlight_style, projection) =
                plot_render_context(path, plot, &highlight_bindings);
            for c in &plot.items {
                if let Component::Mark(m) = c {
                    let flat = new_marks.len();
                    new_marks.push(build_fresh_mark_input(
                        m,
                        scheme,
                        highlight_style.as_ref(),
                        projection,
                    ));
                    new_indices.entry(path.clone()).or_default().push(flat);
                    mark_to_plot.insert(flat, pi);
                }
            }
        }
        self.marks = new_marks;
        self.mark_to_plot = mark_to_plot;
        for plot in self.plots.iter_mut() {
            plot.mark_indices = new_indices.remove(&plot.path).unwrap_or_default();
        }
        self.assert_engine_mark_agreement();

        // Re-execute, reset the launch anchor, and re-scene every plot.
        for pi in 0..self.plots.len() {
            for mi in self.plots[pi].mark_indices.clone() {
                match self.session.execute_mark(mi) {
                    Ok(batches) => {
                        if let Some(m) = self.marks.get_mut(mi) {
                            m.batch = concat_batches(batches);
                        }
                    }
                    Err(e) => eprintln!("undo: mark {mi} re-execute failed: {e}"),
                }
            }
            if let Some(fresh) = self.fresh_plot_scales(pi) {
                self.plots[pi].launch_scales = fresh;
            }
            let scene = self.build_plot_scene(pi);
            let state = self.plots[pi].state.clone();
            state.update(cx, |s| {
                s.set_scene(scene);
                true
            });
        }
        true
    }

    /// The framework-free data half of [`Self::apply_spec_edit`]: swap the session
    /// spec, update marks + flat-index maps, re-execute, and RESET the affected
    /// plot's launch anchor — returning the affected plot index to re-scene, or
    /// `None` for an untracked plot. Separated so the whole mechanism minus the
    /// final `set_scene` is unit-testable without a window (mirrors
    /// [`Self::apply_slider`]).
    fn apply_spec_edit_data(
        &mut self,
        edit: &ChartEdit,
        spec: Spec,
        analysis: SpecAnalysis,
    ) -> Option<usize> {
        let plot_path = edit.plot_path().to_string();
        let pi = self.plots.iter().position(|p| p.path == plot_path)?;

        // Capture the re-analysis inputs a mark rebuild needs BEFORE `reload_spec`
        // MOVES `analysis` (finding 1/2/4): the highlight bindings + the affected
        // plot's render context (highlight `otherwise` style + geo projection), so
        // a rebuilt/retyped mark keeps its dimming + projection
        // instead of silently reverting to no-highlight / equirectangular.
        let highlight_bindings = analysis.highlight_bindings.clone();
        let affected_ctx = collect_plot_nodes(&spec)
            .into_iter()
            .find(|(p, _)| *p == plot_path)
            .map(|(_, node)| plot_render_context(&plot_path, node, &highlight_bindings));

        // (1) Swap the session's private spec so execute_mark re-emits new SQL.
        self.session.reload_spec(spec.clone(), analysis);

        if edit.is_count_changing() {
            // (3, count-changing) Rebuild coordinator.marks + the flat-index maps
            // so the shifted mark space stays consistent with the engine's
            // rebuilt mark_index_map.
            self.rebuild_marks_and_maps(&spec, &highlight_bindings, &plot_path);
        } else {
            // (3, count-stable) In-place mutate the mark at the edit's ORDINAL
            // (finding 7 — was hardcoded `.first()`) then re-execute.
            let (plot_hl, plot_proj) = affected_ctx
                .clone()
                .unwrap_or((None, Projection::default()));
            if let Some(&mi) = self.plots[pi].mark_indices.get(edit.mark_ordinal()) {
                match edit {
                    ChartEdit::ChangeMarkType { new_kind, .. } => {
                        if let Some(m) = self.marks.get_mut(mi) {
                            m.kind = *new_kind;
                            // Geo carries the plot's projection (finding 1/2/4).
                            let mark_projection = (*new_kind == MarkKind::Geo).then_some(plot_proj);
                            m.renderer_override = configured_renderer(
                                *new_kind,
                                self.plots[pi].scheme,
                                m.bandwidth,
                                m.thresholds,
                                m.bin_width,
                                mark_projection,
                            );
                            // Re-gate highlight to the NEW kind: a retype to a
                            // non-honouring kind drops the style; to a honouring
                            // kind keeps the plot's `otherwise` style so a
                            // brush-to-dim survives the retype (finding 1/2/4).
                            m.highlight_style = mark_honours_highlight(*new_kind)
                                .then(|| plot_hl.clone())
                                .flatten();
                        }
                    }
                    ChartEdit::SetChannel {
                        channel, column, ..
                    } => {
                        if let (Some(m), Some(ch)) =
                            (self.marks.get_mut(mi), Channel::from_wire(channel))
                        {
                            m.channels.insert(ch, column.clone());
                        }
                    }
                    _ => {}
                }
            }
        }

        // (2) Re-execute the affected plot's marks -> fresh batches.
        let mark_indices = self.plots[pi].mark_indices.clone();
        for mi in mark_indices {
            match self.session.execute_mark(mi) {
                Ok(batches) => {
                    if let Some(m) = self.marks.get_mut(mi) {
                        m.batch = concat_batches(batches);
                    }
                }
                Err(e) => eprintln!("spec edit: mark {mi} re-execute failed: {e}"),
            }
        }

        // (2b) Validate the coordinator's flat mark space AGREES with the engine's
        // rebuilt mark_index_map (finding 5 — the engine map is the source of
        // truth; both walk the shared collect_plot_nodes order so they must match).
        self.assert_engine_mark_agreement();

        // (4) RESET the affected plot's launch anchor to the FRESH inference, so
        // build_plot_scene renders a CLEAN axis, not a widen-union against the
        // stale launch domain (a channel/kind rebind is neither a subset nor a
        // param-widen). Preserve the launch Fill ramp's cycled scheme.
        //
        // KNOWN DIVERGENCE (finding 6): `fresh_plot_scales` infers from the
        // CURRENT batches, which are selection-FILTERED if a cross-plot brush /
        // legend selection is active when the edit runs — so the reset anchor can
        // reflect the filtered domain rather than the full one. Accepted in v1
        // (the anchor still widens back as selections clear, matching the
        // widen-only discipline); an unfiltered re-inference is a documented
        // follow-up.
        if let Some(fresh) = self.fresh_plot_scales(pi) {
            self.plots[pi].launch_scales = fresh;
        }
        Some(pi)
    }

    /// Validate the coordinator's flat mark space agrees with the engine session's
    /// rebuilt `mark_index_map` (finding 5). The engine builds its map
    /// from ITS OWN depth-first walk (`build_mark_index_map` → `collect_marks_with_path`),
    /// while the coordinator rebuilds from `collect_plot_nodes`; the two walks
    /// agree on mark ORDER only for the non-nested plot / hconcat / vconcat
    /// layouts the live coordinator drives. So this is a CARDINALITY-ONLY backstop:
    /// it catches a count disagreement (a coordinator bug that would route a later
    /// gesture past the end of the mark space), not a per-index reordering. Logged
    /// loudly rather than silently corrupting.
    fn assert_engine_mark_agreement(&self) {
        let engine = self.session.mark_count();
        if engine != self.marks.len() {
            eprintln!(
                "spec edit: coordinator/engine mark-count disagree ({} coordinator vs {} engine) \
                 — flat-index space may be stale",
                self.marks.len(),
                engine
            );
        }
    }

    /// The FRESH (un-anchored) inferred scales for a plot's current batches —
    /// what a `commit -> reload`'s fresh `build_everything` would compute. Used
    /// to reset the launch anchor after a structural edit so the preview axis is
    /// clean. `None` when the plot has no renderable mark.
    fn fresh_plot_scales(&self, plot_index: usize) -> Option<ScaleSet> {
        let mark_indices = &self.plots[plot_index].mark_indices;
        let layout = self.plots[plot_index].layout;
        let titles = self.plots[plot_index].titles.clone();
        let draw_inline_legend = self.plots[plot_index].draw_inline_legend;
        let highlights: Vec<Option<HighlightState>> = mark_indices
            .iter()
            .map(|&mi| {
                let m = self.marks.get(mi)?;
                build_highlight_state(m.batch.as_ref()?, m.highlight_style.as_ref()?)
            })
            .collect();
        let chart_data: Vec<ChartData<'_>> = mark_indices
            .iter()
            .zip(highlights.iter())
            .filter_map(|(&mi, highlight)| {
                let m = self.marks.get(mi)?;
                let batch = m.batch.as_ref()?;
                let renderer: &dyn MarkRenderer = m
                    .renderer_override
                    .as_deref()
                    .or_else(|| find_renderer(&self.renderers, m.kind))?;
                Some(ChartData {
                    batch,
                    channel_map: &m.channels,
                    renderer,
                    layout,
                    view_extent: None,
                    highlight: highlight.as_ref(),
                })
            })
            .collect();
        if chart_data.is_empty() {
            return None;
        }
        let refs: Vec<&ChartData<'_>> = chart_data.iter().collect();
        let (_scene, scales) = build_multi_mark_scene(&refs, draw_inline_legend, &titles);
        Some(scales)
    }

    /// Rebuild `coordinator.marks` + the flat mark-index maps (`mark_to_plot`,
    /// each `LivePlot.mark_indices`) after a count-changing edit renumbered the
    /// mark space. Marks in UNAFFECTED plots are preserved
    /// verbatim (their paths/positions are unchanged); the affected plot's marks
    /// are rebuilt fresh from the mutated spec (`ChannelMap::from_mark` +
    /// `configured_renderer` at the plot's scheme, re-deriving highlight style +
    /// projection from `highlight_bindings` + the plot node — finding 1/2/4). The
    /// new flat order matches the engine's `build_mark_index_map` (depth-first plot
    /// order, contiguous per-plot marks) for the non-nested layouts the live
    /// coordinator drives.
    fn rebuild_marks_and_maps(
        &mut self,
        spec: &Spec,
        highlight_bindings: &[HighlightBinding],
        affected_path: &str,
    ) {
        // Owned per-plot metadata so no `self.plots` borrow is held while we
        // move MarkInputs out of the old `self.marks`. plot_meta is in
        // `self.plots` order, so the rebuilt `mark_to_plot` (keyed by that
        // ordinal) drops straight in.
        let plot_meta: Vec<(String, Vec<usize>, SequentialScheme)> = self
            .plots
            .iter()
            .map(|p| (p.path.clone(), p.mark_indices.clone(), p.scheme))
            .collect();
        let old_marks = std::mem::take(&mut self.marks);
        let (new_marks, mut new_indices, mark_to_plot) = rebuild_flat_index_space(
            &plot_meta,
            old_marks,
            spec,
            highlight_bindings,
            affected_path,
        );
        self.marks = new_marks;
        self.mark_to_plot = mark_to_plot;
        for plot in self.plots.iter_mut() {
            plot.mark_indices = new_indices.remove(&plot.path).unwrap_or_default();
        }
    }

    /// Commit a slider release: dispatch the param value into the
    /// engine, then rebuild and swap the scenes of every plot whose marks
    /// re-executed. Mid-drag (`Dragging`) and `Idle` states are no-ops — only a
    /// `Released` value commits (matches the tested commit-on-release contract),
    /// so the drag itself never re-queries.
    ///
    /// Returns `true` if a value was committed (the caller then refreshes the
    /// window once, repainting the swapped subscriber scenes).
    pub fn commit_slider(
        &mut self,
        slider_index: usize,
        state: &SliderState,
        cx: &mut H::Cx,
    ) -> bool {
        let to_rebuild = match self.apply_slider(slider_index, state) {
            Some(set) => set,
            None => return false,
        };
        for pi in to_rebuild {
            let scene = self.build_plot_scene(pi);
            let state_entity = self.plots[pi].state.clone();
            state_entity.update(cx, |s| {
                s.set_scene(scene);
                true
            });
        }
        true
    }

    /// The framework-free half of [`Self::commit_slider`]: on a `Released` state, dispatch
    /// the param and absorb the re-execution results into the per-mark batches,
    /// returning the set of plots to rebuild. Returns `None` (nothing committed)
    /// for a non-`Released` state or an out-of-range slider index. Separated so
    /// the commit data-path is unit-testable without a window.
    fn apply_slider(&mut self, slider_index: usize, state: &SliderState) -> Option<HashSet<usize>> {
        if !matches!(state, SliderState::Released { .. }) {
            return None;
        }
        let binding = self.slider_bindings.get(slider_index)?.clone();
        let (_next, results) = commit_slider_release(state, &binding, &mut self.session);
        let mut to_rebuild: HashSet<usize> = HashSet::new();
        self.absorb(results, &mut to_rebuild);
        Some(to_rebuild)
    }

    /// Commit a menu-family pick, mirroring [`Self::commit_slider`]:
    /// dispatch the picked option's typed value into the engine, then rebuild
    /// and swap the scenes of every plot whose marks re-executed. Only a
    /// `Committed` state commits — `Closed`/`Open` are no-ops, so opening a
    /// list or hovering never re-queries.
    ///
    /// Returns `true` if a pick was committed (the caller then refreshes the
    /// window once, repainting the swapped subscriber scenes).
    pub fn commit_menu(&mut self, menu_index: usize, state: &MenuState, cx: &mut H::Cx) -> bool {
        let to_rebuild = match self.apply_menu(menu_index, state) {
            Some(set) => set,
            None => return false,
        };
        for pi in to_rebuild {
            let scene = self.build_plot_scene(pi);
            let state_entity = self.plots[pi].state.clone();
            state_entity.update(cx, |s| {
                s.set_scene(scene);
                true
            });
        }
        true
    }

    /// The framework-free half of [`Self::commit_menu`] (the `apply_slider` split):
    /// on a `Committed` state, dispatch the picked option's [`SpecValue`]
    /// verbatim through `commit_menu_release` — which reads the param's LIVE
    /// value from the session so a same-value pick dispatches nothing — and
    /// absorb the re-execution results into the per-mark batches, returning
    /// the set of plots to rebuild. Returns `None` (nothing committed) for a
    /// non-`Committed` state or an out-of-range widget index — the
    /// negative-control surface. Separated so the commit data-path is
    /// unit-testable without a window.
    fn apply_menu(&mut self, menu_index: usize, state: &MenuState) -> Option<HashSet<usize>> {
        if !matches!(state, MenuState::Committed { .. }) {
            return None;
        }
        let binding = self.menu_bindings.get(menu_index)?.clone();
        // The live param value decides the same-value no-op — read from the
        // session (the source of truth), never a widget-side mirror.
        let current = self
            .session
            .current_params()
            .get(&binding.param_name)
            .cloned();
        let (_next, results) =
            commit_menu_release(state, &binding, current.as_ref(), &mut self.session);
        let mut to_rebuild: HashSet<usize> = HashSet::new();
        self.absorb(results, &mut to_rebuild);
        Some(to_rebuild)
    }

    /// Commit a legend swatch click: drive the single-select
    /// toggle for legend binding `legend_index`, dispatch or clear its
    /// selection through the live `Session`, then rebuild and swap the scenes
    /// of every plot whose marks re-executed. `hit` is the clicked category
    /// (`None` for a click on the legend panel that misses every entry).
    ///
    /// Returns `true` if the click changed the selection (the caller then
    /// refreshes the window once); `false` for a no-op (empty-panel click
    /// with nothing selected, or an unknown index).
    pub fn commit_legend_click(
        &mut self,
        legend_index: usize,
        hit: Option<&str>,
        additive: bool,
        categories: &[String],
        cx: &mut H::Cx,
    ) -> bool {
        // Shift-click (`additive`) toggles a category into/out of an OR'd union;
        // a plain click is the single-select replace/toggle.
        let applied = if additive {
            self.apply_legend_toggle(legend_index, hit, categories)
        } else {
            self.apply_legend_click(legend_index, hit)
        };
        let to_rebuild = match applied {
            Some(set) => set,
            None => return false,
        };
        for pi in to_rebuild {
            let scene = self.build_plot_scene(pi);
            let state = self.plots[pi].state.clone();
            state.update(cx, |s| {
                s.set_scene(scene);
                true
            });
        }
        true
    }

    /// The framework-free half of [`Self::commit_legend_click`] — the single-select
    /// toggle state machine:
    ///
    /// - a category whose exact point predicate is NOT the slot's current
    ///   predicate dispatches `column = 'category'` via
    ///   [`SelectionValue::Text`]'s quoted+escaped literal (covers new,
    ///   different, and slot-replaced-by-a-brush cases alike);
    /// - a category whose predicate IS the slot's current one clears
    ///   (toggle off);
    /// - an empty-panel click clears whatever the slot holds;
    /// - an empty-panel click with an empty slot is a no-op (`None`).
    ///
    /// The decision reads the engine's live `(selection, contributor)` slot
    /// (`Session::contributor_predicate`) rather than a UI-side mirror: the
    /// slot is shared with the `for:`-plot's brush/point interactors, so a
    /// brush that replaced it (or an empty plot click that removed it) is
    /// observed directly — a mirror would invert the toggle after any such
    /// gesture.
    ///
    /// Dispatch and clear go through the same [`SelectionDispatcher`] surface
    /// (`Session::propagate_selection` / `clear_selection`) the brush path
    /// uses, and results fold through the same `absorb` loop. Returns the set
    /// of plots to rebuild; `None` when nothing committed. Separated so the
    /// commit data-path is unit-testable without a window.
    fn apply_legend_click(
        &mut self,
        legend_index: usize,
        hit: Option<&str>,
    ) -> Option<HashSet<usize>> {
        let binding = self.legend_bindings.get(legend_index)?.clone();
        let results = match hit {
            Some(cat) => {
                let predicate = point_predicate(
                    &binding.column,
                    &SelectionValue::Text(cat.to_string()).literal(),
                );
                let slot_holds_same = self
                    .session
                    .contributor_predicate(&binding.selection_name, &binding.contributor.0)
                    == Some(&predicate);
                if slot_holds_same {
                    // The same category again: toggle off.
                    self.session
                        .clear(&binding.selection_name, binding.contributor.clone())
                } else {
                    // New/different category — or a brush currently occupies
                    // the slot: single-select, the fresh predicate REPLACES
                    // this contributor's previous one (the store is keyed by
                    // contributor), so no interim clear is needed.
                    self.session.dispatch(
                        &binding.selection_name,
                        binding.contributor.clone(),
                        predicate,
                    )
                }
            }
            None => {
                if self
                    .session
                    .contributor_predicate(&binding.selection_name, &binding.contributor.0)
                    .is_some()
                {
                    // Empty-panel click with a live contribution: clear it.
                    self.session
                        .clear(&binding.selection_name, binding.contributor.clone())
                } else {
                    // Empty-panel click with an empty slot: no-op.
                    return None;
                }
            }
        };
        let mut to_rebuild: HashSet<usize> = HashSet::new();
        self.absorb(results, &mut to_rebuild);
        Some(to_rebuild)
    }

    /// The additive (shift-click) half of a legend click (legend
    /// multi-select): toggle `hit`'s category into/out of an OR'd union built
    /// from THIS legend's categories, reusing [`Self::legend_selected_categories`]
    /// to derive the current member set — so a foreign predicate sharing the
    /// slot (a brush, or a same-plot point selection) contributes nothing and a
    /// shift-click starts a fresh set, and the built union can never disagree
    /// with the displayed selected state. Removing the last member CLEARS the
    /// contributor (never an empty `Or`, which would filter to zero rows); a
    /// single-member set collapses to a bare `Expr` (single-select parity,
    /// shared statement cache). A shift-click that hits no entry (`None`, a
    /// gap/padding click) is a no-op that leaves the union untouched.
    fn apply_legend_toggle(
        &mut self,
        legend_index: usize,
        hit: Option<&str>,
        categories: &[String],
    ) -> Option<HashSet<usize>> {
        let cat = hit?; // a miss (gap/padding click) leaves the union untouched
        let binding = self.legend_bindings.get(legend_index)?.clone();

        // Derive the current member categories from the live slot (dropping any
        // foreign predicate), then toggle the clicked category in or out.
        let mut members = self.legend_selected_categories(legend_index, categories);
        if let Some(pos) = members.iter().position(|c| c == cat) {
            members.remove(pos);
        } else {
            members.push(cat.to_string());
        }

        let results = if members.is_empty() {
            // Last member removed: clear the contributor — never store Or([]).
            self.session
                .clear(&binding.selection_name, binding.contributor.clone())
        } else {
            // Canonical order → identical sets emit identical SQL; a single
            // member collapses to a bare Expr (single-select parity).
            members.sort();
            let predicate = legend_union_predicate(&binding.column, &members);
            self.session.dispatch(
                &binding.selection_name,
                binding.contributor.clone(),
                predicate,
            )
        };
        let mut to_rebuild: HashSet<usize> = HashSet::new();
        self.absorb(results, &mut to_rebuild);
        Some(to_rebuild)
    }

    /// The bound legend's currently-selected categories, for the host's
    /// legend widget to dim the non-members
    /// (selected-state, extended to a multi-select union).
    /// Derived per call from the engine's contributor slot — NO stored mirror
    /// (the F1 lesson): the slot predicate is decomposed into its OR
    /// members and every candidate category whose point predicate is a member is
    /// returned, in `categories` order.
    ///
    /// Empty when the slot is empty, holds a brush or any predicate matching no
    /// candidate (the F1a scenario, now for display state), or the legend index
    /// is unknown — so a gesture that replaces or clears the slot behind the
    /// legend's back is observed directly, exactly as the toggle decision reads
    /// it. The shift-toggle build reuses this so display and build cannot
    /// disagree.
    #[must_use]
    pub fn legend_selected_categories(
        &self,
        legend_index: usize,
        categories: &[String],
    ) -> Vec<String> {
        let Some(binding) = self.legend_bindings.get(legend_index) else {
            return Vec::new();
        };
        let Some(current) = self
            .session
            .contributor_predicate(&binding.selection_name, &binding.contributor.0)
        else {
            return Vec::new();
        };
        let members = predicate_members(current);
        categories
            .iter()
            .filter(|cat| {
                let predicate = point_predicate(
                    &binding.column,
                    &SelectionValue::Text((*cat).clone()).literal(),
                );
                members.iter().any(|m| **m == predicate)
            })
            .cloned()
            .collect()
    }

    /// Fold re-execution results into the per-mark batch store, recording which
    /// plots need their scene rebuilt. A failed mark keeps its previous batch.
    fn absorb(&mut self, results: Vec<DispatchResult>, to_rebuild: &mut HashSet<usize>) {
        for (mark_index, result) in results {
            match result {
                Ok(batches) => {
                    if let Some(m) = self.marks.get_mut(mark_index) {
                        m.batch = concat_batches(batches);
                    }
                    if let Some(&pi) = self.mark_to_plot.get(&mark_index) {
                        to_rebuild.insert(pi);
                    }
                }
                Err(e) => eprintln!("crossfilter: mark {mark_index} re-execute failed: {e}"),
            }
        }
    }

    /// Rebuild one plot's scene from the current batches of all its marks,
    /// rendered against LAUNCH-ANCHORED scales: a fresh inference over the
    /// current batches folded (widen-only) against the immutable launch set, so
    /// the frame of reference holds still under a filter yet widens to keep
    /// query-rewrite rows on-plot (render fidelity, F1/F2). Stores the
    /// anchored set as the plot's displayed `scales` so a subsequent brush/click
    /// inverts against exactly what was drawn.
    fn build_plot_scene(&mut self, plot_index: usize) -> Scene {
        // Own the inputs up front so no `self.plots` borrow is held across the
        // later `self.plots[..].scales = …` write.
        let mark_indices = self.plots[plot_index].mark_indices.clone();
        let layout = self.plots[plot_index].layout;
        let draw_inline_legend = self.plots[plot_index].draw_inline_legend;
        let titles = self.plots[plot_index].titles.clone();
        let launch = self.plots[plot_index].launch_scales.clone();

        let (scene, anchored) = render_plot_scene(
            &self.marks,
            &self.renderers,
            &mark_indices,
            &layout,
            draw_inline_legend,
            &titles,
            &launch,
        );
        self.plots[plot_index].scales = anchored;
        scene
    }
}

/// Rebuild one plot's scene from its marks, launch-anchored, independent of any
/// `self`/`Entity` state so it is unit-testable headlessly. Returns the scene
/// AND the anchored scales it rendered against (the caller stores them as the
/// plot's displayed set).
///
/// Renders through [`build_multi_mark_scene_anchored`]: a FRESH inference over
/// the current batches, folded (widen-only) against the passed `launch` set. A
/// subset filter yields exactly `launch` — axes, colour assignments, and ramp
/// anchoring hold still — while a query-rewrite gesture (a slider changing a
/// `$param`) widens the domain to keep new rows on-plot instead of clipping
/// them (F1), and a mark whose batch arrived after launch contributes its
/// scales via the fresh inference (F2).
///
/// Each mark dispatches to its own `renderer_override` (the scheme/attribute-
/// configured renderer its FIRST render used — raster/heatmap/cell scheme,
/// heatmap/contour bandwidth, contour thresholds), falling back to the shared
/// registry for an unconfigured mark. This is the single renderer-config seam
/// the first render and every live rebuild share, closing the
/// heatmap/cell/contour and raster live-scheme losses in one place.
///
/// `draw_inline_legend` mirrors the app's first-render suppression (a
/// standalone `legend: color for:` relocates the inline legend), so a live
/// re-render honours the same choice rather than resurrecting the inline
/// legend — which matters because every raster plot carries a Fill (Sequential)
/// scale and would otherwise grow a gradient bar after the first gesture.
fn render_plot_scene(
    marks: &[MarkInput],
    renderers: &[(MarkKind, Box<dyn MarkRenderer + Send + Sync>)],
    mark_indices: &[usize],
    layout: &ChartLayout,
    draw_inline_legend: bool,
    titles: &ResolvedTitles,
    launch: &ScaleSet,
) -> (Scene, ScaleSet) {
    // Highlight states must outlive the borrowed `ChartData`, so materialise
    // them first (one per honouring mark whose current batch carries the
    // membership column), then borrow into the entries.
    let highlights: Vec<Option<HighlightState>> = mark_indices
        .iter()
        .map(|&mi| {
            let m = marks.get(mi)?;
            let batch = m.batch.as_ref()?;
            build_highlight_state(batch, m.highlight_style.as_ref()?)
        })
        .collect();
    let chart_data: Vec<ChartData<'_>> = mark_indices
        .iter()
        .zip(highlights.iter())
        .filter_map(|(&mi, highlight)| {
            let m = marks.get(mi)?;
            let batch = m.batch.as_ref()?;
            let renderer: &dyn MarkRenderer = m
                .renderer_override
                .as_deref()
                .or_else(|| find_renderer(renderers, m.kind))?;
            Some(ChartData {
                batch,
                channel_map: &m.channels,
                renderer,
                layout: *layout,
                view_extent: None,
                highlight: highlight.as_ref(),
            })
        })
        .collect();
    let refs: Vec<&ChartData<'_>> = chart_data.iter().collect();
    build_multi_mark_scene_anchored(&refs, draw_inline_legend, titles, launch)
}

/// Whether a coordinator has any live-driving surface — a brush, a slider, a
/// menu-family widget, a bound legend, OR a sequential Fill ramp
/// (the `c` colour-cycle). `new` returns `None` when none
/// hold. Factored out (over booleans, not `LivePlot`s) so the gate — including
/// the sequential-Fill relaxation that keeps an otherwise-static heatmap
/// dashboard live — is headlessly testable without a window (a `LivePlot`
/// holds the host's reactive handle).
fn coordinator_has_live_surface(
    any_brush: bool,
    any_slider: bool,
    any_menu: bool,
    any_legend: bool,
    any_sequential_fill: bool,
    editing_active: bool,
) -> bool {
    editing_active || any_brush || any_slider || any_menu || any_legend || any_sequential_fill
}

/// Whether a `ScaleSet`'s Fill channel is a `Scale::Sequential` ramp — the signal
/// that a plot is colour-cyclable. Pinned to Channel::Fill +
/// the Sequential variant, and shared by both `new`'s live-surface gate and
/// `cycle_scheme`'s guard so the two can't drift.
fn has_sequential_fill(scales: &ScaleSet) -> bool {
    matches!(scales.get(Channel::Fill), Some(Scale::Sequential { .. }))
}

/// The framework-free core of [`CrossfilterCoordinator::rebuild_marks_and_maps`]:
/// recompute the flat mark-index space after a
/// count-changing edit. Given each tracked plot's `(path, old flat indices,
/// scheme)` in coordinator-plots order, the old flat `marks`, the mutated
/// `spec`, its re-analysis `highlight_bindings`, and the affected plot's path, it
/// returns the NEW contiguous `marks` vector, each plot's new flat indices
/// (`path -> Vec<flat>`), and the rebuilt `mark_to_plot` (flat index -> plot
/// ordinal, in `plot_meta` order).
///
/// Marks in UNAFFECTED plots are MOVED verbatim (their batches / renderers /
/// channels / highlight / projection preserved by position, so a later gesture
/// re-scenes them exactly); the affected plot's marks are rebuilt fresh from the
/// spec, re-deriving highlight style + projection from its render context
/// (finding 1/2/4). Extracted from the coordinator so the count-changing
/// integrity is unit-testable WITHOUT a window (a headless test has no host
/// reactive cell to hand a `LivePlot`) — mirroring the
/// `apply_slider` / `apply_spec_edit_data` data-half split.
// Returns the three things the caller must swap in together: the rebuilt marks,
// the per-plot flat-index map, and the old→new index remap. They are a tuple
// rather than a named struct because the caller destructures all three on the
// spot and never stores them; a `type` alias would name the tuple without
// telling the reader what the three positions are, which the doc comment above
// already does.
#[allow(clippy::type_complexity)]
fn rebuild_flat_index_space(
    plot_meta: &[(String, Vec<usize>, SequentialScheme)],
    old_marks: Vec<MarkInput>,
    spec: &Spec,
    highlight_bindings: &[HighlightBinding],
    affected_path: &str,
) -> (
    Vec<MarkInput>,
    HashMap<String, Vec<usize>>,
    HashMap<usize, usize>,
) {
    let mut old_slots: Vec<Option<MarkInput>> = old_marks.into_iter().map(Some).collect();
    let plot_nodes = collect_plot_nodes(spec);
    let mut new_marks: Vec<MarkInput> = Vec::new();
    // plot path -> its new contiguous flat indices.
    let mut new_indices: HashMap<String, Vec<usize>> = HashMap::new();
    // new flat index -> plot ordinal (plot_meta / self.plots order).
    let mut mark_to_plot: HashMap<usize, usize> = HashMap::new();

    for (path, plot) in &plot_nodes {
        let Some(pi) = plot_meta.iter().position(|(p, _, _)| p == path) else {
            continue; // a plot the coordinator does not track
        };
        let (_, old_flat, scheme) = &plot_meta[pi];
        let (highlight_style, projection) = plot_render_context(path, plot, highlight_bindings);
        let marks_in_plot: Vec<&Mark> = plot
            .items
            .iter()
            .filter_map(|c| match c {
                Component::Mark(m) => Some(m),
                _ => None,
            })
            .collect();
        for (j, mark) in marks_in_plot.iter().enumerate() {
            let new_flat = new_marks.len();
            let input = if path == affected_path {
                build_fresh_mark_input(mark, *scheme, highlight_style.as_ref(), projection)
            } else {
                // Preserve by (plot, position) — unaffected plots keep their
                // batches/renderers/channels/highlight/projection.
                old_flat
                    .get(j)
                    .and_then(|&oi| old_slots.get_mut(oi).and_then(Option::take))
                    .unwrap_or_else(|| {
                        build_fresh_mark_input(mark, *scheme, highlight_style.as_ref(), projection)
                    })
            };
            new_marks.push(input);
            new_indices.entry(path.clone()).or_default().push(new_flat);
            mark_to_plot.insert(new_flat, pi);
        }
    }
    (new_marks, new_indices, mark_to_plot)
}

/// The per-plot render context a mark rebuild must re-derive from the swapped
/// spec + analysis (finding 1/2/4): the plot's highlight `otherwise`
/// style and its map projection (geo). Mirrors app
/// assembly (main.rs) so a re-queried mark carries the SAME highlight/projection
/// it launched with — a rebuild that reset these to `None` silently killed
/// dashboard-wide highlight dimming and reverted a US-Albers basemap to
/// equirectangular. `build_fresh_mark_input` gates each by mark kind.
fn plot_render_context(
    plot_path: &str,
    plot: &PlotNode,
    highlight_bindings: &[HighlightBinding],
) -> (Option<HighlightStyle>, Projection) {
    let highlight_style = highlight_bindings
        .iter()
        .find(|b| b.parent_plot.0 == plot_path)
        .map(|b| HighlightStyle::from(&b.style));
    let projection = Projection::from(resolve_projection(plot));
    (highlight_style, projection)
}

/// Build a fresh [`MarkInput`] from a spec mark + its plot's render context
/// (scheme, highlight style, projection). `highlight_style` is applied only to
/// the honouring families (`mark_honours_highlight`); `projection` only to a
/// `geo` mark — exactly the two gates app assembly applies (finding
/// 1/2/4). Used by the count-changing rebuild + the undo full-reload path.
fn build_fresh_mark_input(
    mark: &Mark,
    scheme: SequentialScheme,
    plot_highlight: Option<&HighlightStyle>,
    projection: Projection,
) -> MarkInput {
    let bandwidth = mark_attr_f64(mark, "bandwidth");
    let thresholds = mark_attr_f64(mark, "thresholds")
        .filter(|t| *t >= 1.0)
        .map(|t| t as usize);
    let bin_width = mark_attr_f64(mark, "binWidth");
    // Geo carries the plot's projection; every other kind ignores it.
    let mark_projection = (mark.kind == MarkKind::Geo).then_some(projection);
    // Only the honouring families dim, so a non-honouring mark stays `None`.
    let highlight_style = mark_honours_highlight(mark.kind)
        .then(|| plot_highlight.cloned())
        .flatten();
    MarkInput {
        batch: None,
        channels: ChannelMap::from_mark(mark),
        kind: mark.kind,
        renderer_override: configured_renderer(
            mark.kind,
            scheme,
            bandwidth,
            thresholds,
            bin_width,
            mark_projection,
        ),
        bandwidth,
        thresholds,
        bin_width,
        highlight_style,
    }
}

/// Read a numeric mark attribute (`bandwidth` / `thresholds` / `binWidth`) from
/// a mark's option bag as `f64`, if present as a literal number.
fn mark_attr_f64(mark: &Mark, key: &str) -> Option<f64> {
    match mark.options.get(key) {
        Some(ValueOrParamRef::Value(SpecValue::Float(f))) => Some(*f),
        Some(ValueOrParamRef::Value(SpecValue::Integer(i))) => Some(*i as f64),
        _ => None,
    }
}

/// Rebuild a mark's renderer through `scheme`, THREADING the mark's own retained
/// render-config attrs (bandwidth / thresholds / bin_width) — the exact per-mark
/// rebuild `cycle_scheme` performs, so a colour cycle re-colours WITHOUT reshaping
/// the KDE surface, contour iso-lines, or hex mesh. Extracted
/// so the attr threading is headlessly testable (`cycle_scheme` itself needs a
/// window).
fn recolour_override(
    m: &MarkInput,
    scheme: SequentialScheme,
) -> Option<Box<dyn MarkRenderer + Send + Sync>> {
    // Geo's projection is NOT retained here (v1): a colour cycle rebuilds a geo
    // choropleth at the DEFAULT (equirectangular) projection. This is a narrow,
    // documented limitation — geo is a STATIC sole-in-plot mark, and no shipped
    // example hits it (the hermetic inline example is equirectangular; the live
    // Albers example is a stroke-only basemap with no Sequential fill, so it is
    // not colour-cyclable). Passing the resolved projection would need a
    // `projection` field on MarkInput, deferred with geo interaction.
    configured_renderer(m.kind, scheme, m.bandwidth, m.thresholds, m.bin_width, None)
}

/// Swap a `ScaleSet`'s Fill ramp stops in place, preserving its domain.
/// Returns `true` when a `Scale::Sequential` Fill was present and
/// recoloured; `false` for a categorical or absent Fill. The load-bearing half
/// of a colour cycle: `anchor_scale` copies LAUNCH stops (scale.rs), so
/// rewriting the launch Fill stops is what actually moves the on-screen ramp.
fn recolour_fill(scales: &mut ScaleSet, stops: Vec<[f32; 4]>) -> bool {
    if let Some(Scale::Sequential {
        domain_min,
        domain_max,
        ..
    }) = scales.get(Channel::Fill)
    {
        let (domain_min, domain_max) = (*domain_min, *domain_max);
        scales.insert(
            Channel::Fill,
            Scale::Sequential {
                domain_min,
                domain_max,
                stops,
            },
        );
        true
    } else {
        false
    }
}

/// Invert a pixel-space brush (two corners in element-local logical pixels) into
/// a data-coordinate [`Rect`] using a plot's scales. Each axis is inverted
/// independently via [`Scale::inverse_f64`]; a categorical/missing scale (where
/// continuous inversion is undefined) leaves that axis in pixel units, which is
/// harmless for the common case because `brush_rect_to_predicate` only reads the
/// axis its `BrushKind` needs (and numeric brush channels invert cleanly).
pub(crate) fn invert_pixel_brush(start: Point, current: Point, scales: &ScaleSet) -> Rect {
    let (x0, x1) = invert_axis(scales.get(Channel::X), start.x, current.x);
    let (y0, y1) = invert_axis(scales.get(Channel::Y), start.y, current.y);
    Rect::new(x0, y0, x1, y1)
}

/// Invert two pixel positions on one axis to a normalised `(lo, hi)` data range.
/// Falls back to the raw pixel range when the scale can't invert (categorical).
fn invert_axis(scale: Option<&Scale>, a: f64, b: f64) -> (f64, f64) {
    match scale.and_then(|s| Some((s.inverse_f64(a)?, s.inverse_f64(b)?))) {
        Some((da, db)) => (da.min(db), da.max(db)),
        None => (a.min(b), a.max(b)),
    }
}

/// Capture one axis's live [`Scale`] as the structured-clause metadata a
/// brush carries into the query layer: the transform kind, the data-space
/// domain, and the pixel-space range the gesture was inverted through.
/// `pixel_size` is the per-pixel selection granularity (`1.0` — one pixel of
/// gesture = one selection step; nothing coarsens brushes today).
///
/// Colour-family scales never sit on a brush axis, but the mapping is total
/// so a caller can feed any channel's scale through it.
#[must_use]
pub fn clause_meta_for_scale(scale: &Scale) -> ClauseMeta {
    let descriptor = match scale {
        Scale::Linear {
            domain_min,
            domain_max,
            range_start,
            range_end,
        } => ScaleDescriptor {
            kind: "linear".to_string(),
            domain: Some((*domain_min, *domain_max)),
            range: Some((*range_start, *range_end)),
        },
        Scale::Time {
            domain_min_us,
            domain_max_us,
            range_start,
            range_end,
        } => ScaleDescriptor {
            kind: "time".to_string(),
            domain: Some((*domain_min_us as f64, *domain_max_us as f64)),
            range: Some((*range_start, *range_end)),
        },
        Scale::Band {
            range_start,
            range_end,
            ..
        } => ScaleDescriptor {
            kind: "band".to_string(),
            domain: None,
            range: Some((*range_start, *range_end)),
        },
        Scale::Colour { .. } => ScaleDescriptor {
            kind: "colour".to_string(),
            domain: None,
            range: None,
        },
        Scale::Sequential {
            domain_min,
            domain_max,
            ..
        } => ScaleDescriptor {
            kind: "sequential".to_string(),
            domain: Some((*domain_min, *domain_max)),
            range: None,
        },
    };
    ClauseMeta {
        scale: Some(descriptor),
        pixel_size: Some(1.0),
    }
}

/// The structured (lossless) form of the drag-commit predicate: invert the
/// pixel gesture through the plot's scales exactly as `invert_pixel_brush`
/// does for the string path, then build `Predicate::Interval` clauses that
/// keep the column, data-space bounds, and per-axis scale/pixel metadata
/// machine-readable all the way into the query layer. Emits SQL equivalent
/// to the string path's predicate for the same gesture (see `ir.rs`).
///
/// The structured path is the LIVE DEFAULT (the release commit dispatches
/// structured clauses, which the engine's automatic pre-aggregation layer
/// derives its cube from); the string path (`invert_pixel_brush` +
/// `brush_rect_to_predicate`) remains as the SQL-equivalent compatibility
/// surface.
#[must_use]
pub fn structured_brush_predicate(
    start: Point,
    current: Point,
    scales: &ScaleSet,
    binding: &BrushBinding,
) -> Predicate {
    let data_rect = invert_pixel_brush(start, current, scales);
    let x_meta = scales.get(Channel::X).map(clause_meta_for_scale);
    let y_meta = scales.get(Channel::Y).map(clause_meta_for_scale);
    brush_rect_to_structured(data_rect, binding.kind, &binding.channels, x_meta, y_meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::commit_brush_release_multi;
    use crate::chart_state::ChartState;
    use brightfield_render::mark::configured_renderer;
    use brightfield_render::scale::{anchor_scales, SequentialScheme};
    use brightfield_render::scene::build_multi_mark_scene;

    /// The [`ReactiveHandle`] for these tests, all of which run with NO live
    /// plots (the data halves are exercised directly; scene swaps need a
    /// window). With no plot there is no state cell to address, so `update`
    /// is unreachable by construction.
    #[derive(Clone)]
    struct NoPlots;

    impl ReactiveHandle for NoPlots {
        type Surface = ();
        type Cx = ();
        fn update(&self, _: &mut (), _: impl FnOnce(&mut ChartState<()>) -> bool) {
            unreachable!("coordinator tests carry no live plots");
        }
    }

    /// The coordinator specialised to the no-plot test handle.
    type TestCoordinator = CrossfilterCoordinator<NoPlots>;

    /// Build the launch `ScaleSet` the app captures at startup: infer over the
    /// given marks' batches through the SAME inferring multi-mark path
    /// `build_everything` uses, so a pinned rebuild renders against exactly what
    /// the first render saw. Mirrors `render_plot_scene`'s renderer dispatch
    /// (mark override, else registry) so the augmenting renderer matches too.
    fn launch_scales(
        marks: &[MarkInput],
        renderers: &[(MarkKind, Box<dyn MarkRenderer + Send + Sync>)],
        mark_indices: &[usize],
        layout: &ChartLayout,
    ) -> ScaleSet {
        let chart_data: Vec<ChartData<'_>> = mark_indices
            .iter()
            .filter_map(|&mi| {
                let m = marks.get(mi)?;
                let batch = m.batch.as_ref()?;
                let renderer: &dyn MarkRenderer = m
                    .renderer_override
                    .as_deref()
                    .or_else(|| find_renderer(renderers, m.kind))?;
                Some(ChartData {
                    batch,
                    channel_map: &m.channels,
                    renderer,
                    layout: *layout,
                    view_extent: None,
                    highlight: None,
                })
            })
            .collect();
        let refs: Vec<&ChartData<'_>> = chart_data.iter().collect();
        build_multi_mark_scene(&refs, true, &ResolvedTitles::default()).1
    }

    /// The scene the OLD re-inferring rebuild produced: infer scales fresh from
    /// the current batches AND render against them (no launch anchor). The
    /// stand-in for the pre-fix behaviour a launch-anchored rebuild must differ
    /// from once the batch is a strict subset of launch.
    fn reinferred_scene(
        marks: &[MarkInput],
        renderers: &[(MarkKind, Box<dyn MarkRenderer + Send + Sync>)],
        mark_indices: &[usize],
        layout: &ChartLayout,
    ) -> Scene {
        let chart_data: Vec<ChartData<'_>> = mark_indices
            .iter()
            .filter_map(|&mi| {
                let m = marks.get(mi)?;
                let batch = m.batch.as_ref()?;
                let renderer: &dyn MarkRenderer = m
                    .renderer_override
                    .as_deref()
                    .or_else(|| find_renderer(renderers, m.kind))?;
                Some(ChartData {
                    batch,
                    channel_map: &m.channels,
                    renderer,
                    layout: *layout,
                    view_extent: None,
                    highlight: None,
                })
            })
            .collect();
        let refs: Vec<&ChartData<'_>> = chart_data.iter().collect();
        build_multi_mark_scene(&refs, true, &ResolvedTitles::default()).0
    }

    /// A fingerprint of a scene's geometry (`path_data`: the packed coordinates
    /// of every dot / line / tick) and colours (`draw_data`: every fill / stroke
    /// paint). Two scenes with equal fingerprints are pixel-identical; a
    /// re-fitted axis moves `path_data`, a re-anchored ramp moves `draw_data`.
    fn scene_bytes(scene: &Scene) -> (Vec<u32>, Vec<u32>) {
        let e = scene.encoding();
        (e.path_data.clone(), e.draw_data.clone())
    }

    /// ChangeMarkType, on the RIGHT surface: a retype's real
    /// downstream effect is the SCENE GEOMETRY, not the SQL — dot/bar/line/rect
    /// all map to SimpleLowerer, which ignores `mark.kind` (byte-identical SQL,
    /// no "kind" column in the batch). Feed one batch through the render path as
    /// a dot, fingerprint the scene, retype to bar, re-render, and assert the
    /// `scene_bytes` CHANGED; retype BACK and assert it reverts (the undo half).
    /// This is the surface the silent-no-op lesson demands for a
    /// retype — a headless assertion the eyeball backstop could otherwise miss.
    #[test]
    fn change_mark_type_changes_the_scene_fingerprint() {
        use arrow::array::Float64Array;
        use arrow::datatypes::{DataType, Field, Schema};
        use std::sync::Arc;
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Float64, false),
            Field::new("b", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap();
        let mut channels = ChannelMap::new();
        channels.insert(Channel::X, "a".to_string());
        channels.insert(Channel::Y, "b".to_string());
        let renderers = default_renderers();
        let layout = ChartLayout::new(400.0, 300.0);

        let fingerprint = |kind: MarkKind| -> (Vec<u32>, Vec<u32>) {
            let ro = configured_renderer(kind, SequentialScheme::default(), None, None, None, None);
            let renderer: &dyn MarkRenderer = ro
                .as_deref()
                .or_else(|| find_renderer(&renderers, kind))
                .unwrap();
            let cd = ChartData {
                batch: &batch,
                channel_map: &channels,
                renderer,
                layout,
                view_extent: None,
                highlight: None,
            };
            let (scene, _) = build_multi_mark_scene(&[&cd], false, &ResolvedTitles::default());
            scene_bytes(&scene)
        };

        let dot = fingerprint(MarkKind::Dot);
        let bar = fingerprint(MarkKind::BarY);
        assert_ne!(
            dot, bar,
            "dot->bar retype changes the scene geometry (circles vs bars)"
        );
        let dot_again = fingerprint(MarkKind::Dot);
        assert_eq!(
            dot, dot_again,
            "retype back to dot reverts the scene fingerprint"
        );
    }

    /// Count-changing flat-index integrity, COORDINATOR half: an
    /// AddMark on plot P must rebuild the flat mark space so (a) `mark_to_plot`
    /// has the NEW flat index -> P, (b) every pre-existing mark still maps to its
    /// ORIGINAL plot, (c) an UNAFFECTED plot's mark is MOVED verbatim (its
    /// retained render-config survives) while the affected plot's marks are
    /// rebuilt fresh from the spec; a RemoveMark then re-checks the same
    /// integrity. Drives the framework-free `rebuild_flat_index_space` directly (the
    /// data core of `rebuild_marks_and_maps` — LivePlot's non-Send
    /// `Entity<ChartState>` can't be built headless). The engine half
    /// (mark_index_map rebuild + resolve + execute) is pinned by
    /// brightfield-engine's `count_change_rebuilds_mark_index_map...`.
    #[test]
    fn count_change_rebuilds_coordinator_mark_to_plot() {
        use brightfield_spec::analysis::ComponentPath;
        use brightfield_spec::edit::{apply, ChartEdit};
        use brightfield_spec::{parse_spec, Format};

        let yaml = "\
data:
  t: SELECT 1 AS a, 2 AS b
vconcat:
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: b
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: b
";
        let spec = parse_spec(yaml, Format::Yaml).expect("parse").spec;
        let cp = |s: &str| ComponentPath(s.to_string());

        // A minimal MarkInput carrying a bandwidth SENTINEL: it survives a
        // verbatim MOVE but not a fresh rebuild (the spec has no bandwidth attr,
        // so a rebuilt mark's bandwidth is None) — the discriminator between
        // "moved" (unaffected plot) and "rebuilt" (affected plot).
        let sentinel = |bw: f64| MarkInput {
            batch: None,
            channels: ChannelMap::new(),
            kind: MarkKind::Dot,
            renderer_override: None,
            bandwidth: Some(bw),
            thresholds: None,
            bin_width: None,
            highlight_style: None,
        };
        // Old flat space: plot0 -> [0] (bw 0.11), plot1 -> [1] (bw 0.42).
        let plot_meta = vec![
            (
                "root/vconcat[0]".to_string(),
                vec![0usize],
                SequentialScheme::default(),
            ),
            (
                "root/vconcat[1]".to_string(),
                vec![1usize],
                SequentialScheme::default(),
            ),
        ];
        let old_marks = vec![sentinel(0.11), sentinel(0.42)];

        // AddMark a line to plot 0 (the affected plot).
        let mut spec_b = spec.clone();
        apply(
            &mut spec_b,
            &ChartEdit::AddMark {
                plot: cp("root/vconcat[0]"),
                kind: MarkKind::Line,
            },
        )
        .expect("clean");
        let (new_marks, new_indices, mark_to_plot) =
            rebuild_flat_index_space(&plot_meta, old_marks, &spec_b, &[], "root/vconcat[0]");

        // Three marks now; plot 0 owns two contiguous indices, plot 1 owns one.
        assert_eq!(new_marks.len(), 3);
        let p0 = new_indices["root/vconcat[0]"].clone();
        let p1 = new_indices["root/vconcat[1]"].clone();
        assert_eq!(p0.len(), 2, "AddMark grows the affected plot by one");
        assert_eq!(p1.len(), 1, "the unaffected plot keeps its one mark");
        // (a)+(b) every flat index maps to the RIGHT plot.
        for &mi in &p0 {
            assert_eq!(
                mark_to_plot[&mi], 0,
                "plot 0's marks (incl. the new one) map to plot 0"
            );
        }
        for &mi in &p1 {
            assert_eq!(
                mark_to_plot[&mi], 1,
                "the pre-existing plot-1 mark still maps to plot 1"
            );
        }
        // (c) the affected plot's marks were REBUILT fresh (no sentinel bandwidth);
        // the unaffected plot's mark was MOVED verbatim (its 0.42 sentinel survives).
        for &mi in &p0 {
            assert_eq!(
                new_marks[mi].bandwidth, None,
                "affected plot's marks rebuilt fresh"
            );
        }
        assert_eq!(
            new_marks[p1[0]].bandwidth,
            Some(0.42),
            "unaffected plot's mark moved verbatim"
        );

        // RemoveMark the primary from plot 0: re-check integrity (one mark each).
        let mut spec_c = spec_b.clone();
        apply(
            &mut spec_c,
            &ChartEdit::RemoveMark {
                plot: cp("root/vconcat[0]"),
                mark_ordinal: 0,
            },
        )
        .expect("clean");
        let post_add_meta = vec![
            (
                "root/vconcat[0]".to_string(),
                p0,
                SequentialScheme::default(),
            ),
            (
                "root/vconcat[1]".to_string(),
                p1,
                SequentialScheme::default(),
            ),
        ];
        let (nm2, ni2, mtp2) =
            rebuild_flat_index_space(&post_add_meta, new_marks, &spec_c, &[], "root/vconcat[0]");
        assert_eq!(
            nm2.len(),
            2,
            "RemoveMark shrinks the flat space back to two"
        );
        assert_eq!(ni2["root/vconcat[0]"].len(), 1);
        assert_eq!(ni2["root/vconcat[1]"].len(), 1);
        let q2 = ni2["root/vconcat[1]"][0];
        assert_eq!(mtp2[&q2], 1, "plot 1 still maps to plot 1 after the remove");
        assert_eq!(
            nm2[q2].bandwidth,
            Some(0.42),
            "plot 1's mark still moved verbatim, not corrupted"
        );
    }

    /// Finding 1/2/4: `plot_render_context` re-derives a plot's highlight
    /// `otherwise` style + map projection from the swapped
    /// spec + analysis, so a mark rebuild carries them instead of `None`.
    #[test]
    fn findings124_plot_render_context_derives_highlight_and_projection() {
        use brightfield_spec::analysis::HighlightStyle as SpecHl;
        use brightfield_spec::{parse_spec, Format};

        // A geo plot with projectionType: albers → context carries Albers.
        let geo = "\
data:
  t: SELECT 1 AS a
plot:
  - mark: geo
    data: { from: t }
projectionType: albers
";
        let spec = parse_spec(geo, Format::Yaml).expect("parse").spec;
        let nodes = collect_plot_nodes(&spec);
        let (path, node) = nodes.first().expect("one plot");
        let (hl, proj) = plot_render_context(path, node, &[]);
        assert_eq!(
            proj,
            Projection::Albers,
            "projectionType: albers resolves to Albers"
        );
        assert!(hl.is_none(), "no highlight binding on this plot → no style");

        // A dot plot with a highlight binding → context carries the style.
        let dot = "\
data:
  t: SELECT 1 AS a, 2 AS b
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
";
        let spec = parse_spec(dot, Format::Yaml).expect("parse").spec;
        let nodes = collect_plot_nodes(&spec);
        let (path, node) = nodes.first().expect("one plot");
        let bindings = vec![HighlightBinding {
            interactor_path: ComponentPath(format!("{path}/interactor[highlight]")),
            parent_plot: ComponentPath(path.clone()),
            selection: "sel".to_string(),
            style: SpecHl {
                opacity: Some(0.2),
                fill: None,
                fill_opacity: None,
                stroke: None,
                stroke_opacity: None,
            },
        }];
        let (hl, proj) = plot_render_context(path, node, &bindings);
        assert!(
            hl.is_some(),
            "a highlight binding on this plot yields a render style"
        );
        assert_eq!(
            proj,
            Projection::Equirectangular,
            "no projectionType → the default fit"
        );
    }

    /// Finding 1/2/4: `build_fresh_mark_input` gates highlight by the honouring
    /// families and projection by geo — the two gates app-assembly applies, so a
    /// rebuilt honouring mark dims and a rebuilt geo mark keeps its projection,
    /// while a non-honouring mark never wrongly carries a highlight style.
    #[test]
    fn findings124_build_fresh_mark_input_gates_by_kind() {
        use brightfield_spec::ast::Mark;
        use brightfield_spec::vocab::MarkKind as K;
        let mk = |kind: K| Mark {
            kind,
            status: kind.status(),
            data: None,
            options: Default::default(),
        };
        let style = HighlightStyle {
            opacity: Some(0.2),
            ..Default::default()
        };

        // A honouring dot with a plot highlight style → the mark dims.
        let dot = build_fresh_mark_input(
            &mk(K::Dot),
            SequentialScheme::default(),
            Some(&style),
            Projection::Albers,
        );
        assert!(
            dot.highlight_style.is_some(),
            "a honouring mark carries the plot highlight style"
        );
        assert!(
            dot.renderer_override.is_none(),
            "a non-geo dot ignores the projection (registry renderer)"
        );

        // A non-honouring line with the SAME plot style → no highlight (the 12
        // non-honouring families never dim).
        let line = build_fresh_mark_input(
            &mk(K::Line),
            SequentialScheme::default(),
            Some(&style),
            Projection::Albers,
        );
        assert!(
            line.highlight_style.is_none(),
            "a non-honouring mark never carries a highlight style"
        );

        // A geo mark threads the projection into its configured renderer — and
        // it must be the ALBERS we passed, not the equirectangular default: the
        // finding-1/2/4 regression was the rebuild dropping the projection, which
        // `unwrap_or_default()` silently masks as equirectangular. Read it back
        // through the renderer's `projection()` seam so the guard has teeth.
        let geo = build_fresh_mark_input(
            &mk(K::Geo),
            SequentialScheme::default(),
            None,
            Projection::Albers,
        );
        assert_eq!(
            geo.renderer_override.as_ref().and_then(|r| r.projection()),
            Some(Projection::Albers),
            "a geo mark carries the ALBERS projection through the rebuilt renderer (distinct from the default)"
        );

        // And the equirectangular default threads through as itself (a sanity
        // pin that `projection()` is not hardwired to Albers).
        let geo_default = build_fresh_mark_input(
            &mk(K::Geo),
            SequentialScheme::default(),
            None,
            Projection::Equirectangular,
        );
        assert_eq!(
            geo_default
                .renderer_override
                .as_ref()
                .and_then(|r| r.projection()),
            Some(Projection::Equirectangular),
            "the equirectangular default threads through unchanged"
        );
    }

    /// Finding 1/2/4 (the defect surface): a count-changing rebuild (the
    /// RemoveMark / AddMark path) must carry the affected plot's highlight style
    /// onto its rebuilt honouring marks — the pre-fix rebuild hardcoded `None`,
    /// silently killing dashboard-wide dimming on the card's OWN `d` verb.
    #[test]
    fn findings124_highlight_survives_count_changing_rebuild() {
        use brightfield_spec::analysis::HighlightStyle as SpecHl;
        use brightfield_spec::{parse_spec, Format};
        // plot0 carries TWO honouring dots (so a RemoveMark leaves one); plot1 one.
        let yaml = "\
data:
  t: SELECT 1 AS a, 2 AS b
vconcat:
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: b
      - mark: dot
        data: { from: t }
        x: a
        y: b
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: b
";
        let spec = parse_spec(yaml, Format::Yaml).expect("parse").spec;
        let bindings = vec![HighlightBinding {
            interactor_path: ComponentPath("root/vconcat[0]/interactor[highlight]".to_string()),
            parent_plot: ComponentPath("root/vconcat[0]".to_string()),
            selection: "sel".to_string(),
            style: SpecHl {
                opacity: Some(0.2),
                fill: None,
                fill_opacity: None,
                stroke: None,
                stroke_opacity: None,
            },
        }];
        let plot_meta = vec![
            (
                "root/vconcat[0]".to_string(),
                vec![0usize, 1usize],
                SequentialScheme::default(),
            ),
            (
                "root/vconcat[1]".to_string(),
                vec![2usize],
                SequentialScheme::default(),
            ),
        ];
        let blank = |_i: usize| MarkInput {
            batch: None,
            channels: ChannelMap::new(),
            kind: MarkKind::Dot,
            renderer_override: None,
            bandwidth: None,
            thresholds: None,
            bin_width: None,
            highlight_style: None,
        };
        let old_marks = vec![blank(0), blank(1), blank(2)];
        // Rebuild the AFFECTED plot0 (as a RemoveMark/AddMark on it would).
        let (new_marks, new_indices, _) =
            rebuild_flat_index_space(&plot_meta, old_marks, &spec, &bindings, "root/vconcat[0]");
        for &mi in &new_indices["root/vconcat[0]"] {
            assert!(
                new_marks[mi].highlight_style.is_some(),
                "the affected plot's rebuilt honouring dots keep the highlight style (survives the rebuild)"
            );
        }
    }

    /// Finding 1/2/4 (the geo half, with teeth): a count-changing rebuild of a
    /// geo plot must re-derive the ALBERS projection from the swapped spec — the
    /// pre-fix rebuild passed `projection: None`, so the rebuilt renderer fell
    /// back through `unwrap_or_default()` to equirectangular, silently reverting a
    /// US-Albers basemap to plate carrée on the card's OWN `d`/undo verb. The
    /// renderer's `projection()` seam distinguishes Albers from that default, so
    /// this assertion (unlike a bare `is_some()`) actually pins the fix.
    #[test]
    fn findings124_geo_projection_survives_count_changing_rebuild() {
        use brightfield_spec::{parse_spec, Format};
        // A geo plot carrying `projectionType: albers` and TWO geo marks (so a
        // RemoveMark leaves one). The rebuild reads only the mark kind + the
        // plot's resolved projection — no data is executed — so a trivial source
        // suffices.
        let yaml = "\
data:
  t: SELECT 1 AS a
plot:
  - mark: geo
    data: { from: t }
  - mark: geo
    data: { from: t }
projectionType: albers
";
        let spec = parse_spec(yaml, Format::Yaml).expect("parse").spec;
        let nodes = collect_plot_nodes(&spec);
        let (path, _) = nodes.first().expect("one plot");
        let path = path.clone();
        let plot_meta = vec![(
            path.clone(),
            vec![0usize, 1usize],
            SequentialScheme::default(),
        )];
        let blank = |_i: usize| MarkInput {
            batch: None,
            channels: ChannelMap::new(),
            kind: MarkKind::Geo,
            renderer_override: None,
            bandwidth: None,
            thresholds: None,
            bin_width: None,
            highlight_style: None,
        };
        let old_marks = vec![blank(0), blank(1)];
        let (new_marks, new_indices, _) =
            rebuild_flat_index_space(&plot_meta, old_marks, &spec, &[], &path);
        for &mi in &new_indices[&path] {
            assert_eq!(
                new_marks[mi].renderer_override.as_ref().and_then(|r| r.projection()),
                Some(Projection::Albers),
                "the affected geo plot's rebuilt marks keep the ALBERS projection (survives the rebuild, not reverted to equirectangular)"
            );
        }
    }

    /// A pixel brush inverts to the right data range through a linear scale,
    /// including the y-axis flip (screen y grows downward, data y upward).
    #[test]
    fn invert_pixel_brush_maps_through_linear_scales() {
        let mut scales = ScaleSet::new();
        // x: data 0..100 over pixels 0..400 (range_start < range_end).
        scales.insert(
            Channel::X,
            Scale::Linear {
                domain_min: 0.0,
                domain_max: 100.0,
                range_start: 0.0,
                range_end: 400.0,
            },
        );
        // y: data 0..50 over pixels 300..0 (range_start = bottom pixel, as
        // ChartLayout::y_range yields), so a smaller pixel = larger data value.
        scales.insert(
            Channel::Y,
            Scale::Linear {
                domain_min: 0.0,
                domain_max: 50.0,
                range_start: 300.0,
                range_end: 0.0,
            },
        );

        // Drag from pixel (100, 60) to (300, 240).
        let rect = invert_pixel_brush(Point::new(100.0, 60.0), Point::new(300.0, 240.0), &scales);

        // x: 100/400*100 = 25, 300/400*100 = 75.
        assert!((rect.x0 - 25.0).abs() < 1e-9, "x0 = {}", rect.x0);
        assert!((rect.x1 - 75.0).abs() < 1e-9, "x1 = {}", rect.x1);
        // y: pixel 60 → (60-300)/(0-300)*50 = 40; pixel 240 → 10. Normalised lo..hi.
        assert!((rect.y0 - 10.0).abs() < 1e-9, "y0 = {}", rect.y0);
        assert!((rect.y1 - 40.0).abs() < 1e-9, "y1 = {}", rect.y1);
    }

    /// A 3×3 binned grid `(x_bin, y_bin, __bf_count)` with a central peak — the
    /// shape a density lowerer emits, consumed by raster (raw counts) and
    /// heatmap / contour (KDE-smoothed). ≥2 distinct centres per axis so the
    /// KDE grid builds.
    fn grid_batch() -> (RecordBatch, ChannelMap) {
        use arrow::array::Float64Array;
        use arrow::datatypes::{DataType, Field, Schema};
        use std::sync::Arc;
        let schema = Arc::new(Schema::new(vec![
            Field::new("x_bin", DataType::Float64, false),
            Field::new("y_bin", DataType::Float64, false),
            Field::new("__bf_count", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![
                    0.0, 1.0, 2.0, 0.0, 1.0, 2.0, 0.0, 1.0, 2.0,
                ])),
                Arc::new(Float64Array::from(vec![
                    0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0,
                ])),
                Arc::new(Float64Array::from(vec![
                    1.0, 2.0, 1.0, 2.0, 9.0, 2.0, 1.0, 2.0, 1.0,
                ])),
            ],
        )
        .unwrap();
        let mut channels = ChannelMap::new();
        channels.insert(Channel::X, "x_bin".to_string());
        channels.insert(Channel::Y, "y_bin".to_string());
        (batch, channels)
    }

    /// Regression (review finding): a live plot re-render honours the app's
    /// inline-legend suppression instead of hardcoding it on. `render_plot_scene`
    /// (the Entity-free core `build_plot_scene` calls) draws the inline gradient
    /// legend for a raster plot when `draw_inline_legend` is true, and omits it
    /// when false — so a suppressed raster plot doesn't grow a gradient bar after
    /// the first brush.
    #[test]
    fn render_plot_scene_honours_inline_legend_suppression() {
        use brightfield_render::mark::count_scene_paths;

        // A raster batch whose augment_scales builds a Fill Sequential, so the
        // inline legend is a gradient bar.
        let (batch, channels) = grid_batch();
        let marks = vec![MarkInput {
            batch: Some(batch),
            channels,
            kind: MarkKind::Raster,
            renderer_override: None,
            bandwidth: None,
            thresholds: None,
            bin_width: None,
            highlight_style: None,
        }];
        let renderers = default_renderers();
        let layout = ChartLayout::new(400.0, 300.0);
        // The launch scales (inferred once) carry the Fill Sequential; both
        // rebuilds anchor a fresh inference of the same batch against them (a
        // no-op — anchored == launch).
        let launch = launch_scales(&marks, &renderers, &[0], &layout);

        let (with_legend, _) = render_plot_scene(
            &marks,
            &renderers,
            &[0],
            &layout,
            true,
            &ResolvedTitles::default(),
            &launch,
        );
        let (without_legend, _) = render_plot_scene(
            &marks,
            &renderers,
            &[0],
            &layout,
            false,
            &ResolvedTitles::default(),
            &launch,
        );
        assert!(
            count_scene_paths(&with_legend) > count_scene_paths(&without_legend),
            "the inline gradient legend adds paths when draw_inline_legend is true \
             ({} with vs {} without)",
            count_scene_paths(&with_legend),
            count_scene_paths(&without_legend),
        );
    }

    /// Reworked onto the renderer-override seam: the live
    /// rebuild keeps the plot's declared colorScheme because the scheme now rides
    /// the mark's `renderer_override` (`configured_renderer`) — the SAME seam the
    /// first render uses — not a deleted `LivePlot.scheme` field. A raster mark
    /// whose override is the blues renderer yields a launch Fill Sequential with
    /// the blues ramp (its `augment_scales` carries the scheme); the default
    /// override stays viridis. Render-only: no SQL / plan-hash.
    #[test]
    fn live_rebuild_uses_declared_scheme() {
        let (batch, channels) = grid_batch();
        let renderers = default_renderers();
        let layout = ChartLayout::new(400.0, 300.0);

        let stops_for = |scheme: SequentialScheme| {
            let marks = vec![MarkInput {
                batch: Some(batch.clone()),
                channels: channels.clone(),
                kind: MarkKind::Raster,
                renderer_override: configured_renderer(
                    MarkKind::Raster,
                    scheme,
                    None,
                    None,
                    None,
                    None,
                ),
                bandwidth: None,
                thresholds: None,
                bin_width: None,
                highlight_style: None,
            }];
            match launch_scales(&marks, &renderers, &[0], &layout).get(Channel::Fill) {
                Some(Scale::Sequential { stops, .. }) => stops.clone(),
                other => panic!("expected a Fill Sequential, got {other:?}"),
            }
        };

        assert_eq!(
            stops_for(SequentialScheme::Blues),
            SequentialScheme::Blues.stops(),
            "a blues raster rebuilds with the blues ramp through its override, not the default"
        );
        assert_eq!(
            stops_for(SequentialScheme::default()),
            SequentialScheme::Viridis.stops(),
            "the default override stays viridis"
        );
    }

    /// Recolour is load-bearing: rewriting the LAUNCH Fill stops moves
    /// the anchored ramp; a renderer_override-only scheme swap does NOT (anchor_scale
    /// copies launch stops). The mechanic `cycle_scheme` relies on — mirrors
    /// the launch-Fill-stops probe.
    #[test]
    fn recolour_launch_stops_moves_the_anchored_ramp() {
        let (batch, channels) = grid_batch();
        let renderers = default_renderers();
        let layout = ChartLayout::new(400.0, 300.0);

        // A viridis raster: launch Fill carries the viridis ramp.
        let viridis_marks = vec![MarkInput {
            batch: Some(batch.clone()),
            channels: channels.clone(),
            kind: MarkKind::Raster,
            renderer_override: configured_renderer(
                MarkKind::Raster,
                SequentialScheme::Viridis,
                None,
                None,
                None,
                None,
            ),
            bandwidth: None,
            thresholds: None,
            bin_width: None,
            highlight_style: None,
        }];
        let mut launch = launch_scales(&viridis_marks, &renderers, &[0], &layout);
        match launch.get(Channel::Fill) {
            Some(Scale::Sequential { stops, .. }) => {
                assert_eq!(
                    *stops,
                    SequentialScheme::Viridis.stops(),
                    "launch starts viridis"
                )
            }
            other => panic!("expected a launch Fill Sequential, got {other:?}"),
        }

        // A blues renderer_override, rebuilt against the STILL-viridis launch.
        let blues_override_marks = vec![MarkInput {
            batch: Some(batch.clone()),
            channels: channels.clone(),
            kind: MarkKind::Raster,
            renderer_override: configured_renderer(
                MarkKind::Raster,
                SequentialScheme::Blues,
                None,
                None,
                None,
                None,
            ),
            bandwidth: None,
            thresholds: None,
            bin_width: None,
            highlight_style: None,
        }];

        // Negative: the override swap alone is inert — anchor_scale copies launch
        // stops, so the anchored ramp stays viridis.
        let (_, override_only) = render_plot_scene(
            &blues_override_marks,
            &renderers,
            &[0],
            &layout,
            false,
            &ResolvedTitles::default(),
            &launch,
        );
        match override_only.get(Channel::Fill) {
            Some(Scale::Sequential { stops, .. }) => assert_eq!(
                *stops,
                SequentialScheme::Viridis.stops(),
                "a renderer_override-only swap does NOT move the anchored ramp"
            ),
            other => panic!("expected an anchored Fill Sequential, got {other:?}"),
        }

        // Positive: recolour the launch Fill stops → the anchored rebuild renders
        // the blues ramp (the load-bearing step cycle_scheme performs).
        assert!(recolour_fill(&mut launch, SequentialScheme::Blues.stops()));
        let (_, anchored) = render_plot_scene(
            &blues_override_marks,
            &renderers,
            &[0],
            &layout,
            false,
            &ResolvedTitles::default(),
            &launch,
        );
        match anchored.get(Channel::Fill) {
            Some(Scale::Sequential { stops, .. }) => assert_eq!(
                *stops,
                SequentialScheme::Blues.stops(),
                "recolouring the launch stops moves the anchored ramp"
            ),
            other => panic!("expected an anchored Fill Sequential, got {other:?}"),
        }
    }

    /// None-gate relaxation: the relaxation has two parts and both are
    /// pinned here — (1) `has_sequential_fill` detects a colour-cyclable plot from
    /// its launch Fill being a `Scale::Sequential` (so a wrong-channel or
    /// wrong-variant regression makes `c` inert on the heatmap dashboards it
    /// targets), and (2) `coordinator_has_live_surface` OR-folds that flag so `new`
    /// keeps the coordinator alive for an otherwise-static sequential dashboard.
    /// (`new` itself runs headlessly only with no plots — a `LivePlot` holds
    /// the host's reactive state handle.)
    #[test]
    fn sequential_fill_keeps_the_coordinator_live() {
        // (1) Detection over real ScaleSets: a Sequential Fill is seen; an empty
        // set and a categorical (non-Sequential) Fill are not.
        let mut seq = ScaleSet::new();
        seq.insert(
            Channel::Fill,
            Scale::Sequential {
                domain_min: 0.0,
                domain_max: 1.0,
                stops: SequentialScheme::Viridis.stops(),
            },
        );
        assert!(has_sequential_fill(&seq), "a Sequential Fill is cyclable");
        assert!(
            !has_sequential_fill(&ScaleSet::new()),
            "no Fill → not cyclable"
        );
        let mut categorical = ScaleSet::new();
        categorical.insert(
            Channel::Fill,
            Scale::Colour {
                categories: vec!["a".into()],
                palette: vec![[0.0, 0.0, 0.0, 1.0]],
            },
        );
        assert!(
            !has_sequential_fill(&categorical),
            "a categorical Fill is not a Sequential ramp"
        );
        // And through real inference: a raster's launch Fill IS a Sequential ramp
        // (pins the channel — probing Color instead would miss it).
        let (batch, channels) = grid_batch();
        let renderers = default_renderers();
        let layout = ChartLayout::new(400.0, 300.0);
        let raster = vec![MarkInput {
            batch: Some(batch),
            channels,
            kind: MarkKind::Raster,
            renderer_override: None,
            bandwidth: None,
            thresholds: None,
            bin_width: None,
            highlight_style: None,
        }];
        assert!(has_sequential_fill(&launch_scales(
            &raster,
            &renderers,
            &[0],
            &layout
        )));
        // (2) The gate OR-folds every driving surface, including sequential-Fill.
        assert!(!coordinator_has_live_surface(
            false, false, false, false, false, false
        ));
        assert!(coordinator_has_live_surface(
            false, false, false, false, true, false
        ));
        assert!(coordinator_has_live_surface(
            true, false, false, false, false, false
        ));
        assert!(coordinator_has_live_surface(
            false, true, false, false, false, false
        ));
        assert!(coordinator_has_live_surface(
            false, false, false, true, false, false
        ));
        // a menu-family widget is a live surface of its own — a
        // menu-only dashboard must keep the coordinator alive.
        assert!(coordinator_has_live_surface(
            false, false, true, false, false, false
        ));
        // the editing grammar is itself a live surface. Without this
        // disjunct a static spec (categorical/absent fill, no selection — e.g.
        // scatter.yaml) built no coordinator, so m/a/e/d/u mutated the working
        // spec but never re-rendered — the silent-no-op class, on the
        // simplest possible spec.
        assert!(coordinator_has_live_surface(
            false, false, false, false, false, true
        ));
    }

    /// regression (the scatter silent-no-op): a STATIC spec — a plain
    /// `dot` mark with a categorical fill, no params / selection / sequential
    /// ramp — must still get a live coordinator when the editing grammar is
    /// active, so a structural edit can re-render. Before the `editing_active`
    /// disjunct `new` returned `None` here and every m/a/e/d/u was a canvas
    /// no-op (the working spec updated, the plot never moved). The flag is
    /// load-bearing: `false` (the dump/embedding path) still yields `None`;
    /// `true` (the authoring window) yields `Some`.
    #[test]
    fn static_spec_gets_a_coordinator_for_keyboard_editing() {
        use brightfield_engine::Engine;
        use brightfield_spec::analysis::analyse_spec;
        use brightfield_spec::{parse_spec, Format};
        use brightfield_sql::collect_marks;

        // A static scatter: categorical fill, no params, no selection, no
        // sequential ramp — exactly the shape that built no coordinator.
        let yaml = r#"
data:
  points:
    - { weight: 52, height: 158, group: A }
    - { weight: 70, height: 173, group: B }
    - { weight: 95, height: 188, group: C }
plot:
  - mark: dot
    data: { from: points }
    x: weight
    y: height
    fill: group
"#;
        let build_marks = || {
            let parsed = parse_spec(yaml, Format::Yaml).expect("parse");
            let analysis = analyse_spec(&parsed.spec).expect("analyse");
            let engine = Engine::new();
            let mut session = engine
                .load_spec(parsed.spec.clone(), analysis, None)
                .expect("load")
                .session;
            let results = session.execute_all();
            let marks_ast = collect_marks(&parsed.spec);
            let marks: Vec<MarkInput> = results
                .into_iter()
                .enumerate()
                .map(|(i, r)| MarkInput {
                    batch: r.ok().and_then(concat_batches),
                    channels: ChannelMap::from_mark(marks_ast[i]),
                    kind: marks_ast[i].kind,
                    renderer_override: None,
                    bandwidth: None,
                    thresholds: None,
                    bin_width: None,
                    highlight_style: None,
                })
                .collect();
            (session, marks)
        };

        // No live surface AND editing off (the PNG/embedding path) → None,
        // exactly as before.
        let (session, marks) = build_marks();
        assert!(
            TestCoordinator::new(session, marks, vec![], vec![], vec![], vec![], false).is_none(),
            "a static spec with editing off stays coordinator-free (dump path)"
        );

        // The authoring window activates the editing grammar → the same static
        // spec now gets a coordinator, so a structural edit can re-render
        // (scatter fix).
        let (session, marks) = build_marks();
        assert!(
            TestCoordinator::new(session, marks, vec![], vec![], vec![], vec![], true).is_some(),
            "the editing grammar is a live surface: a static scatter's m/a/e/d/u \
             must re-render, not silently no-op"
        );
    }

    /// Attr retention: drives `recolour_override` — the EXACT per-mark
    /// rebuild `cycle_scheme` performs — and proves it threads the mark's OWN
    /// bandwidth into the new renderer, so a colour cycle can't reshape the KDE
    /// surface. Both marks are rebuilt by `recolour_override` and anchored against
    /// one fixed scale set, so the retained bandwidth is the only variable: a
    /// regression that dropped the attrs (`configured_renderer(.., None, None,
    /// None)`) would make these render-identical and fail the `assert_ne`.
    #[test]
    fn cycle_retains_bandwidth() {
        let (batch, channels) = grid_batch();
        let renderers = default_renderers();
        let layout = ChartLayout::new(400.0, 300.0);
        // A heatmap MarkInput carrying `bw`, with its override rebuilt by
        // recolour_override through the NEXT scheme (exactly what cycle_scheme does).
        let cycled = |bw: Option<f64>| {
            let m = MarkInput {
                batch: Some(batch.clone()),
                channels: channels.clone(),
                kind: MarkKind::Heatmap,
                renderer_override: None,
                bandwidth: bw,
                thresholds: None,
                bin_width: None,
                highlight_style: None,
            };
            let ovr = recolour_override(&m, SequentialScheme::Blues);
            vec![MarkInput {
                renderer_override: ovr,
                ..m
            }]
        };
        // One fixed scale set (union of both bandwidths' inferences) so only the
        // retained bandwidth varies.
        let l1 = launch_scales(&cycled(Some(2.0)), &renderers, &[0], &layout);
        let l2 = launch_scales(&cycled(None), &renderers, &[0], &layout);
        let fixed = anchor_scales(&l1, l2);
        let kept = render_plot_scene(
            &cycled(Some(2.0)),
            &renderers,
            &[0],
            &layout,
            false,
            &ResolvedTitles::default(),
            &fixed,
        )
        .0;
        let dropped = render_plot_scene(
            &cycled(None),
            &renderers,
            &[0],
            &layout,
            false,
            &ResolvedTitles::default(),
            &fixed,
        )
        .0;
        assert_ne!(
            scene_bytes(&kept),
            scene_bytes(&dropped),
            "recolour_override threads the mark's own bandwidth; dropping it reshapes the KDE surface"
        );
    }

    /// Launch-anchored scales: after a legend click FILTERS the
    /// subscriber to a subset, the rebuild anchors its fresh inference against
    /// the launch scales widen-only — so a subset yields exactly the launch
    /// domain and the axes hold still, instead of shrinking to the filtered
    /// batch. The anchored rebuild differs from the old re-inferring build (the
    /// axes would have jumped), and the anchored scales it stores equal launch.
    #[test]
    fn rebuild_anchors_to_launch_not_reinferred() {
        let coord = legend_toggle_coordinator();
        let mut c = coord.borrow_mut();
        let layout = ChartLayout::new(360.0, 300.0);

        // Subscriber = mark 1 (the `filterBy: $sel` dot plot). Its launch scales
        // span the full batch (x in 1..6).
        let launch = launch_scales(&c.marks, &c.renderers, &[1], &layout);
        let launch_x = launch.get(Channel::X).and_then(|s| s.domain_max()).unwrap();

        // Filter to gentoo → 3 of 6 rows (x in 3..5), a strict subset.
        assert!(c.apply_legend_click(0, Some("gentoo")).is_some());
        assert_eq!(c.marks[1].batch.as_ref().unwrap().num_rows(), 3);

        // Re-inferring over the now-filtered batch yields a NARROWER x-domain —
        // exactly the axis jump anchoring suppresses.
        let reinferred = launch_scales(&c.marks, &c.renderers, &[1], &layout);
        let reinferred_x = reinferred
            .get(Channel::X)
            .and_then(|s| s.domain_max())
            .unwrap();
        assert!(
            reinferred_x < launch_x,
            "re-inference would shrink the x-domain: {reinferred_x} < {launch_x}"
        );

        // The launch-anchored rebuild: a subset folds to exactly launch, so the
        // stored (displayed) scales equal launch and the scene differs from the
        // old re-inferring build.
        let (anchored_scene, anchored) = render_plot_scene(
            &c.marks,
            &c.renderers,
            &[1],
            &layout,
            true,
            &ResolvedTitles::default(),
            &launch,
        );
        assert_eq!(
            anchored.get(Channel::X).and_then(|s| s.domain_max()),
            Some(launch_x),
            "a subset filter anchors back to exactly the launch domain"
        );
        // The old behaviour rendered against the re-inferred (shrunk) scales —
        // a visibly different scene (axes + point positions moved).
        let reinferred_scene = reinferred_scene(&c.marks, &c.renderers, &[1], &layout);
        assert_ne!(
            scene_bytes(&anchored_scene),
            scene_bytes(&reinferred_scene),
            "anchored vs re-inferred rebuilds differ — the axes would have jumped"
        );
    }

    /// All-channel pinning — colour: a `fill:species` scatter
    /// filtered to one species still encodes that species' LAUNCH palette
    /// colour, not the palette[0] a single-category re-inference would assign.
    /// Categorical Fill rides the same launch-pinned `ScaleSet` as x/y.
    #[test]
    fn filtered_fill_keeps_launch_colour() {
        use arrow::array::{Float64Array, StringArray};
        use arrow::datatypes::{DataType, Field, Schema};
        use peniko::Color;
        use std::sync::Arc;

        fn packed(c: [f32; 4]) -> u32 {
            Color::new(c).premultiply().to_rgba8().to_u32()
        }

        // Categories are inferred in first-appearance order, so "gentoo" (last)
        // lands at palette index 2 — distinct from the palette[0] it would get
        // alone.
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("species", DataType::Utf8, false),
        ]));
        let full = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0, 40.0])),
                Arc::new(StringArray::from(vec![
                    "adelie",
                    "chinstrap",
                    "gentoo",
                    "gentoo",
                ])),
            ],
        )
        .unwrap();
        let filtered = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![3.0, 4.0])),
                Arc::new(Float64Array::from(vec![30.0, 40.0])),
                Arc::new(StringArray::from(vec!["gentoo", "gentoo"])),
            ],
        )
        .unwrap();
        let mut channels = ChannelMap::new();
        channels.insert(Channel::X, "x".to_string());
        channels.insert(Channel::Y, "y".to_string());
        channels.insert(Channel::Fill, "species".to_string());

        let renderers = default_renderers();
        let layout = ChartLayout::new(360.0, 300.0);

        let full_marks = vec![MarkInput {
            batch: Some(full),
            channels: channels.clone(),
            kind: MarkKind::Dot,
            renderer_override: None,
            bandwidth: None,
            thresholds: None,
            bin_width: None,
            highlight_style: None,
        }];
        let launch = launch_scales(&full_marks, &renderers, &[0], &layout);

        // The launch colour for gentoo (index 2) differs from the colour a
        // filtered-batch re-inference would give it (index 0).
        let filtered_marks = vec![MarkInput {
            batch: Some(filtered),
            channels,
            kind: MarkKind::Dot,
            renderer_override: None,
            bandwidth: None,
            thresholds: None,
            bin_width: None,
            highlight_style: None,
        }];
        let reinferred = launch_scales(&filtered_marks, &renderers, &[0], &layout);
        let launch_gentoo = launch
            .get(Channel::Fill)
            .unwrap()
            .map_colour("gentoo")
            .unwrap();
        let reinferred_gentoo = reinferred
            .get(Channel::Fill)
            .unwrap()
            .map_colour("gentoo")
            .unwrap();
        assert_ne!(
            launch_gentoo, reinferred_gentoo,
            "re-inference recolours gentoo (palette[2] launch vs palette[0] alone)"
        );

        // The anchored rebuild of the filtered batch encodes the LAUNCH gentoo
        // colour — not the re-inferred one. Rendered WITHOUT the inline legend
        // so the colour probe sees only the dots: the swatch legend would draw
        // every category (including adelie at palette[0] == reinferred_gentoo),
        // masking whether a dot was recoloured.
        let (anchored_scene, _) = render_plot_scene(
            &filtered_marks,
            &renderers,
            &[0],
            &layout,
            false,
            &ResolvedTitles::default(),
            &launch,
        );
        let drawn: std::collections::HashSet<u32> = anchored_scene
            .encoding()
            .draw_data
            .iter()
            .copied()
            .collect();
        assert!(
            drawn.contains(&packed(launch_gentoo)),
            "the filtered dots keep the launch palette colour for gentoo"
        );
        assert!(
            !drawn.contains(&packed(reinferred_gentoo)),
            "the filtered dots are NOT recoloured to the re-inferred single-category colour"
        );

        // --- F3: the promised heatmap-Sequential clause. A heatmap rebuilt over
        // a FILTERED batch keeps the LAUNCH ramp domain, so its cell colours do
        // not re-anchor to the filtered max. A regression that re-anchored the
        // Sequential (or mapped cells against the local grid.max_density) would
        // pass every other test and the PNG gate; this catches it. ---
        let (hbatch, hchannels) = grid_batch();
        // A filtered heatmap batch: same bins, smaller counts → a smaller KDE
        // max, so a re-anchor would brighten every cell.
        let hfiltered = {
            use arrow::array::Float64Array;
            use arrow::datatypes::{DataType, Field, Schema};
            let schema = Arc::new(Schema::new(vec![
                Field::new("x_bin", DataType::Float64, false),
                Field::new("y_bin", DataType::Float64, false),
                Field::new("__bf_count", DataType::Float64, false),
            ]));
            RecordBatch::try_new(
                schema,
                vec![
                    Arc::new(Float64Array::from(vec![
                        0.0, 1.0, 2.0, 0.0, 1.0, 2.0, 0.0, 1.0, 2.0,
                    ])),
                    Arc::new(Float64Array::from(vec![
                        0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0,
                    ])),
                    Arc::new(Float64Array::from(vec![
                        1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0,
                    ])),
                ],
            )
            .unwrap()
        };
        let hmark = |batch: RecordBatch| {
            vec![MarkInput {
                batch: Some(batch),
                channels: hchannels.clone(),
                kind: MarkKind::Heatmap,
                renderer_override: configured_renderer(
                    MarkKind::Heatmap,
                    SequentialScheme::default(),
                    None,
                    None,
                    None,
                    None,
                ),
                bandwidth: None,
                thresholds: None,
                bin_width: None,
                highlight_style: None,
            }]
        };
        let hlaunch_marks = hmark(hbatch);
        let hlaunch = launch_scales(&hlaunch_marks, &renderers, &[0], &layout);
        let launch_fill_max = match hlaunch.get(Channel::Fill) {
            Some(Scale::Sequential { domain_max, .. }) => *domain_max,
            other => panic!("expected a launch Fill Sequential, got {other:?}"),
        };

        let hfiltered_marks = hmark(hfiltered);
        // Fresh (re-inferred) Fill over the filtered batch has a SMALLER max.
        let hreinferred = launch_scales(&hfiltered_marks, &renderers, &[0], &layout);
        let reinferred_fill_max = match hreinferred.get(Channel::Fill) {
            Some(Scale::Sequential { domain_max, .. }) => *domain_max,
            other => panic!("expected a re-inferred Fill Sequential, got {other:?}"),
        };
        assert!(
            reinferred_fill_max < launch_fill_max,
            "the filtered heatmap has a smaller density max ({reinferred_fill_max} < {launch_fill_max})"
        );

        // The anchored rebuild pins the Fill Sequential to the LAUNCH domain...
        let (hanchored_scene, hanchored) = render_plot_scene(
            &hfiltered_marks,
            &renderers,
            &[0],
            &layout,
            false,
            &ResolvedTitles::default(),
            &hlaunch,
        );
        match hanchored.get(Channel::Fill) {
            Some(Scale::Sequential { domain_max, .. }) => assert_eq!(
                *domain_max, launch_fill_max,
                "the heatmap's rebuilt Fill Sequential anchors to the launch domain, not the filtered max"
            ),
            other => panic!("expected an anchored Fill Sequential, got {other:?}"),
        }
        // ...and the ENCODED cell colours reflect it: rendering the SAME filtered
        // field against the re-inferred (filtered-max) scales gives provably
        // different colours, so a re-anchor would be visible.
        let hreinferred_scene = reinferred_scene(&hfiltered_marks, &renderers, &[0], &layout);
        assert_ne!(
            scene_bytes(&hanchored_scene),
            scene_bytes(&hreinferred_scene),
            "launch-anchored cell colours differ from re-anchored ones (filtered max would brighten cells)"
        );
    }

    /// Round-trip identity — the crown invariant: a gesture sequence
    /// that returns the engine to unfiltered state rebuilds a scene byte-identical
    /// to launch. Any residual re-inference (item 1) or renderer-config loss
    /// (item 3) would break the equality. Checked for BOTH a plain dot plot
    /// (via a real legend toggle) and a blues + bandwidth heatmap (via a
    /// batch-swap round-trip through its configured override).
    #[test]
    fn round_trip_returns_to_launch_scene() {
        // --- Dot plot: real session, select then toggle off. ---
        {
            let coord = legend_toggle_coordinator();
            let mut c = coord.borrow_mut();
            let layout = ChartLayout::new(360.0, 300.0);
            let launch = launch_scales(&c.marks, &c.renderers, &[1], &layout);
            let (launch_scene, _) = render_plot_scene(
                &c.marks,
                &c.renderers,
                &[1],
                &layout,
                true,
                &ResolvedTitles::default(),
                &launch,
            );

            assert!(c.apply_legend_click(0, Some("gentoo")).is_some());
            assert_eq!(c.marks[1].batch.as_ref().unwrap().num_rows(), 3);
            // Toggle the same category off → back to the full 6-row result.
            assert!(c.apply_legend_click(0, Some("gentoo")).is_some());
            assert_eq!(c.marks[1].batch.as_ref().unwrap().num_rows(), 6);

            let (rebuilt, _) = render_plot_scene(
                &c.marks,
                &c.renderers,
                &[1],
                &layout,
                true,
                &ResolvedTitles::default(),
                &launch,
            );
            assert_eq!(
                scene_bytes(&rebuilt),
                scene_bytes(&launch_scene),
                "dot round-trip returns to a byte-identical launch scene"
            );
        }

        // --- Blues + bandwidth heatmap: batch-swap round-trip. ---
        {
            let (full, channels) = grid_batch();
            let renderers = default_renderers();
            let layout = ChartLayout::new(400.0, 300.0);
            // A filtered subset (drop the peak row) stands in for a gesture.
            let filtered = full.slice(0, 6);

            let override_of = || {
                configured_renderer(
                    MarkKind::Heatmap,
                    SequentialScheme::Blues,
                    Some(0.8),
                    None,
                    None,
                    None,
                )
            };
            let marks_for = |batch: RecordBatch| {
                vec![MarkInput {
                    batch: Some(batch),
                    channels: channels.clone(),
                    kind: MarkKind::Heatmap,
                    renderer_override: override_of(),
                    bandwidth: None,
                    thresholds: None,
                    bin_width: None,
                    highlight_style: None,
                }]
            };

            let launch_marks = marks_for(full.clone());
            let launch = launch_scales(&launch_marks, &renderers, &[0], &layout);
            let (launch_scene, _) = render_plot_scene(
                &launch_marks,
                &renderers,
                &[0],
                &layout,
                true,
                &ResolvedTitles::default(),
                &launch,
            );

            // Filter (rebuild launch-anchored, same override).
            let filtered_marks = marks_for(filtered);
            let (filtered_scene, _) = render_plot_scene(
                &filtered_marks,
                &renderers,
                &[0],
                &layout,
                true,
                &ResolvedTitles::default(),
                &launch,
            );
            assert_ne!(
                scene_bytes(&filtered_scene),
                scene_bytes(&launch_scene),
                "the filter visibly changes the heatmap"
            );

            // Return to the full batch → byte-identical to launch (scheme,
            // bandwidth, and scales all held; a subset anchors back to launch).
            let restored_marks = marks_for(full);
            let (restored_scene, _) = render_plot_scene(
                &restored_marks,
                &renderers,
                &[0],
                &layout,
                true,
                &ResolvedTitles::default(),
                &launch,
            );
            assert_eq!(
                scene_bytes(&restored_scene),
                scene_bytes(&launch_scene),
                "heatmap round-trip returns to a byte-identical launch scene"
            );
        }
    }

    /// Live renderer-config seam: a mark rebuilds through its
    /// `renderer_override` — the SAME configured renderer its first render used.
    /// A blues heatmap keeps blues stops; explicit bandwidth changes the render
    /// vs Silverman; contour keeps its thresholds (more levels ⇒ more iso-line
    /// paths); a dot mark with no override still renders through the registry.
    #[test]
    fn live_rebuild_uses_configured_renderer() {
        use brightfield_render::mark::count_scene_paths;

        let (batch, channels) = grid_batch();
        let renderers = default_renderers();
        let layout = ChartLayout::new(400.0, 300.0);

        // (a) Blues heatmap → launch Fill Sequential carries the blues ramp.
        let blues = vec![MarkInput {
            batch: Some(batch.clone()),
            channels: channels.clone(),
            kind: MarkKind::Heatmap,
            renderer_override: configured_renderer(
                MarkKind::Heatmap,
                SequentialScheme::Blues,
                None,
                None,
                None,
                None,
            ),
            bandwidth: None,
            thresholds: None,
            bin_width: None,
            highlight_style: None,
        }];
        match launch_scales(&blues, &renderers, &[0], &layout).get(Channel::Fill) {
            Some(Scale::Sequential { stops, .. }) => {
                assert_eq!(
                    *stops,
                    SequentialScheme::Blues.stops(),
                    "heatmap keeps blues stops"
                )
            }
            other => panic!("expected a Fill Sequential, got {other:?}"),
        }

        // (b) Explicit bandwidth renders differently from Silverman (its
        // default). Both renders share ONE fixed ScaleSet — the widen-only
        // union of each bandwidth's fresh scales — so the Fill ramp domain is
        // held constant and the ONLY thing that varies between the two renders
        // is the KDE bandwidth carried by `renderer_override`. A mental revert
        // that dropped the override plumbing (both marks falling back to the
        // registry's default Silverman renderer) would collapse these to
        // identical bytes; anchoring both against `fixed` proves the difference
        // is the override, not a re-inferred scale.
        let marks_bw = |bandwidth: Option<f64>| {
            vec![MarkInput {
                batch: Some(batch.clone()),
                channels: channels.clone(),
                kind: MarkKind::Heatmap,
                renderer_override: configured_renderer(
                    MarkKind::Heatmap,
                    SequentialScheme::default(),
                    bandwidth,
                    None,
                    None,
                    None,
                ),
                bandwidth: None,
                thresholds: None,
                bin_width: None,
                highlight_style: None,
            }]
        };
        let l1 = launch_scales(&marks_bw(Some(2.0)), &renderers, &[0], &layout);
        let l2 = launch_scales(&marks_bw(None), &renderers, &[0], &layout);
        let fixed = anchor_scales(&l1, l2);
        assert_ne!(
            scene_bytes(
                &render_plot_scene(
                    &marks_bw(Some(2.0)),
                    &renderers,
                    &[0],
                    &layout,
                    true,
                    &ResolvedTitles::default(),
                    &fixed
                )
                .0
            ),
            scene_bytes(
                &render_plot_scene(
                    &marks_bw(None),
                    &renderers,
                    &[0],
                    &layout,
                    true,
                    &ResolvedTitles::default(),
                    &fixed
                )
                .0
            ),
            "an explicit bandwidth changes the heatmap vs Silverman's rule"
        );

        // (c) Contour keeps its threshold count: more iso-levels ⇒ more paths.
        let render_contour = |thresholds: Option<usize>| {
            let marks = vec![MarkInput {
                batch: Some(batch.clone()),
                channels: channels.clone(),
                kind: MarkKind::Contour,
                renderer_override: configured_renderer(
                    MarkKind::Contour,
                    SequentialScheme::default(),
                    None,
                    thresholds,
                    None,
                    None,
                ),
                bandwidth: None,
                thresholds: None,
                bin_width: None,
                highlight_style: None,
            }];
            let scales = launch_scales(&marks, &renderers, &[0], &layout);
            count_scene_paths(
                &render_plot_scene(
                    &marks,
                    &renderers,
                    &[0],
                    &layout,
                    true,
                    &ResolvedTitles::default(),
                    &scales,
                )
                .0,
            )
        };
        assert!(
            render_contour(Some(8)) > render_contour(Some(2)),
            "more contour thresholds draw more iso-line paths"
        );

        // (d) A dot mark with no override renders through the registry.
        let dot = vec![MarkInput {
            batch: Some(batch.clone()),
            channels: channels.clone(),
            kind: MarkKind::Dot,
            renderer_override: None,
            bandwidth: None,
            thresholds: None,
            bin_width: None,
            highlight_style: None,
        }];
        let scales = launch_scales(&dot, &renderers, &[0], &layout);
        assert!(
            count_scene_paths(
                &render_plot_scene(
                    &dot,
                    &renderers,
                    &[0],
                    &layout,
                    true,
                    &ResolvedTitles::default(),
                    &scales
                )
                .0
            ) > 0,
            "an unconfigured dot mark still renders via the registry"
        );
    }

    /// F1 (widen-only, the SUPERSET half): a slider that WIDENS the
    /// query swaps in a batch whose domain is a strict superset of launch. The
    /// launch-anchored rebuild must WIDEN to admit the new point — the opposite
    /// failure mode from the subset (which clips back to launch). A
    /// launch-PINNED rebuild (the F1 bug) would leave the domain at launch and
    /// the new point would render past the plot edge. Proof is at the scale the
    /// renderer actually encodes with: the new point maps INSIDE the plot under
    /// the anchored scale but PAST the right edge under the (pinned) launch
    /// scale, and a round-trip back to the launch batch is byte-identical.
    #[test]
    fn cfr_f1_superset_widens_scales_and_admits_new_point() {
        use arrow::array::Float64Array;
        use arrow::datatypes::{DataType, Field, Schema};
        use std::sync::Arc;

        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let dot_batch = |xs: Vec<f64>, ys: Vec<f64>| {
            RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(Float64Array::from(xs)),
                    Arc::new(Float64Array::from(ys)),
                ],
            )
            .unwrap()
        };
        let mut channels = ChannelMap::new();
        channels.insert(Channel::X, "x".to_string());
        channels.insert(Channel::Y, "y".to_string());
        let dot_marks = |batch: RecordBatch| {
            vec![MarkInput {
                batch: Some(batch),
                channels: channels.clone(),
                kind: MarkKind::Dot,
                renderer_override: None,
                bandwidth: None,
                thresholds: None,
                bin_width: None,
                highlight_style: None,
            }]
        };

        let renderers = default_renderers();
        let layout = ChartLayout::new(400.0, 300.0);

        // Launch: three points spanning x 0..2. A far-away new point (x = 10)
        // sits well outside launch, so nice-number padding can't blur subset
        // from superset.
        let launch_marks = dot_marks(dot_batch(vec![0.0, 1.0, 2.0], vec![0.0, 1.0, 2.0]));
        let launch = launch_scales(&launch_marks, &renderers, &[0], &layout);
        let (launch_scene, _) = render_plot_scene(
            &launch_marks,
            &renderers,
            &[0],
            &layout,
            true,
            &ResolvedTitles::default(),
            &launch,
        );
        let launch_x = launch
            .get(Channel::X)
            .cloned()
            .expect("launch has an x scale");
        let launch_x_max = launch_x.domain_max().expect("linear x has a domain max");
        assert!(
            launch_x_max < 10.0,
            "the new point is outside the launch domain"
        );

        // Slider widens → superset batch adds the point at x = 10.
        let superset_marks = dot_marks(dot_batch(
            vec![0.0, 1.0, 2.0, 10.0],
            vec![0.0, 1.0, 2.0, 10.0],
        ));
        let (superset_scene, anchored) = render_plot_scene(
            &superset_marks,
            &renderers,
            &[0],
            &layout,
            true,
            &ResolvedTitles::default(),
            &launch,
        );
        let anchored_x = anchored
            .get(Channel::X)
            .cloned()
            .expect("anchored has an x scale");

        // The domain WIDENED to include the new point (a pinned rebuild would
        // have left it at launch_x_max).
        assert!(
            anchored_x.domain_max().unwrap() > launch_x_max,
            "a superset batch widens the x-domain past launch ({:?} > {launch_x_max})",
            anchored_x.domain_max()
        );
        // ...and the rebuilt scene visibly differs from the launch render.
        assert_ne!(
            scene_bytes(&superset_scene),
            scene_bytes(&launch_scene),
            "the widened rebuild is a different scene from launch"
        );

        // The load-bearing claim: the new point lands INSIDE the plot under the
        // anchored scale, but PAST the right edge under the launch scale (the
        // pinned counterfactual that would clip it). x range is left..right.
        let (left, right) = (anchored_x.range_start(), anchored_x.range_end());
        let placed = anchored_x.map_f64(10.0);
        assert!(
            placed >= left.min(right) - 1e-6 && placed <= left.max(right) + 1e-6,
            "under the anchored scale the new point lands inside the plot ({placed} in {left}..{right})"
        );
        assert!(
            launch_x.map_f64(10.0) > launch_x.range_end() + 1e-6,
            "under the (pinned) launch scale the new point would render past the plot edge"
        );

        // Round-trip: slider back to the launch batch → byte-identical to launch.
        let (restored_scene, _) = render_plot_scene(
            &launch_marks,
            &renderers,
            &[0],
            &layout,
            true,
            &ResolvedTitles::default(),
            &launch,
        );
        assert_eq!(
            scene_bytes(&restored_scene),
            scene_bytes(&launch_scene),
            "returning to the launch batch rebuilds the byte-identical launch scene"
        );
    }

    /// F2 (fresh-only channels are ADOPTED): a two-mark plot whose raster overlay
    /// is empty (batch `None`) at launch. Because the raster contributes nothing,
    /// the launch `ScaleSet` has NO `Fill` channel. When a selection absorbs data
    /// into the raster, the rebuild must ADOPT the raster's freshly-inferred Fill
    /// ramp — rendering its cells through the configured scheme. A launch-pinned
    /// rebuild (the F2 bug) would keep the Fill-less launch scales, and the raster
    /// would fall back to the legacy alpha-on-default-blue path instead of the ramp.
    #[test]
    fn cfr_f2_absorbed_raster_adopts_configured_ramp() {
        use peniko::Color;

        fn packed(c: [f32; 4]) -> u32 {
            Color::new(c).premultiply().to_rgba8().to_u32()
        }

        let renderers = default_renderers();
        let layout = ChartLayout::new(400.0, 300.0);
        let (grid, grid_channels) = grid_batch();

        // Base layer (mark 0): a plain dot layer over the grid's x/y so the plot
        // has x/y scales at launch — but NO Fill channel, so launch has no ramp.
        let base = MarkInput {
            batch: Some(grid.clone()),
            channels: {
                let mut ch = ChannelMap::new();
                ch.insert(Channel::X, "x_bin".to_string());
                ch.insert(Channel::Y, "y_bin".to_string());
                ch
            },
            kind: MarkKind::Dot,
            renderer_override: None,
            bandwidth: None,
            thresholds: None,
            bin_width: None,
            highlight_style: None,
        };
        // Overlay (mark 1): a blues raster, EMPTY at launch.
        let mut marks = vec![
            base,
            MarkInput {
                batch: None,
                channels: grid_channels.clone(),
                kind: MarkKind::Raster,
                renderer_override: configured_renderer(
                    MarkKind::Raster,
                    SequentialScheme::Blues,
                    None,
                    None,
                    None,
                    None,
                ),
                bandwidth: None,
                thresholds: None,
                bin_width: None,
                highlight_style: None,
            },
        ];

        // Launch over both marks: the empty raster is skipped, so there is no
        // Fill scale to pin.
        let launch = launch_scales(&marks, &renderers, &[0, 1], &layout);
        assert!(
            launch.get(Channel::Fill).is_none(),
            "with the raster empty at launch there is no Fill ramp to anchor"
        );

        // Absorb: the selection populates the raster's batch.
        marks[1].batch = Some(grid);

        // Rebuild adopts the raster's fresh Fill ramp (the blues scheme).
        let (_, anchored) = render_plot_scene(
            &marks,
            &renderers,
            &[0, 1],
            &layout,
            false,
            &ResolvedTitles::default(),
            &launch,
        );
        match anchored.get(Channel::Fill) {
            Some(Scale::Sequential { stops, .. }) => assert_eq!(
                *stops,
                SequentialScheme::Blues.stops(),
                "the absorbed raster adopts its configured blues ramp"
            ),
            other => panic!("expected an adopted Fill Sequential, got {other:?}"),
        }

        // The raster's cells render THROUGH that ramp, not the default-blue fallback.
        // Probe the raster in isolation (mark 1 only) so the base layer's
        // default-blue dots can't be mistaken for a fallback cell. The peak cell
        // (count 9 == the ramp's domain max) samples the ramp's top stop; the
        // fallback would have painted it full-alpha default blue (DEFAULT_COLOUR).
        let (raster_scene, _) = render_plot_scene(
            &marks,
            &renderers,
            &[1],
            &layout,
            false,
            &ResolvedTitles::default(),
            &launch,
        );
        let drawn: std::collections::HashSet<u32> =
            raster_scene.encoding().draw_data.iter().copied().collect();
        let blues_top = *SequentialScheme::Blues.stops().last().unwrap();
        assert!(
            drawn.contains(&packed(blues_top)),
            "the peak cell is coloured through the blues ramp's top stop"
        );
        assert!(
            !drawn.contains(&packed([0.306, 0.475, 0.655, 1.0])),
            "no cell falls back to full-alpha default blue — the ramp path was taken"
        );
    }

    /// A categorical (Band) axis can't invert; that axis falls back to pixels
    /// rather than panicking or fabricating a domain value.
    #[test]
    fn invert_pixel_brush_falls_back_for_categorical_axis() {
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::X,
            Scale::Band {
                categories: vec!["a".into(), "b".into()],
                range_start: 0.0,
                range_end: 200.0,
                padding: 0.1,
            },
        );
        let rect = invert_pixel_brush(Point::new(20.0, 5.0), Point::new(80.0, 15.0), &scales);
        // x stays in pixel units (no continuous inverse), normalised.
        assert!((rect.x0 - 20.0).abs() < 1e-9);
        assert!((rect.x1 - 80.0).abs() < 1e-9);
    }

    // commit_slider's data path — a Released value
    // re-executes the subscribing mark (row count changes); a Dragging value is a
    // no-op. An empty `plots` vec means no host context is needed: the rebuild loop
    // has nothing to repaint, and we assert on the swapped batch directly.
    #[test]
    fn apply_slider_reexecutes_on_release_only() {
        use brightfield_engine::Engine;
        use brightfield_spec::analysis::analyse_spec;
        use brightfield_spec::{parse_spec, Format};
        use brightfield_sql::collect_marks;

        let yaml = r#"
params:
  threshold: 2
data:
  t:
    - { x: 1, y: 1 }
    - { x: 2, y: 1 }
    - { x: 3, y: 1 }
    - { x: 4, y: 1 }
    - { x: 5, y: 1 }
    - { x: 6, y: 1 }
plot:
  - mark: dot
    data: { from: t, filter: "x > $threshold" }
    x: x
    y: y
"#;
        let parsed = parse_spec(yaml, Format::Yaml).expect("parse");
        let analysis = analyse_spec(&parsed.spec).expect("analyse");
        let engine = Engine::new();
        let mut session = engine
            .load_spec(parsed.spec.clone(), analysis, None)
            .expect("load")
            .session;

        let results = session.execute_all();
        let marks_ast = collect_marks(&parsed.spec);
        let marks: Vec<MarkInput> = results
            .into_iter()
            .enumerate()
            .map(|(i, r)| MarkInput {
                batch: r.ok().and_then(concat_batches),
                channels: ChannelMap::from_mark(marks_ast[i]),
                kind: marks_ast[i].kind,
                renderer_override: None,
                bandwidth: None,
                thresholds: None,
                bin_width: None,
                highlight_style: None,
            })
            .collect();

        let binding = SliderBinding {
            param_name: "threshold".to_string(),
            min: 0.0,
            max: 6.0,
            step: Some(1.0),
        };
        let coord =
            TestCoordinator::new(session, marks, vec![], vec![binding], vec![], vec![], false)
                .expect("a slider binding keeps the coordinator alive with no brushes");
        let mut c = coord.borrow_mut();

        // threshold=2 default → x in {3,4,5,6} = 4 rows.
        let before = c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows());
        assert_eq!(before, 4);

        // mid-drag never commits.
        assert!(
            c.apply_slider(0, &SliderState::Dragging { value: 5.0 })
                .is_none(),
            "Dragging is a no-op"
        );
        assert_eq!(
            c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows()),
            before
        );

        // release at threshold=5 re-executes → x in {6} = 1 row.
        let rebuilt = c.apply_slider(0, &SliderState::Released { value: 5.0 });
        assert!(rebuilt.is_some(), "Released commits");
        let after = c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows());
        assert!(
            after < before,
            "raising threshold drops rows: {before} -> {after}"
        );
        assert_eq!(after, 1);
    }

    // the shipped example ties the whole chain together —
    // its `input: slider` yields a binding via the layout join, and committing a
    // higher threshold through the coordinator drops points.
    #[test]
    fn example_slider_drives_the_mark() {
        use brightfield_engine::Engine;
        use brightfield_spec::analysis::analyse_spec;
        use brightfield_spec::layout::placed_input_nodes;
        use brightfield_spec::layout::Rect as LayoutRect;
        use brightfield_spec::vocab::InputKind;
        use brightfield_spec::{parse_spec, Format};
        use brightfield_sql::collect_marks;

        let yaml = include_str!("../../../examples/param-slider.yaml");
        let parsed = parse_spec(yaml, Format::Yaml).expect("example parses");

        // The example's input:slider yields a binding via the layout path-join.
        let (_, input) = placed_input_nodes(&parsed.spec, LayoutRect::new(0.0, 0.0, 0.0, 0.0))
            .into_iter()
            .find(|(_, i)| i.kind == InputKind::Slider)
            .expect("example declares an input:slider");
        let binding = SliderBinding::from_input(input).expect("binding from the example slider");
        assert_eq!(binding.param_name, "threshold");

        let analysis = analyse_spec(&parsed.spec).expect("analyse");
        let engine = Engine::new();
        let mut session = engine
            .load_spec(parsed.spec.clone(), analysis, None)
            .expect("load")
            .session;
        let results = session.execute_all();
        let marks_ast = collect_marks(&parsed.spec);
        let marks: Vec<MarkInput> = results
            .into_iter()
            .enumerate()
            .map(|(i, r)| MarkInput {
                batch: r.ok().and_then(concat_batches),
                channels: ChannelMap::from_mark(marks_ast[i]),
                kind: marks_ast[i].kind,
                renderer_override: None,
                bandwidth: None,
                thresholds: None,
                bin_width: None,
                highlight_style: None,
            })
            .collect();

        let coord =
            TestCoordinator::new(session, marks, vec![], vec![binding], vec![], vec![], false)
                .expect("coordinator");
        let mut c = coord.borrow_mut();
        let before = c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows());
        c.apply_slider(0, &SliderState::Released { value: 7.0 });
        let after = c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows());
        assert!(
            after < before,
            "raising the example slider drops points: {before} -> {after}"
        );
    }

    // -----------------------------------------------------------------------
    // The menu-family coordinator lane, against a REAL session — the same
    // headless pattern the slider lane uses (an empty `plots` vec
    // means no host context is needed; we assert on the swapped batch directly).
    // -----------------------------------------------------------------------

    /// A String param filtering a dot plot — the menu data-effect fixture.
    /// `region: east` default → 2 of 4 rows; the third region carries an
    /// O'Brien-class single quote for the SQL-integrity pin.
    const MENU_SPEC: &str = r#"
params:
  region: east
data:
  t:
    - { x: 1, region: east }
    - { x: 2, region: east }
    - { x: 3, region: west }
    - { x: 4, region: "O'Brien" }
plot:
  - mark: dot
    data: { from: t, filter: "region = $region" }
    x: x
    y: x
"#;

    /// Build a menu-only coordinator over MENU_SPEC's live session. The
    /// `Some(..)` here IS the liveness assertion: a dashboard whose
    /// only interactive surface is a menu widget must keep the coordinator
    /// alive.
    fn menu_coordinator() -> Rc<RefCell<TestCoordinator>> {
        use brightfield_engine::Engine;
        use brightfield_spec::analysis::analyse_spec;
        use brightfield_spec::{parse_spec, Format};
        use brightfield_sql::collect_marks;

        let parsed = parse_spec(MENU_SPEC, Format::Yaml).expect("parse");
        let analysis = analyse_spec(&parsed.spec).expect("analyse");
        let engine = Engine::new();
        let mut session = engine
            .load_spec(parsed.spec.clone(), analysis, None)
            .expect("load")
            .session;
        let results = session.execute_all();
        let marks_ast = collect_marks(&parsed.spec);
        let marks: Vec<MarkInput> = results
            .into_iter()
            .enumerate()
            .map(|(i, r)| MarkInput {
                batch: r.ok().and_then(concat_batches),
                channels: ChannelMap::from_mark(marks_ast[i]),
                kind: marks_ast[i].kind,
                renderer_override: None,
                bandwidth: None,
                thresholds: None,
                bin_width: None,
                highlight_style: None,
            })
            .collect();

        let binding = crate::menu::MenuBinding {
            param_name: "region".to_string(),
            style: crate::menu::MenuStyle::Menu,
            options: vec![
                SpecValue::String("O'Brien".to_string()),
                SpecValue::String("east".to_string()),
                SpecValue::String("west".to_string()),
            ],
            derived: None,
        };
        TestCoordinator::new(session, marks, vec![], vec![], vec![binding], vec![], false)
            .expect("liveness: a menu binding alone keeps the coordinator alive")
    }

    /// DATA-EFFECT: apply_menu on a Committed String pick
    /// re-executes the subscribing mark — the batch row count changes to the
    /// predicted value. The silent-no-op defence at the
    /// framework-free half of the commit split.
    #[test]
    fn apply_menu_reexecutes_subscriber_on_commit() {
        let coord = menu_coordinator();
        let mut c = coord.borrow_mut();

        // region=east default → 2 rows.
        assert_eq!(c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows()), 2);

        // Non-committed states never re-query (open/hover are overlay-only).
        assert!(c.apply_menu(0, &MenuState::Closed).is_none());
        assert!(c
            .apply_menu(0, &MenuState::Open { hover: Some(1) })
            .is_none());
        assert_eq!(c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows()), 2);

        // Commit "west" (index 2) → exactly the 1 west row.
        let rebuilt = c.apply_menu(0, &MenuState::Committed { index: 2 });
        assert!(rebuilt.is_some(), "Committed commits");
        assert_eq!(
            c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows()),
            1,
            "the subscriber re-executed at region=west"
        );
    }

    /// NEGATIVE CONTROL: the wire can fail — an out-of-range
    /// binding index returns `None` with the batch unchanged, at the same
    /// apply_menu surface the positive test drives.
    #[test]
    fn negative_control_out_of_range_index_is_inert() {
        let coord = menu_coordinator();
        let mut c = coord.borrow_mut();
        let before = c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows());
        assert_eq!(before, 2);

        assert!(
            c.apply_menu(7, &MenuState::Committed { index: 0 })
                .is_none(),
            "an out-of-range widget index commits nothing"
        );
        assert_eq!(
            c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows()),
            before,
            "the batch is untouched — the positive test's effect is real"
        );
    }

    /// Same-value no-op at the coordinator: committing the
    /// already-current option is absorbed as an empty rebuild set — no
    /// re-query, no batch movement (decisions_locked). Row counts alone are
    /// INSENSITIVE here (a spurious same-filter re-query yields an identical
    /// 2-row batch, and the empty-plots fixture returns `Some(empty set)`
    /// either way), so the observable is the batch's OBJECT IDENTITY:
    /// `absorb` swaps `marks[i].batch` (fresh column allocations) only on a
    /// real dispatch. The different-value pick at the end is the sensitivity
    /// control — the same probe MUST move when a dispatch happens.
    #[test]
    fn same_value_pick_no_requery() {
        use std::sync::Arc;

        let coord = menu_coordinator();
        let mut c = coord.borrow_mut();
        let column_of = |c: &TestCoordinator| {
            c.marks[0]
                .batch
                .as_ref()
                .expect("the fixture mark has a batch")
                .column(0)
                .clone()
        };
        let before = column_of(&c);

        // "east" (index 1) is already current: absorbed, batch NOT swapped.
        let rebuilt = c.apply_menu(0, &MenuState::Committed { index: 1 });
        assert_eq!(
            rebuilt,
            Some(HashSet::new()),
            "a same-value pick dispatches nothing — empty rebuild set"
        );
        assert!(
            Arc::ptr_eq(&before, &column_of(&c)),
            "the same-value pick left the batch allocation untouched — no re-query"
        );
        assert_eq!(c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows()), 2);

        // Sensitivity control: a DIFFERENT-value pick ("west", index 2)
        // dispatches, and absorb swaps the batch — the identity probe moves.
        let rebuilt = c.apply_menu(0, &MenuState::Committed { index: 2 });
        assert!(rebuilt.is_some(), "the different-value pick commits");
        assert!(
            !Arc::ptr_eq(&before, &column_of(&c)),
            "a real dispatch swaps the batch allocation — the probe is sensitive"
        );
        assert_eq!(c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows()), 1);
    }

    /// String-param SQL integrity: a menu value carrying a single
    /// quote (O'Brien-class) flows propagate_param → interpolated SQL →
    /// correct filtered results, riding the shipped emit.rs escaping — no
    /// brightfield-sql change (the byte-untouched machine gate co-verifies).
    #[test]
    fn quoted_string_value_filters_correctly() {
        let coord = menu_coordinator();
        let mut c = coord.borrow_mut();

        let rebuilt = c.apply_menu(0, &MenuState::Committed { index: 0 });
        assert!(rebuilt.is_some(), "the quoted pick commits");
        assert_eq!(
            c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows()),
            1,
            "region = 'O''Brien' matches exactly its one row — quoting escaped, not broken"
        );

        // And back out: a subsequent plain pick still works (the statement
        // path was never corrupted by the quote).
        c.apply_menu(0, &MenuState::Committed { index: 1 });
        assert_eq!(c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows()), 2);
    }

    // -----------------------------------------------------------------------
    // legend single-select toggle through the
    // coordinator seam, against a REAL session — and the liveness guard.
    // -----------------------------------------------------------------------

    /// A bound legend + a downstream subscriber over a categorical column.
    /// The legend's `for:` plot is the contributor; the second plot's mark
    /// (flat index 1) subscribes via `filterBy: $sel`.
    const LEGEND_TOGGLE_SPEC: &str = r#"
params:
  sel: { select: crossfilter }
data:
  t:
    - { x: 1, y: 10, species: adelie }
    - { x: 2, y: 20, species: adelie }
    - { x: 3, y: 30, species: gentoo }
    - { x: 4, y: 40, species: gentoo }
    - { x: 5, y: 50, species: gentoo }
    - { x: 6, y: 60, species: chinstrap }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: species
    name: scatter
  - legend: color
    for: scatter
    as: $sel
  - plot:
    - mark: dot
      data: { from: t, filterBy: $sel }
      x: x
      y: y
"#;

    /// Build a legend-only coordinator (no brush bindings, no sliders) over
    /// LEGEND_TOGGLE_SPEC's live session. The `Some(..)` here IS the liveness
    /// assertion: a dashboard whose only interactive surface is a bound
    /// legend must keep the coordinator alive.
    fn legend_toggle_coordinator() -> Rc<RefCell<TestCoordinator>> {
        use brightfield_engine::Engine;
        use brightfield_spec::analysis::analyse_spec;
        use brightfield_spec::{parse_spec, Format};
        use brightfield_sql::collect_marks;

        let parsed = parse_spec(LEGEND_TOGGLE_SPEC, Format::Yaml).expect("parse");
        let analysis = analyse_spec(&parsed.spec).expect("analyse");
        let legend_bindings: Vec<LegendSelectBinding> =
            analysis.legend_bindings.iter().map(Into::into).collect();
        assert_eq!(legend_bindings.len(), 1, "the fixture binds one legend");
        assert_eq!(legend_bindings[0].column, "species");

        let engine = Engine::new();
        let mut session = engine
            .load_spec(parsed.spec.clone(), analysis, None)
            .expect("load")
            .session;
        let results = session.execute_all();
        let marks_ast = collect_marks(&parsed.spec);
        let marks: Vec<MarkInput> = results
            .into_iter()
            .enumerate()
            .map(|(i, r)| MarkInput {
                batch: r.ok().and_then(concat_batches),
                channels: ChannelMap::from_mark(marks_ast[i]),
                kind: marks_ast[i].kind,
                renderer_override: None,
                bandwidth: None,
                thresholds: None,
                bin_width: None,
                highlight_style: None,
            })
            .collect();

        TestCoordinator::new(
            session,
            marks,
            vec![],
            vec![],
            vec![],
            legend_bindings,
            false,
        )
        .expect("liveness: a bound legend alone keeps the coordinator alive")
    }

    /// The predicate the legend's contributor slot currently holds, read
    /// through the same engine lookup `apply_legend_click` decides from.
    fn slot_expr(c: &TestCoordinator) -> Option<String> {
        let b = &c.legend_bindings[0];
        c.session
            .contributor_predicate(&b.selection_name, &b.contributor.0)
            .map(|p| format!("{p}"))
    }

    /// the toggle state machine drives dispatch/clear through the
    /// coordinator against a real session — new category filters the
    /// downstream mark, a different category switches, the same category
    /// clears, and an empty-panel click clears (or no-ops when nothing is
    /// selected). The subscriber's batch (mark 1) is the observable; the
    /// engine's contributor slot (not a UI mirror) is the toggle state.
    #[test]
    fn legend_toggle_state_machine_dispatches_and_clears() {
        let coord = legend_toggle_coordinator();
        let mut c = coord.borrow_mut();
        let rows = |c: &TestCoordinator| c.marks[1].batch.as_ref().map_or(0, |b| b.num_rows());
        let baseline = rows(&c);
        assert_eq!(baseline, 6, "all rows before any click");

        // Empty-panel click with nothing selected: no-op, no dispatch.
        assert!(
            c.apply_legend_click(0, None).is_none(),
            "empty click with no selection is a no-op"
        );
        assert_eq!(rows(&c), baseline);

        // NEW: click 'gentoo' → col = 'gentoo' → 3 of 6 rows downstream.
        assert!(c.apply_legend_click(0, Some("gentoo")).is_some());
        assert!(slot_expr(&c).unwrap().contains("'gentoo'"));
        assert_eq!(rows(&c), 3, "species = 'gentoo' keeps 3 rows");

        // DIFFERENT: click 'adelie' → switches, no stacking → 2 rows.
        assert!(c.apply_legend_click(0, Some("adelie")).is_some());
        assert!(slot_expr(&c).unwrap().contains("'adelie'"));
        assert_eq!(rows(&c), 2, "switching selects only the new category");

        // SAME: click 'adelie' again → toggle off → all rows restored.
        assert!(c.apply_legend_click(0, Some("adelie")).is_some());
        assert_eq!(slot_expr(&c), None, "toggle-off empties the slot");
        assert_eq!(rows(&c), baseline, "toggle-off restores the full result");

        // EMPTY after a select: select then click empty panel → cleared.
        assert!(c.apply_legend_click(0, Some("chinstrap")).is_some());
        assert_eq!(rows(&c), 1);
        assert!(
            c.apply_legend_click(0, None).is_some(),
            "empty click clears"
        );
        assert_eq!(slot_expr(&c), None);
        assert_eq!(rows(&c), baseline);

        // Out-of-range legend index: no-op.
        assert!(c.apply_legend_click(9, Some("gentoo")).is_none());
    }

    /// Regression: the `(selection, contributor)` slot is
    /// shared with the `for:`-plot's brush/point interactors. After a brush
    /// replaces the legend's dispatched predicate, clicking the SAME swatch
    /// again must DISPATCH (replacing the brush with the category predicate)
    /// — a UI-side selected-category mirror would instead clear, inverting
    /// the toggle.
    #[test]
    fn lcf_f1a_brush_replacing_the_slot_does_not_invert_the_toggle() {
        let coord = legend_toggle_coordinator();
        let mut c = coord.borrow_mut();
        let rows = |c: &TestCoordinator| c.marks[1].batch.as_ref().map_or(0, |b| b.num_rows());

        // Legend click: species = 'gentoo' → 3 rows downstream.
        assert!(c.apply_legend_click(0, Some("gentoo")).is_some());
        assert_eq!(rows(&c), 3);

        // A brush on the same plot writes the SAME (selection, contributor)
        // slot — exactly what commit_brush dispatches for an intervalX on
        // the scatter plot (contributor = the plot's node path).
        let contributor = c.legend_bindings[0].contributor.clone();
        let _ = c.session.dispatch(
            "sel",
            contributor,
            Predicate::And(vec![
                Predicate::Expr("x >= 1".to_string()),
                Predicate::Expr("x <= 2".to_string()),
            ]),
        );
        assert!(
            slot_expr(&c).unwrap().contains("x >= 1"),
            "the brush replaced the legend's predicate in the shared slot"
        );

        // Same-swatch click: the slot holds a brush, not the category
        // predicate — so this must DISPATCH species = 'gentoo', not clear.
        assert!(c.apply_legend_click(0, Some("gentoo")).is_some());
        assert!(
            slot_expr(&c).unwrap().contains("'gentoo'"),
            "same-swatch click after a brush dispatches the category"
        );
        assert_eq!(rows(&c), 3, "downstream re-filters to the category");
    }

    /// Regression: after the slot is emptied behind the
    /// legend's back (an empty plot click clears this contributor), the same
    /// swatch must dispatch again in ONE click — a mirror still holding the
    /// category would treat it as a toggle-off no-op round trip.
    #[test]
    fn lcf_f1b_external_clear_does_not_eat_the_next_swatch_click() {
        let coord = legend_toggle_coordinator();
        let mut c = coord.borrow_mut();
        let rows = |c: &TestCoordinator| c.marks[1].batch.as_ref().map_or(0, |b| b.num_rows());
        let baseline = rows(&c);

        // Legend click: species = 'gentoo' → 3 rows downstream.
        assert!(c.apply_legend_click(0, Some("gentoo")).is_some());
        assert_eq!(rows(&c), 3);

        // An empty click on the plot clears this contributor's slot — the
        // same clear commit_brush issues for a zero-area gesture.
        let contributor = c.legend_bindings[0].contributor.clone();
        let _ = c.session.clear("sel", contributor);
        assert_eq!(slot_expr(&c), None, "the plot click emptied the slot");

        // ONE same-swatch click must re-dispatch (slot empty → dispatch),
        // not no-op against a stale mirror.
        assert!(
            c.apply_legend_click(0, Some("gentoo")).is_some(),
            "the first click after an external clear dispatches"
        );
        assert!(slot_expr(&c).unwrap().contains("'gentoo'"));
        assert_eq!(rows(&c), 3);
        assert_ne!(rows(&c), baseline);
    }

    /// Selected-category lookup: `legend_selected_categories` reads
    /// the bound legend's active categories from the engine's contributor slot
    /// per call — never a UI mirror. Select → one; switch → the new one; toggle
    /// off → empty; a brush replacing the slot → empty (the F1a scenario, now
    /// for display state); an external clear → empty. Unknown index → empty.
    #[test]
    fn legend_selected_categories_reads_the_engine_slot() {
        let coord = legend_toggle_coordinator();
        let mut c = coord.borrow_mut();
        let categories: Vec<String> = ["adelie", "gentoo", "chinstrap"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        // Empty slot → empty.
        assert!(c.legend_selected_categories(0, &categories).is_empty());

        // Select gentoo → [gentoo].
        assert!(c.apply_legend_click(0, Some("gentoo")).is_some());
        assert_eq!(
            c.legend_selected_categories(0, &categories),
            vec!["gentoo".to_string()]
        );

        // Switch to adelie → [adelie].
        assert!(c.apply_legend_click(0, Some("adelie")).is_some());
        assert_eq!(
            c.legend_selected_categories(0, &categories),
            vec!["adelie".to_string()]
        );

        // Toggle adelie off → empty.
        assert!(c.apply_legend_click(0, Some("adelie")).is_some());
        assert!(c.legend_selected_categories(0, &categories).is_empty());

        // A brush REPLACES the slot with a non-category predicate → empty (the
        // display state must not claim a category the slot no longer holds).
        assert!(c.apply_legend_click(0, Some("gentoo")).is_some());
        let contributor = c.legend_bindings[0].contributor.clone();
        let _ = c
            .session
            .dispatch("sel", contributor, Predicate::Expr("x >= 1".to_string()));
        assert!(
            c.legend_selected_categories(0, &categories).is_empty(),
            "a brush in the slot matches no candidate category"
        );

        // Re-select, then an external clear empties the slot → empty.
        assert!(c.apply_legend_click(0, Some("chinstrap")).is_some());
        assert_eq!(
            c.legend_selected_categories(0, &categories),
            vec!["chinstrap".to_string()]
        );
        let contributor = c.legend_bindings[0].contributor.clone();
        let _ = c.session.clear("sel", contributor);
        assert!(c.legend_selected_categories(0, &categories).is_empty());

        // Unknown legend index → empty.
        assert!(c.legend_selected_categories(9, &categories).is_empty());
    }

    /// Multi-select: a shift-click toggles a category into
    /// an OR'd union built from THIS legend's categories, canonically sorted so
    /// the same set emits the same SQL; a single member is a bare Expr; a second
    /// shift-click on a member removes it. `slot_expr` is Display-formatted, so
    /// the ' OR ' substring is the real SQL, not the Debug variant tag.
    #[test]
    fn shift_click_builds_a_sorted_or_union() {
        let coord = legend_toggle_coordinator();
        let mut c = coord.borrow_mut();
        let cats: Vec<String> = ["adelie", "gentoo", "chinstrap"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let rows = |c: &TestCoordinator| c.marks[1].batch.as_ref().map_or(0, |b| b.num_rows());

        // Shift-click gentoo → a bare Expr, 3 rows (single-select parity).
        assert!(c.apply_legend_toggle(0, Some("gentoo"), &cats).is_some());
        let s = slot_expr(&c).unwrap();
        assert!(
            s.contains("'gentoo'") && !s.contains(" OR "),
            "one member is a bare Expr: {s}"
        );
        assert_eq!(rows(&c), 3);

        // Shift-click adelie → the union of both → 5 rows (3 + 2).
        assert!(c.apply_legend_toggle(0, Some("adelie"), &cats).is_some());
        let s = slot_expr(&c).unwrap();
        assert!(
            s.contains(" OR ") && s.contains("'gentoo'") && s.contains("'adelie'"),
            "the union ORs both categories: {s}"
        );
        assert_eq!(rows(&c), 5, "adelie u gentoo keeps 5 of 6 rows");
        assert_eq!(
            c.legend_selected_categories(0, &cats),
            vec!["adelie".to_string(), "gentoo".to_string()],
            "both categories read as selected, in category order"
        );
        let forward = slot_expr(&c).unwrap();

        // Canonical order: the same set built in the other click order emits the
        // SAME SQL (statement-cache stable).
        assert!(c.apply_legend_toggle(0, Some("adelie"), &cats).is_some());
        assert!(c.apply_legend_toggle(0, Some("gentoo"), &cats).is_some());
        assert_eq!(slot_expr(&c), None, "removing both members clears the slot");
        assert!(c.apply_legend_toggle(0, Some("adelie"), &cats).is_some());
        assert!(c.apply_legend_toggle(0, Some("gentoo"), &cats).is_some());
        assert_eq!(
            slot_expr(&c).unwrap(),
            forward,
            "member order is canonical, independent of click order"
        );
        assert_eq!(rows(&c), 5);

        // Remove one member → back to the other as a bare Expr.
        assert!(c.apply_legend_toggle(0, Some("adelie"), &cats).is_some());
        let s = slot_expr(&c).unwrap();
        assert!(
            s.contains("'gentoo'") && !s.contains(" OR "),
            "back to a single member: {s}"
        );
        assert_eq!(rows(&c), 3);
    }

    /// removing the LAST member of a union clears the
    /// contributor — never an empty `Or` (which Displays FALSE -> zero rows) —
    /// so the subscriber returns to its unfiltered baseline, not a blank plot.
    #[test]
    fn shift_removing_the_last_member_clears() {
        let coord = legend_toggle_coordinator();
        let mut c = coord.borrow_mut();
        let cats: Vec<String> = ["adelie", "gentoo", "chinstrap"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let rows = |c: &TestCoordinator| c.marks[1].batch.as_ref().map_or(0, |b| b.num_rows());
        let baseline = rows(&c);

        assert!(c.apply_legend_toggle(0, Some("gentoo"), &cats).is_some());
        assert_eq!(rows(&c), 3);
        assert!(c.apply_legend_toggle(0, Some("gentoo"), &cats).is_some());
        assert_eq!(slot_expr(&c), None, "the last member out clears the slot");
        assert_eq!(
            rows(&c),
            baseline,
            "cleared, not an empty Or (which is zero rows)"
        );
        assert!(c.legend_selected_categories(0, &cats).is_empty());
    }

    /// a shift-click never folds a FOREIGN predicate (a
    /// same-plot point selection occupying the shared slot) into the union — it
    /// starts a fresh set; and a shift-click that hits no entry (`None`) is a
    /// no-op that leaves an active union untouched.
    #[test]
    fn shift_ignores_a_foreign_slot_and_a_miss() {
        let coord = legend_toggle_coordinator();
        let mut c = coord.borrow_mut();
        let cats: Vec<String> = ["adelie", "gentoo", "chinstrap"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let rows = |c: &TestCoordinator| c.marks[1].batch.as_ref().map_or(0, |b| b.num_rows());

        // A foreign point selection occupies the shared (selection, contributor)
        // slot — a bare Expr that is NOT one of this legend's categories.
        let contributor = c.legend_bindings[0].contributor.clone();
        let _ = c
            .session
            .dispatch("sel", contributor, Predicate::Expr("x = 5".to_string()));

        // Shift-click gentoo → the foreign Expr is dropped, a FRESH single member.
        assert!(c.apply_legend_toggle(0, Some("gentoo"), &cats).is_some());
        let s = slot_expr(&c).unwrap();
        assert!(
            s.contains("'gentoo'") && !s.contains("x = 5") && !s.contains(" OR "),
            "a foreign slot is replaced, never folded into the Or: {s}"
        );
        assert_eq!(
            c.legend_selected_categories(0, &cats),
            vec!["gentoo".to_string()]
        );

        // Build a 2-member union, then a miss (hit=None) is a no-op.
        assert!(c.apply_legend_toggle(0, Some("adelie"), &cats).is_some());
        let before = slot_expr(&c).unwrap();
        let before_rows = rows(&c);
        assert_eq!(
            c.apply_legend_toggle(0, None, &cats),
            None,
            "a shift-click resolving to no entry is a no-op"
        );
        assert_eq!(
            slot_expr(&c).unwrap(),
            before,
            "the union is untouched by a miss"
        );
        assert_eq!(rows(&c), before_rows);
    }

    /// A minimal cross-filter spec: plot 0 brushes `intervalX` into
    /// `$brush`; plot 1 (mark index 1) is `filterBy: $brush`, so it is the sole
    /// subscriber. Even temp spacing (3 apart) makes a resize's row count
    /// deterministic.
    const DRB_BRUSH_SPEC: &str = r#"
params:
  brush: { select: crossfilter }
data:
  readings:
    - { temp: 2,  power: 14 }
    - { temp: 5,  power: 18 }
    - { temp: 8,  power: 21 }
    - { temp: 11, power: 25 }
    - { temp: 14, power: 30 }
    - { temp: 17, power: 34 }
    - { temp: 20, power: 39 }
    - { temp: 23, power: 41 }
    - { temp: 26, power: 44 }
    - { temp: 29, power: 47 }
    - { temp: 32, power: 49 }
    - { temp: 35, power: 52 }
hconcat:
  - plot:
      - mark: dot
        data: { from: readings }
        x: temp
        y: power
      - select: intervalX
        as: $brush
  - plot:
      - mark: dot
        data: { from: readings, filterBy: $brush }
        x: temp
        y: power
"#;

    /// RE-DISPATCH ON RELEASE CHANGES THE DATA (the silent-no-op defence
    /// — the HIGH the green tests hid). A resize of the persisted brush
    /// re-dispatches from the NEW corners by DRIVING the production
    /// `redispatch_brushing_from`, and the DOWNSTREAM DATA changes (contributor
    /// predicate P2 != P1 AND subscriber rows N2 != N1) — driven from the
    /// transform output, never hand-built corners. A NEGATIVE CONTROL
    /// proves the wire is load-bearing: the raw moved `Selected` fed straight to
    /// commit_brush_release_multi (the unchanged release path) dispatches NOTHING.
    #[test]
    fn release_redispatch_changes_the_data() {
        use crate::interaction::{redispatch_brushing_from, resize_brush, BrushEdge, BrushRegion};
        use brightfield_engine::Engine;
        use brightfield_spec::analysis::analyse_spec;
        use brightfield_spec::{parse_spec, Format};

        let parsed = parse_spec(DRB_BRUSH_SPEC, Format::Yaml).expect("parse");
        let analysis = analyse_spec(&parsed.spec).expect("analyse");
        // The brush SOURCE: a real analysis-derived binding, not hand-built.
        assert_eq!(
            analysis.brushable_bindings.len(),
            1,
            "one intervalX binding"
        );
        let binding: BrushBinding = (&analysis.brushable_bindings[0]).into();
        assert_eq!(binding.selection_name, "brush");
        assert_eq!(binding.channels.x.as_deref(), Some("temp"));
        let contributor = binding.contributor.0.clone();

        let engine = Engine::new();
        let mut session = engine
            .load_spec(parsed.spec.clone(), analysis, None)
            .expect("load")
            .session;
        let _ = session.execute_all();

        // The plot's inversion scales: temp domain [2,35] over the plot-area x
        // range [40,340] (a 360-wide plot's default margins), so a pixel brush
        // inverts to a temp interval.
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::X,
            Scale::Linear {
                domain_min: 2.0,
                domain_max: 35.0,
                range_start: 40.0,
                range_end: 340.0,
            },
        );

        // Sum subscriber rows across a commit's aggregated results.
        fn subscriber_rows(aggregated: &[(String, Vec<DispatchResult>)]) -> usize {
            aggregated
                .iter()
                .flat_map(|(_, results)| results.iter())
                .map(|(_, r)| {
                    r.as_ref()
                        .map(|bs| bs.iter().map(|b| b.num_rows()).sum::<usize>())
                        .unwrap_or(0)
                })
                .sum()
        }

        // Commit a pixel brush (invert → data-space Brushing → dispatch), exactly
        // as `commit_brush` does internally, and read the resulting subscriber rows.
        let commit_pixels = |session: &mut Session, start: Point, current: Point| -> usize {
            let data_rect = invert_pixel_brush(start, current, &scales);
            let synthetic = InteractionState::Brushing {
                start: Point::new(data_rect.x0, data_rect.y0),
                current: Point::new(data_rect.x1, data_rect.y1),
            };
            let (_next, aggregated) =
                commit_brush_release_multi(&synthetic, std::slice::from_ref(&binding), session);
            subscriber_rows(&aggregated)
        };

        // --- Initial brush: pixel x∈[100,160] → temp∈[8.6,15.2] → {11,14}. ---
        let start_px = Point::new(100.0, 100.0);
        let current_px = Point::new(160.0, 200.0);
        let n1 = commit_pixels(&mut session, start_px, current_px);
        let p1 = session
            .contributor_predicate("brush", &contributor)
            .map(|p| format!("{p}"))
            .expect("the initial brush wrote the slot");
        assert_eq!(n1, 2, "temp∈[8.6,15.2] keeps 2 of 12 rows downstream");

        // --- The move/resize end-state, produced by the transform
        // (NOT hand-built corners): drag the RIGHT edge from px 160 → 280, which
        // provably crosses further datums so N2 != N1 is guaranteed. ---
        let anchor = kurbo::Rect::new(100.0, 100.0, 160.0, 200.0);
        let frame = kurbo::Rect::new(40.0, 20.0, 340.0, 280.0);
        let moved = resize_brush(
            anchor,
            BrushRegion::Edge(BrushEdge::Right),
            Point::new(280.0, 150.0),
            frame,
        );
        let end_state = InteractionState::Dragging {
            region: BrushRegion::Edge(BrushEdge::Right),
            origin: Point::new(160.0, 150.0),
            start: Point::new(moved.x0, moved.y0),
            current: Point::new(moved.x1, moved.y1),
            anchor,
        };

        // --- (b) DRIVE the production redispatch fn; feed ONLY its Brushing
        // through invert_pixel_brush → commit_brush_release_multi. ---
        let brushing =
            redispatch_brushing_from(&end_state).expect("a moved grab synthesises a Brushing");
        match &brushing {
            InteractionState::Brushing { current, .. } => assert!(
                (current.x - 280.0).abs() < f64::EPSILON,
                "the synthesised corners are the MOVED px corners, not the original"
            ),
            other => panic!("expected Brushing, got {other:?}"),
        }
        let (start2, current2) = match &brushing {
            InteractionState::Brushing { start, current } => (*start, *current),
            _ => unreachable!(),
        };
        let n2 = commit_pixels(&mut session, start2, current2);
        let p2 = session
            .contributor_predicate("brush", &contributor)
            .map(|p| format!("{p}"))
            .expect("the re-dispatch rewrote the slot");
        assert_ne!(
            p2, p1,
            "the re-dispatched predicate differs from the original"
        );
        assert_ne!(n2, n1, "the re-dispatch changes the downstream row count");
        assert_eq!(n2, 6, "temp∈[8.6,28.4] keeps 6 of 12 rows downstream");

        // --- (c) NEGATIVE CONTROL: the RAW moved Selected fed straight to the
        // unchanged release path (no redispatch synthesis) dispatches NOTHING —
        // so this test FAILS if the synthesis wire is ever dropped. ---
        let raw_selected = InteractionState::Selected {
            start: Point::new(moved.x0, moved.y0),
            current: Point::new(moved.x1, moved.y1),
        };
        let (_next, aggregated) =
            commit_brush_release_multi(&raw_selected, std::slice::from_ref(&binding), &mut session);
        assert!(
            aggregated.is_empty(),
            "a raw Selected through the unchanged path dispatches nothing (the wire is load-bearing)"
        );
    }

    /// THE LOSSLESS CHANNEL, END TO END: the same pixel gesture committed
    /// through the string path and through [`structured_brush_predicate`]
    /// filters the SAME subscriber rows — and the structured session's live
    /// contributor slot still holds the full structured clause (column, typed
    /// data-space bounds, and the scale/pixel metadata captured from the
    /// plot's live ScaleSet). Today's string path erases all of that at
    /// dispatch; this proves the structured variant carries it through the
    /// engine's selection store without changing what DuckDB returns.
    #[test]
    fn structured_brush_flows_to_query_layer_without_erasure() {
        use brightfield_engine::Engine;
        use brightfield_spec::analysis::analyse_spec;
        use brightfield_spec::{parse_spec, Format};
        use brightfield_sql::ir::ScalarValue;

        let parsed = parse_spec(DRB_BRUSH_SPEC, Format::Yaml).expect("parse");
        let analysis = analyse_spec(&parsed.spec).expect("analyse");
        let binding: BrushBinding = (&analysis.brushable_bindings[0]).into();
        let contributor = binding.contributor.0.clone();

        let load = |spec: &brightfield_spec::ast::Spec| -> Session {
            let mut session = Engine::new()
                .load_spec(spec.clone(), analyse_spec(spec).expect("analyse"), None)
                .expect("load")
                .session;
            let _ = session.execute_all();
            session
        };
        let mut session_string = load(&parsed.spec);
        let mut session_structured = load(&parsed.spec);

        // The plot's inversion scales (temp [2,35] over plot-area x [40,340]).
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::X,
            Scale::Linear {
                domain_min: 2.0,
                domain_max: 35.0,
                range_start: 40.0,
                range_end: 340.0,
            },
        );

        let start_px = Point::new(100.0, 100.0);
        let current_px = Point::new(160.0, 200.0);

        // String path — exactly what commit_brush does today.
        let data_rect = invert_pixel_brush(start_px, current_px, &scales);
        let synthetic = InteractionState::Brushing {
            start: Point::new(data_rect.x0, data_rect.y0),
            current: Point::new(data_rect.x1, data_rect.y1),
        };
        let (_next, aggregated) = commit_brush_release_multi(
            &synthetic,
            std::slice::from_ref(&binding),
            &mut session_string,
        );

        // Structured path — the same gesture through the lossless channel.
        let structured = structured_brush_predicate(start_px, current_px, &scales, &binding);
        let structured_results = session_structured.dispatch(
            &binding.selection_name,
            binding.contributor.clone(),
            structured.clone(),
        );

        // Identical filtered data on the subscriber (mark index 1).
        let rows_of = |results: &[DispatchResult]| -> usize {
            results
                .iter()
                .map(|(_, r)| {
                    r.as_ref()
                        .map(|bs| bs.iter().map(|b| b.num_rows()).sum::<usize>())
                        .unwrap_or(0)
                })
                .sum()
        };
        let string_rows: usize = aggregated.iter().map(|(_, r)| rows_of(r)).sum();
        assert_eq!(
            rows_of(&structured_results),
            string_rows,
            "string and structured forms of the same gesture filter identical rows"
        );
        assert_eq!(
            string_rows, 2,
            "pixel x∈[100,160] → temp∈[8.6,15.2] → 2 rows"
        );

        // No erasure: the engine's live slot holds the structured clause whole.
        let slot = session_structured
            .contributor_predicate(&binding.selection_name, &contributor)
            .expect("the structured dispatch wrote the slot");
        assert_eq!(
            slot, &structured,
            "the stored predicate is the dispatched structured clause, bit for bit"
        );
        match slot {
            Predicate::Interval {
                column,
                lo,
                hi,
                meta,
            } => {
                assert_eq!(column, "temp");
                assert_eq!(lo, &ScalarValue::Float(data_rect.x0));
                assert_eq!(hi, &ScalarValue::Float(data_rect.x1));
                let meta = meta.as_ref().expect("x-axis metadata captured");
                let scale = meta.scale.as_ref().expect("scale descriptor captured");
                assert_eq!(scale.kind, "linear");
                assert_eq!(scale.domain, Some((2.0, 35.0)));
                assert_eq!(scale.range, Some((40.0, 340.0)));
                assert_eq!(meta.pixel_size, Some(1.0));
            }
            other => panic!("expected the structured Interval in the slot, got {other:?}"),
        }

        // The string path's slot, by contrast, is the flattened And-of-Exprs —
        // the erasure the structured channel exists to avoid.
        match session_string
            .contributor_predicate(&binding.selection_name, &contributor)
            .expect("the string dispatch wrote the slot")
        {
            Predicate::And(clauses) => {
                assert!(clauses.iter().all(|c| matches!(c, Predicate::Expr(_))));
            }
            other => panic!("expected the flattened string form, got {other:?}"),
        }
    }
}
