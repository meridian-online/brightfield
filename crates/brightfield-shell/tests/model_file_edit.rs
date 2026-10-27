//! Editing the Ossie file by hand in the model pane's file view. Each pause in typing reads the
//! text: a document Brightfield reads becomes the model, with what the edit changed marked as the
//! analyst's, and the diagram follows; one it cannot read leaves the last good model standing and
//! says why, at its line. What a readable edit names that Brightfield cannot act on is listed
//! under the editor, one warning each: a join to a missing table and a metric stay in the file,
//! and a role outside the six is ignored, so the file always reads. The editor keeps what was
//! typed while it holds the keyboard, and a reshape reads a waiting edit first.
//!
//! The Protocols here are opened through the app's own open, with the `brightfield-shell` binary
//! cargo built as their runner, under a scratch folder of this file's that the config directory
//! and arc's directories point at before anything reads them. Nothing here needs a FineType
//! bundle, and nothing opens a GPU device: the pane lays out the same without a raster. The pause
//! is driven with an explicit clock, so nothing sleeps.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use brightfield_ossie::{Band, DecidedBy, Edit, ListRow, Model, Role};
use brightfield_render::model_scene::Selection;
use brightfield_shell::data_file::{self, Opened};
use brightfield_shell::design::Mode;
use brightfield_shell::discovery::{Discovery, Pass};
use brightfield_shell::model_pane::{Dispatched, ModelPane, ModelView, FILE_PAUSE_MS};
use brightfield_shell::own_files::OwnProtocol;
use brightfield_shell::run::Runner;
use brightfield_shell::window::{Boot, MeridianApp};

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-model-file-edit-{}", std::process::id()));
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

fn retail_model() -> Model {
    let text = std::fs::read_to_string(retail().join("model.ossie.yaml")).unwrap();
    brightfield_ossie::read(&text).unwrap()
}

/// Held around each open that makes a Protocol folder: the opens share one.
fn folder_lock() -> MutexGuard<'static, ()> {
    static OPENS: Mutex<()> = Mutex::new(());
    OPENS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The retail extract opened as a Protocol and run, with its written model saved over the one the
/// open found. The lock is held for the test's whole life, since a hand edit runs the Protocol
/// in the folder every open shares.
fn opened() -> (ModelPane, MutexGuard<'static, ()>) {
    let held = folder_lock();
    let Opened::Model(protocol) = data_file::open_any(&retail(), &runner()).expect("it opens")
    else {
        panic!("several tables are a model");
    };
    let mut pane = ModelPane::open(protocol, runner()).expect("the model reads");
    pane.save_model(retail_model())
        .expect("the written model runs");
    (pane, held)
}

/// A pane over the written retail model, with no Protocol or database behind it: enough for an
/// edit that writes nothing.
fn written_retail(name: &str) -> ModelPane {
    let dir = scratch().join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(
        retail().join("model.ossie.yaml"),
        dir.join("model.ossie.yaml"),
    )
    .unwrap();
    let protocol = brightfield_shell::own_files::OwnProtocol {
        dir,
        sources: brightfield_engine::sources::SourceSet::default(),
        profiles: Vec::new(),
    };
    ModelPane::open(protocol, runner()).expect("the retail model reads")
}

/// The Ossie file as it is on disk in `pane`'s folder.
fn on_disk(pane: &ModelPane) -> String {
    std::fs::read_to_string(pane.protocol.dir.join("model.ossie.yaml")).unwrap()
}

/// Type `text` into the file view as one edit, then let the pause pass.
fn type_and_pause(pane: &mut ModelPane, text: String) {
    pane.set_file_draft(text);
    assert_eq!(pane.tick_file_edit(0), None, "an edit arms the pause");
    assert_eq!(
        pane.tick_file_edit(FILE_PAUSE_MS - 1),
        None,
        "nothing is read before the pause is out"
    );
    assert!(
        pane.tick_file_edit(FILE_PAUSE_MS).is_some(),
        "the text is read once the pause is out"
    );
}

/// `text` with the one occurrence of `from` replaced by `to`.
fn replaced(text: &str, from: &str, to: &str) -> String {
    assert_eq!(text.matches(from).count(), 1, "{from} occurs once");
    text.replacen(from, to, 1)
}

/// The customers dataset's Meridian entry, as written for the retail model.
const CUSTOMERS_ROLE: &str =
    r#"data: '{"role":"entity","role_evidence":["one row per customer_id""#;

fn card_band(pane: &ModelPane, dataset: &str) -> Option<Band> {
    pane.layout()
        .cards
        .iter()
        .find(|c| c.dataset == dataset)
        .map(|c| c.band)
}

fn button(at: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// Draw `pane` once, with `events`, in a context of its own.
fn draw(ctx: &egui::Context, pane: &mut ModelPane, events: Vec<egui::Event>) {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, 900.0),
        )),
        events,
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| pane.ui(ui, Mode::Light));
}

#[test]
fn broken_yaml_keeps_last_good() {
    let mut pane = written_retail("broken-yaml");
    let model = pane.model.clone();
    let layout = pane.layout().clone();
    let disk = on_disk(&pane);
    let text = pane.file_text().to_string();
    assert_eq!(text.lines().nth(2), Some("datasets:"));

    type_and_pause(
        &mut pane,
        replaced(&text, "\ndatasets:\n", "\ndatasets: :\n"),
    );

    let error = pane
        .file_error()
        .expect("the broken text is refused")
        .to_string();
    assert!(error.starts_with("Line 3:"), "{error}");
    assert_eq!(pane.model, model, "the last good model stands");
    assert_eq!(pane.layout(), &layout, "the diagram is unchanged");
    assert_eq!(on_disk(&pane), disk, "nothing is written");
    assert!(
        pane.file_draft().contains("datasets: :"),
        "the analyst's text stays in the editor to be put right"
    );
    let ctx = egui::Context::default();
    pane.view = ModelView::File;
    draw(&ctx, &mut pane, Vec::new());
    assert!(
        pane.drawn().file_notes.contains(&error),
        "the error stands under the editor: {:?}",
        pane.drawn().file_notes
    );
}

#[test]
fn valid_edit_redraws() {
    let (mut pane, _held) = opened();
    assert_eq!(card_band(&pane, "customers"), Some(Band::Entities));
    let text = pane.file_text().to_string();
    let edited = replaced(
        &text,
        CUSTOMERS_ROLE,
        &CUSTOMERS_ROLE.replace("entity", "event"),
    );

    type_and_pause(&mut pane, edited);

    assert_eq!(pane.file_error(), None);
    assert_eq!(card_band(&pane, "customers"), Some(Band::Events));
    let disk = brightfield_ossie::read(&on_disk(&pane)).unwrap();
    assert_eq!(disk, pane.model, "the pane holds what the file holds");
    assert_eq!(pane.file_text(), on_disk(&pane));
}

#[test]
fn hand_edit_marks_analyst() {
    let (mut pane, _held) = opened();
    let text = pane.file_text().to_string();
    // The role of customers, the expression of its email field, the keys of one join, a trailing
    // space after another's key, and a third join taken out.
    let edited = replaced(
        &text,
        CUSTOMERS_ROLE,
        &CUSTOMERS_ROLE.replace("entity", "reference"),
    );
    let edited = replaced(&edited, "sql: email\n", "sql: lower(email)\n");
    let edited = replaced(
        &edited,
        "- name: loyalty_visits_customers\n  from: loyalty_visits\n  to: customers\n  from_columns:\n  - customer_id\n",
        "- name: loyalty_visits_customers\n  from: loyalty_visits\n  to: customers\n  from_columns:\n  - visit_id\n",
    );
    let edited = replaced(
        &edited,
        "- name: transactions_stores\n  from: transactions\n  to: stores\n  from_columns:\n  - store_id\n",
        "- name: transactions_stores\n  from: transactions\n  to: stores\n  from_columns:\n  - store_id \n",
    );
    let taken_out = {
        let start = edited.find("- name: loyalty_visits_stores\n").unwrap();
        let end = start
            + edited[start..]
                .find("- name: customer_segments_customers\n")
                .unwrap();
        format!("{}{}", &edited[..start], &edited[end..])
    };

    type_and_pause(&mut pane, taken_out);
    assert_eq!(pane.file_error(), None);

    let disk = brightfield_ossie::read(&on_disk(&pane)).unwrap();
    let dataset = |name: &str| disk.datasets.iter().find(|d| d.name == name).unwrap();
    let customers = dataset("customers");
    assert_eq!(customers.meta.role, Role::Reference);
    assert_eq!(customers.meta.decided_by, DecidedBy::Analyst);
    let field = |name: &str| customers.fields.iter().find(|f| f.name == name).unwrap();
    assert_eq!(field("email").meta.decided_by, DecidedBy::Analyst);
    assert_eq!(
        field("name").meta.decided_by,
        DecidedBy::Dovetail,
        "a field the edit did not touch keeps who decided it"
    );
    assert_eq!(dataset("stores").meta.decided_by, DecidedBy::Dovetail);
    // A trailing space is not a change to the keys, so the join is untouched.
    let join = |name: &str| disk.relationships.iter().find(|r| r.name == name).unwrap();
    assert_eq!(
        join("transactions_stores").meta.decided_by,
        DecidedBy::Dovetail
    );
    assert_eq!(
        join("transactions_customers").meta.decided_by,
        DecidedBy::Dovetail
    );
    assert_eq!(
        join("loyalty_visits_customers").meta.decided_by,
        DecidedBy::Analyst
    );
    // The join taken out is gone, and rejected as the analyst's, as a Drop would leave it.
    assert!(!disk
        .relationships
        .iter()
        .any(|r| r.name == "loyalty_visits_stores"));
    let rejected = disk
        .meta
        .rejected
        .iter()
        .find(|r| r.name == "loyalty_visits_stores")
        .expect("the join taken out is rejected");
    assert_eq!(rejected.meta.decided_by, DecidedBy::Analyst);
}

/// A discoverer whose every pass proposes `0`.
struct Proposes(Model);

impl Discovery for Proposes {
    fn discover(&self, _protocol: &OwnProtocol, _pass: Pass) -> Result<Model, String> {
        Ok(self.0.clone())
    }
}

/// `text` without the list item that begins `- name: {name}` at the start of a line: up to the
/// next line that starts a sibling item or a key of the document.
fn cut_item(text: &str, name: &str) -> String {
    let head = format!("- name: {name}\n");
    assert_eq!(text.matches(&head).count(), 1, "{name} is one item");
    let start = text.find(&head).unwrap();
    let rest = &text[start + head.len()..];
    let mut end = start + head.len();
    for line in rest.split_inclusive('\n') {
        if !line.starts_with(' ') {
            break;
        }
        end += line.len();
    }
    format!("{}{}", &text[..start], &text[end..])
}

#[test]
fn a_join_deleted_by_hand_stays_out_after_a_full_pass() {
    let (mut pane, _held) = opened();
    let text = pane.file_text().to_string();
    type_and_pause(&mut pane, cut_item(&text, "transactions_stores"));
    assert_eq!(pane.file_error(), None);
    assert!(!pane
        .model
        .relationships
        .iter()
        .any(|r| r.name == "transactions_stores"));

    // A full pass that still finds the join, as dovetail would.
    pane.start_full_pass(Proposes(retail_model()));
    assert!(pane.finish_discovery().is_some(), "the pass lands");
    let disk = brightfield_ossie::read(&on_disk(&pane)).unwrap();
    for model in [&pane.model, &disk] {
        assert!(
            !model
                .relationships
                .iter()
                .any(|r| r.from == "transactions" && r.to == "stores"),
            "the pass did not bring the join back"
        );
        assert!(model
            .meta
            .rejected
            .iter()
            .any(|r| r.name == "transactions_stores" && r.meta.decided_by == DecidedBy::Analyst));
    }
}

#[test]
fn a_dataset_deleted_by_hand_stays_left_out() {
    let (mut pane, _held) = opened();
    let text = pane.file_text().to_string();
    type_and_pause(&mut pane, cut_item(&text, "promo_codes"));
    assert_eq!(pane.file_error(), None);
    let left_out = |model: &Model| {
        let d = model
            .datasets
            .iter()
            .find(|d| d.name == "promo_codes")
            .expect("the deleted table stays in the model");
        d.meta.excluded && d.meta.decided_by == DecidedBy::Analyst
    };
    assert!(left_out(&pane.model), "deleting a table leaves it out");
    assert!(brightfield_ossie::list_rows(&pane.model)
        .iter()
        .any(|row| matches!(row, ListRow::LeftOut { name, .. } if name == "promo_codes")));
    assert_eq!(
        card_band(&pane, "promo_codes"),
        None,
        "no card on the diagram"
    );

    // A full pass that proposes it as a table of the model again.
    pane.start_full_pass(Proposes(retail_model()));
    assert!(pane.finish_discovery().is_some(), "the pass lands");
    assert!(left_out(&pane.model), "the pass did not bring it back");
    assert!(left_out(&brightfield_ossie::read(&on_disk(&pane)).unwrap()));
}

#[test]
fn a_join_renamed_through_another_name_is_not_rejected() {
    let (mut pane, _held) = opened();
    let text = pane.file_text().to_string();
    type_and_pause(
        &mut pane,
        replaced(
            &text,
            "- name: transactions_stores\n",
            "- name: transactions_stor\n",
        ),
    );
    let text = pane.file_text().to_string();
    type_and_pause(
        &mut pane,
        replaced(
            &text,
            "- name: transactions_stor\n",
            "- name: transactions_stores\n",
        ),
    );
    assert_eq!(pane.file_error(), None);
    let disk = brightfield_ossie::read(&on_disk(&pane)).unwrap();
    for model in [&pane.model, &disk] {
        assert!(model
            .relationships
            .iter()
            .any(|r| r.name == "transactions_stores"));
        let rejected: Vec<&str> = model
            .meta
            .rejected
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert!(
            !rejected.contains(&"transactions_stores") && !rejected.contains(&"transactions_stor"),
            "{rejected:?}"
        );
    }
}

#[test]
fn a_join_a_pass_adds_under_a_focused_editor_is_not_deleted_by_hand() {
    let (mut pane, _held) = opened();
    let ctx = egui::Context::default();
    pane.view = ModelView::File;
    draw(&ctx, &mut pane, Vec::new());
    let at = pane.drawn().file_editor.expect("the editor drew").center();
    draw(
        &ctx,
        &mut pane,
        vec![egui::Event::PointerMoved(at), button(at, true)],
    );
    draw(&ctx, &mut pane, vec![button(at, false)]);
    let base = pane.file_draft().to_string();

    // A full pass finds a join the editor's text has never held, while the editor holds the
    // keyboard.
    let mut proposed = retail_model();
    let mut found = proposed
        .relationships
        .iter()
        .find(|r| r.name == "loyalty_visits_customers")
        .unwrap()
        .clone();
    found.name = "loyalty_visits_customer_segments".into();
    found.to = "customer_segments".into();
    proposed.relationships.push(found);
    pane.start_full_pass(Proposes(proposed));
    assert!(pane.finish_discovery().is_some(), "the pass lands");
    draw(&ctx, &mut pane, Vec::new());
    assert_eq!(pane.file_draft(), base, "the focused editor keeps its text");
    let active =
        |pane: &ModelPane, name: &str| pane.model.relationships.iter().any(|r| r.name == name);
    let rejected =
        |pane: &ModelPane, name: &str| pane.model.meta.rejected.iter().any(|r| r.name == name);
    assert!(active(&pane, "loyalty_visits_customer_segments"));

    // An unrelated edit to the editor's text, which lacks the join: the join is not deleted.
    type_and_pause(
        &mut pane,
        replaced(
            &base,
            CUSTOMERS_ROLE,
            &CUSTOMERS_ROLE.replace("entity", "reference"),
        ),
    );
    assert_eq!(pane.file_error(), None);
    assert!(active(&pane, "loyalty_visits_customer_segments"));
    assert!(!rejected(&pane, "loyalty_visits_customer_segments"));

    // A join the editor's text held, deleted from it, is.
    let draft = pane.file_draft().to_string();
    type_and_pause(&mut pane, cut_item(&draft, "transactions_stores"));
    assert_eq!(pane.file_error(), None);
    assert!(!active(&pane, "transactions_stores"));
    assert!(rejected(&pane, "transactions_stores"));
    assert!(active(&pane, "loyalty_visits_customer_segments"));
    assert!(!rejected(&pane, "loyalty_visits_customer_segments"));
    let disk = brightfield_ossie::read(&on_disk(&pane)).unwrap();
    assert!(disk
        .relationships
        .iter()
        .any(|r| r.name == "loyalty_visits_customer_segments"));
    assert!(disk
        .meta
        .rejected
        .iter()
        .any(|r| r.name == "transactions_stores"));
}

#[test]
fn unknown_dataset_warns_and_rest_applies() {
    let (mut pane, _held) = opened();
    let text = pane.file_text().to_string();
    let edited = replaced(
        &text,
        CUSTOMERS_ROLE,
        &CUSTOMERS_ROLE.replace("entity", "event"),
    );
    let edited = replaced(
        &edited,
        "relationships:\n",
        "relationships:\n- name: transactions_suppliers\n  from: transactions\n  to: suppliers\n  from_columns:\n  - store_id\n  to_columns:\n  - supplier_id\n",
    );

    type_and_pause(&mut pane, edited);

    assert_eq!(pane.file_error(), None);
    assert_eq!(
        pane.file_warnings(),
        vec!["transactions_suppliers joins suppliers, which is not a table in the model: it stays in the file, and is left out of the diagram and the silver tables".to_string()]
    );
    // The rest of the edit took effect.
    assert_eq!(card_band(&pane, "customers"), Some(Band::Events));
    // The join to nothing is in the file, as the analyst wrote it, and nowhere else.
    let disk = on_disk(&pane);
    assert!(disk.contains("to: suppliers"), "{disk}");
    assert!(!pane
        .model
        .relationships
        .iter()
        .any(|r| r.name == "transactions_suppliers"));
    assert!(!pane
        .layout()
        .edges
        .iter()
        .any(|e| e.relationship == "transactions_suppliers"));
    assert!(pane.file_text().contains("to: suppliers"));
}

#[test]
fn an_unknown_role_is_ignored_and_the_file_reopens() {
    let (mut pane, _held) = opened();
    let text = pane.file_text().to_string();
    let edited = replaced(
        &text,
        CUSTOMERS_ROLE,
        &CUSTOMERS_ROLE.replace("entity", "fact"),
    );
    let edited = replaced(
        &edited,
        "relationships:\n",
        "metrics:\n- name: revenue\n  expression:\n    sql: SUM(amount)\nrelationships:\n",
    );
    let edited = replaced(&edited, "sql: email\n", "sql: lower(email)\n");

    type_and_pause(&mut pane, edited);

    assert_eq!(pane.file_error(), None);
    assert_eq!(
        pane.file_warnings(),
        vec![
            "role 'fact' is not one of the six; customers stays entity".to_string(),
            "The metric revenue stays in the file: Brightfield does not act on metrics".to_string(),
        ]
    );
    // The rest of the edit took effect, and the role the table had stands.
    assert_eq!(card_band(&pane, "customers"), Some(Band::Entities));
    let email = pane
        .model
        .datasets
        .iter()
        .find(|d| d.name == "customers")
        .and_then(|d| d.fields.iter().find(|f| f.name == "email"))
        .unwrap();
    assert_eq!(email.expression_sql, "lower(email)");
    // The file keeps the role the table had, so it reads; the editor keeps what was typed.
    let disk = on_disk(&pane);
    assert!(!disk.contains("fact"), "{disk}");
    assert!(disk.contains("name: revenue"), "{disk}");
    assert!(pane.file_draft().contains(r#""role":"fact""#));
    let reopened = ModelPane::open(pane.protocol.clone(), runner()).expect("the folder reopens");
    let customers = reopened
        .model
        .datasets
        .iter()
        .find(|d| d.name == "customers")
        .unwrap();
    assert_eq!(customers.meta.role, Role::Entity);
}

#[test]
fn a_read_error_beside_an_unknown_role_names_the_typed_line() {
    let mut pane = written_retail("read-error-line");
    // A comment, which a document written again would not keep, so every line after it would
    // move up one.
    let text = format!("# Notes on the retail model\n{}", pane.file_text());
    let edited = replaced(
        &text,
        CUSTOMERS_ROLE,
        &CUSTOMERS_ROLE.replace("entity", "fact"),
    );
    // The first field of stores, with Meridian data the reader refuses.
    let city = edited.lines().position(|l| l == "  - name: city").unwrap();
    let mut lines: Vec<&str> = edited.lines().collect();
    let data = city + 5;
    assert_eq!(lines[data], r#"      data: '{"decided_by":"dovetail"}'"#);
    lines[data] = r#"      data: '{"decided_by":"nobody"}'"#;
    let edited = format!("{}\n", lines.join("\n"));

    type_and_pause(&mut pane, edited);

    let error = pane.file_error().expect("the field's data is refused");
    assert!(error.starts_with(&format!("Line {}:", city + 1)), "{error}");
}

#[test]
fn an_applied_edit_leaves_the_focused_editor_as_typed() {
    let (mut pane, _held) = opened();
    let ctx = egui::Context::default();
    pane.view = ModelView::File;
    draw(&ctx, &mut pane, Vec::new());
    let at = pane.drawn().file_editor.expect("the editor drew").center();
    draw(
        &ctx,
        &mut pane,
        vec![egui::Event::PointerMoved(at), button(at, true)],
    );
    draw(&ctx, &mut pane, vec![button(at, false)]);

    // A join to a missing table at the head of the joins, which the file writes at their end.
    let typed = replaced(
        pane.file_text(),
        "relationships:\n",
        "relationships:\n- name: transactions_suppliers\n  from: transactions\n  to: suppliers\n  from_columns:\n  - store_id\n  to_columns:\n  - supplier_id\n",
    );
    pane.set_file_draft(typed.clone());
    draw(&ctx, &mut pane, Vec::new());
    type_and_pause(&mut pane, typed.clone());
    draw(&ctx, &mut pane, Vec::new());
    assert_eq!(pane.tick_file_edit(FILE_PAUSE_MS + 1), None);
    assert_ne!(
        on_disk(&pane),
        typed,
        "the file is written as the writer emits it"
    );
    assert_eq!(
        pane.file_draft(),
        typed,
        "the focused editor keeps what was typed"
    );

    // Let go of the keyboard, and the editor takes the file.
    draw(&ctx, &mut pane, vec![key(egui::Key::Escape)]);
    draw(&ctx, &mut pane, Vec::new());
    assert_eq!(pane.tick_file_edit(FILE_PAUSE_MS + 2), None);
    assert_eq!(pane.file_draft(), on_disk(&pane));
}

#[test]
fn a_reshape_reads_a_waiting_edit_first() {
    let (mut pane, _held) = opened();
    let edited = replaced(
        pane.file_text(),
        CUSTOMERS_ROLE,
        &CUSTOMERS_ROLE.replace("entity", "event"),
    );
    // Typed, and the reshape comes before the pause is out.
    pane.set_file_draft(edited);
    pane.focus.selected = Some(Selection::Relationship("transactions_stores".into()));
    let done = pane.perform_reshape("drop-join").expect("a reshape verb");
    assert!(matches!(done, Dispatched::Done(_)), "{done:?}");

    let disk = brightfield_ossie::read(&on_disk(&pane)).unwrap();
    let customers = disk
        .datasets
        .iter()
        .find(|d| d.name == "customers")
        .unwrap();
    assert_eq!(
        customers.meta.role,
        Role::Event,
        "the waiting edit was read"
    );
    assert!(!disk
        .relationships
        .iter()
        .any(|r| r.name == "transactions_stores"));
}

#[test]
fn a_refused_edit_blocks_a_reshape_until_discarded() {
    let (mut pane, _held) = opened();
    let disk = on_disk(&pane);
    let broken = replaced(pane.file_text(), "\ndatasets:\n", "\ndatasets: :\n");
    pane.set_file_draft(broken.clone());
    pane.focus.selected = Some(Selection::Relationship("transactions_stores".into()));

    let refused = pane.perform_reshape("drop-join").expect("a reshape verb");
    assert_eq!(
        refused,
        Dispatched::Failed(
            "The file has an edit that doesn't read — fix or discard it first".to_string()
        )
    );
    assert_eq!(on_disk(&pane), disk, "nothing is written");
    assert_eq!(pane.file_draft(), broken, "the edit stays to be put right");
    assert!(pane.file_error().is_some_and(|e| e.starts_with("Line 3:")));

    // Discard edit, under the editor, puts the file back.
    let ctx = egui::Context::default();
    pane.view = ModelView::File;
    draw(&ctx, &mut pane, Vec::new());
    let discard = pane
        .drawn()
        .file_discard
        .expect("a refused edit offers Discard");
    draw(
        &ctx,
        &mut pane,
        vec![
            egui::Event::PointerMoved(discard.center()),
            button(discard.center(), true),
        ],
    );
    draw(&ctx, &mut pane, vec![button(discard.center(), false)]);
    assert_eq!(pane.file_draft(), disk);
    assert_eq!(pane.file_error(), None);
    let done = pane.perform_reshape("drop-join").expect("a reshape verb");
    assert!(matches!(done, Dispatched::Done(_)), "{done:?}");
}

#[test]
fn a_join_to_a_missing_table_survives_a_reshape() {
    let (mut pane, _held) = opened();
    let edited = replaced(
        pane.file_text(),
        "relationships:\n",
        "relationships:\n- name: transactions_suppliers\n  from: transactions\n  to: suppliers\n  from_columns:\n  - store_id\n  to_columns:\n  - supplier_id\n",
    );
    type_and_pause(&mut pane, edited);
    assert!(on_disk(&pane).contains("to: suppliers"));

    let model = brightfield_ossie::apply(
        &pane.model,
        &Edit::DropJoin {
            relationship: "transactions_stores".into(),
        },
    )
    .unwrap();
    pane.save_model(model).expect("the reshaped model runs");

    let disk = on_disk(&pane);
    assert!(disk.contains("name: transactions_suppliers"), "{disk}");
    assert!(!disk.contains("name: transactions_stores"), "{disk}");
    assert_eq!(pane.file_warnings().len(), 1, "{:?}", pane.file_warnings());
}

#[test]
fn selection_scrolls_file() {
    let mut pane = written_retail("selection-scrolls");
    let text = pane.file_text().to_string();
    let line_of = |entry: &str| {
        text.lines()
            .position(|l| l == entry)
            .map(|i| i + 1)
            .unwrap_or_else(|| panic!("no line {entry}"))
    };
    let ctx = egui::Context::default();

    // Selected on the diagram, then shown in the file.
    pane.focus.selected = Some(Selection::Relationship("transactions_stores".into()));
    draw(&ctx, &mut pane, Vec::new());
    pane.view = ModelView::File;
    draw(&ctx, &mut pane, Vec::new());
    assert_eq!(
        pane.drawn().file_scrolled_to,
        Some(line_of("- name: transactions_stores"))
    );
    draw(&ctx, &mut pane, Vec::new());
    assert_eq!(
        pane.drawn().file_scrolled_to,
        None,
        "the file scrolls once, and is the analyst's to scroll after"
    );

    // Selected in the list, then shown in the file.
    pane.view = ModelView::List;
    pane.focus.selected = Some(Selection::Dataset("stores".into()));
    draw(&ctx, &mut pane, Vec::new());
    pane.view = ModelView::File;
    draw(&ctx, &mut pane, Vec::new());
    assert_eq!(
        pane.drawn().file_scrolled_to,
        Some(line_of("- name: stores"))
    );
}

/// One headless window that opened the retail extract into the model pane.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
}

impl Window {
    fn over_retail() -> Self {
        let mut win = Self {
            app: MeridianApp::headless(Boot::empty(), Mode::Light).running_with(Some(runner())),
            ctx: egui::Context::default(),
        };
        win.frame(Vec::new());
        win.app
            .open_path_knowing(&win.ctx, &retail().to_string_lossy(), &[]);
        let pane = win.app.model_pane_mut().expect("the folder opens a model");
        // The open's full pass lands before the test's model is saved over it.
        pane.finish_discovery();
        pane.save_model(retail_model())
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

    /// One keystroke as a keyboard sends it: the key, and the text it types.
    fn typed(&mut self, key: egui::Key, text: &str) {
        self.frame(vec![
            egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Text(text.into()),
        ]);
        self.frame(Vec::new());
    }
}

#[test]
fn typing_in_the_file_view_fires_no_reshape_key() {
    let _held = folder_lock();
    let mut win = Window::over_retail();
    let joined = "transactions_customers";
    {
        let pane = win.app.model_pane_mut().unwrap();
        pane.focus.selected = Some(Selection::Relationship(joined.into()));
        pane.view = ModelView::File;
    }
    win.frame(Vec::new());
    let editor = win
        .app
        .model_pane()
        .unwrap()
        .drawn()
        .file_editor
        .expect("the file view drew its editor");
    let at = editor.center();
    win.frame(vec![
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    win.frame(vec![egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    let disk_before = on_disk(win.app.model_pane().unwrap());

    // `y` keeps the selected join and `v` steps the view, but here each is a letter typed.
    win.typed(egui::Key::Y, "y");
    win.typed(egui::Key::V, "v");

    let pane = win.app.model_pane().unwrap();
    assert_eq!(pane.view, ModelView::File, "v stepped no view");
    assert_eq!(pane.status(), None, "no reshape verb ran");
    assert_eq!(on_disk(pane), disk_before, "nothing was written");
    let join = pane
        .model
        .relationships
        .iter()
        .find(|r| r.name == joined)
        .unwrap();
    assert_eq!(join.meta.decided_by, DecidedBy::Dovetail, "y kept no join");
    assert!(
        pane.file_draft().contains("yv"),
        "the letters went into the text"
    );
}
