//! The model pane holds its Protocol's spine: the model's step first, then the tables, built
//! again on a frame when the model's generation moves. Windows are headless, as in the model
//! pane's own tests, and what a test reads is the spine the window holds, not what was drawn.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use brightfield_shell::design::Mode;
use brightfield_shell::model_inspector::ModelAction;
use brightfield_shell::model_pane::ModelPane;
use brightfield_shell::protocol::{SpineRole, SpineRow, MODEL_STEP};
use brightfield_shell::run::Runner;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::arrangement;

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-spine-model-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "protocols"] {
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

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// The retail extract: eight CSVs, and the model written for them. The own-files fixture's
/// discovery proposes no join and its Protocol makes no silver tables before a model is saved,
/// so the spine would list no table of the model.
fn fixture() -> PathBuf {
    scratch();
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../brightfield-ossie/fixtures/retail_extract")
}

/// One headless window that opened the fixture folder and saved its written model.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    /// The frame clock, which a click needs to move on between frames.
    time: f64,
}

impl Window {
    fn over_fixture() -> Self {
        Self::open(&fixture(), true)
    }

    /// The engine's own-files fixture as discovery left it: no join, no written model saved.
    fn over_own_files() -> Self {
        scratch();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../brightfield-engine/tests/data/own_files");
        Self::open(&dir, false)
    }

    fn open(dir: &Path, save_written: bool) -> Self {
        let mut win = Self {
            app: MeridianApp::headless(Boot::empty(), Mode::Light).running_with(Some(runner())),
            ctx: egui::Context::default(),
            time: 0.0,
        };
        win.frame(Vec::new());
        win.app
            .open_path_knowing(&win.ctx, &dir.to_string_lossy(), &[]);
        // The open's full pass lands before the test looks, then the written model is saved over
        // the one discovery found, which makes the silver tables and the joins the spine reads.
        let pane = win
            .app
            .model_pane_mut()
            .expect("the folder opens into the model pane");
        pane.finish_discovery();
        if save_written {
            let text = std::fs::read_to_string(dir.join("model.ossie.yaml")).unwrap();
            pane.save_model(brightfield_ossie::read(&text).unwrap())
                .expect("the written model runs");
        }
        win.frame(Vec::new());
        // The open leaves the sources browser over the window, which would take every click.
        if win.app.sources_browser().is_some() {
            win.frame(vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            win.frame(Vec::new());
        }
        win
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 1.0;
        let raw = egui::RawInput {
            time: Some(self.time),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440.0, 900.0),
            )),
            events,
            ..Default::default()
        };
        let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
    }

    fn rows(&self) -> Vec<SpineRow> {
        self.app
            .model_spine_rows()
            .expect("a frame built the spine")
    }

    fn pane(&self) -> &ModelPane {
        self.app.model_pane().expect("the model pane is open")
    }
}

/// The one join of the written retail model between two tables the spine lists. The Protocol's
/// silver steps for the other tables are gathered into a failed family the spine does not list
/// as tables, so a join to one of them does not count in the model's row.
const LISTED_JOIN: &str = "loyalty_visits_customers";
const LISTED_TABLES: [&str; 2] = ["loyalty_visits", "customers"];

#[test]
fn the_model_pane_holds_its_protocols_spine_model_first() {
    let win = Window::over_fixture();
    let rows = win.rows();
    let model = rows
        .iter()
        .find(|r| r.role == SpineRole::Model && r.label == MODEL_STEP);
    assert!(model.is_some(), "no model row in {rows:?}");
    for table in LISTED_TABLES {
        assert!(
            rows.iter()
                .any(|r| r.role == SpineRole::Asset && r.label == table),
            "no row for {table} in {rows:?}"
        );
    }
    // The joined tables lead, ahead of the model's step.
    let at = |label: &str| rows.iter().position(|r| r.label == label).unwrap();
    assert!(at("customers") < at(MODEL_STEP) && at("loyalty_visits") < at(MODEL_STEP));
}

#[test]
fn the_spine_follows_the_model_when_a_join_changes() {
    let mut win = Window::over_fixture();
    let before = win.rows();
    let joins = |rows: &[SpineRow]| {
        rows.iter()
            .find(|r| r.role == SpineRole::Model)
            .map(|r| r.kind.clone())
    };
    win.app
        .perform_model_action(&win.ctx, ModelAction::Drop(LISTED_JOIN.to_string()));
    win.frame(Vec::new());
    let after = win.rows();
    assert_ne!(before, after, "the spine did not follow the dropped join");
    assert_ne!(joins(&before), joins(&after), "the model row's kind held");
}

#[test]
fn a_missing_manifest_leaves_an_empty_spine_and_the_pane() {
    let mut win = Window::over_fixture();
    let manifest = win.pane().protocol.dir.join("arcform.yaml");
    std::fs::remove_file(&manifest).expect("the opened Protocol has a manifest");
    win.frame(Vec::new());
    let rows = win.rows();
    assert!(
        !rows.iter().any(|r| r.role == SpineRole::Asset),
        "table rows without a manifest: {rows:?}"
    );
    assert!(win.app.model_pane().is_some());
}

#[test]
fn a_manifest_change_rebuilds_the_spine() {
    let mut win = Window::over_fixture();
    assert!(!win.rows().iter().any(|r| r.label == "extra_step"));
    // A step written into the manifest, as a save from a table's grid does, with no model action.
    let manifest = win.pane().protocol.dir.join("arcform.yaml");
    let mut text = std::fs::read_to_string(&manifest).unwrap();
    text.push_str(
        "- name: extra_step\n  sql: models/01_load_customers.sql\n  produces:\n  - extra_table\n",
    );
    std::fs::write(&manifest, text).unwrap();
    // The write may land inside the clock's granularity of the last one.
    let file = std::fs::File::options()
        .write(true)
        .open(&manifest)
        .unwrap();
    file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(10))
        .unwrap();
    win.frame(Vec::new());
    assert!(
        win.rows().iter().any(|r| r.label == "extra_step"),
        "the new step is not in {:?}",
        win.rows()
    );
}

#[test]
fn a_model_with_no_join_still_lists_the_model_step() {
    // The own-files fixture's discovery proposes no join, so its model makes none.
    let mut win = Window::over_own_files();
    let rows = win.rows();
    let model: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.role == SpineRole::Model)
        .map(|(at, _)| at)
        .collect();
    assert_eq!(model, vec![rows.len() - 1], "{rows:?}");
    assert_eq!(rows.last().unwrap().kind, "model \u{b7} 0 joins");
    win.frame(Vec::new());
}

fn button_at(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

impl Window {
    fn click(&mut self, at: egui::Pos2) {
        // One frame for the whole click: the frame clock moves a second a frame, and a press
        // held across a second is no click.
        self.frame(vec![
            egui::Event::PointerMoved(at),
            button_at(at, true),
            button_at(at, false),
        ]);
        self.frame(Vec::new());
        self.frame(Vec::new());
    }

    fn drawn_row(&self, label: &str) -> brightfield_shell::protocol::SpineRowDrawn {
        self.app
            .spine_rows()
            .iter()
            .find(|r| r.label == label)
            .unwrap_or_else(|| panic!("no drawn row {label} in {:?}", self.app.spine_rows()))
            .clone()
    }
}

#[test]
fn a_protocol_of_several_sources_draws_its_spine_beside_the_model() {
    let win = Window::over_fixture();
    let drawn = win.app.spine_rows();
    let at = |label: &str| {
        drawn
            .iter()
            .position(|r| r.label == label)
            .unwrap_or_else(|| panic!("no drawn row {label} in {drawn:?}"))
    };
    for table in LISTED_TABLES {
        assert!(at(table) < at(MODEL_STEP), "{table} follows the model row");
    }
    let model = &drawn[at(MODEL_STEP)];
    assert_eq!(model.role, SpineRole::Model);
    assert!(model.washed, "the model's row is not the selected one");
    if let Some(unjoined) = drawn
        .iter()
        .position(|r| r.role == SpineRole::Caption && r.label.to_uppercase().contains("UNJOINED"))
    {
        assert!(unjoined > at(MODEL_STEP));
    }
    let canvas = win
        .app
        .region_rect(arrangement::CANVAS)
        .expect("the canvas drew");
    for row in drawn {
        assert!(
            row.rect.max.x <= canvas.min.x + 0.5,
            "{} reaches into the model pane: {:?} vs {canvas:?}",
            row.label,
            row.rect
        );
    }
    // One curve for each non-proposal join between two tables the spine lists.
    let rows = win.rows();
    let listed: Vec<&str> = rows
        .iter()
        .filter(|r| r.role == SpineRole::Asset)
        .map(|r| r.label.as_str())
        .collect();
    let expected = win
        .pane()
        .model
        .relationships
        .iter()
        .filter(|r| !brightfield_ossie::is_proposal(r))
        .filter(|r| listed.contains(&r.from.as_str()) && listed.contains(&r.to.as_str()))
        .count();
    assert_eq!(win.app.spine_curves().len(), expected);
    assert!(expected > 0);
}

#[test]
fn a_click_on_a_tables_row_opens_its_grid_and_keeps_the_spine() {
    let mut win = Window::over_fixture();
    let row = win.drawn_row("customers");
    assert!(row.control, "the table's row is not a control");
    win.click(row.rect.center());
    assert!(win.app.chart_doc().grid_drawn().is_some(), "no grid opened");
    assert!(win.app.model_pane().is_some(), "the model pane was let go");
    assert!(win.app.is_model_behind_grid(), "the model is not waiting");
    assert_eq!(win.app.model_grid_table(), Some("customers"));
}

/// The labels of the spine's rows as drawn, the head row first, without the column outline the
/// grid's table adds below them.
fn spine_labels(win: &Window, count: usize) -> Vec<String> {
    win.app
        .spine_rows()
        .iter()
        .take(count)
        .map(|r| r.label.clone())
        .collect()
}

#[test]
fn the_spine_stays_the_protocols_while_a_table_is_in_the_grid() {
    let mut win = Window::over_fixture();
    let before = win.app.spine_rows().to_vec();
    let row = win.drawn_row("customers");
    win.click(row.rect.center());
    assert!(win.app.is_model_behind_grid());
    let after = spine_labels(&win, before.len());
    let was: Vec<String> = before.iter().map(|r| r.label.clone()).collect();
    assert_eq!(after, was, "the rows changed when the grid opened");
    assert!(
        win.drawn_row("customers").washed,
        "the table's row is not selected"
    );
    assert!(
        !win.drawn_row(MODEL_STEP).washed,
        "the model's row is still selected"
    );
}

#[test]
fn the_model_row_goes_back_to_the_model() {
    let mut win = Window::over_fixture();
    let row = win.drawn_row("customers");
    win.click(row.rect.center());
    assert!(win.app.is_model_behind_grid());
    let model = win.drawn_row(MODEL_STEP);
    win.click(model.rect.center());
    assert!(!win.app.is_model_behind_grid());
    assert!(win.app.model_pane().is_some());
    assert!(
        win.drawn_row(MODEL_STEP).washed,
        "the model's row is not selected again"
    );
}

#[test]
fn another_tables_row_opens_that_table_instead() {
    let mut win = Window::over_fixture();
    let row = win.drawn_row("customers");
    win.click(row.rect.center());
    assert_eq!(win.app.model_grid_table(), Some("customers"));
    let other = win.drawn_row("loyalty_visits");
    win.click(other.rect.center());
    assert!(win.app.is_model_behind_grid());
    assert_eq!(win.app.model_grid_table(), Some("loyalty_visits"));
    assert!(win.app.chart_doc().grid_drawn().is_some());
    assert!(win.drawn_row("loyalty_visits").washed);
    assert!(!win.drawn_row("customers").washed);
}

#[test]
fn unsaved_grid_edits_hold_the_spine_click() {
    let mut win = Window::over_fixture();
    let row = win.drawn_row("customers");
    win.click(row.rect.center());
    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some("1 = 1".to_string()),
            order: None,
        });
    win.frame(Vec::new());
    assert!(win.app.chart_doc().has_unsaved_work(), "the grid is clean");
    let model = win.drawn_row(MODEL_STEP);
    win.click(model.rect.center());
    assert!(win.app.is_model_behind_grid(), "the grid let go");
    assert!(
        win.app
            .notifications()
            .iter()
            .any(|n| n.title == "These rows have unsaved edits"),
        "no notice of the unsaved edits"
    );
}

#[test]
fn a_collapsed_navigator_beside_the_model_records_no_rows() {
    let mut win = Window::over_fixture();
    assert!(!win.app.spine_rows().is_empty());
    let toggle = win
        .app
        .rail_collapse_rect(arrangement::NAVIGATOR_RAIL)
        .expect("the rail drew a collapse control");
    win.click(toggle.center());
    assert!(
        win.app.spine_rows().is_empty(),
        "rows beside a collapsed rail"
    );
    assert!(win.app.region_rect(arrangement::CANVAS).is_some());
    let toggle = win
        .app
        .rail_collapse_rect(arrangement::NAVIGATOR_RAIL)
        .expect("the stub drew its control");
    win.click(toggle.center());
    assert!(!win.app.spine_rows().is_empty(), "the rows did not return");
}

#[test]
fn the_spine_rows_beside_the_model_are_named_controls() {
    let win = Window::over_fixture();
    for table in LISTED_TABLES {
        let row = win.drawn_row(table);
        assert!(
            win.app
                .named_controls()
                .iter()
                .any(|c| c.rect == row.rect && c.name == table),
            "{table} is not a named control"
        );
    }
}

/// Opens `customers`' grid and makes its dashboard dirty, as the unsaved-edits test does.
fn dirty_customers_grid() -> Window {
    let mut win = Window::over_fixture();
    let row = win.drawn_row("customers");
    win.click(row.rect.center());
    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some("1 = 1".to_string()),
            order: None,
        });
    win.frame(Vec::new());
    assert!(win.app.chart_doc().has_unsaved_work(), "the grid is clean");
    win
}

#[test]
fn unsaved_grid_edits_hold_a_click_on_another_tables_row() {
    let mut win = dirty_customers_grid();
    let other = win.drawn_row("loyalty_visits");
    win.click(other.rect.center());
    assert!(win.app.is_model_behind_grid());
    assert_eq!(win.app.model_grid_table(), Some("customers"));
    assert!(
        win.app
            .notifications()
            .iter()
            .any(|n| n.title == "These rows have unsaved edits"),
        "no notice of the unsaved edits"
    );
}

#[test]
fn a_click_on_the_row_of_the_table_in_the_grid_does_nothing() {
    // With a dirty dashboard: a reopen would raise the unsaved-edits notice and let go of the
    // edits, and neither happens.
    let mut win = dirty_customers_grid();
    let own = win.drawn_row("customers");
    assert!(!own.control, "the grid's own table is drawn as a control");
    win.click(own.rect.center());
    assert!(win.app.is_model_behind_grid());
    assert_eq!(win.app.model_grid_table(), Some("customers"));
    assert!(
        win.app.chart_doc().has_unsaved_work(),
        "the grid was reopened"
    );
    assert!(
        !win.app
            .notifications()
            .iter()
            .any(|n| n.title == "These rows have unsaved edits"),
        "the click tried to go back"
    );
}
