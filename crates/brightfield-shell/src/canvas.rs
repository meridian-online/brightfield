//! egui host for the framework-free `CanvasHost` / `ChartSurface` /
//! `OverlayPainter` render seam defined in `brightfield-render`.
//!
//! [`EguiCanvasHost`] realises [`CanvasHost`] with `Surface = egui::TextureId`:
//! a `vello::Scene` is rasterised straight onto a wgpu texture on eframe's
//! **shared** device (`VelloRenderer::render_to_texture`, no readback) and then
//! handed to egui **zero-copy** via `egui_wgpu::Renderer::register_native_texture`
//! — deleting the Metal↔wgpu readback the gpui host needed, which existed only
//! because that host rendered on its own device and had to copy pixels through
//! host memory to hand them over.
//!
//! [`EguiChartFrame`] realises [`ChartSurface`] + [`OverlayPainter`] for one
//! egui frame: it reserves the chart's on-screen rect (painting the registered
//! texture into it), draws the transient interaction overlay as egui shapes on
//! top, and sets the pointer cursor. [`surface_input`] maps egui pointer state
//! over the reserved rect into the framework-free [`SurfaceInput`].
//!
//! # One texture slot per pane
//!
//! The host keys its live textures by [`PaneKey`]. It used to hold exactly one —
//! a `current: Option<(Texture, TextureId)>` that each present freed before
//! replacing. That was correct only while the app could show one canvas at a
//! time: with two canvas panes visible in the same frame, the second present
//! would free the first's registration *before* the frame was drawn, and
//! `egui_wgpu::Renderer::render` resolves a mesh's bind group by id at render
//! time — so the first pane would silently paint nothing (a `Missing texture`
//! warning is all egui emits). The dock tree makes two canvases reachable, so
//! the slot map is a precondition for it, not a tidy-up after it.
//!
//! Each slot owns its texture, its view and its registered id. On a size change
//! the texture is rebuilt but the **id is re-pointed rather than replaced**, so
//! a caller may hold an id across frames and across resizes.
//!
//! # Liveness is declared, not inferred
//!
//! [`EguiCanvasHost::end_frame`] takes the set of panes that were on screen. It
//! deliberately does *not* infer liveness from "did anyone touch this slot this
//! frame". That would be a contract every caller has to remember on every paint
//! path, and both surfaces in the tree today would already break it: `ShellState`
//! and `ProtocolShell` each cache the [`egui::TextureId`] in a field of their own
//! and, on an unchanged frame, paint from that field without coming back to the
//! host at all. A sweep keyed on "was I asked" would free the texture of a pane
//! that is visibly on screen — the same silent blank the slot map exists to
//! abolish, arriving one frame later. Taking the set makes the statement explicit
//! and puts it in one place, the frame loop that knows its dock tree, rather than
//! on every path that paints.
//!
//! No shell code calls `end_frame` yet — this increment has no consumers — so the
//! sweep, and the ordering it is safe at, are exercised only by the
//! `keyed_canvas` integration test.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use brightfield_render::canvas_host::{
    ButtonState, CanvasHost, ChartSurface, Color, Modifiers, OverlayPainter, PixelSize,
    SurfaceCursor, SurfaceInput, SurfaceRect,
};
use brightfield_render::vello_renderer::VelloRenderer;
use brightfield_workbench::{ItemId, PaneKey, ViewKind};
use kurbo::{Point, Vec2};
use vello::{wgpu, Scene};

/// The shared `egui_wgpu::Renderer`, in the `Arc<RwLock<…>>` shape eframe's
/// `RenderState` hands out (and the offscreen shot builds itself).
pub type SharedEguiRenderer = Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>;

/// The single slot every [`CanvasHost::present_scene`] call routes through.
///
/// The trait is also implemented by the dying gpui host, which has no notion of
/// a pane, so its signature stays key-free and the egui implementation forwards
/// here. Today's `ShellState` and `ProtocolShell` each own their own host, so one
/// legacy key per host is one slot per surface — the pre-existing behaviour
/// exactly. The migration replaces those calls with real pane keys.
pub const LEGACY_CANVAS: PaneKey =
    PaneKey::new(ViewKind::Charts, ItemId::new("legacy-canvas-host"));

/// One pane's live canvas: the Vello target, the view egui samples through, the
/// registered id, and the size it was built at.
struct Slot {
    /// The Vello target. Kept as a handle — not merely as the `view` below —
    /// because [`EguiCanvasHost::slot_texture`] reads its pixels back, and
    /// because the slot has to *own* it for the sweep's drop to release the
    /// memory. (Holding it is not what keeps the pixels alive during a frame: a
    /// `TextureView` keeps its `Texture` alive on its own, and only an explicit
    /// `Texture::destroy` would pull them out from under a bound view.)
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    id: egui::TextureId,
    size: PixelSize,
    /// Rastered during the frame in progress. Set by
    /// [`EguiCanvasHost::present_keyed`], cleared by
    /// [`EguiCanvasHost::end_frame`].
    presented: bool,
}

/// The egui [`CanvasHost`]: owns the shared wgpu device/queue, the Vello
/// renderer pointed at that device, and the egui_wgpu renderer that registers
/// Vello's textures for zero-copy sampling — one slot per pane.
pub struct EguiCanvasHost {
    device: wgpu::Device,
    queue: wgpu::Queue,
    vello: Arc<Mutex<VelloRenderer>>,
    egui_renderer: SharedEguiRenderer,
    /// One live texture per pane. A `BTreeMap` rather than a hash map so
    /// [`Self::end_frame`]'s sweep runs in a fixed order — the frees it issues
    /// are observable, through the renderer's texture table.
    slots: BTreeMap<PaneKey, Slot>,
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
            slots: BTreeMap::new(),
        }
    }

    /// Rasterise `scene` at `size` over `base` into `key`'s slot and return the
    /// texture id to paint.
    ///
    /// The slot's texture is **reused** when `size` is unchanged, so a re-present
    /// at a steady size allocates nothing. On a size change the texture is
    /// rebuilt and the same [`egui::TextureId`] is re-pointed at the new view, so
    /// an id held across frames stays valid through a resize.
    ///
    /// Presenting marks the slot as rastered this frame, so a pane that presents
    /// survives [`Self::end_frame`] whether or not the caller also names it in
    /// the visible set. The reverse is not true: a cached pane that does not
    /// present must be named. See [`Self::end_frame`].
    pub fn present_keyed(
        &mut self,
        key: PaneKey,
        scene: &Scene,
        size: PixelSize,
        base: Color,
    ) -> egui::TextureId {
        match self.slots.get(&key).map(|s| (s.id, s.size)) {
            Some((_, have)) if have == size => {}
            Some((id, _)) => {
                let (texture, view) = self.create_target(key, size);
                // Re-point the existing id rather than registering a new one:
                // a caller may be holding this id from an earlier frame.
                self.egui_renderer
                    .write()
                    .update_egui_texture_from_wgpu_texture(
                        &self.device,
                        &view,
                        wgpu::FilterMode::Linear,
                        id,
                    );
                let slot = self.slots.get_mut(&key).expect("slot looked up above");
                slot.texture = texture;
                slot.view = view;
                slot.size = size;
            }
            None => {
                let (texture, view) = self.create_target(key, size);
                let id = self.egui_renderer.write().register_native_texture(
                    &self.device,
                    &view,
                    wgpu::FilterMode::Linear,
                );
                self.slots.insert(
                    key,
                    Slot {
                        texture,
                        view,
                        id,
                        size,
                        presented: false,
                    },
                );
            }
        }

        let vello = Arc::clone(&self.vello);
        let slot = self.slots.get_mut(&key).expect("slot created above");
        slot.presented = true;
        vello
            .lock()
            .expect("VelloRenderer mutex poisoned")
            .render_to_texture(
                scene,
                &slot.view,
                size.width,
                size.height,
                base.into_peniko(),
            );
        slot.id
    }

    /// `key`'s texture id, if it has a live slot.
    ///
    /// A plain getter with no lifetime meaning: asking does **not** keep a slot
    /// alive across [`Self::end_frame`], because a pane that caches the id in a
    /// field of its own never asks again and would be swept. Declare visibility
    /// to `end_frame` instead.
    #[must_use]
    pub fn texture_id(&self, key: PaneKey) -> Option<egui::TextureId> {
        self.slots.get(&key).map(|s| s.id)
    }

    /// Drop every slot that neither presented this frame nor appears in
    /// `visible`, freeing its egui registration.
    ///
    /// `visible` is the set of panes the frame just laid out — for the dock tree,
    /// the panes it actually drew. A pane that presented this frame is kept
    /// regardless, so forgetting one in `visible` can only leak, never blank.
    ///
    /// Safe to call after the frame's UI has run and before eframe paints (the
    /// end of `App::update`): a slot that is neither presented nor declared
    /// visible was not painted either, so the frame's tessellated shapes carry no
    /// mesh pointing at its id, and freeing it cannot strand the render pass that
    /// follows.
    ///
    /// Without this a pane that is closed, or tabbed out of view, keeps its
    /// texture and its registration for the life of the process.
    pub fn end_frame(&mut self, visible: &BTreeSet<PaneKey>) {
        let renderer = Arc::clone(&self.egui_renderer);
        let mut r = renderer.write();
        self.slots.retain(|key, slot| {
            if slot.presented || visible.contains(key) {
                slot.presented = false;
                true
            } else {
                r.free_texture(&slot.id);
                false
            }
        });
    }

    /// How many slots are live. Test-facing; the shell never asks.
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// `key`'s Vello target, for reading its pixels back. Test-facing — the
    /// shell paints through the [`egui::TextureId`] and never touches the
    /// texture — but "which slot did these pixels land in" is not answerable
    /// from the id alone, and that is the question the slot map exists to get
    /// right.
    #[must_use]
    pub fn slot_texture(&self, key: PaneKey) -> Option<&wgpu::Texture> {
        self.slots.get(&key).map(|s| &s.texture)
    }

    fn create_target(&self, key: PaneKey, size: PixelSize) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("brightfield-vello-egui-target:{key}")),
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
        (texture, view)
    }
}

/// Give every remaining slot's registration back to the shared renderer.
///
/// The `egui_wgpu::Renderer` outlives the host (eframe owns it; the offscreen
/// shot builds both together), so a registration the host never frees is a bind
/// group and a sampler kept for the renderer's life. With one slot that was one
/// leak; with a slot per pane it is one per pane the host ever showed.
impl Drop for EguiCanvasHost {
    fn drop(&mut self) {
        let mut r = self.egui_renderer.write();
        for slot in self.slots.values() {
            r.free_texture(&slot.id);
        }
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

    /// The key-free trait present: [`LEGACY_CANVAS`]'s slot.
    ///
    /// A thin forward on purpose. `CanvasHost` is also implemented by the gpui
    /// host, so keying belongs on the inherent API rather than in the shared
    /// seam.
    fn present_scene(&mut self, scene: &Scene, size: PixelSize, base: Color) -> egui::TextureId {
        self.present_keyed(LEGACY_CANVAS, scene, size, base)
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
        let (rect, _resp) = self
            .ui
            .allocate_exact_size(logical, egui::Sense::click_and_drag());
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
        self.ui.painter().rect_filled(rect, 0.0, to_color32(c));
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
        self.ui.painter().line_segment(
            [self.pt(a), self.pt(b)],
            egui::Stroke::new(w, to_color32(c)),
        );
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
/// (surface-local logical pixels). The render seam's egui input boundary: past
/// this point nothing downstream knows egui exists.
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
        assert_eq!(
            surface_cursor_to_egui(SurfaceCursor::Grab),
            egui::CursorIcon::Grab
        );
        assert_eq!(
            surface_cursor_to_egui(SurfaceCursor::ResizeNwSe),
            egui::CursorIcon::ResizeNwSe
        );
    }

    #[test]
    fn color_boundary_unmultiplied() {
        let c = to_color32(Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 0.5,
        });
        assert_eq!(c.a(), 128);
    }
}
