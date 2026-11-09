//! Cmd+S saves the Protocol from anywhere in the window: with nothing holding the keys, with the
//! grid, the navigator or the inspector focused, and while a text field other than the YAML editor
//! holds the keyboard. In the YAML editor it saves the editor's buffer first, then the Protocol. A
//! save into a one-source Protocol shows the recorded step on the spine, and the next run's record
//! is taken onto the ledger strip.
//!
//! Every window is headless, with the `brightfield-shell` binary cargo built as its runner, and the
//! config directory, the Protocols folder and arc's directories point at a scratch folder of this
//! file's, so nothing is written under the home directory.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime};

use brightfield_shell::design::Mode;
use brightfield_shell::open_queue::OpenRequest;
use brightfield_shell::run::Runner;
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_workbench::arrangement::LEDGER_RAIL;
use brightfield_workbench::PaneKey;

/// The condition every test keeps on the grid, and the word the step arc records for it carries.
const CONDITION: &str = "house_age > 28";
const FILTER_STEP_SUFFIX: &str = "_filtered_by_house_age";

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-cmd-s-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "protocols", "input"] {
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

fn command() -> egui::Modifiers {
    egui::Modifiers {
        command: true,
        ..Default::default()
    }
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

fn button_at(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
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
        scratch();
        let csv = housing_copy(name);
        let mut win = Self {
            app: MeridianApp::headless(Boot::empty(), Mode::Light)
                .running_with(Some(Runner::at(env!("CARGO_BIN_EXE_brightfield-shell")))),
            ctx: egui::Context::default(),
            name: name.to_owned(),
            time: 0.0,
        };
        win.app.open_queue().push(OpenRequest::Path(csv));
        win.settle();
        assert_eq!(
            win.app.own_protocol().map(|p| p.dir.clone()),
            Some(win.dir()),
            "the window holds the Protocol it made: {}",
            win.said()
        );
        win
    }

    fn dir(&self) -> PathBuf {
        scratch().join("protocols").join(&self.name)
    }

    fn manifest(&self) -> String {
        std::fs::read_to_string(self.dir().join("arcform.yaml")).expect("the manifest reads")
    }

    fn filter_step(&self) -> String {
        format!("{}{FILTER_STEP_SUFFIX}", self.name)
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        self.time += 1.0;
        let modifiers = events
            .iter()
            .find_map(|e| match e {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440.0, 900.0),
            )),
            time: Some(self.time),
            modifiers,
            events,
            ..Default::default()
        };
        let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    /// Press Cmd+S, as one frame, then let the window settle.
    fn cmd_s(&mut self) {
        self.run(vec![press(egui::Key::S, command())]);
        self.settle();
    }

    fn click(&mut self, at: egui::Pos2) {
        self.run(vec![
            egui::Event::PointerMoved(at),
            button_at(at, true),
            button_at(at, false),
        ]);
        self.settle();
    }

    fn type_text(&mut self, text: &str) {
        self.run(vec![egui::Event::Text(text.to_owned())]);
        self.settle();
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
        assert!(
            self.app.title().contains(UNSAVED_MARK),
            "the kept filter marks the title unsaved: {}",
            self.app.title()
        );
        assert!(
            !self.manifest().contains(&self.filter_step()),
            "the filter is a draft until Cmd+S"
        );
    }

    /// Assert the filter was recorded into the Protocol and the title lost its mark.
    fn assert_saved(&self, from: &str) {
        let manifest = self.manifest();
        assert!(
            manifest.contains(&self.filter_step()),
            "Cmd+S {from} recorded the filter: {manifest}; {}",
            self.said()
        );
        assert!(
            !self.app.title().contains(UNSAVED_MARK),
            "Cmd+S {from} took the unsaved mark off the title: {}",
            self.app.title()
        );
    }

    fn said(&self) -> String {
        self.app
            .notifications()
            .iter()
            .map(|n| format!("{}: {}", n.title, n.body.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("; ")
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
            self.app
                .focus_pane(PaneKey::new(brightfield_shell::editor::EDITOR)),
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

/// The modification time of `arcform.yaml` and of each file under `panels/` and `models/`.
fn modification_times(dir: &Path) -> Vec<(PathBuf, SystemTime)> {
    let mut files = vec![dir.join("arcform.yaml")];
    for sub in ["panels", "models"] {
        if let Ok(entries) = std::fs::read_dir(dir.join(sub)) {
            files.extend(entries.flatten().map(|e| e.path()));
        }
    }
    files.sort();
    files
        .into_iter()
        .map(|f| {
            let modified = std::fs::metadata(&f)
                .and_then(|m| m.modified())
                .expect("a modification time");
            (f, modified)
        })
        .collect()
}

#[test]
fn cmd_s_on_the_dashboard_records_a_kept_filter() {
    let mut win = Window::opened("cmd_s_dashboard");
    win.keep_filter();
    assert!(
        win.ctx.memory(|m| m.focused()).is_none(),
        "no text field holds the keyboard"
    );

    win.cmd_s();

    win.assert_saved("on the dashboard");
}

#[test]
fn cmd_s_saves_from_the_grid_the_navigator_and_the_inspector() {
    for (name, pane) in [
        (
            "cmd_s_grid",
            PaneKey::new(brightfield_shell::data_grid::DATA),
        ),
        (
            "cmd_s_navigator",
            PaneKey::new(brightfield_shell::protocol::OUTLINE),
        ),
        (
            "cmd_s_inspector",
            PaneKey::new(brightfield_shell::protocol::INSPECTOR),
        ),
    ] {
        let mut win = Window::opened(name);
        win.keep_filter();
        assert!(win.app.focus_pane(pane), "{pane:?} is in the window");
        win.settle();
        assert_eq!(win.app.focused_pane(), Some(pane));

        win.cmd_s();

        win.assert_saved(&format!("with {pane:?} focused"));
    }
}

#[test]
fn cmd_s_while_typing_sql_saves_the_protocol_and_types_nothing() {
    let mut win = Window::opened("cmd_s_typing");
    win.keep_filter();
    win.run(vec![press(egui::Key::Space, egui::Modifiers::NONE)]);
    win.settle();
    let statement = "SELECT * FROM cmd_s_typing";
    win.type_text(statement);
    assert_eq!(
        win.app.typed_statement(),
        Some(statement),
        "the statement form holds what was typed"
    );

    win.cmd_s();

    win.assert_saved("while typing SQL");
    assert_eq!(
        win.app.typed_statement(),
        Some(statement),
        "Cmd+S typed nothing into the statement"
    );
}

#[test]
fn cmd_s_in_the_editor_saves_the_buffer_then_the_protocol() {
    let mut win = Window::opened("cmd_s_editor");
    win.keep_filter();
    win.click_into_the_editor();
    let spec = win
        .app
        .chart_doc()
        .spec_path
        .clone()
        .expect("the editor's buffer has a file");
    let before = std::fs::read_to_string(&spec).expect("the spec reads");
    win.type_text("# typed in the editor\n");

    win.cmd_s();

    let after = std::fs::read_to_string(&spec).expect("the spec reads");
    assert_ne!(after, before, "Cmd+S in the editor saved its buffer");
    assert!(
        after.contains("# typed in the editor"),
        "the buffer's text is in the file: {after}"
    );
    let manifest = win.manifest();
    assert!(
        manifest.contains(&win.filter_step()),
        "…and then the Protocol: {manifest}; {}",
        win.said()
    );
}

#[test]
fn cmd_s_with_nothing_pending_writes_nothing() {
    let mut win = Window::opened("cmd_s_nothing");
    assert!(!win.app.title().contains(UNSAVED_MARK));
    let before = modification_times(&win.dir());
    // A file written now would carry a later time than any written before this, on a filesystem
    // whose clock is coarser than the test.
    std::thread::sleep(Duration::from_millis(1100));

    win.cmd_s();

    assert_eq!(modification_times(&win.dir()), before);
}

#[test]
fn cmd_s_on_a_one_source_protocol_shows_the_step_on_the_spine() {
    let mut win = Window::opened("cmd_s_spine");
    win.keep_filter();

    win.cmd_s();

    win.assert_saved("on a one-source Protocol");
    let step = win.filter_step();
    let drawn: Vec<String> = win
        .app
        .spine_rows()
        .iter()
        .map(|r| r.label.clone())
        .collect();
    assert!(
        drawn.contains(&step),
        "the spine shows the recorded step {step}: {drawn:?}"
    );

    // Run it: the record of a run of the Protocol's steps is taken onto the strip.
    win.run(vec![press(egui::Key::Enter, command())]);
    assert!(
        win.app.run_in_progress(),
        "cmd-enter started a run: {}",
        win.said()
    );
    let deadline = Instant::now() + Duration::from_secs(120);
    while win.app.run_outstanding() {
        assert!(Instant::now() < deadline, "the run did not land");
        win.run(Vec::new());
        std::thread::sleep(Duration::from_millis(5));
    }
    win.settle();
    assert!(
        win.app.protocol_model().run().is_some(),
        "the strip took on the run's record: {}",
        win.said()
    );
    assert!(
        win.app.protocol_model().step_ran(&step),
        "the record says the recorded step ran: {}",
        win.said()
    );
}

#[test]
fn cmd_s_saves_an_edited_buffer_when_the_editor_does_not_have_the_keyboard() {
    let mut win = Window::opened("cmd_s_buffer_unfocused");
    win.keep_filter();
    win.click_into_the_editor();
    let spec = win
        .app
        .chart_doc()
        .spec_path
        .clone()
        .expect("the editor's buffer has a file");
    win.type_text("# typed, then left\n");
    // The keyboard and the focus go elsewhere: the grid holds the focus, and no text field the
    // keys.
    win.ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    assert!(win
        .app
        .focus_pane(PaneKey::new(brightfield_shell::data_grid::DATA)));
    win.settle();
    assert!(win.ctx.memory(|m| m.focused()).is_none());

    win.cmd_s();

    let after = std::fs::read_to_string(&spec).expect("the spec reads");
    assert!(
        after.contains("# typed, then left"),
        "Cmd+S away from the editor saved its buffer: {after}"
    );
    win.assert_saved("away from an edited editor");
}

#[test]
fn a_second_cmd_s_keeps_the_made_with_and_the_recorded_steps() {
    let mut win = Window::opened("cmd_s_twice");
    win.keep_filter();
    win.cmd_s();
    win.assert_saved("the first time");
    let first = win.manifest();

    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some(CONDITION.to_owned()),
            order: Some("median_income".to_owned()),
        });
    win.settle();
    assert!(
        win.app.title().contains(UNSAVED_MARK),
        "the order is a draft"
    );

    win.cmd_s();

    assert!(
        !win.app.title().contains(UNSAVED_MARK),
        "the second Cmd+S took the mark off: {}",
        win.said()
    );
    let manifest = win.manifest();
    let steps: Vec<String> = brightfield_protocol::parse_manifest_str(&manifest)
        .expect("the manifest parses")
        .steps
        .into_iter()
        .map(|s| s.name)
        .collect();
    let first_steps: Vec<String> = brightfield_protocol::parse_manifest_str(&first)
        .expect("the first manifest parses")
        .steps
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert!(
        first_steps.iter().all(|s| steps.contains(s)),
        "every step the first save left is still there: {first_steps:?} in {steps:?}"
    );
    assert!(
        steps.contains(&win.filter_step()),
        "the recorded filter: {steps:?}"
    );
    assert!(
        steps.iter().any(|s| s.contains("median_income")),
        "and the recorded order: {steps:?}"
    );
    assert!(
        steps.len() > first_steps.len(),
        "the second save added a step: {steps:?}"
    );
    assert!(
        first_steps.iter().any(|s| s != &win.filter_step()),
        "the steps the Protocol was made with are among them: {first_steps:?}"
    );
}

#[test]
fn cmd_s_on_a_protocol_opened_as_a_folder_of_two_tables() {
    scratch();
    let dir = scratch().join("input").join("cmd_s_folder");
    std::fs::create_dir_all(&dir).expect("an input folder");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv"),
        dir.join("cmd_s_houses.csv"),
    )
    .expect("the housing sample copies");
    std::fs::write(
        dir.join("regions.csv"),
        "region,manager\nnorth,Ann\nsouth,Bo\n",
    )
    .expect("the second table writes");
    let mut win = Window {
        app: MeridianApp::headless(Boot::empty(), Mode::Light)
            .running_with(Some(Runner::at(env!("CARGO_BIN_EXE_brightfield-shell")))),
        ctx: egui::Context::default(),
        name: "cmd_s_folder".to_owned(),
        time: 0.0,
    };
    win.app.open_queue().push(OpenRequest::Path(dir));
    win.settle();
    assert!(
        win.app.model_pane().is_some(),
        "two tables open the model: {}",
        win.said()
    );
    assert!(!win.app.title().contains(UNSAVED_MARK));
    // Discovery's full pass rewrites the folder's steps when it lands; it is let land first, so
    // what is measured is Cmd+S alone.
    discovery_landed(&mut win);

    // On the model, nothing is a draft: making the Protocol and its joins are kept at once.
    let before = modification_times(&win.dir());
    std::thread::sleep(Duration::from_millis(1100));
    win.cmd_s();
    assert_eq!(
        modification_times(&win.dir()),
        before,
        "Cmd+S on the model writes nothing"
    );
    assert!(win.said().is_empty(), "and says nothing: {}", win.said());

    // From a table's grid, a kept filter is a draft that Cmd+S saves.
    let ctx = win.ctx.clone();
    assert!(
        win.app.open_model_in_grid(
            &ctx,
            &brightfield_render::model_scene::Selection::Dataset("cmd_s_houses".into())
        ),
        "the table opens in the grid: {}",
        win.said()
    );
    win.settle();
    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some(CONDITION.to_owned()),
            order: None,
        });
    win.settle();
    assert!(
        win.app.title().contains(UNSAVED_MARK),
        "the filter is a draft"
    );

    win.cmd_s();

    assert!(
        !win.app.title().contains(UNSAVED_MARK),
        "Cmd+S from the folder Protocol's grid saved the filter: {}",
        win.said()
    );
    assert!(win.said().is_empty(), "with no refusal: {}", win.said());
    let manifest = win.manifest();
    assert!(
        manifest.contains("cmd_s_houses_filtered_by_house_age"),
        "the step is recorded into the Protocol's own manifest: {manifest}"
    );
    assert!(
        !win.dir().join(".grid").join("arcform.yaml").exists(),
        "and nothing manifest-like is left beside the grid's rows"
    );
    let step = "cmd_s_houses_filtered_by_house_age";
    let drawn: Vec<String> = win
        .app
        .spine_rows()
        .iter()
        .map(|r| r.label.clone())
        .collect();
    assert!(
        drawn.iter().any(|l| l == step),
        "the spine shows the recorded step after the Save: {drawn:?}"
    );

    // Run it: the record of the run of the Protocol's steps is taken onto the strip.
    win.run(vec![press(egui::Key::Enter, command())]);
    assert!(
        win.app.run_in_progress(),
        "cmd-enter started a run: {}",
        win.said()
    );
    let deadline = Instant::now() + Duration::from_secs(120);
    while win.app.run_outstanding() {
        assert!(Instant::now() < deadline, "the run did not land");
        win.run(Vec::new());
        std::thread::sleep(Duration::from_millis(5));
    }
    win.settle();
    assert!(
        win.app.protocol_model().run().is_some(),
        "the strip took on the run's record: {}",
        win.said()
    );
    assert!(
        win.app.protocol_model().step_ran(step),
        "the record says the recorded step ran: {}",
        win.said()
    );
    assert_eq!(
        grid_files(&win.dir()),
        ["cmd_s_houses.parquet"],
        "the grid's folder holds its rows alone"
    );
}

/// The files the grid's copies of a folder Protocol's rows are kept in, by name.
fn grid_files(dir: &Path) -> Vec<String> {
    fn walk(at: &Path, base: &Path, into: &mut Vec<String>) {
        if let Ok(entries) = std::fs::read_dir(at) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, base, into);
                } else {
                    into.push(
                        path.strip_prefix(base)
                            .expect("under the folder")
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
            }
        }
    }
    let base = dir.join(".grid");
    let mut found = Vec::new();
    walk(&base, &base, &mut found);
    found.sort();
    found
}

/// Wait for the model pane's discovery pass to land.
fn discovery_landed(win: &mut Window) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while win.app.model_pane().is_some_and(|pane| pane.discovering()) {
        assert!(Instant::now() < deadline, "discovery's pass did not land");
        win.run(Vec::new());
        std::thread::sleep(Duration::from_millis(20));
    }
    win.settle();
}

/// A window over a folder Protocol of orders and the stores they were taken in, named `name`,
/// with the grid of the model's join between them open, and the join's name.
fn joined_grid(name: &str) -> (Window, String) {
    scratch();
    let dir = scratch().join("input").join(name);
    std::fs::create_dir_all(&dir).expect("an input folder");
    let mut orders = String::from("order_id,store_id,amount\n");
    for i in 1..=60 {
        orders.push_str(&format!("{i},{},{}\n", i % 5 + 1, i * 3));
    }
    std::fs::write(dir.join("orders.csv"), orders).expect("the orders write");
    std::fs::write(
        dir.join("stores.csv"),
        "store_id,city\n1,Perth\n2,Darwin\n3,Hobart\n4,Cairns\n5,Broome\n",
    )
    .expect("the stores write");
    let mut win = Window {
        app: MeridianApp::headless(Boot::empty(), Mode::Light)
            .running_with(Some(Runner::at(env!("CARGO_BIN_EXE_brightfield-shell")))),
        ctx: egui::Context::default(),
        name: name.to_owned(),
        time: 0.0,
    };
    win.app.open_queue().push(OpenRequest::Path(dir));
    win.settle();
    discovery_landed(&mut win);
    let join = {
        let pane = win
            .app
            .model_pane()
            .unwrap_or_else(|| panic!("two tables open the model: {}", win.said()));
        pane.model
            .relationships
            .first()
            .unwrap_or_else(|| panic!("the model joins orders to stores: {:?}", pane.model))
            .name
            .clone()
    };
    let ctx = win.ctx.clone();
    assert!(
        win.app.open_model_in_grid(
            &ctx,
            &brightfield_render::model_scene::Selection::Relationship(join.clone())
        ),
        "the join opens in the grid: {}",
        win.said()
    );
    win.settle();
    (win, join)
}

#[test]
fn a_joins_grid_writes_nothing_manifest_like_and_its_view_is_no_draft() {
    let (mut win, join) = joined_grid("cmd_s_joined");
    let before = modification_times(&win.dir());
    std::thread::sleep(Duration::from_millis(1100));

    win.cmd_s();

    assert_eq!(
        modification_times(&win.dir()),
        before,
        "an idle Cmd+S on a join's grid writes nothing"
    );
    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some("\"orders.amount\" > 30".to_owned()),
            order: None,
        });
    win.settle();
    assert!(
        !win.app.title().contains(UNSAVED_MARK),
        "a view on a join's rows is no draft: {}",
        win.app.title()
    );
    win.cmd_s();
    assert_eq!(
        modification_times(&win.dir()),
        before,
        "and Cmd+S writes nothing for it"
    );
    assert_eq!(
        grid_files(&win.dir()),
        [format!("{join}.parquet")],
        "the grid's folder holds the join's rows alone"
    );
    assert!(win.said().is_empty(), "nothing is refused: {}", win.said());
}

#[test]
fn run_on_a_just_opened_csv_shows_its_run_on_the_strip() {
    let mut win = Window::opened("run_fresh");
    // The open's own run is on the strip from the start.
    let opened_with = win
        .app
        .protocol_model()
        .run()
        .map(|r| r.run_id.clone())
        .unwrap_or_else(|| panic!("the open's run is on the strip: {}", win.said()));

    win.run(vec![press(egui::Key::Enter, command())]);
    assert!(
        win.app.run_in_progress(),
        "cmd-enter started a run: {}",
        win.said()
    );
    let deadline = Instant::now() + Duration::from_secs(120);
    while win.app.run_outstanding() {
        assert!(Instant::now() < deadline, "the run did not land");
        win.run(Vec::new());
        std::thread::sleep(Duration::from_millis(5));
    }
    win.settle();

    let ran = win.app.protocol_model().run().map(|r| r.run_id.clone());
    assert!(
        ran.is_some() && ran.as_deref() != Some(opened_with.as_str()),
        "the strip took on the new run of a Protocol with nothing recorded yet: {ran:?}; {}",
        win.said()
    );
}

#[test]
fn cmd_s_away_from_the_editor_never_writes_over_a_change_on_disk() {
    let mut win = Window::opened("cmd_s_buffer_conflict");
    win.click_into_the_editor();
    let spec = win
        .app
        .chart_doc()
        .spec_path
        .clone()
        .expect("the editor's buffer has a file");
    win.type_text("# typed in the editor\n");
    win.ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    assert!(win
        .app
        .focus_pane(PaneKey::new(brightfield_shell::data_grid::DATA)));
    win.settle();
    // Someone else changes the file while the buffer is unsaved.
    let external = format!(
        "{}# changed outside\n",
        std::fs::read_to_string(&spec).expect("the spec reads")
    );
    std::fs::write(&spec, &external).expect("the outside change writes");

    win.cmd_s();
    win.cmd_s();

    assert_eq!(
        std::fs::read_to_string(&spec).expect("the spec reads"),
        external,
        "two presses away from the editor did not write over the change"
    );
    assert!(
        win.app
            .notifications()
            .iter()
            .any(|n| n.title == "The YAML editor's buffer was not saved"
                && n.body
                    .as_deref()
                    .is_some_and(|b| b.contains("changed on disk"))),
        "a banner says why: {}",
        win.said()
    );
    assert!(
        win.app.title().contains(UNSAVED_MARK),
        "the buffer is still unsaved"
    );
}

#[test]
fn a_statement_typed_on_a_joins_grid_is_not_kept() {
    let (mut win, join) = joined_grid("cmd_s_join_typed");
    let manifest = win.manifest();

    win.run(vec![press(egui::Key::Space, egui::Modifiers::NONE)]);
    win.settle();
    win.type_text(&format!("select * from \"{join}\""));
    assert!(
        win.app.finish_typed_reads(Duration::from_secs(60)),
        "the read did not finish: {}",
        win.said()
    );
    win.run(vec![press(egui::Key::Enter, command())]);
    win.settle();

    assert!(
        win.app.chart_doc().typed_steps().is_empty(),
        "nothing is kept"
    );
    assert!(
        win.app
            .notifications()
            .iter()
            .any(|n| n.body.as_deref() == Some(brightfield_shell::window::NOT_KEPT_ON_A_JOIN_BODY)),
        "a banner says why: {}",
        win.said()
    );
    assert!(
        !win.app.title().contains(UNSAVED_MARK),
        "nothing is unsaved"
    );
    win.cmd_s();
    assert_eq!(win.manifest(), manifest, "and nothing is recorded");
}

#[test]
fn run_on_a_joins_grid_runs_the_protocol_and_shows_its_run() {
    let (mut win, _) = joined_grid("cmd_s_join_run");
    let before = win.app.protocol_model().run().map(|r| r.run_id.clone());

    win.run(vec![press(egui::Key::Enter, command())]);
    assert!(
        win.app.run_in_progress(),
        "cmd-enter started a run: {}",
        win.said()
    );
    let deadline = Instant::now() + Duration::from_secs(120);
    while win.app.run_outstanding() {
        assert!(Instant::now() < deadline, "the run did not land");
        win.run(Vec::new());
        std::thread::sleep(Duration::from_millis(5));
    }
    win.settle();

    let ran = win.app.protocol_model().run().map(|r| r.run_id.clone());
    assert!(
        ran.is_some() && ran != before,
        "the strip shows the run of the Protocol: {ran:?}; {}",
        win.said()
    );
    assert_eq!(
        win.app.protocol_model().run().map(|r| r.protocol.clone()),
        Some("cmd_s_join_run".to_owned()),
        "the run is the Protocol's own"
    );
}

#[test]
fn a_joins_chart_is_saved_apart_from_the_tables_and_read_back() {
    let (mut win, join) = joined_grid("cmd_s_join_chart");
    // A table's panel of the same name as the join, which the join's chart must not touch.
    let panels = win.dir().join("panels");
    std::fs::create_dir_all(&panels).expect("the panels folder");
    let table_panel = brightfield_model::panel_capture::panel_file(&win.dir(), &join);
    std::fs::write(&table_panel, "# a table's panel\n").expect("the table's panel writes");
    let join_panel = brightfield_model::panel_capture::join_panel_file(&win.dir(), &join);

    let thrown = (0..8).any(|plot| {
        win.app.chart_doc_mut().set_plot_scale(
            plot,
            brightfield_spec::layout::PlotAxis::Y,
            brightfield_spec::layout::ScaleType::Log,
        )
    });
    assert!(thrown, "a tile took the log scale: {}", win.said());
    win.settle();
    assert!(
        win.app.title().contains(UNSAVED_MARK),
        "the chart edit is a draft"
    );

    win.cmd_s();

    assert!(
        !win.app.title().contains(UNSAVED_MARK),
        "Cmd+S saved the join's chart: {}",
        win.said()
    );
    let saved = std::fs::read_to_string(&join_panel).expect("the join's chart is written");
    assert!(saved.contains("log"), "{saved}");
    assert_eq!(
        std::fs::read_to_string(&table_panel).expect("the table's panel reads"),
        "# a table's panel\n",
        "the table's panel of the same name is its own"
    );
    assert_eq!(
        grid_files(&win.dir()),
        [format!("{join}.parquet")],
        "nothing is written beside the join's rows"
    );

    // Back to the model and into the join's grid again: the chart reads back from its file.
    assert!(win.app.back_to_model(), "{}", win.said());
    win.settle();
    let ctx = win.ctx.clone();
    assert!(win.app.open_model_in_grid(
        &ctx,
        &brightfield_render::model_scene::Selection::Relationship(join.clone())
    ));
    win.settle();
    assert_eq!(
        win.app
            .chart_doc()
            .spec_path
            .as_deref()
            .map(|p| std::path::absolute(p).unwrap()),
        Some(std::path::absolute(&join_panel).unwrap()),
        "the join's grid opens its saved chart"
    );
}
