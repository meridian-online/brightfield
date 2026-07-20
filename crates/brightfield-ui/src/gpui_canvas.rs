//! gpui host for the [`CanvasHost`] / [`ChartSurface`] boundary.
//!
//! [`GpuiCanvasHost`] realises [`CanvasHost`]: it owns the shared Vello renderer
//! (a dedicated wgpu device) and turns a `vello::Scene` into a `RenderImage` via
//! GPU readback — the transitional present path.
//!
//! [`GpuiChartSurface`] is the gpui `Element` for one plot (created fresh by
//! `ChartView::render()` each frame). It holds the plot's `Entity<ChartState>`
//! and, on paint:
//!   1. registers window mouse listeners that gather gpui pointer events into
//!      [`SurfaceInput`] and route them into the interaction transitions
//!      (`chart_element::route_*`) — the deferred half of the lifecycle. gpui
//!      clears per-frame listeners each frame, so they are re-registered every
//!      paint; a state change calls `window.refresh()`.
//!   2. drives the framework-free [`crate::chart_element::paint_chart`] through a
//!      per-frame [`GpuiFrame`] (which implements [`ChartSurface`] + the
//!      [`OverlayPainter`]) — present the cached base raster, draw the overlay as
//!      gpui quads, set the position-dependent cursor.
//!
//! All gpui element/paint types live in this module; the chart logic in
//! `chart_element` names none of them.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui::{
    fill, point, px, size, App, BorderStyle, Bounds, Corners, CursorStyle, Element, ElementId,
    Entity, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels,
    RenderImage, Size, Style, TextAlign, TextRun, Window,
};
use kurbo::Point;
use smallvec::SmallVec;
use vello::Scene;

use crate::canvas_host::{
    ButtonState, CanvasHost, ChartSurface, Color, Modifiers, OverlayPainter, PixelSize,
    SurfaceCursor, SurfaceInput, SurfaceRect,
};
use crate::chart_element::{paint_chart, redispatch_target, route_pointer_down, route_pointer_move};
use crate::chart_state::ChartState;
use crate::crossfilter::CrossfilterCoordinator;
use crate::interaction::BrushRegion;
use crate::vello_renderer::VelloRenderer;

// ---------------------------------------------------------------------------
// CanvasHost — Vello scene → RenderImage (GPU readback).
// ---------------------------------------------------------------------------

/// The gpui [`CanvasHost`]: owns the shared [`VelloRenderer`] (dedicated wgpu
/// device) and presents a scene as a `RenderImage` via GPU readback — the
/// transitional present path (`vello_renderer.rs` semantics).
pub struct GpuiCanvasHost {
    renderer: Arc<Mutex<VelloRenderer>>,
}

impl GpuiCanvasHost {
    /// Wrap the shared renderer.
    pub fn new(renderer: Arc<Mutex<VelloRenderer>>) -> Self {
        Self { renderer }
    }
}

impl CanvasHost for GpuiCanvasHost {
    type Surface = Arc<RenderImage>;

    fn device(&self) -> vello::wgpu::Device {
        self.renderer
            .lock()
            .expect("VelloRenderer mutex poisoned")
            .device()
            .clone()
    }

    fn queue(&self) -> vello::wgpu::Queue {
        self.renderer
            .lock()
            .expect("VelloRenderer mutex poisoned")
            .queue()
            .clone()
    }

    fn present_scene(&mut self, scene: &Scene, size: PixelSize, base: Color) -> Arc<RenderImage> {
        let mut pixels = self
            .renderer
            .lock()
            .expect("VelloRenderer mutex poisoned")
            .render_to_pixels_with_base(scene, size.width, size.height, base.into_peniko());
        // RenderImage expects BGRA.
        for px in pixels.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        let buffer = image::RgbaImage::from_raw(size.width, size.height, pixels)
            .expect("pixel buffer size mismatch");
        Arc::new(RenderImage::new(SmallVec::from_elem(image::Frame::new(buffer), 1)))
    }
}

// ---------------------------------------------------------------------------
// GpuiChartSurface — the gpui Element for one plot.
// ---------------------------------------------------------------------------

/// gpui `Element` that presents a chart from its `ChartState` and routes mouse
/// input. Created by `ChartView::render()` each frame; owns no chart state of its
/// own — it reads and updates the shared `Entity<ChartState>`.
pub struct GpuiChartSurface {
    /// The reactive chart state entity.
    state: Entity<ChartState>,
    /// This plot's index — both the stable element id and the plot index into
    /// the cross-filter coordinator (charts are hosted in plot order).
    index: usize,
    /// Stable, per-plot element id.
    id: ElementId,
    /// Shared cross-filter coordinator. When present, a brush release routes
    /// through it to re-query and re-render subscriber plots; `None` means the
    /// brush is purely visual (no linked views).
    coordinator: Option<Rc<RefCell<CrossfilterCoordinator>>>,
    /// The pointer's grab region over the persisted `Selected` rect,
    /// shared with the persistent `ChartView` so it survives the per-frame
    /// element recreation. Set by the mouse-move listener, read each paint to
    /// pick the position-dependent cursor.
    region: Rc<Cell<BrushRegion>>,
}

impl GpuiChartSurface {
    /// Create a chart surface bound to the given state entity. `index` gives the
    /// element a stable id distinguishing it from sibling plots, and is the plot
    /// index into `coordinator`. `coordinator` is `None` for a dashboard that
    /// doesn't cross-filter. `region` is the persistent cursor-region cell the
    /// hosting `ChartView` owns per plot.
    pub fn new(
        state: Entity<ChartState>,
        index: usize,
        coordinator: Option<Rc<RefCell<CrossfilterCoordinator>>>,
        region: Rc<Cell<BrushRegion>>,
    ) -> Self {
        Self {
            state,
            index,
            id: ElementId::from(("brightfield-plot", index)),
            coordinator,
            region,
        }
    }

    /// Register the window mouse listeners that gather gpui pointer events into
    /// [`SurfaceInput`] and route them into the interaction transitions. Called
    /// FIRST each paint, so input survives even a transient zero-size frame (the
    /// listeners only need the element origin + hitbox). gpui clears per-frame
    /// listeners, so they are re-registered every paint; `window.refresh()` is
    /// the repaint trigger when the interaction state changes.
    fn register_input(&self, window: &mut Window, hitbox: &Hitbox, bounds: Bounds<Pixels>) {
        // The element origin maps window-space positions to chart-local
        // coordinates.
        let origin = Point::new(bounds.origin.x.to_f64(), bounds.origin.y.to_f64());

        // Mouse down — begin a brush / grab a persisted selection if the press is
        // over the chart (the hitbox gate + left-button gate ride in the input).
        window.on_mouse_event({
            let state = self.state.clone();
            let hitbox = hitbox.clone();
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase.bubble() {
                    let input = SurfaceInput {
                        pointer_pos: Some(surface_point(event.position)),
                        pointer_primary: button_state(event.button == MouseButton::Left),
                        pointer_secondary: button_state(event.button == MouseButton::Right),
                        drag_delta: kurbo::Vec2::ZERO,
                        scroll_delta: kurbo::Vec2::ZERO,
                        modifiers: modifiers_from(event.modifiers),
                        hovered: hitbox.is_hovered(window),
                    };
                    let changed =
                        state.update(cx, |s, _| route_pointer_down(&input, s, origin));
                    if changed {
                        window.refresh();
                    }
                }
            }
        });

        // Mouse move — extend the brush / move-resize the grab while held, or
        // update hover; and re-classify the pointer over any persisted selection
        // so the paint-phase cursor tracks the region under it. Refresh on EITHER
        // an interaction change or a cursor-region change.
        window.on_mouse_event({
            let state = self.state.clone();
            let region = self.region.clone();
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase.bubble() {
                    let input = SurfaceInput {
                        pointer_pos: Some(surface_point(event.position)),
                        pointer_primary: button_state(
                            event.pressed_button == Some(MouseButton::Left),
                        ),
                        pointer_secondary: button_state(
                            event.pressed_button == Some(MouseButton::Right),
                        ),
                        drag_delta: kurbo::Vec2::ZERO,
                        scroll_delta: kurbo::Vec2::ZERO,
                        modifiers: modifiers_from(event.modifiers),
                        hovered: false,
                    };
                    let outcome = state.update(cx, |s, _| route_pointer_move(&input, s, origin));
                    let region_changed = region.get() != outcome.region;
                    if region_changed {
                        region.set(outcome.region);
                    }
                    if outcome.changed || region_changed {
                        window.refresh();
                    }
                }
            }
        });

        // Mouse up — commit the brush (cross-filter the linked views, if wired)
        // then end the visual brush. The commit reads this plot's redispatch
        // target BEFORE `pointer_up` clears it, dispatches through the
        // coordinator's live Session, and swaps the re-queried subscriber scenes.
        window.on_mouse_event({
            let state = self.state.clone();
            let coordinator = self.coordinator.clone();
            let plot_index = self.index;
            move |event: &MouseUpEvent, phase, window, cx| {
                if phase.bubble() && event.button == MouseButton::Left {
                    // `committed` is true only for the plot actually brushed /
                    // grabbed, so an untouched sibling's listener doesn't trigger
                    // a redundant window refresh.
                    let committed = match &coordinator {
                        Some(coord) => {
                            let to_commit = redispatch_target(state.read(cx));
                            coord.borrow_mut().commit_brush(plot_index, &to_commit, cx)
                        }
                        None => false,
                    };
                    let changed = state.update(cx, |s, _| s.pointer_up());
                    if changed || committed {
                        window.refresh();
                    }
                }
            }
        });
    }
}

impl IntoElement for GpuiChartSurface {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GpuiChartSurface {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
        // 1. Register the input listeners FIRST (they only need bounds.origin +
        //    the hitbox), so input survives a transient zero-size frame.
        self.register_input(window, prepaint, bounds);

        // 2. Read this frame's paint inputs from the shared state.
        let sf = window.scale_factor();
        let (interaction, size) = {
            let s = self.state.read(cx);
            (
                s.interaction().clone(),
                PixelSize { width: s.width(), height: s.height() },
            )
        };
        let region = self.region.get();

        // 3. Drive the framework-free chart paint through the host surface:
        //    present the cached raster, overlay the interaction quads, set the
        //    cursor. A zero-size plot presents nothing (the listeners already ran).
        let mut frame = GpuiFrame {
            window,
            cx,
            state: &self.state,
            hitbox: prepaint,
            bounds,
            sf,
        };
        paint_chart(&mut frame, &interaction, region, size);
    }
}

// ---------------------------------------------------------------------------
// GpuiFrame — per-paint ChartSurface + OverlayPainter.
// ---------------------------------------------------------------------------

/// A per-paint gpui realisation of [`ChartSurface`] (and its [`OverlayPainter`]).
/// Borrows the live window + app for one paint; presents the cached base raster,
/// paints the overlay as gpui quads, and sets the cursor.
struct GpuiFrame<'a> {
    window: &'a mut Window,
    cx: &'a mut App,
    state: &'a Entity<ChartState>,
    hitbox: &'a Hitbox,
    bounds: Bounds<Pixels>,
    sf: f32,
}

impl ChartSurface for GpuiFrame<'_> {
    fn present(&mut self, _size: PixelSize) -> SurfaceRect {
        // The cached, device-resolution base raster (re-rendered only when the
        // scene changes) painted into the reserved bounds — hovering/brushing
        // reuse it instead of re-running Vello.
        let base = self.state.read(self.cx).base_image(self.sf);
        let _ = self
            .window
            .paint_image(self.bounds, Corners::default(), base, 0, false);
        SurfaceRect::new(
            self.bounds.origin.x.to_f64(),
            self.bounds.origin.y.to_f64(),
            self.bounds.size.width.to_f64(),
            self.bounds.size.height.to_f64(),
        )
    }

    fn overlay(&mut self) -> &mut dyn OverlayPainter {
        self
    }

    fn set_cursor(&mut self, cursor: Option<SurfaceCursor>) {
        // Reuses the set_cursor_style seam; bound to the element's whole
        // hitbox — the position dependence comes from re-picking each paint. No
        // cursor is set when `None` (the plot default).
        if let Some(c) = cursor {
            self.window
                .set_cursor_style(surface_cursor_to_gpui(c), self.hitbox);
        }
    }
}

impl OverlayPainter for GpuiFrame<'_> {
    fn fill_rect(&mut self, r: SurfaceRect, c: Color) {
        self.window.paint_quad(fill(self.rect_to_bounds(r), to_hsla(c)));
    }

    fn stroke_rect(&mut self, r: SurfaceRect, c: Color, w: f32) {
        let mut q = fill(self.rect_to_bounds(r), to_hsla(Color::TRANSPARENT));
        q.border_widths = w.into();
        q.border_color = to_hsla(c);
        q.border_style = BorderStyle::Solid;
        self.window.paint_quad(q);
    }

    fn fill_circle(&mut self, center: Point, radius: f64, c: Color) {
        let d = (radius * 2.0) as f32;
        let rect = Bounds {
            origin: point(
                self.bounds.origin.x + px((center.x - radius) as f32),
                self.bounds.origin.y + px((center.y - radius) as f32),
            ),
            size: size(px(d), px(d)),
        };
        let mut q = fill(rect, to_hsla(c));
        q.corner_radii = (radius as f32).into(); // round the quad into a circle
        self.window.paint_quad(q);
    }

    fn line(&mut self, a: Point, b: Point, c: Color, w: f32) {
        let mut pb = PathBuilder::stroke(px(w));
        pb.move_to(self.local_point(a));
        pb.line_to(self.local_point(b));
        if let Ok(path) = pb.build() {
            self.window.paint_path(path, to_hsla(c));
        }
    }

    fn text(&mut self, at: Point, s: &str, c: Color, size: f32) {
        let text_system = self.window.text_system().clone();
        let run = TextRun {
            len: s.len(),
            font: gpui::font("Helvetica"),
            color: to_hsla(c),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = text_system.shape_line(s.to_string().into(), px(size), &[run], None);
        let origin = self.local_point(at);
        let _ = shaped.paint(origin, px(size), TextAlign::Left, None, self.window, self.cx);
    }
}

impl GpuiFrame<'_> {
    /// A surface-local rect → window-space gpui bounds (offset by the element
    /// origin).
    fn rect_to_bounds(&self, r: SurfaceRect) -> Bounds<Pixels> {
        Bounds {
            origin: point(
                self.bounds.origin.x + px(r.x as f32),
                self.bounds.origin.y + px(r.y as f32),
            ),
            size: size(px(r.width as f32), px(r.height as f32)),
        }
    }

    /// A surface-local point → window-space gpui point.
    fn local_point(&self, p: Point) -> gpui::Point<Pixels> {
        point(
            self.bounds.origin.x + px(p.x as f32),
            self.bounds.origin.y + px(p.y as f32),
        )
    }
}

// ---------------------------------------------------------------------------
// Framework-free ↔ gpui mappings.
// ---------------------------------------------------------------------------

/// Map a surface cursor to its gpui glyph: an open hand
/// over the interior (closed while dragging), horizontal/vertical resize on
/// edges, diagonal resize on corners. NOT PointingHand — that is the legend's
/// clickable cue.
fn surface_cursor_to_gpui(cursor: SurfaceCursor) -> CursorStyle {
    match cursor {
        SurfaceCursor::Grab => CursorStyle::OpenHand,
        SurfaceCursor::Grabbing => CursorStyle::ClosedHand,
        SurfaceCursor::ResizeHorizontal => CursorStyle::ResizeLeftRight,
        SurfaceCursor::ResizeVertical => CursorStyle::ResizeUpDown,
        SurfaceCursor::ResizeNwSe => CursorStyle::ResizeUpLeftDownRight,
        SurfaceCursor::ResizeNeSw => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// Convert a framework-free straight-alpha colour to a gpui `Hsla` (the
/// `theme_bridge::rgba` conversion — a straight component copy into `gpui::Rgba`).
fn to_hsla(c: Color) -> Hsla {
    gpui::Rgba { r: c.r, g: c.g, b: c.b, a: c.a }.into()
}

/// A gpui window-space position → a surface-local `kurbo::Point`. (The element
/// origin is subtracted downstream by the layout's `window_to_local`.)
fn surface_point(p: gpui::Point<Pixels>) -> Point {
    Point::new(p.x.to_f64(), p.y.to_f64())
}

/// `true`→`Down`, `false`→`Up`.
fn button_state(down: bool) -> ButtonState {
    if down {
        ButtonState::Down
    } else {
        ButtonState::Up
    }
}

/// gpui modifiers → framework-free modifiers.
fn modifiers_from(m: gpui::Modifiers) -> Modifiers {
    Modifiers {
        shift: m.shift,
        control: m.control,
        alt: m.alt,
        platform: m.platform,
    }
}

#[cfg(test)]
mod tests {
    use super::surface_cursor_to_gpui;
    use crate::canvas_host::SurfaceCursor;
    use gpui::CursorStyle;

    /// Glyph: each surface cursor maps to the exact gpui glyph the
    /// pre-refactor `region_cursor` produced — open/closed hand over the interior,
    /// axis resize on edges, diagonal resize on corners. Composed with
    /// `chart_element::overlay_cursor` (region → SurfaceCursor), this preserves
    /// the whole region → CursorStyle guarantee across the boundary.
    #[test]
    fn surface_cursor_glyph_mapping() {
        assert_eq!(surface_cursor_to_gpui(SurfaceCursor::Grab), CursorStyle::OpenHand);
        assert_eq!(surface_cursor_to_gpui(SurfaceCursor::Grabbing), CursorStyle::ClosedHand);
        assert_eq!(
            surface_cursor_to_gpui(SurfaceCursor::ResizeHorizontal),
            CursorStyle::ResizeLeftRight
        );
        assert_eq!(
            surface_cursor_to_gpui(SurfaceCursor::ResizeVertical),
            CursorStyle::ResizeUpDown
        );
        assert_eq!(
            surface_cursor_to_gpui(SurfaceCursor::ResizeNwSe),
            CursorStyle::ResizeUpLeftDownRight
        );
        assert_eq!(
            surface_cursor_to_gpui(SurfaceCursor::ResizeNeSw),
            CursorStyle::ResizeUpRightDownLeft
        );
    }
}
