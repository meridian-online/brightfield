//! The front door's Recents and the tree of the Protocols folder, and the run state each row
//! carries.
//!
//! The tree is read from disk: an ordinary folder is a group, a folder holding `arcform.yaml` is a
//! Protocol and a leaf, and nothing inside a Protocol is looked at. It is reread whenever the door
//! is shown again, so what was arranged in the Finder meanwhile is what the door draws. A row's run
//! state is what the Protocol's newest run record says when the door is drawn, so a run that lands
//! after the last save still reaches the row.
//!
//! Every window is headless, with the `brightfield-shell` binary cargo built as its runner. Each
//! test points the config directory and the Protocols folder at folders of its own under a scratch
//! root, one test at a time, so nothing is written under the home directory and no test sees
//! another's Protocols.

#[path = "support/agent_protocol.rs"]
mod agent_protocol;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use brightfield_shell::design::Mode;
use brightfield_shell::door_tree::{self, Node};
use brightfield_shell::open_queue::OpenRequest;
use brightfield_shell::run::Runner;
use brightfield_shell::window::{Boot, DoorRow, MeridianApp, FOLDER_GONE_TITLE};
use brightfield_shell::{own_files, protocols_dir, startup};
use brightfield_workbench::RunState;

/// The scratch root every test here writes under, emptied once per process.
fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        agent_protocol::arc_stores();
        let root = std::env::temp_dir().join(format!("bf-door-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a scratch root");
        root
    })
}

/// Held for the whole of each test that points the process-wide folders at its own.
static FOLDERS: Mutex<()> = Mutex::new(());

/// Point the config directory and the Protocols folder at fresh folders of `test`'s own, and
/// hold them there until the guard drops. The Protocols folder itself is not made: a test that
/// wants one makes it, as the first open does.
#[must_use = "the folders are only this test's while the guard is held"]
fn folders_of(test: &str) -> (MutexGuard<'static, ()>, PathBuf) {
    let held = FOLDERS.lock().unwrap_or_else(PoisonError::into_inner);
    let root = scratch().join(test);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("config")).expect("a config folder");
    std::env::set_var(startup::CONFIG_DIR_VAR, root.join("config"));
    std::env::set_var(protocols_dir::PROTOCOLS_DIR_VAR, root.join("protocols"));
    (held, root)
}

/// A copy of the committed California Housing sample, named `<name>.csv`, in a folder of its own
/// under `root`.
fn housing_copy(root: &Path, name: &str) -> PathBuf {
    let dir = root.join("input").join(name);
    std::fs::create_dir_all(&dir).expect("an input folder");
    let csv = dir.join(format!("{name}.csv"));
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv"),
        &csv,
    )
    .expect("the housing sample copies");
    csv
}

/// A folder at `dir` that reads as a Protocol named `name`, with no steps.
fn protocol_at(dir: &Path, name: &str) -> PathBuf {
    std::fs::create_dir_all(dir).expect("a Protocol folder");
    std::fs::write(
        dir.join("arcform.yaml"),
        format!("name: {name}\nengine: duckdb\nsteps: []\n"),
    )
    .expect("a manifest");
    dir.to_path_buf()
}

/// Leave a run record in the Protocol at `dir` named `name`, from the committed sample contract,
/// that says the run succeeded or that a step failed.
fn record_a_run(dir: &Path, name: &str, failed: bool) {
    let sample = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../brightfield-protocol/fixtures/sample_run.contract.json");
    let mut text = std::fs::read_to_string(sample).expect("the sample contract reads");
    text = text.replace("\"widgets_demo\"", &format!("\"{name}\""));
    if failed {
        text = text.replacen(
            "\"status\": { \"state\": \"success\"",
            "\"status\": { \"state\": \"failed\"",
            1,
        );
        text = text.replace("\"outcome\": \"success\"", "\"outcome\": \"failed\"");
    }
    let runs = brightfield_shell::run::runs_dir(dir);
    std::fs::create_dir_all(&runs).expect("a runs folder");
    std::fs::write(runs.join("20260718-100107-208baa40.json"), text).expect("a run record");
}

/// Make the working database arc keeps for the Protocol at `dir`, as its loads do.
fn compute_its_loads(dir: &Path) {
    let db = arc::spec::protocol_db_path(dir).expect("arc reads the manifest");
    std::fs::create_dir_all(db.parent().expect("the database's folder")).unwrap();
    std::fs::write(db, b"").expect("a working database");
}

/// The names in `nodes`, depth first, a group's with a trailing `/` and indented by its depth.
fn outline(nodes: &[Node]) -> Vec<String> {
    fn walk(nodes: &[Node], depth: usize, out: &mut Vec<String>) {
        for node in nodes {
            match node {
                Node::Group { name, children } => {
                    out.push(format!("{}{name}/", "  ".repeat(depth)));
                    walk(children, depth + 1, out);
                }
                Node::Protocol { name, .. } => out.push(format!("{}{name}", "  ".repeat(depth))),
            }
        }
    }
    let mut out = Vec::new();
    walk(nodes, 0, &mut out);
    out
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

/// One headless window.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    time: f64,
}

impl Window {
    fn new() -> Self {
        Self {
            app: MeridianApp::headless(Boot::empty(), Mode::Light)
                .running_with(Some(Runner::at(env!("CARGO_BIN_EXE_brightfield-shell")))),
            ctx: egui::Context::default(),
            time: 0.0,
        }
    }

    /// The window, with the one-source Protocol of a fresh copy of the housing sample named
    /// `name` made and opened through the open queue.
    fn opened(root: &Path, name: &str) -> Self {
        let mut win = Self::new();
        win.settle();
        win.open(root, name);
        win
    }

    /// Open a fresh copy of the housing sample named `name`, which makes its Protocol.
    fn open(&mut self, root: &Path, name: &str) {
        self.app
            .open_queue()
            .push(OpenRequest::Path(housing_copy(root, name)));
        self.settle();
        assert_eq!(
            self.dir(),
            Some(protocols_dir::root().join(name)),
            "the window holds the Protocol it made: {}",
            self.said()
        );
    }

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

    /// Press the registry's `open-home` keystroke and draw the door.
    fn go_home(&mut self) {
        self.run(vec![press(
            egui::Key::H,
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        )]);
        self.settle();
        assert!(self.app.front_door_is_live(), "home is the door");
    }

    fn said(&self) -> String {
        self.app
            .notifications()
            .iter()
            .map(|n| format!("{}: {}", n.title, n.body.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Run the whole Protocol with Cmd+Enter and wait for it to land.
    fn run_it(&mut self) {
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
    }

    /// The door's rows for the Protocol in `dir`: its Recents row and its row in the tree.
    fn rows_of(&self, dir: &Path) -> (Option<DoorRow>, Option<DoorRow>) {
        let id = dir.join("arcform.yaml").to_string_lossy().into_owned();
        let find = |in_tree: bool| {
            self.app
                .front_door_rows()
                .iter()
                .find(|r| r.id == id && r.in_tree == in_tree)
                .cloned()
        };
        (find(false), find(true))
    }
}

#[test]
fn groups_nest_and_protocols_are_leaves() {
    let root = tempfile::tempdir().expect("a temp root");
    let root = root.path();
    protocol_at(&root.join("Work/2026/q1"), "q1");
    // A Protocol holding a folder with a manifest of its own is still one leaf.
    let outer = protocol_at(&root.join("Work/outer"), "outer");
    protocol_at(&outer.join("inner"), "inner");
    protocol_at(&root.join("alpha"), "alpha");
    protocol_at(&root.join("Beta"), "beta");
    std::fs::create_dir_all(root.join("Archive")).unwrap();
    protocol_at(&root.join(".hidden/secret"), "secret");
    std::fs::write(root.join("notes.txt"), "a file is neither").unwrap();

    let tree = door_tree::read(root);
    assert_eq!(
        outline(&tree),
        vec!["Archive/", "Work/", "  2026/", "    q1", "  outer", "alpha", "Beta",],
        "groups first, then Protocols, each by name; hidden folders skipped; nothing read inside \
         a Protocol"
    );
    let Node::Group { children, .. } = &tree[1] else {
        panic!("Work is a group");
    };
    assert!(
        matches!(&children[1], Node::Protocol { dir, .. } if *dir == outer),
        "a Protocol leaf names its folder: {children:?}"
    );

    let missing = root.join("not-made");
    assert!(door_tree::read(&missing).is_empty());
    assert!(!missing.exists(), "reading a missing root does not make it");
}

#[test]
fn a_previewed_protocol_reads_previewed_not_run() {
    let (_held, root) = folders_of("previewed");
    let previewed = protocol_at(&root.join("protocols/previewed"), "previewed");
    compute_its_loads(&previewed);
    let unrun = protocol_at(&root.join("protocols/unrun"), "unrun");
    let ran = protocol_at(&root.join("protocols/ran"), "ran");
    compute_its_loads(&ran);
    record_a_run(&ran, "ran", false);
    let broke = protocol_at(&root.join("protocols/broke"), "broke");
    record_a_run(&broke, "broke", true);
    // A record of another Protocol sharing the folder is not this one's run.
    let other = protocol_at(&root.join("protocols/other"), "other");
    compute_its_loads(&other);
    record_a_run(&other, "someone_else", false);

    assert_eq!(door_tree::run_state(&previewed), RunState::Previewed);
    assert_eq!(RunState::Previewed.label(), "previewed, not run");
    assert_eq!(door_tree::run_state(&unrun), RunState::NeverRun);
    assert_eq!(door_tree::run_state(&ran), RunState::Fresh);
    assert_eq!(door_tree::run_state(&broke), RunState::Failed);
    assert_eq!(door_tree::run_state(&other), RunState::Previewed);
    assert_eq!(
        door_tree::run_state(&root.join("protocols/gone")),
        RunState::NeverRun
    );

    // …and the door draws it in those words.
    let mut win = Window::new();
    win.settle();
    let (_, row) = win.rows_of(&previewed);
    assert_eq!(
        row.expect("the tree draws the previewed Protocol").state,
        "previewed, not run"
    );
}

#[test]
fn a_new_protocol_is_on_the_door_before_any_save() {
    let (_held, root) = folders_of("new_protocol");
    let mut win = Window::opened(&root, "made_now");
    let dir = win.dir().expect("a Protocol");
    assert!(
        !win.app
            .title()
            .contains(brightfield_shell::window::UNSAVED_MARK),
        "nothing was saved or is waiting to be"
    );
    win.go_home();
    let (recent, in_tree) = win.rows_of(&dir);
    assert_eq!(
        recent.expect("Recents lists the new Protocol").name,
        "made_now"
    );
    assert_eq!(
        in_tree.expect("the tree lists the new Protocol").name,
        "made_now"
    );
}

#[test]
fn a_run_updates_the_row_without_a_save() {
    let (_held, root) = folders_of("run_updates");
    let mut win = Window::opened(&root, "runs_later");
    let dir = win.dir().expect("a Protocol");
    // The source is taken away, so the run's record says its load failed: a state the open did
    // not leave the Protocol in, and one no save records.
    std::fs::remove_file(root.join("input/runs_later/runs_later.csv"))
        .expect("the source is removed");
    win.run_it();
    win.go_home();
    let (recent, in_tree) = win.rows_of(&dir);
    assert_eq!(
        in_tree.expect("a tree row").state,
        RunState::Failed.label(),
        "the tree row reads the newest run record"
    );
    assert_eq!(
        recent.expect("a Recents row").state,
        RunState::Failed.label(),
        "the Recents row reads the newest run record, not what was remembered at the open"
    );
}

#[test]
fn a_folder_deleted_in_finder_drops_its_row_and_an_open_window_says_so() {
    let (_held, root) = folders_of("deleted_in_finder");
    let mut win = Window::opened(&root, "deleted");
    let deleted = win.dir().expect("a Protocol");
    win.go_home();
    assert!(win.rows_of(&deleted).1.is_some(), "the tree lists it");

    // Deleted in the Finder while the door shows it: its rows go on the next draw.
    std::fs::remove_dir_all(&deleted).expect("the folder is deleted");
    win.settle();
    assert_eq!(
        win.rows_of(&deleted),
        (None, None),
        "neither Recents nor the tree draws a deleted folder"
    );

    // Moved in the Finder while it is open: the window says so, and nothing makes it again.
    win.open(&root, "moved");
    let moved = win.dir().expect("a Protocol");
    let elsewhere = root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::rename(&moved, elsewhere.join("moved")).expect("the folder moves");
    win.settle();
    assert!(
        win.app
            .notifications()
            .iter()
            .any(|n| n.title == FOLDER_GONE_TITLE),
        "the window says the folder is gone: {}",
        win.said()
    );
    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some("house_age > 28".to_owned()),
            order: None,
        });
    win.settle();
    win.run(vec![press(egui::Key::S, command())]);
    win.settle();
    win.run(vec![press(egui::Key::Enter, command())]);
    win.settle();
    assert!(
        !moved.exists(),
        "no save, run or redraw makes the folder again: {}",
        win.said()
    );
    assert!(
        win.app
            .notifications()
            .iter()
            .any(|n| n.title == FOLDER_GONE_TITLE),
        "still said: {}",
        win.said()
    );
}

#[test]
fn old_own_protocols_stay_in_recents() {
    let (_held, _root) = folders_of("old_own");
    let old = startup::datasets_dir()
        .expect("a datasets folder")
        .join("protocols")
        .join(own_files::OWN_DIR)
        .join("made_before-1");
    protocol_at(&old, "made_before-1");
    std::fs::write(old.join(brightfield_shell::discovery::MODEL_FILE), "{}\n").unwrap();
    let old = std::path::absolute(&old).unwrap();

    let mut win = Window::new();
    win.settle();
    let (recent, in_tree) = win.rows_of(&old);
    assert_eq!(
        recent
            .expect("Recents lists the older own-files Protocol")
            .name,
        "made_before-1"
    );
    assert!(in_tree.is_none(), "it is not in the Protocols folder");
}

// The baselines: the door over a fixed tree, in both themes, through the crate's real capture
// path, diffed with the `UPDATE_SNAPSHOTS=1` workflow `front_door.rs` uses.

/// The fixed tree the baselines photograph: groups nested two deep, and a Protocol in each run
/// state the door can say.
fn baseline_tree(root: &Path) {
    let protocols = root.join("protocols");
    let fresh = protocol_at(&protocols.join("Work/2026/earthquakes_2024"), "eq");
    compute_its_loads(&fresh);
    record_a_run(&fresh, "eq", false);
    let failed = protocol_at(&protocols.join("Work/2026/shipments"), "shipments");
    record_a_run(&failed, "shipments", true);
    let previewed = protocol_at(&protocols.join("Work/housing"), "housing");
    compute_its_loads(&previewed);
    protocol_at(&protocols.join("Personal/budget"), "budget");
    protocol_at(&protocols.join("crosswalk"), "crosswalk");
}

fn door_tree_surface(mode: Mode, name: &str) {
    let (_held, root) = folders_of(name);
    baseline_tree(&root);
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.capture.png"));
    let (w, h) = brightfield_shell::capture::capture_png_at(
        Boot::empty(),
        mode,
        1.0,
        (1280.0, 820.0),
        &out,
        Vec::new(),
    )
    .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();
    egui_kittest::image_snapshot(&image, name);
}

#[test]
fn door_tree_light_surface() {
    door_tree_surface(Mode::Light, "door_tree_light");
}

#[test]
fn door_tree_dark_surface() {
    door_tree_surface(Mode::Dark, "door_tree_dark");
}
