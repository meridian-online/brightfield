//! `brightfield-shell` — the live egui/eframe window (Tier-3 of the loop).
//!
//! Opens the real docked window over a spec, rendering the Vello canvases on
//! eframe's shared wgpu device. The window holds **both** views — the chart
//! workbench and the Protocol panel — with a switcher in the top bar; the spec
//! decides which of them opens first, and a Protocol manifest still needs
//! `BRIGHTFIELD_PROTOCOL_OFFLINE=1` to be rendered without a run. Press F12 to
//! fire a `ViewportCommand::Screenshot` and write a PNG of the live window for
//! on-device sign-off.
//!
//! Usage: `brightfield-shell [SPEC.yaml] [--theme light|dark] [--shot-out PATH]
//!         [--flow vertical|horizontal]`

use std::path::PathBuf;
use std::sync::Arc;

use brightfield_protocol::layout::Flow;
use brightfield_render::vello_renderer::VelloRenderer;
use brightfield_shell::canvas::EguiCanvasHost;
use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp};

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

/// The F12 → live-screenshot latch (request in `ui`, capture next frame).
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

/// The **one** `eframe::App` in the product.
///
/// It used to be two — one over the chart shell and one over the protocol
/// shell, chosen by sniffing the spec, each opening its own window. Two
/// `eframe::App`s is precisely what made it impossible for the chart and the
/// DAG to share a window, whatever the workbench underneath them could express.
/// What is left here is the window's own business and nothing else: the
/// screenshot latch, and handing the frame to [`MeridianApp`].
struct BrightfieldApp {
    app: MeridianApp,
    shot: ShotLatch,
}

impl eframe::App for BrightfieldApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.app.draw(ui);
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
///
/// Called once per document. Every handle it takes is an `Arc` clone, so a
/// second host costs one `VelloRenderer` and nothing else — which is what lets
/// the two views go on owning their own canvases inside one window.
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
    let boot = Boot::open(&args.spec, args.flow, None)?;
    eprintln!("{} from {}", boot.describe(), args.spec);

    // Both read before the window exists, and both from the boot rather than
    // open-coded here. `main.rs` having its own copy of the chart window's
    // budget — 200 logical points, against a shell that said 214 and a panel
    // declared at 180 — is the drift this arrangement ends.
    let (win_w, win_h) = boot.window_size();
    let title = boot.title();

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
            let chart_host = host_from_frame(cc)?;
            let protocol_host = host_from_frame(cc)?;
            Ok(Box::new(BrightfieldApp {
                app: MeridianApp::new(boot, chart_host, protocol_host, mode),
                shot: ShotLatch::new(shot_out),
            }) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe run failed: {e}"))
}
