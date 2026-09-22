//! The first screen runs the Protocol a data file opened as, and every surface
//! that reports a run reads the run it just took.
//!
//! # What was wrong
//!
//! A data file opens as a Protocol of one SQL step, and the ledger's Log and
//! Quality panes said their content appears *once the Protocol runs* — with
//! nothing on the screen that ran it. A reader was left asking what step they
//! had not taken.
//!
//! # What these tests drive
//!
//! **A real run.** The window is given the `brightfield-shell` binary cargo
//! built for this suite as its runner, so taking the control starts that binary
//! as `arc` over the spec it wrote, and what the assertions read back is the
//! record `arc` wrote — not a fixture shaped like one. The data is the California
//! Housing Parquet the front door's start ships, written into a directory each
//! test owns: the start's click lands on the same `Boot::data_file` route with
//! the same bytes, and a directory of the test's own keeps a developer's real
//! datasets directory out of it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use brightfield_protocol::layout::Flow;
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::SpineRole;
use brightfield_shell::run::{self, Runner};
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp, RUNNING_LABEL, RUN_LABEL, RUN_PROTOCOL};
use brightfield_workbench::arrangement::LEDGER_RAIL;

/// How long a run may take before the test calls it hung. A run here is a
/// debug build of this binary starting, DuckDB reading a 20,640-row Parquet
/// and a contract written; the ceiling is for a loaded CI runner, not a
/// prediction of the time.
const PATIENCE: Duration = Duration::from_secs(180);

/// The step the housing file's Protocol runs, as the spine names it.
const STEP: &str = "load";

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "bf-run-control-{name}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// The housing start's own bytes, under the start's own file name.
    fn housing(&self) -> PathBuf {
        let path = self.0.join(starts::HOUSING_FILE);
        std::fs::write(&path, starts::HOUSING_PARQUET).expect("the housing file writes");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The runner this suite hands a window: the `brightfield-shell` binary cargo
/// built beside this test, whose `main` becomes `arc` under
/// [`run::RUNNER_ENV`].
fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// A window under test, with one `egui::Context` for its whole life.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    /// A window over the data file at `path`, given `runner`.
    fn over_file(path: &Path, runner: Option<Runner>) -> Self {
        let boot = Boot::data_file(&path.to_string_lossy()).expect("the housing file opens");
        Self::over(boot, runner)
    }

    fn over(boot: Boot, runner: Option<Runner>) -> Self {
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light)
                .running_with(runner),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
        };
        win.settle();
        win
    }

    /// Draw one frame with `events`, and hand back every text it drew with the
    /// rect the text landed in.
    fn frame(&mut self, events: Vec<egui::Event>) -> Vec<(egui::Rect, String)> {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            events,
            ..Default::default()
        };
        let out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        let mut text = Vec::new();
        for clipped in &out.shapes {
            collect_placed_text(&clipped.shape, &mut text);
        }
        text
    }

    /// Three frames, for the reason `protocol_run_start.rs` gives: a resizable
    /// panel reports the size it settled at on the frame after.
    fn settle(&mut self) -> Vec<(egui::Rect, String)> {
        self.frame(Vec::new());
        self.frame(Vec::new());
        self.frame(Vec::new())
    }

    /// Click at `at`, the way a pointer does: move, press, release, in one
    /// frame.
    fn click(&mut self, at: egui::Pos2) -> Vec<(egui::Rect, String)> {
        self.frame(vec![
            egui::Event::PointerMoved(at),
            button_at(at, true),
            button_at(at, false),
        ])
    }

    /// Take the ledger strip's Run control where the last frame drew it.
    fn take_run_control(&mut self) {
        let at = self
            .app
            .rail_action_rect(LEDGER_RAIL)
            .expect("the ledger strip drew no Run control to take")
            .center();
        // Hover first: a button a pointer arrives on and clicks within the
        // same frame is still a click, but the frame before settles hover.
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.click(at);
    }

    /// Draw frames until the run this window started has landed, and say how
    /// many frames were drawn while it was outstanding, with the longest of
    /// them and every text those frames drew inside the Run control.
    fn drive_until_landed(&mut self) -> Driven {
        let deadline = Instant::now() + PATIENCE;
        let mut driven = Driven::default();
        while self.app.run_in_progress() {
            assert!(
                Instant::now() < deadline,
                "the run did not land inside {PATIENCE:?} — this test is hung, not slow"
            );
            let started = Instant::now();
            let text = self.frame(Vec::new());
            let took = started.elapsed();
            if self.app.run_in_progress() {
                driven.frames += 1;
                driven.longest = driven.longest.max(took);
                if let Some(control) = self.app.rail_action_rect(LEDGER_RAIL) {
                    driven.control_words.extend(
                        text.iter()
                            .filter(|(rect, _)| control.expand(1.0).contains_rect(*rect))
                            .map(|(_, t)| t.clone()),
                    );
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.settle();
        driven
    }

    /// The texts the last settled frame drew inside `rect`.
    fn text_in(&mut self, rect: egui::Rect) -> Vec<String> {
        self.frame(Vec::new())
            .into_iter()
            .filter(|(r, _)| rect.expand(1.0).contains_rect(*r))
            .map(|(_, t)| t)
            .collect()
    }

    /// What the ledger strip's summary reads on the next frame.
    fn strip_summary(&mut self) -> Vec<String> {
        self.frame(Vec::new());
        let summary = self
            .app
            .rail_summary_rect(LEDGER_RAIL)
            .expect("the ledger's strip drew a summary");
        self.text_in(summary)
    }

    /// The kind column of the spine's step row for [`STEP`].
    fn step_row_kind(&mut self) -> String {
        self.frame(Vec::new());
        self.app
            .spine_rows()
            .iter()
            .find(|row| row.role == SpineRole::Step && row.label == STEP)
            .map(|row| row.kind.clone())
            .unwrap_or_else(|| panic!("the spine drew no step row for {STEP}"))
    }

    /// Open the ledger rail on its `index`-th pane, and hand back what the rail
    /// drew.
    fn open_ledger_pane(&mut self, index: usize) -> Vec<String> {
        let at = self
            .app
            .rail_name_rect(LEDGER_RAIL, index)
            .expect("the ledger strip drew that name")
            .center();
        self.click(at);
        self.settle();
        let rail = self
            .app
            .region_rect(LEDGER_RAIL)
            .expect("the ledger rail drew");
        // By where each text starts, not by its whole box: the run's log is one
        // galley of many lines inside a scroll area, taller than the rail, so
        // its box is never inside the rail's even though it is drawn there.
        self.frame(Vec::new())
            .into_iter()
            .filter(|(r, _)| rail.contains(r.min))
            .map(|(_, t)| t)
            .collect()
    }
}

/// What [`Window::drive_until_landed`] saw.
#[derive(Default)]
struct Driven {
    frames: usize,
    longest: Duration,
    control_words: Vec<String>,
}

fn button_at(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// Every text in `shape`, with where it was drawn — `protocol_run_start.rs`'s
/// walk, for its reason: the claim is about what a particular control or strip
/// drew, not about the words appearing somewhere on the window.
fn collect_placed_text(shape: &egui::epaint::Shape, into: &mut Vec<(egui::Rect, String)>) {
    match shape {
        egui::epaint::Shape::Text(t) => {
            into.push((
                t.galley.rect.translate(t.pos.to_vec2()),
                t.galley.text().to_string(),
            ));
        }
        egui::epaint::Shape::Vec(shapes) => {
            for s in shapes {
                collect_placed_text(s, into);
            }
        }
        _ => {}
    }
}

/// The run records under `dir`, newest first.
fn records(dir: &Path) -> Vec<PathBuf> {
    run::record_paths_newest_first(dir)
}

// ---------------------------------------------------------------------------
// AC1 — a control a stranger can read without the palette
// ---------------------------------------------------------------------------

/// **The housing file's first screen carries a Run control on the ledger
/// strip, and the verb behind it is bound and labelled.**
///
/// The control is read off the frame: the strip recorded a rect for it, and the
/// word drawn inside that rect is [`RUN_LABEL`]. The verb is read off the
/// registry: built, bound, with a help line for the palette. And the binding is
/// driven — the keystroke the registry declares starts a run — because a bound
/// verb the window does not consume is a binding only on paper.
///
/// Watched redden, one mutation: the ledger's `ledger_trailing` built with
/// `action: None` fails the expect on `rail_action_rect` — and the same
/// mutation reddens `the_generated_dashboard_light_baseline`, which is AC6's
/// claim that the photographs carry the control.
#[test]
fn the_housing_file_offers_a_run_control_on_the_ledger_strip() {
    let verb = brightfield_keys::registry()
        .into_iter()
        .find(|v| v.longname == RUN_PROTOCOL)
        .expect("run-protocol is in the registry");
    assert!(!verb.is_reserved(), "run-protocol is still reserved");
    assert_eq!(
        verb.primary_key(),
        Some("cmd-enter"),
        "run-protocol carries no binding"
    );
    assert!(
        !verb.help.is_empty(),
        "run-protocol has no palette label to read"
    );

    let dir = TempDir::new("offers");
    let path = dir.housing();
    let mut win = Window::over_file(&path, Some(runner()));
    let control = win
        .app
        .rail_action_rect(LEDGER_RAIL)
        .expect("the ledger strip drew no Run control on the housing file");
    let words = win.text_in(control);
    assert_eq!(
        words,
        vec![RUN_LABEL.to_string()],
        "the ledger strip's control at {control:?} reads {words:?}"
    );
    let strip = win
        .app
        .rail_summary_rect(LEDGER_RAIL)
        .expect("the strip drew its summary");
    assert!(
        control.right() <= strip.left(),
        "the Run control at {control:?} is not before the summary at {strip:?} \
         — it should sit beside the word it changes"
    );

    // The binding, driven: cmd-enter starts a run.
    let modifiers = egui::Modifiers {
        command: true,
        ..Default::default()
    };
    win.frame(vec![egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }]);
    assert!(
        win.app.run_in_progress(),
        "the registry's cmd-enter did not start a run"
    );
    win.drive_until_landed();
}

/// **A Protocol with no spec behind it draws no Run control** — a shipped
/// start whose Protocol came off a bundled record rather than a file.
///
/// The negative half of AC1's placement: the control is the data file's, and a
/// control drawn over a document with nothing to run would be a verb that
/// confirms and does nothing. The strip still draws its summary there, so the
/// absence is of the control and not of the strip.
///
/// Watched redden, one mutation: `run_action` built whatever
/// `source().is_some()` says: the `assert_eq!` reads a rect.
#[test]
fn a_protocol_with_no_spec_behind_it_draws_no_run_control() {
    let boot = Boot::start(starts::CROSSWALK_RUN, Flow::Vertical).expect("the run start loads");
    let mut win = Window::over(boot, Some(runner()));
    assert!(
        win.app.rail_summary_rect(LEDGER_RAIL).is_some(),
        "the crosswalk run's strip drew no summary, so its absent control \
         proves nothing"
    );
    assert_eq!(win.app.rail_action_rect(LEDGER_RAIL), None);
    assert!(
        !win.app.run_protocol(&win.ctx.clone()),
        "run_protocol started a run over a Protocol with no spec"
    );
}

// ---------------------------------------------------------------------------
// AC2 — the run writes the record, and the shell reads it in the session
// ---------------------------------------------------------------------------

/// **Taking Run writes the record where `arc run` writes it, and the strip, the
/// step row, Log and Quality read it in the same session.**
///
/// Before the click: no record, the strip reads *not run*, the step row reads
/// *not run*. After the run lands: one contract under
/// `<dir>/build/.arcform/runs/`, the strip reads *last run · success*, the step
/// row reads `ok`, and both run panes head with the record's own run id rather
/// than the not-run empty state.
///
/// Watched redden, one mutation: `poll_run`'s reload opening `source.inputs()`
/// instead of `inputs_with_last_run()` leaves the strip reading
/// `["last run · not run"]` after the run.
#[test]
fn taking_run_writes_the_record_where_arc_writes_it() {
    let dir = TempDir::new("writes");
    let path = dir.housing();
    let mut win = Window::over_file(&path, Some(runner()));

    assert!(records(dir.path()).is_empty(), "a record before any run");
    let before = win.strip_summary();
    assert!(
        before.iter().any(|t| t.contains("not run")),
        "the strip reads {before:?} before any run"
    );
    assert!(
        win.step_row_kind().contains("not run"),
        "the step row reads a state before any run"
    );

    win.take_run_control();
    win.drive_until_landed();

    let written = records(dir.path());
    assert_eq!(
        written.len(),
        1,
        "the run wrote {written:?} under {}",
        run::runs_dir(dir.path()).display()
    );
    assert_eq!(
        written[0].parent(),
        Some(dir.path().join("build/.arcform/runs").as_path()),
        "the record is not where `arc run` writes one"
    );
    let run_id = written[0]
        .file_stem()
        .and_then(|s| s.to_str())
        .expect("a run id")
        .to_string();

    let after = win.strip_summary();
    assert!(
        after
            .iter()
            .any(|t| t.contains("last run") && t.contains("success")),
        "after the run the strip reads {after:?}"
    );
    assert!(
        after.iter().all(|t| !t.contains("not run")),
        "after the run the strip still reads not run: {after:?}"
    );
    let kind = win.step_row_kind();
    assert!(
        kind.contains("ok") && !kind.contains("not run"),
        "after the run the step row reads {kind:?}"
    );

    for (index, pane) in [(0, "Log"), (1, "Quality")] {
        let drawn = win.open_ledger_pane(index);
        assert!(
            drawn
                .iter()
                .any(|t| t.contains(&run_id) && t.contains("success")),
            "{pane} drew {drawn:?}, which does not head with run {run_id}"
        );
        assert!(
            !drawn.iter().any(|t| t == "Not run"),
            "{pane} still draws the not-run empty state after the run: {drawn:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// AC3 — the window draws while the run executes
// ---------------------------------------------------------------------------

/// **Frames go by while the run is under way, the control says so on them, and
/// no one of them is held for the run.**
///
/// The frame count is the remote start's own budget: more than one frame drawn
/// with the work outstanding, which a run taken inside the click cannot
/// produce — it returns with the run already landed. The control's words are
/// read off those frames. And the longest frame is held under the run's own
/// wall time, which is the frame a synchronous run would have had to draw.
///
/// Watched redden, one mutation: the strip's `label` bound to `RUN_LABEL`
/// whatever `running` says — the frames drawn during the run read *Run*.
#[test]
fn the_window_keeps_drawing_while_the_run_executes() {
    let dir = TempDir::new("draws");
    let path = dir.housing();
    let mut win = Window::over_file(&path, Some(runner()));

    let clicked = Instant::now();
    win.take_run_control();
    assert!(
        win.app.run_in_progress(),
        "the run was not under way when the click's frame returned"
    );
    let driven = win.drive_until_landed();
    let whole_run = clicked.elapsed();

    assert!(
        driven.frames > 1,
        "only {} frame(s) were drawn while the run was outstanding",
        driven.frames
    );
    assert!(
        driven.control_words.iter().all(|w| w == RUNNING_LABEL) && !driven.control_words.is_empty(),
        "while the run was under way the control read {:?}",
        driven.control_words
    );
    assert!(
        driven.longest < whole_run,
        "a frame took {:?} of a run that took {whole_run:?}",
        driven.longest
    );
    let settled = win.text_in(
        win.app
            .rail_action_rect(LEDGER_RAIL)
            .expect("the control is drawn after the run"),
    );
    assert_eq!(settled, vec![RUN_LABEL.to_string()]);
}

// ---------------------------------------------------------------------------
// AC4 — a failed step reads failed, and the Log carries the error
// ---------------------------------------------------------------------------

/// **A step whose SQL fails leaves the strip reading *last run · failed*, the
/// Log pane carrying the error, and the window up.**
///
/// The SQL fails for real: the file the step reads is overwritten, after the
/// window has opened it, with bytes that are not a Parquet, so DuckDB refuses
/// `read_parquet` inside the step and `arc` records the outcome `error`.
///
/// Watched redden, one mutation: `LogPane::ui` reading no log — the Log pane
/// draws `last run · failed` and no error text.
#[test]
fn a_step_whose_sql_fails_reads_failed_and_the_log_says_why() {
    let dir = TempDir::new("fails");
    let path = dir.housing();
    let mut win = Window::over_file(&path, Some(runner()));
    std::fs::write(&path, b"these bytes are not a parquet file\n").expect("the file is spoiled");

    win.take_run_control();
    win.drive_until_landed();

    assert_eq!(
        records(dir.path()).len(),
        1,
        "the failed run wrote no record"
    );
    let after = win.strip_summary();
    assert!(
        after
            .iter()
            .any(|t| t.contains("last run") && t.contains("failed")),
        "after a failed run the strip reads {after:?}"
    );
    assert!(
        !after.iter().any(|t| t.contains("success")),
        "a failed run reads success: {after:?}"
    );
    let log = win.open_ledger_pane(0).join("\n");
    assert!(
        log.contains("step 'load' failed"),
        "the Log pane drew no error text for the failed step:\n{log}"
    );
    // Still drawing: the failure closed nothing.
    assert!(win.app.rail_action_rect(LEDGER_RAIL).is_some());
}

// ---------------------------------------------------------------------------
// AC5 — a second launch reads the record the first run wrote
// ---------------------------------------------------------------------------

/// **A second window on the same file reads the last run without running.**
///
/// The second window is built the way a relaunch builds one — a fresh
/// `Boot::data_file` over the path — and is given **no runner**, so a run it
/// shows cannot have come from a run of its own. Its strip reads *last run ·
/// success* and its step row `ok`, and no second record appears.
///
/// Watched redden, one mutation: `Boot::of_opened_file` building its inputs
/// with `protocol.inputs()` instead of `inputs_with_last_run()` — the second
/// window reads `["last run · not run"]`.
#[test]
fn a_second_launch_reads_the_record_the_first_run_wrote() {
    let dir = TempDir::new("relaunch");
    let path = dir.housing();
    {
        let mut first = Window::over_file(&path, Some(runner()));
        first.take_run_control();
        first.drive_until_landed();
    }
    assert_eq!(records(dir.path()).len(), 1);

    let mut second = Window::over_file(&path, None);
    let summary = second.strip_summary();
    assert!(
        summary
            .iter()
            .any(|t| t.contains("last run") && t.contains("success")),
        "a second launch on the same file reads {summary:?}"
    );
    let kind = second.step_row_kind();
    assert!(
        kind.contains("ok") && !kind.contains("not run"),
        "a second launch's step row reads {kind:?}"
    );
    assert_eq!(
        records(dir.path()).len(),
        1,
        "the second launch ran the Protocol again"
    );
}

// ---------------------------------------------------------------------------
// The engine the child is handed, and the two refusals a landing run passes
// ---------------------------------------------------------------------------

/// **A runner laid out the way the tarball ships hands its child the DuckDB
/// staged beside it.**
///
/// The layout is built from parts: `pkg/brightfield` is a link to the binary
/// cargo built for this suite, and `pkg/engine/duckdb` — the path
/// `scripts/package.sh` stages the CLI at — is a two-line script that marks a
/// file and then runs the real CLI (the one an inherited
/// [`run::ENGINE_ENV`] names, else `duckdb` on the search path). The run must
/// land as a success AND the mark must be there: an inherited engine completes
/// the run just as well, so the success alone says nothing about which engine
/// the child was told.
///
/// Watched redden, one mutation: the `command.env(ENGINE_ENV, engine)` in
/// `run_to_completion` removed — the run still succeeds on the inherited
/// engine and the mark is absent.
#[cfg(unix)]
#[test]
fn a_packaged_runner_hands_its_child_the_engine_staged_beside_it() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new("staged-engine");
    let path = dir.housing();
    let pkg = dir.path().join("pkg");
    std::fs::create_dir_all(pkg.join("engine")).expect("the package layout");
    std::os::unix::fs::symlink(
        env!("CARGO_BIN_EXE_brightfield-shell"),
        pkg.join("brightfield"),
    )
    .expect("the runner links into the layout");
    let mark = dir.path().join("staged-engine-ran");
    let real = std::env::var_os(run::ENGINE_ENV).map_or_else(
        || "duckdb".to_string(),
        |p| p.to_string_lossy().into_owned(),
    );
    let engine = pkg.join("engine").join("duckdb");
    std::fs::write(
        &engine,
        format!(
            "#!/bin/sh\n: > '{}'\nexec '{}' \"$@\"\n",
            mark.display(),
            real
        ),
    )
    .expect("the staged engine writes");
    std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o755))
        .expect("the staged engine is executable");

    let mut win = Window::over_file(&path, Some(Runner::at(pkg.join("brightfield"))));
    win.take_run_control();
    win.drive_until_landed();

    let summary = win.strip_summary();
    assert!(
        summary
            .iter()
            .any(|t| t.contains("last run") && t.contains("success")),
        "the run through the packaged layout reads {summary:?}"
    );
    assert!(
        mark.is_file(),
        "the run succeeded without the engine staged at {} — the child was not told it",
        engine.display()
    );
}

/// **A record that names this Protocol but not its steps is not this
/// Protocol's run.**
///
/// The crosswalk's contract fixture is a real `arc` record of four steps. Its
/// protocol name is rewritten to the housing file's, and it is written where a
/// run of the housing file would be. The name matches, so the name filter in
/// [`run::records_newest_first`] passes it; the step set does not — the housing
/// Protocol is one step — so the window must read no run at all.
///
/// Watched redden, one mutation: `|| declared != recorded` dropped from
/// `ProtocolInputs::adopt_run` — the strip reads the fixture's *success*.
#[test]
fn a_record_whose_steps_are_not_this_protocols_is_not_read_as_its_run() {
    let dir = TempDir::new("foreign-steps");
    let path = dir.housing();
    let name = Path::new(starts::HOUSING_FILE)
        .file_stem()
        .and_then(|s| s.to_str())
        .expect("the housing file has a stem");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../brightfield-protocol/fixtures/edgar_gleif.contract.json");
    let mut contract: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&fixture).expect("the crosswalk contract fixture reads"),
    )
    .expect("the fixture is JSON");
    contract["run"]["protocol"]["name"] = serde_json::Value::from(name);
    assert_eq!(
        contract["run"]["outcome"], "success",
        "the fixture no longer records a success, so the strip could not tell adopted from refused"
    );
    let runs = run::runs_dir(dir.path());
    std::fs::create_dir_all(&runs).expect("the runs directory");
    std::fs::write(
        runs.join("foreign-steps.json"),
        serde_json::to_string_pretty(&contract).expect("the contract serialises"),
    )
    .expect("the record writes");
    assert_eq!(
        run::records_newest_first(dir.path(), name).count(),
        1,
        "the rewritten record does not carry the housing Protocol's name, so the step-set \
         refusal is never reached"
    );

    let mut win = Window::over_file(&path, None);
    let summary = win.strip_summary();
    assert!(
        summary.iter().any(|t| t.contains("not run"))
            && !summary.iter().any(|t| t.contains("success")),
        "a record of other steps was read as this Protocol's run: the strip reads {summary:?}"
    );
    assert!(
        win.step_row_kind().contains("not run"),
        "a record of other steps set the step row"
    );
}

/// **A run that finishes after the reader opened another file lands on
/// nothing.**
///
/// The housing file's run is taken in one directory, and before it lands a CSV
/// in a second directory is opened into the same window. Frames are drawn until
/// the run has provably been taken ([`MeridianApp::run_outstanding`] clears)
/// and its record is on disk. The CSV's window must then read as it opened:
/// the strip *not run*, the Log pane its not-run empty state, and no record in
/// its directory.
///
/// Watched redden, one mutation: `poll_run`'s `if !self.holds_protocol(…)`
/// replaced with `if false` — the housing run's log lands on the CSV's Log
/// pane.
#[test]
fn a_run_that_lands_after_another_file_opened_lands_on_nothing() {
    let first = TempDir::new("lands-first");
    let housing = first.housing();
    let second = TempDir::new("lands-second");
    let csv = second.path().join("tiny.csv");
    std::fs::write(&csv, "a,b\n1,2\n3,4\n").expect("the CSV writes");

    let mut win = Window::over_file(&housing, Some(runner()));
    win.take_run_control();
    assert!(win.app.run_outstanding(), "taking Run started no run");
    win.app.open_data_file(&win.ctx, &csv.to_string_lossy());
    win.settle();
    assert!(
        !win.app.run_in_progress(),
        "the CSV's window reads the housing run as its own"
    );

    let deadline = Instant::now() + PATIENCE;
    while win.app.run_outstanding() {
        assert!(
            Instant::now() < deadline,
            "the run did not land inside {PATIENCE:?} — this test is hung, not slow"
        );
        win.frame(Vec::new());
        std::thread::sleep(Duration::from_millis(5));
    }
    win.settle();
    assert_eq!(
        records(first.path()).len(),
        1,
        "the housing run left no record, so nothing landed for the guard to refuse"
    );

    let summary = win.strip_summary();
    assert!(
        summary.iter().any(|t| t.contains("not run")),
        "the CSV's strip reads {summary:?} after another file's run landed"
    );
    let log = win.open_ledger_pane(0);
    assert!(
        log.iter().any(|t| t == "Not run"),
        "the CSV's Log pane drew {log:?}, not its not-run empty state"
    );
    assert!(
        records(second.path()).is_empty(),
        "a record appeared beside the CSV"
    );
}
