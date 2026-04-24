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
}
