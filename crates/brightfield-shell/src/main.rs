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

/// Set by the outer phase of `--check-type-source` on the sealed child, so the
/// child knows it is the child. Not trusted on its own — the child verifies
/// the environment it actually got.
const SEALED_MARKER: &str = "BRIGHTFIELD_TYPE_SOURCE_SEALED";

/// What the outer phase puts back after clearing the environment.
///
/// Not the whole of what the sealed phase may see: [`SEALED_ENV_SELF_SET`] is
/// permitted alongside it, and the two together are what the stowaway check
/// admits.
///
/// An ALLOWLIST, not a list of variables to strip, and that choice is the
/// point. A denylist is a list of the redirections somebody has thought of: it
/// was wrong about `FINETYPE_INJECT_LABEL` before anyone looked, and it will be
/// wrong again the next time FineType, `hf_hub` or DuckDB reads a new one. This
/// is right by construction — a variable nobody has heard of yet is absent
/// because everything is absent.
///
/// Each entry is here because the run needs it, and each is SET by the outer
/// phase rather than inherited, so none of them carries a value the caller
/// chose:
///
/// - `PATH` — a fixed system path. Nothing is resolved through it (the child is
///   exec'd by absolute path); it is set because a process with no `PATH` at
///   all is a strange thing to hand a library.
/// - `HOME`, `TMPDIR` — inside a fresh directory the outer phase creates and
///   deletes. Any cache a dependency reaches for lands there, empty, and is
///   thrown away. This is what closes `hf_hub`'s fallback, which resolves
///   `$HOME/.cache/huggingface` when `HF_HOME` is unset — and `HF_HOME` is
///   unset because it is not on this list.
/// - `LC_ALL`, `LANG` — pinned to `C` so a locale cannot move a comparison.
const SEALED_ENV: &[&str] = &["PATH", "HOME", "TMPDIR", "LC_ALL", "LANG", SEALED_MARKER];

/// Variables the child writes into ITSELF after `execve`, which the clear
/// cannot remove and the outer phase does not set.
///
/// One entry, and it is here on a measurement rather than a hunch.
/// `__CF_USER_TEXT_ENCODING` appears in this binary's sealed child because it
/// links CoreFoundation, which records the account's text encoding in its own
/// environment at initialisation. A bare Rust binary spawning a child with
/// `env_clear` shows an environment of exactly what it set — including when the
/// grandparent exported `__CF_USER_TEXT_ENCODING=HOSTILE`, which does not
/// survive the clear. So on a child this binary re-exec'd, the clear removed
/// whatever the caller exported and this value arrived afterwards — which is
/// why the allowlist has to name it rather than exclude it.
///
/// Anything ELSE a future dependency injects will fail this check loudly rather
/// than pass quietly, which is the direction to fail in.
const SEALED_ENV_SELF_SET: &[&str] = &["__CF_USER_TEXT_ENCODING"];

/// `--check-type-source`: run the bundled semantic type source and report what
/// happened, from inside this binary, with no window.
///
/// This exists because the thing worth checking about a packaged Brightfield's
/// type source cannot be seen from outside it. The bundle's FILES can be read
/// off disk by `scripts/check-bundled-extension.sh`; what that cannot say is
/// whether the DuckDB this binary links will load that extension, whether the
/// model beside it is loadable, and whether the pair can actually put a label
/// on a column.
///
/// # What a zero exit does and does not establish
///
/// Read this before quoting the result, because the useful reading is narrower
/// than the obvious one. Exit 0 says:
///
/// - a bundle is present beside this executable and, if it carries a manifest,
///   matches it;
/// - the extension's ABI matches the DuckDB this binary links, and it loaded;
/// - the environment was sealed to [`SEALED_ENV`] and [`SEALED_ENV_SELF_SET`];
/// - a column was typed and its values validated against that label.
///
/// **It does not say the artefact is self-contained, and cannot.** The
/// environment is sealed; THE FILESYSTEM IS NOT. Two routes out of the bundle
/// are known and neither is visible here: `config.value_embed_model` in the
/// model's own config is resolved as a path and an absolute one leaves the
/// artefact entirely, and FineType's Model2Vec loader falls back to a
/// cwd-relative `models/model2vec`. The working directory is sealed below,
/// which closes the second; the first is inside FineType's path resolution and
/// nothing in this repository can see it.
///
/// That is a limitation of the approach, not an omission. Proving "no file
/// outside the artefact was read" by naming the ways out is an enumeration, and
/// an enumeration is only ever as good as its last revision — this one has been
/// wrong about a code path, an environment variable and a working directory in
/// turn. The bounded form is deny-by-default: a jail that refuses reads outside
/// the artefact, the way `scripts/verify-airgapped.sh` already refuses the
/// network. That is not built.
///
/// # Why it runs twice
///
/// An earlier version of this check reported success on a bundle whose weights
/// were eighteen bytes of rubbish, whenever `FINETYPE_MODEL_DIR` happened to be
/// exported in the caller's environment: the extension took the model from
/// there, classified perfectly, and the check said the artefact worked. It had
/// asked whether the answer was right without asking where the inputs came
/// from, which is the only question an artefact check exists to answer.
///
/// Refusing that one variable would have been the nearest fix and the wrong
/// shape. So the outer phase re-execs this binary with the environment CLEARED
/// and only [`SEALED_ENV`] put back, and the inner phase refuses to proceed if
/// it sees anything outside that list and [`SEALED_ENV_SELF_SET`]. A re-exec
/// rather than clearing the variables in
/// place, because the dynamic loader reads `DYLD_*` / `LD_*` before `main` runs
/// and a process cannot un-substitute a library that was already interposed
/// into it.
///
/// # Exit codes
///
/// Distinct because the outcomes want different actions:
///
/// - `0` — a bundle was found, came up, and typed a column. Its name, the
///   directory, the label and the value-check result go to stdout.
/// - `1` — a bundle is present and did not work, or the sealed environment
///   could not be established. The reason goes to stderr. A packaging defect.
/// - `2` — there is no bundle beside this executable. Not a defect: a build
///   packaged without one is supported (see `scripts/package.sh`).
fn check_type_source() -> i32 {
    if std::env::var_os(SEALED_MARKER).is_none() {
        return reexec_sealed();
    }
    // The marker is a hint, not a credential — a caller can export it too. What
    // is trusted is the environment actually present.
    let stowaways = unsealed_variables();
    if !stowaways.is_empty() {
        eprintln!(
            "check-type-source: refusing to report on an artefact with {} variable(s) outside \
             the sealed allowlist: {}. Each one can redirect a file the check reads, so a \
             verdict taken here would be about the machine and not the artefact.",
            stowaways.len(),
            stowaways.join(", ")
        );
        return 1;
    }
    sealed_check_type_source()
}

/// Environment variables present that the sealed phase does not permit.
fn unsealed_variables() -> Vec<String> {
    unsealed_among(std::env::vars_os().map(|(k, _)| k.to_string_lossy().into_owned()))
}

/// The allowlist decision itself, over a supplied set of names.
///
/// Split from the live environment so a test can drive it: a test process is
/// full of Cargo's own variables, so a check that reads the real environment
/// can only ever be exercised by the sealed child it is meant to protect.
fn unsealed_among(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut found: Vec<String> = names
        .into_iter()
        .filter(|k| !SEALED_ENV.contains(&k.as_str()) && !SEALED_ENV_SELF_SET.contains(&k.as_str()))
        .collect();
    found.sort();
    found.dedup();
    found
}

/// Re-run this binary's `--check-type-source` with a scrubbed environment, and
/// forward its verdict.
///
/// The sandbox a caller has put this process in — `sandbox-exec` on macOS,
/// `unshare -rn` on Linux — applies to the whole process tree, so the child is
/// jailed exactly as the parent was. Neither scrubs the environment, which is
/// why this is needed at all.
fn reexec_sealed() -> i32 {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("check-type-source: cannot locate this executable: {e}");
            return 1;
        }
    };
    // A directory of this run's own, so HOME and TMPDIR point somewhere empty
    // and disposable rather than anywhere the caller chose.
    let sealed_root =
        std::env::temp_dir().join(format!("brightfield-type-source-{}", std::process::id()));
    let home = sealed_root.join("home");
    let tmp = sealed_root.join("tmp");
    if let Err(e) = std::fs::create_dir_all(&home).and_then(|()| std::fs::create_dir_all(&tmp)) {
        eprintln!("check-type-source: cannot make a sealed working directory: {e}");
        return 1;
    }

    let status = std::process::Command::new(&exe)
        .arg("--check-type-source")
        // The child inherits the caller's working directory unless told
        // otherwise, and a relative path is an input like any other: FineType's
        // Model2Vec loader falls back to a cwd-relative `models/model2vec`, so
        // running the check from a directory that happens to contain one lets
        // a bundle with its encoder deleted pass. Measured: from a neutral
        // directory that bundle exits 1, from such a directory it exited 0.
        .current_dir(&sealed_root)
        .env_clear()
        .env(SEALED_MARKER, "1")
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &home)
        .env("TMPDIR", &tmp)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .status();

    std::fs::remove_dir_all(&sealed_root).ok();

    match status {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!(
                "check-type-source: cannot re-run {} sealed: {e}",
                exe.display()
            );
            1
        }
    }
}

/// The check itself, in a process whose environment has been verified minimal.
///
/// It opens by reporting the seal it is running under — the working directory,
/// `HOME`, and the variables it can see, by name. That line is not decoration:
/// it is how the seal is observable from outside the process, and
/// `crates/brightfield-shell/tests/type_source_seal.rs` reads it to prove that
/// a hostile `HOME` and working directory did not reach here. A seal nothing
/// can observe is a seal nothing can test.
fn sealed_check_type_source() -> i32 {
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

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("?"));
    let home = std::env::var("HOME").unwrap_or_else(|_| "?".to_string());
    // The NAMES, not just a count. A count cannot distinguish a sealed run from
    // one carrying a stowaway of the same size, and the list is short by
    // construction — if it ever is not, that is the thing worth seeing.
    let mut names: Vec<String> = std::env::vars_os()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    names.sort();
    println!(
        "check-type-source: sealed — env [{}], cwd {}, HOME {}",
        names.join(" "),
        cwd.display(),
        home
    );

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("check-type-source: cannot locate this executable: {e}");
            return 1;
        }
    };
    let Some(bundle) = semantic::bundle_beside(&exe) else {
        println!(
            "check-type-source: no type source bundled beside {}",
            exe.display()
        );
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
    // An empty directory of this run's own. `LOAD` takes the bundled extension
    // by absolute path so it could not be redirected here — but DuckDB's
    // extension directory defaults to `~/.duckdb`, and a machine with FineType
    // already installed has one sitting in it. Pointing this at an empty
    // directory means an extension resolved by NAME has nowhere to come from,
    // so a warm cache cannot stand in for the artefact. One route closed, not
    // the filesystem — see this function's caller for what that leaves open.
    let ext_dir = std::env::temp_dir().join("brightfield-type-source-extensions");
    if let Err(e) = std::fs::create_dir_all(&ext_dir) {
        eprintln!("check-type-source: cannot make an empty extension directory: {e}");
        return 1;
    }
    let options = LoadOptions {
        network: NetworkPolicy::Disabled,
        extension_directory: Some(ext_dir),
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
        assert!(matches!(parse(&["-V"]), Ok(Invocation::Version)));
        // Wins over anything else on the line, and opens no window.
        assert!(matches!(
            parse(&["examples/bars.yaml", "--version"]),
            Ok(Invocation::Version)
        ));
    }

    /// `--check-type-source` short-circuits like `--version` and `--help`.
    ///
    /// Nothing else on the line can turn it back into a window: it exists to be
    /// run unattended by `scripts/verify-airgapped.sh`, and a flag that
    /// sometimes opened a window would hang that script on a build machine
    /// rather than fail it.
    #[test]
    fn check_type_source_flag_short_circuits() {
        assert!(matches!(
            parse(&["--check-type-source"]),
            Ok(Invocation::CheckTypeSource)
        ));
        assert!(matches!(
            parse(&["spec.yaml", "--check-type-source", "--shot-after", "45"]),
            Ok(Invocation::CheckTypeSource)
        ));
    }

    /// The sealed phase admits what the outer phase put back, and what the
    /// platform writes into it after `execve`, and refuses everything else.
    ///
    /// An allowlist, asserted as one: the named intruders below are the
    /// redirections that were actually found or looked for — a model path, a
    /// label override, three HuggingFace cache homes, a DuckDB extension
    /// directory, and the two dynamic-loader families — but the case that
    /// matters is `SOMETHING_INVENTED_TOMORROW`, which nobody enumerated and
    /// which is refused anyway.
    #[test]
    fn the_sealed_environment_admits_the_allowlist_and_what_the_platform_adds() {
        let permitted = ["PATH", "HOME", "TMPDIR", "LC_ALL", "LANG", SEALED_MARKER];
        assert!(
            unsealed_among(permitted.iter().map(|s| (*s).to_string())).is_empty(),
            "the sealed phase rejected the environment the outer phase builds for it"
        );
        assert!(
            unsealed_among(SEALED_ENV_SELF_SET.iter().map(|s| (*s).to_string())).is_empty(),
            "a variable the child writes into itself was treated as a stowaway"
        );

        for intruder in [
            "FINETYPE_MODEL_DIR",
            "FINETYPE_INJECT_LABEL",
            "RHH_DISABLE_HINTS",
            "HF_HOME",
            "HF_HUB_CACHE",
            "HUGGINGFACE_HUB_CACHE",
            "XDG_CACHE_HOME",
            "DUCKDB_EXTENSION_DIRECTORY",
            "DYLD_INSERT_LIBRARIES",
            "DYLD_LIBRARY_PATH",
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "SOMETHING_INVENTED_TOMORROW",
        ] {
            let mut env: Vec<String> = permitted.iter().map(|s| (*s).to_string()).collect();
            env.push(intruder.to_string());
            assert_eq!(
                unsealed_among(env),
                vec![intruder.to_string()],
                "{intruder} was admitted to the sealed environment"
            );
        }
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
