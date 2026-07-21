//! `brightfield-shell` — the live egui/eframe window (Tier-3 of the loop).
//!
//! Opens the real docked shell over a spec, rendering the Vello canvas on
//! eframe's shared wgpu device. Auto-routes a Mosaic dashboard to the chart
//! shell and a Protocol manifest (with `BRIGHTFIELD_PROTOCOL_OFFLINE=1`) to the
//! egui Protocol panel (dock + DAG + outline + inspector + steps). Press F12 to
//! fire a `ViewportCommand::Screenshot` and write a PNG of the live window for
//! on-device sign-off.
//!
//! Usage: `brightfield-shell [SPEC.yaml] [--theme light|dark] [--shot-out PATH]
//!         [--flow vertical|horizontal]`

use std::path::PathBuf;
use std::sync::Arc;

use brightfield_protocol::layout::Flow;
use brightfield_render::vello_renderer::VelloRenderer;
use brightfield_shell::app::{draw_shell, window_size_for, ShellState};
use brightfield_shell::canvas::EguiCanvasHost;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::compose_spec;
use brightfield_shell::protocol::{load_protocol_offline, ProtocolShell};

struct Args {
    spec: String,
    mode: Mode,
    shot_out: PathBuf,
    flow: Flow,
}

fn parse_args() -> Args {
    let mut spec = "examples/dashboard.yaml".to_string();
    let mut mode = Mode::Light;
    let mut shot_out = PathBuf::from("render-proof/live-screenshot.png");
    let mut flow = Flow::Vertical;
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
            "--flow" => {
                if let Some(v) = it.next() {
                    flow = if v == "horizontal" {
                        Flow::Horizontal
                    } else {
                        Flow::Vertical
                    };
                }
            }
            other if !other.starts_with("--") => spec = other.to_string(),
            other => eprintln!("ignoring unknown flag {other}"),
        }
    }
    Args {
        spec,
        mode,
        shot_out,
        flow,
    }
}

/// The shared F12 → live-screenshot latch (request in `ui`, capture next frame).
struct ShotLatch {
    shot_out: PathBuf,
    pending: bool,
}

impl ShotLatch {
    fn new(shot_out: PathBuf) -> Self {
        Self {
            shot_out,
            pending: false,
        }
    }

    /// Request on F12, then save the image handed back on a later frame.
    fn tick(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.key_pressed(egui::Key::F12)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.pending = true;
        }
        if self.pending {
            let image = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = image {
                self.pending = false;
                if let Err(e) = save_color_image(&image, &self.shot_out) {
                    eprintln!("screenshot save failed: {e}");
                } else {
                    eprintln!("live screenshot written: {}", self.shot_out.display());
                }
            }
        }
    }
}

struct ShellApp {
    state: ShellState,
    shot: ShotLatch,
}

impl eframe::App for ShellApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        draw_shell(ui, &mut self.state);
        self.shot.tick(&ctx);
    }
}

struct ProtocolApp {
    shell: ProtocolShell,
    shot: ShotLatch,
}

impl eframe::App for ProtocolApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.shell.draw(ui);
        self.shot.tick(&ctx);
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

/// eframe's shared wgpu device/queue/renderer → an [`EguiCanvasHost`].
fn host_from_frame(cc: &eframe::CreationContext<'_>) -> Result<EguiCanvasHost, String> {
    let rs = cc
        .wgpu_render_state
        .as_ref()
        .ok_or_else(|| "eframe did not start on the wgpu backend".to_string())?;
    let device = rs.device.clone();
    let queue = rs.queue.clone();
    let egui_renderer: Arc<egui::mutex::RwLock<egui_wgpu::Renderer>> = rs.renderer.clone();
    let vello = VelloRenderer::from_shared(device.clone(), queue.clone());
    Ok(EguiCanvasHost::new(device, queue, vello, egui_renderer))
}

fn main() -> Result<(), String> {
    let args = parse_args();
    let text = std::fs::read_to_string(&args.spec).unwrap_or_default();

    if brightfield_protocol::is_protocol_manifest(&text) {
        if std::env::var("BRIGHTFIELD_PROTOCOL_OFFLINE").is_err() {
            return Err(format!(
                "{} is a Protocol manifest; set BRIGHTFIELD_PROTOCOL_OFFLINE=1 to render it offline",
                args.spec
            ));
        }
        return run_protocol_window(args);
    }

    run_mosaic_window(args)
}

fn run_mosaic_window(args: Args) -> Result<(), String> {
    let composed = compose_spec(&args.spec)?;
    // The same function `ShellState::window_size` reads. This line used to be
    // `+ 200.0 / + 56.0`, against a `ShellState::window_size` that said 214/60
    // and a side panel declared at 180 — three numbers for one layout, already
    // drifted.
    let (win_w, win_h) = window_size_for(&composed);
    let title = composed
        .title
        .clone()
        .unwrap_or_else(|| "Brightfield".to_string());

    let options = native_options(win_w, win_h);
    let mode = args.mode;
    let shot_out = args.shot_out.clone();
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            let host = host_from_frame(cc)?;
            let state = ShellState::new(composed, host, mode);
            Ok(Box::new(ShellApp {
                state,
                shot: ShotLatch::new(shot_out),
            }) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe run failed: {e}"))
}

fn run_protocol_window(args: Args) -> Result<(), String> {
    let inputs = load_protocol_offline(&args.spec)?;
    eprintln!(
        "protocol {} ({} collapsed / {} full nodes, {} steps, {:?} flow)",
        inputs.protocol,
        inputs.graph_collapsed.nodes.len(),
        inputs.graph_full.nodes.len(),
        inputs.sheet_rows.len(),
        args.flow,
    );
    let title = format!("Protocol · {}", inputs.protocol);
    let options = native_options(1440.0, 900.0);
    let mode = args.mode;
    let flow = args.flow;
    let shot_out = args.shot_out.clone();
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            let host = host_from_frame(cc)?;
            let shell = ProtocolShell::new(inputs, host, mode, flow);
            Ok(Box::new(ProtocolApp {
                shell,
                shot: ShotLatch::new(shot_out),
            }) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe run failed: {e}"))
}

fn native_options(win_w: f32, win_h: f32) -> eframe::NativeOptions {
    eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_inner_size([win_w, win_h]),
        ..Default::default()
    }
}
