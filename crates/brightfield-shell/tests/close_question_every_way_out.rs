//! Every way out of a Protocol with a draft pending asks the close question first: going Home,
//! opening another Protocol, quitting and closing the window. Enter saves and then goes, D closes
//! without saving and then goes, and Esc keeps editing. A close without saving keeps every draft it
//! drops as a version, the grid view and the typed SQL with the chart, and a quit with a run in
//! flight keeps the run.
//!
//! Every window is headless, with the `brightfield-shell` binary cargo built as its runner and a
//! store of versions of its own, and the config directory, the Protocols folder and arc's
//! directories point at a scratch folder of this file's, so nothing is written under the home
//! directory.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use brightfield_shell::design::Mode;
use brightfield_shell::open_queue::OpenRequest;
use brightfield_shell::run::Runner;
use brightfield_shell::versions::{Listing, CLOSED_UNSAVED};
use brightfield_shell::window::{
    Boot, MeridianApp, KEPT_WHILE_CLOSING_TITLE, UNSAVED_MARK, WAITING_FOR_RUN_BODY,
    WAITING_FOR_RUN_TITLE,
};
use brightfield_workbench::arrangement::LEDGER_RAIL;

/// The overlay's name for the question, as `MeridianApp::open_overlay` says it.
const QUESTION: &str = "close-question";

/// The condition kept on the grid, and the word the step arc records for it carries.
const CONDITION: &str = "house_age > 28";
const FILTER_STEP_SUFFIX: &str = "_filtered_by_house_age";

/// The Versions panel's place in the ledger strip.
const VERSIONS_AT: usize = 4;

/// How long a run may take before a test calls it hung.
const PATIENCE: Duration = Duration::from_secs(180);

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-every-way-out-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "protocols", "input", "versions"] {
            std::fs::create_dir_all(root.join(sub)).expect("a scratch folder");
        }
        // Set once, before any test in this file reads them, and never changed.
        std::env::set_var(
            brightfield_shell::startup::CONFIG_DIR_VAR,
            root.join("config"),
        );
        std::env::set_var(
            brightfield_shell::protocols_dir::PROTOCOLS_DIR_VAR,
            root.join("protocols"),
        );
        std::env::set_var("ARCFORM_HISTORY_DIR", root.join("history"));
        std::env::set_var("ARCFORM_DB_DIR", root.join("db"));
        root
    })
}

/// A copy of the committed California Housing sample, named `<name>.csv`, in a folder of its own.
fn housing_copy(name: &str) -> PathBuf {
    let dir = scratch().join("input").join(name);
    std::fs::create_dir_all(&dir).expect("an input folder");
    let csv = dir.join(format!("{name}.csv"));
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv"),
        &csv,
    )
    .expect("the housing sample copies");
    csv
}

fn press(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// One headless window holding the one-source Protocol made of a copy of the housing sample, and
/// what its frames have sent the operating system.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    name: String,
    time: f64,
    /// Whether a frame since the last [`Self::forget_sent`] sent `CancelClose`, and `Close`.
    cancelled_close: bool,
    closed: bool,
}

impl Window {
    /// The window, with the Protocol named `name` opened on it through the open queue.
    fn opened(name: &str) -> Self {
        Self::opened_with(name, Runner::at(env!("CARGO_BIN_EXE_brightfield-shell")))
    }

    /// [`Self::opened`], handing the window `runner` to open and run with.
    fn opened_with(name: &str, runner: Runner) -> Self {
        scratch();
        let csv = housing_copy(name);
        let mut win = Self {
            app: MeridianApp::headless(Boot::empty(), Mode::Light)
                .running_with(Some(runner))
                .keeping_history(Some(brightfield_protocol::HistoryStore::At(
                    scratch().join("versions"),
                ))),
            ctx: egui::Context::default(),
            name: name.to_owned(),
            time: 0.0,
            cancelled_close: false,
            closed: false,
        };
        win.app.open_queue().push(OpenRequest::Path(csv));
        win.settle();
        win.assert_holds(name);
        win
    }

    fn dir_of(name: &str) -> PathBuf {
        scratch().join("protocols").join(name)
    }

    fn dir(&self) -> PathBuf {
        Self::dir_of(&self.name)
    }

    fn manifest(&self) -> String {
        std::fs::read_to_string(self.dir().join("arcform.yaml")).expect("the manifest reads")
    }

    fn filter_step(&self) -> String {
        format!("{}{FILTER_STEP_SUFFIX}", self.name)
    }

    fn assert_holds(&self, name: &str) {
        assert_eq!(
            self.app.own_protocol().map(|p| p.dir.clone()),
            Some(Self::dir_of(name)),
            "the window holds the Protocol {name}: {}",
            self.said()
        );
    }

    /// Run one frame with `events`, `close` raising a close request in it as the window's close
    /// button does.
    fn frame(&mut self, events: Vec<egui::Event>, close: bool) {
        self.time += 1.0;
        let modifiers = events
            .iter()
            .find_map(|e| match e {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440.0, 900.0),
            )),
            time: Some(self.time),
            modifiers,
            events,
            ..Default::default()
        };
        if close {
            raw.viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .events
                .push(egui::ViewportEvent::Close);
        }
        let out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        if let Some(root) = out.viewport_output.get(&egui::ViewportId::ROOT) {
            for command in &root.commands {
                match command {
                    egui::ViewportCommand::CancelClose => self.cancelled_close = true,
                    egui::ViewportCommand::Close => self.closed = true,
                    _ => {}
                }
            }
        }
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        self.frame(events, false);
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        self.run(vec![press(key, modifiers)]);
        self.settle();
    }

    fn forget_sent(&mut self) {
        self.cancelled_close = false;
        self.closed = false;
    }

    fn click(&mut self, at: egui::Pos2) {
        self.run(vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ]);
        self.settle();
    }

    fn type_text(&mut self, text: &str) {
        self.run(vec![egui::Event::Text(text.to_owned())]);
        self.settle();
    }

    fn marked(&self) -> bool {
        self.app.title().contains(UNSAVED_MARK)
    }

    /// Keep a filter on the grid, unsaved.
    fn keep_filter(&mut self) {
        self.app
            .chart_doc_mut()
            .apply_grid_view(brightfield_engine::RowsView {
                condition: Some(CONDITION.to_owned()),
                order: None,
            });
        self.settle();
        assert!(self.marked(), "the kept filter marks the title unsaved");
        assert!(
            !self.manifest().contains(&self.filter_step()),
            "the filter is a draft until Cmd+S"
        );
    }

    /// Keep `statement` as a step from the statement form: Space, the text, its read, Cmd+Enter.
    /// Answers the name it was kept under.
    fn keep_statement(&mut self, statement: &str) -> String {
        self.key(egui::Key::Space, egui::Modifiers::NONE);
        self.type_text(statement);
        assert!(
            self.app.finish_typed_reads(Duration::from_secs(60)),
            "the read of {statement:?} did not finish: {}",
            self.said()
        );
        self.key(egui::Key::Enter, egui::Modifiers::COMMAND);
        assert_eq!(
            self.app.open_overlay(),
            None,
            "{statement:?} was not kept: {}",
            self.said()
        );
        self.app
            .chart_doc()
            .typed_steps()
            .last()
            .expect("a statement is kept")
            .name
            .clone()
    }

    /// Press Home's key, cmd-shift-h.
    fn home(&mut self) {
        self.key(
            egui::Key::H,
            egui::Modifiers {
                command: true,
                shift: true,
                ..Default::default()
            },
        );
    }

    /// Press Cmd+Q, as one frame, then let the window settle.
    fn cmd_q(&mut self) {
        self.key(egui::Key::Q, egui::Modifiers::COMMAND);
    }

    fn asking(&self) -> bool {
        self.app.open_overlay() == Some(QUESTION)
    }

    fn said(&self) -> String {
        self.app
            .notifications()
            .iter()
            .map(|n| format!("{}: {}", n.title, n.body.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Draw frames until `pred` holds, or fail after `patience`.
    fn until(&mut self, what: &str, pred: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !pred(self) {
            assert!(Instant::now() < deadline, "{what}: {}", self.said());
            std::thread::sleep(Duration::from_millis(5));
            self.run(Vec::new());
        }
        self.settle();
    }

    /// Start a run with Cmd+Enter.
    fn start_run(&mut self) {
        self.run(vec![press(egui::Key::Enter, egui::Modifiers::COMMAND)]);
        assert!(
            self.app.run_in_progress(),
            "cmd-enter started a run: {}",
            self.said()
        );
    }

    /// Open the ledger on Versions and answer what the panel lists.
    fn versions_listed(&mut self) -> Listing {
        let name = self
            .app
            .rail_name_rect(LEDGER_RAIL, VERSIONS_AT)
            .expect("the ledger strip drew its Versions name")
            .center();
        self.click(name);
        self.settle();
        self.app.chart_doc().versions().listing().clone()
    }
}

#[test]
fn home_with_a_draft_asks() {
    let mut win = Window::opened("home_asks");
    win.keep_filter();

    win.home();

    assert!(win.asking(), "Home over a draft asks first");
    assert!(!win.app.front_door_is_live(), "and does not go yet");

    win.key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(!win.asking());
    assert!(!win.app.front_door_is_live(), "Keep editing stays");
    assert!(win.marked(), "with the draft still there");

    win.home();
    assert!(win.asking(), "Home asks again");
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(
        win.app.front_door_is_live(),
        "Close without saving goes Home: {}",
        win.said()
    );
    assert!(
        !win.manifest().contains(&win.filter_step()),
        "and records nothing in the Protocol"
    );
}

#[test]
fn home_with_nothing_pending_goes_at_once() {
    let mut win = Window::opened("home_clean");

    win.home();

    assert!(!win.asking());
    assert!(win.app.front_door_is_live());
}

#[test]
fn opening_another_protocol_with_a_draft_asks() {
    let mut win = Window::opened("open_asks");
    win.keep_filter();
    let other = housing_copy("open_asks_other");

    win.app.open_queue().push(OpenRequest::Path(other));
    win.settle();

    assert!(win.asking(), "an open over a draft asks first");
    win.assert_holds("open_asks");
    assert!(
        !Window::dir_of("open_asks_other").exists(),
        "nothing is made of the open before the answer"
    );

    win.key(egui::Key::D, egui::Modifiers::NONE);
    win.settle();

    win.assert_holds("open_asks_other");
    assert!(!win.marked(), "the draft did not come along");
}

#[test]
fn a_newer_open_waits_behind_the_one_asked_about() {
    let mut win = Window::opened("open_newer");
    win.keep_filter();
    let older = housing_copy("open_newer_older");
    let newer = housing_copy("open_newer_newer");

    win.app.open_queue().push(OpenRequest::Path(older));
    win.settle();
    assert!(win.asking());
    win.app.open_queue().push(OpenRequest::Path(newer));
    win.settle();
    assert!(win.asking(), "the question stays up");

    win.key(egui::Key::D, egui::Modifiers::NONE);
    win.settle();

    // The open the question was asked for goes, then the one held behind it.
    assert!(
        Window::dir_of("open_newer_older")
            .join("arcform.yaml")
            .is_file(),
        "the open asked about went ahead: {}",
        win.said()
    );
    win.assert_holds("open_newer_newer");
}

#[test]
fn keep_editing_drops_the_open() {
    let mut win = Window::opened("open_declined");
    win.keep_filter();
    let declined = housing_copy("open_declined_other");
    win.app.open_queue().push(OpenRequest::Path(declined));
    win.settle();
    assert!(win.asking());

    win.key(egui::Key::Escape, egui::Modifiers::NONE);
    // Saving afterwards does not bring the declined open back.
    win.key(egui::Key::S, egui::Modifiers::COMMAND);
    for _ in 0..10 {
        win.run(Vec::new());
    }

    win.assert_holds("open_declined");
    assert!(
        win.manifest().contains(&win.filter_step()),
        "{}",
        win.said()
    );
    assert!(
        !Window::dir_of("open_declined_other").exists(),
        "nothing is made of a declined open"
    );
}

/// **Untrusting a Protocol with a draft asks first, and keeping editing leaves it unable to run.**
/// The reopen restricted waits behind the close question; Esc keeps the draft on the canvas, and
/// Run refuses the Protocol the store no longer trusts.
#[test]
fn keep_editing_after_untrusting_refuses_a_run() {
    let mut win = Window::opened("untrust_kept");
    win.keep_filter();
    let ctx = win.ctx.clone();
    assert!(win.app.untrust_open_protocol(&ctx));
    win.settle();
    assert!(win.asking(), "a draft asks before the reopen");

    win.key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(!win.asking());
    assert!(
        win.app.restricted().is_none(),
        "kept editing, it is not reopened"
    );
    assert!(!win.app.run_protocol(&ctx), "it does not run untrusted");
    assert!(
        win.said().contains("not trusted"),
        "the window says why: {}",
        win.said()
    );
}

#[test]
fn quit_with_a_draft_asks_and_waits() {
    let mut win = Window::opened("quit_asks");
    win.keep_filter();

    win.cmd_q();

    assert!(win.asking(), "Cmd+Q over a draft asks first");
    assert!(!win.closed, "and the window is not closed while it asks");

    // The system's own close while the question is up is cancelled, and the question stays.
    win.frame(Vec::new(), true);
    win.settle();
    assert!(win.cancelled_close, "the close request was cancelled");
    assert!(!win.closed);
    assert!(win.asking());

    win.forget_sent();
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(
        win.closed,
        "Close without saving then closes: {}",
        win.said()
    );
}

#[test]
fn the_macs_quit_asks_too() {
    let mut win = Window::opened("quit_menu");
    win.keep_filter();

    // What the app menu's Quit and the Dock's do: ask on the window's queue.
    win.app.open_queue().ask_quit();
    win.settle();

    assert!(win.asking(), "the Mac's quit over a draft asks first");
    assert!(!win.closed);
    win.key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(!win.closed, "Keep editing stays");
    assert!(win.marked());
}

#[test]
fn quit_with_nothing_pending_closes_at_once() {
    let mut win = Window::opened("quit_clean");

    win.cmd_q();

    assert!(!win.asking());
    assert!(win.closed, "a quit with nothing to lose closes");
}

#[test]
fn quit_during_a_run_keeps_the_run() {
    let mut win = Window::opened("quit_run");
    // A step recorded first, so the run is one of the Protocol's own steps and its record is
    // taken onto the strip when it lands.
    win.keep_filter();
    win.key(egui::Key::S, egui::Modifiers::COMMAND);
    assert!(!win.marked(), "the filter is saved: {}", win.said());
    let before = brightfield_shell::run::record_paths_newest_first(&win.dir()).len();
    win.start_run();
    // A draft made while the run is under way, which the run did not record.
    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some(CONDITION.to_owned()),
            order: Some("median_income".to_owned()),
        });
    win.settle();
    assert!(win.marked(), "the order is a draft");

    win.cmd_q();
    assert!(win.asking(), "Cmd+Q over the draft asks");
    win.key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(!win.closed);

    win.until("the run never landed", |w| !w.app.run_outstanding());
    assert!(
        brightfield_shell::run::record_paths_newest_first(&win.dir()).len() > before,
        "the run's record is in the folder"
    );
    assert!(
        win.app.protocol_model().run().is_some(),
        "the run's record was taken onto the strip: {}",
        win.said()
    );
    assert!(win.marked(), "and the draft is still there");
}

/// A runner that is the real one, except that a run waits while the file `hold` exists, for two
/// minutes at most, before it runs: a run a test can keep outstanding for as long as it needs, and
/// then let finish. The test removes `hold` before it ends, so no run is left waiting.
struct HeldRunner {
    hold: PathBuf,
    runner: Runner,
}

impl HeldRunner {
    fn new(name: &str) -> Self {
        let dir = scratch().join("input").join(format!("{name}_runner"));
        std::fs::create_dir_all(&dir).expect("the runner's folder");
        let hold = dir.join("hold");
        let script = dir.join("held-runner");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nif [ \"$1\" = run ]; then\n  i=0\n  while [ -f '{}' ] && [ $i -lt 1200 ]; do sleep 0.1; i=$((i+1)); done\nfi\nexec '{}' \"$@\"\n",
                hold.display(),
                env!("CARGO_BIN_EXE_brightfield-shell"),
            ),
        )
        .expect("the runner writes");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("the runner is executable");
        }
        Self {
            hold,
            runner: Runner::at(script),
        }
    }

    fn hold(&self) {
        std::fs::write(&self.hold, "").expect("the hold writes");
    }

    fn release(&self) {
        let _ = std::fs::remove_file(&self.hold);
    }
}

impl Drop for HeldRunner {
    fn drop(&mut self) {
        self.release();
    }
}

/// A window over a Protocol with a saved step, a run of it held outstanding, and a draft made
/// since: what a close or quit answered during a run meets.
fn waiting_on_a_held_run(name: &str) -> (Window, HeldRunner) {
    let held = HeldRunner::new(name);
    let mut win = Window::opened_with(name, held.runner.clone());
    win.keep_filter();
    win.key(egui::Key::S, egui::Modifiers::COMMAND);
    assert!(!win.marked(), "the filter is saved: {}", win.said());
    held.hold();
    win.start_run();
    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some(CONDITION.to_owned()),
            order: Some("median_income".to_owned()),
        });
    win.settle();
    assert!(win.marked(), "the order is a draft");
    (win, held)
}

impl Window {
    fn says_waiting(&self) -> bool {
        self.app.notifications().iter().any(|n| {
            n.title == WAITING_FOR_RUN_TITLE && n.body.as_deref() == Some(WAITING_FOR_RUN_BODY)
        })
    }
}

#[test]
fn closing_without_saving_during_a_run_waits_for_it() {
    let (mut win, held) = waiting_on_a_held_run("close_run");

    win.cmd_q();
    assert!(win.asking());
    win.forget_sent();
    win.key(egui::Key::D, egui::Modifiers::NONE);
    for _ in 0..20 {
        win.run(Vec::new());
    }

    assert!(win.app.run_outstanding(), "the held run is still going");
    assert!(!win.closed, "the window waits for the run before it closes");
    assert!(!win.asking(), "and does not ask again what was answered");
    assert!(
        win.says_waiting(),
        "a banner says what it waits for: {}",
        win.said()
    );

    let before = brightfield_shell::run::record_paths_newest_first(&win.dir()).len();
    held.release();
    win.until("the window never closed", |w| w.closed);
    assert!(
        !win.app.run_outstanding(),
        "it closed once the run had landed"
    );
    assert!(
        brightfield_shell::run::record_paths_newest_first(&win.dir()).len() > before,
        "the run's record is in the folder"
    );
    assert!(!win.says_waiting(), "the banner went with the wait");
}

#[test]
fn a_second_quit_while_waiting_for_a_run_closes_now() {
    let (mut win, held) = waiting_on_a_held_run("quit_twice");
    win.cmd_q();
    win.forget_sent();
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(!win.closed && win.says_waiting(), "{}", win.said());

    // Home while it waits is refused, with the banner that says why: the analyst has already
    // chosen to leave.
    win.home();
    assert!(
        !win.app.front_door_is_live(),
        "Home is refused during the wait"
    );
    assert!(!win.asking());
    assert!(win.says_waiting());
    assert!(!win.closed);

    // An open asked for during the wait is not done, and the window says so rather than drop it
    // unsaid.
    let other = housing_copy("quit_twice_other");
    win.app.open_queue().push(OpenRequest::Path(other));
    win.settle();
    win.assert_holds("quit_twice");
    assert!(
        !Window::dir_of("quit_twice_other").exists(),
        "nothing is made of it"
    );
    assert!(
        win.app
            .notifications()
            .iter()
            .any(|n| n.title == "quit_twice_other.csv was not opened"),
        "a banner says the open was not done: {}",
        win.said()
    );
    assert!(win.says_waiting() && !win.asking());

    win.cmd_q();

    assert!(win.closed, "a second quit closes at once: {}", win.said());
    assert!(win.app.run_outstanding(), "with the run still held");
    held.release();
    win.until("the held run never finished", |w| !w.app.run_outstanding());
}

#[test]
fn a_second_quit_keeps_the_edits_made_while_waiting() {
    let (mut win, held) = waiting_on_a_held_run("quit_twice_edits");
    win.cmd_q();
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(!win.closed && win.says_waiting(), "{}", win.said());

    // An edit made while the quit waits, which the quit was not answered over.
    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some("house_age > 40".to_owned()),
            order: None,
        });
    win.settle();
    win.forget_sent();
    win.cmd_q();

    assert!(win.closed, "a second quit closes at once: {}", win.said());
    assert!(
        win.app
            .notifications()
            .iter()
            .any(|n| n.title == KEPT_WHILE_CLOSING_TITLE),
        "a banner says the edits were kept: {}",
        win.said()
    );
    held.release();
    win.until("the held run never finished", |w| !w.app.run_outstanding());
    let Listing::Listed { rows, .. } = win.versions_listed() else {
        panic!("the Versions panel lists nothing: {}", win.said());
    };
    assert!(
        rows.iter()
            .any(|r| r.kind == CLOSED_UNSAVED && r.changed.contains("house_age > 40")),
        "the edit made during the wait is kept as a version: {rows:?}"
    );
}

#[test]
fn a_second_quit_keeps_yaml_typed_while_waiting() {
    let (mut win, held) = waiting_on_a_held_run("quit_twice_yaml");
    let spec = win
        .app
        .chart_doc()
        .spec_path
        .clone()
        .expect("the editor's buffer has a file");
    win.click_into_the_editor();
    win.type_text("# typed before the quit\n");
    win.let_go_of_the_keyboard();
    win.cmd_q();
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(!win.closed && win.says_waiting(), "{}", win.said());

    // More typing in the same buffer while the quit waits: the question's line for it is the
    // same, though its text is not.
    win.click_into_the_editor();
    win.type_text("# typed while waiting\n");
    win.let_go_of_the_keyboard();
    win.forget_sent();
    win.cmd_q();

    assert!(win.closed, "a second quit closes at once: {}", win.said());
    held.release();
    win.until("the held run never finished", |w| !w.app.run_outstanding());
    let store = brightfield_protocol::HistoryStore::At(scratch().join("versions"));
    let versions = store.versions(&spec).expect("the spec's versions list");
    assert!(
        versions.versions.iter().any(|v| {
            v.kind == brightfield_protocol::chart_history::HistoryKind::Unsaved
                && store
                    .read_version(&spec, &v.id)
                    .is_ok_and(|text| text.contains("# typed while waiting"))
        }),
        "the typing done while waiting is kept as a version"
    );
}

#[test]
fn a_quit_whose_run_lands_keeps_yaml_typed_while_waiting() {
    let (mut win, held) = waiting_on_a_held_run("land_yaml");
    let spec = win
        .app
        .chart_doc()
        .spec_path
        .clone()
        .expect("the editor's buffer has a file");
    win.click_into_the_editor();
    win.type_text("# typed before the quit\n");
    win.let_go_of_the_keyboard();
    win.cmd_q();
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(!win.closed && win.says_waiting(), "{}", win.said());

    win.click_into_the_editor();
    win.type_text("# typed while the run went on\n");
    win.let_go_of_the_keyboard();
    win.forget_sent();

    // No second quit: the run lands, and the window closes as the quit asked.
    held.release();
    win.until("the window never closed", |w| w.closed);
    let store = brightfield_protocol::HistoryStore::At(scratch().join("versions"));
    let versions = store.versions(&spec).expect("the spec's versions list");
    assert!(
        versions.versions.iter().any(|v| {
            v.kind == brightfield_protocol::chart_history::HistoryKind::Unsaved
                && store
                    .read_version(&spec, &v.id)
                    .is_ok_and(|text| text.contains("# typed while the run went on"))
        }),
        "the typing done while the run went on is kept as a version"
    );
}

#[test]
fn a_second_quit_that_cannot_keep_the_work_asks_again() {
    let (mut win, held) = waiting_on_a_held_run("quit_twice_unkept");
    win.cmd_q();
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(!win.closed && win.says_waiting(), "{}", win.said());

    // The store taken away during the wait, so the work can no longer be kept.
    win.app.set_history(None);
    win.forget_sent();
    win.cmd_q();
    assert!(!win.closed, "the window stays over work it could not keep");
    let question = win.app.close_question().expect("the question asks again");
    assert!(question.not_kept.is_some(), "saying why: {question:?}");

    // Close without saving again is the analyst choosing to lose it: the quit waits for the run
    // as before, and the next quit closes.
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(!win.closed && win.says_waiting(), "{}", win.said());
    win.cmd_q();
    assert!(win.closed, "the loss was chosen: {}", win.said());
    held.release();
    win.until("the held run never finished", |w| !w.app.run_outstanding());
}

/// A file that asks which Protocol when opened alone, being one of the two sources of a folder's
/// Protocol: `<name>_left.csv`, beside `<name>_right.csv`.
fn a_file_that_asks(name: &str) -> PathBuf {
    let pair = scratch().join("input").join(format!("{name}_pair"));
    std::fs::create_dir_all(&pair).expect("a folder");
    let left = pair.join(format!("{name}_left.csv"));
    std::fs::write(&left, "id,name\n1,Ada\n2,Bo\n").unwrap();
    std::fs::write(
        pair.join(format!("{name}_right.csv")),
        "id,left_id\n10,1\n11,2\n",
    )
    .unwrap();
    let mut maker = Window::opened(&format!("{name}_maker"));
    maker.app.open_queue().push(OpenRequest::Path(pair));
    maker.settle();
    assert!(maker.app.model_pane().is_some(), "{}", maker.said());
    left
}

#[test]
fn files_held_behind_a_quit_that_waits_for_a_run_are_all_said_not_opened() {
    let first = a_file_that_asks("wait_held_one");
    let second = a_file_that_asks("wait_held_two");
    let (mut win, held) = waiting_on_a_held_run("wait_held");
    win.cmd_q();
    assert!(win.asking());
    win.app.open_queue().push(OpenRequest::Path(first));
    win.app.open_queue().push(OpenRequest::Path(second));
    win.settle();
    assert!(win.asking(), "the question stands: {}", win.said());

    win.key(egui::Key::D, egui::Modifiers::NONE);
    win.settle();
    assert!(win.says_waiting() && !win.closed, "{}", win.said());
    assert_eq!(
        win.app.open_overlay(),
        None,
        "no sheet asks: {}",
        win.said()
    );
    assert!(
        win.app
            .notifications()
            .iter()
            .any(|n| n.title == "wait_held_one_left.csv and 1 more was not opened"),
        "one banner names both: {}",
        win.said()
    );
    held.release();
    win.until("the held run never finished", |w| !w.app.run_outstanding());
}

#[test]
fn a_run_landing_over_work_that_cannot_be_kept_asks_again() {
    let (mut win, held) = waiting_on_a_held_run("land_unkept");
    // No store, so no work can be kept as a version.
    win.app.set_history(None);
    win.cmd_q();
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(
        win.app
            .close_question()
            .is_some_and(|q| q.not_kept.is_some()),
        "{}",
        win.said()
    );
    // Close without saving again: the loss is chosen, and the quit waits for the run.
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(!win.closed && win.says_waiting(), "{}", win.said());

    // An edit the analyst never saw listed, then the run lands with no second quit.
    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some("house_age > 43".to_owned()),
            order: None,
        });
    win.settle();
    win.forget_sent();
    held.release();
    win.until("the run never landed", |w| !w.app.run_outstanding());
    win.settle();
    assert!(!win.closed, "the window did not close over the edit");
    assert!(
        win.app
            .close_question()
            .is_some_and(|q| q.not_kept.is_some()),
        "the question is back, saying why: {}",
        win.said()
    );
}

#[test]
fn a_loss_chosen_going_home_is_not_carried_into_a_later_quit() {
    let held = HeldRunner::new("stale_loss");
    let mut win = Window::opened_with("stale_loss", held.runner.clone());
    // No store, so no work can be kept as a version.
    win.app.set_history(None);
    win.keep_filter();
    win.home();
    assert!(win.asking());
    win.key(egui::Key::D, egui::Modifiers::NONE);
    let question = win.app.close_question().expect("the question says why");
    assert!(question.not_kept.is_some(), "{question:?}");
    // Close without saving again: the analyst chooses to lose the filter, going Home.
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(win.app.front_door_is_live(), "{}", win.said());

    // Later, the Protocol again, clean, quit with a run outstanding.
    win.app.open_queue().push(OpenRequest::Path(win.dir()));
    win.settle();
    win.assert_holds("stale_loss");
    assert!(!win.marked(), "{}", win.said());
    held.hold();
    win.start_run();
    win.cmd_q();
    assert!(!win.asking() && win.says_waiting(), "{}", win.said());

    // An edit during the wait, which cannot be kept: the second quit asks rather than close.
    win.keep_filter();
    win.forget_sent();
    win.cmd_q();
    assert!(!win.closed, "the window stays over work it could not keep");
    assert!(
        win.app
            .close_question()
            .is_some_and(|q| q.not_kept.is_some()),
        "and asks again, saying why: {}",
        win.said()
    );
    held.release();
    win.until("the held run never finished", |w| !w.app.run_outstanding());
}

#[test]
fn a_close_while_a_quit_waits_for_a_run_closes_now() {
    let (mut win, held) = waiting_on_a_held_run("close_twice");
    win.cmd_q();
    assert!(win.asking());
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(win.says_waiting(), "{}", win.said());
    win.forget_sent();

    // The window's close button while the quit waits: the request is let through, which is how
    // eframe closes.
    win.frame(Vec::new(), true);

    assert!(!win.cancelled_close, "the second close is not cancelled");
    assert!(win.app.is_closing(), "the window lets it through");
    held.release();
    win.until("the held run never finished", |w| !w.app.run_outstanding());
}

#[test]
fn a_close_while_a_quit_waits_keeps_the_edits_made_while_waiting() {
    let (mut win, held) = waiting_on_a_held_run("close_twice_edits");
    win.cmd_q();
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(win.says_waiting(), "{}", win.said());
    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some("house_age > 41".to_owned()),
            order: None,
        });
    win.settle();
    win.forget_sent();

    // The window's close button while the quit waits.
    win.frame(Vec::new(), true);

    assert!(!win.cancelled_close, "the close is let through");
    assert!(win.app.is_closing());
    assert!(
        win.app
            .notifications()
            .iter()
            .any(|n| n.title == KEPT_WHILE_CLOSING_TITLE),
        "a banner says the work was kept: {}",
        win.said()
    );
    held.release();
    win.until("the held run never finished", |w| !w.app.run_outstanding());
    let Listing::Listed { rows, .. } = win.versions_listed() else {
        panic!("the Versions panel lists nothing: {}", win.said());
    };
    assert!(
        rows.iter()
            .any(|r| r.kind == CLOSED_UNSAVED && r.changed.contains("house_age > 41")),
        "the edit made during the wait is kept as a version: {rows:?}"
    );
}

#[test]
fn a_close_on_the_door_waits_for_a_run_as_a_quit_does() {
    let (mut win, held) = waiting_on_a_held_run("door_close_run");
    win.frame(Vec::new(), true);
    win.settle();
    assert!(win.asking());
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(win.app.front_door_is_live(), "{}", win.said());
    assert!(
        win.app.run_outstanding(),
        "the held run goes on behind the door"
    );
    win.forget_sent();

    // A close on the door with the run out is a quit, which waits for the run.
    win.frame(Vec::new(), true);
    win.settle();
    assert!(
        win.cancelled_close,
        "the close is cancelled while the run is out"
    );
    assert!(!win.closed && !win.app.is_closing());
    assert!(
        win.says_waiting(),
        "a banner says what it waits for: {}",
        win.said()
    );

    // Another close while it waits ends the app now.
    win.forget_sent();
    win.frame(Vec::new(), true);
    assert!(!win.cancelled_close, "the next close is let through");
    assert!(win.app.is_closing());
    held.release();
    win.until("the held run never finished", |w| !w.app.run_outstanding());
}

#[test]
fn closing_the_window_without_saving_records_the_kept_filter_and_typed_sql() {
    let mut win = Window::opened("close_discard_keeps");
    win.keep_filter();
    let step = win.keep_statement(
        "select house_age, count(*) as n from close_discard_keeps group by house_age",
    );

    win.frame(Vec::new(), true);
    win.settle();
    assert!(win.asking());
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(
        win.app.front_door_is_live() && !win.closed,
        "D goes to the door: {}",
        win.said()
    );

    // Opened again from where it is, the Versions panel lists what the close kept.
    win.app.open_queue().push(OpenRequest::Path(win.dir()));
    win.settle();
    win.assert_holds("close_discard_keeps");
    let Listing::Listed { rows, .. } = win.versions_listed() else {
        panic!("the Versions panel lists nothing: {}", win.said());
    };
    let row = rows
        .iter()
        .find(|r| r.kind == CLOSED_UNSAVED)
        .unwrap_or_else(|| panic!("no closed unsaved version: {rows:?}"));
    assert!(
        row.changed
            .contains(&format!("Grid view \u{b7} where {CONDITION}")),
        "the version names the filter: {:?}",
        row.changed
    );
    assert!(
        row.changed.contains(&format!("SQL step \u{b7} {step}")),
        "and the typed SQL: {:?}",
        row.changed
    );
}

/// A SQL step written and not saved is a change to the Protocol, and the question says so: it
/// asks to save the Protocol, names it, and calls the edit a Protocol edit, never a chart edit.
#[test]
fn a_sql_step_draft_is_asked_about_as_a_protocol_edit() {
    let mut win = Window::opened("step_is_protocol");
    let step = win
        .keep_statement("select house_age, count(*) as n from step_is_protocol group by house_age");
    win.cmd_q();
    assert!(win.asking());
    let question = win.app.close_question().expect("the question").clone();
    let name = win
        .app
        .open_protocol_dir()
        .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
        .expect("a Protocol is open");
    assert_eq!(
        question.title(),
        brightfield_shell::overlays::CLOSE_PROTOCOL_QUESTION_TITLE
    );
    assert_eq!(
        question.headline(),
        format!("{name} has 1 Protocol edit that is not saved."),
        "{:?}",
        question.edits
    );
    assert!(
        question.edits.iter().any(|e| e.contains(&step)),
        "{:?}",
        question.edits
    );
}

#[test]
fn discard_records_the_kept_filter_and_typed_sql() {
    let mut win = Window::opened("discard_keeps");
    win.keep_filter();
    let step =
        win.keep_statement("select house_age, count(*) as n from discard_keeps group by house_age");

    // Asked by a quit, which leaves the window holding the Protocol for the Versions panel to be
    // read below; the close button would take it to the door.
    win.cmd_q();
    assert!(win.asking());
    let edits = win
        .app
        .close_question()
        .expect("the question")
        .edits
        .clone();
    assert!(
        edits.iter().any(|e| e.contains(CONDITION)),
        "the question lists the filter: {edits:?}"
    );
    assert!(
        edits.iter().any(|e| e.contains(&step)),
        "and the statement: {edits:?}"
    );
    let manifest_before = win.manifest();
    win.forget_sent();
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(win.closed, "{}", win.said());
    assert_eq!(
        win.manifest(),
        manifest_before,
        "arcform.yaml is not written"
    );

    let Listing::Listed { rows, .. } = win.versions_listed() else {
        panic!("the Versions panel lists nothing: {}", win.said());
    };
    let row = rows
        .iter()
        .find(|r| r.kind == CLOSED_UNSAVED)
        .unwrap_or_else(|| panic!("no closed unsaved version: {rows:?}"));
    assert!(
        row.changed
            .contains(&format!("Grid view \u{b7} where {CONDITION}")),
        "the version names the filter: {:?}",
        row.changed
    );
    assert!(
        row.changed.contains(&format!("SQL step \u{b7} {step}")),
        "and the typed SQL: {:?}",
        row.changed
    );
}

#[test]
fn enter_saves_and_leaves() {
    let mut win = Window::opened("enter_saves");
    win.keep_filter();

    win.home();
    assert!(win.asking());
    win.key(egui::Key::Enter, egui::Modifiers::NONE);

    assert!(
        win.manifest().contains(&win.filter_step()),
        "Enter saved the draft: {}",
        win.said()
    );
    assert!(win.app.front_door_is_live(), "and went Home");
}

#[test]
fn an_edited_yaml_buffer_is_unsaved_work_and_a_discard_keeps_it() {
    let mut win = Window::opened("yaml_unsaved");
    assert!(!win.marked());
    let spec = win
        .app
        .chart_doc()
        .spec_path
        .clone()
        .expect("the editor's buffer has a file");
    let typed = "# typed in the editor\n";
    win.click_into_the_editor();
    win.type_text(typed);
    // The keyboard taken away from the editor: the buffer is still unsaved.
    win.ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    win.settle();
    assert!(win.marked(), "an edited YAML buffer marks the title");

    win.frame(Vec::new(), true);
    win.settle();
    assert!(win.asking(), "and the close question asks about it");
    assert!(
        win.app
            .close_question()
            .expect("the question")
            .edits
            .iter()
            .any(|e| e.starts_with("YAML editor")),
        "naming the editor"
    );
    let before = std::fs::read_to_string(&spec).expect("the spec reads");
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(win.app.front_door_is_live() && !win.closed);
    assert_eq!(
        std::fs::read_to_string(&spec).expect("the spec reads"),
        before,
        "the buffer is not written to its file"
    );
    let versions = brightfield_protocol::HistoryStore::At(scratch().join("versions"))
        .versions(&spec)
        .expect("the spec's versions list");
    let kept = versions
        .versions
        .iter()
        .find(|v| v.kind == brightfield_protocol::chart_history::HistoryKind::Unsaved)
        .expect("the buffer is kept as an unsaved version");
    let text = brightfield_protocol::HistoryStore::At(scratch().join("versions"))
        .read_version(&spec, &kept.id)
        .expect("reads back");
    assert!(text.contains(typed.trim()), "{text}");
}

impl Window {
    /// Take the keyboard away from whatever has it, as a click elsewhere does.
    fn let_go_of_the_keyboard(&mut self) {
        self.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        self.settle();
    }

    /// Open the YAML editor on the ledger rail, focus it, and click into its text.
    fn click_into_the_editor(&mut self) {
        let tab = self
            .app
            .rail_name_rect(LEDGER_RAIL, 3)
            .expect("the ledger rail drew its editor tab")
            .center();
        self.click(tab);
        assert!(
            self.app.focus_pane(brightfield_workbench::PaneKey::new(
                brightfield_shell::editor::EDITOR
            )),
            "the editor pane is in the window"
        );
        self.settle();
        let rail = self
            .app
            .region_rect(LEDGER_RAIL)
            .expect("the ledger rail drew open");
        let body_top = rail.top() + brightfield_workbench::chrome::rail_selector_height();
        // Past the gutter's line numbers, which no click reaches a widget through.
        self.click(egui::pos2(rail.left() + 150.0, body_top + 32.0));
        assert!(
            self.ctx.memory(|m| m.focused()).is_some(),
            "the click gave the editor's text the keyboard"
        );
    }
}

#[test]
fn save_on_the_question_over_a_buffer_changed_on_disk_stays() {
    let mut win = Window::opened("yaml_conflict_close");
    let spec = win
        .app
        .chart_doc()
        .spec_path
        .clone()
        .expect("the editor's buffer has a file");
    win.click_into_the_editor();
    win.type_text("# typed in the editor\n");
    win.ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    win.settle();
    let external = format!(
        "{}# changed outside\n",
        std::fs::read_to_string(&spec).expect("the spec reads")
    );
    std::fs::write(&spec, &external).expect("the outside change writes");

    win.frame(Vec::new(), true);
    win.settle();
    assert!(win.asking());
    win.forget_sent();
    win.key(egui::Key::Enter, egui::Modifiers::NONE);

    assert!(!win.closed, "the window stays open: {}", win.said());
    assert!(
        win.app
            .notifications()
            .iter()
            .any(|n| n.title == "The YAML editor's buffer was not saved"),
        "with a banner saying why: {}",
        win.said()
    );
    assert_eq!(
        std::fs::read_to_string(&spec).expect("the spec reads"),
        external,
        "and the change on disk stands"
    );
}

#[test]
fn a_stepped_back_unsaved_version_saves_as_the_chart_alone() {
    let mut win = Window::opened("step_back_kept");
    let dir = win.dir();
    let chart = dir.join("panels").join("step_back_kept.yaml");
    win.keep_filter();
    win.home();
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(win.app.front_door_is_live(), "{}", win.said());

    // Opened again, stepped back to the version the close kept, and saved.
    win.app.open_queue().push(OpenRequest::Path(dir.clone()));
    win.settle();
    win.assert_holds("step_back_kept");
    let Listing::Listed { rows, .. } = win.versions_listed() else {
        panic!("the Versions panel lists nothing: {}", win.said());
    };
    assert_eq!(rows[0].kind, CLOSED_UNSAVED, "{rows:?}");
    assert!(win.app.chart_doc_mut().move_version_cursor(true));
    assert!(
        win.app.chart_doc_mut().step_back_to_cursor(),
        "{}",
        win.said()
    );
    win.settle();
    win.key(egui::Key::S, egui::Modifiers::COMMAND);
    let saved = std::fs::read_to_string(&chart).expect("the chart file reads");
    assert!(
        !saved.contains(brightfield_protocol::PENDING_STEPS_HEADING),
        "the chart file holds the chart alone: {saved}"
    );

    // A later discard of other steps names those, and only those.
    let step = win
        .keep_statement("select house_age, count(*) as n from step_back_kept group by house_age");
    win.home();
    win.key(egui::Key::D, egui::Modifiers::NONE);
    win.app.open_queue().push(OpenRequest::Path(dir));
    win.settle();
    let Listing::Listed { rows, .. } = win.versions_listed() else {
        panic!("the Versions panel lists nothing: {}", win.said());
    };
    let newest = &rows[0];
    assert_eq!(newest.kind, CLOSED_UNSAVED, "{rows:?}");
    assert!(
        newest.changed.contains(&format!("SQL step \u{b7} {step}")),
        "the newest version names the new step: {:?}",
        newest.changed
    );
    assert!(
        !newest.changed.contains("Grid view"),
        "and not the one already saved: {:?}",
        newest.changed
    );
}
