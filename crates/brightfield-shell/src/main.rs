//! `brightfield-shell` — the live egui/eframe window (Tier-3 of the loop).
//!
//! Opens the real docked-ish shell over a Mosaic spec, rendering the Vello
//! dashboard on eframe's shared wgpu device. Press F12 to fire a
//! `ViewportCommand::Screenshot` and write a PNG of the live window for
//! on-device sign-off (the request is issued in `update`, the image captured
//! from the following frame's input events — eframe's post-render hand-back).
//!
//! Usage: `brightfield-shell [SPEC.yaml] [--theme light|dark] [--shot-out PATH]`

use std::path::PathBuf;
use std::sync::Arc;

use brightfield_render::vello_renderer::VelloRenderer;
use brightfield_shell::app::{draw_shell, ShellState};
use brightfield_shell::canvas::EguiCanvasHost;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::compose_spec;

struct Args {
    spec: String,
    mode: Mode,
    shot_out: PathBuf,
}

fn parse_args() -> Args {
    let mut spec = "examples/dashboard.yaml".to_string();
    let mut mode = Mode::Light;
    let mut shot_out = PathBuf::from("render-proof/live-screenshot.png");
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--theme" => {
                if let Some(v) = it.next() {
                    mode = if v == "dark" { Mode::Dark } else { Mode::Light };
                }
            }
            "--shot-out" => {
                if let Some(v) = it.next() {
                    shot_out = PathBuf::from(v);
                }
            }
            other if !other.starts_with("--") => spec = other.to_string(),
            other => eprintln!("ignoring unknown flag {other}"),
        }
    }
    Args { spec, mode, shot_out }
}

struct ShellApp {
    state: ShellState,
    shot_out: PathBuf,
    shot_pending: bool,
}

impl eframe::App for ShellApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        draw_shell(ui, &mut self.state);

        // Tier-3: request a live-window screenshot on F12.
        if ctx.input(|i| i.key_pressed(egui::Key::F12)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.shot_pending = true;
        }
        // Capture the screenshot handed back on a later frame.
        if self.shot_pending {
            let image = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = image {
                self.shot_pending = false;
                if let Err(e) = save_color_image(&image, &self.shot_out) {
                    eprintln!("screenshot save failed: {e}");
                } else {
                    eprintln!("live screenshot written: {}", self.shot_out.display());
                }
            }
        }
    }
}

fn save_color_image(image: &egui::ColorImage, out: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let [w, h] = image.size;
    let mut bytes = Vec::with_capacity(w * h * 4);
    for px in &image.pixels {
        bytes.extend_from_slice(&[px.r(), px.g(), px.b(), px.a()]);
    }
    let img = image::RgbaImage::from_raw(w as u32, h as u32, bytes)
        .ok_or_else(|| "screenshot buffer size mismatch".to_string())?;
    img.save(out).map_err(|e| format!("{}: {e}", out.display()))
}

fn main() -> Result<(), String> {
    let args = parse_args();
    let composed = compose_spec(&args.spec)?;
    let (win_w, win_h) = (composed.width as f32 + 200.0, composed.height as f32 + 56.0);
    let title = composed.title.clone().unwrap_or_else(|| "Brightfield".to_string());

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_inner_size([win_w, win_h]),
        ..Default::default()
    };

    let mode = args.mode;
    let shot_out = args.shot_out.clone();
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            let rs = cc
                .wgpu_render_state
                .as_ref()
                .ok_or_else(|| "eframe did not start on the wgpu backend".to_string())?;
            let device = rs.device.clone();
            let queue = rs.queue.clone();
            let egui_renderer: Arc<egui::mutex::RwLock<egui_wgpu::Renderer>> = rs.renderer.clone();
            let vello = VelloRenderer::from_shared(device.clone(), queue.clone());
            let host = EguiCanvasHost::new(device, queue, vello, egui_renderer);
            let state = ShellState::new(composed, host, mode);
            Ok(Box::new(ShellApp {
                state,
                shot_out,
                shot_pending: false,
            }) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe run failed: {e}"))
}
