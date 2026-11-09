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
use brightfield_shell::overlays::{CloseAnswer, ADD_SOURCE_QUESTION_TITLE, MAKE_QUESTION_TITLE};
use brightfield_shell::own_files::{self, AcceptedJoin};
use brightfield_shell::run::Runner;
use brightfield_shell::window::{
    Boot, MeridianApp, ADD_SOURCE_REFUSED_BANNER, FOLDER_GONE_BANNER, NEW_PROTOCOL_BANNER,
    UNSAVED_MARK,
};

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
    press_with(key, egui::Modifiers::NONE)
}

fn press_with(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// A runner that runs the Protocol as the real one does `passing` times, and fails every run after:
/// how a test makes the last run of an add fail once the add has written its model.
fn failing_runner(name: &str, passing: u32) -> Runner {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = scratch().join("runners");
    std::fs::create_dir_all(&dir).expect("a folder for runners");
    let script = dir.join(name);
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncount=\"$0.count\"\nn=$(cat \"$count\" 2>/dev/null || echo 0)\n\
             n=$((n + 1))\necho \"$n\" > \"$count\"\n\
             if [ \"$n\" -gt {passing} ]; then echo 'this run was made to fail' >&2; exit 1; fi\n\
             exec '{}' \"$@\"\n",
            env!("CARGO_BIN_EXE_brightfield-shell")
        ),
    )
    .expect("the runner script writes");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("the runner script made runnable");
    Runner::at(script)
}

/// The folders under `dir`, relative to it, at any depth: what a failed add puts back.
fn folders_of(dir: &Path) -> std::collections::BTreeSet<PathBuf> {
    let mut found = std::collections::BTreeSet::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.insert(path.strip_prefix(dir).unwrap().to_path_buf());
                stack.push(path);
            }
        }
    }
    found
}

/// The files under `dir`, relative to it, with their bytes: what a failed add must leave as it was.
fn files_of(dir: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut found = std::collections::BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes = std::fs::read(&path).expect("a file reads");
                found.insert(path.strip_prefix(dir).unwrap().to_path_buf(), bytes);
            }
        }
    }
    found
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
            // A chart kept as a version, rather than saved, needs somewhere to keep it.
            app: MeridianApp::headless(Boot::empty(), Mode::Light)
                .running_with(Some(runner()))
                .keeping_history(Some(brightfield_protocol::HistoryStore::At(
                    scratch().join("history"),
                ))),
            ctx: egui::Context::default(),
            time: 0.0,
        };
        win.settle();
        win
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        self.time += 1.0;
        let modifiers = events
            .iter()
            .find_map(|e| match e {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let raw = egui::RawInput {
            modifiers,
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

    /// Whether the window raised the banner `id`.
    fn raised(&self, id: &str) -> bool {
        self.app
            .notifications()
            .iter()
            .any(|n| n.id == meridian_egui::NotificationId::new(id))
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

#[test]
fn a_failed_first_add_leaves_it_untitled_and_a_later_add_names_it() {
    let parent = place("failed-first");
    let mut p = own_files::create_empty(&parent).expect("an empty Protocol");
    let quakes = csv("failed-first", "earthquakes_2024.csv", QUAKES);
    let failed = own_files::add_source_with(
        &mut p,
        &quakes,
        &[],
        &failing_runner("failed-first", 1),
        &brightfield_shell::discovery::StandIn,
    );
    assert!(failed.is_err(), "the last run was made to fail");
    let untitled = parent.join("Untitled");
    assert_eq!(p.dir, untitled, "a failed add renames nothing");
    assert!(own_files::still_untitled(&untitled));
    assert!(own_files::empty_protocol_dir(&untitled).is_some());

    own_files::add_source_with(
        &mut p,
        &quakes,
        &[],
        &runner(),
        &brightfield_shell::discovery::StandIn,
    )
    .expect("the add goes through");
    assert_eq!(p.dir, parent.join("earthquakes_2024"));
    assert!(!untitled.exists());
}

#[test]
fn a_failed_add_puts_an_empty_protocol_back() {
    let parent = place("failed-empty");
    let mut p = own_files::create_empty(&parent).expect("an empty Protocol");
    let before = files_of(&p.dir);
    let folders_before = folders_of(&p.dir);
    let paths = [
        csv("failed-empty", "customers.csv", CUSTOMERS),
        csv("failed-empty", "orders.csv", ORDERS),
    ];
    let failed = own_files::add_sources_with(
        &mut p,
        &paths,
        &[],
        &failing_runner("failed-empty", 1),
        &brightfield_shell::discovery::StandIn,
    );
    assert!(
        failed.is_err(),
        "the last run, after the model, was made to fail"
    );
    assert_eq!(
        own_files::empty_protocol_dir(&p.dir),
        Some(p.dir.clone()),
        "it opens as the empty Protocol it was"
    );
    let after: std::collections::BTreeMap<_, _> = files_of(&p.dir)
        .into_iter()
        .filter(|(path, _)| !path.starts_with("build") && !path.starts_with(".previews"))
        .collect();
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>()
    );
    assert_eq!(after, before, "every file is as it was");
    let folders_after: std::collections::BTreeSet<_> = folders_of(&p.dir)
        .into_iter()
        .filter(|path| !path.starts_with("build"))
        .collect();
    assert_eq!(
        folders_after, folders_before,
        "no folder the add made is left"
    );
}

#[test]
fn a_failed_add_puts_a_one_source_protocol_back() {
    let parent = place("failed-one");
    let mut p = own_files::create_empty(&parent).expect("an empty Protocol");
    own_files::add_source_with(
        &mut p,
        &csv("failed-one", "customers.csv", CUSTOMERS),
        &[],
        &runner(),
        &brightfield_shell::discovery::StandIn,
    )
    .expect("the first source adds");
    let watched = |dir: &Path| {
        files_of(dir)
            .into_iter()
            .filter(|(path, _)| {
                path.starts_with("models")
                    || path.starts_with("packages")
                    || path == Path::new("arcform.yaml")
                    || path == Path::new(".sources.json")
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let before = watched(&p.dir);
    let folders = |dir: &Path| {
        folders_of(dir)
            .into_iter()
            .filter(|path| path.starts_with("models") || path.starts_with("packages"))
            .collect::<std::collections::BTreeSet<_>>()
    };
    let folders_before = folders(&p.dir);
    let failed = own_files::add_source_with(
        &mut p,
        &csv("failed-one", "orders.csv", ORDERS),
        &[],
        &failing_runner("failed-one", 1),
        &brightfield_shell::discovery::StandIn,
    );
    assert!(
        failed.is_err(),
        "the last run, after the model, was made to fail"
    );
    assert_eq!(
        own_files::one_source_protocol_dir(&p.dir),
        Some(p.dir.clone()),
        "it opens as the Protocol of one source it was"
    );
    assert!(!p.dir.join("model.ossie.yaml").exists(), "no model is left");
    assert_eq!(watched(&p.dir), before, "its files are as they were");
    assert_eq!(
        folders(&p.dir),
        folders_before,
        "no folder the add made is left, such as the new source's package"
    );
    assert!(p.dir.join(".profile").is_dir(), "its profile is back");
}

#[test]
fn an_add_reads_the_protocols_sources_again_first() {
    let parent = place("stale");
    let mut p = own_files::create_empty(&parent).expect("an empty Protocol");
    // Named by the analyst, so the adds below rename nothing.
    brightfield_shell::rename::rename(&p.dir, "Shop").expect("the rename");
    p.dir = parent.join("Shop");
    let mut stale = p.clone();
    own_files::add_source_with(
        &mut p,
        &csv("stale", "customers.csv", CUSTOMERS),
        &[],
        &runner(),
        &brightfield_shell::discovery::StandIn,
    )
    .expect("the first source adds");
    own_files::add_source_with(
        &mut stale,
        &csv("stale", "customers.csv", CUSTOMERS),
        &[],
        &runner(),
        &brightfield_shell::discovery::StandIn,
    )
    .expect("a second add through a Protocol held from before the first");
    let names: Vec<&str> = stale
        .sources
        .tables
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(names, ["customers", "customers_2"], "the first add is kept");
    assert!(stale.dir.join("models/01_load_customers.sql").is_file());
    assert!(stale.dir.join("models/01_load_customers_2.sql").is_file());
}

#[test]
fn files_dropped_on_an_empty_protocol_are_added_together() {
    let parent = place("dropped");
    let mut win = Window::made_in(&parent);
    let files = [
        csv("dropped", "customers.csv", CUSTOMERS),
        csv("dropped", "orders.csv", ORDERS),
    ];
    win.time += 1.0;
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1440.0, 900.0),
        )),
        dropped_files: files
            .iter()
            .map(|f| egui::DroppedFile {
                path: Some(f.clone()),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let _ = win.ctx.run_ui(raw, |ui| win.app.draw(ui));
    win.settle();
    let pane = win
        .app
        .model_pane()
        .unwrap_or_else(|| panic!("both were added as one: {}", win.said()));
    assert_eq!(pane.protocol.sources.tables.len(), 2);
    assert_eq!(win.dir(), Some(parent.join("customers")));
}

/// The window with a one-source Protocol of the housing sample, made by New Protocol, holding a
/// filter not yet saved.
fn housing_with_a_draft(test: &str) -> (Window, PathBuf) {
    let parent = place(test);
    let mut win = Window::made_in(&parent);
    let input = scratch().join("input").join(test);
    std::fs::create_dir_all(&input).expect("an input folder");
    let houses = input.join("houses.csv");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv"),
        &houses,
    )
    .expect("the housing sample copies");
    win.add(&houses);
    assert!(win.app.own_protocol().is_some(), "{}", win.said());
    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some("house_age > 28".to_owned()),
            order: None,
        });
    win.settle();
    assert!(win.app.carries_unsaved_mark());
    let dir = win.dir().expect("a Protocol");
    (win, dir)
}

#[test]
fn adding_a_source_over_a_draft_asks_and_saves_first() {
    let (mut win, dir) = housing_with_a_draft("draft-save");
    win.add(&csv(
        "draft-save",
        "regions.csv",
        "region,name\nnorth,North\n",
    ));
    let question = win.app.close_question().expect("the question is up");
    assert_eq!(question.title(), ADD_SOURCE_QUESTION_TITLE);
    assert_eq!(
        question.answer_label(CloseAnswer::SaveAndClose),
        "Save and add"
    );
    assert_eq!(
        question.answer_label(CloseAnswer::CloseWithoutSaving),
        "Keep as a version and add"
    );
    assert!(win.app.model_pane().is_none(), "nothing is added yet");

    win.app
        .answer_close_question(&win.ctx.clone(), CloseAnswer::SaveAndClose);
    win.settle();
    assert!(win.app.model_pane().is_some(), "{}", win.said());
    assert!(
        std::fs::read_to_string(dir.join("arcform.yaml"))
            .expect("the manifest")
            .contains("_filtered_by_house_age"),
        "the draft was saved before the add"
    );
    assert!(!win.app.carries_unsaved_mark());
}

#[test]
fn adding_a_source_over_a_draft_can_keep_it_as_a_version() {
    let (mut win, dir) = housing_with_a_draft("draft-version");
    win.add(&csv(
        "draft-version",
        "regions.csv",
        "region,name\nnorth,North\n",
    ));
    assert!(win.app.close_question().is_some(), "the question is up");
    win.app
        .answer_close_question(&win.ctx.clone(), CloseAnswer::CloseWithoutSaving);
    win.settle();
    assert!(win.app.model_pane().is_some(), "{}", win.said());
    assert!(
        !std::fs::read_to_string(dir.join("arcform.yaml"))
            .expect("the manifest")
            .contains("_filtered_by_house_age"),
        "the draft was not saved into the Protocol"
    );
}

#[test]
fn sources_are_added_only_to_protocols_brightfield_made() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/shipped-protocol");
    let dir = scratch()
        .join("input")
        .join("by-hand")
        .join("shipped_sample");
    for file in [
        "arcform.yaml",
        "models/measures.sql",
        "data/measures.csv",
        "panels/overview.yaml",
    ] {
        let to = dir.join(file);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(fixture.join(file), &to).expect("the fixture copies");
    }
    let mut win = Window::new();
    win.app.open_queue().push(OpenRequest::Path(dir.clone()));
    win.settle();
    assert!(!win.app.front_door_is_live(), "{}", win.said());
    // The opened Protocol's own folder, which no other test writes to: the
    // shared places folder grows while the binary's other tests run beside
    // this one, so a count of it says nothing about this add.
    let held = files_of(&dir);
    win.add(&csv("by-hand", "customers.csv", CUSTOMERS));
    assert!(win.raised(ADD_SOURCE_REFUSED_BANNER), "{}", win.said());
    assert!(win.app.own_protocol().is_none() && win.app.model_pane().is_none());
    assert_eq!(files_of(&dir), held, "the refused add left the Protocol as it was");
}

#[test]
fn adding_to_a_protocol_whose_folder_has_gone_says_so() {
    let parent = place("gone");
    let mut win = Window::made_in(&parent);
    std::fs::remove_dir_all(parent.join("Untitled")).expect("the folder removed");
    win.add(&csv("gone", "customers.csv", CUSTOMERS));
    assert!(win.raised(FOLDER_GONE_BANNER), "{}", win.said());
    assert!(!parent.join("Untitled").exists(), "nothing is made again");
}

#[test]
fn a_refused_folder_is_worded_for_making_a_protocol() {
    use std::os::unix::fs::PermissionsExt as _;
    let parent = place("refused");
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o555))
        .expect("the folder made read-only");
    let mut win = Window::new();
    win.app.make_new_protocol_in(&win.ctx.clone(), &parent);
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755))
        .expect("the folder made writable");
    let body = win
        .app
        .notifications()
        .iter()
        .find(|n| n.id == meridian_egui::NotificationId::new(NEW_PROTOCOL_BANNER))
        .and_then(|n| n.body.clone())
        .unwrap_or_else(|| panic!("the refusal is said: {}", win.said()));
    assert!(body.contains("Privacy & Security"), "{body}");
    assert!(body.contains("choose New Protocol again"), "{body}");
    assert!(!body.contains("open the file again"), "{body}");
}

#[test]
fn cmd_n_asks_where_and_the_arrows_move_the_choice() {
    let mut win = Window::new();
    let command = egui::Modifiers {
        command: true,
        ..Default::default()
    };
    win.run(vec![press_with(egui::Key::N, command)]);
    win.settle();
    assert_eq!(win.app.open_overlay(), Some("new-protocol"));
    assert_eq!(win.app.new_protocol_selected(), Some(0));
    win.key(egui::Key::ArrowDown);
    assert_eq!(win.app.new_protocol_selected(), Some(1));
    win.key(egui::Key::ArrowUp);
    win.key(egui::Key::ArrowUp);
    assert_eq!(
        win.app.new_protocol_selected(),
        Some(0),
        "the choice stops at the top"
    );
    win.key(egui::Key::Escape);
    assert_eq!(win.app.open_overlay(), None);
}

#[test]
fn a_profile_left_aside_by_an_add_that_stopped_is_put_back() {
    let parent = place("aside");
    let mut p = own_files::create_empty(&parent).expect("an empty Protocol");
    own_files::add_source_with(
        &mut p,
        &csv("aside", "customers.csv", CUSTOMERS),
        &[],
        &runner(),
        &brightfield_shell::discovery::StandIn,
    )
    .expect("the first source adds");
    let profile = p.dir.join(".profile");
    let aside = p.dir.join(".profile.before-add");
    let held = files_of(&profile);
    assert!(!held.is_empty(), "the first add profiled its table");
    // As an add the process ended under leaves it.
    std::fs::rename(&profile, &aside).expect("the profile set aside");

    let failed = own_files::add_source_with(
        &mut p,
        &csv("aside", "orders.csv", ORDERS),
        &[],
        &failing_runner("aside", 0),
        &brightfield_shell::discovery::StandIn,
    );
    assert!(failed.is_err(), "every run was made to fail");
    assert_eq!(files_of(&profile), held, "the profile is back as it was");
    assert!(!aside.exists(), "and not left aside");
}

#[test]
fn first_sources_that_cannot_rename_the_protocol_are_added_and_say_so() {
    use std::os::unix::fs::PermissionsExt as _;
    let parent = place("not-named");
    let mut p = own_files::create_empty(&parent).expect("an empty Protocol");
    // The folder it is in takes no new name, so the Protocol cannot be renamed in it.
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o555))
        .expect("the folder made read-only");
    let added = own_files::add_source_with(
        &mut p,
        &csv("not-named", "customers.csv", CUSTOMERS),
        &[],
        &runner(),
        &brightfield_shell::discovery::StandIn,
    );
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755))
        .expect("the folder made writable");
    let added = added.expect("the source is added all the same");
    assert!(added.not_named.is_some(), "why it was not named is said");
    assert!(added.warnings.is_empty(), "{:?}", added.warnings);
    assert_eq!(p.dir, parent.join("Untitled"));
    assert_eq!(p.sources.tables.len(), 1);
}

#[test]
fn new_protocol_over_a_draft_asks_worded_for_making() {
    let (mut win, _dir) = housing_with_a_draft("draft-make");
    win.run(vec![press_with(
        egui::Key::N,
        egui::Modifiers {
            command: true,
            ..Default::default()
        },
    )]);
    win.settle();
    assert_eq!(win.app.open_overlay(), Some("new-protocol"));
    win.key(egui::Key::Enter);
    let question = win.app.close_question().expect("the question is up");
    assert_eq!(question.title(), MAKE_QUESTION_TITLE);
    assert_eq!(
        question.answer_label(CloseAnswer::SaveAndClose),
        "Save and make"
    );
    assert_eq!(
        question.answer_label(CloseAnswer::CloseWithoutSaving),
        "Keep as a version and make"
    );
}

/// The window with the model of two sources, made by New Protocol, and an edit typed into its
/// file view that has not been read yet: a join added by hand.
fn model_with_an_unread_edit(test: &str) -> (Window, PathBuf) {
    let parent = place(test);
    let mut win = Window::made_in(&parent);
    win.add(&csv(test, "customers.csv", CUSTOMERS));
    win.add(&csv(test, "orders.csv", ORDERS));
    let dir = win.dir().expect("a Protocol");
    let pane = win
        .app
        .model_pane_mut()
        .unwrap_or_else(|| panic!("two sources show the model"));
    let text = pane.file_text().to_string();
    let join = "- name: hand_join\n  from: orders\n  to: customers\n  from_columns:\n  - customer_id\n  to_columns:\n  - id\n";
    let edited = if text.contains("relationships: []\n") {
        text.replace("relationships: []\n", &format!("relationships:\n{join}"))
    } else {
        text.replace("relationships:\n", &format!("relationships:\n{join}"))
    };
    assert_ne!(edited, text, "the join went into the text: {text}");
    pane.set_file_draft(edited);
    assert!(win.app.carries_unsaved_mark(), "an unread edit is unsaved");
    (win, dir)
}

/// Ask the window's queue to add `path`, and run the one frame that takes it, so the file view's
/// pause has not passed.
fn add_in_one_frame(win: &mut Window, path: &Path) {
    win.app
        .open_queue()
        .push(OpenRequest::AddSource(path.to_path_buf()));
    win.run(Vec::new());
}

#[test]
fn adding_over_an_unread_model_edit_asks_and_save_keeps_it() {
    let (mut win, dir) = model_with_an_unread_edit("model-save");
    add_in_one_frame(
        &mut win,
        &csv("model-save", "regions.csv", "region,name\nnorth,North\n"),
    );
    let question = win.app.close_question().expect("the question is up");
    assert_eq!(question.title(), ADD_SOURCE_QUESTION_TITLE);
    assert!(
        question.edits.iter().any(|e| e.starts_with("Model")),
        "{:?}",
        question.edits
    );
    win.app
        .answer_close_question(&win.ctx.clone(), CloseAnswer::SaveAndClose);
    win.settle();
    let pane = win
        .app
        .model_pane()
        .unwrap_or_else(|| panic!("the source was added: {}", win.said()));
    assert_eq!(pane.protocol.sources.tables.len(), 3);
    let model = std::fs::read_to_string(dir.join("model.ossie.yaml")).expect("the model file");
    assert!(
        model.contains("hand_join"),
        "the edit is in the model: {model}"
    );
}

#[test]
fn adding_over_an_unread_model_edit_can_keep_it_as_a_version() {
    let (mut win, dir) = model_with_an_unread_edit("model-version");
    add_in_one_frame(
        &mut win,
        &csv("model-version", "regions.csv", "region,name\nnorth,North\n"),
    );
    assert!(win.app.close_question().is_some(), "the question is up");
    win.app
        .answer_close_question(&win.ctx.clone(), CloseAnswer::CloseWithoutSaving);
    win.settle();
    assert!(win.app.close_question().is_none(), "{}", win.said());
    let file = dir.join("model.ossie.yaml");
    let store = brightfield_protocol::HistoryStore::At(scratch().join("history"));
    let versions = store
        .versions(&file)
        .expect("the model file's versions list");
    let kept = versions
        .versions
        .iter()
        .find(|v| v.kind == brightfield_protocol::chart_history::HistoryKind::Unsaved)
        .expect("the edit is kept as an unsaved version");
    let text = store.read_version(&file, &kept.id).expect("reads back");
    assert!(text.contains("hand_join"), "{text}");
    assert!(
        !std::fs::read_to_string(&file)
            .expect("the model file")
            .contains("hand_join"),
        "and not written into the model"
    );
}

#[test]
fn adding_over_an_unsaved_yaml_buffer_asks() {
    let parent = place("yaml-buffer");
    let mut win = Window::made_in(&parent);
    let input = scratch().join("input").join("yaml-buffer");
    std::fs::create_dir_all(&input).expect("an input folder");
    let houses = input.join("houses.csv");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv"),
        &houses,
    )
    .expect("the housing sample copies");
    win.add(&houses);
    assert!(win.app.own_protocol().is_some(), "{}", win.said());

    let tab = win
        .app
        .rail_name_rect(brightfield_workbench::arrangement::LEDGER_RAIL, 3)
        .expect("the ledger rail drew its editor tab")
        .center();
    win.click(tab);
    assert!(win.app.focus_pane(brightfield_workbench::PaneKey::new(
        brightfield_shell::editor::EDITOR
    )));
    win.settle();
    let rail = win
        .app
        .region_rect(brightfield_workbench::arrangement::LEDGER_RAIL)
        .expect("the ledger rail drew open");
    let body_top = rail.top() + brightfield_workbench::chrome::rail_selector_height();
    win.click(egui::pos2(rail.left() + 150.0, body_top + 32.0));
    win.run(vec![egui::Event::Text(
        "# typed in the editor\n".to_owned(),
    )]);
    win.ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    win.settle();
    assert!(win.app.carries_unsaved_mark(), "the buffer is unsaved");

    win.add(&csv(
        "yaml-buffer",
        "regions.csv",
        "region,name\nnorth,North\n",
    ));
    let question = win.app.close_question().expect("the question is up");
    assert_eq!(question.title(), ADD_SOURCE_QUESTION_TITLE);
    assert!(
        question.edits.iter().any(|e| e.starts_with("YAML editor")),
        "{:?}",
        question.edits
    );
    assert!(win.app.model_pane().is_none(), "nothing is added yet");
}

/// Whether the window raised the banner that says the model could not be saved.
fn model_not_saved(win: &Window) -> bool {
    win.raised("save-model")
}

#[test]
fn a_refused_model_edit_is_asked_about_and_kept_as_a_version() {
    let parent = place("model-refused");
    let mut win = Window::made_in(&parent);
    win.add(&csv("model-refused", "customers.csv", CUSTOMERS));
    win.add(&csv("model-refused", "orders.csv", ORDERS));
    let dir = win.dir().expect("a Protocol");
    let pane = win
        .app
        .model_pane_mut()
        .unwrap_or_else(|| panic!("two sources show the model"));
    let broken = pane.file_text().replace("\ndatasets:\n", "\ndatasets: :\n");
    assert_ne!(broken, pane.file_text(), "the text was broken");
    pane.set_file_draft(broken);
    // The pause passes and the text is read, and refused.
    win.settle();
    let pane = win.app.model_pane().expect("the model");
    assert!(pane.file_error().is_some(), "the edit does not read");
    assert!(!pane.file_edit_armed(), "it was read");
    assert!(win.app.carries_unsaved_mark(), "a refused edit is unsaved");

    win.app
        .leave(&win.ctx.clone(), brightfield_shell::window::Leaving::Home);
    assert!(win.app.close_question().is_some(), "Home asks about it");
    win.app
        .answer_close_question(&win.ctx.clone(), CloseAnswer::SaveAndClose);
    win.settle();
    assert!(model_not_saved(&win), "Save refuses it: {}", win.said());
    assert!(!win.app.front_door_is_live(), "and the window stays");

    win.app
        .leave(&win.ctx.clone(), brightfield_shell::window::Leaving::Home);
    assert!(win.app.close_question().is_some(), "Home asks again");
    win.app
        .answer_close_question(&win.ctx.clone(), CloseAnswer::CloseWithoutSaving);
    win.settle();
    assert!(win.app.front_door_is_live(), "{}", win.said());
    let file = dir.join("model.ossie.yaml");
    let store = brightfield_protocol::HistoryStore::At(scratch().join("history"));
    let versions = store
        .versions(&file)
        .expect("the model file's versions list");
    let kept = versions
        .versions
        .iter()
        .find(|v| v.kind == brightfield_protocol::chart_history::HistoryKind::Unsaved)
        .expect("the refused edit is kept as an unsaved version");
    let text = store.read_version(&file, &kept.id).expect("reads back");
    assert!(text.contains("datasets: :"), "{text}");
}

#[test]
fn a_model_edit_whose_file_cannot_be_written_stops_the_save() {
    use std::os::unix::fs::PermissionsExt as _;
    let (mut win, dir) = model_with_an_unread_edit("model-unwritable");
    let file = dir.join("model.ossie.yaml");
    let before = std::fs::read_to_string(&file).expect("the model file");
    // The model file is written beside itself and renamed over, so a folder that takes no new
    // file is one the model cannot be written in.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555))
        .expect("the folder made read-only");
    win.app
        .leave(&win.ctx.clone(), brightfield_shell::window::Leaving::Home);
    let asked = win.app.close_question().is_some();
    if asked {
        win.app
            .answer_close_question(&win.ctx.clone(), CloseAnswer::SaveAndClose);
    }
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
        .expect("the folder made writable");
    win.settle();
    assert!(asked, "Home asks about the edit");
    assert!(
        model_not_saved(&win),
        "the failed write is said: {}",
        win.said()
    );
    assert!(!win.app.front_door_is_live(), "and the window stays");
    assert_eq!(
        std::fs::read_to_string(&file).expect("the model file"),
        before
    );
    assert!(
        win.app.carries_unsaved_mark(),
        "the edit is still the window's to save"
    );

    // With the folder writable again, the next Save writes it.
    win.app
        .leave(&win.ctx.clone(), brightfield_shell::window::Leaving::Home);
    assert!(win.app.close_question().is_some(), "Home asks again");
    win.app
        .answer_close_question(&win.ctx.clone(), CloseAnswer::SaveAndClose);
    win.settle();
    assert!(win.app.front_door_is_live(), "{}", win.said());
    assert!(
        std::fs::read_to_string(&file)
            .expect("the model file")
            .contains("hand_join"),
        "the edit is in the model"
    );
}
