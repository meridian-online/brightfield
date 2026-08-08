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
//! `--version` prints the crate version and `--help` prints usage; both answer
//! on stdout and exit 0 *before* any window, layout read or spec boot happens.
//!
//! Usage: `brightfield-shell [SPEC.yaml] [--theme light|dark] [--shot-out PATH]
//!         [--shot-after N] [--force-sample N] [--flow vertical|horizontal]
//!         [--help] [--version]`

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use brightfield_protocol::layout::Flow;
use brightfield_render::vello_renderer::VelloRenderer;
use brightfield_shell::canvas::EguiCanvasHost;
use brightfield_shell::design::Mode;
use brightfield_shell::startup;
use brightfield_shell::window::{fit_window_to_display, DisplayFit, MeridianApp};
use brightfield_workbench::persist::SAVE_DEBOUNCE_MS;

struct Args {
    spec: Option<String>,
    mode: Mode,
    shot_out: PathBuf,
    flow: Flow,
    /// `Some(n)`: capture the window automatically after `n` frames, write it
    /// to `shot_out`, and close. The scriptable form of F12.
    shot_after: Option<u32>,
    /// `--force-sample N`: open the document drawing one row in N through the
    /// pushed-down sampling clause, with the notice in the plot's own ink.
    /// The window's half of the flag `brightfield-shot` carries, so the live
    /// surface and the captures are judged at the same rate.
    ///
    /// A plot with a band axis or a sequential ramp is REFUSED before the
    /// window opens — those domains are read off the rows that were drawn and
    /// the unsampled query's value set does not put them back, so the sampled
    /// render would place or colour a value differently from the complete one
    /// under a notice that mentions only the dropped rows. A colour channel is
    /// drawn, in the complete render's colours.
    force_sample: Option<brightfield_sql::ir::SampleRate>,
}

/// What the command line asked `main` to do, resolved before any window work.
///
/// `--version` and `--help` are answered as their own variants rather than as
/// fields on [`Args`] because they must run *nothing else*: no layout is read,
/// no spec is booted, and `eframe::run_native` is never reached. A query about
/// the binary should never open the binary's window — nor touch the developer's
/// real `workspace-layout.json`, which the window-open path reads on the way in.
enum Invocation {
    /// Open the live window with these settings.
    Run(Args),
    /// `--version`: print the crate version to stdout and exit 0. No window.
    Version,
    /// `--help`: print usage to stdout and exit 0. No window.
    Help,
    /// `--check-type-source`: bring up the bundled semantic type source and
    /// type one column with it, then exit. No window.
    CheckTypeSource,
}

/// The `--help` body. Mirrors the module-doc usage line and the flag grammar in
/// [`parse_args_from`]; keep the three in step when a flag is added or removed.
const HELP: &str = "\
brightfield — the Meridian live chart and Protocol window.

Usage: brightfield [SPEC.yaml] [OPTIONS]

Arguments:
  [SPEC.yaml]                     A chart spec or Protocol manifest to open.
                                  With none, opens on the bundled gallery.

Options:
  --theme <light|dark>            Colour mode (default: light).
  --flow <vertical|horizontal>    Pane flow for the opening view
                                  (default: vertical).
  --shot-out <PATH>               Where F12 and --shot-after write the PNG
                                  (default: render-proof/live-screenshot.png).
  --force-sample <N>              Draw one row in N (a power of two) through the
                                  pushed-down sampling clause, with the notice
                                  in the plot's own ink. Refuses a plot with a
                                  band axis or a sequential ramp.
  --shot-after <N>                Render N frames, capture the window to
                                  --shot-out, then exit. Exit 0 means the PNG
                                  landed; for unattended smoke tests.
  --check-type-source             Bring up the bundled semantic type source,
                                  type one column with it, and exit. Opens no
                                  window. Exit 0 typed a column, 1 a bundle is
                                  present and did not work, 2 there is none.
  -h, --help                      Print this help and exit.
  -V, --version                   Print the version and exit.

Press F12 in the window to capture a screenshot to --shot-out.";

fn parse_args() -> Result<Invocation, String> {
    parse_args_from(std::env::args().skip(1))
}

/// The flag grammar, split from `std::env::args` so a test can feed it.
///
/// `--version`/`-V` and `--help`/`-h` short-circuit the moment they are seen,
/// whatever else is on the line: asking the binary its version or usage is not
/// a request to open it, so it wins over any spec or flag typed alongside.
///
/// Otherwise, unknown flags are reported and ignored — this window would rather
/// open than argue. `--shot-after` is the one exception: a value that does not
/// parse is an error, because the flag only exists for scripts, and a script
/// whose countdown was silently dropped gets a window that never exits instead
/// of a failed check.
fn parse_args_from(mut it: impl Iterator<Item = String>) -> Result<Invocation, String> {
    let mut spec: Option<String> = None;
    let mut mode = Mode::Light;
    let mut shot_out = PathBuf::from("render-proof/live-screenshot.png");
    let mut flow = Flow::Vertical;
    let mut shot_after: Option<u32> = None;
    let mut force_sample: Option<brightfield_sql::ir::SampleRate> = None;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--version" | "-V" => return Ok(Invocation::Version),
            "--help" | "-h" => return Ok(Invocation::Help),
            "--check-type-source" => return Ok(Invocation::CheckTypeSource),
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
            "--force-sample" => {
                let v = it.next().ok_or("--force-sample needs a power of two")?;
                let raw: u32 = v
                    .parse()
                    .map_err(|_| format!("--force-sample needs a positive integer, not {v}"))?;
                force_sample = Some(
                    brightfield_sql::ir::SampleRate::from_modulus(raw).ok_or_else(|| {
                        format!(
                            "--force-sample {raw} is not a power of two. Power-of-two moduli \
                             nest, so halving the rate can only ADD points; rounding {raw} \
                             silently would keep that true while the stated rate was a lie."
                        )
                    })?,
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
    Ok(Invocation::Run(Args {
        spec,
        mode,
        shot_out,
        flow,
        shot_after,
        force_sample,
    }))
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
    /// The window this launch asked for because of what it loaded, held until
    /// a frame has checked it against the monitor the window landed on — then
    /// `None`, and the user's own resizes are their business.
    ///
    /// `None` from the start for the two launches the cap has no business
    /// touching: one that restored the user's saved geometry, and one that
    /// opened on nothing and took `WindowGeometry`'s default. See the call in
    /// `main` that sets it.
    fit: Option<(f32, f32)>,
}

/// Whether a frame reporting `outcome` is the last one that needs to ask.
///
/// One line of logic, lifted out of the frame loop because it is the whole of
/// the display cap's decision and inside `ui` it was reachable only through an
/// `eframe::Frame` — so nothing held it. Retiring on `MonitorUnknown` and never
/// arming the slot are, between them, exactly the two edits that put a window
/// back off the side of the screen, and a review found both with the entire
/// suite green. `window::fit_window_to_display` correctly *reports*
/// `MonitorUnknown`; whether the caller then retries or gives up is what
/// decides if the cap ever fires at all, and that belongs to this side.
const fn retires_the_fit(outcome: &DisplayFit) -> bool {
    match outcome {
        // Not mapped yet, so nothing was decided. Asking again costs one
        // comparison a frame until a monitor appears; giving up costs the cap.
        DisplayFit::MonitorUnknown => false,
        DisplayFit::Fits | DisplayFit::Shrunk(_) => true,
    }
}

/// Whether this launch's window size is the cap's business.
///
/// Only a content-derived size is: a geometry the user themselves dragged out
/// is theirs to have made too big, and the default a bootless launch takes is
/// small by construction. Extracted alongside [`retires_the_fit`] for the same
/// reason — the arming decision is half of the mechanism, and a `main` that
/// never arms is indistinguishable from one that has no cap.
const fn cap_applies(kept_geometry: bool, boot_is_empty: bool) -> bool {
    !kept_geometry && !boot_is_empty
}

impl eframe::App for BrightfieldApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // The size this window was created at was read outwards from the graph
        // and knows nothing about the screen it landed on. This is the first
        // frame that can say — `window::fit_window_to_display` carries the
        // whole argument for why it cannot be said sooner, and why eframe's own
        // creation-time clamp does not cover this case.
        if let Some(natural) = self.fit {
            let outcome = fit_window_to_display(&ctx, natural);
            if let DisplayFit::Shrunk(size) = outcome {
                eprintln!(
                    "window: this display shows {}x{} of the {}x{} the document asks for — \
                     the canvas scrolls the rest",
                    size.x, size.y, natural.0, natural.1
                );
            }
            if retires_the_fit(&outcome) {
                self.fit = None;
            }
        }

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

/// `--check-type-source`: prove the bundled semantic type source works, from
/// inside this binary, with no window.
///
/// This exists because the thing worth proving about a packaged Brightfield's
/// type source cannot be seen from outside it. The bundle's FILES can be read
/// off disk by `scripts/check-bundled-extension.sh`; what that cannot say is
/// whether the DuckDB this binary links will load that extension, whether the
/// model beside it is loadable, and whether the pair can actually put a label
/// on a column — from wherever the artefact happens to be unpacked, and with
/// whatever network the caller has denied it.
///
/// So `scripts/verify-airgapped.sh` runs THIS inside its jail. Everything the
/// run needs is in the binary and the bundle: the fixture is four inline rows,
/// the load is [`brightfield_engine::NetworkPolicy::Disabled`], and the verdict
/// is an exit code rather than a line of prose somebody has to grep for.
///
/// Exit codes are distinct because the three outcomes want different actions:
///
/// - `0` — a bundle was found, came up, and typed a column. Its name, the
///   directory, the label and the value-check result go to stdout.
/// - `1` — a bundle is present and did not work. The reason goes to stderr.
///   This is a packaging defect.
/// - `2` — there is no bundle beside this executable. Not a defect: a build
///   packaged without one is supported (see `scripts/package.sh`).
fn check_type_source() -> i32 {
    use brightfield_engine::semantic::{self, TypeSourceSpec};
    use brightfield_engine::{
        Engine, LoadOptions, NetworkPolicy, ProfileOutcome, SemanticType, ValueCheck,
    };
    use brightfield_spec::analysis::analyse_spec;
    use brightfield_spec::{parse_spec, Format};

    // Deliberately values whose meaning a DuckDB type cannot carry: the point
    // of the check is that something ANSWERED, not that VARCHAR is VARCHAR.
    const FIXTURE: &str = r#"
data:
  probe:
    - { email: "alice@example.com" }
    - { email: "bob@example.org" }
    - { email: "carol@example.net" }
    - { email: "dan@corp.co.uk" }
plot:
  - mark: dot
    data: { from: probe }
"#;

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("check-type-source: cannot locate this executable: {e}");
            return 1;
        }
    };
    let Some(bundle) = semantic::bundle_beside(&exe) else {
        println!("check-type-source: no type source bundled beside {}", exe.display());
        return 2;
    };

    let Ok(parsed) = parse_spec(FIXTURE, Format::Yaml) else {
        eprintln!("check-type-source: the built-in fixture does not parse");
        return 1;
    };
    let Ok(analysis) = analyse_spec(&parsed.spec) else {
        eprintln!("check-type-source: the built-in fixture does not analyse");
        return 1;
    };
    let options = LoadOptions {
        network: NetworkPolicy::Disabled,
        extension_directory: None,
        type_source: Some(TypeSourceSpec::Bundle(bundle.clone())),
    };
    let session = match Engine::new().load_spec_with(parsed.spec, analysis, None, &options) {
        Ok(load) => load.session,
        Err(e) => {
            eprintln!("check-type-source: the fixture would not load: {e}");
            return 1;
        }
    };
    if let Some(reason) = session.type_source_error() {
        eprintln!("check-type-source: {bundle:?} did not come up: {reason}");
        return 1;
    }
    let Some(name) = session.type_source_name() else {
        eprintln!("check-type-source: no type source and no reason given — this is a bug");
        return 1;
    };
    println!("check-type-source: {name} at {}", bundle.display());

    let Some(profile) = session
        .profile_sources()
        .into_iter()
        .find(|p| p.name == "probe")
    else {
        eprintln!("check-type-source: the fixture produced no source to profile");
        return 1;
    };
    let ProfileOutcome::Profiled { columns, .. } = profile.outcome else {
        eprintln!("check-type-source: the fixture's source could not be profiled");
        return 1;
    };
    let Some(column) = columns.into_iter().find(|c| c.name == "email") else {
        eprintln!("check-type-source: the fixture's column vanished");
        return 1;
    };

    // A label is required, and so is a check behind it. `Unlabelled` is exactly
    // what a bundle whose model did not load produces, and it is the outcome
    // this whole check exists to refuse.
    match column.semantic {
        SemanticType::Labelled { label, check, .. } => {
            match check {
                ValueCheck::Checked { checked, failed } => println!(
                    "check-type-source: {} typed as {label}, {}/{} values satisfy it",
                    column.type_name,
                    checked - failed,
                    checked
                ),
                other => {
                    eprintln!(
                        "check-type-source: typed as {label}, but nothing checked the values \
                         ({other:?}) — the schema catalogue does not describe what the model \
                         emits"
                    );
                    return 1;
                }
            }
            0
        }
        other => {
            eprintln!("check-type-source: the bundle put no usable label on the column: {other:?}");
            1
        }
    }
}

fn main() -> Result<(), String> {
    // Answered before any window work: print to stdout, exit 0, open nothing.
    // Nothing below this match — the layout read, the spec boot, the viewport,
    // `run_native` — is reached for a `--version` or `--help` invocation.
    let args = match parse_args()? {
        Invocation::Version => {
            println!("brightfield {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Invocation::Help => {
            println!("{HELP}");
            return Ok(());
        }
        Invocation::CheckTypeSource => std::process::exit(check_type_source()),
        Invocation::Run(args) => args,
    };

    // The layout is read *before* anything else, because two things downstream
    // depend on it: the window's size, which has to be settled before the
    // viewport is built, and what to open when the command line named nothing.
    // `startup::boot_layout` publishes both views' item vocabularies first —
    // see its docs for why the read is invalid without that.
    let path = startup::layout_path();
    let (mut layout, outcome) = startup::boot_layout(path.as_deref());
    eprintln!("layout: {}", outcome.reason());

    let boot = startup::opening_boot(
        args.spec.as_deref(),
        layout.opened.as_deref(),
        args.flow,
        args.force_sample,
    )?;

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
    //
    // A budget derived from content knows what the graph wants and nothing
    // about the screen, so it is also the only case the display cap applies to:
    // a window the *user* sized is theirs to have made too big, and a default
    // is small by construction. `fit` carries that size to the first frame,
    // which is the earliest point a real monitor can be read — see
    // `window::fit_window_to_display` for why it cannot be read before then.
    let mut fit = None;
    if cap_applies(startup::kept_window_geometry(outcome), boot.is_empty()) {
        let (w, h) = boot.window_size(view);
        layout.window.size = [w, h];
        fit = Some((w, h));
    }
    let mut viewport = egui::ViewportBuilder::default().with_inner_size(layout.window.size);
    if let Some([x, y]) = layout.window.position {
        viewport = viewport.with_position([x, y]);
    }
    let title = boot.title(view);

    // The live window rides eframe's wgpu device — the Vello canvases are built
    // with `VelloRenderer::from_shared` on it — so the ceiling this window draws
    // at is set HERE, not by anything brightfield's own device code does. Left
    // at `Default`, `egui-wgpu` requests `Limits::default()`: the WebGL-friendly
    // floor, whose 128 MiB storage-buffer binding is what an encoded
    // row-per-mark scene runs out of first. Fixing only the in-repo devices
    // would have left a headless capture reporting one ceiling and the window —
    // the one surface an optical sign-off actually judges — dying at another.
    //
    // The adapter's limits are passed through UNMODIFIED, and no floor is
    // raised over them. `required_limits` is a demand, not a wish: wgpu refuses
    // device creation outright when a required limit exceeds what the adapter
    // reports. An earlier version asked for
    // `max_texture_dimension_2d.max(8192)` to "keep egui's 8192 floor" — on any
    // adapter reporting less than 8192 that would have turned a smaller texture
    // ceiling into no window at all, which is the opposite of what the comment
    // beside it claimed. Whatever the adapter has is the most that can be asked
    // for; a surface that needs more than that needs a different adapter.
    //
    // This REPLACES egui-wgpu's own descriptor rather than amending it, so what
    // is given up is worth naming. Its version (setup.rs, 0.35) sets a label,
    // picks downlevel-webgl2 limits on a GL backend and desktop defaults
    // elsewhere, then forces `max_texture_dimension_2d: 8192` — and leaves
    // `required_features` and `memory_hints` at `Default`, exactly as this does.
    // So the only substantive difference is that 8192 floor, and dropping it is
    // strictly safer: `required_limits` is a demand, so egui's version fails
    // device creation outright on an adapter reporting less, where asking the
    // adapter degrades instead. The GL branch is subsumed too, since the
    // adapter's own limits are never below what that backend accepts.
    let mut wgpu_setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    wgpu_setup.device_descriptor =
        std::sync::Arc::new(
            |adapter: &eframe::wgpu::Adapter| eframe::wgpu::DeviceDescriptor {
                label: Some("brightfield-window"),
                required_limits: brightfield_render::vello_renderer::device_limits(adapter),
                ..Default::default()
            },
        );
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport,
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup: wgpu_setup.into(),
            ..Default::default()
        },
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
                // `allowing_dialogs` here and nowhere else: this is the one
                // construction with an operating-system window behind it and a
                // person in front of it, so it is the one that may raise a file
                // dialog. The capture tiers build the same app without it.
                app: MeridianApp::with_layout(boot, layout, chart_host, protocol_host, mode)
                    .allowing_dialogs(),
                shot: ShotLatch::new(shot_out, shot_after, saved),
                layout_path: path,
                fit,
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

    fn parse(list: &[&str]) -> Result<Invocation, String> {
        parse_args_from(list.iter().map(|s| (*s).to_string()))
    }

    /// The frame that reports no monitor must not be the last one that asks.
    ///
    /// This is the arm a review disarmed with the whole suite green: a window
    /// is not mapped on its first frames, so retiring here retires the cap
    /// before it has ever seen a screen, and the window stays at the size the
    /// graph asked for — which is the exact defect the cap exists to prevent,
    /// restored invisibly.
    #[test]
    fn an_unmapped_frame_keeps_the_cap_waiting() {
        assert!(
            !retires_the_fit(&DisplayFit::MonitorUnknown),
            "a frame with no monitor retired the display cap — it would never \
             fire, and a window larger than the screen would open with every \
             test still green"
        );
    }

    /// Both decided outcomes are final: there is no second monitor to wait for,
    /// and re-asking every frame would re-send an `InnerSize` the user may have
    /// since dragged away from.
    #[test]
    fn a_decided_frame_retires_the_cap() {
        assert!(
            retires_the_fit(&DisplayFit::Fits),
            "a fitting window asked again"
        );
        assert!(
            retires_the_fit(&DisplayFit::Shrunk(egui::Vec2::new(800.0, 600.0))),
            "a shrunk window asked again, so it would fight a later resize"
        );
    }

    /// The other half of the mechanism. A `main` that never arms the slot is
    /// indistinguishable from one with no cap at all, and the two together are
    /// what the review removed to restore the original defect.
    #[test]
    fn only_a_content_derived_window_is_capped() {
        assert!(
            cap_applies(false, false),
            "a content-sized window was not armed, so the cap never runs"
        );
        assert!(
            !cap_applies(true, false),
            "a geometry the user dragged out was capped — theirs to have made too big"
        );
        assert!(
            !cap_applies(false, true),
            "a bootless launch was capped, and its default is small by construction"
        );
    }

    /// The `Run` arguments, or a panic — for the cases that are meant to open a
    /// window rather than short-circuit on `--version`/`--help`.
    fn run_args(list: &[&str]) -> Args {
        match parse(list) {
            Ok(Invocation::Run(a)) => a,
            Ok(Invocation::Version) => panic!("expected Run, got Version"),
            Ok(Invocation::Help) => panic!("expected Run, got Help"),
            Ok(Invocation::CheckTypeSource) => panic!("expected Run, got CheckTypeSource"),
            Err(e) => panic!("expected Run, got Err: {e}"),
        }
    }

    #[test]
    fn bare_invocation_stays_interactive() {
        let a = run_args(&[]);
        assert!(a.spec.is_none());
        assert!(a.shot_after.is_none());
    }

    #[test]
    fn shot_after_parses_a_frame_count() {
        let a = run_args(&["examples/bars.yaml", "--shot-after", "30"]);
        assert_eq!(a.spec.as_deref(), Some("examples/bars.yaml"));
        assert_eq!(a.shot_after, Some(30));
    }

    #[test]
    fn shot_after_rejects_a_missing_or_bad_count() {
        assert!(parse(&["--shot-after"]).is_err());
        assert!(parse(&["--shot-after", "soon"]).is_err());
    }

    #[test]
    fn unknown_flags_are_still_ignored() {
        let a = run_args(&["--wobble", "--theme", "dark"]);
        assert!(matches!(a.mode, Mode::Dark));
    }

    #[test]
    fn version_flag_short_circuits() {
        assert!(matches!(parse(&["--version"]), Ok(Invocation::Version)));
        // Short-circuits like --version/--help: nothing else on the line can
        // turn it back into a window, because it exists to be run unattended.
        assert!(matches!(
            parse(&["--check-type-source"]),
            Ok(Invocation::CheckTypeSource)
        ));
        assert!(matches!(
            parse(&["spec.yaml", "--check-type-source", "--shot-after", "45"]),
            Ok(Invocation::CheckTypeSource)
        ));
        assert!(matches!(parse(&["-V"]), Ok(Invocation::Version)));
        // Wins over anything else on the line, and opens no window.
        assert!(matches!(
            parse(&["examples/bars.yaml", "--version"]),
            Ok(Invocation::Version)
        ));
    }

    #[test]
    fn help_flag_short_circuits() {
        assert!(matches!(parse(&["--help"]), Ok(Invocation::Help)));
        assert!(matches!(parse(&["-h"]), Ok(Invocation::Help)));
        assert!(matches!(
            parse(&["--theme", "dark", "--help"]),
            Ok(Invocation::Help)
        ));
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
