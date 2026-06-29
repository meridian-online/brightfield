//! ChartState — reactive chart state for GPUI Entity wrapping.
//!
//! ChartState holds all mutable chart state: the Vello scene, interaction state,
//! navigation state, transition state, layout dimensions, and a shared
//! VelloRenderer reference. It is wrapped in `gpui::Entity<ChartState>` for
//! reactive notifications.
//!
//! ChartElement borrows from ChartState for one paint cycle. ChartState owns
//! all mutable state; ChartElement owns none.

use std::sync::{Arc, Mutex};

use kurbo::Point;
use vello::Scene;

use crate::chart_layout::ChartLayout;
use crate::interaction::{InteractionState, NavigationState};
use crate::vello_renderer::VelloRenderer;
use brightfield_render::transition::Transition;

/// Reactive chart state, wrapped in `gpui::Entity` for notifications.
///
/// Owns all mutable chart state. ChartElement borrows from this for
/// one paint cycle — it is a stateless rendering shell.
pub struct ChartState {
    /// The Vello scene containing the full chart (marks + axes + grid + legend).
    scene: Scene,
    /// Current interaction state (idle, brushing, hovering).
    interaction: InteractionState,
    /// Navigation state (pan/zoom), if navigation interactors are active.
    navigation: Option<NavigationState>,
    /// Active mark transition (data animation), if any.
    transition: Option<Transition>,
    /// Chart width in pixels.
    width: u32,
    /// Chart height in pixels.
    height: u32,
    /// Shared VelloRenderer for GPU rendering.
    renderer: Arc<Mutex<VelloRenderer>>,
    /// Layout with coordinate mapping (derived from width/height).
    layout: ChartLayout,
}

impl ChartState {
    /// Create a new ChartState.
    pub fn new(scene: Scene, width: u32, height: u32, renderer: Arc<Mutex<VelloRenderer>>) -> Self {
        Self {
            scene,
            interaction: InteractionState::Idle,
            navigation: None,
            transition: None,
            width,
            height,
            renderer,
            layout: ChartLayout::new(width as f64, height as f64),
        }
    }

    /// Access the current scene.
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// Replace the scene (e.g. after re-render on data change).
    ///
    /// After calling this, the caller should call `cx.notify()` on the
    /// entity to trigger a repaint.
    pub fn set_scene(&mut self, scene: Scene) {
        self.scene = scene;
    }

    /// Access the current interaction state.
    pub fn interaction(&self) -> &InteractionState {
        &self.interaction
    }

    /// Update the interaction state.
    pub fn set_interaction(&mut self, state: InteractionState) {
        self.interaction = state;
    }

    /// Access the navigation state.
    pub fn navigation(&self) -> Option<&NavigationState> {
        self.navigation.as_ref()
    }

    /// Set the navigation state.
    pub fn set_navigation(&mut self, nav: Option<NavigationState>) {
        self.navigation = nav;
    }

    /// Access the active transition.
    pub fn transition(&self) -> Option<&Transition> {
        self.transition.as_ref()
    }

    /// Set the active transition.
    pub fn set_transition(&mut self, transition: Option<Transition>) {
        self.transition = transition;
    }

    /// Chart width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Chart height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Update the chart dimensions (e.g. on window resize).
    pub fn set_dimensions(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.layout = ChartLayout::new(width as f64, height as f64);
    }

    /// Access the shared VelloRenderer.
    pub fn renderer(&self) -> &Arc<Mutex<VelloRenderer>> {
        &self.renderer
    }

    /// Access the chart layout for coordinate mapping.
    pub fn layout(&self) -> &ChartLayout {
        &self.layout
    }

    // --- Pointer interaction transitions ---
    //
    // These translate a window-space pointer position (with the element's
    // origin) into the chart's local plot coordinates and update the
    // interaction state. Each returns `true` when the state changed so the
    // caller can trigger a repaint. They are the single source of truth shared
    // by the live event wiring (ChartElement) and the ChartView handlers.

    /// Pointer pressed. Begins a brush if the press lands inside the plot area.
    /// Returns `true` when the interaction state changed.
    pub fn pointer_down(&mut self, window_pos: Point, element_origin: Point) -> bool {
        let local = self.layout.window_to_local(window_pos, element_origin);
        if self.layout.contains(local) {
            self.interaction = InteractionState::start_brush(local);
            true
        } else {
            false
        }
    }

    /// Pointer moved. `button_held` is whether the (left) mouse button is still
    /// down. While brushing, extends the brush to the new point — clamped to the
    /// plot area so the rect can't spill over the axes — or ends the brush if the
    /// button is no longer held (a release that never reached us). While idle or
    /// hovering, sets a hover at the point if it is inside the plot area, or
    /// clears a hover when the pointer leaves it. Returns `true` on any change.
    pub fn pointer_move(&mut self, window_pos: Point, element_origin: Point, button_held: bool) -> bool {
        let local = self.layout.window_to_local(window_pos, element_origin);
        match &self.interaction {
            InteractionState::Brushing { .. } => {
                if !button_held {
                    // The button was released without a mouse-up reaching us
                    // (focus steal, or a release outside the window). End it.
                    self.interaction = InteractionState::Idle;
                    return true;
                }
                let clamped = self.clamp_to_plot(local);
                let mut next = self.interaction.clone();
                next.update_brush(clamped);
                self.interaction = next;
                true
            }
            InteractionState::Hovering { .. } | InteractionState::Idle => {
                if self.layout.contains(local) {
                    self.interaction = InteractionState::Hovering {
                        point: local,
                        nearest: None,
                    };
                    true
                } else if matches!(self.interaction, InteractionState::Hovering { .. }) {
                    // Pointer left the plot area — drop the hover marker.
                    self.interaction = InteractionState::Idle;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Clamp a local-space point to the plot area, so a brush dragged into the
    /// margins keeps its rectangle within the data region.
    fn clamp_to_plot(&self, p: Point) -> Point {
        let area = self.layout.plot_area();
        Point::new(p.x.clamp(area.x0, area.x1), p.y.clamp(area.y0, area.y1))
    }

    /// Pointer released. Ends an active brush, returning to idle. Returns `true`
    /// when a brush was in progress. Selection dispatch (re-query) is handled
    /// separately by the app shell via `commit_brush_release`.
    pub fn pointer_up(&mut self) -> bool {
        if matches!(self.interaction, InteractionState::Brushing { .. }) {
            self.interaction = InteractionState::Idle;
            true
        } else {
            false
        }
    }
}

// ChartState must be Send for gpui::Entity.
// This is safe because all fields are Send:
// - Scene is Send
// - InteractionState is Send (Point, NearestHit are Send)
// - NavigationState is Send
// - Transition is Send
// - VelloRenderer contains wgpu types which are Send
// Compile-time assertion: ChartState must be Send for gpui::Entity.
fn _assert_chart_state_send() {
    fn _assert<T: Send>() {}
    _assert::<ChartState>();
}

#[cfg(all(test, feature = "gpu-tests"))]
mod tests {
    use super::*;
    use vello::Scene;

    // --- gmr_ac01: ChartState struct ---

    #[test]
    fn gmr_ac01_chart_state_construction() {
        let renderer = VelloRenderer::new();
        let scene = Scene::new();
        let state = ChartState::new(scene, 640, 480, renderer);

        assert_eq!(state.width(), 640);
        assert_eq!(state.height(), 480);
        assert!(matches!(state.interaction(), InteractionState::Idle));
        assert!(state.navigation().is_none());
        assert!(state.transition().is_none());
    }

    #[cfg(feature = "gpu-tests")]
    #[test]
    fn gmr_ac01_chart_state_scene_update() {
        let renderer = VelloRenderer::new();
        let mut state = ChartState::new(Scene::new(), 640, 480, renderer);

        let mut new_scene = Scene::new();
        use kurbo::{Affine, Circle};
        use peniko::{Color, Fill};
        let circle = Circle::new((100.0, 100.0), 5.0);
        new_scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::new([1.0, 0.0, 0.0, 1.0]),
            None,
            &circle,
        );
        state.set_scene(new_scene);

        let encoding = state.scene().encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "updated scene should have content"
        );
    }

    #[cfg(feature = "gpu-tests")]
    #[test]
    fn gmr_ac01_chart_state_dimensions_update() {
        let renderer = VelloRenderer::new();
        let mut state = ChartState::new(Scene::new(), 640, 480, renderer);

        state.set_dimensions(1024, 768);
        assert_eq!(state.width(), 1024);
        assert_eq!(state.height(), 768);
        assert!((state.layout().width - 1024.0).abs() < f64::EPSILON);
        assert!((state.layout().height - 768.0).abs() < f64::EPSILON);
    }

    #[cfg(feature = "gpu-tests")]
    #[test]
    fn gmr_ac01_chart_state_interaction_update() {
        let renderer = VelloRenderer::new();
        let mut state = ChartState::new(Scene::new(), 640, 480, renderer);

        state.set_interaction(InteractionState::start_brush(kurbo::Point::new(10.0, 20.0)));
        assert!(matches!(state.interaction(), InteractionState::Brushing { .. }));
    }

    #[cfg(feature = "gpu-tests")]
    #[test]
    fn pointer_down_inside_starts_brush_outside_does_not() {
        let renderer = VelloRenderer::new();
        let mut state = ChartState::new(Scene::new(), 640, 480, renderer);

        // Inside the plot area (default margins 40/20/20/30) at origin (0,0).
        assert!(state.pointer_down(Point::new(300.0, 200.0), Point::new(0.0, 0.0)));
        assert!(matches!(state.interaction(), InteractionState::Brushing { .. }));

        // In the left margin (x=10 < 40) — no brush.
        let mut state2 = ChartState::new(Scene::new(), 640, 480, VelloRenderer::new());
        assert!(!state2.pointer_down(Point::new(10.0, 200.0), Point::new(0.0, 0.0)));
        assert!(matches!(state2.interaction(), InteractionState::Idle));
    }

    #[cfg(feature = "gpu-tests")]
    #[test]
    fn pointer_move_drags_brush_and_hovers() {
        let renderer = VelloRenderer::new();
        let mut state = ChartState::new(Scene::new(), 640, 480, renderer);

        // Hover inside the plot area (no button held).
        assert!(state.pointer_move(Point::new(300.0, 200.0), Point::new(0.0, 0.0), false));
        assert!(matches!(state.interaction(), InteractionState::Hovering { .. }));

        // Begin a brush and drag with the button held — the rect tracks the pointer.
        state.pointer_down(Point::new(100.0, 100.0), Point::new(0.0, 0.0));
        assert!(state.pointer_move(Point::new(250.0, 300.0), Point::new(0.0, 0.0), true));
        let rect = state.interaction().brush_rect().expect("brushing");
        assert!((rect.x1 - 250.0).abs() < f64::EPSILON);

        // A move with the button no longer held ends the brush.
        assert!(state.pointer_move(Point::new(260.0, 310.0), Point::new(0.0, 0.0), false));
        assert!(matches!(state.interaction(), InteractionState::Idle));

        // pointer_up on an already-idle state is a no-op.
        assert!(!state.pointer_up());
    }
}
