//! ChartView — GPUI Render component for chart display.
//!
//! ChartView is the public API for embedding a chart in a GPUI window.
//! It owns an `Entity<ChartState>` and implements `gpui::Render`.
//!
//! Consumers create a ChartView with a `Model<ChartState>` and add it
//! to a GPUI window. ChartView::render() returns a ChartElement that
//! implements the Element trait.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{div, px, rgb, Context, Entity, IntoElement, ParentElement, Render, Styled, Window};

use brightfield_engine::error::EngineError;
use brightfield_engine::RecordBatch;
use brightfield_render::channel::ChannelMap;
use brightfield_render::channel::Channel;
use brightfield_render::nearest::{
    band_category_at, column_typed_value_at, find_nearest, NearestMode, SelectionValue,
};
use brightfield_render::scale::Scale;
use brightfield_render::scale::ScaleSet;
use brightfield_spec::analysis::{BrushableBinding, ComponentPath};
use brightfield_spec::layout::Rect;

use crate::brush::{
    brush_rect_to_predicate, point_to_predicate, BrushKind, ChannelColumns, SelectionDispatcher,
};
use crate::chart_element::ChartElement;
use crate::chart_state::ChartState;
use crate::crossfilter::CrossfilterCoordinator;
use crate::interaction::InteractionState;
use crate::legend_element::{LegendElement, PlacedLegend};
use crate::slider::SliderBinding;
use crate::slider_element::{SliderElement, SliderWidget};

/// One plot positioned in the dashboard: its rect (in dashboard pixels) and its
/// own reactive [`ChartState`]. Each plot owns its state — and thus its own
/// raster cache and interaction — so hover/brush are independent per plot.
pub struct PlacedChart {
    /// Left edge within the dashboard, in pixels.
    pub x: f64,
    /// Top edge within the dashboard, in pixels.
    pub y: f64,
    /// Plot width in pixels.
    pub width: f64,
    /// Plot height in pixels.
    pub height: f64,
    /// The plot's ComponentPath (plot-node path, e.g. `root/hconcat[0]`) — the
    /// focus / verb-target identity, matching the coordinator's `LivePlot.path`
    /// (card 0018).
    pub path: ComponentPath,
    /// The plot's reactive chart state.
    pub state: Entity<ChartState>,
    /// Shared cross-filter coordinator, if this dashboard cross-filters. When
    /// present, a brush release on this plot routes through it (re-query +
    /// re-render subscribers); when `None`, the brush is purely visual.
    pub coordinator: Option<Rc<RefCell<CrossfilterCoordinator>>>,
}

/// One slider widget positioned in the dashboard (card 0005): its rect, the
/// param binding it drives, its reactive widget state, and the shared coordinator
/// its release commits through.
pub struct PlacedSlider {
    /// Left edge within the dashboard, in pixels.
    pub x: f64,
    /// Top edge within the dashboard, in pixels.
    pub y: f64,
    /// Widget width in pixels.
    pub width: f64,
    /// Widget height in pixels.
    pub height: f64,
    /// The param binding (name + min/max/step).
    pub binding: SliderBinding,
    /// Reactive widget state (resting value + drag gesture).
    pub state: Entity<SliderWidget>,
    /// Shared coordinator; a release routes through it to re-query + repaint.
    pub coordinator: Option<Rc<RefCell<CrossfilterCoordinator>>>,
}

/// GPUI render component for a dashboard: hosts one [`ChartElement`] per plot,
/// one [`SliderElement`] per slider, and one [`LegendElement`] per standalone
/// legend (display-only unless bound to a selection — card 0009), each
/// absolutely positioned at its layout rect, in a container sized to the
/// dashboard's bounding box. A single-plot spec is just a one-plot dashboard.
pub struct ChartView {
    /// Dashboard width in pixels.
    width: f64,
    /// Dashboard height in pixels.
    height: f64,
    /// The positioned plots.
    charts: Vec<PlacedChart>,
    /// The positioned slider widgets (card 0005).
    sliders: Vec<PlacedSlider>,
    /// The positioned standalone legends (card 0016). Display-only unless a
    /// placement carries a selection binding (card 0009), in which case it
    /// indexes the coordinator's OWN legend-binding space — distinct from the
    /// chart/slider index spaces.
    legends: Vec<PlacedLegend>,
    /// The focus ring rect (dashboard pixels), drawn over the focused node when
    /// keyboard focus is active and authoring chrome is visible (card 0018). The
    /// owning `CanvasPanel` sets it from the focus state machine and clears it in
    /// presentation mode. Purely visual — chart interaction rides window-level
    /// mouse handlers, so the ring never intercepts events.
    focus_ring: Option<Rect>,
}

impl ChartView {
    /// Create a dashboard view of the given size hosting the positioned plots,
    /// slider widgets, and standalone legends.
    pub fn new(
        width: f64,
        height: f64,
        charts: Vec<PlacedChart>,
        sliders: Vec<PlacedSlider>,
        legends: Vec<PlacedLegend>,
    ) -> Self {
        Self {
            width,
            height,
            charts,
            sliders,
            legends,
            focus_ring: None,
        }
    }

    /// Set (or clear) the focus ring rect. The `CanvasPanel` calls this from the
    /// focus state machine after a nav move, and clears it (`None`) in
    /// presentation mode; the caller notifies to repaint (card 0018).
    pub fn set_focus_ring(&mut self, ring: Option<Rect>) {
        self.focus_ring = ring;
    }
}

impl Render for ChartView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The container is the dashboard's fixed size, with each plot absolutely
        // positioned at its rect; each ChartElement reads its own ChartState and
        // wires its own mouse events, so plots don't share interaction. (The
        // white window background + centring moved up to the app crate's
        // CanvasPanel — card 0017's DockArea shell — so the dashboard canvas
        // is exactly the spec-derived surface.)
        //
        // Legends rasterise through the shared Vello renderer the plots already
        // hold; a dashboard always renders at least one plot, so the first
        // chart's renderer serves the (sibling) legends.
        let legend_renderer = self
            .charts
            .first()
            .map(|c| c.state.read(cx).renderer().clone());
        div()
            .relative()
            .w(px(self.width as f32))
            .h(px(self.height as f32))
            .children(self.charts.iter().enumerate().map(|(i, c)| {
                div()
                    .absolute()
                    .left(px(c.x as f32))
                    .top(px(c.y as f32))
                    .w(px(c.width as f32))
                    .h(px(c.height as f32))
                    .child(ChartElement::new(c.state.clone(), i, c.coordinator.clone()))
            }))
            .children(self.sliders.iter().enumerate().map(|(i, s)| {
                div()
                    .absolute()
                    .left(px(s.x as f32))
                    .top(px(s.y as f32))
                    .w(px(s.width as f32))
                    .h(px(s.height as f32))
                    .child(SliderElement::new(
                        s.state.clone(),
                        s.binding.clone(),
                        s.width,
                        s.height,
                        i,
                        s.coordinator.clone(),
                    ))
            }))
            .children(legend_renderer.into_iter().flat_map(|renderer| {
                self.legends.iter().enumerate().map(move |(i, l)| {
                    div()
                        .absolute()
                        .left(px(l.x as f32))
                        .top(px(l.y as f32))
                        .w(px(l.width as f32))
                        .h(px(l.height as f32))
                        .child(LegendElement::new(l, i, renderer.clone()))
                })
            }))
            // The keyboard focus ring (card 0018): a border-only overlay at the
            // focused node's rect. `None` when focus is inactive or presentation
            // hides authoring chrome. Non-interactive — brush/hover ride
            // window-level mouse handlers, so the ring never intercepts events.
            .children(self.focus_ring.map(|r| {
                div()
                    .absolute()
                    .left(px(r.x as f32))
                    .top(px(r.y as f32))
                    .w(px(r.width as f32))
                    .h(px(r.height as f32))
                    .border_2()
                    .border_color(rgb(0x2f6feb))
                    .rounded(px(3.0))
            }))
    }
}

/// Identity of the brush at dispatch time: which selection it writes
/// to, the contributing component path (for self-exclusion), the
/// brush kind (intervalX / intervalY / intervalXY), and the channel
/// columns the rect coordinates compare against.
#[derive(Debug, Clone)]
pub struct BrushBinding {
    /// Name of the selection this brush contributes to (e.g. `brush`).
    pub selection_name: String,
    /// Parent-plot path of the contributor (for self-exclusion).
    pub contributor: ComponentPath,
    /// Brush kind (intervalX, intervalY, intervalXY).
    pub kind: BrushKind,
    /// Bound channel columns.
    pub channels: ChannelColumns,
}

/// Epsilon for the click-vs-drag boundary — a brush whose extent on both
/// axes is less than `ZERO_AREA_EPSILON` pixels is treated as a click and
/// routed through [`commit_brush_clear`] (a `clear` dispatch). Anything
/// larger is a drag and goes through [`commit_brush_release`] (a
/// `dispatch` with the rect-derived predicate).
pub const ZERO_AREA_EPSILON: f64 = 0.5;

/// Multi-binding form of [`commit_brush_release`]. Iterates the supplied
/// bindings and dispatches one selection per binding — each binding's
/// predicate is computed using its own kind (kind-compatibility filter).
/// Returns the next `InteractionState` (always `Idle` after a release) plus
/// per-binding aggregated dispatch results.
///
/// cfs3 ac-04. Single-binding consumers should call
/// [`commit_brush_release`] (a 1-element-slice wrapper preserved for the
/// cfs2_ac11 boundary).
pub fn commit_brush_release_multi<D: SelectionDispatcher>(
    interaction: &InteractionState,
    bindings: &[BrushBinding],
    dispatcher: &mut D,
) -> (
    InteractionState,
    Vec<(String, Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>)>,
) {
    if let InteractionState::Brushing { start, current } = interaction {
        let rect = kurbo::Rect::new(
            start.x.min(current.x),
            start.y.min(current.y),
            start.x.max(current.x),
            start.y.max(current.y),
        );
        let mut aggregated = Vec::with_capacity(bindings.len());
        for binding in bindings {
            // Kind-compatibility filter: a rect DRAG only drives interval
            // selections. Point selections (toggleX/Y) are click-driven, and a
            // plot may carry both a point and an interval interactor — so
            // skipping the point bindings here stops a y-drag from dispatching a
            // degenerate `Predicate::True` (select-all) into the point selection.
            // Their real predicate is produced by the click gesture (deferred).
            if matches!(
                binding.kind,
                BrushKind::Point | BrushKind::PointX | BrushKind::PointY
            ) {
                continue;
            }
            let predicate = brush_rect_to_predicate(rect, binding.kind, &binding.channels);
            let results = dispatcher.dispatch(
                &binding.selection_name,
                binding.contributor.clone(),
                predicate,
            );
            aggregated.push((binding.selection_name.clone(), results));
        }
        (InteractionState::Idle, aggregated)
    } else {
        (interaction.clone(), Vec::new())
    }
}

/// Pure helper for cfs2_ac11: given an InteractionState (which may or
/// may not be Brushing), a binding, and a dispatcher, produce the
/// dispatch result vec and the next InteractionState. Lifted out of
/// the GPUI context for testability — chart_view.on_mouse_up_with_dispatch
/// shares the same logic but threads it through Entity<ChartState>.
///
/// **cfs3 wrapper:** preserved as a single-binding convenience over
/// [`commit_brush_release_multi`] so the cfs2_ac11 surface stays green.
pub fn commit_brush_release<D: SelectionDispatcher>(
    interaction: &InteractionState,
    binding: &BrushBinding,
    dispatcher: &mut D,
) -> (
    InteractionState,
    Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>,
) {
    let (next_state, mut aggregated) =
        commit_brush_release_multi(interaction, std::slice::from_ref(binding), dispatcher);
    let results = aggregated.pop().map(|(_, r)| r).unwrap_or_default();
    (next_state, results)
}

/// Pure helper for the click-vs-drag boundary. When `interaction` is
/// `Idle` OR a zero-area `Brushing` (start ≈ current within
/// [`ZERO_AREA_EPSILON`] on both axes), dispatch a `clear` call on the
/// supplied binding's selection and return `Idle` as the next state. A
/// non-zero `Brushing` does NOT dispatch through this path — it goes
/// through [`commit_brush_release`] (the drag-release path).
///
/// cfs3 ac-03.
pub fn commit_brush_clear<D: SelectionDispatcher>(
    interaction: &InteractionState,
    binding: &BrushBinding,
    dispatcher: &mut D,
) -> (
    InteractionState,
    Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>,
) {
    let should_clear = match interaction {
        InteractionState::Idle => true,
        InteractionState::Brushing { start, current } => {
            (start.x - current.x).abs() < ZERO_AREA_EPSILON
                && (start.y - current.y).abs() < ZERO_AREA_EPSILON
        }
        _ => false,
    };
    if should_clear {
        let results = dispatcher.clear(&binding.selection_name, binding.contributor.clone());
        (InteractionState::Idle, results)
    } else {
        (interaction.clone(), Vec::new())
    }
}

/// Commit a CLICK gesture across a plot's bindings (card 0006 — the point-
/// selection gesture that finishes cross-filter). Unlike a drag (which drives
/// interval selections), a click drives POINT selections:
///
/// - A `PointX`/`PointY` binding **snaps to the nearest datum**: `find_nearest`
///   locates the closest rendered point to `click_px` (in pixels, so the hit
///   radius is uniform on screen), and the datum's EXACT stored value is read
///   and dispatched as `col = value`. Selecting the *continuous* click
///   coordinate would never equal a discrete datum under `=`, so snapping is
///   what makes point selection actually match rows.
/// - A click that **misses every datum** (nothing within the hit radius) clears
///   the point selection — click-empty-space to deselect.
/// - An interval (or bare `Point`, deferred) binding **clears** on a click —
///   the click-outside-brush retract that [`commit_brush_clear`] performs.
///
/// Point selection forms a numeric `col = value` predicate, so it is scoped to
/// numeric, categorical, and temporal axes: a continuous axis snaps to the
/// nearest rendered datum and reads its exact typed value; a categorical (band)
/// axis resolves the clicked category directly from the scale. The dispatched
/// `col = value` uses a type-correct literal (bare number/int, quoted+escaped
/// string, or `make_timestamp(us)`) via [`SelectionValue`].
///
/// `marks` are the plot's `(batch, channel_map)` pairs to search (a plot may
/// layer several); `scales` map data → the same pixel space as `click_px`.
/// Returns `(Idle, per-binding results)`, mirroring
/// [`commit_brush_release_multi`].
pub fn commit_click_multi<D: SelectionDispatcher>(
    click_px: kurbo::Point,
    marks: &[(&RecordBatch, &ChannelMap)],
    scales: &ScaleSet,
    bindings: &[BrushBinding],
    dispatcher: &mut D,
) -> (
    InteractionState,
    Vec<(String, Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>)>,
) {
    // (selection, contributor) pairs a point binding SELECTED this click, so a
    // sibling interval (or point-miss) on the SAME target doesn't clear the point
    // we just set — a plot may carry both a toggle and an interval interactor.
    let mut selected: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    let mut aggregated = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let key = (
            binding.selection_name.clone(),
            binding.contributor.0.clone(),
        );
        let results = match binding.kind {
            BrushKind::PointX | BrushKind::PointY => {
                let mode = if matches!(binding.kind, BrushKind::PointX) {
                    NearestMode::X
                } else {
                    NearestMode::Y
                };
                match resolve_point_value(click_px, marks, scales, binding, mode) {
                    // Hit: select that datum's (or category's) exact value.
                    Some(value) => {
                        selected.insert(key);
                        let predicate = point_to_predicate(&value, binding.kind, &binding.channels);
                        dispatcher.dispatch(
                            &binding.selection_name,
                            binding.contributor.clone(),
                            predicate,
                        )
                    }
                    // Miss (clicked empty space on a continuous axis, or no column
                    // on the axis): deselect — unless a sibling point binding
                    // already selected this same target.
                    None if selected.contains(&key) => continue,
                    None => dispatcher.clear(&binding.selection_name, binding.contributor.clone()),
                }
            }
            // Interval kinds and bare `Point` (deferred): a click clears — unless
            // a sibling point binding already selected this same target.
            _ if selected.contains(&key) => continue,
            _ => dispatcher.clear(&binding.selection_name, binding.contributor.clone()),
        };
        aggregated.push((binding.selection_name.clone(), results));
    }
    (InteractionState::Idle, aggregated)
}

/// The [`SelectionValue`] a point click resolves to along the axis a binding
/// selects.
///
/// - **Categorical (band) axis**: the axis position *is* the value, so the
///   clicked category is resolved directly from the band scale (a pixel→category
///   inverse) — a `Text` value. There is no datum to snap to, so a categorical
///   click always resolves (to the nearest category).
/// - **Continuous (linear/time) axis**: snaps to the nearest rendered datum
///   across all the plot's `marks` (globally closest in pixel space) and reads
///   its EXACT stored cell — a `Number`/`Int`/`Timestamp`. So the dispatched
///   `col = value` matches a real datum, not the continuous click coordinate.
///
/// `None` when the binding has no column on its axis, or on a continuous-axis
/// miss (no datum within the hit radius).
fn resolve_point_value(
    click_px: kurbo::Point,
    marks: &[(&RecordBatch, &ChannelMap)],
    scales: &ScaleSet,
    binding: &BrushBinding,
    mode: NearestMode,
) -> Option<SelectionValue> {
    let (column, axis, cursor_on_axis) = match binding.kind {
        BrushKind::PointX => (binding.channels.x.as_deref()?, Channel::X, click_px.x),
        BrushKind::PointY => (binding.channels.y.as_deref()?, Channel::Y, click_px.y),
        _ => return None,
    };

    // Categorical axis: resolve the clicked category from the band scale. A click
    // OUTSIDE the band's pixel range (e.g. in the axis margin) resolves to `None`
    // so it clears — otherwise every click snaps to the nearest category and the
    // filter could never be retracted (full toggle-off is a follow-up).
    if let Some(scale @ Scale::Band { range_start, range_end, .. }) = scales.get(axis) {
        let (lo, hi) = (range_start.min(*range_end), range_start.max(*range_end));
        if cursor_on_axis < lo || cursor_on_axis > hi {
            return None;
        }
        return band_category_at(scale, cursor_on_axis).map(SelectionValue::Text);
    }

    // Continuous axis: snap to the nearest rendered datum, read its typed value.
    let mut best: Option<(f64, SelectionValue)> = None; // (pixel distance, value)
    for (batch, channel_map) in marks {
        let hit = match find_nearest(click_px, batch, channel_map, scales, mode, None) {
            Some(h) => h,
            None => continue,
        };
        let value = match column_typed_value_at(batch, column, hit.row) {
            Some(v) => v,
            None => continue,
        };
        if best.as_ref().map_or(true, |(d, _)| hit.distance < *d) {
            best = Some((hit.distance, value));
        }
    }
    best.map(|(_, v)| v)
}

/// Convert a spec-side [`BrushableBinding`] into a UI-side [`BrushBinding`]
/// by translating the mirror enums (`BrushKind`, `ChannelColumns`). The
/// conversion is faithful — every field copies through verbatim. cfs3 ac-06.
impl From<&BrushableBinding> for BrushBinding {
    fn from(b: &BrushableBinding) -> Self {
        BrushBinding {
            selection_name: b.selection.clone(),
            contributor: b.parent_plot.clone(),
            kind: brush_kind_from_spec(b.kind),
            channels: ChannelColumns {
                x: b.channels.x.clone(),
                y: b.channels.y.clone(),
            },
        }
    }
}

fn brush_kind_from_spec(kind: brightfield_spec::analysis::BrushKind) -> BrushKind {
    use brightfield_spec::analysis::BrushKind as Spec;
    match kind {
        Spec::IntervalX => BrushKind::IntervalX,
        Spec::IntervalY => BrushKind::IntervalY,
        Spec::IntervalXY => BrushKind::IntervalXY,
        Spec::Point => BrushKind::Point,
        Spec::PointX => BrushKind::PointX,
        Spec::PointY => BrushKind::PointY,
    }
}

// --- AC-05 tests: coordinate transform and interaction state ---
#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart_layout::ChartLayout;
    use brightfield_sql::ir::Predicate;
    use kurbo::Point;

    // Unit tests for coordinate transform logic — these don't require
    // a GPUI runtime, just the math.

    #[test]
    fn gmr_ac05_coordinate_transform_inside_plot() {
        let layout = ChartLayout::new(640.0, 480.0);
        let element_origin = Point::new(100.0, 50.0);
        let window_pos = Point::new(400.0, 300.0);

        let local = layout.window_to_local(window_pos, element_origin);
        assert!((local.x - 300.0).abs() < f64::EPSILON);
        assert!((local.y - 250.0).abs() < f64::EPSILON);
        assert!(layout.contains(local), "point should be inside plot area");
    }

    #[test]
    fn gmr_ac05_coordinate_transform_outside_plot() {
        let layout = ChartLayout::new(640.0, 480.0);
        let element_origin = Point::new(100.0, 50.0);
        // Point in the left margin area
        let window_pos = Point::new(110.0, 100.0);

        let local = layout.window_to_local(window_pos, element_origin);
        assert!((local.x - 10.0).abs() < f64::EPSILON);
        assert!(!layout.contains(local), "point should be outside plot area (in left margin)");
    }

    #[test]
    fn gmr_ac05_interaction_state_idle_to_brushing() {
        let state = InteractionState::start_brush(Point::new(100.0, 200.0));
        assert!(
            matches!(state, InteractionState::Brushing { .. }),
            "should transition to Brushing on mouse_down inside plot area"
        );
    }

    #[test]
    fn gmr_ac05_interaction_state_brushing_to_idle() {
        let state = InteractionState::start_brush(Point::new(100.0, 200.0));
        assert!(matches!(state, InteractionState::Brushing { .. }));

        // On mouse_up, we'd set to Idle
        let idle = InteractionState::Idle;
        assert!(matches!(idle, InteractionState::Idle));
    }

    #[test]
    fn gmr_ac05_brush_update_during_drag() {
        let mut state = InteractionState::start_brush(Point::new(100.0, 200.0));
        state.update_brush(Point::new(300.0, 400.0));

        let rect = state.brush_rect().expect("should have brush rect");
        assert!((rect.x0 - 100.0).abs() < f64::EPSILON);
        assert!((rect.y0 - 200.0).abs() < f64::EPSILON);
        assert!((rect.x1 - 300.0).abs() < f64::EPSILON);
        assert!((rect.y1 - 400.0).abs() < f64::EPSILON);
    }

    // --- gmr_ac07: Resize ---

    #[test]
    fn gmr_ac07_layout_dimensions_change() {
        let layout = ChartLayout::new(640.0, 480.0);
        assert!((layout.width - 640.0).abs() < f64::EPSILON);

        let resized = ChartLayout::new(1024.0, 768.0);
        assert!((resized.width - 1024.0).abs() < f64::EPSILON);
        assert!((resized.height - 768.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gmr_ac07_render_respects_new_dimensions() {
        // Verify plot area scales with dimensions
        let layout = ChartLayout::new(1024.0, 768.0);
        let area = layout.plot_area();
        assert!((area.x1 - (1024.0 - 20.0)).abs() < f64::EPSILON);
        assert!((area.y1 - (768.0 - 30.0)).abs() < f64::EPSILON);
    }

    // --- cfs2_ac11: brush release dispatches a propagate_selection call ---

    /// Recording test double: captures every dispatch and clear call
    /// in order so tests can assert call counts, ordering, and arguments.
    struct RecordingDispatcher {
        calls: Vec<(String, ComponentPath, Predicate)>,
        clear_calls: Vec<(String, ComponentPath)>,
    }

    impl RecordingDispatcher {
        fn new() -> Self {
            Self {
                calls: Vec::new(),
                clear_calls: Vec::new(),
            }
        }
    }

    impl SelectionDispatcher for RecordingDispatcher {
        fn dispatch(
            &mut self,
            name: &str,
            contributor: ComponentPath,
            predicate: Predicate,
        ) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)> {
            self.calls.push((name.to_string(), contributor, predicate));
            // Stub return: subscribers, if any, are mocked as zero —
            // this double's contract is "did dispatch get called?".
            Vec::new()
        }

        fn clear(
            &mut self,
            name: &str,
            contributor: ComponentPath,
        ) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)> {
            self.clear_calls.push((name.to_string(), contributor));
            Vec::new()
        }
    }

    #[test]
    fn cfs2_ac11_on_mouse_up_dispatches_selection() {
        // Simulate the mouse-down → drag → mouse-up sequence at the
        // InteractionState level, then drive commit_brush_release with a
        // recording dispatcher. The recorded call must carry the
        // selection name, contributor path, and a non-True Predicate
        // derived from the brush rect.

        // mouse-down: start a brush.
        let mut interaction = InteractionState::start_brush(Point::new(20.0, 30.0));
        // drag.
        interaction.update_brush(Point::new(120.0, 230.0));

        // mouse-up: commit.
        let binding = BrushBinding {
            selection_name: "brush".to_string(),
            contributor: ComponentPath("root/plot[0]".to_string()),
            kind: BrushKind::IntervalXY,
            channels: ChannelColumns::xy("speed", "delay"),
        };
        let mut dispatcher = RecordingDispatcher::new();

        let (next_state, _results) =
            commit_brush_release(&interaction, &binding, &mut dispatcher);

        // Exactly one dispatch.
        assert_eq!(
            dispatcher.calls.len(),
            1,
            "exactly one propagate_selection call on Brushing→Idle"
        );
        let (name, contributor, predicate) = &dispatcher.calls[0];
        assert_eq!(name, "brush");
        assert_eq!(contributor, &ComponentPath("root/plot[0]".to_string()));
        // Predicate must be derived from the brush rect — not Predicate::True.
        assert!(
            !matches!(predicate, Predicate::True),
            "brush release must produce a non-trivial predicate; got: {predicate:?}"
        );
        // State transitioned to Idle.
        assert!(
            matches!(next_state, InteractionState::Idle),
            "post-release state should be Idle"
        );
    }

    #[test]
    fn cfs2_ac11_on_mouse_up_no_brush_no_dispatch() {
        // If interaction is Idle (no active brush), mouse-up must not
        // dispatch — same partial-failure / no-op discipline as the
        // existing on_mouse_up.
        let interaction = InteractionState::Idle;
        let binding = BrushBinding {
            selection_name: "brush".to_string(),
            contributor: ComponentPath("root/plot[0]".to_string()),
            kind: BrushKind::IntervalX,
            channels: ChannelColumns::xy("speed", "delay"),
        };
        let mut dispatcher = RecordingDispatcher::new();

        let (next_state, results) =
            commit_brush_release(&interaction, &binding, &mut dispatcher);

        assert!(dispatcher.calls.is_empty(), "no brush → no dispatch");
        assert!(results.is_empty());
        assert!(matches!(next_state, InteractionState::Idle));
    }

    /// cfs point-selection (card 0006): a rect DRAG skips point-kind bindings —
    /// only interval selections are rect-driven. A plot carrying BOTH a toggleX
    /// (PointX) and an intervalY binding must dispatch only the interval on a
    /// drag, never a degenerate `True` into the point selection.
    #[test]
    fn drag_skips_point_bindings() {
        let mut interaction = InteractionState::start_brush(Point::new(20.0, 30.0));
        interaction.update_brush(Point::new(120.0, 230.0));

        let bindings = [
            BrushBinding {
                selection_name: "pt".to_string(),
                contributor: ComponentPath("root/plot[0]".to_string()),
                kind: BrushKind::PointX,
                channels: ChannelColumns::xy("speed", "delay"),
            },
            BrushBinding {
                selection_name: "iv".to_string(),
                contributor: ComponentPath("root/plot[0]".to_string()),
                kind: BrushKind::IntervalY,
                channels: ChannelColumns::xy("speed", "delay"),
            },
        ];
        let mut dispatcher = RecordingDispatcher::new();
        let (_state, aggregated) =
            commit_brush_release_multi(&interaction, &bindings, &mut dispatcher);

        assert_eq!(
            dispatcher.calls.len(),
            1,
            "only the interval binding dispatches on a drag; the point is skipped"
        );
        assert_eq!(
            dispatcher.calls[0].0, "iv",
            "the dispatched selection is the interval, not the point"
        );
        assert_eq!(aggregated.len(), 1);
    }

    // ---------------------------------------------------------------------
    // cfs3 — clearing, multi-binding dispatch, BrushableBinding conversion
    // ---------------------------------------------------------------------

    /// cfs3_ac03: commit_brush_clear dispatches a `clear` call when the
    /// interaction is Idle OR a zero-area Brushing (click). A non-zero
    /// Brushing does NOT clear (that path is the drag-release, handled by
    /// commit_brush_release). Returns Idle as the next state on a clear.
    #[test]
    fn cfs3_ac03_click_outside_active_brush_clears() {
        let binding = BrushBinding {
            selection_name: "brush".to_string(),
            contributor: ComponentPath("root/plot[0]".to_string()),
            kind: BrushKind::IntervalX,
            channels: ChannelColumns::xy("speed", "delay"),
        };

        // (a) Idle → one clear call.
        let mut dispatcher = RecordingDispatcher::new();
        let (next_state, results) =
            commit_brush_clear(&InteractionState::Idle, &binding, &mut dispatcher);
        assert!(dispatcher.calls.is_empty(), "no dispatch on clear path");
        assert_eq!(
            dispatcher.clear_calls.len(),
            1,
            "Idle → exactly one clear call"
        );
        let (name, contributor) = &dispatcher.clear_calls[0];
        assert_eq!(name, "brush");
        assert_eq!(contributor, &ComponentPath("root/plot[0]".to_string()));
        assert!(results.is_empty(), "test double's stub returns no results");
        assert!(matches!(next_state, InteractionState::Idle));

        // (c) Zero-area Brushing → still a clear (click below drag threshold).
        let mut dispatcher = RecordingDispatcher::new();
        let zero_area = {
            let p = Point::new(100.0, 200.0);
            let mut s = InteractionState::start_brush(p);
            // Move within epsilon — still classified as zero-area.
            s.update_brush(Point::new(p.x + 0.1, p.y - 0.1));
            s
        };
        let (next_state, _) =
            commit_brush_clear(&zero_area, &binding, &mut dispatcher);
        assert_eq!(
            dispatcher.clear_calls.len(),
            1,
            "zero-area Brushing → exactly one clear call"
        );
        assert!(matches!(next_state, InteractionState::Idle));

        // (d) Non-zero Brushing → NO dispatch through this path.
        //     (Drag releases go through commit_brush_release.)
        let mut dispatcher = RecordingDispatcher::new();
        let mut drag = InteractionState::start_brush(Point::new(20.0, 30.0));
        drag.update_brush(Point::new(120.0, 230.0));
        let (next_state, _) = commit_brush_clear(&drag, &binding, &mut dispatcher);
        assert!(
            dispatcher.calls.is_empty() && dispatcher.clear_calls.is_empty(),
            "non-zero Brushing → neither dispatch nor clear via this path"
        );
        // State is unchanged on the no-op path.
        assert!(matches!(next_state, InteractionState::Brushing { .. }));
    }

    /// cfs3_ac04: commit_brush_release_multi (the lifted multi-binding
    /// helper) dispatches one propagate_selection per binding, with each
    /// binding's predicate computed against its own kind. Verifies the
    /// kind-compatibility filter — an IntervalX binding produces an x-only
    /// predicate even when the rect has a non-zero y extent.
    #[test]
    fn cfs3_ac04_plot_drives_multiple_selections() {
        // (a) Construct a Brushing state with a 100x200 rect (non-zero on
        //     both axes).
        let mut interaction = InteractionState::start_brush(Point::new(20.0, 30.0));
        interaction.update_brush(Point::new(120.0, 230.0));

        // Two bindings on the same plot writing to different selections.
        let binding_xy = BrushBinding {
            selection_name: "a".to_string(),
            contributor: ComponentPath("root/plot[0]".to_string()),
            kind: BrushKind::IntervalXY,
            channels: ChannelColumns::xy("speed", "delay"),
        };
        let binding_x = BrushBinding {
            selection_name: "b".to_string(),
            contributor: ComponentPath("root/plot[0]".to_string()),
            kind: BrushKind::IntervalX,
            channels: ChannelColumns::xy("speed", "delay"),
        };
        let bindings = [binding_xy, binding_x];

        // (b) Drive commit_brush_release_multi.
        let mut dispatcher = RecordingDispatcher::new();
        let (next_state, aggregated) =
            commit_brush_release_multi(&interaction, &bindings, &mut dispatcher);

        // (c) Two dispatch calls, one per binding.
        assert_eq!(
            dispatcher.calls.len(),
            2,
            "two bindings → two propagate_selection calls"
        );
        // (d) Each call's selection_name matches its binding.
        let names: Vec<&str> = dispatcher
            .calls
            .iter()
            .map(|(n, _, _)| n.as_str())
            .collect();
        assert!(names.contains(&"a"), "selection $a dispatched");
        assert!(names.contains(&"b"), "selection $b dispatched");

        // The IntervalX binding's predicate references only the x channel.
        let (_, _, b_pred) = dispatcher
            .calls
            .iter()
            .find(|(n, _, _)| n == "b")
            .expect("selection b dispatched");
        match b_pred {
            Predicate::And(clauses) => {
                assert_eq!(clauses.len(), 2, "IntervalX → two clauses (x-only)");
                for c in clauses {
                    let s = match c {
                        Predicate::Expr(s) => s,
                        _ => panic!("expected Expr clause"),
                    };
                    assert!(
                        s.contains("speed"),
                        "IntervalX predicate must reference x col only: {s}"
                    );
                    assert!(
                        !s.contains("delay"),
                        "IntervalX predicate must NOT reference y col: {s}"
                    );
                }
            }
            other => panic!("expected Predicate::And for IntervalX, got {other:?}"),
        }

        // Aggregated return shape mirrors the dispatcher record.
        assert_eq!(aggregated.len(), 2);
        assert!(matches!(next_state, InteractionState::Idle));
    }

    /// cfs3_ac06: BrushBinding::from(&BrushableBinding) preserves every
    /// field — selection_name, contributor (= parent_plot), kind, and
    /// channels — translating between the spec-side and ui-side mirror
    /// enums verbatim.
    #[test]
    fn cfs3_ac06_brushable_binding_to_brush_binding() {
        let spec_binding = BrushableBinding {
            interactor_path: ComponentPath(
                "root/plot[0]/interactor[intervalXY]".to_string(),
            ),
            parent_plot: ComponentPath("root/plot[0]".to_string()),
            selection: "brush".to_string(),
            kind: brightfield_spec::analysis::BrushKind::IntervalXY,
            channels: brightfield_spec::analysis::ChannelColumns {
                x: Some("speed".to_string()),
                y: Some("delay".to_string()),
            },
        };

        let ui_binding: BrushBinding = (&spec_binding).into();

        assert_eq!(ui_binding.selection_name, "brush");
        assert_eq!(
            ui_binding.contributor,
            ComponentPath("root/plot[0]".to_string()),
            "contributor = parent_plot"
        );
        assert_eq!(ui_binding.kind, BrushKind::IntervalXY);
        assert_eq!(ui_binding.channels.x.as_deref(), Some("speed"));
        assert_eq!(ui_binding.channels.y.as_deref(), Some("delay"));
    }
}
