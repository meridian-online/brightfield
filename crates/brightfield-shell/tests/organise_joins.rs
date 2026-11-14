//! `o` organises a Protocol's joins: over a Protocol of two tables or more it opens a sheet of the
//! model's joins, each ticked when it is part of the model and unticked when it is a proposal
//! waiting on the analyst; `Space` ticks or unticks the join under the cursor, `Enter` turns what
//! changed into the model pane's Keep and Drop, and `Esc` leaves the model as it was (`escape_changes_nothing`). Windows are headless,
//! over the retail extract and the model written for it, as in the spine's tests.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use brightfield_ossie::{DecidedBy, Model};
use brightfield_shell::design::Mode;
use brightfield_shell::run::Runner;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::PaneKey;

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-organise-joins-{}", std::process::id()));
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

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// The retail extract: eight CSVs, and the model written for them, with six likely joins
/// discovery found, one possible join waiting on the analyst, and categories folded into
/// products by the only join between them.
fn retail() -> PathBuf {
    scratch();
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../brightfield-ossie/fixtures/retail_extract")
}

/// The written retail model's one proposal, waiting on the analyst.
const PROPOSAL: &str = "customer_segments_customers";
/// The written retail model's one join folding categories into products.
const FOLDING: &str = "products_categories";
/// A likely join discovery found, already the model's.
const LIKELY: &str = "transactions_customers";

fn key_down(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// One headless window over a Protocol.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    /// The frame clock, at sixty frames a second, so no toast a test counts ages out within it.
    time: f64,
}

impl Window {
    fn blank() -> Self {
        scratch();
        let mut win = Self {
            app: MeridianApp::headless(Boot::empty(), Mode::Light).running_with(Some(runner())),
            ctx: egui::Context::default(),
            time: 0.0,
        };
        win.frame(Vec::new());
        win
    }

    /// A window that opened the retail folder and saved its written model, with the
    /// model pane on the canvas and the spine holding the keys.
    fn over_retail() -> Self {
        let mut win = Self::blank();
        let dir = retail();
        win.app
            .open_path_knowing(&win.ctx, &dir.to_string_lossy(), &[]);
        let pane = win
            .app
            .model_pane_mut()
            .expect("the folder opens into the model pane");
        pane.finish_discovery();
        let text = std::fs::read_to_string(dir.join("model.ossie.yaml")).unwrap();
        pane.save_model(brightfield_ossie::read(&text).unwrap())
            .expect("the written model runs");
        win.frame(Vec::new());
        // The open leaves the sources browser over the window, which would take every key.
        if win.app.sources_browser().is_some() {
            win.key(egui::Key::Escape);
        }
        win.spine_holds_the_keys();
        win
    }

    /// A window over a Protocol Brightfield made of one CSV, with the spine holding the keys.
    fn over_one_table() -> Self {
        let mut win = Self::blank();
        let csv = scratch().join("input").join("housing.csv");
        std::fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/data/california_housing_sample.csv"),
            &csv,
        )
        .expect("the housing sample copies");
        win.app
            .open_queue()
            .push(brightfield_shell::open_queue::OpenRequest::Path(csv));
        win.settle();
        if win.app.sources_browser().is_some() {
            win.key(egui::Key::Escape);
        }
        win.spine_holds_the_keys();
        win
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 1.0 / 60.0;
        let raw = egui::RawInput {
            time: Some(self.time),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440.0, 1600.0),
            )),
            events,
            ..Default::default()
        };
        let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.frame(Vec::new());
        }
    }

    /// Press `key` and let the frame after run.
    fn key(&mut self, key: egui::Key) {
        self.frame(vec![key_down(key)]);
        self.frame(Vec::new());
    }

    /// `o`, as a keyboard brings it: the key and the text it types.
    fn o(&mut self) {
        self.frame(vec![key_down(egui::Key::O), egui::Event::Text("o".into())]);
        self.frame(Vec::new());
    }

    /// Give the Protocol's keys to the spine: the Outline pane focused, as a click on the rail
    /// focuses it, which is where `o` is read with the model pane or a grid on the canvas.
    fn spine_holds_the_keys(&mut self) {
        assert!(
            self.app
                .focus_pane(PaneKey::new(brightfield_shell::protocol::OUTLINE)),
            "the Outline took no focus"
        );
        self.frame(Vec::new());
    }

    fn model(&self) -> &Model {
        &self.app.model_pane().expect("the model is open").model
    }

    fn rows(&self) -> Vec<(String, bool)> {
        self.app
            .organise_joins_rows()
            .expect("the organise sheet is open")
    }

    /// Move the sheet's cursor onto the row reading `relationship`'s join.
    fn cursor_to(&mut self, relationship: &str) {
        let at = self
            .model()
            .relationships
            .iter()
            .position(|r| r.name == relationship)
            .expect("the model holds the join");
        for _ in 0..at {
            self.key(egui::Key::J);
        }
    }
}

fn relationship<'a>(m: &'a Model, name: &str) -> Option<&'a brightfield_ossie::Relationship> {
    m.relationships.iter().find(|r| r.name == name)
}

#[test]
fn o_opens_the_sheet_with_the_models_joins() {
    let mut win = Window::over_retail();
    win.o();
    assert_eq!(win.app.open_overlay(), Some("organise-joins"));
    let rows = win.rows();
    let model = win.model();
    assert_eq!(rows.len(), model.relationships.len(), "{rows:?}");
    for (r, (label, kept)) in model.relationships.iter().zip(&rows) {
        assert!(label.starts_with(&format!("{}.", r.from)), "{label}");
        assert_eq!(*kept, r.name != PROPOSAL, "{} reads {label}", r.name);
    }
}

#[test]
fn space_and_enter_keep_a_proposal_as_the_analysts() {
    let mut win = Window::over_retail();
    win.o();
    win.cursor_to(PROPOSAL);
    win.key(egui::Key::Space);
    win.key(egui::Key::Enter);
    assert_eq!(win.app.open_overlay(), None);
    let kept = relationship(win.model(), PROPOSAL).expect("the proposal is still a join");
    assert_eq!(kept.meta.decided_by, DecidedBy::Analyst);
}

#[test]
fn space_and_enter_drop_a_kept_join() {
    let mut win = Window::over_retail();
    win.o();
    win.cursor_to(LIKELY);
    win.key(egui::Key::Space);
    win.key(egui::Key::Enter);
    assert_eq!(win.app.open_overlay(), None);
    let model = win.model();
    assert!(
        relationship(model, LIKELY).is_none(),
        "{LIKELY} is still a join"
    );
    assert!(
        model.meta.rejected.iter().any(|r| r.name == LIKELY),
        "{LIKELY} is not among the rejected joins"
    );
}

#[test]
fn escape_changes_nothing() {
    let mut win = Window::over_retail();
    let before = win.model().clone();
    win.o();
    win.key(egui::Key::Space);
    win.key(egui::Key::J);
    win.key(egui::Key::Space);
    win.key(egui::Key::Escape);
    assert_eq!(win.app.open_overlay(), None);
    assert_eq!(win.model(), &before);
}

#[test]
fn a_refused_drop_leaves_the_other_changes() {
    let mut win = Window::over_retail();
    let toasts = win.app.toasts().len();
    win.o();
    win.cursor_to(PROPOSAL);
    win.key(egui::Key::Space);
    // The folding join is the row after the proposal's.
    win.key(egui::Key::J);
    win.key(egui::Key::Space);
    win.key(egui::Key::Enter);
    assert_eq!(win.app.open_overlay(), None);
    let model = win.model();
    assert_eq!(
        relationship(model, PROPOSAL).map(|r| r.meta.decided_by),
        Some(DecidedBy::Analyst),
        "the proposal was not kept"
    );
    assert!(
        relationship(model, FOLDING).is_some(),
        "the only join folding categories into products was dropped"
    );
    assert!(
        win.app.toasts().len() >= toasts + 2,
        "the keep and the refusal were not both reported"
    );
}

#[test]
fn several_changes_land_together_with_one_save() {
    let mut win = Window::over_retail();
    let before = win.app.model_pane().unwrap().generation();
    win.o();
    win.cursor_to(PROPOSAL);
    win.key(egui::Key::Space);
    // Back to the first row, a likely join, and untick it.
    for _ in 0..8 {
        win.key(egui::Key::K);
    }
    win.key(egui::Key::Space);
    win.key(egui::Key::Enter);
    let model = win.model();
    assert_eq!(
        relationship(model, PROPOSAL).map(|r| r.meta.decided_by),
        Some(DecidedBy::Analyst),
        "the proposal was not kept"
    );
    assert!(
        relationship(model, LIKELY).is_none(),
        "{LIKELY} was not dropped"
    );
    let batch = win.app.model_pane().unwrap().generation() - before;

    // One change on its own, through the model pane's own Drop, writes the model as often as
    // the two changes together did: one save, not one per change.
    let one = win.app.model_pane().unwrap().generation();
    let next = win.model().relationships[0].name.clone();
    win.app.perform_model_action(
        &win.ctx.clone(),
        brightfield_shell::model_inspector::ModelAction::Drop(next),
    );
    let single = win.app.model_pane().unwrap().generation() - one;
    assert_eq!(
        batch, single,
        "two changes wrote the model {batch} times, one {single}"
    );
}

#[test]
fn o_over_one_table_opens_nothing_and_says_why() {
    let mut win = Window::over_one_table();
    let toasts = win.app.toasts().len();
    win.o();
    assert_eq!(win.app.open_overlay(), None);
    assert!(win.app.toasts().len() > toasts, "nothing said why");
}

#[test]
fn o_while_a_text_field_has_the_keys_opens_nothing() {
    let mut win = Window::over_retail();
    win.key(egui::Key::Space);
    assert_eq!(win.app.open_overlay(), Some("palette"));
    win.o();
    assert_eq!(win.app.open_overlay(), Some("palette"));
}
