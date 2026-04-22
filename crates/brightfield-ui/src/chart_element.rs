//! ChartElement — GPUI element wrapping a Vello-rendered chart texture.
//!
//! Renders a `vello::Scene` to a pixel buffer via wgpu, then presents it
//! as a GPUI image element. CPU readback for v1 — on Apple Silicon unified
//! memory, this is near-free (pointer cast, no actual GPU-to-CPU copy).

use vello::Scene;

use crate::interaction::InteractionState;

/// A chart element that wraps a Vello scene for display in GPUI.
///
/// In v1, the element holds a pre-rendered scene and interaction state.
/// The GPUI Element trait implementation (which requires the gpui runtime)
/// is deferred to when the full GPUI application shell is wired up.
pub struct ChartElement {
    /// The Vello scene containing the full chart (marks + axes + grid + legend).
    scene: Scene,
    /// Current interaction state (idle, brushing, hovering).
    interaction: InteractionState,
    /// Chart width in pixels.
    width: u32,
    /// Chart height in pixels.
    height: u32,
}

impl ChartElement {
    /// Create a new chart element from a Vello scene.
    pub fn new(scene: Scene, width: u32, height: u32) -> Self {
        Self {
            scene,
            interaction: InteractionState::Idle,
            width,
            height,
        }
    }

    /// Access the current scene.
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// Replace the scene (e.g. after re-render on data change).
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

    /// Chart width.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Chart height.
    pub fn height(&self) -> u32 {
        self.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vello::Scene;

    #[test]
    fn gpu_ac09_chart_element_creation() {
        let scene = Scene::new();
        let element = ChartElement::new(scene, 640, 480);
        assert_eq!(element.width(), 640);
        assert_eq!(element.height(), 480);
        assert!(matches!(element.interaction(), InteractionState::Idle));
    }

    #[test]
    fn gpu_ac09_chart_element_scene_update() {
        let mut element = ChartElement::new(Scene::new(), 640, 480);

        // Create a scene with some content.
        let mut new_scene = Scene::new();
        use kurbo::{Affine, Circle};
        use peniko::{Color, Fill};
        let circle = Circle::new((100.0, 100.0), 5.0);
        new_scene.fill(Fill::NonZero, Affine::IDENTITY, Color::new([1.0, 0.0, 0.0, 1.0]), None, &circle);

        element.set_scene(new_scene);

        let encoding = element.scene().encoding();
        assert!(encoding.path_tags.len() > 0, "updated scene should have content");
    }
}
