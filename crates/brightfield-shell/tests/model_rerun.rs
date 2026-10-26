//! After each run the model pane makes, and when it opens over a Protocol already run, each join's
//! evidence is measured again over the silver tables. A join the data no longer bears out warns,
//! on its pill and in its inspector, and keeps its grade and who decided it. Rediscover folds a
//! fresh discovery pass into the model around what the analyst decided.
//!
//! The Protocols here are opened through the app's own open, with the `brightfield-shell` binary
//! cargo built as their runner, under a scratch folder of this file's that the config directory
//! and arc's directories point at before anything reads them. Nothing here needs a FineType
//! bundle, and nothing opens a GPU device.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use brightfield_keys::{Altitude, CommandTier, Drives};
use brightfield_ossie::{DecidedBy, Edit, Grade, Model, Role, Verified};
use brightfield_render::model_scene::Selection;
use brightfield_shell::data_file::{self, Opened};
use brightfield_shell::design::Mode;
use brightfield_shell::discovery::{Discovery, Pass};
use brightfield_shell::model_pane::{
    self, Dispatched, ModelPane, ModelView, FILE_PAUSE_MS, REDISCOVER_MODEL,
};
use brightfield_shell::own_files::OwnProtocol;
use brightfield_shell::run::Runner;

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-model-rerun-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db"] {
            std::fs::create_dir_all(root.join(sub)).expect("a scratch folder");
        }
        // Set once, before any test in this file reads them, and never changed.
        std::env::set_var(
            brightfield_shell::startup::CONFIG_DIR_VAR,
            root.join("config"),
        );
        std::env::set_var("ARCFORM_HISTORY_DIR", root.join("history"));
        std::env::set_var("ARCFORM_DB_DIR", root.join("db"));
        root
    })
}

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// The retail extract: eight CSVs, and the model written for them.
fn retail() -> PathBuf {
    scratch();
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../brightfield-ossie/fixtures/retail_extract")
}

/// The model written for the retail extract, with every join's evidence as measured on it.
fn retail_model() -> Model {
    let text = std::fs::read_to_string(retail().join("model.ossie.yaml")).unwrap();
    brightfield_ossie::read(&text).unwrap()
}

/// The retail extract's CSVs copied into a folder named `name`, with `extra` added to the end of
/// `transactions.csv`.
fn retail_copy(name: &str, extra: &str) -> PathBuf {
    let dir = scratch().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for entry in std::fs::read_dir(retail()).unwrap().flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if file.ends_with(".csv") {
            std::fs::copy(entry.path(), dir.join(&file)).unwrap();
        }
    }
    let transactions = dir.join("transactions.csv");
    let mut text = std::fs::read_to_string(&transactions).unwrap();
    text.push_str(extra);
    std::fs::write(&transactions, text).unwrap();
    dir
}

/// Held around each open that makes a Protocol folder.
fn folder_lock() -> MutexGuard<'static, ()> {
    static OPENS: Mutex<()> = Mutex::new(());
    OPENS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The folder at `dir` opened as a Protocol and run, with `model` saved over the one the open
/// found.
fn opened(dir: &Path, model: Model) -> ModelPane {
    let opened = {
        let _held = folder_lock();
        data_file::open_any(dir, &runner()).expect("it opens")
    };
    let Opened::Model(protocol) = opened else {
        panic!("several tables are a model");
    };
    let mut pane = ModelPane::open(protocol, runner()).expect("the model reads");
    pane.save_model(model).expect("the written model runs");
    pane
}

/// The model as the Ossie file in `pane`'s folder holds it.
fn written(pane: &ModelPane) -> Model {
    let text = std::fs::read_to_string(pane.protocol.dir.join("model.ossie.yaml")).unwrap();
    brightfield_ossie::read(&text).unwrap()
}

fn rel<'a>(m: &'a Model, name: &str) -> &'a brightfield_ossie::Relationship {
    m.relationships
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no join {name}"))
}

/// Draw `pane` once in a context of its own, and answer what its inspector drew.
fn inspector_lines(pane: &mut ModelPane) -> Vec<String> {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, 900.0),
        )),
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| pane.ui(ui, Mode::Light));
    pane.drawn()
        .inspector
        .clone()
        .expect("the inspector drew")
        .lines
}

const ORPHANS_SAID: &str = "2 transactions rows now point at no customers row";

#[test]
fn new_orphans_warn_without_regrading() {
    // Two sales to a customer the extract does not hold, and one to no customer at all, which is
    // no orphan: a NULL key points at nothing.
    let dir = retail_copy(
        "retail-orphans",
        "1009,99,1,BEV-001,1,4.50,2025-01-12\n\
         1010,99,2,BEV-001,1,4.50,2025-01-12\n\
         1011,,3,BEV-001,1,4.50,2025-01-12\n",
    );
    let mut pane = opened(&dir, retail_model());

    let before = rel(&retail_model(), "transactions_customers").meta.clone();
    let file = written(&pane);
    let r = rel(&file, "transactions_customers");
    assert_eq!(r.meta.warning.as_deref(), Some(ORPHANS_SAID));
    assert_eq!(r.meta.evidence.orphans, 2);
    assert_eq!(r.meta.evidence.child_rows, 11);
    // Five customers referenced, one of them (99) not found.
    assert!(
        (r.meta.evidence.value_overlap - 5.0 / 6.0).abs() < 1e-9,
        "{r:?}"
    );
    assert_eq!(r.meta.grade, before.grade);
    assert_eq!(r.meta.decided_by, before.decided_by);
    assert_eq!(r.meta.verified, before.verified);
    assert_eq!(pane.model, file, "the pane holds what the file holds");
    // The stores join measured the new rows and found every store.
    let stores = rel(&file, "transactions_stores");
    assert_eq!(stores.meta.evidence.child_rows, 11);
    assert_eq!(stores.meta.warning, None);
    // The folded table has no silver table of its own, so its join is left as written.
    assert_eq!(
        rel(&file, "products_categories"),
        rel(&retail_model(), "products_categories")
    );

    // The join's inspector says so.
    pane.focus.selected = Some(Selection::Relationship("transactions_customers".into()));
    let lines = inspector_lines(&mut pane);
    assert!(lines.iter().any(|l| l == ORPHANS_SAID), "{lines:?}");

    // Writing the evidence is not a run: saving the measured model again changes nothing.
    let text = pane.file_text().to_string();
    pane.save_model(pane.model.clone()).expect("it saves");
    assert_eq!(pane.file_text(), text);
}

#[test]
fn opening_over_a_run_protocol_measures_its_joins() {
    let dir = retail_copy(
        "retail-reopen",
        "1009,99,1,BEV-001,1,4.50,2025-01-12\n1010,98,2,BEV-001,1,4.50,2025-01-12\n",
    );
    let pane = opened(&dir, retail_model());
    // The written model, with no warning, put back over the measured one, as a file from before
    // the data changed would be.
    let protocol: OwnProtocol = pane.protocol.clone();
    drop(pane);
    std::fs::write(
        protocol.dir.join("model.ossie.yaml"),
        brightfield_ossie::write(&retail_model()),
    )
    .unwrap();
    let reopened = ModelPane::open(protocol, runner()).expect("the model reads");
    let r = rel(&reopened.model, "transactions_customers");
    assert_eq!(r.meta.warning.as_deref(), Some(ORPHANS_SAID));
    assert_eq!(r.meta.grade, Grade::Likely);
    assert_eq!(written(&reopened), reopened.model);
}

#[test]
fn added_join_gets_measured_evidence() {
    let mut pane = opened(&retail_copy("retail-added", ""), retail_model());
    // Measured on the data it was written for, the fixture's evidence is what it says.
    assert_eq!(pane.model.relationships, retail_model().relationships);
    let done = pane.dispatch(model_pane_add_join());
    assert!(matches!(done, Dispatched::Done(_)), "{done:?}");
    let file = written(&pane);
    let added = file
        .relationships
        .iter()
        .find(|r| r.from == "loyalty_visits" && r.to == "customer_segments")
        .expect("the added join");
    // Six visits, by five customers, one of whom (3) has no segment.
    assert_eq!(added.meta.evidence.child_rows, 6);
    assert_eq!(added.meta.evidence.orphans, 1);
    assert!(
        (added.meta.evidence.value_overlap - 0.8).abs() < 1e-9,
        "{added:?}"
    );
    assert_eq!(added.meta.evidence.parent_duplicates, 0);
    assert_eq!(added.meta.decided_by, DecidedBy::Analyst);
    assert_eq!(added.meta.grade, Grade::Possible);
    assert_eq!(added.meta.verified, Verified::Unmeasured);
    assert_eq!(
        added.meta.warning, None,
        "a possible join's orphans do not warn"
    );
}

fn model_pane_add_join() -> brightfield_shell::model_inspector::ModelAction {
    brightfield_shell::model_inspector::ModelAction::Edit(Edit::AddJoin {
        from: "loyalty_visits".into(),
        from_columns: vec!["customer_id".into()],
        to: "customer_segments".into(),
        to_columns: vec!["customer_id".into()],
    })
}

/// A discoverer that proposes `0` for both passes.
struct Proposes(Model);

impl Discovery for Proposes {
    fn discover(&self, _protocol: &OwnProtocol, _pass: Pass) -> Result<Model, String> {
        Ok(self.0.clone())
    }
}

#[test]
fn rediscover_keeps_analyst_decisions() {
    let mut pane = opened(&retail_copy("retail-rediscover", ""), retail_model());
    // The analyst keeps the possible join, drops the stores join and calls promo_codes a
    // reference table.
    for edit in [
        Edit::KeepJoin {
            relationship: "customer_segments_customers".into(),
        },
        Edit::DropJoin {
            relationship: "transactions_stores".into(),
        },
        Edit::SetRole {
            dataset: "promo_codes".into(),
            role: Role::Reference,
        },
    ] {
        let done = pane.dispatch(brightfield_shell::model_inspector::ModelAction::Edit(edit));
        assert!(matches!(done, Dispatched::Done(_)), "{done:?}");
    }

    // A fresh pass that would undo all three, and changes what Dovetail decided besides: stores
    // becomes a reference table, and the loyalty visits' stores join is no longer found.
    let mut proposed = retail_model();
    for d in &mut proposed.datasets {
        match d.name.as_str() {
            "promo_codes" => d.meta.role = Role::Event,
            "stores" => d.meta.role = Role::Reference,
            _ => {}
        }
    }
    for r in &mut proposed.relationships {
        if r.name == "customer_segments_customers" {
            r.meta.evidence.name_similarity = 0.5;
        }
    }
    proposed
        .relationships
        .retain(|r| r.name != "loyalty_visits_stores");
    let done = pane.rediscover(Proposes(proposed));
    assert!(matches!(done, Dispatched::Done(_)), "{done:?}");

    let file = written(&pane);
    assert_eq!(pane.model, file, "the pane holds what the file holds");
    let role = |name: &str| {
        file.datasets
            .iter()
            .find(|d| d.name == name)
            .unwrap()
            .meta
            .role
    };
    // The analyst's decisions stand.
    assert_eq!(role("promo_codes"), Role::Reference);
    let kept = rel(&file, "customer_segments_customers");
    assert_eq!(kept.meta.decided_by, DecidedBy::Analyst);
    assert!((kept.meta.evidence.name_similarity - 1.0).abs() < f64::EPSILON);
    assert!(!file
        .relationships
        .iter()
        .any(|r| r.name == "transactions_stores"));
    assert!(file
        .meta
        .rejected
        .iter()
        .any(|r| r.name == "transactions_stores"));
    // What Dovetail decided takes the fresh pass.
    assert_eq!(role("stores"), Role::Reference);
    assert!(!file
        .relationships
        .iter()
        .any(|r| r.name == "loyalty_visits_stores"));
}

#[test]
fn rediscover_is_a_durable_verb_of_the_model() {
    let reg = brightfield_keys::registry();
    let v = reg
        .iter()
        .find(|v| v.longname == REDISCOVER_MODEL)
        .expect("rediscover-model is registered");
    assert_eq!(v.tier, CommandTier::Data, "it rewrites the Ossie file");
    assert_eq!(v.drives, Drives::SpecEdit);
    assert_eq!(v.scope_applicability, vec![Altitude::Model]);
    assert!(v.scores.is_some());
    assert!(model_pane::is_model_verb(REDISCOVER_MODEL));
    assert!(model_pane::model_palette_verbs().contains(&REDISCOVER_MODEL));

    // With the stand-in discoverer, which proposes the model on disk, nothing changes.
    let mut pane = opened(&retail_copy("retail-rediscover-verb", ""), retail_model());
    let text = pane.file_text().to_string();
    let done = pane
        .perform_reshape(REDISCOVER_MODEL)
        .expect("a model verb");
    assert!(matches!(done, Dispatched::Done(_)), "{done:?}");
    assert_eq!(pane.file_text(), text);
}

#[test]
fn a_hand_edit_never_claims_measured_evidence() {
    // Sales to a customer the extract does not hold, so measuring the written model warns.
    let dir = retail_copy(
        "retail-hand-edit",
        "1009,99,1,BEV-001,1,4.50,2025-01-12\n1010,98,2,BEV-001,1,4.50,2025-01-12\n",
    );
    let opened = {
        let _held = folder_lock();
        data_file::open_any(&dir, &runner()).expect("it opens")
    };
    let Opened::Model(protocol) = opened else {
        panic!("several tables are a model");
    };
    let mut pane = ModelPane::open(protocol, runner()).expect("the model reads");

    // The file view's editor takes the keyboard.
    let ctx = egui::Context::default();
    let frame = |pane: &mut ModelPane, events: Vec<egui::Event>| {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| pane.ui(ui, Mode::Light));
    };
    pane.view = ModelView::File;
    frame(&mut pane, Vec::new());
    let at = pane.drawn().file_editor.expect("the editor drew").center();
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(&mut pane, vec![egui::Event::PointerMoved(at), button(true)]);
    frame(&mut pane, vec![button(false)]);

    // The written model runs and is measured while the editor holds the keyboard: the file on
    // disk warns, and the editor still holds text without the warning.
    pane.save_model(retail_model())
        .expect("the written model runs");
    assert_eq!(
        rel(&written(&pane), "transactions_customers")
            .meta
            .warning
            .as_deref(),
        Some(ORPHANS_SAID)
    );

    // An unrelated hand edit to the text from before the measurement: promo_codes made a
    // reference table.
    let stale = brightfield_ossie::write(&retail_model());
    let promo = r#"data: '{"role":"unclear""#;
    assert_eq!(stale.matches(promo).count(), 1, "{stale}");
    let typed = stale.replacen(promo, r#"data: '{"role":"reference""#, 1);
    pane.set_file_draft(typed.clone());
    frame(&mut pane, Vec::new());
    assert_eq!(pane.tick_file_edit(0), None);
    let read = pane
        .tick_file_edit(FILE_PAUSE_MS)
        .expect("the edit is read");
    assert_eq!(read, Ok(()));
    assert_eq!(
        pane.file_draft(),
        typed,
        "the focused editor keeps what was typed"
    );

    let file = written(&pane);
    let promo_codes = file
        .datasets
        .iter()
        .find(|d| d.name == "promo_codes")
        .unwrap();
    assert_eq!(promo_codes.meta.role, Role::Reference);
    assert_eq!(promo_codes.meta.decided_by, DecidedBy::Analyst);
    // The join whose evidence was measured stays Dovetail's, with the measured evidence.
    let r = rel(&file, "transactions_customers");
    assert_eq!(r.meta.decided_by, DecidedBy::Dovetail);
    assert_eq!(r.meta.warning.as_deref(), Some(ORPHANS_SAID));
    assert_eq!(r.meta.evidence.orphans, 2);
    assert_eq!(r.meta.evidence.child_rows, 10);
    for r in &file.relationships {
        assert_eq!(r.meta.decided_by, DecidedBy::Dovetail, "{}", r.name);
    }
}
