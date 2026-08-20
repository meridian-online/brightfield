//! Headless real-UI capture (Tier-2): boot the real shell on egui, render the
//! full window through `egui_wgpu` into an offscreen wgpu texture, read it back,
//! write a PNG.
//!
//! This is the loop the gpui host never had: no display server, no window — the
//! same [`crate::window::MeridianApp`] that runs live is driven by a synthetic
//! [`egui::RawInput`] and rendered by egui's real wgpu backend, so the PNG is
//! the actual UI, Vello canvas included, not a Vello-only composite.

use std::path::Path;
use std::sync::Arc;

use brightfield_render::vello_renderer::{device_limits, VelloRenderer};
use meridian_design::chrome::{INK_DARK, INK_LIGHT};
use vello::wgpu;

use crate::canvas::SharedEguiRenderer;
use crate::design::Mode;
use crate::pipeline::Composed;
use crate::protocol::host_on_device;
use crate::window::{Boot, MeridianApp};

/// The device pixel ratio a capture runs at when nobody names one — what
/// `brightfield-shot` uses without `--scale`, and a HiDPI screen's ratio.
///
/// Declared beside the capture rather than inside the binary's argument parser
/// so a test can hold the capture path **at the setting the shipped default
/// actually uses**. It was a `2.0` literal in `brightfield-shot`'s parser and a
/// `1.0` literal in the test, and the gap between the two hid a capture defect
/// that bit at every scale except exactly 1.0: the test issued its certificate
/// at the one value where the bug did not fire. A shared constant is what makes
/// moving the default move the guard with it.
pub const DEFAULT_SCALE: f32 = 2.0;

/// Create a headless (surface-less) wgpu device on the default adapter.
///
/// Asks for [`device_limits`] — the adapter's real limits — so a capture and
/// the live window share one ceiling. A capture on the conservative default
/// limits would refuse scenes the window draws, or draw scenes the window
/// refuses, and either way the PNG would stop being evidence about the window.
///
/// # Errors
/// Returns a message if no adapter or device is available.
pub fn headless_device() -> Result<(wgpu::Device, wgpu::Queue), String> {
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .map_err(|e| format!("no suitable GPU adapter: {e}"))?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("brightfield-shot"),
        required_features: wgpu::Features::empty(),
        required_limits: device_limits(&adapter),
        memory_hints: wgpu::MemoryHints::default(),
        ..Default::default()
    }))
    .map_err(|e| format!("device creation failed: {e}"))
}

/// Render the window to `out` as a PNG at `scale` device pixels per logical
/// point, optionally applying one scripted frame of events per entry in
/// `script`. Returns the device pixel dimensions written.
///
/// **One entry point for both documents.** It used to be two — `capture_png`
/// over the chart shell and `capture_protocol_png` over the protocol shell —
/// and those two functions were two of the four places the old fork was
/// load-bearing: each constructed a different shell, read a different
/// `window_size`, and could only ever photograph the surface it was named
/// after. [`Boot`] carries both documents' contents and derives which one the
/// canvas holds, so what used to be the choice of function is now the
/// contents.
///
/// The window is sized from the boot's content, which is the right answer for
/// every boot that *has* content and a wrong one for [`Boot::empty`] — an
/// empty boot's content self-measures a few tens of points, and a capture at
/// that size photographs a sliver of top bar and calls it the front door. A
/// capture of the empty window goes through [`capture_png_at`] with the size
/// said out loud.
///
/// # Errors
/// Returns a message on GPU, encode, or file-write failure.
pub fn capture_png(
    boot: Boot,
    mode: Mode,
    scale: f32,
    out: &Path,
    script: Vec<Vec<egui::Event>>,
) -> Result<(u32, u32), String> {
    let size = boot.window_size();
    capture_png_at(boot, mode, scale, size, out, script)
}

/// [`capture_png`] at an explicit logical window size, instead of the size the
/// boot's content asks for.
///
/// The caller that needs this is the one whose boot has no content to derive
/// a size from: [`Boot::empty`], whose window is the front door. The live
/// binary answers that case from the saved geometry or
/// [`WindowGeometry`](brightfield_workbench::WindowGeometry)'s default — see
/// [`Boot::is_empty`] — and a capture has neither, so the size is a parameter
/// rather than a guess.
///
/// # Errors
/// As [`capture_png`].
pub fn capture_png_at(
    boot: Boot,
    mode: Mode,
    scale: f32,
    size: (f32, f32),
    out: &Path,
    script: Vec<Vec<egui::Event>>,
) -> Result<(u32, u32), String> {
    capture_png_at_with_layout(
        boot,
        crate::startup::default_layout(),
        mode,
        scale,
        size,
        out,
        script,
    )
}

/// [`capture_png_at`] over a window arranged as `layout` says, instead of over
/// the default arrangement.
///
/// The caller that needs this is the front door's **returning** state: the
/// door's two halves differ only in what the layout remembers, so
/// photographing the second one means handing in a layout that remembers
/// something. Everything else about the capture is identical, which is the
/// property that makes the pair of baselines comparable — a difference between
/// them is a difference the recents made.
///
/// Like [`MeridianApp::with_layout`], the layout is a parameter rather than
/// something this function reads: a capture that read the developer's real
/// `workspace-layout.json` would photograph their session.
///
/// # Errors
/// As [`capture_png`].
#[allow(clippy::too_many_arguments)]
pub fn capture_png_at_with_layout(
    boot: Boot,
    layout: brightfield_workbench::SavedLayout,
    mode: Mode,
    scale: f32,
    size: (f32, f32),
    out: &Path,
    script: Vec<Vec<egui::Event>>,
) -> Result<(u32, u32), String> {
    let (device, queue) = headless_device()?;
    let target_format = wgpu::TextureFormat::Rgba8Unorm;
    let egui_renderer = new_egui_renderer(&device, target_format);
    // One host per document, as the live window builds them: a document owns
    // the canvas it rasters into, and the two views' rasters are independent.
    let chart_host = host_on_device(device.clone(), queue.clone(), egui_renderer.clone());
    let protocol_host = host_on_device(device.clone(), queue.clone(), egui_renderer.clone());

    let (win_w, win_h) = size;
    let mut app = MeridianApp::with_layout(boot, layout, chart_host, protocol_host, mode);

    let ctx = egui::Context::default();
    let screen = egui::vec2(win_w, win_h);

    let full = run_ui_frames(
        &ctx,
        &egui_renderer,
        &device,
        &queue,
        screen,
        scale,
        script,
        |ui| {
            app.draw(ui);
        },
    );
    finish_capture(
        &ctx,
        &egui_renderer,
        &device,
        &queue,
        full,
        win_w,
        win_h,
        scale,
        mode,
        target_format,
        out,
    )
}

/// A headless `egui_wgpu` renderer with the shot's deterministic options
/// (dithering off, predictable filtering) so captures are stable across machines.
fn new_egui_renderer(device: &wgpu::Device, format: wgpu::TextureFormat) -> SharedEguiRenderer {
    let r = egui_wgpu::Renderer::new(
        device,
        format,
        egui_wgpu::RendererOptions {
            msaa_samples: 1,
            depth_stencil_format: None,
            dithering: false,
            predictable_texture_filtering: true,
        },
    );
    Arc::new(egui::mutex::RwLock::new(r))
}

/// One capture frame's input: the window's logical rect, this frame's events,
/// and `scale` as the viewport's **`native_pixels_per_point`**.
///
/// The scale is carried here, on the input, rather than through
/// [`egui::Context::set_pixels_per_point`] — and that is a correctness
/// requirement, not a preference. `set_pixels_per_point` is a *zoom* control:
/// it defers to the start of the next pass, and when that pass begins egui
/// **replaces the caller's `screen_rect`** with the previous pass's content
/// rect divided by the zoom ratio (`Context::begin_pass`, the "bit hacky, but
/// is required to avoid jitter" branch). Before any pass has run there is no
/// previous content rect to divide, only `InputState`'s default. Measured, at
/// scale 2 on `examples/bars.yaml`: the first frame laid the whole window out
/// at 5000×5000 logical points, the dashboard reflowed into that box (composed
/// 640×480 → 3962×4904), and the next frame rastered the reflowed size onto a
/// 7924×9808 texture. A vello raster of that size comes back **uniform** — one
/// distinct colour over 7680×5760, at exit 0, measured through `--vello-only`
/// — so the PNG carried an empty chart pane and reported success. At exactly
/// 1.0 the setter is a no-op, nothing was ever clobbered, and the capture drew;
/// that is the whole of "every scale except 1.0".
///
/// `native_pixels_per_point` is plain input: egui multiplies it by the zoom
/// factor (left at 1.0) to get `pixels_per_point`, from the very first pass,
/// and never rewrites `screen_rect` over it. It is also what a HiDPI capture
/// means — the same field `egui-winit` fills from the window's scale factor —
/// so the capture and the live window now reach `pixels_per_point` the same
/// way.
fn frame_input(screen: egui::Vec2, scale: f32, events: Vec<egui::Event>) -> egui::RawInput {
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
        events,
        ..Default::default()
    };
    let id = raw.viewport_id;
    raw.viewports.entry(id).or_default().native_pixels_per_point = Some(scale);
    raw
}

/// The most settle frames a capture will run before giving up on the window
/// ever going quiet. At the 1/60 s `predicted_dt` egui advances its clock by
/// when [`frame_input`] hands it no `time`, this is four seconds of animation
/// — an order of magnitude past `Style::animation_time`, which the design
/// system sets to 120 ms. A capture that reaches it has found a window that
/// asks for a new frame forever, and [`run_ui_frames`] panics rather than
/// photographing whatever the last one happened to hold.
const MAX_SETTLE_FRAMES: usize = 240;

/// Whether `out` asks for another frame **immediately** — egui's way of saying
/// "what I just drew is not the final appearance".
///
/// `repaint_delay` is the shortest delay any viewport asked for; zero is the
/// value [`egui::Context::request_repaint`] writes, and `Duration::MAX` is a
/// window with nothing outstanding.
fn wants_another_frame(out: &egui::FullOutput) -> bool {
    out.viewport_output
        .values()
        .any(|v| v.repaint_delay.is_zero())
}

/// Drive the window's draw through a warm-up frame (fonts atlas + layout
/// settle), the scripted frames (one per entry), and then settle frames until
/// the window stops asking for another — keeping the renderer's texture atlas
/// current each frame. The last frame run is the one captured. Returns its
/// [`egui::FullOutput`].
///
/// # Why settling is a loop and not one frame
///
/// It used to be one frame, and that is what made every modal baseline a
/// picture of something the app never draws. `egui::Area` — which is what
/// `egui::Modal`, and so every `meridian_egui::ModalLayer` overlay, floats on
/// — **fades in**. `Area::begin` reads how long the area has been visible,
/// remaps it over `Style::animation_time`, and calls `ui.multiply_opacity` with
/// the result, which scales the alpha of every shape in that layer: the card's
/// fill, its hairline, its shadow and the modal's backdrop scrim all together.
/// While the fade is unfinished it calls `ctx.request_repaint()`.
///
/// A fixed frame list drops that request. One settle frame after the keystroke
/// that opens a modal is ~25 ms of egui clock against a 120 ms animation, so
/// the layer was photographed at roughly a third of its opacity: the opaque
/// overlay surface composited as a wash, the chart behind it read straight
/// through the command list, and the scrim dimmed the page by a fraction of
/// what the token asks for. The live window has no such stop — eframe honours
/// the repaint request, so by the time anyone looks the fade is over and the
/// card is opaque. **The product was right and the capture path was wrong**,
/// which is exactly the direction that is invisible to a golden.
///
/// Honouring the request settles every time-driven animation rather than this
/// one, and it leaves a window with nothing animating on the same single
/// settle frame it had before — so a capture with no animation in it is
/// byte-identical to what this ran previously.
#[allow(clippy::too_many_arguments)]
fn run_ui_frames(
    ctx: &egui::Context,
    egui_renderer: &SharedEguiRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    screen: egui::Vec2,
    scale: f32,
    script: Vec<Vec<egui::Event>>,
    mut draw: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    let mut one = |events: Vec<egui::Event>| {
        let out = ctx.run_ui(frame_input(screen, scale, events), |ui| draw(ui));
        {
            let mut r = egui_renderer.write();
            for (id, delta) in &out.textures_delta.set {
                r.update_texture(device, queue, *id, delta);
            }
            for id in &out.textures_delta.free {
                r.free_texture(id);
            }
        }
        out
    };

    let mut frames: Vec<Vec<egui::Event>> = Vec::with_capacity(script.len() + 1);
    frames.push(Vec::new());
    frames.extend(script);

    let mut full: Option<egui::FullOutput> = None;
    for events in frames {
        full = Some(one(events));
    }

    for settled in 0..MAX_SETTLE_FRAMES {
        let out = one(Vec::new());
        let quiet = !wants_another_frame(&out);
        full = Some(out);
        if quiet {
            return full.expect("a settle frame ran");
        }
        assert!(
            settled + 1 < MAX_SETTLE_FRAMES,
            "the window still asked for an immediate repaint after \
             {MAX_SETTLE_FRAMES} settle frames, so no frame here is the \
             settled appearance. Find what calls request_repaint every frame \
             — an unfinished egui animation settles well inside this — rather \
             than raising the cap"
        );
    }
    unreachable!("the loop above returns or asserts on its last iteration")
}

/// Tessellate + render the final frame into an offscreen target on the page tone,
/// read it back, and write the PNG. Returns the device pixel dimensions.
#[allow(clippy::too_many_arguments)]
fn finish_capture(
    ctx: &egui::Context,
    egui_renderer: &SharedEguiRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    full: egui::FullOutput,
    win_w: f32,
    win_h: f32,
    scale: f32,
    mode: Mode,
    target_format: wgpu::TextureFormat,
    out: &Path,
) -> Result<(u32, u32), String> {
    let clipped = ctx.tessellate(full.shapes, scale);
    let size_px = [
        ((win_w * scale).round() as u32).max(1),
        ((win_h * scale).round() as u32).max(1),
    ];
    let screen_desc = egui_wgpu::ScreenDescriptor {
        size_in_pixels: size_px,
        pixels_per_point: scale,
    };

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("brightfield-shot-target"),
        size: wgpu::Extent3d {
            width: size_px[0],
            height: size_px[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: target_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let page = match mode {
        Mode::Light => INK_LIGHT.page,
        Mode::Dark => INK_DARK.page,
    };
    let clear = wgpu::Color {
        r: f64::from(page.r),
        g: f64::from(page.g),
        b: f64::from(page.b),
        a: 1.0,
    };

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("brightfield-shot-encoder"),
    });
    let user_cmds = {
        let mut r = egui_renderer.write();
        r.update_buffers(device, queue, &mut encoder, &clipped, &screen_desc)
    };
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("brightfield-shot-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        let r = egui_renderer.read();
        r.render(&mut pass, &clipped, &screen_desc);
    }
    queue.submit(
        user_cmds
            .into_iter()
            .chain(std::iter::once(encoder.finish())),
    );

    let pixels = read_texture(device, queue, &target, size_px[0], size_px[1]);
    let img = image::RgbaImage::from_raw(size_px[0], size_px[1], pixels)
        .ok_or_else(|| "readback pixel buffer size mismatch".to_string())?;
    img.save(out)
        .map_err(|e| format!("failed to write {}: {e}", out.display()))?;
    Ok((size_px[0], size_px[1]))
}

/// Run the real shell headlessly for `frames` frames and time each one — the
/// measurement twin of [`capture_png`], for the performance baseline.
///
/// Every frame is a complete produce-a-picture cycle, timed end to end:
///
/// 1. `on_frame(&mut app, i)` — the caller's per-frame action (inject an
///    interaction through [`MeridianApp::chart_doc_mut`], or nothing for a
///    steady-state frame);
/// 2. the egui pass over the real [`MeridianApp::draw`] (which is where a
///    live-document interaction's re-query, re-composite and canvas re-raster
///    happen, exactly as in the live window);
/// 3. tessellation + the `egui_wgpu` render pass into an offscreen target;
/// 4. a blocking wait for the GPU to finish the submitted work.
///
/// What it deliberately is NOT: there is no swapchain, no present and no
/// vsync — the number is the cost of *producing* a frame, not of displaying
/// one — and no pixel readback (a live frame never reads back). The first
/// frame includes one-off costs (font atlas upload, layout settle, first
/// materialisation): callers discard warm-up frames themselves so the discard
/// count is visible at the call site, next to the numbers it shapes.
///
/// # Errors
/// Returns a message if no GPU adapter/device is available.
pub fn bench_frames(
    boot: Boot,
    mode: Mode,
    scale: f32,
    frames: usize,
    mut on_frame: impl FnMut(&mut MeridianApp, usize),
) -> Result<Vec<std::time::Duration>, String> {
    let (device, queue) = headless_device()?;
    let target_format = wgpu::TextureFormat::Rgba8Unorm;
    let egui_renderer = new_egui_renderer(&device, target_format);
    let chart_host = host_on_device(device.clone(), queue.clone(), egui_renderer.clone());
    let protocol_host = host_on_device(device.clone(), queue.clone(), egui_renderer.clone());

    let (win_w, win_h) = boot.window_size();
    let mut app = MeridianApp::new(boot, chart_host, protocol_host, mode);

    let ctx = egui::Context::default();
    let screen = egui::vec2(win_w, win_h);

    let size_px = [
        ((win_w * scale).round() as u32).max(1),
        ((win_h * scale).round() as u32).max(1),
    ];
    let screen_desc = egui_wgpu::ScreenDescriptor {
        size_in_pixels: size_px,
        pixels_per_point: scale,
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("brightfield-bench-target"),
        size: wgpu::Extent3d {
            width: size_px[0],
            height: size_px[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: target_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let page = match mode {
        Mode::Light => INK_LIGHT.page,
        Mode::Dark => INK_DARK.page,
    };
    let clear = wgpu::Color {
        r: f64::from(page.r),
        g: f64::from(page.g),
        b: f64::from(page.b),
        a: 1.0,
    };

    let mut times = Vec::with_capacity(frames);
    for i in 0..frames {
        let started = std::time::Instant::now();
        on_frame(&mut app, i);

        let out = ctx.run_ui(frame_input(screen, scale, Vec::new()), |ui| app.draw(ui));
        {
            let mut r = egui_renderer.write();
            for (id, delta) in &out.textures_delta.set {
                r.update_texture(&device, &queue, *id, delta);
            }
            for id in &out.textures_delta.free {
                r.free_texture(id);
            }
        }
        let clipped = ctx.tessellate(out.shapes, scale);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("brightfield-bench-encoder"),
        });
        let user_cmds = {
            let mut r = egui_renderer.write();
            r.update_buffers(&device, &queue, &mut encoder, &clipped, &screen_desc)
        };
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("brightfield-bench-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(clear),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            let r = egui_renderer.read();
            r.render(&mut pass, &clipped, &screen_desc);
        }
        queue.submit(
            user_cmds
                .into_iter()
                .chain(std::iter::once(encoder.finish())),
        );
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("GPU wait failed: {e:?}"))?;
        times.push(started.elapsed());
    }
    Ok(times)
}

/// Render one gallery component solo — themed page, no window chrome — to a
/// PNG. The `--gallery <component-id>` arm of `brightfield-shot`.
///
/// This is the confirmation that the determinism apparatus generalises past
/// `--spec`: the headless device, the deterministic renderer options
/// (`new_egui_renderer`: dithering off, predictable filtering), the
/// warm-up/settle frame loop (`run_ui_frames`) and the readback
/// (`finish_capture`) are all spec-agnostic — only [`Boot`] is spec-shaped,
/// and a component needs no boot. The frame drawn is
/// [`crate::gallery::solo`], the same composition the pixel-tier gate
/// measures, so what an agent screenshots is what the gate held.
///
/// `size` is the logical window; `None` takes the component's own
/// [`solo_size`](crate::gallery::ComponentInfo::solo_size). (On this path
/// `--size` is honoured; on the `--spec` path it remains a reserved
/// override, because there the window is derived from the document.)
///
/// # Errors
/// An unknown id — reported with the catalog, before any GPU work — or a
/// GPU, encode, or file-write failure.
pub fn capture_component(
    id: &str,
    mode: Mode,
    scale: f32,
    size: Option<(f32, f32)>,
    out: &Path,
) -> Result<(u32, u32), String> {
    let mut component = crate::gallery::catalog()
        .into_iter()
        .find(|c| c.info().id == id)
        .ok_or_else(|| {
            let ids: Vec<&str> = crate::gallery::catalog()
                .iter()
                .map(|c| c.info().id)
                .collect();
            format!(
                "unknown gallery component {id:?}; catalog: {}",
                ids.join(", ")
            )
        })?;
    let (win_w, win_h) = size.unwrap_or(component.info().solo_size);

    let (device, queue) = headless_device()?;
    let target_format = wgpu::TextureFormat::Rgba8Unorm;
    let egui_renderer = new_egui_renderer(&device, target_format);

    let ctx = egui::Context::default();
    let screen = egui::vec2(win_w, win_h);

    let full = run_ui_frames(
        &ctx,
        &egui_renderer,
        &device,
        &queue,
        screen,
        scale,
        Vec::new(),
        |ui| {
            crate::gallery::solo(ui, mode, component.as_mut());
        },
    );
    finish_capture(
        &ctx,
        &egui_renderer,
        &device,
        &queue,
        full,
        win_w,
        win_h,
        scale,
        mode,
        target_format,
        out,
    )
}

/// Render just the composited Vello dashboard (no egui chrome) to a PNG on a
/// dedicated device — the pipeline's "Vello baseline", for parity-checking the
/// egui path against the app's `BRIGHTFIELD_DUMP_PNG` output. `scale` scales the
/// scene onto the device-resolution raster.
///
/// # Errors
/// Returns a message on GPU or file-write failure.
pub fn capture_vello_only(
    composed: Composed,
    scale: f32,
    out: &Path,
) -> Result<(u32, u32), String> {
    let dev_w = ((composed.width as f32) * scale).round().max(1.0) as u32;
    let dev_h = ((composed.height as f32) * scale).round().max(1.0) as u32;
    let mut scaled = vello::Scene::new();
    scaled.append(
        &composed.scene,
        Some(kurbo::Affine::scale(f64::from(scale))),
    );
    let renderer = VelloRenderer::new();
    let pixels = renderer
        .lock()
        .expect("renderer poisoned")
        .render_to_pixels(&scaled, dev_w, dev_h);
    let img = image::RgbaImage::from_raw(dev_w, dev_h, pixels)
        .ok_or_else(|| "vello pixel buffer size mismatch".to_string())?;
    img.save(out)
        .map_err(|e| format!("write {}: {e}", out.display()))?;
    Ok((dev_w, dev_h))
}

/// Fit-and-pad (letterbox) a capture into exactly `w`×`h` — the shape a
/// shipped start's gallery thumbnail takes.
///
/// One definition, used by the regeneration test that produces the committed
/// `assets/starts/*.png` files and by nothing else at runtime: the shipped
/// thumbnail is bytes in the binary, never a render. **Letterbox rather than
/// centre-crop**, so a start's whole dashboard shows on its card instead of a
/// centre slice of it — every card is still the same `w`×`h` shape whatever
/// window its content asked for, the picture inside is just fitted rather than
/// cropped. Lanczos3 because it is deterministic for a given input, which is
/// what lets the regeneration test hold the committed file against the
/// bundled spec.
///
/// The pad is **transparent**, not an inked tone: the door draws each card
/// over its own `surfaces.raised` and composites the thumbnail with its alpha
/// honoured (`ColorImage::from_rgba_unmultiplied`), so transparent bars take
/// the card's colour in both light and dark. A fixed grey would fight one mode
/// or the other; transparent reads as the fitted picture floating on a uniform
/// card. The capture's own opaque page tone stays inside the fitted rectangle.
#[must_use]
pub fn thumbnail(capture: &image::RgbaImage, w: u32, h: u32) -> image::RgbaImage {
    let fitted = image::DynamicImage::ImageRgba8(capture.clone())
        .resize(w, h, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let mut canvas = image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 0]));
    let x = i64::from((w - fitted.width()) / 2);
    let y = i64::from((h - fitted.height()) / 2);
    image::imageops::overlay(&mut canvas, &fitted, x, y);
    canvas
}

/// Read an `Rgba8Unorm` texture back into tightly-packed RGBA bytes.
fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let bytes_per_row = width * 4;
    let padded = (bytes_per_row + 255) & !255;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("brightfield-shot-readback"),
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("readback"),
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("GPU poll failed during readback");
    rx.recv()
        .expect("readback channel closed")
        .expect("readback failed");

    let data = slice.get_mapped_range();
    if padded == bytes_per_row {
        data.to_vec()
    } else {
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let start = (row * padded) as usize;
            out.extend_from_slice(&data[start..start + bytes_per_row as usize]);
        }
        out
    }
}

/// Parse a `--script` ndjson file into per-frame egui events. Supported line
/// forms (one JSON object per line):
///
/// - `{"pointer":[x,y]}`  — move the pointer to logical (x,y)
/// - `{"click":[x,y]}`    — move + press + release the primary button
/// - `{"key":"A"}`        — press+release a named key
/// - `{"key":"S","shift":true}` — with the shift modifier (chorded verbs)
///
/// The `z a` fold chord is two lines: `{"key":"Z"}` then `{"key":"A"}`.
/// Unknown lines are skipped with a warning.
///
/// # Errors
/// Returns a message if the file cannot be read.
pub fn parse_script(path: &Path) -> Result<Vec<Vec<egui::Event>>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut frames = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("script line {}: skipped ({e})", n + 1);
                continue;
            }
        };
        let mut events = Vec::new();
        if let Some(p) = v.get("pointer").and_then(xy) {
            events.push(egui::Event::PointerMoved(egui::pos2(p.0, p.1)));
        } else if let Some(p) = v.get("click").and_then(xy) {
            let pos = egui::pos2(p.0, p.1);
            events.push(egui::Event::PointerMoved(pos));
            for pressed in [true, false] {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                });
            }
        } else if let Some(k) = v.get("key").and_then(|k| k.as_str()) {
            if let Some(key) = egui::Key::from_name(k) {
                // Optional `"shift": true` for chorded verbs (e.g. shift-S).
                let shift = v
                    .get("shift")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let modifiers = egui::Modifiers {
                    shift,
                    ..Default::default()
                };
                for pressed in [true, false] {
                    events.push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers,
                    });
                }
            }
        }
        if !events.is_empty() {
            frames.push(events);
        }
    }
    Ok(frames)
}

fn xy(v: &serde_json::Value) -> Option<(f32, f32)> {
    let a = v.as_array()?;
    Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
}
