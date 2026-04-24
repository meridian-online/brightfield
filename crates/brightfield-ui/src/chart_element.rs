//! ChartElement — stateless GPUI Element shell for Vello chart rendering.
//!
//! ChartElement is a lightweight rendering shell that borrows scene and
//! renderer from ChartState for one paint cycle. It owns no mutable state.
//!
//! Lifecycle:
//! - `request_layout()` — returns a fixed-size layout matching ChartState dimensions
//! - `prepaint()` — registers a hitbox covering the element bounds
//! - `paint()` — renders the Vello scene to pixels and paints as a GPUI image

use std::sync::{Arc, Mutex};

use gpui::{
    App, Bounds, Corners, Element, ElementId, GlobalElementId, HitboxBehavior,
    InspectorElementId, IntoElement, LayoutId, Pixels, RenderImage, Size, Style, Window, px,
};
use image::RgbaImage;
use smallvec::SmallVec;
use vello::Scene;

use crate::vello_renderer::VelloRenderer;

/// Stateless chart element for one GPUI paint cycle.
///
/// Created by `ChartView::render()` each frame. Borrows the scene and
/// renderer from ChartState — owns no mutable chart state itself.
pub struct ChartElement {
    /// The Vello scene to render this frame.
    scene: Scene,
    /// Shared VelloRenderer for GPU rendering.
    renderer: Arc<Mutex<VelloRenderer>>,
    /// Chart width in pixels.
    width: u32,
    /// Chart height in pixels.
    height: u32,
}

impl ChartElement {
    /// Create a new chart element for one paint cycle.
    pub fn new(scene: Scene, renderer: Arc<Mutex<VelloRenderer>>, width: u32, height: u32) -> Self {
        Self {
            scene,
            renderer,
            width,
            height,
        }
    }

    /// Access the scene (for testing).
    pub fn scene(&self) -> &Scene {
        &self.scene
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

impl IntoElement for ChartElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ChartElement {
    type RequestLayoutState = ();
    type PrepaintState = gpui::Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size = Size {
            width: gpui::Length::Definite(gpui::DefiniteLength::Absolute(
                gpui::AbsoluteLength::Pixels(px(self.width as f32)),
            )),
            height: gpui::Length::Definite(gpui::DefiniteLength::Absolute(
                gpui::AbsoluteLength::Pixels(px(self.height as f32)),
            )),
        };
        let layout_id = window.request_layout(style, [], cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        // Register a hitbox covering the full element bounds for mouse events.
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        if self.width == 0 || self.height == 0 {
            return;
        }

        // Render the Vello scene to RGBA pixels.
        let pixels = self
            .renderer
            .lock()
            .expect("VelloRenderer mutex poisoned")
            .render_to_pixels(&self.scene, self.width, self.height);

        // Convert RGBA to BGRA (RenderImage expects BGRA format).
        let mut bgra_pixels = pixels;
        for pixel in bgra_pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2); // Swap R and B channels
        }

        // Construct the RenderImage from the pixel buffer.
        let image_buffer =
            RgbaImage::from_raw(self.width, self.height, bgra_pixels)
                .expect("pixel buffer size mismatch");
        let frame = image::Frame::new(image_buffer);
        let render_image = Arc::new(RenderImage::new(SmallVec::from_elem(frame, 1)));

        // Paint the image into the GPUI window.
        let _ = window.paint_image(
            bounds,
            Corners::default(),
            render_image,
            0,
            false,
        );
    }
}

#[cfg(all(test, feature = "gpu-tests"))]
mod tests {
    use super::*;

    // Preserve existing test assertions — now testing ChartElement as
    // a stateless rendering shell. These tests verify the struct can be
    // constructed and the scene is accessible.

    #[test]
    fn gpu_ac09_chart_element_creation() {
        let renderer = crate::vello_renderer::VelloRenderer::new();
        let scene = Scene::new();
        let element = ChartElement::new(scene, renderer, 640, 480);
        assert_eq!(element.width(), 640);
        assert_eq!(element.height(), 480);
    }

    #[test]
    fn gpu_ac09_chart_element_scene_update() {
        let renderer = crate::vello_renderer::VelloRenderer::new();
        let mut scene = Scene::new();
        use kurbo::{Affine, Circle};
        use peniko::{Color, Fill};
        let circle = Circle::new((100.0, 100.0), 5.0);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::new([1.0, 0.0, 0.0, 1.0]),
            None,
            &circle,
        );

        let element = ChartElement::new(scene, renderer, 640, 480);

        let encoding = element.scene().encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "scene should have content"
        );
    }
}
