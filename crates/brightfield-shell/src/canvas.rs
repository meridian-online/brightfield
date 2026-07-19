//! egui host for the T46 `CanvasHost` / `ChartSurface` / `OverlayPainter` seam.
//!
//! [`EguiCanvasHost`] realises [`CanvasHost`] with `Surface = egui::TextureId`:
//! a `vello::Scene` is rasterised straight onto a wgpu texture on eframe's
//! **shared** device (`VelloRenderer::render_to_texture`, no readback) and then
//! handed to egui **zero-copy** via `egui_wgpu::Renderer::register_native_texture`
//! — deleting the Metal↔wgpu readback the gpui host needed (decision-68).
//!
//! [`EguiChartFrame`] realises [`ChartSurface`] + [`OverlayPainter`] for one
//! egui frame: it reserves the chart's on-screen rect (painting the registered
//! texture into it), draws the transient interaction overlay as egui shapes on
//! top, and sets the pointer cursor. [`surface_input`] maps egui pointer state
//! over the reserved rect into the framework-free [`SurfaceInput`].

use std::sync::{Arc, Mutex};

use brightfield_render::canvas_host::{
    ButtonState, CanvasHost, ChartSurface, Color, Modifiers, OverlayPainter, PixelSize,
    SurfaceCursor, SurfaceInput, SurfaceRect,
};
use brightfield_render::vello_renderer::VelloRenderer;
use kurbo::{Point, Vec2};
use vello::{wgpu, Scene};

/// The shared `egui_wgpu::Renderer`, in the `Arc<RwLock<…>>` shape eframe's
/// `RenderState` hands out (and the offscreen shot builds itself).
pub type SharedEguiRenderer = Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>;

/// The egui [`CanvasHost`]: owns the shared wgpu device/queue, the Vello
/// renderer pointed at that device, and the egui_wgpu renderer that registers
/// Vello's texture for zero-copy sampling.
pub struct EguiCanvasHost {
    device: wgpu::Device,
    queue: wgpu::Queue,
    vello: Arc<Mutex<VelloRenderer>>,
    egui_renderer: SharedEguiRenderer,
    /// The live Vello texture, kept alive while egui samples it, plus its
    /// registered id — freed and replaced on the next present.
    current: Option<(wgpu::Texture, egui::TextureId)>,
}

impl EguiCanvasHost {
    /// Build a host on eframe's shared device. `vello` must have been created via
    /// [`VelloRenderer::from_shared`] with the *same* device/queue.
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        vello: Arc<Mutex<VelloRenderer>>,
        egui_renderer: SharedEguiRenderer,
    ) -> Self {
        Self {
            device,
            queue,
            vello,
            egui_renderer,
            current: None,
        }
    }

    /// The most recently presented texture id, if any.
    pub fn texture_id(&self) -> Option<egui::TextureId> {
        self.current.as_ref().map(|(_, id)| *id)
    }
}

impl CanvasHost for EguiCanvasHost {
    type Surface = egui::TextureId;

    fn device(&self) -> wgpu::Device {
        self.device.clone()
    }

    fn queue(&self) -> wgpu::Queue {
        self.queue.clone()
    }

    fn present_scene(&mut self, scene: &Scene, size: PixelSize, base: Color) -> egui::TextureId {
        // Release the previous frame's registration before replacing it.
        if let Some((_, id)) = self.current.take() {
            self.egui_renderer.write().free_texture(&id);
        }

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("brightfield-vello-egui-target"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            // STORAGE_BINDING + RENDER_ATTACHMENT: Vello writes here.
            // TEXTURE_BINDING: egui samples it. COPY_SRC: the shot reads it back.
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.vello
            .lock()
            .expect("VelloRenderer mutex poisoned")
            .render_to_texture(scene, &view, size.width, size.height, base.into_peniko());

        let id = self.egui_renderer.write().register_native_texture(
            &self.device,
            &view,
            wgpu::FilterMode::Linear,
        );
        self.current = Some((texture, id));
        id
    }
}

/// One egui frame's realisation of [`ChartSurface`] + [`OverlayPainter`].
/// Borrows the live `egui::Ui`; presents the registered Vello texture into a
/// reserved rect, paints the overlay on top, and records the desired cursor.
pub struct EguiChartFrame<'u> {
    ui: &'u mut egui::Ui,
    texture: egui::TextureId,
    /// Filled by [`ChartSurface::present`]: the reserved on-screen rect.
    rect: Option<egui::Rect>,
    cursor: Option<SurfaceCursor>,
}

impl<'u> EguiChartFrame<'u> {
    /// Wrap a `Ui` and the presented texture for this frame.
    pub fn new(ui: &'u mut egui::Ui, texture: egui::TextureId) -> Self {
        Self {
            ui,
            texture,
            rect: None,
            cursor: None,
        }
    }

    /// The reserved rect after [`ChartSurface::present`] (window-space logical
    /// pixels), for mapping pointer input.
    pub fn reserved(&self) -> Option<egui::Rect> {
        self.rect
    }

    fn origin(&self) -> egui::Vec2 {
        self.rect.map_or(egui::Vec2::ZERO, |r| r.min.to_vec2())
    }

    fn pt(&self, p: Point) -> egui::Pos2 {
        let o = self.origin();
        egui::pos2(p.x as f32 + o.x, p.y as f32 + o.y)
    }
}

impl ChartSurface for EguiChartFrame<'_> {
    fn present(&mut self, size: PixelSize) -> SurfaceRect {
        // Reserve the chart rect at logical size and paint the (device-res)
        // Vello texture into it — egui downsamples for crisp HiDPI output.
        let logical = egui::vec2(size.width as f32, size.height as f32);
        let (rect, _resp) = self.ui.allocate_exact_size(
            logical,
            egui::Sense::click_and_drag(),
        );
        egui::Image::new((self.texture, rect.size()))
            .tint(egui::Color32::WHITE)
            .paint_at(self.ui, rect);
        self.rect = Some(rect);
        SurfaceRect::new(
            rect.min.x as f64,
            rect.min.y as f64,
            rect.width() as f64,
            rect.height() as f64,
        )
    }

    fn overlay(&mut self) -> &mut dyn OverlayPainter {
        self
    }

    fn set_cursor(&mut self, cursor: Option<SurfaceCursor>) {
        self.cursor = cursor;
        if let Some(c) = cursor {
            self.ui.ctx().set_cursor_icon(surface_cursor_to_egui(c));
        }
    }
}

impl OverlayPainter for EguiChartFrame<'_> {
    fn fill_rect(&mut self, r: SurfaceRect, c: Color) {
        let o = self.origin();
        let rect = egui::Rect::from_min_size(
            egui::pos2(r.x as f32 + o.x, r.y as f32 + o.y),
            egui::vec2(r.width as f32, r.height as f32),
        );
        self.ui
            .painter()
            .rect_filled(rect, 0.0, to_color32(c));
    }

    fn stroke_rect(&mut self, r: SurfaceRect, c: Color, w: f32) {
        let o = self.origin();
        let rect = egui::Rect::from_min_size(
            egui::pos2(r.x as f32 + o.x, r.y as f32 + o.y),
            egui::vec2(r.width as f32, r.height as f32),
        );
        self.ui.painter().rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(w, to_color32(c)),
            egui::StrokeKind::Inside,
        );
    }

    fn fill_circle(&mut self, center: Point, radius: f64, c: Color) {
        self.ui
            .painter()
            .circle_filled(self.pt(center), radius as f32, to_color32(c));
    }

    fn line(&mut self, a: Point, b: Point, c: Color, w: f32) {
        self.ui
            .painter()
            .line_segment([self.pt(a), self.pt(b)], egui::Stroke::new(w, to_color32(c)));
    }

    fn text(&mut self, at: Point, s: &str, c: Color, size: f32) {
        self.ui.painter().text(
            self.pt(at),
            egui::Align2::LEFT_TOP,
            s,
            egui::FontId::proportional(size),
            to_color32(c),
        );
    }
}

/// Map egui pointer state over `rect` into the framework-free [`SurfaceInput`]
/// (surface-local logical pixels). The T46 seam's egui input boundary.
pub fn surface_input(ctx: &egui::Context, rect: egui::Rect) -> SurfaceInput {
    ctx.input(|i| {
        let hovered = i.pointer.hover_pos().is_some_and(|p| rect.contains(p));
        let pointer_pos = i
            .pointer
            .hover_pos()
            .filter(|p| rect.contains(*p))
            .map(|p| Point::new((p.x - rect.min.x) as f64, (p.y - rect.min.y) as f64));
        let m = i.modifiers;
        let d = i.pointer.delta();
        SurfaceInput {
            pointer_pos,
            pointer_primary: button(i.pointer.primary_down()),
            pointer_secondary: button(i.pointer.secondary_down()),
            drag_delta: Vec2::new(d.x as f64, d.y as f64),
            scroll_delta: Vec2::new(
                i.smooth_scroll_delta.x as f64,
                i.smooth_scroll_delta.y as f64,
            ),
            modifiers: Modifiers {
                shift: m.shift,
                control: m.ctrl,
                alt: m.alt,
                platform: m.mac_cmd || m.command,
            },
            hovered,
        }
    })
}

fn button(down: bool) -> ButtonState {
    if down {
        ButtonState::Down
    } else {
        ButtonState::Up
    }
}

/// Map a framework-free surface cursor to its egui glyph.
fn surface_cursor_to_egui(c: SurfaceCursor) -> egui::CursorIcon {
    match c {
        SurfaceCursor::Grab => egui::CursorIcon::Grab,
        SurfaceCursor::Grabbing => egui::CursorIcon::Grabbing,
        SurfaceCursor::ResizeHorizontal => egui::CursorIcon::ResizeHorizontal,
        SurfaceCursor::ResizeVertical => egui::CursorIcon::ResizeVertical,
        SurfaceCursor::ResizeNwSe => egui::CursorIcon::ResizeNwSe,
        SurfaceCursor::ResizeNeSw => egui::CursorIcon::ResizeNeSw,
    }
}

/// Framework-free straight-alpha colour → `egui::Color32` (unmultiplied).
fn to_color32(c: Color) -> egui::Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgba_unmultiplied(q(c.r), q(c.g), q(c.b), q(c.a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_glyph_mapping() {
        assert_eq!(surface_cursor_to_egui(SurfaceCursor::Grab), egui::CursorIcon::Grab);
        assert_eq!(
            surface_cursor_to_egui(SurfaceCursor::ResizeNwSe),
            egui::CursorIcon::ResizeNwSe
        );
    }

    #[test]
    fn color_boundary_unmultiplied() {
        let c = to_color32(Color { r: 1.0, g: 0.0, b: 0.0, a: 0.5 });
        assert_eq!(c.a(), 128);
    }
}
