//! New Protocol on the front door asks where to make the Protocol, makes an empty one there named
//! `Untitled`, and opens it with an empty canvas saying how to add a source. Adding a source to
//! an open Protocol is kept at once: the first names an Untitled Protocol after itself unless the
//! analyst has named it, one source shows its grid and dashboard, and a second switches the canvas
//! to the model, from which selecting a source shows its grid again.
//!
//! Every window is headless, with the `brightfield-shell` binary cargo built as its runner. The
//! config directory, the Protocols folder and arc's directories point at a scratch folder of this
//! file's before anything reads them, so nothing is written under the home directory.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use brightfield_shell::design::Mode;
use brightfield_shell::open_queue::OpenRequest;
use brightfield_shell::own_files::{self, AcceptedJoin};
use brightfield_shell::run::Runner;
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-new-protocol-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "protocols", "input", "places"] {
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

/// A folder of this test's own to make Protocols in, outside the Protocols folder, so the tests
/// running beside it do not see each other's `Untitled`.
fn place(name: &str) -> PathBuf {
    let dir = scratch().join("places").join(name);
    std::fs::create_dir_all(&dir).expect("a place");
    dir
}

/// A CSV named `name` holding `text`, in a folder of the test's own.
fn csv(test: &str, name: &str, text: &str) -> PathBuf {
    let dir = scratch().join("input").join(test);
    std::fs::create_dir_all(&dir).expect("an input folder");
    let file = dir.join(name);
    std::fs::write(&file, text).expect("the CSV writes");
    file
}

const QUAKES: &str = "id,magnitude,region\n1,4.5,north\n2,5.1,south\n3,3.9,north\n";
const CUSTOMERS: &str = "id,name\n1,Ada\n2,Bo\n3,Cy\n";
const ORDERS: &str = "id,customer_id\n10,1\n11,2\n12,3\n";

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

fn press(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
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

/// The `name:` an `arcform.yaml` gives its Protocol.
fn manifest_name(dir: &Path) -> String {
    let text = std::fs::read_to_string(dir.join("arcform.yaml")).expect("the manifest reads");
    brightfield_protocol::parse_manifest_str(&text)
        .expect("the manifest parses")
        .name
}

/// One headless window, its frames run through one context.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    time: f64,
}

impl Window {
    fn new() -> Self {
        scratch();
        let mut win = Self {
            app: MeridianApp::headless(Boot::empty(), Mode::Light).running_with(Some(runner())),
            ctx: egui::Context::default(),
            time: 0.0,
        };
        win.settle();
        win
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        self.time += 1.0;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440.0, 900.0),
            )),
            time: Some(self.time),
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

    fn click(&mut self, at: egui::Pos2) {
        self.run(vec![
            egui::Event::PointerMoved(at),
            button_at(at, true),
            button_at(at, false),
        ]);
        self.settle();
    }

    fn key(&mut self, key: egui::Key) {
        self.run(vec![press(key)]);
        self.settle();
    }

    /// The window with an empty Protocol made in `parent` and open.
    fn made_in(parent: &Path) -> Self {
        let mut win = Self::new();
        win.app.make_new_protocol_in(&win.ctx.clone(), parent);
        win.settle();
        assert!(
            win.app.empty_protocol().is_some(),
            "the empty Protocol is open: {}",
            win.said()
        );
        win
    }

    /// Ask the window's queue to add `path` to the open Protocol, and run the frames that take it.
    fn add(&mut self, path: &Path) {
        self.app
            .open_queue()
            .push(OpenRequest::AddSource(path.to_path_buf()));
        self.settle();
    }

    fn dir(&self) -> Option<PathBuf> {
        self.app.open_protocol_dir()
    }

    fn said(&self) -> String {
        self.app
            .notifications()
            .iter()
            .map(|n| format!("{}: {}", n.title, n.body.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[test]
fn new_protocol_asks_where_with_the_protocols_folder_preselected() {
    let root = scratch().join("protocols");
    let group = root.join("Clients");
    std::fs::create_dir_all(&group).expect("a group in the Protocols folder");

    let mut win = Window::new();
    assert!(win.app.front_door_is_live());
    let button = win
        .app
        .front_door_new_protocol_rect()
        .expect("the door draws New Protocol");
    win.click(button.center());
    assert_eq!(win.app.open_overlay(), Some("new-protocol"));
    assert_eq!(
        win.app.new_protocol_places(),
        Some(vec![
            "Protocols".to_string(),
            "Clients".to_string(),
            "Other\u{2026}".to_string()
        ]),
        "the Protocols folder, its groups, and any other folder"
    );
    assert_eq!(
        win.app.new_protocol_selected(),
        Some(0),
        "the Protocols folder is preselected"
    );

    // Esc cancels and makes nothing.
    win.key(egui::Key::Escape);
    assert_eq!(win.app.open_overlay(), None);
    assert!(win.app.front_door_is_live(), "nothing was made or opened");
    assert_eq!(
        std::fs::read_dir(&group).expect("the group reads").count(),
        0,
        "Esc makes nothing"
    );

    // A click on the group chooses it, and Enter makes the Protocol there.
    win.click(button.center());
    let row = win
        .app
        .new_protocol_place_rect(1)
        .expect("the sheet draws the group's row");
    win.click(row.center());
    assert_eq!(win.app.new_protocol_selected(), Some(1));
    win.key(egui::Key::Enter);
    assert_eq!(win.app.open_overlay(), None);
    let made = group.join("Untitled");
    assert!(
        made.join("arcform.yaml").is_file(),
        "the Protocol is made where chosen: {}",
        win.said()
    );
    assert_eq!(win.dir(), Some(made));
}

#[test]
fn new_protocol_makes_untitled_and_opens_the_browser() {
    let parent = place("untitled");
    let win = Window::made_in(&parent);
    let dir = parent.join("Untitled");
    assert_eq!(win.dir(), Some(dir.clone()), "{}", win.said());
    assert_eq!(manifest_name(&dir), "Untitled");
    let sources: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join(".sources.json")).expect("the sources record"),
    )
    .expect("the record parses");
    assert_eq!(sources["tables"], serde_json::json!([]), "no sources yet");
    assert!(
        !win.app.front_door_is_live(),
        "the Protocol replaced the door"
    );
    assert!(win.app.model_pane().is_none());
    assert!(
        win.app.browser_requested(),
        "an empty Protocol asks for the sources browser"
    );
    let canvas = win
        .app
        .empty_canvas_said()
        .expect("the canvas says how to add a source");
    assert!(
        canvas.contains("Drop a data file") && canvas.contains("Add source"),
        "{canvas}"
    );
    assert!(win.app.title().contains("Untitled"), "{}", win.app.title());

    // The next takes `-2`.
    let again = Window::made_in(&parent);
    assert_eq!(again.dir(), Some(parent.join("Untitled-2")));
}

#[test]
fn the_first_source_names_an_untitled_protocol() {
    let parent = place("naming");
    let mut win = Window::made_in(&parent);
    win.add(&csv("naming", "earthquakes_2024.csv", QUAKES));

    let dir = parent.join("earthquakes_2024");
    assert_eq!(win.dir(), Some(dir.clone()), "{}", win.said());
    assert!(!parent.join("Untitled").exists(), "the folder was renamed");
    assert_eq!(manifest_name(&dir), "earthquakes_2024");
    assert!(
        win.app.own_protocol().is_some(),
        "one source opens its grid"
    );
    assert!(win.app.chart_doc().is_live(), "and its dashboard");
    assert!(!win.app.browser_requested(), "the source was added");

    // A second Untitled, given the same source while that name is taken, takes `-2`.
    let mut other = Window::made_in(&parent);
    other.add(&csv("naming", "earthquakes_2024.csv", QUAKES));
    assert_eq!(
        other.dir(),
        Some(parent.join("earthquakes_2024-2")),
        "{}",
        other.said()
    );
}

#[test]
fn a_renamed_untitled_keeps_its_name_when_a_source_is_added() {
    let parent = place("renamed");
    let mut win = Window::made_in(&parent);
    win.app.rename_protocol(&win.ctx.clone(), "Quakes");
    win.settle();
    assert_eq!(win.dir(), Some(parent.join("Quakes")), "{}", win.said());
    win.add(&csv("renamed", "earthquakes_2024.csv", QUAKES));
    assert_eq!(win.dir(), Some(parent.join("Quakes")), "{}", win.said());
    assert!(win.app.own_protocol().is_some(), "{}", win.said());

    // Named by history, not by the name's pattern: an analyst who renames to "Untitled" keeps it.
    let kept = place("renamed-untitled");
    let mut win = Window::made_in(&kept);
    win.app.rename_protocol(&win.ctx.clone(), "Untitled");
    win.settle();
    win.add(&csv("renamed", "earthquakes_2024.csv", QUAKES));
    assert_eq!(win.dir(), Some(kept.join("Untitled")), "{}", win.said());
}

#[test]
fn a_second_source_switches_the_canvas_to_the_model() {
    let parent = place("second");
    let mut win = Window::made_in(&parent);
    win.add(&csv("second", "customers.csv", CUSTOMERS));
    assert!(win.app.own_protocol().is_some(), "{}", win.said());
    win.add(&csv("second", "orders.csv", ORDERS));

    let pane = win
        .app
        .model_pane()
        .unwrap_or_else(|| panic!("two sources show the model: {}", win.said()));
    let names: Vec<&str> = pane
        .protocol
        .sources
        .tables
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(names, ["customers", "orders"]);
    assert!(win.app.own_protocol().is_none(), "the model pane holds it");
    let dir = parent.join("customers");
    assert_eq!(win.dir(), Some(dir.clone()), "the first source named it");
    assert!(
        dir.join("model.ossie.yaml").is_file() || own_files::own_protocol_dir(&dir).is_some(),
        "a model is written for two tables"
    );
}

#[test]
fn selecting_a_source_on_the_spine_shows_its_grid() {
    let parent = place("spine");
    let mut win = Window::made_in(&parent);
    win.add(&csv("spine", "customers.csv", CUSTOMERS));
    win.add(&csv("spine", "orders.csv", ORDERS));
    assert!(win.app.model_pane().is_some(), "{}", win.said());

    assert!(
        win.app.select_source(&win.ctx.clone(), "orders"),
        "{}",
        win.said()
    );
    win.settle();
    assert!(
        win.app.chart_doc().is_live(),
        "the table's dashboard is drawn"
    );
    assert!(
        win.app.chart_doc().title().contains("orders"),
        "{}",
        win.app.chart_doc().title()
    );
}

#[test]
fn adding_a_source_leaves_no_unsaved_mark() {
    let parent = place("unsaved");
    let mut win = Window::made_in(&parent);
    win.add(&csv("unsaved", "customers.csv", CUSTOMERS));
    assert!(win.app.own_protocol().is_some(), "{}", win.said());
    assert!(!win.app.carries_unsaved_mark());
    assert!(
        !win.app.title().contains(UNSAVED_MARK),
        "{}",
        win.app.title()
    );
    win.add(&csv("unsaved", "orders.csv", ORDERS));
    assert!(win.app.model_pane().is_some(), "{}", win.said());
    assert!(!win.app.carries_unsaved_mark());
    assert!(
        !win.app.title().contains(UNSAVED_MARK),
        "{}",
        win.app.title()
    );
}

#[test]
fn adding_writes_no_run_record() {
    let parent = place("records");
    let mut win = Window::made_in(&parent);
    win.add(&csv("records", "customers.csv", CUSTOMERS));
    win.add(&csv("records", "orders.csv", ORDERS));
    let dir = win.dir().expect("a Protocol is open");
    assert!(win.app.model_pane().is_some(), "{}", win.said());

    let previews = brightfield_shell::run::preview_ids(&dir);
    let records: Vec<String> = std::fs::read_dir(brightfield_shell::run::runs_dir(&dir))
        .map(|d| {
            d.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    name.strip_suffix(".json").map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    for record in &records {
        assert!(
            previews.contains(record),
            "{record} is a run record, not a preview: {previews:?}"
        );
    }
    assert_ne!(
        brightfield_shell::door_tree::run_state(&dir),
        brightfield_workbench::RunState::Fresh,
        "the door does not say it was run"
    );
}

#[test]
fn accepted_joins_are_written_into_the_model() {
    let parent = place("joins");
    let mut p = own_files::create_empty(&parent).expect("an empty Protocol");
    let runner = runner();
    let discovery = brightfield_shell::discovery::StandIn;
    own_files::add_source_with(
        &mut p,
        &csv("joins", "customers.csv", CUSTOMERS),
        &[],
        &runner,
        &discovery,
    )
    .expect("the first source adds");
    own_files::add_source_with(
        &mut p,
        &csv("joins", "orders.csv", ORDERS),
        &[AcceptedJoin {
            left: ("orders".into(), "customer_id".into()),
            right: ("customers".into(), "id".into()),
        }],
        &runner,
        &discovery,
    )
    .expect("the second source adds with its join");
    let text = std::fs::read_to_string(p.dir.join("model.ossie.yaml")).expect("a model file");
    let model = brightfield_ossie::read(&text).expect("the model reads");
    let join = model
        .relationships
        .iter()
        .find(|r| r.from == "orders" && r.to == "customers")
        .unwrap_or_else(|| panic!("the accepted join is in the model: {text}"));
    assert_eq!(join.from_columns, ["customer_id"]);
    assert_eq!(join.to_columns, ["id"]);
    assert_eq!(
        join.meta.decided_by,
        brightfield_ossie::DecidedBy::Analyst,
        "the analyst accepted it"
    );
}
