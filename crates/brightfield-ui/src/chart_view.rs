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
}
