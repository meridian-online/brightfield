//! The model pane holds its Protocol's spine, built again on a frame when the model's generation
//! moves, and the navigator rail draws it beside the pane and beside a table's grid. Windows are
//! headless, as in the model pane's own tests. A test reads the spine the window holds through
//! `model_spine_rows`, and what the rail drew through `spine_rows` and `spine_curves`.

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

/// The retail extract: eight CSVs, and the model written for them, whose joins the spine leads
/// with. The own-files fixture's discovery proposes no join, so its spine has no joined tables.
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
            // Tall enough that every row of the retail Protocol's spine is drawn inside the rail,
            // beside the model pane and beside the grid, so a click lands without scrolling.
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440.0, 1600.0),
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

/// Two of the joined tables of the written retail model, and the join between them.
const LISTED_TABLES: [&str; 2] = ["loyalty_visits", "customers"];
/// The written retail model's one possible join discovery proposed, which waits on the analyst:
/// `customer_segments` is listed with no join until it is kept.
const PROPOSED_JOIN: &str = "customer_segments_customers";
const PROPOSED_TABLE: &str = "customer_segments";

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
    let drawn = |win: &Window| -> Vec<String> {
        win.app
            .spine_rows()
            .iter()
            .map(|r| r.label.clone())
            .collect()
    };
    let drawn_before = drawn(&win);
    let curves_before = win.app.spine_curves().len();
    let at = |labels: &[String], label: &str| labels.iter().position(|l| l == label).unwrap();
    assert!(at(&drawn_before, PROPOSED_TABLE) > at(&drawn_before, MODEL_STEP));
    win.app
        .perform_model_action(&win.ctx, ModelAction::Keep(PROPOSED_JOIN.to_string()));
    win.frame(Vec::new());
    let after = win.rows();
    assert_ne!(before, after, "the spine did not follow the kept join");
    assert_ne!(joins(&before), joins(&after), "the model row's kind held");
    // What the next frame drew: one curve more, and the table the join reaches moved up among
    // the joined tables, ahead of the model's row.
    let drawn_after = drawn(&win);
    assert_eq!(win.app.spine_curves().len(), curves_before + 1);
    assert_ne!(drawn_before, drawn_after, "the drawn rows did not move");
    assert!(
        at(&drawn_after, PROPOSED_TABLE) < at(&drawn_after, MODEL_STEP),
        "{PROPOSED_TABLE} is not among the joined tables: {drawn_after:?}"
    );
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
    // The retail model joins no table to `customer_segments` until its proposal is kept, so the
    // heading over the sources with no join is drawn, after the model's row.
    let heading = drawn
        .iter()
        .position(|r| r.role == SpineRole::Unjoined)
        .unwrap_or_else(|| panic!("no heading over the sources with no join in {drawn:?}"));
    assert!(drawn[heading]
        .label
        .starts_with(brightfield_shell::protocol::UNJOINED_HEADING));
    assert!(heading > at(MODEL_STEP), "the heading leads the model row");
    assert!(at(PROPOSED_TABLE) > heading);
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
    // The Protocol's graph is not what the head row's chip would put on the canvas, so the chip
    // is a readout.
    let chip = win
        .app
        .spine_rows()
        .first()
        .and_then(|head| head.chip)
        .expect("the head row drew the graph chip");
    assert!(!chip.live, "the graph chip is a control beside the grid");
    // The column outline under the spine is the opened table's.
    let columns: Vec<String> = win
        .app
        .spine_rows()
        .iter()
        .filter(|r| r.role == SpineRole::Column)
        .map(|r| r.label.clone())
        .collect();
    let customers = win
        .pane()
        .model
        .datasets
        .iter()
        .find(|d| d.name == "customers")
        .expect("the model has customers");
    for field in &customers.fields {
        assert!(
            columns.contains(&field.name),
            "the outline lists no {} in {columns:?}",
            field.name
        );
    }
    assert!(!columns.iter().any(|c| c == "store_id"), "{columns:?}");
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
    let mut win = dirty_customers_grid();
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

/// Opens `customers`' grid and makes its dashboard dirty, with a condition its view holds and no
/// file does.
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

/// The model's tables that have rows of their own for the grid to open, by name, in name order.
fn model_tables(win: &Window) -> Vec<String> {
    let model = &win.pane().model;
    let mut tables: Vec<String> = model
        .datasets
        .iter()
        .map(|d| d.name.clone())
        .filter(|name| brightfield_shell::model_inspector::materialised(model, name))
        .collect();
    tables.sort();
    tables
}

/// The table the grid's last open said it could not draw, read from the file it wrote for it.
fn refused_table(win: &Window) -> Option<String> {
    win.app
        .notifications()
        .iter()
        .find(|n| n.title == "Could not open these rows in the grid")
        .and_then(|n| n.body.as_deref())
        .and_then(|body| body.split(".grid/").nth(1))
        .and_then(|rest| rest.split(".parquet").next())
        .map(str::to_string)
}

/// Each table of the model that has rows of its own has one row beside the model pane that
/// answers a click, and the click opens that table's rows in the grid with its row selected on
/// the rail. The rows are clicked in turn, with the model's row taken back to the model between
/// them. A table whose rows the grid cannot draw says so, naming the file it wrote for that table,
/// which is the click asking for the right table.
fn every_table_opens_from_one_row(mut win: Window) {
    let tables = model_tables(&win);
    assert!(!tables.is_empty());
    let drawn: Vec<(String, String, bool)> = win
        .app
        .spine_rows()
        .iter()
        .map(|r| (r.label.clone(), r.kind.clone(), r.control))
        .collect();
    let controls: Vec<String> = win
        .app
        .spine_rows()
        .iter()
        .filter(|r| r.control)
        .map(|r| r.label.clone())
        .collect();
    assert_eq!(
        controls.len(),
        tables.len(),
        "control rows {controls:?} for tables {tables:?}; drawn {drawn:?}"
    );
    let mut opened = Vec::new();
    for label in controls {
        let row = win.drawn_row(&label);
        win.click(row.rect.center());
        let Some(table) = win.app.model_grid_table().map(str::to_string) else {
            let refused =
                refused_table(&win).unwrap_or_else(|| panic!("a click on {label} opened no table"));
            opened.push(refused);
            continue;
        };
        assert!(
            win.drawn_row(&label).washed,
            "{label} is not selected while {table} is in the grid"
        );
        opened.push(table);
        let model = win.drawn_row(MODEL_STEP);
        win.click(model.rect.center());
        assert!(!win.app.is_model_behind_grid(), "{label}'s grid held");
    }
    opened.sort();
    assert_eq!(opened, tables, "the rows opened these tables");
}

#[test]
fn every_table_of_the_retail_model_opens_from_its_row() {
    every_table_opens_from_one_row(Window::over_fixture());
}

#[test]
fn every_table_of_a_protocol_with_no_saved_model_opens_from_its_row() {
    every_table_opens_from_one_row(Window::over_own_files());
}

// ---- the spine picks a table for an operation, among the model's tables ----

mod pick {
    use std::cell::RefCell;

    use brightfield_shell::operation_ui::{
        Begin, By, Draft, Keeps, NameRule, Need, OperationUi, Stage, Start,
    };

    thread_local! {
        /// The tables the last `begin` on this thread was handed, each with its columns.
        pub static TAKEN: RefCell<Vec<(String, Vec<String>)>> = const { RefCell::new(Vec::new()) };
    }

    fn begin_join(b: &Begin<'_>) -> Result<Draft, String> {
        TAKEN.with(|taken| *taken.borrow_mut() = b.taken.to_vec());
        let mut draft = Draft::empty("join-tables");
        draft.on = b.table.map(str::to_string);
        draft.reads = b
            .taken
            .iter()
            .map(|(table, _)| ("other".to_string(), table.clone()))
            .collect();
        draft.set("using", b.column.unwrap_or_default().to_string());
        Ok(draft)
    }

    fn begin_append(b: &Begin<'_>) -> Result<Draft, String> {
        let mut draft = Draft::empty("append-rows");
        draft.on = b.table.map(str::to_string);
        Ok(draft)
    }

    /// An operation that reads a list of other tables, as appending rows does.
    pub static APPEND_LIKE: OperationUi = OperationUi {
        long_name: "append-rows",
        verb: "append-rows",
        stage: Stage::Model,
        icon: "row-insert-bottom",
        starts: &[Start::Spine { role: "other" }],
        menu_words: "Append rows",
        needs: &[Need::AnotherTable],
        begin: begin_append,
        keeps: Keeps::OnEnter,
        field: None,
        form: None,
        name: NameRule {
            word: "appended",
            from: By::Nothing,
            with_by: false,
        },
    };

    /// An operation that reads one other table, as joining two tables does.
    pub static JOIN_LIKE: OperationUi = OperationUi {
        long_name: "join-tables",
        verb: "join-tables",
        stage: Stage::Model,
        icon: "arrows-join",
        starts: &[Start::Spine { role: "other" }],
        menu_words: "Join tables",
        // What a built join declares: the pick's tables are the ones this need is checked
        // against, so it is met by the model's other tables.
        needs: &[Need::Rows, Need::AnotherTable],
        begin: begin_join,
        keeps: Keeps::OnEnter,
        field: None,
        form: None,
        name: NameRule {
            word: "joined",
            from: By::Nothing,
            with_by: false,
        },
    };
}

/// While the grid holds one of a model's tables, an operation that reads a second table picks it
/// among the model's other tables: each the model joins to the one in front says `related`, the
/// cursor starts on the first of them, and the table taken is handed over with the columns of its
/// grid copy, which the operation's preview reads.
#[test]
fn the_spine_picks_among_the_models_tables_while_one_is_in_the_grid() {
    use brightfield_shell::protocol::IN_FRONT;
    use brightfield_workbench::PaneKey;

    let mut win = Window::over_fixture();
    let row = win.drawn_row("customers");
    win.click(row.rect.center());
    assert_eq!(win.app.model_grid_table(), Some("customers"));
    assert!(win
        .app
        .focus_pane(PaneKey::new(brightfield_shell::data_grid::DATA)));
    win.frame(Vec::new());

    // Start from the spine on `customer_id`, once arc's description has come.
    let ctx = win.ctx.clone();
    let from = brightfield_shell::operation_ui::Start::Spine { role: "other" };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !win
        .app
        .start_operation_spec(&ctx, &pick::JOIN_LIKE, from, Some("customer_id"))
    {
        let said = win.app.operation_said().unwrap_or_default().to_string();
        assert!(said.starts_with("Reading arc's description"), "{said}");
        assert!(
            std::time::Instant::now() < deadline,
            "arc never described the join"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        win.frame(Vec::new());
    }
    // The columns of each table's grid copy are read on a worker.
    while win
        .app
        .spine_pick()
        .is_some_and(|p| p.candidates().iter().any(|c| c.columns.is_none()))
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the columns never arrived"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
        win.frame(Vec::new());
    }
    win.frame(Vec::new());

    // The candidates are the model's other tables that have rows to read.
    let model = win.pane().model.clone();
    let pick = win.app.spine_pick().expect("the pick is open").clone();
    assert_eq!(pick.in_front(), "customers");
    let candidates: Vec<&str> = pick.candidates().iter().map(|c| c.table.as_str()).collect();
    assert!(!candidates.contains(&"customers"), "{candidates:?}");
    assert!(candidates.contains(&"loyalty_visits"), "{candidates:?}");
    // Each the model joins to `customers` says so, and no other does.
    let joined = |table: &str| {
        model.relationships.iter().any(|r| {
            (r.from == table && r.to == "customers") || (r.from == "customers" && r.to == table)
        })
    };
    for candidate in pick.candidates() {
        assert_eq!(
            candidate.related,
            joined(&candidate.table),
            "{}",
            candidate.table
        );
        assert!(
            candidate.columns.as_ref().is_some_and(|c| !c.is_empty()),
            "{} has no columns",
            candidate.table
        );
    }
    let under = pick.under_cursor().expect("a candidate").clone();
    assert!(under.related, "the cursor starts on {}", under.table);
    // The rail says the same: the table in front, and `related` on the cursor's row.
    assert_eq!(win.drawn_row("customers").kind, IN_FRONT);
    let drawn = win.drawn_row(&under.table);
    assert_eq!(drawn.kind, "related");
    assert!(drawn.washed);

    // Enter hands the table over with its columns, and the draft's preview reads it.
    win.frame(vec![egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    win.frame(Vec::new());
    assert!(win.app.spine_pick().is_none());
    let taken = pick::TAKEN.with(|taken| taken.borrow().clone());
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].0, under.table);
    assert_eq!(Some(&taken[0].1), under.columns.as_ref());
    assert!(taken[0].1.contains(&"customer_id".to_string()), "{taken:?}");
    let making = win
        .app
        .making()
        .unwrap_or_else(|| panic!("no draft: {:?}", win.app.operation_said()));
    assert_eq!(
        making.draft.reads,
        [("other".to_string(), under.table.clone())]
    );
    assert!(win
        .app
        .finish_operation_reads(std::time::Duration::from_secs(60)));
    let reads = win.app.draft_reads().expect("the draft is read");
    assert_eq!(reads.refusal(), None);
    let answer = reads.current().expect("the draft is answered");
    assert_eq!(answer.unread, None);
    let joined_rows = answer.rows;
    assert!(joined_rows > 0, "the join of the two tables has rows");
    // The copy of the table taken was written at the take, and of no other candidate.
    let grid = win.pane().protocol.dir.join(".grid");
    for candidate in pick.candidates() {
        let copied = grid.join(format!("{}.parquet", candidate.table)).is_file();
        assert_eq!(
            copied,
            candidate.table == under.table,
            "{}",
            candidate.table
        );
    }

    // Enter keeps the draft, and the kept step reads the table taken too: its grid
    // previews, and its rows are counted.
    let enter = || egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    win.frame(vec![enter()]);
    win.frame(Vec::new());
    let kept = win.app.chart_doc().typed_steps().to_vec();
    assert_eq!(kept.len(), 1, "{:?}", win.app.operation_said());
    let name = kept[0].name.clone();
    let counted = |win: &Window| win.app.chart_doc().typed_steps()[0].rows;
    let previewing = |win: &Window| {
        win.app
            .chart_doc()
            .grid_preview()
            .is_none_or(|p| p.is_reading())
    };
    while counted(&win).is_none() || previewing(&win) {
        assert!(
            std::time::Instant::now() < deadline,
            "{name} was never read"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
        win.frame(Vec::new());
    }
    assert_eq!(counted(&win), Some(joined_rows));
    let page = win
        .app
        .chart_doc()
        .grid_preview()
        .and_then(|p| p.page())
        .unwrap_or_else(|| panic!("the kept step's grid shows no page"));
    assert!(!page.rows.is_empty(), "the kept step's grid shows no rows");

    // Another operation started over the kept step's grid: the pick offers the tables the rail lists there,
    // the model's, and no other: each candidate has a row of its own, named in the primary ink,
    // and the cursor's is the one drawn selected. The kept step has no row on that rail and is
    // no candidate.
    assert!(win
        .app
        .focus_pane(PaneKey::new(brightfield_shell::data_grid::DATA)));
    win.frame(Vec::new());
    while !win
        .app
        .start_operation_spec(&ctx, &pick::APPEND_LIKE, from, None)
    {
        let said = win.app.operation_said().unwrap_or_default().to_string();
        assert!(said.starts_with("Reading arc's description"), "{said}");
        assert!(
            std::time::Instant::now() < deadline,
            "arc never described the append"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        win.frame(Vec::new());
    }
    win.frame(Vec::new());
    win.frame(Vec::new());
    let again = win.app.spine_pick().expect("the pick opened again").clone();
    let offered: Vec<&str> = again
        .candidates()
        .iter()
        .map(|c| c.table.as_str())
        .collect();
    assert!(!offered.contains(&name.as_str()), "{offered:?}");
    // The grid's own table is one of them now that a step of it is in front, and it is read
    // as the loaded table is, with no copy to write.
    let own = again.candidate("customers").expect("customers is offered");
    assert_eq!(
        own.source,
        brightfield_shell::spine_pick::CandidateSource::Step
    );
    let selected = again.under_cursor().expect("a candidate").table.clone();
    for candidate in again.candidates() {
        let row = win.drawn_row(&candidate.table);
        assert!(
            !row.muted && row.control,
            "{} has no row to pick",
            candidate.table
        );
        assert_eq!(
            row.washed,
            candidate.table == selected,
            "{}",
            candidate.table
        );
    }
    win.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    win.frame(Vec::new());
    assert!(win.app.spine_pick().is_none());
    assert_eq!(win.app.chart_doc().typed_steps().len(), 1);

    // Save records the step through arc. Which tables its preview loads is read from the
    // step, not from what this window picked: with the taken table's copy gone, as in a folder
    // another session left, the saved step's grid writes the copy again and shows the join's rows.
    let copy = grid.join(format!("{}.parquet", under.table));
    std::fs::remove_file(&copy).expect("the copy written at the take is there");
    win.app
        .save_protocol(&ctx)
        .expect("a Protocol to save")
        .unwrap_or_else(|e| panic!("the Save records {name}: {e}"));
    assert!(
        win.app.chart_doc().typed_steps().is_empty(),
        "{name} is recorded"
    );
    win.frame(Vec::new());
    while previewing(&win) {
        assert!(
            std::time::Instant::now() < deadline,
            "{name} was never read again"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
        win.frame(Vec::new());
    }
    assert!(copy.is_file(), "the copy was not written again");
    // A copy from before the Protocol's database last changed is not current either, and is
    // written again as a missing one is.
    let protocol = win.pane().protocol.clone();
    assert!(brightfield_shell::own_files::grid_copy_is_current(
        &protocol,
        &under.table
    ));
    std::fs::File::options()
        .write(true)
        .open(&copy)
        .and_then(|file| file.set_modified(std::time::SystemTime::UNIX_EPOCH))
        .expect("the copy's time is set back");
    assert!(!brightfield_shell::own_files::grid_copy_is_current(
        &protocol,
        &under.table
    ));
    assert!(!brightfield_shell::own_files::grid_copy_is_current(
        &protocol,
        "no_such_table"
    ));
    let page = win
        .app
        .chart_doc()
        .grid_preview()
        .and_then(|p| p.page())
        .unwrap_or_else(|| panic!("the saved step's grid shows no page"));
    assert!(!page.rows.is_empty(), "the saved step's grid shows no rows");
}

/// Over a join's rows the rail does not draw the model's spine, so an operation that reads a
/// second table is told it needs one, and no pick opens over tables the rail does not list.
#[test]
fn over_a_joins_rows_the_spine_offers_no_model_table() {
    let mut win = Window::over_fixture();
    let join = win
        .pane()
        .model
        .relationships
        .iter()
        .find(|r| !brightfield_ossie::is_proposal(r))
        .map(|r| r.name.clone())
        .expect("the written model makes a join");
    win.app.perform_model_action(
        &win.ctx.clone(),
        ModelAction::OpenInGrid(brightfield_render::model_scene::Selection::Relationship(
            join,
        )),
    );
    win.frame(Vec::new());
    win.frame(Vec::new());
    assert!(
        win.app.is_model_behind_grid(),
        "the join's rows did not open"
    );
    assert_eq!(win.app.model_grid_table(), None);
    let ctx = win.ctx.clone();
    let from = brightfield_shell::operation_ui::Start::Spine { role: "other" };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        assert!(!win
            .app
            .start_operation_spec(&ctx, &pick::JOIN_LIKE, from, None));
        let said = win.app.operation_said().unwrap_or_default().to_string();
        if !said.starts_with("Reading arc's description") {
            assert!(win.app.spine_pick().is_none());
            assert_eq!(said, "Join tables needs a second table in the Protocol.");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "arc never described the join"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        win.frame(Vec::new());
    }
}
