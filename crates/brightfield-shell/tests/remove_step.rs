//! A step is removed from the spine with `d d` on the row of what it makes. The removal is a draft
//! until Cmd+S: the row is struck through and the title marked, and `arcform.yaml` keeps the step
//! until the Save takes it out, through arc, and deletes its SQL file. A load and the model's steps
//! are not offered the verb. A step that later steps read is refused, with a sentence naming each
//! of them. Closing without saving keeps the step, and the version the close keeps names the
//! removal.
//!
//! Every window is headless, with the `brightfield-shell` binary cargo built as its runner, and the
//! config directory, the Protocols folder and arc's stores point at scratch folders of this file's,
//! so nothing is written under the home directory.

#[path = "support/agent_protocol.rs"]
mod agent_protocol;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use brightfield_shell::design::Mode;
use brightfield_shell::open_queue::OpenRequest;
use brightfield_shell::protocol::{SpineRole, SpineRowDrawn};
use brightfield_shell::run::Runner;
use brightfield_shell::versions::{Listing, CLOSED_UNSAVED};
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_workbench::arrangement::LEDGER_RAIL;

/// The Versions panel's place in the ledger strip.
const VERSIONS_AT: usize = 4;

/// The close question's overlay name.
const QUESTION: &str = "close-question";

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        agent_protocol::arc_stores();
        let root = std::env::temp_dir().join(format!("bf-remove-step-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "protocols", "input", "versions"] {
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

/// One headless window holding the one-source Protocol made of a copy of the housing sample.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    name: String,
    time: f64,
}

impl Window {
    /// The window, with the Protocol named `name` opened on it through the open queue.
    fn opened(name: &str) -> Self {
        Self::opened_with(name, Runner::at(env!("CARGO_BIN_EXE_brightfield-shell")))
    }

    /// [`Self::opened`], handing the window `runner` to open, record and run with.
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
        };
        win.app.open_queue().push(OpenRequest::Path(csv));
        win.settle();
        win.assert_holds();
        win
    }

    fn dir(&self) -> PathBuf {
        scratch().join("protocols").join(&self.name)
    }

    fn assert_holds(&self) {
        assert_eq!(
            self.app.own_protocol().map(|p| p.dir.clone()),
            Some(self.dir()),
            "the window holds the Protocol {}: {}",
            self.name,
            self.said()
        );
    }

    fn manifest(&self) -> String {
        std::fs::read_to_string(self.dir().join("arcform.yaml")).expect("the manifest reads")
    }

    /// The names of the manifest's steps, in order.
    fn manifest_steps(&self) -> Vec<String> {
        brightfield_protocol::parse_manifest_str(&self.manifest())
            .expect("the manifest parses")
            .steps
            .into_iter()
            .map(|s| s.name)
            .collect()
    }

    /// The SQL file the manifest's step `name` names, inside the Protocol's folder.
    fn model_file(&self, name: &str) -> PathBuf {
        let manifest = brightfield_protocol::parse_manifest_str(&self.manifest())
            .expect("the manifest parses");
        let step = manifest
            .steps
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no step {name} in the manifest"));
        self.dir()
            .join(step.sql.as_ref().expect("a sql step names its file"))
    }

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
        let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
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

    /// Type a letter as a keyboard does: the key, and the text it makes.
    fn letter(&mut self, key: egui::Key, text: &str) {
        self.run(vec![
            press(key, egui::Modifiers::NONE),
            egui::Event::Text(text.to_owned()),
        ]);
        self.settle();
    }

    /// `d d`.
    fn d_d(&mut self) {
        self.letter(egui::Key::D, "d");
        self.letter(egui::Key::D, "d");
    }

    fn cmd_s(&mut self) {
        self.key(egui::Key::S, egui::Modifiers::COMMAND);
    }

    fn click(&mut self, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.run(vec![
            egui::Event::PointerMoved(at),
            button(true),
            button(false),
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

    fn said(&self) -> String {
        self.app
            .notifications()
            .iter()
            .map(|n| format!("{}: {}", n.title, n.body.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("; ")
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

    /// A statement kept, then saved with Cmd+S: answers its step's name.
    fn saved_statement(&mut self, statement: &str) -> String {
        let step = self.keep_statement(statement);
        self.cmd_s();
        assert!(
            self.manifest_steps().contains(&step),
            "Cmd+S recorded {step}: {:?}; {}",
            self.manifest_steps(),
            self.said()
        );
        assert!(
            !self.marked(),
            "and took the mark off: {}",
            self.app.title()
        );
        step
    }

    /// A step that groups the Protocol's table by `house_age`, kept and saved.
    fn saved_grouping(&mut self) -> String {
        let statement = format!(
            "select house_age, count(*) as n from {} group by house_age",
            self.name
        );
        self.saved_statement(&statement)
    }

    /// Put the graph on the canvas by the chip in the spine's head, if it is not there.
    fn graph_to_canvas(&mut self) {
        if self.app.graph_on_canvas() {
            return;
        }
        let chip = self
            .app
            .spine_rows()
            .iter()
            .find_map(|r| r.chip)
            .expect("the spine's head drew its graph chip");
        self.click(chip.rect.center());
        assert!(
            self.app.graph_on_canvas(),
            "the graph chip put no graph on the canvas"
        );
    }

    /// The spine's rows as the last frame drew them.
    fn rows(&self) -> Vec<SpineRowDrawn> {
        self.app.spine_rows().to_vec()
    }

    fn labels(&self) -> Vec<String> {
        self.rows()
            .iter()
            .map(|r| format!("{:?} {} [{}]", r.role, r.label, r.kind))
            .collect()
    }

    /// The step row labelled `step`.
    fn step_row(&self, step: &str) -> Option<SpineRowDrawn> {
        self.rows()
            .into_iter()
            .find(|r| r.role == SpineRole::Step && r.label == step)
    }

    /// With the graph on the canvas, select what the step `step` makes by a click on its row,
    /// the asset row under the step's.
    fn select_made_by(&mut self, step: &str) {
        self.graph_to_canvas();
        self.click_made_by(step);
        self.graph_to_canvas();
    }

    /// Click the asset row under the step `step`'s row, whatever the canvas holds.
    fn click_made_by(&mut self, step: &str) {
        let rows = self.rows();
        let at = rows
            .iter()
            .position(|r| r.role == SpineRole::Step && r.label == step)
            .unwrap_or_else(|| panic!("the spine drew no row for {step}: {:?}", self.labels()));
        let made = rows[at + 1..]
            .iter()
            .find(|r| r.role == SpineRole::Asset)
            .unwrap_or_else(|| panic!("no asset row under {step}: {:?}", self.labels()))
            .clone();
        self.click(made.rect.center());
        assert_eq!(
            self.app
                .protocol_model()
                .selected_step()
                .map(String::as_str),
            Some(step),
            "the click on {} selected what {step} makes",
            made.label
        );
    }

    /// The rows the Protocol palette lists when `space` opens it, closed again after.
    fn palette_rows(&mut self) -> Vec<String> {
        self.key(egui::Key::Space, egui::Modifiers::NONE);
        assert_eq!(
            self.app.open_overlay(),
            Some("palette"),
            "space opened no palette"
        );
        let rows = self.app.open_palette_rows();
        self.key(egui::Key::Escape, egui::Modifiers::NONE);
        assert_eq!(
            self.app.open_overlay(),
            None,
            "escape left the palette open"
        );
        rows
    }

    /// The banner titled `title`'s body, if one is up.
    fn banner(&self, title: &str) -> Option<String> {
        self.app
            .notifications()
            .iter()
            .find(|n| n.title == title)
            .map(|n| n.body.clone().unwrap_or_default())
    }

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
fn removing_a_typed_sql_step_is_a_draft_until_save() {
    let mut win = Window::opened("remove_draft");
    let step = win.saved_grouping();
    let model = win.model_file(&step);
    assert!(model.is_file(), "the step's SQL file is written");
    // A comment in the manifest that is not the step's: the removal keeps it.
    let commented = format!("# kept by hand\n{}", win.manifest());
    std::fs::write(win.dir().join("arcform.yaml"), &commented).expect("the manifest writes");
    win.settle();

    win.select_made_by(&step);
    win.d_d();

    let row = win
        .step_row(&step)
        .unwrap_or_else(|| panic!("the removed step stays on the spine: {:?}", win.labels()));
    assert!(row.struck, "the removed step's row is struck through");
    assert!(
        win.marked(),
        "the removal marks the title: {}",
        win.app.title()
    );
    assert!(
        win.manifest_steps().contains(&step) && model.is_file(),
        "the removal is a draft until Cmd+S"
    );

    win.cmd_s();

    assert!(
        !win.manifest_steps().contains(&step),
        "Cmd+S removed {step} from arcform.yaml: {:?}; {}",
        win.manifest_steps(),
        win.said()
    );
    assert!(!model.exists(), "and deleted its SQL file");
    assert!(
        win.manifest().starts_with("# kept by hand\n"),
        "and kept the manifest's comments: {}",
        win.manifest()
    );
    assert!(!win.marked(), "and took the mark off: {}", win.app.title());
    assert!(
        win.step_row(&step).is_none(),
        "and the step is off the spine: {:?}",
        win.labels()
    );
}

#[test]
fn d_d_again_puts_a_removed_step_back() {
    let mut win = Window::opened("remove_undo");
    let step = win.saved_grouping();
    win.select_made_by(&step);

    win.d_d();
    assert!(win.marked() && win.step_row(&step).is_some_and(|r| r.struck));
    win.d_d();

    assert!(
        win.step_row(&step).is_some_and(|r| !r.struck),
        "the step is put back: {:?}",
        win.labels()
    );
    assert!(
        !win.marked(),
        "with nothing left unsaved: {}",
        win.app.title()
    );
}

#[test]
fn a_load_step_and_the_model_step_cannot_be_removed() {
    let mut win = Window::opened("remove_fixed");
    let step = win.saved_grouping();
    let before = win.manifest();
    win.graph_to_canvas();

    let fixed: Vec<String> = win
        .rows()
        .into_iter()
        .filter(|r| r.role == SpineRole::Step && r.label != step)
        .map(|r| r.label)
        .collect();
    let load = format!("load_{}", win.name);
    assert!(
        fixed.contains(&load),
        "the spine draws the load: {:?}",
        win.labels()
    );
    for other in &fixed {
        assert!(
            *other == load || other.starts_with("silver_") || other.starts_with("failed_"),
            "a step the Protocol was made with: {other}"
        );
        win.select_made_by(other);
        assert!(
            !win.palette_rows().iter().any(|r| r == "remove-step"),
            "the palette offers no removal of {other}"
        );
        win.d_d();
        assert!(
            win.step_row(other).is_some_and(|r| !r.struck) && !win.marked(),
            "d d on {other} removes nothing"
        );
    }
    assert_eq!(win.manifest(), before);

    // The step kept from typed SQL is offered it.
    win.select_made_by(&step);
    assert!(
        win.palette_rows().iter().any(|r| r == "remove-step"),
        "the palette offers the removal of {step}"
    );
}

#[test]
fn a_step_others_read_is_refused_naming_them() {
    let mut win = Window::opened("remove_read");
    let grouped = win.saved_grouping();
    let first = win.saved_statement(&format!(
        "select house_age, n from {grouped} order by n desc"
    ));
    let second = win.saved_statement(&format!("select house_age from {grouped} where n > 10"));
    let before = win.manifest();

    win.select_made_by(&grouped);
    win.d_d();

    let banner = win
        .app
        .notifications()
        .iter()
        .find(|n| n.title == brightfield_shell::window::REMOVE_STEP_REFUSED)
        .unwrap_or_else(|| panic!("no refusal was said: {}", win.said()))
        .body
        .clone()
        .unwrap_or_default();
    for reader in [&first, &second] {
        assert!(
            banner.contains(reader.as_str()),
            "the refusal names {reader}: {banner}"
        );
    }
    assert!(
        win.step_row(&grouped).is_some_and(|r| !r.struck) && !win.marked(),
        "nothing is removed"
    );
    win.cmd_s();
    assert_eq!(win.manifest(), before, "and a Save writes nothing");

    // Removed first, the readers no longer refuse it, and one Save takes all three out.
    win.select_made_by(&first);
    win.d_d();
    win.select_made_by(&second);
    win.d_d();
    win.select_made_by(&grouped);
    win.d_d();
    assert!(
        win.step_row(&grouped).is_some_and(|r| r.struck),
        "{grouped} is removed once what reads it is: {}",
        win.said()
    );
    // A reader is not put back while what it reads is removed.
    win.select_made_by(&first);
    win.d_d();
    assert!(
        win.app.notifications().iter().any(|n| n.title
            == brightfield_shell::window::PUT_BACK_REFUSED
            && n.body
                .as_deref()
                .is_some_and(|b| b.contains(grouped.as_str()))),
        "the put-back of {first} is refused, naming {grouped}: {}",
        win.said()
    );
    assert!(win.step_row(&first).is_some_and(|r| r.struck));
    win.cmd_s();
    for step in [&grouped, &first, &second] {
        assert!(
            !win.manifest_steps().contains(step),
            "{step} is out of the manifest: {:?}; {}",
            win.manifest_steps(),
            win.said()
        );
    }
}

#[test]
fn close_without_saving_keeps_the_step() {
    let mut win = Window::opened("remove_close");
    let step = win.saved_grouping();
    let model = win.model_file(&step);
    let before = win.manifest();
    win.select_made_by(&step);
    win.d_d();
    assert!(win.marked());

    win.frame(Vec::new(), true);
    win.settle();
    assert_eq!(
        win.app.open_overlay(),
        Some(QUESTION),
        "the close asks first"
    );
    let edits = win
        .app
        .close_question()
        .expect("the question")
        .edits
        .clone();
    assert!(
        edits.iter().any(|e| e.contains(&step)),
        "the question lists the removal: {edits:?}"
    );
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(
        win.app.front_door_is_live(),
        "D goes to the door: {}",
        win.said()
    );

    assert_eq!(win.manifest(), before, "the step is still in arcform.yaml");
    assert!(model.is_file(), "and its SQL file is still there");

    // Opened again, the step is on the spine, and the version the close kept names the removal.
    win.app.open_queue().push(OpenRequest::Path(win.dir()));
    win.settle();
    win.assert_holds();
    win.graph_to_canvas();
    assert!(
        win.step_row(&step).is_some_and(|r| !r.struck),
        "the step is on the spine again: {:?}",
        win.labels()
    );
    let Listing::Listed { rows, .. } = win.versions_listed() else {
        panic!("the Versions panel lists nothing: {}", win.said());
    };
    let row = rows
        .iter()
        .find(|r| r.kind == CLOSED_UNSAVED)
        .unwrap_or_else(|| panic!("no closed unsaved version: {rows:?}"));
    assert!(
        row.changed.contains(&format!("Removed step \u{b7} {step}")),
        "the version names the removal: {:?}",
        row.changed
    );
}

/// The step text an agent might add to the manifest: `entry` inserted as a step before the step
/// `before`, at the same indent as the manifest's own step entries.
fn with_step_before(manifest: &str, before: &str, entry: &[&str]) -> String {
    let mut out = String::new();
    let mut done = false;
    for line in manifest.lines() {
        if !done && line.trim_start() == format!("- name: {before}") {
            let indent = &line[..line.len() - line.trim_start().len()];
            for (i, part) in entry.iter().enumerate() {
                let lead = if i == 0 { "- " } else { "  " };
                out.push_str(&format!("{indent}{lead}{part}\n"));
            }
            done = true;
        }
        out.push_str(line);
        out.push('\n');
    }
    assert!(done, "the manifest has no step {before}: {manifest}");
    out
}

/// A runner that, the first time it is asked to edit the Protocol, puts `inject` in place of
/// `arcform.yaml` first, as an agent writing between Brightfield's read and arc's would, and hands
/// everything to the real runner. `after` is shell run after that edit, as the agent writing once
/// more before Brightfield reads the manifest again.
#[cfg(unix)]
fn injecting(dir: &Path, inject: &Path, after: &str) -> Runner {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("injecting.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nedit=\ncase \"$*\" in\n  *edit-protocol*) if [ -f '{inject}' ]; then cp '{inject}' arcform.yaml; rm '{inject}'; edit=1; fi;;\nesac\n'{runner}' \"$@\"\nstatus=$?\nif [ -n \"$edit\" ]; then\n{after}  :\nfi\nexit $status\n",
            runner = env!("CARGO_BIN_EXE_brightfield-shell"),
            inject = inject.display()
        ),
    )
    .expect("the script writes");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("the script is executable");
    Runner::at(script)
}

/// The agent's write after arc's edit: a comment line, so the file is no longer what arc wrote.
const AGAIN: &str = "  echo '# the agent again' >> arcform.yaml\n";

#[cfg(unix)]
#[test]
fn a_removal_is_not_applied_when_the_manifest_changes_under_it() {
    let shims = scratch().join("shims").join("changed");
    std::fs::create_dir_all(&shims).expect("a shim folder");
    let inject = shims.join("agent.yaml");
    let mut win = Window::opened_with("remove_changed", injecting(&shims, &inject, ""));
    let step = win.saved_grouping();
    let model = win.model_file(&step);
    // What an agent writes between the read that finds the step and arc's delete: a step of its
    // own before it, so the position read names the agent's step.
    let changed = with_step_before(
        &win.manifest(),
        &step,
        &["name: agent_note", "command: echo agent"],
    );
    std::fs::write(&inject, &changed).expect("the agent's manifest writes");
    win.select_made_by(&step);
    win.d_d();

    win.cmd_s();

    assert!(!inject.exists(), "the agent's write happened");
    let said = win
        .banner("Could not remove the step")
        .unwrap_or_else(|| panic!("no banner said why: {}", win.said()));
    assert_eq!(said, brightfield_shell::operation::CHANGED_MEANWHILE);
    assert_eq!(
        win.manifest(),
        changed,
        "the manifest is the agent's, with both steps in it"
    );
    assert!(model.is_file(), "and the step's SQL file is kept");
    assert!(win.marked(), "the removal is still a draft");
}

#[test]
fn a_statement_that_reads_a_removed_step_is_not_kept() {
    let mut win = Window::opened("remove_then_keep");
    let step = win.saved_grouping();
    win.select_made_by(&step);
    win.d_d();
    // The graph chip again puts back the dashboard the graph was reached from.
    let chip = win
        .app
        .spine_rows()
        .iter()
        .find_map(|r| r.chip)
        .expect("the graph chip");
    win.click(chip.rect.center());
    assert!(!win.app.graph_on_canvas());

    win.key(egui::Key::Space, egui::Modifiers::NONE);
    win.type_text(&format!("select house_age from {step} where n > 10"));
    assert!(win.app.finish_typed_reads(Duration::from_secs(60)));
    win.key(egui::Key::Enter, egui::Modifiers::COMMAND);

    let said = win
        .banner(brightfield_shell::window::NOT_KEPT_READS_REMOVED)
        .unwrap_or_else(|| panic!("no banner said why: {}", win.said()));
    assert!(said.contains(&step), "it names {step}: {said}");
    assert!(
        win.app.chart_doc().typed_steps().is_empty(),
        "nothing is kept"
    );
}

#[test]
fn a_reader_an_agent_adds_before_save_refuses_the_removal() {
    let mut win = Window::opened("remove_agent_reader");
    let step = win.saved_grouping();
    win.select_made_by(&step);
    win.d_d();
    assert!(win.marked());

    // An agent adds a step that reads it, after the removal was drafted.
    std::fs::write(
        win.dir().join("models").join("agent_reader.sql"),
        format!("CREATE OR REPLACE TABLE \"agent_reader\" AS\nselect * from {step}\n"),
    )
    .expect("the agent's model writes");
    let manifest = format!(
        "{}- name: agent_reader\n  sql: models/agent_reader.sql\n",
        win.manifest()
    );
    std::fs::write(win.dir().join("arcform.yaml"), &manifest).expect("the agent's manifest writes");
    assert!(
        brightfield_protocol::parse_manifest_str(&manifest).is_ok(),
        "the agent's manifest loads: {manifest}"
    );
    win.settle();

    win.cmd_s();

    let said = win
        .banner("Could not remove the step")
        .unwrap_or_else(|| panic!("no banner said why: {}", win.said()));
    assert!(said.contains("agent_reader"), "it names the reader: {said}");
    assert_eq!(win.manifest(), manifest, "the manifest is unchanged");
    assert!(win.marked(), "the removal is still a draft");
}

#[cfg(unix)]
#[test]
fn a_sql_file_that_cannot_be_deleted_is_left_with_a_notice() {
    use std::os::unix::fs::PermissionsExt;
    let mut win = Window::opened("remove_left_behind");
    let step = win.saved_grouping();
    let model = win.model_file(&step);
    win.select_made_by(&step);
    win.d_d();
    let models = model.parent().expect("the models folder").to_path_buf();
    std::fs::set_permissions(&models, std::fs::Permissions::from_mode(0o555))
        .expect("the models folder is made read-only");

    win.cmd_s();
    let _ = std::fs::set_permissions(&models, std::fs::Permissions::from_mode(0o755));

    assert!(
        !win.manifest_steps().contains(&step),
        "the step is out of the manifest: {}",
        win.said()
    );
    assert!(model.is_file(), "its file is left behind");
    let said = win
        .banner(brightfield_shell::window::SQL_FILE_LEFT_BEHIND)
        .unwrap_or_else(|| panic!("no notice names the file: {}", win.said()));
    assert!(said.contains(&model.display().to_string()), "{said}");
    assert!(!win.marked(), "the removal is saved: {}", win.app.title());
}

#[test]
fn d_d_on_the_spine_removes_with_the_dashboard_on_the_canvas() {
    let mut win = Window::opened("remove_from_outline");
    let step = win.saved_grouping();
    assert!(!win.app.graph_on_canvas(), "the canvas holds no graph");
    win.click_made_by(&step);
    assert!(
        !win.app.graph_on_canvas(),
        "the click put no graph on the canvas"
    );

    win.d_d();

    assert!(
        win.step_row(&step).is_some_and(|r| r.struck),
        "{step} is removed: {:?}",
        win.labels()
    );
    assert!(win.marked());
    // The Versions panel's row for what is not yet saved names the removal.
    win.versions_listed();
    let words = win
        .app
        .chart_doc()
        .versions()
        .unsaved_words()
        .map(str::to_owned);
    assert!(
        words
            .as_deref()
            .is_some_and(|w| w.contains(&format!("Removed step \u{b7} {step}"))),
        "{words:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_wrong_step_removed_that_cannot_be_put_back_is_named() {
    let shims = scratch().join("shims").join("again");
    std::fs::create_dir_all(&shims).expect("a shim folder");
    let inject = shims.join("agent.yaml");
    let mut win = Window::opened_with("remove_again", injecting(&shims, &inject, AGAIN));
    let step = win.saved_grouping();
    let model = win.model_file(&step);
    let changed = with_step_before(
        &win.manifest(),
        &step,
        &["name: agent_note", "command: echo agent"],
    );
    std::fs::write(&inject, &changed).expect("the agent's manifest writes");
    win.select_made_by(&step);
    win.d_d();

    win.cmd_s();

    assert!(!inject.exists(), "the agent's write happened");
    let said = win
        .banner(brightfield_shell::window::OTHER_STEP_REMOVED)
        .unwrap_or_else(|| panic!("no banner said another step went: {}", win.said()));
    assert!(
        said.contains("agent_note"),
        "it names the step removed instead: {said}"
    );
    assert!(said.contains(&step), "and the step still there: {said}");
    assert!(
        win.banner("Could not remove the step").is_none(),
        "and does not say nothing was removed: {}",
        win.said()
    );
    assert!(
        win.manifest_steps().contains(&step)
            && !win.manifest_steps().contains(&"agent_note".to_string()),
        "arc took out the agent's step and left {step}: {:?}",
        win.manifest_steps()
    );
    assert!(model.is_file(), "the step's SQL file is kept");
    assert!(win.marked(), "the removal is still a draft");
}

#[cfg(unix)]
#[test]
fn an_agent_write_after_the_right_step_is_removed_keeps_both() {
    let shims = scratch().join("shims").join("late");
    std::fs::create_dir_all(&shims).expect("a shim folder");
    let inject = shims.join("agent.yaml");
    let late = "  printf -- '- name: agent_late\\n  command: echo late\\n' >> arcform.yaml\n";
    let mut win = Window::opened_with("remove_late", injecting(&shims, &inject, late));
    let step = win.saved_grouping();
    let model = win.model_file(&step);
    // Nothing changes before arc's delete: the copy is the manifest as it is.
    std::fs::write(&inject, win.manifest()).expect("the copy writes");
    win.select_made_by(&step);
    win.d_d();

    win.cmd_s();

    assert!(!inject.exists(), "the shim ran");
    let steps = win.manifest_steps();
    assert!(
        !steps.contains(&step) && steps.contains(&"agent_late".to_string()),
        "the step is out and the agent's is kept: {steps:?}; {}",
        win.said()
    );
    assert!(!model.exists(), "the step's SQL file is deleted");
    for title in [
        brightfield_shell::window::OTHER_STEP_REMOVED,
        "Could not remove the step",
    ] {
        assert!(win.banner(title).is_none(), "{title}: {}", win.said());
    }
    assert!(!win.marked(), "the removal is saved: {}", win.app.title());
}

#[cfg(unix)]
#[test]
fn a_manifest_that_no_longer_loads_after_the_delete_is_named_from_the_history() {
    let shims = scratch().join("shims").join("broken");
    std::fs::create_dir_all(&shims).expect("a shim folder");
    let inject = shims.join("agent.yaml");
    let broken = "  echo 'not: [valid' >> arcform.yaml\n";
    let mut win = Window::opened_with("remove_broken", injecting(&shims, &inject, broken));
    let step = win.saved_grouping();
    let model = win.model_file(&step);
    let changed = with_step_before(
        &win.manifest(),
        &step,
        &["name: agent_note", "command: echo agent"],
    );
    std::fs::write(&inject, &changed).expect("the agent's manifest writes");
    win.select_made_by(&step);
    win.d_d();

    win.cmd_s();

    assert!(!inject.exists(), "the shim ran");
    let said = win
        .banner(brightfield_shell::window::OTHER_STEP_REMOVED)
        .unwrap_or_else(|| panic!("no banner said another step went: {}", win.said()));
    assert!(
        said.contains("the step agent_note"),
        "it names arc's step: {said}"
    );
    assert!(model.is_file(), "the step's SQL file is kept");
}
