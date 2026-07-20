//! The canvas host's slot map, on a real GPU.
//!
//! Every test here runs against a wgpu device created with
//! [`wgpu::InstanceFlags::VALIDATION`] and `DEBUG`, and wraps its GPU work in a
//! validation error scope that is asserted empty. A "works on my machine" pass
//! with validation off would prove nothing about lifetimes, which is the whole
//! subject.
//!
//! # What the host has to guarantee
//!
//! 1. Two panes presenting in one frame do not free each other's registration.
//!    This is the defect the slot map exists to fix: a single-slot host frees the
//!    previous registration before replacing it, and `egui_wgpu::Renderer::render`
//!    resolves a mesh's bind group by id *at render time*, so the first pane
//!    would paint nothing and egui would emit only a `Missing texture` log line.
//! 2. A slot survives a resize with the *same* `TextureId`, so a caller may hold
//!    an id across frames.
//! 3. `end_frame` frees what nobody referenced and keeps what someone did.
//!
//! # The ordering question these tests answer
//!
//! `ShellState`/`ProtocolShell` present *before* the panels draw, so the raster
//! size is last frame's — a pane-filling chart would lag one frame during a
//! resize drag. The fix would be to present from **inside** the pane's draw,
//! once the rect is known, and the question is whether that is safe.
//!
//! It is, and two tests say so. [`in_frame_present_paints_both_panes`] presents
//! two panes from inside their own draws, across a frame that resizes one of
//! them, and reads the rendered window back. The sharper one is
//! [`a_mesh_painted_before_the_present_still_shows_the_presented_pixels`]:
//! it paints with an id held from the previous frame and *then* presents at a
//! new size, so the binding is replaced and the old texture dropped after the
//! paint call returned — and the readback is the new colour, with no validation
//! error. `egui_wgpu::Renderer::render` resolves a mesh's bind group by id at
//! render time, and Vello submits on the same queue eframe paints on.
//!
//! So the lag is current, not permanent, and `last_rect` / `note_rect` are
//! vestigial: nothing needs a remembered rect to size a raster. They are kept
//! because the migration may still want a rect for a pane that has to size
//! something *other* than its own present; if it turns out nothing does, the API
//! can shed them without touching this evidence.

use std::sync::{Arc, Mutex};

use brightfield_render::canvas_host::{CanvasHost, Color, PixelSize};
use brightfield_render::vello_renderer::VelloRenderer;
use brightfield_shell::canvas::{EguiCanvasHost, SharedEguiRenderer, LEGACY_CANVAS};
use brightfield_workbench::{ItemId, PaneKey, ViewKind};
use vello::wgpu;

const PANE_A: PaneKey = PaneKey::new(ViewKind::Charts, ItemId::new("keyed-canvas-test-a"));
const PANE_B: PaneKey = PaneKey::new(ViewKind::Protocol, ItemId::new("keyed-canvas-test-b"));

const RED: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};
const GREEN: Color = Color {
    r: 0.0,
    g: 1.0,
    b: 0.0,
    a: 1.0,
};
const BLUE: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A validating headless device. Deliberately *not*
/// `brightfield_shell::capture::headless_device`, which builds a default
/// instance: these tests are about resource lifetimes, so the validation layers
/// are the point.
fn validating_device() -> (wgpu::Device, wgpu::Queue) {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.flags = wgpu::InstanceFlags::VALIDATION | wgpu::InstanceFlags::DEBUG;
    let instance = wgpu::Instance::new(desc);
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("a GPU adapter");
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("keyed-canvas-test"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::default(),
        ..Default::default()
    }))
    .expect("a GPU device")
}

const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn new_egui_renderer(device: &wgpu::Device) -> SharedEguiRenderer {
    Arc::new(egui::mutex::RwLock::new(egui_wgpu::Renderer::new(
        device,
        TARGET_FORMAT,
        egui_wgpu::RendererOptions {
            msaa_samples: 1,
            depth_stencil_format: None,
            dithering: false,
            predictable_texture_filtering: true,
        },
    )))
}

struct Rig {
    device: wgpu::Device,
    queue: wgpu::Queue,
    egui_renderer: SharedEguiRenderer,
    /// The same renderer the host presents through — reused for readback rather
    /// than built per test, because a `VelloRenderer` compiles shaders.
    vello: Arc<Mutex<VelloRenderer>>,
    host: EguiCanvasHost,
    /// The armed validation scope. Held as an `Option` because popping it
    /// consumes the guard, and every check re-arms.
    scope: Option<wgpu::ErrorScopeGuard>,
}

impl Rig {
    fn new() -> Self {
        let (device, queue) = validating_device();
        let egui_renderer = new_egui_renderer(&device);
        let vello = VelloRenderer::from_shared(device.clone(), queue.clone());
        let host = EguiCanvasHost::new(
            device.clone(),
            queue.clone(),
            Arc::clone(&vello),
            Arc::clone(&egui_renderer),
        );
        let scope = Some(device.push_error_scope(wgpu::ErrorFilter::Validation));
        Self {
            device,
            queue,
            egui_renderer,
            vello,
            host,
            scope,
        }
    }

    /// Read a slot's Vello target back, tightly packed `Rgba8Unorm`.
    fn slot_pixels(&self, key: PaneKey, w: u32, h: u32) -> Vec<u8> {
        let texture = self
            .host
            .slot_texture(key)
            .expect("the slot was presented above");
        self.vello
            .lock()
            .expect("VelloRenderer mutex poisoned")
            .read_texture(texture, w, h)
    }

    /// Whether egui still holds a binding for `id`. This is the exact lookup
    /// `Renderer::render` does per mesh, so `false` here *is* "this pane paints
    /// nothing".
    fn registered(&self, id: egui::TextureId) -> bool {
        self.egui_renderer.read().texture(&id).is_some()
    }

    /// Fail if any validation error was raised since the scope was armed.
    #[track_caller]
    fn assert_no_validation_errors(&mut self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let guard = self.scope.take().expect("a scope is always armed");
        let err = pollster::block_on(guard.pop());
        assert!(err.is_none(), "wgpu validation error: {err:?}");
        // Re-arm, so a rig can be checked more than once in a test.
        self.scope = Some(self.device.push_error_scope(wgpu::ErrorFilter::Validation));
    }
}

/// A scene with no marks: `present_keyed` clears the target to `base`, so the
/// whole texture ends up one identifiable colour. What is being tested is which
/// texture the pixels landed in, not what Vello drew.
fn blank() -> vello::Scene {
    vello::Scene::new()
}

fn size(w: u32, h: u32) -> PixelSize {
    PixelSize {
        width: w,
        height: h,
    }
}

/// The `[r, g, b, a]` at the centre of an `Rgba8Unorm` buffer of `w` x `h`.
fn centre_px(pixels: &[u8], w: u32, h: u32) -> [u8; 4] {
    px_at(pixels, w, w / 2, h / 2)
}

fn px_at(pixels: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

/// Assert a sampled pixel is the expected colour, allowing for the sampler's
/// rounding.
#[track_caller]
fn assert_colour(got: [u8; 4], want: [u8; 4], what: &str) {
    let close = got
        .iter()
        .zip(want.iter())
        .all(|(g, w)| i32::from(*g).abs_diff(i32::from(*w)) <= 2);
    assert!(close, "{what}: got {got:?}, want ~{want:?}");
}

// ---------------------------------------------------------------------------
// Slot lifetimes
// ---------------------------------------------------------------------------

/// Would catch: the single-slot host this increment replaces. With a
/// `current: Option<(Texture, TextureId)>` that each present frees before
/// replacing, presenting B frees A's registration, `registered(a)` goes false,
/// and A paints nothing for the frame.
///
/// Verified by giving `present_keyed` back the old single-slot semantics (free
/// every registration before presenting): this test fired `two panes in one
/// frame: A's registration was freed by B's present`. That mutation reddens 7 of
/// the 9 tests in this file.
#[test]
fn two_panes_presented_in_one_frame_keep_both_registrations() {
    let mut rig = Rig::new();

    let a = rig.host.present_keyed(PANE_A, &blank(), size(64, 64), RED);
    let b = rig
        .host
        .present_keyed(PANE_B, &blank(), size(48, 32), GREEN);

    assert_ne!(a, b, "two panes must not share one texture id");
    assert!(
        rig.registered(a),
        "two panes in one frame: A's registration was freed by B's present"
    );
    assert!(rig.registered(b), "B's own registration went missing");
    assert_eq!(rig.host.slot_count(), 2);
    rig.assert_no_validation_errors();
}

/// Would catch: two keys sharing one texture — a slot map that stores the
/// texture but not per key, so B's raster lands in the pixels A is showing.
/// Registration alone cannot see that; only the pixels can.
///
/// Verified by making `present_keyed` render into the *first* slot's view
/// regardless of `key`: `pane A texture: got [0, 255, 0, 255], want ~[255, 0, 0,
/// 255]` fired — A was showing B's green. That mutation leaves
/// [`two_panes_presented_in_one_frame_keep_both_registrations`] green, which is
/// the point of paying for a readback here.
#[test]
fn each_pane_keeps_its_own_pixels() {
    let mut rig = Rig::new();
    rig.host.present_keyed(PANE_A, &blank(), size(64, 64), RED);
    rig.host
        .present_keyed(PANE_B, &blank(), size(48, 32), GREEN);

    let a_px = rig.slot_pixels(PANE_A, 64, 64);
    let b_px = rig.slot_pixels(PANE_B, 48, 32);

    assert_colour(centre_px(&a_px, 64, 64), [255, 0, 0, 255], "pane A texture");
    assert_colour(centre_px(&b_px, 48, 32), [0, 255, 0, 255], "pane B texture");
    rig.assert_no_validation_errors();
}

/// Would catch: a resize that registers a *new* id. A caller holding the id from
/// an earlier frame would then paint a freed binding — the same silent blank as
/// the single-slot bug, but only during a resize.
///
/// Verified by replacing the `update_egui_texture_from_wgpu_texture` branch with
/// a fresh `register_native_texture`: `a resize must re-point the id, not
/// replace it` fired.
#[test]
fn a_resize_repoints_the_same_texture_id() {
    let mut rig = Rig::new();
    let first = rig.host.present_keyed(PANE_A, &blank(), size(64, 64), RED);
    let second = rig.host.present_keyed(PANE_A, &blank(), size(96, 40), BLUE);

    assert_eq!(
        first, second,
        "a resize must re-point the id, not replace it"
    );
    assert!(rig.registered(first), "the re-pointed id lost its binding");
    assert_eq!(rig.host.slot_count(), 1, "a resize must not add a slot");

    let px = rig.slot_pixels(PANE_A, 96, 40);
    assert_colour(
        centre_px(&px, 96, 40),
        [0, 0, 255, 255],
        "the resized texture",
    );
    rig.assert_no_validation_errors();
}

/// Would catch: an `end_frame` that frees everything, or frees nothing.
///
/// Frame 1 presents both panes; frame 2 presents only A. B is gone, A is not —
/// and A is not merely still in the map, its egui binding is still resolvable.
///
/// Verified twice: making `end_frame` a no-op fired `a pane that stopped drawing
/// leaked its slot`; making it clear the whole map fired `a pane that presented
/// this frame was swept`.
#[test]
fn end_frame_frees_the_dropped_slot_and_keeps_the_live_one() {
    let mut rig = Rig::new();
    let a = rig.host.present_keyed(PANE_A, &blank(), size(64, 64), RED);
    let b = rig
        .host
        .present_keyed(PANE_B, &blank(), size(48, 32), GREEN);
    rig.host.end_frame();
    assert_eq!(rig.host.slot_count(), 2, "end_frame swept a live slot");

    // Frame 2: only A draws.
    rig.host.present_keyed(PANE_A, &blank(), size(64, 64), RED);
    rig.host.end_frame();

    assert_eq!(
        rig.host.slot_count(),
        1,
        "a pane that stopped drawing leaked its slot"
    );
    assert!(
        rig.registered(a),
        "a pane that presented this frame was swept"
    );
    assert!(
        !rig.registered(b),
        "a pane that stopped drawing kept its egui binding"
    );
    rig.assert_no_validation_errors();
}

/// Would catch: liveness defined as "re-rastered this frame". Both live surfaces
/// cache their raster and re-present only when the scene or the size changed, so
/// a pane that is visible but unchanged calls `texture_id` and nothing else. If
/// that did not count as a reference, `end_frame` would free the texture of a
/// pane that is on screen — the single-slot bug back again, one frame later.
///
/// Verified by dropping the `referenced = true` from `texture_id`: `a visible
/// pane that only asked for its id was swept` fired.
#[test]
fn asking_for_a_cached_slots_id_keeps_it_alive() {
    let mut rig = Rig::new();
    let a = rig.host.present_keyed(PANE_A, &blank(), size(64, 64), RED);
    rig.host.end_frame();

    // Frame 2: the pane is visible but unchanged, so it only paints its cache.
    assert_eq!(rig.host.texture_id(PANE_A), Some(a));
    rig.host.end_frame();
    assert!(
        rig.registered(a),
        "a visible pane that only asked for its id was swept"
    );

    // Frame 3: nobody mentions it at all.
    rig.host.end_frame();
    assert!(
        !rig.registered(a),
        "a pane nobody referenced kept its binding"
    );
    rig.assert_no_validation_errors();
}

/// Would catch: `note_rect` recording against the wrong slot, or `last_rect`
/// leaking one pane's geometry to another. Cheap, but the two are a pair and an
/// unasserted pair drifts.
///
/// Verified by making `note_rect` write to every slot: `one pane's rect reached
/// another's slot` fired.
#[test]
fn rects_are_recorded_per_pane() {
    let mut rig = Rig::new();
    rig.host.present_keyed(PANE_A, &blank(), size(64, 64), RED);
    rig.host
        .present_keyed(PANE_B, &blank(), size(48, 32), GREEN);

    let r = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(64.0, 64.0));
    rig.host.note_rect(PANE_A, r);

    assert_eq!(rig.host.last_rect(PANE_A), Some(r));
    assert_eq!(
        rig.host.last_rect(PANE_B),
        None,
        "one pane's rect reached another's slot"
    );
    assert_eq!(
        rig.host
            .last_rect(PaneKey::new(ViewKind::Charts, ItemId::new("absent"))),
        None
    );
    rig.assert_no_validation_errors();
}

/// Would catch: the `CanvasHost` trait forward growing a slot per call, or
/// routing to a key other than the one callers can name — either would put the
/// two live surfaces back on a fresh texture every frame, or make
/// `texture_id(LEGACY_CANVAS)` a lie.
///
/// Verified by pointing `present_scene` at a different constant key: the
/// `texture_id(LEGACY_CANVAS)` assertion fired. The per-call-slot half is
/// covered by the single-slot mutation, which reddens this test too.
#[test]
fn the_trait_forward_reuses_one_legacy_slot() {
    let mut rig = Rig::new();
    let first = rig.host.present_scene(&blank(), size(64, 64), RED);
    let second = rig.host.present_scene(&blank(), size(64, 64), RED);

    assert_eq!(first, second, "the legacy present must reuse its slot");
    assert_eq!(rig.host.slot_count(), 1);
    assert_eq!(rig.host.texture_id(LEGACY_CANVAS), Some(first));
    rig.assert_no_validation_errors();
}

// ---------------------------------------------------------------------------
// The ordering question: present from inside the pane's draw
// ---------------------------------------------------------------------------

/// Present **inside** the pane's draw — after the rect is known, before the
/// image is painted — for two panes in one frame, then render the frame through
/// `egui_wgpu` and read the window back.
///
/// The claim under test: because egui resolves a mesh's bind group at render
/// time (end of frame) and Vello submits on the same queue egui does, updating a
/// binding mid-UI cannot be read too early. If that holds, no pane needs a
/// remembered rect and the resize lag is zero.
///
/// The second frame resizes pane A, which is the case that would break it — a
/// re-pointed binding, a destroyed texture, and a paint that happened before the
/// re-point all inside one frame.
///
/// Would catch: an in-frame present that reads back the panel background,
/// garbage, or another pane's pixels; and any validation error raised by
/// presenting mid-UI.
///
/// Verified twice. Dropping the `Image` paint for pane A fired `pane A, frame 0:
/// got [27, 27, 27, 255], want ~[255, 0, 0, 255]` — the panel background, i.e.
/// the assertion really is reading the presented texture and not the chrome.
/// Asserting blue for pane B fired `pane B, frame 0: got [0, 255, 0, 255]`,
/// which pins the sampling arithmetic to the right rect.
#[test]
fn in_frame_present_paints_both_panes() {
    let mut rig = Rig::new();
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(1.0);
    let screen = egui::vec2(320.0, 160.0);

    // (pane A size, pane B size) per frame. Frame 2 resizes A.
    for (frame, (a_size, b_size)) in [((120u32, 100u32), (120u32, 100u32)), ((60, 60), (120, 100))]
        .into_iter()
        .enumerate()
    {
        let mut rects: Vec<egui::Rect> = Vec::new();
        let host = &mut rig.host;
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
                ..Default::default()
            },
            |ui| {
                egui::containers::CentralPanel::default().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (key, (w, h), base) in [(PANE_A, a_size, RED), (PANE_B, b_size, GREEN)]
                        {
                            // The rect first — this is the ordering the current
                            // shell cannot use, because it presents before the
                            // panels draw.
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(w as f32, h as f32),
                                egui::Sense::hover(),
                            );
                            let id = host.present_keyed(key, &blank(), size(w, h), base);
                            egui::Image::new((id, rect.size()))
                                .tint(egui::Color32::WHITE)
                                .paint_at(ui, rect);
                            rects.push(rect);
                        }
                    });
                });
            },
        );

        upload_font_textures(&rig, &out);
        let pixels = render_frame(&rig, &ctx, out, screen);
        rig.host.end_frame();

        let w = screen.x as u32;
        let (a_rect, b_rect) = (rects[0], rects[1]);
        assert_colour(
            px_at(
                &pixels,
                w,
                a_rect.center().x as u32,
                a_rect.center().y as u32,
            ),
            [255, 0, 0, 255],
            &format!("pane A, frame {frame}"),
        );
        assert_colour(
            px_at(
                &pixels,
                w,
                b_rect.center().x as u32,
                b_rect.center().y as u32,
            ),
            [0, 255, 0, 255],
            &format!("pane B, frame {frame}"),
        );
        assert_eq!(
            (a_rect.width() as u32, a_rect.height() as u32),
            a_size,
            "frame {frame}: pane A did not take the size it asked for"
        );
        rig.assert_no_validation_errors();
    }
}

/// Hand egui's own texture deltas (the font atlas) to the renderer, exactly as
/// eframe does between the UI pass and the paint.
fn upload_font_textures(rig: &Rig, out: &egui::FullOutput) {
    let mut r = rig.egui_renderer.write();
    for (id, delta) in &out.textures_delta.set {
        r.update_texture(&rig.device, &rig.queue, *id, delta);
    }
    for id in &out.textures_delta.free {
        r.free_texture(id);
    }
}

/// Tessellate and render one egui frame into an offscreen `Rgba8Unorm` target
/// cleared to opaque black, and read it back tightly packed.
fn render_frame(
    rig: &Rig,
    ctx: &egui::Context,
    out: egui::FullOutput,
    screen: egui::Vec2,
) -> Vec<u8> {
    let clipped = ctx.tessellate(out.shapes, 1.0);
    let size_px = [screen.x as u32, screen.y as u32];
    let desc = egui_wgpu::ScreenDescriptor {
        size_in_pixels: size_px,
        pixels_per_point: 1.0,
    };
    let target = rig.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("keyed-canvas-test-target"),
        size: wgpu::Extent3d {
            width: size_px[0],
            height: size_px[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = rig
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("keyed-canvas-test-encoder"),
        });
    let user_cmds = {
        let mut r = rig.egui_renderer.write();
        r.update_buffers(&rig.device, &rig.queue, &mut encoder, &clipped, &desc)
    };
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("keyed-canvas-test-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        rig.egui_renderer.read().render(&mut pass, &clipped, &desc);
    }
    rig.queue.submit(
        user_cmds
            .into_iter()
            .chain(std::iter::once(encoder.finish())),
    );

    rig.vello
        .lock()
        .expect("VelloRenderer mutex poisoned")
        .read_texture(&target, size_px[0], size_px[1])
}

/// The load-bearing property behind the in-frame present, isolated: a mesh
/// painted **before** `present_keyed` still shows the pixels that present
/// produced.
///
/// Frame 2 paints pane A using the `TextureId` held from frame 1, and only then
/// presents at a new size and a new colour — so the mesh already in the frame's
/// shape list carries an id whose binding is replaced, and whose old texture is
/// dropped, after the paint call returned. If egui resolved bindings at paint
/// time this would read the old colour (or dangle on a destroyed texture and
/// raise a validation error). It reads the new colour, cleanly.
///
/// That is why `last_rect` is not needed to avoid the resize lag: a pane may
/// present from inside its own draw, once its rect is known, and paint in either
/// order.
///
/// Would catch: an egui or wgpu upgrade that starts snapshotting bind groups at
/// paint time, or a `present_keyed` that registers a fresh id on resize instead
/// of re-pointing — which strands the held id.
///
/// Verified by making the resize branch register a new id rather than re-point
/// (the M3 mutation): `frame 2 painted the pre-present id` fired, reading the
/// stale red.
#[test]
fn a_mesh_painted_before_the_present_still_shows_the_presented_pixels() {
    let mut rig = Rig::new();
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(1.0);
    let screen = egui::vec2(320.0, 160.0);

    let mut held: Option<egui::TextureId> = None;
    for (frame, (w, h, base, want)) in [
        (120u32, 100u32, RED, [255u8, 0, 0, 255]),
        (60, 60, BLUE, [0, 0, 255, 255]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut painted: Option<egui::Rect> = None;
        let host = &mut rig.host;
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
                ..Default::default()
            },
            |ui| {
                egui::containers::CentralPanel::default().show(ui, |ui| {
                    let (rect, _) = ui
                        .allocate_exact_size(egui::vec2(w as f32, h as f32), egui::Sense::hover());
                    // Paint first, with whatever id is already held...
                    if let Some(id) = held {
                        egui::Image::new((id, rect.size()))
                            .tint(egui::Color32::WHITE)
                            .paint_at(ui, rect);
                    }
                    // ...then present, which on the second frame rebuilds the
                    // texture and re-points the id the mesh above already
                    // carries.
                    let id = host.present_keyed(PANE_A, &blank(), size(w, h), base);
                    if held.is_none() {
                        egui::Image::new((id, rect.size()))
                            .tint(egui::Color32::WHITE)
                            .paint_at(ui, rect);
                    }
                    held = Some(id);
                    painted = Some(rect);
                });
            },
        );
        upload_font_textures(&rig, &out);
        let pixels = render_frame(&rig, &ctx, out, screen);
        rig.host.end_frame();

        let c = painted.expect("the pane drew").center();
        assert_colour(
            px_at(&pixels, screen.x as u32, c.x as u32, c.y as u32),
            want,
            &format!("frame {frame} painted the pre-present id"),
        );
        rig.assert_no_validation_errors();
    }
}
