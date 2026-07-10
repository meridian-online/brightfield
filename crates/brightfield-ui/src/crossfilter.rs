//! Live cross-filter coordinator (card 0006).
//!
//! Keeps the engine [`Session`] and the per-mark / per-plot render metadata
//! alive past the initial render, so a brush committed in the window
//! re-executes the subscriber marks and swaps their scenes in place. The window
//! holds one coordinator wrapped in `Rc<RefCell<…>>`; each plot's `ChartElement`
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

use gpui::{App, Entity};
use kurbo::{Point, Rect};
use vello::Scene;

use brightfield_engine::error::EngineError;
use brightfield_engine::{concat_batches, RecordBatch, Session};
use brightfield_render::channel::{Channel, ChannelMap};
use brightfield_render::layout::ChartLayout;
use brightfield_render::mark::{default_renderers, find_renderer, MarkRenderer};
use brightfield_render::nearest::SelectionValue;
use brightfield_render::scale::{Scale, ScaleSet};
use brightfield_render::scene::{build_multi_mark_scene_pinned, ChartData};
use brightfield_spec::analysis::{ComponentPath, LegendBinding};
use brightfield_spec::vocab::MarkKind;

use crate::brush::{point_predicate, SelectionDispatcher};
use crate::chart_state::ChartState;
use crate::chart_view::{
    commit_brush_release_multi, commit_click_multi, BrushBinding, ZERO_AREA_EPSILON,
};
use crate::interaction::InteractionState;
use crate::slider::{commit_slider_release, SliderBinding, SliderState};

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
    /// gesture (card 0006 renderer seam). Render-only: no SQL / plan-hash
    /// involvement.
    pub renderer_override: Option<Box<dyn MarkRenderer + Send + Sync>>,
}

/// One plot in the live dashboard: its identity, the marks it owns, its layout,
/// the brushes it contributes, its data scales (for inversion), and its state.
pub struct LivePlot {
    /// Stable component path (diagnostics / matching).
    pub path: String,
    /// Flat indices (into the coordinator's `marks`) of this plot's marks.
    pub mark_indices: Vec<usize>,
    /// Layout (size + margins).
    pub layout: ChartLayout,
    /// Brush bindings this plot contributes (its `intervalX/Y/XY` interactors).
    pub bindings: Vec<BrushBinding>,
    /// The plot's LAUNCH ScaleSet, captured when the coordinator was built and
    /// immutable thereafter: every gesture rebuilds through it (so the axes,
    /// colour assignments, and ramp anchoring hold still), and pixel-brush
    /// inversion reads it too — inversion and rendering stay consistent by
    /// construction (card 0006 render fidelity).
    pub scales: ScaleSet,
    /// Whether this plot draws its own inline (top-right) colour legend. `false`
    /// when a standalone `legend: color for:` node has relocated it — resolved at
    /// the app layer and carried here so a live re-render honours the same
    /// suppression instead of resurrecting the inline legend.
    pub draw_inline_legend: bool,
    /// Reactive state entity — the scene we swap when this plot is re-filtered.
    pub state: Entity<ChartState>,
}

/// A legend's selection-producer binding, UI-side (card 0009) — the mirror of
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
/// legend point selections (swatch clicks — card 0009) across a dashboard's
/// plots. All gestures re-execute subscriber marks through the same live
/// `Session` and rebuild only the affected plot scenes.
pub struct CrossfilterCoordinator {
    session: Session,
    marks: Vec<MarkInput>,
    plots: Vec<LivePlot>,
    /// Dashboard-level slider bindings (card 0005), indexed by hosted slider.
    /// A slider's subscribers may span multiple plots; the affected plots are
    /// resolved generically via `mark_to_plot` from the re-executed mark indices.
    slider_bindings: Vec<SliderBinding>,
    /// Dashboard-level legend producer bindings (card 0009), indexed by the
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

impl CrossfilterCoordinator {
    /// Build a coordinator from the live engine session and the per-mark /
    /// per-plot / slider / legend metadata assembled at startup. Returns `None`
    /// when there is nothing live to drive — no plot has a brush binding AND
    /// there are no sliders AND no bound legends — so the window skips the
    /// wiring entirely and behaves as before. A dashboard whose only
    /// interactive surface is a bound legend stays live (card 0009).
    pub fn new(
        session: Session,
        marks: Vec<MarkInput>,
        plots: Vec<LivePlot>,
        slider_bindings: Vec<SliderBinding>,
        legend_bindings: Vec<LegendSelectBinding>,
    ) -> Option<Rc<RefCell<Self>>> {
        if plots.iter().all(|p| p.bindings.is_empty())
            && slider_bindings.is_empty()
            && legend_bindings.is_empty()
        {
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
        cx: &mut App,
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
            // Invert the pixel rect to data coordinates and dispatch. Reuse the
            // shared helper by synthesising a Brushing state already in data space.
            let data_rect = invert_pixel_brush(start, current, &self.plots[plot_index].scales);
            let synthetic = InteractionState::Brushing {
                start: Point::new(data_rect.x0, data_rect.y0),
                current: Point::new(data_rect.x1, data_rect.y1),
            };
            let (_next, aggregated) =
                commit_brush_release_multi(&synthetic, &bindings, &mut self.session);
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
            state.update(cx, |s, c| {
                s.set_scene(scene);
                c.notify();
            });
        }
        true
    }

    /// Commit a slider release (card 0005): dispatch the param value into the
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
        cx: &mut App,
    ) -> bool {
        let to_rebuild = match self.apply_slider(slider_index, state) {
            Some(set) => set,
            None => return false,
        };
        for pi in to_rebuild {
            let scene = self.build_plot_scene(pi);
            let state_entity = self.plots[pi].state.clone();
            state_entity.update(cx, |s, c| {
                s.set_scene(scene);
                c.notify();
            });
        }
        true
    }

    /// The gpui-free half of [`commit_slider`]: on a `Released` state, dispatch
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

    /// Commit a legend swatch click (card 0009): drive the single-select
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
        cx: &mut App,
    ) -> bool {
        let to_rebuild = match self.apply_legend_click(legend_index, hit) {
            Some(set) => set,
            None => return false,
        };
        for pi in to_rebuild {
            let scene = self.build_plot_scene(pi);
            let state = self.plots[pi].state.clone();
            state.update(cx, |s, c| {
                s.set_scene(scene);
                c.notify();
            });
        }
        true
    }

    /// The gpui-free half of [`Self::commit_legend_click`] — the single-select
    /// toggle state machine (lcf ac-03):
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

    /// Fold re-execution results into the per-mark batch store, recording which
    /// plots need their scene rebuilt. A failed mark keeps its previous batch.
    fn absorb(
        &mut self,
        results: Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>,
        to_rebuild: &mut HashSet<usize>,
    ) {
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
    /// rendered against the plot's LAUNCH scales — never re-inferring from the
    /// filtered batch. `LivePlot.scales` is immutable after construction, so
    /// this reads it without writing it back: the axes the rebuild draws and
    /// the scales a subsequent brush inverts against stay consistent by
    /// construction (card 0006 render fidelity).
    fn build_plot_scene(&self, plot_index: usize) -> Scene {
        let plot = &self.plots[plot_index];
        render_plot_scene(
            &self.marks,
            &self.renderers,
            &plot.mark_indices,
            &plot.layout,
            plot.draw_inline_legend,
            &plot.scales,
        )
    }
}

/// Rebuild one plot's scene from its marks against the LAUNCH `scales`,
/// independent of any `self`/`Entity` state so it is unit-testable headlessly.
///
/// Renders through [`build_multi_mark_scene_pinned`]: the passed `scales` are
/// the plot's launch set, so no inference / `augment_scales` / zero-baseline /
/// view-extent runs — the axes, colour assignments, and ramp anchoring the
/// first render established hold still while only the data moves.
///
/// Each mark dispatches to its own `renderer_override` (the scheme/attribute-
/// configured renderer its FIRST render used — raster/heatmap/cell scheme,
/// heatmap/contour bandwidth, contour thresholds), falling back to the shared
/// registry for an unconfigured mark. This is the single renderer-config seam
/// the first render and every live rebuild share (card 0006), closing the
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
    scales: &ScaleSet,
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
    build_multi_mark_scene_pinned(&refs, draw_inline_legend, scales)
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

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_render::mark::configured_renderer;
    use brightfield_render::scale::SequentialScheme;
    use brightfield_render::scene::build_multi_mark_scene;

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
        build_multi_mark_scene(&refs, true).1
    }

    /// A fingerprint of a scene's geometry (`path_data`: the packed coordinates
    /// of every dot / line / tick) and colours (`draw_data`: every fill / stroke
    /// paint). Two scenes with equal fingerprints are pixel-identical; a
    /// re-fitted axis moves `path_data`, a re-anchored ramp moves `draw_data`.
    fn scene_bytes(scene: &Scene) -> (Vec<u32>, Vec<u32>) {
        let e = scene.encoding();
        (e.path_data.clone(), e.draw_data.clone())
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
        }];
        let renderers = default_renderers();
        let layout = ChartLayout::new(400.0, 300.0);
        // The launch scales (inferred once) carry the Fill Sequential; both
        // rebuilds render against them.
        let scales = launch_scales(&marks, &renderers, &[0], &layout);

        let with_legend = render_plot_scene(&marks, &renderers, &[0], &layout, true, &scales);
        let without_legend = render_plot_scene(&marks, &renderers, &[0], &layout, false, &scales);
        assert!(
            count_scene_paths(&with_legend) > count_scene_paths(&without_legend),
            "the inline gradient legend adds paths when draw_inline_legend is true \
             ({} with vs {} without)",
            count_scene_paths(&with_legend),
            count_scene_paths(&without_legend),
        );
    }

    /// fww_ac06 (card 0016, reworked onto the renderer-override seam): the live
    /// rebuild keeps the plot's declared colorScheme because the scheme now rides
    /// the mark's `renderer_override` (`configured_renderer`) — the SAME seam the
    /// first render uses — not a deleted `LivePlot.scheme` field. A raster mark
    /// whose override is the blues renderer yields a launch Fill Sequential with
    /// the blues ramp (its `augment_scales` carries the scheme); the default
    /// override stays viridis. Render-only: no SQL / plan-hash.
    #[test]
    fn fww_ac06_live_rebuild_uses_declared_scheme() {
        let (batch, channels) = grid_batch();
        let renderers = default_renderers();
        let layout = ChartLayout::new(400.0, 300.0);

        let stops_for = |scheme: SequentialScheme| {
            let marks = vec![MarkInput {
                batch: Some(batch.clone()),
                channels: channels.clone(),
                kind: MarkKind::Raster,
                renderer_override: configured_renderer(MarkKind::Raster, scheme, None, None),
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

    /// cfr_ac01 (launch-pinned scales): after a legend click filters the
    /// subscriber, a rebuild renders against the plot's LAUNCH scales — never
    /// re-inferring from the filtered batch, which would shrink the domain and
    /// jump the axes. The pinned rebuild differs from the old re-inferring
    /// build. (`build_plot_scene` reads the immutable `LivePlot.scales` and
    /// `render_plot_scene` returns no scales, so there is structurally nothing
    /// to write back — the stored set cannot drift.)
    #[test]
    fn cfr_ac01_rebuild_pins_launch_scales_not_reinferred() {
        let coord = legend_toggle_coordinator();
        let mut c = coord.borrow_mut();
        let layout = ChartLayout::new(360.0, 300.0);

        // Subscriber = mark 1 (the `filterBy: $sel` dot plot). Its launch scales
        // span the full batch (x in 1..6).
        let launch = launch_scales(&c.marks, &c.renderers, &[1], &layout);
        let launch_x = launch.get(Channel::X).and_then(|s| s.domain_max()).unwrap();

        // Filter to gentoo → 3 of 6 rows (x in 3..5).
        assert!(c.apply_legend_click(0, Some("gentoo")).is_some());
        assert_eq!(c.marks[1].batch.as_ref().unwrap().num_rows(), 3);

        // Re-inferring over the now-filtered batch yields a NARROWER x-domain —
        // exactly the axis jump pinning suppresses.
        let reinferred = launch_scales(&c.marks, &c.renderers, &[1], &layout);
        let reinferred_x = reinferred.get(Channel::X).and_then(|s| s.domain_max()).unwrap();
        assert!(
            reinferred_x < launch_x,
            "re-inference would shrink the x-domain: {reinferred_x} < {launch_x}"
        );

        // The pinned rebuild renders the filtered batch against the LAUNCH
        // scales; the old behaviour rendered against the re-inferred scales — a
        // visibly different scene (axes + point positions moved).
        let pinned = render_plot_scene(&c.marks, &c.renderers, &[1], &layout, true, &launch);
        let reinferred_scene =
            render_plot_scene(&c.marks, &c.renderers, &[1], &layout, true, &reinferred);
        assert_ne!(
            scene_bytes(&pinned),
            scene_bytes(&reinferred_scene),
            "pinned vs re-inferred rebuilds differ — the axes would have jumped"
        );
    }

    /// cfr_ac02 (all-channel pinning — colour): a `fill:species` scatter
    /// filtered to one species still encodes that species' LAUNCH palette
    /// colour, not the palette[0] a single-category re-inference would assign.
    /// Categorical Fill rides the same launch-pinned `ScaleSet` as x/y.
    #[test]
    fn cfr_ac02_filtered_fill_keeps_launch_colour() {
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
                Arc::new(StringArray::from(vec!["adelie", "chinstrap", "gentoo", "gentoo"])),
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
        }];
        let launch = launch_scales(&full_marks, &renderers, &[0], &layout);

        // The launch colour for gentoo (index 2) differs from the colour a
        // filtered-batch re-inference would give it (index 0).
        let filtered_marks = vec![MarkInput {
            batch: Some(filtered),
            channels,
            kind: MarkKind::Dot,
            renderer_override: None,
        }];
        let reinferred = launch_scales(&filtered_marks, &renderers, &[0], &layout);
        let launch_gentoo = launch.get(Channel::Fill).unwrap().map_colour("gentoo").unwrap();
        let reinferred_gentoo = reinferred.get(Channel::Fill).unwrap().map_colour("gentoo").unwrap();
        assert_ne!(
            launch_gentoo, reinferred_gentoo,
            "re-inference recolours gentoo (palette[2] launch vs palette[0] alone)"
        );

        // The pinned rebuild of the filtered batch encodes the LAUNCH gentoo
        // colour — not the re-inferred one. Rendered WITHOUT the inline legend
        // so the colour probe sees only the dots: the swatch legend would draw
        // every category (including adelie at palette[0] == reinferred_gentoo),
        // masking whether a dot was recoloured.
        let pinned = render_plot_scene(&filtered_marks, &renderers, &[0], &layout, false, &launch);
        let drawn: std::collections::HashSet<u32> =
            pinned.encoding().draw_data.iter().copied().collect();
        assert!(
            drawn.contains(&packed(launch_gentoo)),
            "the filtered dots keep the launch palette colour for gentoo"
        );
        assert!(
            !drawn.contains(&packed(reinferred_gentoo)),
            "the filtered dots are NOT recoloured to the re-inferred single-category colour"
        );
    }

    /// cfr_ac03 (round-trip identity — the crown invariant): a gesture sequence
    /// that returns the engine to unfiltered state rebuilds a scene byte-identical
    /// to launch. Any residual re-inference (item 1) or renderer-config loss
    /// (item 3) would break the equality. Checked for BOTH a plain dot plot
    /// (via a real legend toggle) and a blues + bandwidth heatmap (via a
    /// batch-swap round-trip through its configured override).
    #[test]
    fn cfr_ac03_round_trip_returns_to_launch_scene() {
        // --- Dot plot: real session, select then toggle off. ---
        {
            let coord = legend_toggle_coordinator();
            let mut c = coord.borrow_mut();
            let layout = ChartLayout::new(360.0, 300.0);
            let launch = launch_scales(&c.marks, &c.renderers, &[1], &layout);
            let launch_scene =
                render_plot_scene(&c.marks, &c.renderers, &[1], &layout, true, &launch);

            assert!(c.apply_legend_click(0, Some("gentoo")).is_some());
            assert_eq!(c.marks[1].batch.as_ref().unwrap().num_rows(), 3);
            // Toggle the same category off → back to the full 6-row result.
            assert!(c.apply_legend_click(0, Some("gentoo")).is_some());
            assert_eq!(c.marks[1].batch.as_ref().unwrap().num_rows(), 6);

            let rebuilt = render_plot_scene(&c.marks, &c.renderers, &[1], &layout, true, &launch);
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

            let override_of = || configured_renderer(MarkKind::Heatmap, SequentialScheme::Blues, Some(0.8), None);
            let marks_for = |batch: RecordBatch| {
                vec![MarkInput {
                    batch: Some(batch),
                    channels: channels.clone(),
                    kind: MarkKind::Heatmap,
                    renderer_override: override_of(),
                }]
            };

            let launch_marks = marks_for(full.clone());
            let launch = launch_scales(&launch_marks, &renderers, &[0], &layout);
            let launch_scene =
                render_plot_scene(&launch_marks, &renderers, &[0], &layout, true, &launch);

            // Filter (rebuild against the pinned launch scales + same override).
            let filtered_marks = marks_for(filtered);
            let filtered_scene =
                render_plot_scene(&filtered_marks, &renderers, &[0], &layout, true, &launch);
            assert_ne!(
                scene_bytes(&filtered_scene),
                scene_bytes(&launch_scene),
                "the filter visibly changes the heatmap"
            );

            // Return to the full batch → byte-identical to launch (scheme,
            // bandwidth, and scales all held).
            let restored_marks = marks_for(full);
            let restored_scene =
                render_plot_scene(&restored_marks, &renderers, &[0], &layout, true, &launch);
            assert_eq!(
                scene_bytes(&restored_scene),
                scene_bytes(&launch_scene),
                "heatmap round-trip returns to a byte-identical launch scene"
            );
        }
    }

    /// cfr_ac04 (live renderer-config seam): a mark rebuilds through its
    /// `renderer_override` — the SAME configured renderer its first render used.
    /// A blues heatmap keeps blues stops; explicit bandwidth changes the render
    /// vs Silverman; contour keeps its thresholds (more levels ⇒ more iso-line
    /// paths); a dot mark with no override still renders through the registry.
    #[test]
    fn cfr_ac04_live_rebuild_uses_configured_renderer() {
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
            ),
        }];
        match launch_scales(&blues, &renderers, &[0], &layout).get(Channel::Fill) {
            Some(Scale::Sequential { stops, .. }) => {
                assert_eq!(*stops, SequentialScheme::Blues.stops(), "heatmap keeps blues stops")
            }
            other => panic!("expected a Fill Sequential, got {other:?}"),
        }

        // (b) Explicit bandwidth renders differently from Silverman (its default).
        let render_bw = |bandwidth: Option<f64>| {
            let marks = vec![MarkInput {
                batch: Some(batch.clone()),
                channels: channels.clone(),
                kind: MarkKind::Heatmap,
                renderer_override: configured_renderer(
                    MarkKind::Heatmap,
                    SequentialScheme::default(),
                    bandwidth,
                    None,
                ),
            }];
            let scales = launch_scales(&marks, &renderers, &[0], &layout);
            render_plot_scene(&marks, &renderers, &[0], &layout, true, &scales)
        };
        assert_ne!(
            scene_bytes(&render_bw(Some(2.0))),
            scene_bytes(&render_bw(None)),
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
                ),
            }];
            let scales = launch_scales(&marks, &renderers, &[0], &layout);
            count_scene_paths(&render_plot_scene(&marks, &renderers, &[0], &layout, true, &scales))
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
        }];
        let scales = launch_scales(&dot, &renderers, &[0], &layout);
        assert!(
            count_scene_paths(&render_plot_scene(&dot, &renderers, &[0], &layout, true, &scales)) > 0,
            "an unconfigured dot mark still renders via the registry"
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

    // slw ac-06/ac-07 (card 0005): commit_slider's data path — a Released value
    // re-executes the subscribing mark (row count changes); a Dragging value is a
    // no-op. An empty `plots` vec means no gpui App is needed: the rebuild loop
    // has nothing to repaint, and we assert on the swapped batch directly.
    #[test]
    fn slw_ac06_apply_slider_reexecutes_on_release_only() {
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
            })
            .collect();

        let binding = SliderBinding {
            param_name: "threshold".to_string(),
            min: 0.0,
            max: 6.0,
            step: Some(1.0),
        };
        let coord = CrossfilterCoordinator::new(session, marks, vec![], vec![binding], vec![])
            .expect("a slider binding keeps the coordinator alive with no brushes");
        let mut c = coord.borrow_mut();

        // threshold=2 default → x in {3,4,5,6} = 4 rows.
        let before = c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows());
        assert_eq!(before, 4);

        // ac-07: mid-drag never commits.
        assert!(
            c.apply_slider(0, &SliderState::Dragging { value: 5.0 })
                .is_none(),
            "Dragging is a no-op"
        );
        assert_eq!(c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows()), before);

        // ac-06: release at threshold=5 re-executes → x in {6} = 1 row.
        let rebuilt = c.apply_slider(0, &SliderState::Released { value: 5.0 });
        assert!(rebuilt.is_some(), "Released commits");
        let after = c.marks[0].batch.as_ref().map_or(0, |b| b.num_rows());
        assert!(after < before, "raising threshold drops rows: {before} -> {after}");
        assert_eq!(after, 1);
    }

    // slw ac-09 (card 0005): the shipped example ties the whole chain together —
    // its `input: slider` yields a binding via the layout join, and committing a
    // higher threshold through the coordinator drops points.
    #[test]
    fn slw_ac09_example_slider_drives_the_mark() {
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
            })
            .collect();

        let coord = CrossfilterCoordinator::new(session, marks, vec![], vec![binding], vec![])
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
    // lcf ac-03 (card 0009): legend single-select toggle through the
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
    fn legend_toggle_coordinator() -> Rc<RefCell<CrossfilterCoordinator>> {
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
            })
            .collect();

        CrossfilterCoordinator::new(session, marks, vec![], vec![], legend_bindings)
            .expect("lcf_ac03 liveness: a bound legend alone keeps the coordinator alive")
    }

    /// The predicate the legend's contributor slot currently holds, read
    /// through the same engine lookup `apply_legend_click` decides from.
    fn slot_expr(c: &CrossfilterCoordinator) -> Option<String> {
        let b = &c.legend_bindings[0];
        c.session
            .contributor_predicate(&b.selection_name, &b.contributor.0)
            .map(|p| format!("{p:?}"))
    }

    /// lcf_ac03: the toggle state machine drives dispatch/clear through the
    /// coordinator against a real session — new category filters the
    /// downstream mark, a different category switches, the same category
    /// clears, and an empty-panel click clears (or no-ops when nothing is
    /// selected). The subscriber's batch (mark 1) is the observable; the
    /// engine's contributor slot (not a UI mirror) is the toggle state.
    #[test]
    fn lcf_ac03_legend_toggle_state_machine_dispatches_and_clears() {
        let coord = legend_toggle_coordinator();
        let mut c = coord.borrow_mut();
        let rows = |c: &CrossfilterCoordinator| {
            c.marks[1].batch.as_ref().map_or(0, |b| b.num_rows())
        };
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
        assert!(c.apply_legend_click(0, None).is_some(), "empty click clears");
        assert_eq!(slot_expr(&c), None);
        assert_eq!(rows(&c), baseline);

        // Out-of-range legend index: no-op.
        assert!(c.apply_legend_click(9, Some("gentoo")).is_none());
    }

    /// Regression (card 0009 F1a): the `(selection, contributor)` slot is
    /// shared with the `for:`-plot's brush/point interactors. After a brush
    /// replaces the legend's dispatched predicate, clicking the SAME swatch
    /// again must DISPATCH (replacing the brush with the category predicate)
    /// — a UI-side selected-category mirror would instead clear, inverting
    /// the toggle.
    #[test]
    fn lcf_f1a_brush_replacing_the_slot_does_not_invert_the_toggle() {
        use brightfield_sql::ir::Predicate;

        let coord = legend_toggle_coordinator();
        let mut c = coord.borrow_mut();
        let rows = |c: &CrossfilterCoordinator| {
            c.marks[1].batch.as_ref().map_or(0, |b| b.num_rows())
        };

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

    /// Regression (card 0009 F1b): after the slot is emptied behind the
    /// legend's back (an empty plot click clears this contributor), the same
    /// swatch must dispatch again in ONE click — a mirror still holding the
    /// category would treat it as a toggle-off no-op round trip.
    #[test]
    fn lcf_f1b_external_clear_does_not_eat_the_next_swatch_click() {
        let coord = legend_toggle_coordinator();
        let mut c = coord.borrow_mut();
        let rows = |c: &CrossfilterCoordinator| {
            c.marks[1].batch.as_ref().map_or(0, |b| b.num_rows())
        };
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
}
