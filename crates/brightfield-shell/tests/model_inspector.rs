//! The model pane's inspector: a join's evidence and a sample of its joined rows, a table's types
//! and the values that failed to cast, the actions each offers, and Open in grid, which puts the
//! rows in the grid with the pane kept behind it.
//!
//! The Protocols here are opened through the app's own open, with the `brightfield-shell` binary
//! cargo built as their runner, under a scratch folder of this file's that the config directory
//! and arc's directories point at before anything reads them. Nothing here needs a FineType
//! bundle: a column without a label passes through uncast, so the failures a test reads are ones
//! it made by retyping a column.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use brightfield_ossie::{Edit, Grade, Model};
use brightfield_render::model_scene::Selection;
use brightfield_shell::data_file::{self, Opened};
use brightfield_shell::design::Mode;
use brightfield_shell::model_inspector::{self, Inspected, ModelAction, PREVIEW_ROWS};
use brightfield_shell::model_pane::{Dispatched, ModelPane, ModelView};
use brightfield_shell::run::Runner;
use brightfield_shell::window::{Boot, MeridianApp};

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-model-inspector-{}", std::process::id()));
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

/// The model written for the retail extract, with a possible join and likely ones.
fn retail_model() -> Model {
    let text = std::fs::read_to_string(retail().join("model.ossie.yaml")).unwrap();
    brightfield_ossie::read(&text).unwrap()
}

/// Held around each open that makes a Protocol folder, so a test that checks no folder was made
/// cannot see one its siblings made meanwhile.
fn folder_lock() -> MutexGuard<'static, ()> {
    static OPENS: Mutex<()> = Mutex::new(());
    OPENS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The folder at `dir` opened as a Protocol and run, with `model` saved over the one the open
/// found, so its joins and types are the written ones.
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

/// The retail extract opened as a Protocol and run, with the model written for it.
fn opened_retail() -> ModelPane {
    opened(&retail(), retail_model())
}

/// A pane over the written retail model, with no database behind it.
fn written_retail() -> ModelPane {
    let dir = scratch().join("written-retail");
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

/// The {own} folder each opened Protocol is made in.
fn own_dir() -> PathBuf {
    brightfield_shell::starts::datasets_dir()
        .join("protocols")
        .join(brightfield_shell::own_files::OWN_DIR)
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

/// The two frames of a click at `at`: the press, then the release.
fn click(ctx: &egui::Context, pane: &mut ModelPane, at: egui::Pos2) {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    draw(ctx, pane, vec![egui::Event::PointerMoved(at), button(true)]);
    draw(ctx, pane, vec![button(false)]);
}

#[test]
fn preview_sql_samples_child_then_joins() {
    let model = retail_model();
    let sql = brightfield_protocol::preview::join_preview_sql(
        &model,
        "transactions_customers",
        PREVIEW_ROWS,
    );
    // The child side is sampled in its own subquery, and the join to the whole parent comes after.
    let child = "(SELECT * FROM \"transactions\" USING SAMPLE 20 ROWS) AS child";
    let at_child = sql.find(child).unwrap_or_else(|| panic!("{sql}"));
    let at_join = sql
        .find("JOIN \"customers\" AS parent")
        .unwrap_or_else(|| panic!("{sql}"));
    assert!(at_child < at_join, "{sql}");
    assert!(
        sql.contains("ON child.\"customer_id\" = parent.\"customer_id\""),
        "{sql}"
    );
    assert!(sql.ends_with("LIMIT 20"), "{sql}");
    assert_eq!(sql.matches("USING SAMPLE").count(), 1, "{sql}");

    // Open in grid reads every child row.
    let all = brightfield_protocol::preview::join_rows_sql(&model, "transactions_customers");
    assert!(
        !all.contains("USING SAMPLE") && !all.contains("LIMIT"),
        "{all}"
    );
    assert!(
        all.contains("FROM \"transactions\" AS child LEFT JOIN"),
        "{all}"
    );

    // A table folded into another has no silver table, so its side reads the raw one through
    // silver's cast projection.
    let fold = model
        .relationships
        .iter()
        .find(|r| r.to == "categories" || r.from == "categories")
        .expect("a join to the folded categories");
    let sql = brightfield_protocol::preview::join_preview_sql(&model, &fold.name, PREVIEW_ROWS);
    assert!(sql.contains("FROM \"raw_categories\""), "{sql}");

    assert!(brightfield_protocol::preview::join_preview_sql(&model, "no_such_join", 20).is_empty());
}

#[test]
fn preview_runs_on_retail() {
    let mut pane = opened_retail();
    let joined = pane
        .model
        .relationships
        .iter()
        .find(|r| {
            model_inspector::materialised(&pane.model, &r.from)
                && model_inspector::materialised(&pane.model, &r.to)
                && r.from != r.to
        })
        .cloned()
        .expect("the model joins two of the retail tables");
    pane.focus.selected = Some(Selection::Relationship(joined.name.clone()));
    let Some(Inspected::Join(details)) = pane.inspected().cloned() else {
        panic!("a join's inspector");
    };
    let rows = details.preview.expect("the preview runs");
    assert!(!rows.rows.is_empty(), "the sample holds rows");
    assert!(rows.rows.len() <= PREVIEW_ROWS, "{}", rows.rows.len());
    let from = format!("{}.", joined.from);
    let to = format!("{}.", joined.to);
    assert!(
        rows.columns.iter().any(|c| c.starts_with(&from)),
        "{rows:?}"
    );
    assert!(rows.columns.iter().any(|c| c.starts_with(&to)), "{rows:?}");
    assert!(
        !rows.columns.iter().any(|c| c.contains("__source")),
        "lineage is left out: {:?}",
        rows.columns
    );
    // Each sampled row found its parent: the parent's key matches the child's.
    let child_key = format!("{from}{}", joined.from_columns[0]);
    let parent_key = format!("{to}{}", joined.to_columns[0]);
    for i in 0..rows.rows.len() {
        if let Some(parent) = rows.value(i, &parent_key) {
            assert_eq!(rows.value(i, &child_key), Some(parent));
        }
    }

    // The evidence is listed, and how much of the data it was measured on.
    assert_eq!(details.evidence.len(), 4, "{:?}", details.evidence);
    assert!(
        [
            "Verified on the full data",
            "Sample only",
            "Not measured: the join was written by hand"
        ]
        .contains(&details.verified.as_str()),
        "{}",
        details.verified
    );
}

#[test]
fn table_inspector_lists_failed_rows_with_lineage() {
    let mut pane = opened_retail();
    // `name` holds people's names; read as whole numbers, every one of them fails.
    let retyped = brightfield_ossie::apply(
        &pane.model,
        &Edit::Retype {
            dataset: "customers".into(),
            field: "name".into(),
            finetype: "representation.numeric.integer_number".into(),
        },
    )
    .expect("customers has a name");
    pane.save_model(retyped).expect("the save runs");
    pane.focus.selected = Some(Selection::Dataset("customers".into()));
    let Some(Inspected::Table(details)) = pane.inspected().cloned() else {
        panic!("a table's inspector");
    };
    let failed = details
        .failed
        .expect("customers has tables")
        .expect("the failures read");
    for lineage in ["__source_file", "__source_part", "__source_row"] {
        assert!(failed.columns.iter().any(|c| c == lineage), "{failed:?}");
    }
    assert!(!failed.rows.is_empty(), "the names failed");
    for i in 0..failed.rows.len() {
        assert_eq!(failed.value(i, "column"), Some("name"));
        assert!(
            failed
                .value(i, "__source_file")
                .is_some_and(|f| f.ends_with("customers.csv")),
            "{failed:?}"
        );
        let row: i64 = failed.value(i, "__source_row").unwrap().parse().unwrap();
        assert!(row >= 1);
    }
    let name = details.columns.iter().find(|c| c.name == "name").unwrap();
    assert_eq!(name.failed, failed.rows.len() as u64);
    // The catalogue's title when a bundle is staged, and the label itself without one.
    assert!(
        ["Integer Number", "representation.numeric.integer_number"].contains(&name.chip.as_str()),
        "{}",
        name.chip
    );
    assert!(details.columns.iter().all(|c| !c.name.starts_with("__")));

    // Drawn, the inspector heads with the table, lists its columns and offers Open in grid.
    let ctx = egui::Context::default();
    draw(&ctx, &mut pane, Vec::new());
    let drawn = pane.drawn().inspector.clone().expect("the inspector drew");
    assert_eq!(drawn.title, "Table customers");
    assert!(
        drawn.lines.iter().any(|l| l.starts_with("name [")),
        "{drawn:?}"
    );
    for action in [
        "Change role",
        "Leave out",
        "Retype column\u{2026}",
        "Open in grid",
    ] {
        assert!(
            drawn.actions.iter().any(|a| a == action),
            "{action}: {drawn:?}"
        );
    }
    assert_eq!(
        drawn.table,
        Some((failed.columns.clone(), failed.rows.len()))
    );
}

#[test]
fn possible_join_offers_keep() {
    let mut pane = written_retail();
    let possible = pane
        .model
        .relationships
        .iter()
        .find(|r| r.meta.grade == Grade::Possible)
        .expect("the written model has a possible join")
        .name
        .clone();
    pane.focus.selected = Some(Selection::Relationship(possible.clone()));
    assert_eq!(
        pane.offered_actions(),
        vec![
            ModelAction::Keep(possible.clone()),
            ModelAction::Drop(possible.clone()),
            ModelAction::ChangeKeys(possible.clone()),
            ModelAction::ShowInFile,
            ModelAction::OpenInGrid(Selection::Relationship(possible.clone())),
        ]
    );
    let ctx = egui::Context::default();
    draw(&ctx, &mut pane, Vec::new());
    let drawn = pane.drawn().inspector.clone().expect("the inspector drew");
    assert_eq!(drawn.title, format!("Join {possible}"));
    assert_eq!(
        drawn.actions,
        [
            "Keep",
            "Drop",
            "Change keys\u{2026}",
            "Show in file",
            "Open in grid"
        ]
    );
    assert!(drawn
        .lines
        .iter()
        .any(|l| l == "Grade: possible, found by discovery"));
}

#[test]
fn likely_join_does_not_offer_keep() {
    let mut pane = written_retail();
    pane.focus.selected = Some(Selection::Relationship("transactions_customers".into()));
    let offered = pane.offered_actions();
    assert!(!offered.contains(&ModelAction::Keep("transactions_customers".into())));
    assert_eq!(
        offered.first(),
        Some(&ModelAction::Drop("transactions_customers".into()))
    );
    let ctx = egui::Context::default();
    draw(&ctx, &mut pane, Vec::new());
    let drawn = pane.drawn().inspector.clone().expect("the inspector drew");
    assert!(!drawn.actions.iter().any(|a| a == "Keep"), "{drawn:?}");
    assert!(drawn.lines.iter().any(|l| l == "Verified on the full data"));
    // No database stands behind this pane, and the preview says so rather than drawing nothing.
    assert!(
        drawn
            .lines
            .iter()
            .any(|l| l.starts_with("Could not read the rows")),
        "{drawn:?}"
    );
}

#[test]
fn a_list_row_selects_and_the_rest_dims() {
    let mut pane = written_retail();
    pane.view = ModelView::List;
    let ctx = egui::Context::default();
    draw(&ctx, &mut pane, Vec::new());
    assert!(pane.drawn().inspector.is_none(), "nothing selected yet");
    assert!(pane.drawn().list_lit.iter().all(|lit| *lit));
    let row = pane
        .drawn()
        .body
        .iter()
        .position(|line| line.contains(" · promo_codes"))
        .expect("promo_codes has a row");
    let at = pane.drawn().list_rows[row].center();
    click(&ctx, &mut pane, at);
    assert_eq!(
        pane.focus.selected,
        Some(Selection::Dataset("promo_codes".into()))
    );
    draw(&ctx, &mut pane, Vec::new());
    let drawn = pane.drawn().clone();
    // promo_codes joins nothing, so it alone stays lit.
    for (i, lit) in drawn.list_lit.iter().enumerate() {
        assert_eq!(*lit, i == row, "{}", drawn.body[i]);
    }
    assert_eq!(
        drawn.inspector.map(|i| i.title),
        Some("Table promo_codes".to_string())
    );
    // The diagram shares the selection: switching views keeps it and its inspector.
    pane.view = ModelView::Diagram;
    draw(&ctx, &mut pane, Vec::new());
    assert_eq!(
        pane.drawn().inspector.as_ref().map(|i| i.title.as_str()),
        Some("Table promo_codes")
    );
}

#[test]
fn the_inspectors_actions_change_the_model_through_the_pane() {
    let mut pane = opened_retail();
    let dir = pane.protocol.dir.clone();
    let dropped = pane.model.relationships[0].name.clone();
    pane.focus.selected = Some(Selection::Relationship(dropped.clone()));
    assert!(matches!(
        pane.dispatch(ModelAction::Drop(dropped.clone())),
        Dispatched::Done(_)
    ));
    let written =
        brightfield_ossie::read(&std::fs::read_to_string(dir.join("model.ossie.yaml")).unwrap())
            .unwrap();
    assert!(!written.relationships.iter().any(|r| r.name == dropped));
    assert!(written.meta.rejected.iter().any(|r| r.name == dropped));
    assert_eq!(pane.focus.selected, None, "the dropped join is let go");

    let table = pane.model.datasets[0].name.clone();
    assert!(matches!(
        pane.dispatch(ModelAction::ChangeRole {
            dataset: table.clone(),
            role: brightfield_ossie::Role::Bridge,
        }),
        Dispatched::Done(_)
    ));
    let d = pane
        .model
        .datasets
        .iter()
        .find(|d| d.name == table)
        .unwrap();
    assert_eq!(d.meta.role, brightfield_ossie::Role::Bridge);
    assert_eq!(d.meta.decided_by, brightfield_ossie::DecidedBy::Analyst);

    // The actions that need a choice open a picker, and change nothing yet.
    let joined = pane.model.relationships[0].name.clone();
    let before = pane.model.clone();
    for asks in [
        ModelAction::ChangeKeys(joined),
        ModelAction::FoldInto("customer_segments".into()),
        ModelAction::RetypeColumn(table.clone()),
    ] {
        assert!(
            matches!(pane.dispatch(asks.clone()), Dispatched::Picking(_)),
            "{asks:?}"
        );
        assert!(pane.picker().is_some(), "{asks:?}");
    }
    assert_eq!(pane.model, before);
    assert!(matches!(
        pane.dispatch(ModelAction::ChangeKeys("x".into())),
        Dispatched::Failed(_)
    ));
    assert!(matches!(
        pane.dispatch(ModelAction::Keep("no_such_join".into())),
        Dispatched::Failed(_)
    ));
    pane.dispatch(ModelAction::ShowInFile);
    assert_eq!(pane.view, ModelView::File);
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
}

fn folders_in(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |d| d.flatten().filter(|e| e.path().is_dir()).count())
}

#[test]
fn open_in_grid_shows_silver_rows() {
    // Held throughout, so the counts below see only the folders this test's window makes.
    let _held = folder_lock();
    let mut win = Window::over_retail();
    let dir = win.app.model_pane().unwrap().protocol.dir.clone();
    let folders = folders_in(&own_dir());
    let silver = model_inspector::query(&dir, "SELECT count(*) AS n FROM \"transactions\"")
        .expect("the silver table reads");
    let rows: u64 = silver.value(0, "n").unwrap().parse().unwrap();
    assert!(rows > 0);

    let ctx = win.ctx.clone();
    win.app.perform_model_action(
        &ctx,
        ModelAction::OpenInGrid(Selection::Dataset("transactions".into())),
    );
    win.frame(Vec::new());
    win.frame(Vec::new());
    let pane = win
        .app
        .model_pane()
        .expect("the pane is kept behind the grid");
    assert_eq!(pane.protocol.dir, dir);
    assert_eq!(
        win.app
            .chart_doc()
            .grid_drawn()
            .expect("the grid drew")
            .rows,
        rows,
        "the grid holds every silver row"
    );
    assert!(!win.app.front_door_is_live());

    // Back to the model: the same pane over the same folder, and no new Protocol made.
    assert!(win.app.back_to_model());
    win.frame(Vec::new());
    let pane = win.app.model_pane().expect("the pane is back");
    assert_eq!(pane.protocol.dir, dir);
    assert_eq!(
        pane.drawn().view,
        Some(ModelView::Diagram),
        "it draws again"
    );
    assert_eq!(folders_in(&own_dir()), folders, "no second Protocol folder");

    // A join opens its joined rows, and Esc is the way back.
    let join = pane
        .model
        .relationships
        .iter()
        .find(|r| {
            model_inspector::materialised(&pane.model, &r.from)
                && model_inspector::materialised(&pane.model, &r.to)
        })
        .expect("a join between two tables")
        .name
        .clone();
    assert!(win
        .app
        .open_model_in_grid(&ctx, &Selection::Relationship(join)));
    win.frame(Vec::new());
    win.frame(Vec::new());
    assert!(
        win.app
            .chart_doc()
            .grid_drawn()
            .expect("the grid drew")
            .rows
            > 0
    );
    win.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    win.frame(Vec::new());
    assert_eq!(
        win.app.model_pane().and_then(|p| p.drawn().view),
        Some(ModelView::Diagram)
    );
    assert!(win.app.chart_doc().grid_drawn().is_none());
    assert_eq!(folders_in(&own_dir()), folders);
}

/// Draw `pane` as the list, click the row whose text contains `needle`, and draw it again.
fn click_list_row(ctx: &egui::Context, pane: &mut ModelPane, needle: &str) {
    pane.view = ModelView::List;
    draw(ctx, pane, Vec::new());
    let row = pane
        .drawn()
        .body
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no row with {needle:?}: {:?}", pane.drawn().body));
    let at = pane.drawn().list_rows[row].center();
    click(ctx, pane, at);
    draw(ctx, pane, Vec::new());
}

/// Click the inspector's button reading `label`, and answer the actions it chose.
fn click_button(ctx: &egui::Context, pane: &mut ModelPane, label: &str) -> Vec<ModelAction> {
    let drawn = pane.drawn().inspector.clone().expect("the inspector drew");
    let at = drawn
        .actions
        .iter()
        .filter(|a| *a != "Change role")
        .position(|a| a == label)
        .unwrap_or_else(|| panic!("no {label:?} button: {:?}", drawn.actions));
    click(ctx, pane, drawn.button_rects[at].center());
    pane.take_actions()
}

#[test]
fn a_folded_table_is_selected_from_the_list_and_unfolded() {
    let mut pane = opened_retail();
    let ctx = egui::Context::default();
    click_list_row(&ctx, &mut pane, "categories · folded into products");
    assert_eq!(
        pane.focus.selected,
        Some(Selection::Dataset("categories".into()))
    );
    let drawn = pane.drawn().inspector.clone().expect("the inspector drew");
    assert_eq!(drawn.title, "Table categories");
    assert!(
        drawn.lines.iter().any(|l| l == "Folded into products"),
        "{drawn:?}"
    );
    let chosen = click_button(&ctx, &mut pane, "Unfold categories");
    assert_eq!(chosen, vec![ModelAction::Unfold("categories".into())]);
    for action in chosen {
        assert!(matches!(pane.dispatch(action), Dispatched::Done(_)));
    }
    let categories = pane
        .model
        .datasets
        .iter()
        .find(|d| d.name == "categories")
        .unwrap();
    assert_eq!(categories.meta.folded_into, None);
    // Unfolded, it has a row of its own in the list.
    draw(&ctx, &mut pane, Vec::new());
    assert!(
        pane.drawn()
            .body
            .iter()
            .any(|l| l.contains(" · categories · ") && !l.contains("folded into")),
        "{:?}",
        pane.drawn().body
    );
}

#[test]
fn a_hosts_inspector_unfolds_each_folded_table() {
    let mut pane = written_retail();
    pane.focus.selected = Some(Selection::Dataset("products".into()));
    let ctx = egui::Context::default();
    draw(&ctx, &mut pane, Vec::new());
    let drawn = pane.drawn().inspector.clone().expect("the inspector drew");
    assert!(
        drawn.actions.iter().any(|a| a == "Unfold categories"),
        "{drawn:?}"
    );
    assert!(drawn.actions.iter().any(|a| a == "Fold into\u{2026}"));
}

#[test]
fn a_left_out_table_is_selected_from_the_list_and_offers_include() {
    let mut pane = written_retail();
    pane.model = brightfield_ossie::apply(
        &pane.model,
        &Edit::Exclude {
            dataset: "promo_codes".into(),
        },
    )
    .unwrap();
    let ctx = egui::Context::default();
    click_list_row(&ctx, &mut pane, "left out · promo_codes");
    assert_eq!(
        pane.focus.selected,
        Some(Selection::Dataset("promo_codes".into()))
    );
    let drawn = pane.drawn().inspector.clone().expect("the inspector drew");
    assert_eq!(drawn.title, "Table promo_codes");
    assert!(drawn.lines.iter().any(|l| l == "Left out of the model"));
    assert!(!drawn.actions.iter().any(|a| a == "Leave out"), "{drawn:?}");
    let chosen = click_button(&ctx, &mut pane, "Include");
    assert_eq!(chosen, vec![ModelAction::Include("promo_codes".into())]);
}

#[test]
fn a_fold_joins_preview_casts_the_folded_side() {
    // The retail extract with a category key that is no number, so the raw key column is text.
    let dir = scratch().join("retail-n-a");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for entry in std::fs::read_dir(retail()).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".csv") {
            std::fs::copy(entry.path(), dir.join(&name)).unwrap();
        }
    }
    let categories = dir.join("categories.csv");
    let mut text = std::fs::read_to_string(&categories).unwrap();
    text.push_str("n/a,Unknown\n");
    std::fs::write(&categories, text).unwrap();

    // Both sides' key typed as a whole number: silver casts it, and `n/a` reads NULL.
    let mut model = retail_model();
    for dataset in ["products", "categories"] {
        model = brightfield_ossie::apply(
            &model,
            &Edit::Retype {
                dataset: dataset.into(),
                field: "category_id".into(),
                finetype: "representation.numeric.integer_number".into(),
            },
        )
        .unwrap();
    }
    let mut pane = opened(&dir, model);
    let fold = pane
        .model
        .relationships
        .iter()
        .find(|r| r.from == "products" && r.to == "categories")
        .expect("the fold join")
        .name
        .clone();
    pane.focus.selected = Some(Selection::Relationship(fold));
    let Some(Inspected::Join(details)) = pane.inspected().cloned() else {
        panic!("a join's inspector");
    };
    let rows = details.preview.expect("the preview runs despite the n/a");
    assert_eq!(rows.rows.len(), 5, "every product is sampled: {rows:?}");
    for i in 0..rows.rows.len() {
        // Every product's category is found: the keys compare as silver casts them.
        assert!(rows.value(i, "categories.name").is_some(), "{rows:?}");
        assert_eq!(
            rows.value(i, "categories.category_id"),
            rows.value(i, "products.category_id")
        );
    }
}
