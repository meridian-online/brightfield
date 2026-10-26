//! A folder opened at the front door lands in the model pane: the tables it found and the joins
//! between them, shown as a diagram, a list or the Ossie file, with `v` stepping through the three.
//!
//! Every window here is headless and opens the engine's own-files fixture through the window's
//! own open, with the `brightfield-shell` binary cargo built as its runner. The config directory
//! and arc's directories point at a scratch folder of this file's before anything reads them, so
//! no Protocol lands under the developer's real data directory. What a test asserts about the
//! pane it reads off the frame the pane drew ([`ModelPane::drawn`]).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use brightfield_shell::data_file::{self, Opened};
use brightfield_shell::design::Mode;
use brightfield_shell::model_pane::{ModelPane, ModelView};
use brightfield_shell::run::{self, Runner};
use brightfield_shell::window::{Boot, MeridianApp};

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-model-pane-{}", std::process::id()));
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

/// The engine's own-files fixture: three tables, and three files it cannot read.
fn fixture() -> PathBuf {
    scratch();
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../brightfield-engine/tests/data/own_files")
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

/// One headless window that opened the fixture folder.
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
        assert!(
            win.app.model_pane().is_some(),
            "the folder opens into the model pane"
        );
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

    fn key(&mut self, key: egui::Key) {
        self.frame(vec![press(key, egui::Modifiers::NONE)]);
    }

    fn pane(&self) -> &ModelPane {
        self.app.model_pane().expect("the model pane is open")
    }
}

#[test]
fn opens_on_diagram() {
    let win = Window::over_fixture();
    let pane = win.pane();
    assert_eq!(pane.view, ModelView::Diagram);
    let drawn = pane.drawn();
    assert_eq!(drawn.view, Some(ModelView::Diagram));
    // The summary heads the pane, and the diagram is laid out under it at the layout's size.
    assert_eq!(drawn.header.first(), Some(&pane.summary_line()));
    let tables = pane.protocol.sources.tables.len();
    assert!(
        pane.summary_line().starts_with(&format!("{tables} tables")),
        "{}",
        pane.summary_line()
    );
    let diagram = drawn.diagram.expect("the diagram was laid out");
    assert!(diagram.width() > 0.0 && diagram.height() > 0.0);
    assert!(
        !win.app.front_door_is_live(),
        "the pane stands in the door's place"
    );
}

#[test]
fn v_cycles_three_views() {
    let mut win = Window::over_fixture();
    win.key(egui::Key::V);
    assert_eq!(win.pane().view, ModelView::List);
    let list = win.pane().drawn().clone();
    assert_eq!(list.view, Some(ModelView::List));
    assert!(
        list.body.iter().any(|row| row.contains("items")),
        "the list names each table: {:?}",
        list.body
    );

    win.key(egui::Key::V);
    assert_eq!(win.pane().view, ModelView::File);
    let file = win.pane().drawn().clone();
    assert_eq!(file.view, Some(ModelView::File));
    assert_eq!(
        file.body.join("\n").trim_end(),
        win.pane().file_text().trim_end(),
        "the file view draws the Ossie file as written"
    );
    assert!(file.body.iter().any(|line| line.contains("0.2.0.dev0")));

    win.key(egui::Key::V);
    assert_eq!(win.pane().view, ModelView::Diagram);
    assert_eq!(win.pane().drawn().view, Some(ModelView::Diagram));
}

#[test]
fn each_view_has_a_key_of_its_own() {
    let mut win = Window::over_fixture();
    for (key, view) in [
        (egui::Key::Num3, ModelView::File),
        (egui::Key::Num2, ModelView::List),
        (egui::Key::Num1, ModelView::Diagram),
    ] {
        win.key(key);
        assert_eq!(win.pane().view, view, "{key:?}");
        assert_eq!(win.pane().drawn().view, Some(view));
    }
}

#[test]
fn l_still_moves_focus() {
    let mut win = Window::over_fixture();
    win.key(egui::Key::L);
    assert_eq!(
        win.pane().view,
        ModelView::Diagram,
        "`l` is the window's motion, not a view key"
    );
    win.key(egui::Key::V);
    win.key(egui::Key::L);
    assert_eq!(win.pane().view, ModelView::List);
    for key in [egui::Key::H, egui::Key::J, egui::Key::K] {
        win.key(key);
        assert_eq!(
            win.pane().view,
            ModelView::List,
            "{key:?} is not a view key"
        );
    }
}

#[test]
fn skipped_sources_are_named() {
    let win = Window::over_fixture();
    let found = brightfield_engine::sources::discover(&fixture()).expect("the fixture reads");
    let reason = |name: &str| {
        let (_, why) = found
            .skipped
            .iter()
            .find(|(path, _)| path.file_name().is_some_and(|n| n == name))
            .unwrap_or_else(|| panic!("{name} is skipped"));
        why.lines().next().unwrap_or("").trim().to_string()
    };
    let expected = format!(
        "3 files skipped: .DS_Store ({}), broken.csv ({}), notes.txt ({})",
        reason(".DS_Store"),
        reason("broken.csv"),
        reason("notes.txt"),
    );
    let header = &win.pane().drawn().header;
    assert_eq!(header.get(1), Some(&expected), "{header:?}");
    assert!(expected.contains("notes.txt (not a kind of file Brightfield reads)"));
}

#[test]
fn palette_lists_three_views() {
    let mut win = Window::over_fixture();
    win.key(egui::Key::Space);
    let rows = win.app.open_palette_rows();
    let registry = brightfield_keys::registry();
    let help = |longname: &str| {
        registry
            .iter()
            .find(|v| v.longname == longname)
            .map(|v| v.help)
            .unwrap()
    };
    let helps: Vec<&str> = rows.iter().map(|r| help(r)).collect();
    for wanted in [
        "Show the model as a diagram",
        "Show the model as a list",
        "Show the model as the Ossie file",
    ] {
        assert!(helps.contains(&wanted), "{wanted:?} in {helps:?}");
    }
    assert!(rows.iter().any(|r| r == "cycle-model-view"), "{rows:?}");
    // Every row the palette offers is one of the pane's own verbs.
    assert!(
        rows.iter()
            .all(|r| brightfield_shell::model_pane::is_model_verb(r)),
        "{rows:?}"
    );
}

#[test]
fn the_palettes_rows_change_the_view() {
    let mut pane = opened_pane();
    for (verb, view) in [
        ("show-model-file", ModelView::File),
        ("show-model-list", ModelView::List),
        ("show-model-diagram", ModelView::Diagram),
        ("cycle-model-view", ModelView::List),
    ] {
        assert!(pane.perform(verb));
        assert_eq!(pane.view, view, "{verb}");
    }
    assert!(!pane.perform("dive-in"));
}

fn opened_pane() -> ModelPane {
    let Opened::Model(protocol) =
        data_file::open_any(&fixture(), &runner()).expect("the folder opens")
    else {
        panic!("two tables are a model");
    };
    ModelPane::open(protocol, runner()).expect("the model reads")
}

#[test]
fn save_model_writes_the_file_and_reruns_only_the_silver_steps() {
    let mut pane = opened_pane();
    let dir = pane.protocol.dir.clone();
    let manifest = || std::fs::read_to_string(dir.join("arcform.yaml")).unwrap();
    let before = manifest();
    let left_out = pane
        .model
        .datasets
        .iter()
        .map(|d| d.name.clone())
        .find(|name| before.contains(&format!("silver_{name}")))
        .expect("a dataset with a silver step");

    let mut model = pane.model.clone();
    model
        .datasets
        .iter_mut()
        .find(|d| d.name == left_out)
        .unwrap()
        .meta
        .excluded = true;
    pane.save_model(model.clone()).expect("the save runs");

    // The file holds the new model, and the pane shows it.
    let written = std::fs::read_to_string(dir.join("model.ossie.yaml")).unwrap();
    assert_eq!(brightfield_ossie::read(&written).unwrap(), model);
    assert_eq!(pane.model, model);
    assert_eq!(pane.file_text(), written);
    // The silver steps were rewritten for it: the left-out table has none.
    let after = manifest();
    assert!(!after.contains(&format!("silver_{left_out}")), "{after}");
    assert!(after.contains("load_"), "the load steps stay: {after}");

    // The run that followed executed no load step: arc re-runs only what changed.
    let newest = run::record_paths_newest_first(&dir)
        .into_iter()
        .next()
        .expect("the save ran the Protocol");
    let contract =
        brightfield_protocol::contract::parse_contract(&std::fs::read(&newest).unwrap()).unwrap();
    let loads: Vec<_> = contract
        .steps
        .iter()
        .filter(|s| s.name.starts_with("load_"))
        .collect();
    assert!(!loads.is_empty());
    for step in loads {
        assert_eq!(
            step.status.state,
            brightfield_protocol::contract::StepState::Skipped,
            "{} ran again",
            step.name
        );
    }
}
