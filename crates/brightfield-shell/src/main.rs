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
//! Naming a spec is optional. With none, the window opens on nothing — which
//! is a real surface rather than a blank one: every pane of both views answers
//! an empty document with an empty state, and the two panes that something
//! shipped in the binary can fill offer it as a button. What stood here
//! instead was a hardcoded `examples/dashboard.yaml`, which opened a dashboard
//! nobody asked for when run from the repo root and exited with a read error
//! from anywhere else.
//!
//! Usage: `brightfield-shell [SPEC.yaml] [--theme light|dark] [--shot-out PATH]
//!         [--flow vertical|horizontal]`

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use brightfield_protocol::layout::Flow;
use brightfield_render::vello_renderer::VelloRenderer;
use brightfield_shell::canvas::EguiCanvasHost;
use brightfield_shell::design::Mode;
use brightfield_shell::startup;
use brightfield_shell::window::MeridianApp;
use brightfield_workbench::persist::SAVE_DEBOUNCE_MS;

struct Args {
    spec: Option<String>,
    mode: Mode,
    shot_out: PathBuf,
    flow: Flow,
}

fn parse_args() -> Args {
    let mut spec: Option<String> = None;
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
            other if !other.starts_with("--") => spec = Some(other.to_string()),
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
    /// Where the layout is written, or `None` when this machine has nowhere to
    /// put one.
    ///
    /// Held by the host rather than by [`MeridianApp`] because this is the
    /// only place `startup::layout_path()` is called: `MeridianApp` writes
    /// only to a path it is handed, so nothing the headless tiers construct
    /// can name the developer's real `workspace-layout.json` — the file the
    /// PNG capture tier would otherwise overwrite on every `cargo test`. That
    /// is a property of who calls `layout_path`, checkable with
    /// `git grep -n 'layout_path()' -- crates/`, and **not** a claim that a
    /// headless window cannot write: `poll_layout` and `flush_layout` are
    /// `pub`, take any `&Path`, and `layout_wiring.rs` uses them to write real
    /// files into a scratch directory.
    layout_path: Option<PathBuf>,
}

impl eframe::App for BrightfieldApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.app.draw(ui);
        self.shot.tick(&ctx);

        // After the draw, so this frame's drags are already in the tree.
        self.app.observe_window(&ctx);
        if let Some(path) = &self.layout_path {
            let now_ms = (ctx.input(|i| i.time) * 1000.0) as u64;
            if let Some(Err(e)) = self.app.poll_layout(now_ms, path) {
                eprintln!("layout save failed: {e}");
            }
            // eframe paints on input, not continuously. Without this, a user
            // who drags a splitter and then leaves the window alone generates
            // no more frames, the countdown never reaches its deadline, and
            // the change survives only as far as `on_exit` — which a crash
            // does not reach, and bounding that loss is what the debounce is
            // for.
            if self.app.layout_armed() {
                ctx.request_repaint_after(Duration::from_millis(SAVE_DEBOUNCE_MS));
            }
        }
    }

    /// Write the layout on the way out, debounce or not.
    ///
    /// `on_exit` and not `App::save`: eframe's `persistence` feature is off in
    /// this build, which makes `save` a no-op the integration never calls —
    /// wiring the flush there would look right, compile, and never run. This
    /// signature is the `#[cfg(not(feature = "glow"))]` arm; enabling glow
    /// would grow a `gl` parameter and break this loudly, which is the right
    /// direction for it to break in.
    fn on_exit(&mut self) {
        if let Some(path) = &self.layout_path {
            if let Some(Err(e)) = self.app.flush_layout(path) {
                eprintln!("layout flush failed: {e}");
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

    // The layout is read *before* anything else, because two things downstream
    // depend on it: the window's size, which has to be settled before the
    // viewport is built, and what to open when the command line named nothing.
    // `startup::boot_layout` publishes both views' item vocabularies first —
    // see its docs for why the read is invalid without that.
    let path = startup::layout_path();
    let (mut layout, outcome) = startup::boot_layout(path.as_deref());
    eprintln!("layout: {}", outcome.reason());

    let boot = startup::opening_boot(args.spec.as_deref(), layout.opened.as_deref(), args.flow)?;

    // Which view will actually be drawn, resolved once and asked three times.
    // A boot that named one wins — `MeridianApp::assemble` sets it active — and
    // a boot that named none defers to the layout, which is exactly what
    // `assemble` leaves standing. So this is the view the first frame draws,
    // and it is the only honest subject for the size, the title and the
    // summary line below. Letting a `None` default to the charts view instead
    // titled a restored crosswalk "Brightfield" and logged "composed 0x0
    // dashboard" over a 34-node graph — and since `run_native` takes the title
    // once and only the front door's own click ever sends a
    // `ViewportCommand::Title`, that name stayed wrong for the session.
    let view = boot.view_or(layout.workspace.active());
    eprintln!("{}", boot.describe(view));

    // A window size the user chose is authoritative. Otherwise the boot's own
    // budget stands — unless it has no document to derive one from, in which
    // case `WindowGeometry`'s default is already in `layout.window`.
    if !startup::kept_window_geometry(outcome) && !boot.is_empty() {
        let (w, h) = boot.window_size(view);
        layout.window.size = [w, h];
    }
    let mut viewport = egui::ViewportBuilder::default().with_inner_size(layout.window.size);
    if let Some([x, y]) = layout.window.position {
        viewport = viewport.with_position([x, y]);
    }
    let title = boot.title(view);

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport,
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
                app: MeridianApp::with_layout(boot, layout, chart_host, protocol_host, mode),
                shot: ShotLatch::new(shot_out),
                layout_path: path,
            }) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe run failed: {e}"))
}
