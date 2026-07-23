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
//! `--shot-after N` is the same screenshot, fired by a countdown instead of a
//! finger: render N frames, capture the live window to `--shot-out`, close.
//! It exists so a *packaged* binary can prove "starts, renders, opens the
//! spec" unattended — the air-gapped smoke test runs exactly this under a
//! network-denying sandbox and trusts the exit code, which is only 0 once the
//! PNG is actually on disk.
//!
//! Usage: `brightfield-shell [SPEC.yaml] [--theme light|dark] [--shot-out PATH]
//!         [--shot-after N] [--flow vertical|horizontal]`

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// `Some(n)`: capture the window automatically after `n` frames, write it
    /// to `shot_out`, and close. The scriptable form of F12.
    shot_after: Option<u32>,
}

fn parse_args() -> Result<Args, String> {
    parse_args_from(std::env::args().skip(1))
}

/// The flag grammar, split from `std::env::args` so a test can feed it.
///
/// Unknown flags are reported and ignored — this window would rather open than
/// argue. `--shot-after` is the one exception: a value that does not parse is
/// an error, because the flag only exists for scripts, and a script whose
/// countdown was silently dropped gets a window that never exits instead of a
/// failed check.
fn parse_args_from(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut spec: Option<String> = None;
    let mut mode = Mode::Light;
    let mut shot_out = PathBuf::from("render-proof/live-screenshot.png");
    let mut flow = Flow::Vertical;
    let mut shot_after: Option<u32> = None;
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
            "--shot-after" => {
                let v = it.next().ok_or("--shot-after needs a frame count")?;
                shot_after = Some(
                    v.parse()
                        .map_err(|_| format!("--shot-after needs a frame count, not {v}"))?,
                );
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
    Ok(Args {
        spec,
        mode,
        shot_out,
        flow,
        shot_after,
    })
}

/// The F12 → live-screenshot latch (request in `ui`, capture next frame).
///
/// With `--shot-after N` the same latch also runs a countdown: N frames of
/// real rendering, then the identical request→capture→save path, then a
/// `ViewportCommand::Close`. The countdown holds `request_repaint` high while
/// it runs, because eframe paints on input and an untouched window would
/// otherwise stop producing the very frames being counted.
struct ShotLatch {
    shot_out: PathBuf,
    pending: bool,
    /// Frames left before the automatic capture fires; `None` once fired, or
    /// when the window is interactive-only.
    countdown: Option<u32>,
    /// Whether this latch was armed by `--shot-after` — i.e. whether a save
    /// should be followed by a close, success or not. A failed save still
    /// closes: the countdown run is a check, and a check that hangs on
    /// failure is not one.
    auto: bool,
    /// Raised only when the automatic capture's PNG is actually on disk; the
    /// exit gate `main` reads after the event loop returns.
    saved: Arc<AtomicBool>,
}

impl ShotLatch {
    fn new(shot_out: PathBuf, shot_after: Option<u32>, saved: Arc<AtomicBool>) -> Self {
        Self {
            shot_out,
            pending: false,
            countdown: shot_after,
            auto: shot_after.is_some(),
            saved,
        }
    }

    /// Request on F12 (or countdown expiry), then save the image handed back
    /// on a later frame.
    fn tick(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.key_pressed(egui::Key::F12)) {
            self.request(ctx);
        }
        match self.countdown {
            Some(0) => {
                self.countdown = None;
                self.request(ctx);
            }
            Some(n) => self.countdown = Some(n - 1),
            None => {}
        }
        if self.auto && (self.countdown.is_some() || self.pending) {
            ctx.request_repaint();
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
                match save_color_image(&image, &self.shot_out) {
                    Err(e) => eprintln!("screenshot save failed: {e}"),
                    Ok(()) => {
                        eprintln!("live screenshot written: {}", self.shot_out.display());
                        if self.auto {
                            self.saved.store(true, Ordering::SeqCst);
                        }
                    }
                }
                if self.auto {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn request(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        self.pending = true;
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
    let args = parse_args()?;

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
    let shot_after = args.shot_after;
    let shot_saved = Arc::new(AtomicBool::new(false));
    let saved = shot_saved.clone();
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            let chart_host = host_from_frame(cc)?;
            let protocol_host = host_from_frame(cc)?;
            Ok(Box::new(BrightfieldApp {
                app: MeridianApp::with_layout(boot, layout, chart_host, protocol_host, mode),
                shot: ShotLatch::new(shot_out, shot_after, saved),
                layout_path: path,
            }) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe run failed: {e}"))?;

    // The countdown run's contract: exit 0 *means* the PNG landed. The window
    // closing is not that — a failed save also closes (see `ShotLatch`), and
    // an exit code that can't tell the two apart is no use to the script that
    // asked.
    if args.shot_after.is_some() && !shot_saved.load(Ordering::SeqCst) {
        return Err(format!(
            "--shot-after capture never landed at {}",
            args.shot_out.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, String> {
        parse_args_from(list.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn bare_invocation_stays_interactive() {
        let a = args(&[]).unwrap();
        assert!(a.spec.is_none());
        assert!(a.shot_after.is_none());
    }

    #[test]
    fn shot_after_parses_a_frame_count() {
        let a = args(&["examples/bars.yaml", "--shot-after", "30"]).unwrap();
        assert_eq!(a.spec.as_deref(), Some("examples/bars.yaml"));
        assert_eq!(a.shot_after, Some(30));
    }

    #[test]
    fn shot_after_rejects_a_missing_or_bad_count() {
        assert!(args(&["--shot-after"]).is_err());
        assert!(args(&["--shot-after", "soon"]).is_err());
    }

    #[test]
    fn unknown_flags_are_still_ignored() {
        let a = args(&["--wobble", "--theme", "dark"]).unwrap();
        assert!(matches!(a.mode, Mode::Dark));
    }

    /// The countdown fires the request exactly once, holds repaint high while
    /// it runs, and goes quiet after — checked against a real `egui::Context`
    /// so the frame arithmetic is exercised, not restated.
    #[test]
    fn countdown_fires_once_then_goes_quiet() {
        let ctx = egui::Context::default();
        let saved = Arc::new(AtomicBool::new(false));
        let mut latch = ShotLatch::new(PathBuf::from("unused.png"), Some(2), saved.clone());

        // Ticks run inside a frame so `ctx.input` sees a real (empty) state.
        for _ in 0..2 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ctx| latch.tick(ctx));
            assert!(!latch.pending, "countdown still running");
        }
        let _ = ctx.run_ui(egui::RawInput::default(), |ctx| latch.tick(ctx));
        assert!(latch.pending, "expiry requests the capture");
        assert!(latch.countdown.is_none(), "the countdown does not re-arm");
        assert!(!saved.load(Ordering::SeqCst), "nothing saved yet");

        // With no Screenshot event ever delivered, further ticks keep the
        // request pending rather than inventing a second one.
        let _ = ctx.run_ui(egui::RawInput::default(), |ctx| latch.tick(ctx));
        assert!(latch.pending);
    }

    /// An interactive latch (no `--shot-after`) never counts, never requests.
    #[test]
    fn interactive_latch_is_inert_without_f12() {
        let ctx = egui::Context::default();
        let saved = Arc::new(AtomicBool::new(false));
        let mut latch = ShotLatch::new(PathBuf::from("unused.png"), None, saved);
        for _ in 0..5 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ctx| latch.tick(ctx));
        }
        assert!(!latch.pending);
        assert!(latch.countdown.is_none());
    }
}
