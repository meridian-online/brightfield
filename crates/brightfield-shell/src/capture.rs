//! Headless real-UI capture (Tier-2): boot the real shell on egui, render the
//! full window through `egui_wgpu` into an offscreen wgpu texture, read it back,
//! write a PNG.
//!
//! This is the loop the gpui host never had: no display server, no window — the
//! same [`crate::app::draw_shell`] that runs live is driven by a synthetic
//! [`egui::RawInput`] and rendered by egui's real wgpu backend, so the PNG is
//! the actual UI, Vello canvas included, not a Vello-only composite.

use std::path::Path;
use std::sync::Arc;

use brightfield_protocol::layout::Flow;
use brightfield_render::vello_renderer::VelloRenderer;
use meridian_design::chrome::{INK_DARK, INK_LIGHT};
use vello::wgpu;

use crate::app::{draw_shell, ShellState};
use crate::canvas::{EguiCanvasHost, SharedEguiRenderer};
use crate::design::Mode;
use crate::pipeline::Composed;
use crate::protocol::{host_on_device, ProtocolInputs, ProtocolShell};

/// Create a headless (surface-less) wgpu device on the default adapter.
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
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::default(),
        ..Default::default()
    }))
    .map_err(|e| format!("device creation failed: {e}"))
}

/// Render the shell to `out` as a PNG at `scale` device pixels per logical
/// point, optionally applying one scripted frame of events per entry in
/// `script`. Returns the device pixel dimensions written.
///
/// # Errors
/// Returns a message on GPU, encode, or file-write failure.
pub fn capture_png(
    composed: Composed,
    mode: Mode,
    scale: f32,
    out: &Path,
    script: Vec<Vec<egui::Event>>,
) -> Result<(u32, u32), String> {
    let (device, queue) = headless_device()?;
    let target_format = wgpu::TextureFormat::Rgba8Unorm;
    let egui_renderer = new_egui_renderer(&device, target_format);
    let vello = VelloRenderer::from_shared(device.clone(), queue.clone());
    let host = EguiCanvasHost::new(device.clone(), queue.clone(), vello, egui_renderer.clone());

    let mut state = ShellState::new(composed, host, mode);
    let (win_w, win_h) = state.window_size();

    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(scale);
    let screen = egui::vec2(win_w, win_h);

    let full = run_ui_frames(
        &ctx,
        &egui_renderer,
        &device,
        &queue,
        screen,
        script,
        |ui| {
            draw_shell(ui, &mut state);
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

/// Render the egui **Protocol panel** (dock + DAG + outline + inspector + steps)
/// to `out` as a PNG at `scale`, optionally seeding the selection to `focus`
/// (the dotted asset id a click would target) before applying one scripted frame
/// of events per entry in `script`. The loop the gpui protocol shell never had:
/// prove a keystroke drives the view by capturing the actual pixels.
///
/// # Errors
/// Returns a message on GPU, encode, or file-write failure.
pub fn capture_protocol_png(
    inputs: ProtocolInputs,
    mode: Mode,
    flow: Flow,
    focus: Option<String>,
    scale: f32,
    out: &Path,
    script: Vec<Vec<egui::Event>>,
) -> Result<(u32, u32), String> {
    let (device, queue) = headless_device()?;
    let target_format = wgpu::TextureFormat::Rgba8Unorm;
    let egui_renderer = new_egui_renderer(&device, target_format);
    let host = host_on_device(device.clone(), queue.clone(), egui_renderer.clone());

    let mut shell = ProtocolShell::new(inputs, host, mode, flow);
    if let Some(id) = focus {
        shell.model_mut().select_id(id);
    }
    let (win_w, win_h) = shell.window_size();

    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(scale);
    let screen = egui::vec2(win_w, win_h);

    let full = run_ui_frames(
        &ctx,
        &egui_renderer,
        &device,
        &queue,
        screen,
        script,
        |ui| {
            shell.draw(ui);
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

/// Drive the shared draw function through a warm-up frame (fonts atlas + layout
/// settle), the scripted frames (one per entry), and a final settle frame that
/// becomes the captured image — keeping the renderer's texture atlas current each
/// frame. Returns the last frame's [`egui::FullOutput`].
fn run_ui_frames(
    ctx: &egui::Context,
    egui_renderer: &SharedEguiRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    screen: egui::Vec2,
    script: Vec<Vec<egui::Event>>,
    mut draw: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    let mut frames: Vec<Vec<egui::Event>> = Vec::with_capacity(script.len() + 2);
    frames.push(Vec::new());
    frames.extend(script);
    frames.push(Vec::new());

    let mut full: Option<egui::FullOutput> = None;
    for events in frames {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
            events,
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| draw(ui));
        {
            let mut r = egui_renderer.write();
            for (id, delta) in &out.textures_delta.set {
                r.update_texture(device, queue, *id, delta);
            }
            for id in &out.textures_delta.free {
                r.free_texture(id);
            }
        }
        full = Some(out);
    }
    full.expect("at least one frame ran")
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
