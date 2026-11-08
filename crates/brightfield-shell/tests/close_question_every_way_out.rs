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
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
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
        scratch();
        let csv = housing_copy(name);
        let mut win = Self {
            app: MeridianApp::headless(Boot::empty(), Mode::Light)
                .running_with(Some(Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))))
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
fn a_newer_open_takes_the_place_of_the_one_asked_about() {
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

    win.assert_holds("open_newer_newer");
    assert!(
        !Window::dir_of("open_newer_older").exists(),
        "nothing is made of the open the newer one replaced"
    );
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

#[test]
fn closing_without_saving_during_a_run_waits_for_it() {
    let mut win = Window::opened("close_run");
    win.start_run();
    win.keep_filter();

    win.cmd_q();
    win.forget_sent();
    win.key(egui::Key::D, egui::Modifiers::NONE);
    if win.app.run_outstanding() {
        assert!(!win.closed, "the window waits for the run before it closes");
    }

    win.until("the window never closed", |w| w.closed);
    assert!(
        !win.app.run_outstanding(),
        "it closed once the run had landed"
    );
    let records = brightfield_shell::run::record_paths_newest_first(&win.dir());
    assert!(!records.is_empty(), "the run's record is in the folder");
}

#[test]
fn discard_records_the_kept_filter_and_typed_sql() {
    let mut win = Window::opened("discard_keeps");
    win.keep_filter();
    let step =
        win.keep_statement("select house_age, count(*) as n from discard_keeps group by house_age");

    win.frame(Vec::new(), true);
    win.settle();
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
    assert!(win.closed);
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
