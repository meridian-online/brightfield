//! A Protocol renamed from the window, moved by the locator band's Location field, or moved in the
//! Finder keeps running: its folder and its `name:` change together, arc's working database moves
//! to the new path's key, the source paths written relative to the folder that leave it are
//! rewritten, and the window, its Recents row and the chart's version history follow it. A rename
//! is kept at once, so it leaves no unsaved mark.
//!
//! Every window is headless, with the `brightfield-shell` binary cargo built as its runner, and the
//! config directory, the Protocols folder and arc's directories point at a scratch folder of this
//! file's, so nothing is written under the home directory.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use brightfield_protocol::HistoryStore;
use brightfield_shell::design::Mode;
use brightfield_shell::open_queue::OpenRequest;
use brightfield_shell::rename;
use brightfield_shell::run::Runner;
use brightfield_shell::window::{Boot, Leaving, MeridianApp, PlaceField, UNSAVED_MARK};

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "protocols", "input", "elsewhere"] {
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

/// A copy of the committed California Housing sample, named `<name>.csv`, in a folder of its own.
fn housing_copy(name: &str) -> PathBuf {
    let dir = scratch().join("input").join(name);
    std::fs::create_dir_all(&dir).expect("an input folder");
    let csv = dir.join(format!("{name}.csv"));
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv"),
        &csv,
    )
    .expect("the housing sample copies");
    csv
}

fn command() -> egui::Modifiers {
    egui::Modifiers {
        command: true,
        ..Default::default()
    }
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

/// The version store every window here keeps its chart versions in.
fn history() -> HistoryStore {
    HistoryStore::At(scratch().join("history"))
}

/// The working database arc keeps for the Protocol in `dir`.
fn database(dir: &Path) -> PathBuf {
    arc::spec::protocol_db_path(dir).expect("arc reads the manifest")
}

/// One headless window, holding the one-source Protocol it was opened on.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    time: f64,
}

impl Window {
    fn new() -> Self {
        scratch();
        Self {
            app: MeridianApp::headless(Boot::empty(), Mode::Light)
                .running_with(Some(Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))))
                .keeping_history(Some(history())),
            ctx: egui::Context::default(),
            time: 0.0,
        }
    }

    /// The window, with the one-source Protocol of a fresh copy of the housing sample named
    /// `name` made and opened through the open queue.
    fn opened(name: &str) -> Self {
        let mut win = Self::new();
        win.open(OpenRequest::Path(housing_copy(name)));
        assert_eq!(
            win.dir(),
            Some(scratch().join("protocols").join(name)),
            "the window holds the Protocol it made: {}",
            win.said()
        );
        win
    }

    fn open(&mut self, request: OpenRequest) {
        self.app.open_queue().push(request);
        self.settle();
    }

    /// The folder of the Protocol the window holds.
    fn dir(&self) -> Option<PathBuf> {
        self.app.own_protocol().map(|p| p.dir.clone())
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
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440.0, 900.0),
            )),
            time: Some(self.time),
            modifiers,
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

    /// Replace what the focused field holds with `text` and press Enter.
    fn type_over(&mut self, text: &str) {
        self.run(vec![press(egui::Key::A, command())]);
        self.run(vec![egui::Event::Text(text.to_owned())]);
        self.run(vec![press(egui::Key::Enter, egui::Modifiers::NONE)]);
        self.settle();
    }

    fn said(&self) -> String {
        self.app
            .notifications()
            .iter()
            .map(|n| format!("{}: {}", n.title, n.body.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Run the whole Protocol with Cmd+Enter, wait for it to land, and assert its record says it
    /// succeeded.
    fn run_and_succeed(&mut self) {
        self.run(vec![press(egui::Key::Enter, command())]);
        assert!(
            self.app.run_in_progress(),
            "cmd-enter started a run: {}",
            self.said()
        );
        let deadline = Instant::now() + Duration::from_secs(120);
        while self.app.run_outstanding() {
            assert!(Instant::now() < deadline, "the run did not land");
            self.run(Vec::new());
            std::thread::sleep(Duration::from_millis(5));
        }
        self.settle();
        let run = self.app.protocol_model().run();
        assert!(
            run.is_some_and(|r| r.outcome == brightfield_protocol::contract::Outcome::Success),
            "the run succeeded: {:?}; {}",
            run.map(|r| r.outcome),
            self.said()
        );
    }

    /// Keep a filter on the grid, unsaved.
    fn keep_filter(&mut self) {
        self.app
            .chart_doc_mut()
            .apply_grid_view(brightfield_engine::RowsView {
                condition: Some("house_age > 28".to_owned()),
                order: None,
            });
        self.settle();
        assert!(self.app.title().contains(UNSAVED_MARK));
    }
}

#[test]
fn rename_renames_the_folder_and_the_protocol_still_runs() {
    let mut win = Window::opened("rename_runs");
    let old = win.dir().expect("a Protocol");
    let old_db = database(&old);
    assert!(old_db.is_file(), "the open ran the Protocol: {old_db:?}");

    win.app.rename_protocol(&win.ctx.clone(), "renamed_runs");
    win.settle();

    let new = scratch().join("protocols").join("renamed_runs");
    assert_eq!(win.dir(), Some(new.clone()), "{}", win.said());
    assert!(!old.exists(), "the old folder is gone");
    assert_eq!(manifest_name(&new), "renamed_runs");
    assert!(
        database(&new).is_file(),
        "arc's database is at the new path's key"
    );
    assert!(
        !old_db.exists(),
        "and not left at the old path's key: {old_db:?}"
    );
    assert_eq!(rename::moved_since(&new), None, "the new place is recorded");
    win.run_and_succeed();
}

#[test]
fn rename_keeps_recents_and_versions() {
    let mut win = Window::opened("rename_recents");
    let old = win.dir().expect("a Protocol");
    win.keep_filter();
    win.run(vec![press(egui::Key::S, command())]);
    win.settle();
    assert!(!win.app.title().contains(UNSAVED_MARK), "{}", win.said());
    let chart = win
        .app
        .chart_doc()
        .spec_path
        .clone()
        .expect("the saved chart has a file");
    let inside = chart
        .strip_prefix(&old)
        .expect("inside the Protocol")
        .to_path_buf();
    let before = history()
        .versions(&chart)
        .expect("the versions list")
        .versions
        .len();
    assert!(before > 0, "the save recorded a version");
    assert!(
        win.app
            .layout()
            .recents
            .iter()
            .any(|r| Path::new(&r.id).starts_with(&old)),
        "the save remembered the Protocol: {:?}",
        win.app.layout().recents
    );

    win.app.rename_protocol(&win.ctx.clone(), "renamed_recents");
    win.settle();

    let new = scratch().join("protocols").join("renamed_recents");
    assert_eq!(win.dir(), Some(new.clone()), "{}", win.said());
    let recents = &win.app.layout().recents;
    assert!(
        recents.iter().any(|r| Path::new(&r.id).starts_with(&new)),
        "the row is under the new path: {recents:?}"
    );
    assert!(
        !recents.iter().any(|r| Path::new(&r.id).starts_with(&old)),
        "and none is left under the old one: {recents:?}"
    );
    let listed = history()
        .versions(&new.join(&inside))
        .expect("the versions list under the new path")
        .versions;
    assert_eq!(listed.len(), before, "every version followed");
    assert!(
        listed.iter().all(|v| v.on_this_machine),
        "each with its text on this machine"
    );
}

#[test]
fn rename_to_a_taken_name_is_refused() {
    let mut win = Window::opened("rename_taken");
    let dir = win.dir().expect("a Protocol");
    let taken = scratch().join("protocols").join("rename_taken_other");
    std::fs::create_dir_all(&taken).expect("a folder in the way");
    let manifest = std::fs::read_to_string(dir.join("arcform.yaml")).expect("the manifest");

    let refused = rename::rename(&dir, "rename_taken_other");
    assert!(refused.is_err(), "a taken name is refused");
    for bad in ["", "a/b"] {
        assert!(rename::rename(&dir, bad).is_err(), "{bad:?} is refused");
    }

    // The window says why, and keeps the Protocol where it was.
    win.app
        .rename_protocol(&win.ctx.clone(), "rename_taken_other");
    win.settle();
    assert_eq!(win.dir(), Some(dir.clone()));
    assert!(
        win.said().contains("rename_taken_other"),
        "a banner names the taken name: {}",
        win.said()
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("arcform.yaml")).expect("the manifest"),
        manifest,
        "the manifest is as it was"
    );
    assert_eq!(
        std::fs::read_dir(&taken).expect("the other folder").count(),
        0,
        "nothing was moved into the other folder"
    );
}

#[test]
fn a_protocol_moved_in_finder_finds_its_sources() {
    let win = Window::opened("finder_moved");
    let dir = win.dir().expect("a Protocol");
    let csv = scratch()
        .join("input")
        .join("finder_moved")
        .join("finder_moved.csv")
        .canonicalize()
        .expect("the source");
    let package = dir.join("packages/finder_moved/datapackage.json");
    let path_in = |package: &Path| -> String {
        let text = std::fs::read_to_string(package).expect("the package reads");
        let value: serde_json::Value = serde_json::from_str(&text).expect("the package parses");
        value["resources"][0]["path"]
            .as_str()
            .expect("a resource path")
            .to_owned()
    };
    let relative = path_in(&package);
    assert!(
        relative.starts_with(".."),
        "the fixture's package names its source by a path leaving the folder: {relative}"
    );
    let canonical = dir.canonicalize().expect("the folder");
    drop(win);

    // Moved into a group, as the Finder would.
    let group = scratch().join("protocols").join("a_group");
    std::fs::create_dir_all(&group).expect("a group");
    let moved = group.join("finder_moved");
    std::fs::rename(&dir, &moved).expect("the folder moves");
    let was = rename::moved_since(&moved).expect("the open can tell it moved");
    assert_eq!(was, canonical, "where it was last recorded");

    let mut win = Window::new();
    win.open(OpenRequest::Path(moved.clone()));
    assert_eq!(win.dir(), Some(moved.clone()), "it opened: {}", win.said());
    let repaired = path_in(&moved.join("packages/finder_moved/datapackage.json"));
    assert_ne!(repaired, relative, "the package's path was rewritten");
    assert_eq!(
        moved
            .join("packages/finder_moved")
            .join(&repaired)
            .canonicalize()
            .expect("the repaired path names a file"),
        csv,
        "and it names the source again"
    );
    assert_eq!(
        rename::moved_since(&moved),
        None,
        "the new place is recorded"
    );
    win.run_and_succeed();
}

#[test]
fn repair_rewrites_paths_that_the_move_broke_and_nothing_else() {
    let root = scratch().join("repair");
    let _ = std::fs::remove_dir_all(&root);
    let was = root.join("old").join("p");
    let now = root.join("new").join("deeper").join("p");
    std::fs::create_dir_all(now.join("packages/sales")).expect("the fixture");
    std::fs::create_dir_all(root.join("data")).expect("the data folder");
    std::fs::write(
        now.join("packages/sales/datapackage.json"),
        r#"{"resources":[{"name":"sales","path":"../../../../data/sales.csv"},{"name":"inside","path":"../../local/x.csv"}]}"#,
    )
    .expect("a package");
    std::fs::write(
        now.join(".sources.json"),
        r#"{"tables":[{"name":"sales","file":"../../data/sales.csv","reader":"csv"},{"name":"abs","file":"/data/abs.csv","reader":"csv"}],"skipped":[]}"#,
    )
    .expect("a record");
    std::fs::create_dir_all(now.join("panels/tables")).expect("the panels folder");
    std::fs::write(
        now.join("panels/tables/sales.yaml"),
        format!(
            "data:\n  opened:\n    file: '{}'\n  other:\n    file: '/data/abs.parquet'\n",
            was.join(".grid/sales.parquet").display()
        ),
    )
    .expect("a table's chart");

    let count = rename::repair(&now, &was).expect("the repair runs");

    assert_eq!(
        count, 3,
        "the package's leaving path, the record's, and the chart's grid copy"
    );
    let chart = std::fs::read_to_string(now.join("panels/tables/sales.yaml")).unwrap();
    assert!(
        chart.contains(&format!(
            "file: '{}'",
            now.join(".grid/sales.parquet").display()
        )),
        "{chart}"
    );
    assert!(
        chart.contains("file: '/data/abs.parquet'"),
        "a path outside is left: {chart}"
    );
    let package = std::fs::read_to_string(now.join("packages/sales/datapackage.json")).unwrap();
    assert!(
        package.contains("\"../../../../../data/sales.csv\""),
        "{package}"
    );
    assert!(
        package.contains("\"../../local/x.csv\""),
        "an inside path is left: {package}"
    );
    let sources = std::fs::read_to_string(now.join(".sources.json")).unwrap();
    assert!(sources.contains("\"../../../data/sales.csv\""), "{sources}");
    assert!(
        sources.contains("\"/data/abs.csv\""),
        "an absolute path is left: {sources}"
    );
    assert_eq!(
        rename::repair(&now, &now).expect("again"),
        0,
        "nothing to do in place"
    );
}

#[test]
fn rename_is_kept_at_once() {
    let mut win = Window::opened("kept_at_once");
    assert!(!win.app.title().contains(UNSAVED_MARK));

    // F2 opens the name field in the locator band, holding the Protocol's name.
    win.run(vec![press(egui::Key::F2, egui::Modifiers::NONE)]);
    win.settle();
    assert_eq!(
        win.app.place_field(),
        Some(PlaceField::Name),
        "F2 opened the name field"
    );
    win.type_over("kept_at_once_renamed");

    let new = scratch().join("protocols").join("kept_at_once_renamed");
    assert_eq!(win.dir(), Some(new.clone()), "{}", win.said());
    assert_eq!(win.app.place_field(), None, "the field closed");
    assert!(
        !win.app.title().contains(UNSAVED_MARK),
        "a rename leaves no unsaved mark: {}",
        win.app.title()
    );
    assert!(!win.app.has_unsaved_work());
}

#[test]
fn a_rename_over_a_draft_asks_the_close_question_first() {
    let mut win = Window::opened("rename_over_draft");
    win.keep_filter();

    win.app
        .rename_protocol(&win.ctx.clone(), "rename_over_draft_2");
    win.settle();

    assert!(
        matches!(win.app.leaving(), Some(Leaving::Move(_))),
        "the rename waits for the question: {:?}",
        win.app.leaving()
    );
    assert!(
        scratch()
            .join("protocols")
            .join("rename_over_draft")
            .exists(),
        "nothing moved yet"
    );
    win.app.answer_close_question(
        &win.ctx.clone(),
        brightfield_shell::overlays::CloseAnswer::SaveAndClose,
    );
    win.settle();
    let new = scratch().join("protocols").join("rename_over_draft_2");
    assert_eq!(win.dir(), Some(new.clone()), "{}", win.said());
    assert!(
        std::fs::read_to_string(new.join("arcform.yaml"))
            .expect("the manifest")
            .contains("_filtered_by_house_age"),
        "the draft was saved before the move"
    );
}

#[test]
fn relocate_moves_the_protocol_and_it_still_runs() {
    let mut win = Window::opened("relocated");
    let old = win.dir().expect("a Protocol");
    let elsewhere = scratch().join("elsewhere");
    // A folder of the same name is already there, so the move claims the next name.
    std::fs::create_dir_all(elsewhere.join("relocated")).expect("a folder in the way");

    assert!(
        rename::relocate(&old, &old.join("packages")).is_err(),
        "a move into the Protocol itself is refused"
    );
    win.app.relocate_protocol(&win.ctx.clone(), &elsewhere);
    win.settle();

    let new = elsewhere.join("relocated-2");
    assert_eq!(win.dir(), Some(new.clone()), "{}", win.said());
    assert!(!old.exists(), "the old folder is gone");
    assert_eq!(manifest_name(&new), "relocated-2");
    assert!(database(&new).is_file(), "arc's database moved with it");
    win.run_and_succeed();
}

#[test]
fn the_location_field_moves_it() {
    let mut win = Window::opened("location_field");
    let old = win.dir().expect("a Protocol");
    let to = scratch().join("elsewhere").join("by_location");
    std::fs::create_dir_all(&to).expect("the new parent");

    let field = win
        .app
        .locator_location_rect()
        .expect("the locator band drew the Location field");
    win.click(field.center());
    assert_eq!(win.app.place_field(), Some(PlaceField::Location));
    win.type_over(&to.to_string_lossy());

    let new = to.join("location_field");
    assert_eq!(win.dir(), Some(new.clone()), "{}", win.said());
    assert!(!old.exists());
    assert!(!win.app.title().contains(UNSAVED_MARK));
}

#[test]
fn relocate_across_volumes_copies_then_removes() {
    let win = Window::opened("across_volumes");
    let old = win.dir().expect("a Protocol");
    drop(win);
    let files_before: Vec<PathBuf> = walk(&old)
        .into_iter()
        .map(|p| p.strip_prefix(&old).unwrap().to_path_buf())
        .collect();
    let to = scratch().join("elsewhere").join("other_volume");
    std::fs::create_dir_all(&to).expect("the new parent");
    let tried = std::cell::Cell::new(0);

    let new = rename::relocate_with(&old, &to, &|_, _| {
        tried.set(tried.get() + 1);
        Err(std::io::Error::from(std::io::ErrorKind::CrossesDevices))
    })
    .expect("the move copies instead");

    assert!(tried.get() >= 1, "the rename was tried first");
    assert_eq!(new, to.join("across_volumes"));
    assert!(
        !old.exists(),
        "the original is removed once the copy checks"
    );
    let files_after: Vec<PathBuf> = walk(&new)
        .into_iter()
        .map(|p| p.strip_prefix(&new).unwrap().to_path_buf())
        .collect();
    assert_eq!(files_after, files_before, "every file was copied");
    assert!(database(&new).is_file(), "arc's database moved with it");
}

/// Every file under `dir`, sorted.
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).expect("a folder").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_front_door_row_renames_its_protocol() {
    let mut win = Window::opened("door_row");
    let old = win.dir().expect("a Protocol");
    win.keep_filter();
    win.run(vec![press(egui::Key::S, command())]);
    win.settle();
    win.run(vec![press(
        egui::Key::H,
        command() | egui::Modifiers::SHIFT,
    )]);
    win.settle();
    assert!(win.app.front_door_is_live(), "Home showed the door");
    let row = win
        .app
        .front_door_rows()
        .iter()
        .find(|r| Path::new(&r.id).starts_with(&old))
        .cloned()
        .expect("the door has a row for the Protocol");

    // F2 over the row opens its name for typing.
    win.run(vec![egui::Event::PointerMoved(row.rect.center())]);
    win.run(vec![press(egui::Key::F2, egui::Modifiers::NONE)]);
    win.settle();
    assert_eq!(win.app.door_rename_row(), Some(row.id.as_str()));
    win.type_over("door_row_renamed");

    let new = scratch().join("protocols").join("door_row_renamed");
    assert!(new.join("arcform.yaml").is_file(), "{}", win.said());
    assert!(!old.exists(), "the old folder is gone");
    assert_eq!(manifest_name(&new), "door_row_renamed");
    assert!(
        win.app
            .front_door_rows()
            .iter()
            .any(|r| Path::new(&r.id).starts_with(&new)),
        "the row names the new folder: {:?}",
        win.app.front_door_rows()
    );
}
