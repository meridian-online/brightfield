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
use brightfield_render::mark::{default_renderers, find_renderer, MarkRenderer, RasterRenderer};
use brightfield_render::nearest::SelectionValue;
use brightfield_render::scale::{Scale, ScaleSet, SequentialScheme};
use brightfield_render::scene::{build_multi_mark_scene, ChartData};
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
    /// Data scales for inverting a pixel brush back to data coordinates.
    pub scales: ScaleSet,
    /// Whether this plot draws its own inline (top-right) colour legend. `false`
    /// when a standalone `legend: color for:` node has relocated it — resolved at
    /// the app layer and carried here so a live re-render honours the same
    /// suppression instead of resurrecting the inline legend.
    pub draw_inline_legend: bool,
    /// The plot's declared `colorScheme` (default viridis), applied to its
    /// raster marks — resolved at the app layer (like `draw_inline_legend`) and
    /// carried here so a live rebuild constructs the same scheme-configured
    /// `RasterRenderer` the first render used (card 0016, closing #36's
    /// live-path parity gap). Render-only: no SQL / plan-hash involvement.
    pub scheme: SequentialScheme,
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
    /// hosted `LegendElement` carries.
    legend_bindings: Vec<LegendSelectBinding>,
    /// Per legend binding: the currently toggled category (single-select
    /// toggle state). `None` = no category selected.
    legend_selected: Vec<Option<String>>,
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
        let legend_selected = vec![None; legend_bindings.len()];
        Some(Rc::new(RefCell::new(Self {
            session,
            marks,
            plots,
            slider_bindings,
            legend_bindings,
            legend_selected,
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
    /// - a NEW (or different) category dispatches `column = 'category'` via
    ///   [`SelectionValue::Text`]'s quoted+escaped literal;
    /// - the SAME category clears (toggle off);
    /// - an empty-panel click clears whatever was selected;
    /// - an empty-panel click with nothing selected is a no-op (`None`).
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
        let selected = self.legend_selected.get(legend_index)?.clone();
        let results = match hit {
            // A new or different category: single-select — the fresh
            // predicate REPLACES this contributor's previous one (the store
            // is keyed by contributor), so no interim clear is needed.
            Some(cat) if selected.as_deref() != Some(cat) => {
                let predicate = point_predicate(
                    &binding.column,
                    &SelectionValue::Text(cat.to_string()).literal(),
                );
                self.legend_selected[legend_index] = Some(cat.to_string());
                self.session.dispatch(
                    &binding.selection_name,
                    binding.contributor.clone(),
                    predicate,
                )
            }
            // The same category again: toggle off.
            Some(_) => {
                self.legend_selected[legend_index] = None;
                self.session
                    .clear(&binding.selection_name, binding.contributor.clone())
            }
            // Empty-panel click with a live selection: clear it.
            None if selected.is_some() => {
                self.legend_selected[legend_index] = None;
                self.session
                    .clear(&binding.selection_name, binding.contributor.clone())
            }
            // Empty-panel click with nothing selected: no-op.
            None => return None,
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

    /// Rebuild one plot's scene from the current batches of all its marks, and
    /// refresh the plot's stored `scales` to the freshly inferred ones so a
    /// subsequent brush on this plot inverts against the data it now shows.
    fn build_plot_scene(&mut self, plot_index: usize) -> Scene {
        // Own the inputs up front so no `self.plots` borrow is held across the
        // later `self.plots[..].scales = …` write.
        let mark_indices = self.plots[plot_index].mark_indices.clone();
        let layout = self.plots[plot_index].layout.clone();
        let draw_inline_legend = self.plots[plot_index].draw_inline_legend;
        let scheme = self.plots[plot_index].scheme;

        let (scene, scales) = render_plot_scene(
            &self.marks,
            &self.renderers,
            &mark_indices,
            &layout,
            draw_inline_legend,
            scheme,
        );
        self.plots[plot_index].scales = scales;
        scene
    }
}

/// Rebuild one plot's scene from its marks, independent of any `self`/`Entity`
/// state so it is unit-testable headlessly. `draw_inline_legend` mirrors the
/// app's first-render suppression (a standalone `legend: color for:` relocates
/// the inline legend), so a live re-render honours the same choice rather than
/// resurrecting the inline legend — which now matters because every raster plot
/// carries a Fill (Sequential) scale and would otherwise grow a gradient bar
/// after the first brush. `scheme` likewise mirrors the plot's resolved
/// `colorScheme`: a raster mark renders through a scheme-configured
/// [`RasterRenderer`] (matching the headless first render) instead of the
/// registry's viridis default (card 0016).
fn render_plot_scene(
    marks: &[MarkInput],
    renderers: &[(MarkKind, Box<dyn MarkRenderer + Send + Sync>)],
    mark_indices: &[usize],
    layout: &ChartLayout,
    draw_inline_legend: bool,
    scheme: SequentialScheme,
) -> (Scene, ScaleSet) {
    let raster = RasterRenderer { scheme };
    let chart_data: Vec<ChartData<'_>> = mark_indices
        .iter()
        .filter_map(|&mi| {
            let m = marks.get(mi)?;
            let batch = m.batch.as_ref()?;
            let renderer: &dyn MarkRenderer = if m.kind == MarkKind::Raster {
                &raster
            } else {
                find_renderer(renderers, m.kind)?
            };
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
    build_multi_mark_scene(&refs, draw_inline_legend)
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

    /// Regression (review finding): a live plot re-render honours the app's
    /// inline-legend suppression instead of hardcoding it on. `render_plot_scene`
    /// (the Entity-free core `build_plot_scene` calls) draws the inline gradient
    /// legend for a raster plot when `draw_inline_legend` is true, and omits it
    /// when false — so a suppressed raster plot doesn't grow a gradient bar after
    /// the first brush.
    #[test]
    fn render_plot_scene_honours_inline_legend_suppression() {
        use arrow::array::Float64Array;
        use arrow::datatypes::{DataType, Field, Schema};
        use brightfield_render::mark::count_scene_paths;
        use std::sync::Arc;

        // A minimal raster batch (x_bin, y_bin, __bf_count); its augment_scales
        // builds a Fill Sequential, so the inline legend is a gradient bar.
        let schema = Arc::new(Schema::new(vec![
            Field::new("x_bin", DataType::Float64, false),
            Field::new("y_bin", DataType::Float64, false),
            Field::new("__bf_count", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0])),
                Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0])),
                Arc::new(Float64Array::from(vec![1.0, 5.0, 9.0])),
            ],
        )
        .unwrap();
        let mut channels = ChannelMap::new();
        channels.insert(Channel::X, "x_bin".to_string());
        channels.insert(Channel::Y, "y_bin".to_string());
        let marks = vec![MarkInput {
            batch: Some(batch),
            channels,
            kind: MarkKind::Raster,
        }];
        let renderers = default_renderers();
        let layout = ChartLayout::new(400.0, 300.0);

        let (with_legend, _) =
            render_plot_scene(&marks, &renderers, &[0], &layout, true, SequentialScheme::default());
        let (without_legend, _) =
            render_plot_scene(&marks, &renderers, &[0], &layout, false, SequentialScheme::default());
        assert!(
            count_scene_paths(&with_legend) > count_scene_paths(&without_legend),
            "the inline gradient legend adds paths when draw_inline_legend is true \
             ({} with vs {} without)",
            count_scene_paths(&with_legend),
            count_scene_paths(&without_legend),
        );
    }

    /// fww_ac06 (card 0016): the live rebuild honours the plot's declared
    /// colorScheme. Driving the Entity-free `render_plot_scene` seam for a
    /// raster mark with `SequentialScheme::Blues` yields a Fill Sequential
    /// whose stops are the blues ramp — matching the headless first render —
    /// while the default stays viridis. Render-only: the scheme rides
    /// `LivePlot` (like `draw_inline_legend`), touching no SQL or plan-hash.
    #[test]
    fn fww_ac06_live_rebuild_uses_declared_scheme() {
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
                Arc::new(Float64Array::from(vec![0.0, 1.0])),
                Arc::new(Float64Array::from(vec![0.0, 1.0])),
                Arc::new(Float64Array::from(vec![2.0, 9.0])),
            ],
        )
        .unwrap();
        let mut channels = ChannelMap::new();
        channels.insert(Channel::X, "x_bin".to_string());
        channels.insert(Channel::Y, "y_bin".to_string());
        let marks = vec![MarkInput {
            batch: Some(batch),
            channels,
            kind: MarkKind::Raster,
        }];
        let renderers = default_renderers();
        let layout = ChartLayout::new(400.0, 300.0);

        let stops_for = |scheme: SequentialScheme| {
            let (_, scales) =
                render_plot_scene(&marks, &renderers, &[0], &layout, true, scheme);
            match scales.get(Channel::Fill) {
                Some(Scale::Sequential { stops, .. }) => stops.clone(),
                other => panic!("expected a Fill Sequential, got {other:?}"),
            }
        };

        assert_eq!(
            stops_for(SequentialScheme::Blues),
            SequentialScheme::Blues.stops(),
            "a blues plot rebuilds with the blues ramp, not the registry default"
        );
        assert_eq!(
            stops_for(SequentialScheme::default()),
            SequentialScheme::Viridis.stops(),
            "the default scheme remains viridis"
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
            })
            .collect();

        CrossfilterCoordinator::new(session, marks, vec![], vec![], legend_bindings)
            .expect("lcf_ac03 liveness: a bound legend alone keeps the coordinator alive")
    }

    /// lcf_ac03: the toggle state machine drives dispatch/clear through the
    /// coordinator against a real session — new category filters the
    /// downstream mark, a different category switches, the same category
    /// clears, and an empty-panel click clears (or no-ops when nothing is
    /// selected). The subscriber's batch (mark 1) is the observable.
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
        assert_eq!(c.legend_selected[0].as_deref(), Some("gentoo"));
        assert_eq!(rows(&c), 3, "species = 'gentoo' keeps 3 rows");

        // DIFFERENT: click 'adelie' → switches, no stacking → 2 rows.
        assert!(c.apply_legend_click(0, Some("adelie")).is_some());
        assert_eq!(c.legend_selected[0].as_deref(), Some("adelie"));
        assert_eq!(rows(&c), 2, "switching selects only the new category");

        // SAME: click 'adelie' again → toggle off → all rows restored.
        assert!(c.apply_legend_click(0, Some("adelie")).is_some());
        assert_eq!(c.legend_selected[0], None);
        assert_eq!(rows(&c), baseline, "toggle-off restores the full result");

        // EMPTY after a select: select then click empty panel → cleared.
        assert!(c.apply_legend_click(0, Some("chinstrap")).is_some());
        assert_eq!(rows(&c), 1);
        assert!(c.apply_legend_click(0, None).is_some(), "empty click clears");
        assert_eq!(c.legend_selected[0], None);
        assert_eq!(rows(&c), baseline);

        // Out-of-range legend index: no-op.
        assert!(c.apply_legend_click(9, Some("gentoo")).is_none());
    }
}
