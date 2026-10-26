//! The reshape verbs: keep, drop, add or change a join, set a role, fold, unfold, leave out,
//! include and retype a column, each reached from the inspector, the keyboard and the palette. Each
//! one writes the Ossie file, rewrites silver and runs the Protocol, and the diagram is laid out
//! again from the new model as the file is written.
//!
//! The Protocols here are opened through the app's own open, with the `brightfield-shell` binary
//! cargo built as their runner, under a scratch folder of this file's that the config directory
//! and arc's directories point at before anything reads them. Nothing here needs a FineType
//! bundle, and nothing opens a GPU device: the pane lays out the same without a raster.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use brightfield_keys::{Altitude, CommandTier, Drives};
use brightfield_ossie::{Band, DecidedBy, Edit, Model, Role};
use brightfield_render::model_scene::Selection;
use brightfield_shell::data_file::{self, Opened};
use brightfield_shell::design::Mode;
use brightfield_shell::model_inspector::{self, ModelAction, ModelPicker};
use brightfield_shell::model_pane::{self, Dispatched, ModelPane, ModelView, RESHAPE_VERBS};
use brightfield_shell::run::Runner;
use brightfield_shell::window::{Boot, MeridianApp};

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-model-reshape-{}", std::process::id()));
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

/// The model written for the retail extract, with a possible join, likely ones, and categories
/// folded into products.
fn retail_model() -> Model {
    let text = std::fs::read_to_string(retail().join("model.ossie.yaml")).unwrap();
    brightfield_ossie::read(&text).unwrap()
}

/// Held around each open that makes a Protocol folder, as the inspector's tests hold theirs.
fn folder_lock() -> MutexGuard<'static, ()> {
    static OPENS: Mutex<()> = Mutex::new(());
    OPENS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The retail extract opened as a Protocol and run, with `model` saved over the one the open found.
fn opened(model: Model) -> ModelPane {
    let opened = {
        let _held = folder_lock();
        data_file::open_any(&retail(), &runner()).expect("it opens")
    };
    let Opened::Model(protocol) = opened else {
        panic!("several tables are a model");
    };
    let mut pane = ModelPane::open(protocol, runner()).expect("the model reads");
    pane.save_model(model).expect("the written model runs");
    pane
}

/// A pane over the written retail model, with no Protocol or database behind it: enough for what
/// is refused before anything is written.
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

/// The model as the Ossie file in `pane`'s folder holds it.
fn written(pane: &ModelPane) -> Model {
    let text = std::fs::read_to_string(pane.protocol.dir.join("model.ossie.yaml")).unwrap();
    brightfield_ossie::read(&text).unwrap()
}

/// Whether the Protocol's database holds a table named `table`.
fn has_table(pane: &ModelPane, table: &str) -> bool {
    let rows = model_inspector::query(
        &pane.protocol.dir,
        &format!(
            "SELECT count(*) AS n FROM duckdb_tables() WHERE table_name = '{}'",
            table.replace('\'', "''")
        ),
    )
    .expect("the database reads");
    rows.value(0, "n") != Some("0")
}

/// The silver SQL file written for `dataset`, if one was written.
fn silver_file(pane: &ModelPane, dataset: &str) -> Option<String> {
    std::fs::read_to_string(
        pane.protocol
            .dir
            .join("models")
            .join(format!("02_silver_{dataset}.sql")),
    )
    .ok()
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

fn button(at: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// The two frames of a click at `at`: the press, then the release.
fn click(ctx: &egui::Context, pane: &mut ModelPane, at: egui::Pos2) {
    draw(
        ctx,
        pane,
        vec![egui::Event::PointerMoved(at), button(at, true)],
    );
    draw(ctx, pane, vec![button(at, false)]);
}

#[test]
fn drop_join_rewrites_file_and_diagram() {
    let mut pane = opened(retail_model());
    let dropped = "transactions_customers";
    assert!(pane
        .layout()
        .edges
        .iter()
        .any(|e| e.relationship == dropped));
    // From the keyboard: the verb acts on the selected join.
    pane.focus.selected = Some(Selection::Relationship(dropped.into()));
    let done = pane.perform_reshape("drop-join").expect("a reshape verb");
    assert!(matches!(done, Dispatched::Done(_)), "{done:?}");

    let file = written(&pane);
    assert!(!file.relationships.iter().any(|r| r.name == dropped));
    let rejected = file
        .meta
        .rejected
        .iter()
        .find(|r| r.name == dropped)
        .expect("the dropped join is kept as rejected");
    assert_eq!(rejected.meta.decided_by, DecidedBy::Analyst);
    assert_eq!(pane.model, file, "the pane holds what the file holds");
    // The diagram is laid out from the new model as the file is written.
    assert!(!pane
        .layout()
        .edges
        .iter()
        .any(|e| e.relationship == dropped));
    assert_eq!(pane.focus.selected, None, "the dropped join is let go");
    assert_eq!(pane.status(), Some("Dropped transactions_customers"));
}

#[test]
fn set_role_moves_card_band() {
    let mut pane = opened(retail_model());
    let band = |pane: &ModelPane| {
        pane.layout()
            .cards
            .iter()
            .find(|c| c.dataset == "customers")
            .map(|c| c.band)
    };
    assert_eq!(band(&pane), Some(Band::Entities));
    // The key opens the role picker on the selected table, and its answer is the edit.
    pane.focus.selected = Some(Selection::Dataset("customers".into()));
    assert!(matches!(
        pane.perform_reshape("set-role"),
        Some(Dispatched::Picking(_))
    ));
    assert_eq!(
        pane.picker(),
        Some(&ModelPicker::Role {
            dataset: "customers".into()
        })
    );
    let ctx = egui::Context::default();
    draw(&ctx, &mut pane, Vec::new());
    let drawn = pane.drawn().inspector.clone().expect("the inspector drew");
    let at = drawn
        .picker
        .iter()
        .position(|b| b == "event")
        .unwrap_or_else(|| panic!("no event choice: {:?}", drawn.picker));
    click(&ctx, &mut pane, drawn.picker_rects[at].center());
    let chosen = pane.take_actions();
    assert_eq!(
        chosen,
        vec![ModelAction::ChangeRole {
            dataset: "customers".into(),
            role: Role::Event
        }]
    );
    for action in chosen {
        assert!(matches!(pane.dispatch(action), Dispatched::Done(_)));
    }
    assert_eq!(band(&pane), Some(Band::Events));
    assert_eq!(pane.picker(), None, "the answered picker closes");
    let customers = written(&pane)
        .datasets
        .into_iter()
        .find(|d| d.name == "customers")
        .unwrap();
    assert_eq!(customers.meta.role, Role::Event);
    assert_eq!(customers.meta.decided_by, DecidedBy::Analyst);
}

#[test]
fn fold_rewrites_silver() {
    let unfolded = brightfield_ossie::apply(
        &retail_model(),
        &Edit::Unfold {
            dataset: "categories".into(),
        },
    )
    .unwrap();
    let mut pane = opened(unfolded);
    let before = silver_file(&pane, "products").expect("products has silver");
    assert!(!before.contains("\"categories_name\""), "{before}");
    assert!(has_table(&pane, "categories"));

    // Fold into… offers only the tables a relationship joins categories to.
    pane.focus.selected = Some(Selection::Dataset("categories".into()));
    assert!(matches!(
        pane.perform_reshape("fold-table"),
        Some(Dispatched::Picking(_))
    ));
    assert_eq!(
        pane.picker(),
        Some(&ModelPicker::Fold {
            dataset: "categories".into(),
            hosts: vec!["products".into()],
        })
    );
    let ctx = egui::Context::default();
    draw(&ctx, &mut pane, Vec::new());
    let drawn = pane.drawn().inspector.clone().expect("the inspector drew");
    assert_eq!(drawn.picker, vec!["products".to_string()]);
    click(&ctx, &mut pane, drawn.picker_rects[0].center());
    let chosen = pane.take_actions();
    assert_eq!(
        chosen,
        vec![ModelAction::Edit(Edit::Fold {
            dataset: "categories".into(),
            into: "products".into()
        })]
    );
    for action in chosen {
        let done = pane.dispatch(action);
        assert!(matches!(done, Dispatched::Done(_)), "{done:?}");
    }

    let after = silver_file(&pane, "products").expect("products has silver");
    assert!(after.contains("\"categories_name\""), "{after}");
    assert!(silver_file(&pane, "categories").is_none());
    let rows = model_inspector::query(
        &pane.protocol.dir,
        "SELECT count(categories_name) AS n FROM \"products\"",
    )
    .expect("the folded column is in the silver table");
    assert_eq!(rows.value(0, "n"), Some("5"));
    // A folded table has no table of its own left in the database.
    assert!(!has_table(&pane, "categories"));
    assert!(!has_table(&pane, "categories__failed"));
    assert_eq!(
        written(&pane)
            .datasets
            .iter()
            .find(|d| d.name == "categories")
            .and_then(|d| d.meta.folded_into.clone()),
        Some("products".into())
    );
}

#[test]
fn refused_fold_explains() {
    let mut pane = written_retail("refused-fold");
    let before = pane.model.clone();
    let text = pane.file_text().to_string();
    // promo_codes joins nothing, so there is nothing to offer it.
    pane.focus.selected = Some(Selection::Dataset("promo_codes".into()));
    let Some(Dispatched::Failed(why)) = pane.perform_reshape("fold-table") else {
        panic!("folding promo_codes is refused");
    };
    assert!(why.contains("no relationship joins it"), "{why}");
    assert_eq!(pane.picker(), None);
    // The edit itself is refused the same way, whatever asks for it.
    let Dispatched::Failed(why) = pane.dispatch(ModelAction::Edit(Edit::Fold {
        dataset: "promo_codes".into(),
        into: "customers".into(),
    })) else {
        panic!("the fold is refused");
    };
    assert_eq!(
        why,
        "promo_codes can't fold into customers: no relationship joins them"
    );
    assert_eq!(pane.model, before, "the model is unchanged");
    assert_eq!(pane.file_text(), text, "the file is unchanged");
    let ctx = egui::Context::default();
    draw(&ctx, &mut pane, Vec::new());
    assert!(
        pane.drawn().header.contains(&why),
        "the status line says why: {:?}",
        pane.drawn().header
    );
}

#[test]
fn every_reshape_verb_is_registered_and_reachable_from_palette() {
    let registry = brightfield_keys::registry();
    let palette = model_pane::model_palette_verbs();
    for longname in RESHAPE_VERBS {
        let verb = registry
            .iter()
            .find(|v| v.longname == longname)
            .unwrap_or_else(|| panic!("{longname} is registered"));
        assert_eq!(
            verb.scope_applicability,
            vec![Altitude::Model],
            "{longname}"
        );
        assert_eq!(verb.tier, CommandTier::Data, "{longname}");
        assert_eq!(verb.drives, Drives::SpecEdit, "{longname}");
        assert!(verb.is_bound() && verb.scores.is_some(), "{longname}");
        assert!(palette.contains(&longname), "{longname} in {palette:?}");
        assert!(model_pane::is_model_verb(longname), "{longname}");
    }

    // The window's palette, opened over the model, lists each one, and choosing one acts on the
    // selection.
    let mut win = Window::over_retail();
    win.key(egui::Key::Space, egui::Modifiers::NONE);
    let rows = win.app.open_palette_rows();
    for longname in RESHAPE_VERBS {
        assert!(rows.iter().any(|r| r == longname), "{longname} in {rows:?}");
    }
    win.key(egui::Key::Escape, egui::Modifiers::NONE);

    // And each key reaches it: shift-d drops the selected join.
    let pane = win.app.model_pane_mut().unwrap();
    pane.focus.selected = Some(Selection::Relationship("transactions_stores".into()));
    win.key(egui::Key::D, egui::Modifiers::SHIFT);
    let pane = win.app.model_pane().unwrap();
    assert!(
        !pane
            .model
            .relationships
            .iter()
            .any(|r| r.name == "transactions_stores"),
        "shift-d dropped the join"
    );
    assert!(!written(pane)
        .relationships
        .iter()
        .any(|r| r.name == "transactions_stores"));
}

#[test]
fn left_out_table_is_not_left_materialised() {
    let mut pane = opened(retail_model());
    assert!(has_table(&pane, "promo_codes"));
    assert!(has_table(&pane, "promo_codes__failed"));
    pane.focus.selected = Some(Selection::Dataset("promo_codes".into()));
    let done = pane
        .perform_reshape("leave-out-table")
        .expect("a reshape verb");
    assert!(matches!(done, Dispatched::Done(_)), "{done:?}");
    assert!(!has_table(&pane, "promo_codes"));
    assert!(!has_table(&pane, "promo_codes__failed"));
    assert!(written(&pane)
        .datasets
        .iter()
        .any(|d| d.name == "promo_codes" && d.meta.excluded));

    // Brought back, its tables are made again rather than skipped as unchanged.
    let done = pane
        .perform_reshape("include-table")
        .expect("a reshape verb");
    assert!(matches!(done, Dispatched::Done(_)), "{done:?}");
    assert!(has_table(&pane, "promo_codes"));
    assert!(has_table(&pane, "promo_codes__failed"));
}

#[test]
fn the_pickers_change_keys_and_retype_a_column() {
    let mut pane = opened(retail_model());
    // Change keys… opens on the join's own columns, chosen from each side's.
    assert!(matches!(
        pane.dispatch(ModelAction::ChangeKeys("loyalty_visits_stores".into())),
        Dispatched::Picking(_)
    ));
    let Some(ModelPicker::Keys {
        from_choices,
        to_choices,
        from_columns,
        to_columns,
        ..
    }) = pane.picker().cloned()
    else {
        panic!("the keys picker");
    };
    assert_eq!(from_columns, vec!["store_id".to_string()]);
    assert_eq!(to_columns, vec!["store_id".to_string()]);
    assert!(from_choices.contains(&"customer_id".to_string()));
    assert!(!from_choices.iter().any(|c| c.starts_with("__source")));
    assert!(to_choices.contains(&"city".to_string()));
    if let Some(ModelPicker::Keys { to_columns, .. }) = pane.picker_mut() {
        to_columns[0] = "city".into();
    }
    let ctx = egui::Context::default();
    draw(&ctx, &mut pane, Vec::new());
    let drawn = pane.drawn().inspector.clone().expect("the inspector drew");
    let at = drawn
        .picker
        .iter()
        .position(|b| b == "Change keys")
        .expect("a Change keys button");
    click(&ctx, &mut pane, drawn.picker_rects[at].center());
    for action in pane.take_actions() {
        assert!(matches!(pane.dispatch(action), Dispatched::Done(_)));
    }
    let r = written(&pane)
        .relationships
        .into_iter()
        .find(|r| r.name == "loyalty_visits_stores")
        .unwrap();
    assert_eq!(r.to_columns, vec!["city".to_string()]);

    // Retype column… offers FineType's labels, and its answer recasts silver.
    pane.focus.selected = Some(Selection::Dataset("promo_codes".into()));
    assert!(matches!(
        pane.perform_reshape("retype-column"),
        Some(Dispatched::Picking(_))
    ));
    let Some(ModelPicker::Retype { fields, labels, .. }) = pane.picker().cloned() else {
        panic!("the retype picker");
    };
    assert_eq!(fields, vec!["code", "discount_pct", "valid_to"]);
    assert!(labels
        .iter()
        .any(|l| l == "representation.numeric.integer_number"));
    if let Some(ModelPicker::Retype {
        field, finetype, ..
    }) = pane.picker_mut()
    {
        *field = "code".into();
        *finetype = "representation.numeric.integer_number".into();
    }
    draw(&ctx, &mut pane, Vec::new());
    let drawn = pane.drawn().inspector.clone().expect("the inspector drew");
    let at = drawn
        .picker
        .iter()
        .position(|b| b == "Retype")
        .expect("a Retype button");
    click(&ctx, &mut pane, drawn.picker_rects[at].center());
    for action in pane.take_actions() {
        assert!(matches!(pane.dispatch(action), Dispatched::Done(_)));
    }
    // No code is a whole number, so every one of them failed to cast.
    let failed = model_inspector::query(
        &pane.protocol.dir,
        "SELECT count(*) AS n FROM \"promo_codes__failed\" WHERE \"column\" = 'code'",
    )
    .unwrap();
    assert_eq!(failed.value(0, "n"), Some("2"));
}

#[test]
fn a_drag_from_card_to_card_asks_for_the_join() {
    let mut pane = written_retail("drag-join");
    pane.view = ModelView::Diagram;
    let ctx = egui::Context::default();
    draw(&ctx, &mut pane, Vec::new());
    let origin = pane.drawn().diagram.expect("the diagram laid out").min;
    let centre = |pane: &ModelPane, name: &str| {
        let c = pane
            .layout()
            .cards
            .iter()
            .find(|c| c.dataset == name)
            .unwrap_or_else(|| panic!("{name} has a card"));
        origin + egui::vec2((c.x + c.width / 2.0) as f32, (c.y + c.height / 2.0) as f32)
    };
    let from = centre(&pane, "promo_codes");
    let to = centre(&pane, "stores");
    draw(
        &ctx,
        &mut pane,
        vec![egui::Event::PointerMoved(from), button(from, true)],
    );
    let mid = from + (to - from) / 2.0;
    draw(&ctx, &mut pane, vec![egui::Event::PointerMoved(mid)]);
    draw(&ctx, &mut pane, vec![egui::Event::PointerMoved(to)]);
    draw(&ctx, &mut pane, vec![button(to, false)]);
    let Some(ModelPicker::AddJoin {
        from: start,
        to: end,
        ..
    }) = pane.picker().cloned()
    else {
        panic!("the drag opened the add-join picker: {:?}", pane.picker());
    };
    assert_eq!((start.as_str(), end.as_str()), ("promo_codes", "stores"));
    assert_eq!(
        pane.focus.selected,
        Some(Selection::Dataset("promo_codes".into())),
        "the table the drag began on is selected"
    );

    // Its answer is the join, on a column of each.
    draw(&ctx, &mut pane, Vec::new());
    let drawn = pane.drawn().inspector.clone().expect("the inspector drew");
    let at = drawn
        .picker
        .iter()
        .position(|b| b == "Add join")
        .expect("an Add join button");
    click(&ctx, &mut pane, drawn.picker_rects[at].center());
    assert_eq!(
        pane.take_actions(),
        vec![ModelAction::Edit(Edit::AddJoin {
            from: "promo_codes".into(),
            from_columns: vec!["code".into()],
            to: "stores".into(),
            to_columns: vec!["store_id".into()],
        })]
    );
}

#[test]
fn a_hosts_inspector_offers_no_unfold_for_a_left_out_table() {
    let mut model = retail_model();
    model = brightfield_ossie::apply(
        &model,
        &Edit::Exclude {
            dataset: "categories".into(),
        },
    )
    .unwrap();
    let actions = model_inspector::table_actions(&model, "products");
    assert!(
        !actions.contains(&ModelAction::Unfold("categories".into())),
        "{actions:?}"
    );
    assert!(actions.contains(&ModelAction::AddJoin("products".into())));
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
        {
            let _held = folder_lock();
            win.app
                .open_path_knowing(&win.ctx, &retail().to_string_lossy(), &[]);
        }
        win.app
            .model_pane_mut()
            .expect("the folder opens a model")
            .save_model(retail_model())
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

    fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        self.frame(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]);
        self.frame(Vec::new());
    }
}
