//! ChartElement — GPUI Element that paints the chart and routes mouse input.
//!
//! ChartElement is created fresh by `ChartView::render()` each frame. It holds
//! the `Entity<ChartState>` and, on paint:
//!   1. composites the interaction overlay (brush rect / hover marker) onto a
//!      clone of the current scene,
//!   2. rasterises that scene with the shared Vello renderer and paints it, and
//!   3. registers window mouse listeners that drive the chart's interaction
//!      state (brush / hover). GPUI clears per-frame listeners each frame, so
//!      they are re-registered every paint; a state change calls
//!      `window.refresh()` to repaint with the updated overlay.
//!
//! Lifecycle:
//! - `request_layout()` — fixed-size layout from ChartState dimensions
//! - `prepaint()` — registers a hitbox covering the element bounds
//! - `paint()` — composite + rasterise + paint + wire mouse events

use std::sync::Arc;

use gpui::{
    App, Bounds, Corners, Element, ElementId, Entity, GlobalElementId, HitboxBehavior,
    InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, RenderImage, Size, Style, Window, px,
};
use image::RgbaImage;
use kurbo::Point;
use smallvec::SmallVec;

use crate::chart_state::ChartState;

/// GPUI element that paints a chart from its `ChartState` and routes mouse input.
///
/// Created by `ChartView::render()` each frame. Owns no chart state of its own —
/// it reads and updates the shared `Entity<ChartState>`.
pub struct ChartElement {
    /// The reactive chart state entity.
    state: Entity<ChartState>,
}

impl ChartElement {
    /// Create a chart element bound to the given state entity.
    pub fn new(state: Entity<ChartState>) -> Self {
        Self { state }
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
        let (width, height) = {
            let state = self.state.read(cx);
            (state.width(), state.height())
        };
        let mut style = Style::default();
        style.size = Size {
            width: gpui::Length::Definite(gpui::DefiniteLength::Absolute(
                gpui::AbsoluteLength::Pixels(px(width as f32)),
            )),
            height: gpui::Length::Definite(gpui::DefiniteLength::Absolute(
                gpui::AbsoluteLength::Pixels(px(height as f32)),
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
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Register window mouse listeners FIRST, so input survives even a
        // transient zero-size frame (the raster below may early-return, but the
        // listeners only need bounds.origin + the hitbox). The element origin
        // maps window-space positions to chart-local coordinates; the hitbox
        // restricts presses to this element. GPUI clears per-frame listeners, so
        // they are re-registered every paint, and window.refresh() is the
        // repaint trigger when the interaction state changes.
        let element_origin = Point::new(bounds.origin.x.to_f64(), bounds.origin.y.to_f64());

        // Mouse down — begin a brush if the press is over the chart.
        window.on_mouse_event({
            let state = self.state.clone();
            let hitbox = prepaint.clone();
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase.bubble()
                    && event.button == MouseButton::Left
                    && hitbox.is_hovered(window)
                {
                    let pos = Point::new(event.position.x.to_f64(), event.position.y.to_f64());
                    let changed = state.update(cx, |s, _| s.pointer_down(pos, element_origin));
                    if changed {
                        window.refresh();
                    }
                }
            }
        });

        // Mouse move — extend the brush while the button is held, or update hover.
        window.on_mouse_event({
            let state = self.state.clone();
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase.bubble() {
                    let pos = Point::new(event.position.x.to_f64(), event.position.y.to_f64());
                    let held = event.pressed_button == Some(MouseButton::Left);
                    let changed = state.update(cx, |s, _| s.pointer_move(pos, element_origin, held));
                    if changed {
                        window.refresh();
                    }
                }
            }
        });

        // Mouse up — end an active brush.
        window.on_mouse_event({
            let state = self.state.clone();
            move |event: &MouseUpEvent, phase, window, cx| {
                if phase.bubble() && event.button == MouseButton::Left {
                    let changed = state.update(cx, |s, _| s.pointer_up());
                    if changed {
                        window.refresh();
                    }
                }
            }
        });

        // Pull the current scene + interaction out of state and composite the
        // interaction overlay (brush rect / hover marker) onto a scene clone.
        let (mut scene, interaction, renderer, width, height) = {
            let state = self.state.read(cx);
            (
                state.scene().clone(),
                state.interaction().clone(),
                state.renderer().clone(),
                state.width(),
                state.height(),
            )
        };
        if width == 0 || height == 0 {
            return;
        }
        interaction.render_overlay(&mut scene);

        // Rasterise the composited scene and paint it as a GPUI image.
        let pixels = renderer
            .lock()
            .expect("VelloRenderer mutex poisoned")
            .render_to_pixels(&scene, width, height);

        // Convert RGBA to BGRA (RenderImage expects BGRA format).
        let mut bgra_pixels = pixels;
        for pixel in bgra_pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2); // Swap R and B channels
        }

        let image_buffer =
            RgbaImage::from_raw(width, height, bgra_pixels).expect("pixel buffer size mismatch");
        let frame = image::Frame::new(image_buffer);
        let render_image = Arc::new(RenderImage::new(SmallVec::from_elem(frame, 1)));
        let _ = window.paint_image(bounds, Corners::default(), render_image, 0, false);
    }
}
