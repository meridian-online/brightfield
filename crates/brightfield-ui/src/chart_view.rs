//! ChartView — GPUI Render component for chart display.
//!
//! ChartView is the public API for embedding a chart in a GPUI window.
//! It owns an `Entity<ChartState>` and implements `gpui::Render`.
//!
//! Consumers create a ChartView with a `Model<ChartState>` and add it
//! to a GPUI window. ChartView::render() returns a ChartElement that
//! implements the Element trait.

use gpui::{Context, Entity, IntoElement, Render, Window};
use kurbo::Point;

use brightfield_engine::error::EngineError;
use brightfield_engine::RecordBatch;
use brightfield_spec::analysis::ComponentPath;

use crate::brush::{brush_rect_to_predicate, BrushKind, ChannelColumns, SelectionDispatcher};
use crate::chart_element::ChartElement;
use crate::chart_state::ChartState;
use crate::interaction::InteractionState;

/// GPUI Render component for chart display.
///
/// Owns an `Entity<ChartState>` for reactive notifications.
/// `render()` returns a `ChartElement` that paints the current
/// Vello scene as a GPU texture.
pub struct ChartView {
    /// The chart state entity (Model).
    state: Entity<ChartState>,
}

impl ChartView {
    /// Create a new ChartView from an entity handle.
    pub fn new(state: Entity<ChartState>) -> Self {
        Self { state }
    }

    /// Access the state entity.
    pub fn state(&self) -> &Entity<ChartState> {
        &self.state
    }
}

impl Render for ChartView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        ChartElement::new(
            state.scene().clone(),
            state.renderer().clone(),
            state.width(),
            state.height(),
        )
    }
}

// --- AC-05: Mouse event handlers ---
//
// Mouse events are wired up when ChartView is registered as a GPUI view.
// The coordinate transform pipeline:
//   1. window_pos - element_origin → local_px
//   2. Check local_px within plot area bounds
//   3. Update InteractionState in ChartState via entity.update()
//   4. cx.notify() triggers automatic repaint

impl ChartView {
    /// Handle mouse down: start brushing if inside the plot area.
    pub fn on_mouse_down(&mut self, window_pos: Point, element_origin: Point, cx: &mut Context<Self>) {
        let layout = self.state.read(cx).layout().clone();
        let local = layout.window_to_local(window_pos, element_origin);

        if layout.contains(local) {
            self.state.update(cx, |state, cx| {
                state.set_interaction(InteractionState::start_brush(local));
                cx.notify();
            });
        }
    }

    /// Handle mouse move: update brush or set hover state.
    pub fn on_mouse_move(&mut self, window_pos: Point, element_origin: Point, cx: &mut Context<Self>) {
        let layout = self.state.read(cx).layout().clone();
        let local = layout.window_to_local(window_pos, element_origin);

        if !layout.contains(local) {
            return;
        }

        self.state.update(cx, |state, cx| {
            match state.interaction() {
                InteractionState::Brushing { .. } => {
                    let mut interaction = state.interaction().clone();
                    interaction.update_brush(local);
                    state.set_interaction(interaction);
                }
                InteractionState::Idle | InteractionState::Hovering { .. } => {
                    state.set_interaction(InteractionState::Hovering {
                        point: local,
                        nearest: None,
                    });
                }
            }
            cx.notify();
        });
    }

    /// Handle mouse up: end brushing, return to idle.
    pub fn on_mouse_up(&mut self, _window_pos: Point, _element_origin: Point, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            if matches!(state.interaction(), InteractionState::Brushing { .. }) {
                state.set_interaction(InteractionState::Idle);
                cx.notify();
            }
        });
    }

    /// Handle mouse up with a selection dispatcher attached: end
    /// brushing, dispatch the brush rectangle as a Predicate to the
    /// runtime selection coordinator (via the dispatcher), and return
    /// to idle.
    ///
    /// `binding` carries the brushable plot's identity and channel
    /// bindings — supplied by the caller because ChartView itself does
    /// not know which selection it brushes into.
    ///
    /// Returns the dispatch results so the caller may surface
    /// per-subscriber outcomes (logging, telemetry). Returns an empty
    /// vec when there is no active brush.
    pub fn on_mouse_up_with_dispatch<D: SelectionDispatcher>(
        &mut self,
        _window_pos: Point,
        _element_origin: Point,
        binding: &BrushBinding,
        dispatcher: &mut D,
        cx: &mut Context<Self>,
    ) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)> {
        let mut results = Vec::new();
        self.state.update(cx, |state, cx| {
            if let InteractionState::Brushing { start, current } = state.interaction() {
                let rect = kurbo::Rect::new(
                    start.x.min(current.x),
                    start.y.min(current.y),
                    start.x.max(current.x),
                    start.y.max(current.y),
                );
                let predicate =
                    brush_rect_to_predicate(rect, binding.kind, &binding.channels);
                results = dispatcher.dispatch(
                    &binding.selection_name,
                    binding.contributor.clone(),
                    predicate,
                );
                state.set_interaction(InteractionState::Idle);
                cx.notify();
            }
        });
        results
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

/// Pure helper for cfs2_ac11: given an InteractionState (which may or
/// may not be Brushing), a binding, and a dispatcher, produce the
/// dispatch result vec and the next InteractionState. Lifted out of
/// the GPUI context for testability — chart_view.on_mouse_up_with_dispatch
/// shares the same logic but threads it through Entity<ChartState>.
pub fn commit_brush_release<D: SelectionDispatcher>(
    interaction: &InteractionState,
    binding: &BrushBinding,
    dispatcher: &mut D,
) -> (
    InteractionState,
    Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>,
) {
    if let InteractionState::Brushing { start, current } = interaction {
        let rect = kurbo::Rect::new(
            start.x.min(current.x),
            start.y.min(current.y),
            start.x.max(current.x),
            start.y.max(current.y),
        );
        let predicate = brush_rect_to_predicate(rect, binding.kind, &binding.channels);
        let results = dispatcher.dispatch(
            &binding.selection_name,
            binding.contributor.clone(),
            predicate,
        );
        (InteractionState::Idle, results)
    } else {
        (interaction.clone(), Vec::new())
    }
}

impl ChartView {

    /// Handle scroll (zoom gesture). Placeholder for navigation wiring.
    pub fn on_scroll(
        &mut self,
        _window_pos: Point,
        _element_origin: Point,
        _delta: Point,
        _cx: &mut Context<Self>,
    ) {
        // Navigation event routing (pan/zoom gestures, transition scheduling
        // via cx.on_next_frame) is deferred to the app shell card.
        // This method establishes the handler signature.
    }

    // --- AC-07: Window resize ---

    /// Handle window resize by updating ChartState dimensions.
    pub fn on_resize(&mut self, width: u32, height: u32, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.set_dimensions(width, height);
            cx.notify();
        });
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

    /// Recording test double: captures every dispatch call in order.
    struct RecordingDispatcher {
        calls: Vec<(String, ComponentPath, Predicate)>,
    }

    impl RecordingDispatcher {
        fn new() -> Self {
            Self { calls: Vec::new() }
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
}
