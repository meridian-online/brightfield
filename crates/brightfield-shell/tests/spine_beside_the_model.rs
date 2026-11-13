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
}

impl Window {
    fn over_fixture() -> Self {
        let mut win = Self {
            app: MeridianApp::headless(Boot::empty(), Mode::Light).running_with(Some(runner())),
            ctx: egui::Context::default(),
        };
        win.frame(Vec::new());
        let dir = fixture();
        win.app
            .open_path_knowing(&win.ctx, &dir.to_string_lossy(), &[]);
        // The open's full pass lands before the test looks, then the written model is saved over
        // the one discovery found, which makes the silver tables and the joins the spine reads.
        let pane = win
            .app
            .model_pane_mut()
            .expect("the folder opens into the model pane");
        pane.finish_discovery();
        let text = std::fs::read_to_string(fixture().join("model.ossie.yaml")).unwrap();
        pane.save_model(brightfield_ossie::read(&text).unwrap())
            .expect("the written model runs");
        win.frame(Vec::new());
        win
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
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
    win.app
        .perform_model_action(&win.ctx, ModelAction::Drop(LISTED_JOIN.to_string()));
    win.frame(Vec::new());
    let rows = win.rows();
    assert!(
        !rows.iter().any(|r| r.role == SpineRole::Asset),
        "table rows without a manifest: {rows:?}"
    );
    assert!(win.app.model_pane().is_some());
}
