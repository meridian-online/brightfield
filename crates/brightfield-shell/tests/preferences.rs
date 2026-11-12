//! Preferences, opened by Cmd+, from anywhere in the window, holds the Protocols folder: where new
//! Protocols are made and the folder the door's tree reads. `BRIGHTFIELD_PROTOCOLS_DIR` still
//! wins over the setting, as a test or a portable install needs. Choosing another folder offers
//! once to move the old folder's Protocols across, one at a time through the same move the
//! Location field makes, stopping at the first that fails and saying which moved. An open
//! Protocol with a draft is moved after the close question, or left where it is.
//!
//! The setting is the process's, so every test here holds one lock while it runs, and the
//! Protocols folder variable is unset for the file so the setting decides. The config directory,
//! the first Protocols folder and arc's directories point at a scratch folder of this file's
//! before anything reads them, so nothing is written under the home directory.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use brightfield_protocol::HistoryStore;
use brightfield_shell::default_app;
use brightfield_shell::design::Mode;
use brightfield_shell::open_queue::OpenRequest;
use brightfield_shell::overlays::{CloseAnswer, MOVE_TO_FOLDER_QUESTION_TITLE};
use brightfield_shell::protocols_dir::{self, Left, MoveReport};
use brightfield_shell::rename::FolderOps;
use brightfield_shell::run::Runner;
use brightfield_shell::window::{
    Boot, Leaving, MeridianApp, MOVE_ALL_BANNER, PREFERENCES_BANNER, UNSAVED_MARK,
};

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-preferences-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "start", "input", "roots"] {
            std::fs::create_dir_all(root.join(sub)).expect("a scratch folder");
        }
        // Set once, before any test in this file reads them. The Protocols folder variable is
        // taken away so the setting decides, and the setting is pointed into the scratch folder
        // before anything could make a Protocol in the default one.
        std::env::set_var(
            brightfield_shell::startup::CONFIG_DIR_VAR,
            root.join("config"),
        );
        std::env::remove_var(protocols_dir::PROTOCOLS_DIR_VAR);
        std::env::set_var("ARCFORM_HISTORY_DIR", root.join("history"));
        std::env::set_var("ARCFORM_DB_DIR", root.join("db"));
        protocols_dir::set_root(&root.join("start")).expect("the first setting is kept");
        root
    })
}

/// The one lock every test holds, since the setting it changes is the process's.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    scratch();
    guard
}

/// Two folders of this test's own, the old Protocols folder and the new, with the setting
/// pointed at the old one.
fn roots(test: &str) -> (PathBuf, PathBuf) {
    let base = scratch().join("roots").join(test);
    let old = base.join("old");
    let new = base.join("new");
    std::fs::create_dir_all(&old).expect("the old folder");
    std::fs::create_dir_all(&new).expect("the new folder");
    protocols_dir::set_root(&old).expect("the old folder is set");
    (old, new)
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

/// Whether the Recents of `app` hold a row for the Protocol in `dir`.
fn in_recents(app: &MeridianApp, dir: &Path) -> bool {
    let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    app.layout().recents.iter().any(|r| {
        let id = Path::new(&r.id);
        id.starts_with(dir) || id.starts_with(&canonical)
    })
}

/// One headless window.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    time: f64,
}

impl Window {
    fn new() -> Self {
        scratch();
        let mut win = Self {
            app: MeridianApp::headless(Boot::empty(), Mode::Light)
                .running_with(Some(Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))))
                .keeping_history(Some(HistoryStore::At(scratch().join("history")))),
            ctx: egui::Context::default(),
            time: 0.0,
        };
        win.settle();
        win
    }

    /// The window, holding the one-source Protocol it made of a fresh housing copy named `name`
    /// in the Protocols folder.
    fn opened(name: &str) -> Self {
        let mut win = Self::new();
        win.open(name);
        win
    }

    /// Open a fresh housing copy named `name`, making its Protocol in the Protocols folder.
    fn open(&mut self, name: &str) {
        self.app
            .open_queue()
            .push(OpenRequest::Path(housing_copy(name)));
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

    fn home(&mut self) {
        self.app.leave(&self.ctx.clone(), Leaving::Home);
        self.settle();
    }

    /// Choose `dir` as the Protocols folder, as the sheet's Choose… does once the picker answers.
    fn choose(&mut self, dir: &Path) {
        self.app.open_preferences();
        self.app.choose_protocols_folder(&self.ctx.clone(), dir);
        self.settle();
    }

    fn answer_offer(&mut self, accept: bool) {
        self.app.answer_move_offer(&self.ctx.clone(), accept);
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

    /// The title and body of the banner `id`, when it is up.
    fn banner(&self, id: &str) -> Option<String> {
        self.app
            .notifications()
            .iter()
            .find(|n| n.id == meridian_egui::NotificationId::new(id))
            .map(|n| format!("{} {}", n.title, n.body.clone().unwrap_or_default()))
    }

    /// Start a whole run with Cmd+Enter, and leave it running.
    fn start_run(&mut self) {
        self.run(vec![press(egui::Key::Enter, command())]);
        assert!(
            self.app.run_in_progress(),
            "cmd-enter started a run: {}",
            self.said()
        );
    }

    /// Run frames until every run the window started has landed.
    fn wait_for_runs(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(120);
        while self.app.run_outstanding() {
            assert!(Instant::now() < deadline, "the run did not land");
            self.run(Vec::new());
            std::thread::sleep(Duration::from_millis(5));
        }
        self.settle();
    }

    fn report(&self) -> MoveReport {
        self.app
            .move_report()
            .cloned()
            .unwrap_or_else(|| panic!("the move reported: {}", self.said()))
    }

    /// The Protocol rows the door's tree drew on the last frame, by name.
    fn tree_rows(&self) -> Vec<String> {
        self.app
            .front_door_rows()
            .iter()
            .filter(|r| r.in_tree)
            .map(|r| r.name.clone())
            .collect()
    }

    /// Run the whole Protocol with Cmd+Enter, wait for it to land, and assert it succeeded.
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
fn cmd_comma_opens_preferences() {
    let _serial = serial();
    let mut win = Window::new();
    win.run(vec![press(egui::Key::Comma, command())]);
    win.settle();
    assert_eq!(win.app.open_overlay(), Some("preferences"));
    assert_eq!(
        win.app.preferences_folder(),
        Some(protocols_dir::root()),
        "the sheet shows the Protocols folder"
    );
}

#[test]
fn the_setting_sends_new_protocols_to_the_new_folder() {
    let _serial = serial();
    let (_old, new) = roots("sends");
    let mut win = Window::new();
    win.choose(&new);
    assert_eq!(protocols_dir::root(), new, "{}", win.said());
    win.open("sent_there");
    assert!(new.join("sent_there").join("arcform.yaml").is_file());
}

#[test]
fn the_env_var_still_wins_over_the_setting() {
    let _serial = serial();
    let (old, new) = roots("env_wins");
    std::env::set_var(protocols_dir::PROTOCOLS_DIR_VAR, &new);
    let read = protocols_dir::root();
    std::env::remove_var(protocols_dir::PROTOCOLS_DIR_VAR);
    assert_eq!(read, new, "the variable wins");
    assert_eq!(
        protocols_dir::root(),
        old,
        "and without it the setting does"
    );
}

#[test]
fn changing_the_folder_offers_to_move_and_declining_moves_nothing() {
    let _serial = serial();
    let (old, new) = roots("declined");
    let mut win = Window::opened("stays_put");
    let dir = win.dir().expect("a Protocol");
    win.home();
    win.choose(&new);
    assert_eq!(
        win.app.move_offer(),
        Some((old.as_path(), new.as_path())),
        "choosing offers to move the old folder's Protocols: {}",
        win.said()
    );
    win.answer_offer(false);
    assert_eq!(win.app.move_offer(), None, "the offer is made once");
    assert!(dir.join("arcform.yaml").is_file(), "the Protocol stayed");
    assert!(!new.join("stays_put").exists(), "and nothing was moved");
    assert!(in_recents(&win.app, &dir), "it is still in Recents");
    assert_eq!(protocols_dir::root(), new);
}

#[test]
fn accepting_moves_each_protocol_and_they_still_run() {
    let _serial = serial();
    let (old, new) = roots("accepted");
    let mut win = Window::opened("top_level");
    win.open("grouped");
    // A Protocol in a group keeps its group in the new folder.
    std::fs::create_dir_all(old.join("work")).expect("a group");
    win.app
        .relocate_protocol(&win.ctx.clone(), &old.join("work"));
    win.settle();
    assert_eq!(win.dir(), Some(old.join("work").join("grouped")));
    win.home();

    win.choose(&new);
    win.answer_offer(true);
    let report = win.report();
    assert_eq!(report.failed, None, "{}", win.said());
    assert_eq!(report.moved.len(), 2, "{report:?}");
    assert!(!old.join("top_level").exists());
    assert!(!old.join("work").join("grouped").exists());
    let moved = new.join("work").join("grouped");
    assert!(moved.join("arcform.yaml").is_file(), "{report:?}");
    assert!(new.join("top_level").join("arcform.yaml").is_file());
    assert!(in_recents(&win.app, &moved), "Recents follow the move");
    assert!(!in_recents(&win.app, &old.join("top_level")));
    assert!(
        win.app.front_door_is_live() && win.dir().is_none(),
        "a move from the door opens nothing: {}",
        win.said()
    );

    let mut runs = Window::new();
    runs.app
        .open_protocol_path(&runs.ctx.clone(), &moved.to_string_lossy());
    runs.settle();
    assert_eq!(runs.dir(), Some(moved), "{}", runs.said());
    runs.run_and_succeed();
}

#[test]
fn a_failed_move_stops_and_names_what_moved() {
    let _serial = serial();
    let (old, new) = roots("failed");
    for _ in 0..3 {
        brightfield_shell::own_files::create_empty(&old).expect("an empty Protocol");
    }
    let rename = |from: &Path, to: &Path| {
        if from.ends_with("Untitled-2") {
            Err(std::io::Error::other("this move was made to fail"))
        } else {
            std::fs::rename(from, to)
        }
    };
    let remove = |dir: &Path| std::fs::remove_dir_all(dir);
    let ops = FolderOps {
        rename: &rename,
        remove: &remove,
    };
    let report = protocols_dir::move_all_with(&old, &new, &[], &ops);
    assert_eq!(report.moved.len(), 1, "{report:?}");
    assert_eq!(report.moved[0].1, new.join("Untitled"));
    let (failed, why) = report.failed.clone().expect("the second failed");
    assert_eq!(failed, old.join("Untitled-2"));
    assert!(why.contains("this move was made to fail"), "{why}");
    assert!(old.join("Untitled-2").is_dir(), "the failed one stays");
    assert!(
        old.join("Untitled-3").is_dir() && !new.join("Untitled-3").exists(),
        "and the move stopped there"
    );
    let said = report.said();
    assert!(
        said.contains("Untitled") && said.contains("Untitled-2") && said.contains("1 of"),
        "it says which moved and which stopped it: {said}"
    );
}

#[test]
fn an_open_protocol_is_moved_after_the_close_question_or_left() {
    let _serial = serial();
    // Saved first, then moved.
    let (_old, new) = roots("open_saved");
    let mut win = Window::opened("open_one");
    win.keep_filter();
    win.choose(&new);
    win.answer_offer(true);
    let question = win.app.close_question().expect("the close question is up");
    assert_eq!(question.title(), MOVE_TO_FOLDER_QUESTION_TITLE);
    win.app
        .answer_close_question(&win.ctx.clone(), CloseAnswer::SaveAndClose);
    win.settle();
    assert_eq!(
        win.dir(),
        Some(new.join("open_one")),
        "it moved, and the window holds it there: {}",
        win.said()
    );
    assert!(!win.app.title().contains(UNSAVED_MARK), "it was saved");
    assert_eq!(win.report().left, Vec::new());

    // Kept editing, so left where it is while the others move.
    let (old, new) = roots("open_left");
    let mut win = Window::opened("other_one");
    win.open("left_one");
    let left = win.dir().expect("a Protocol");
    win.keep_filter();
    win.choose(&new);
    win.answer_offer(true);
    win.app
        .answer_close_question(&win.ctx.clone(), CloseAnswer::KeepEditing);
    win.settle();
    let report = win.report();
    assert_eq!(report.left, vec![(left.clone(), Left::Draft)], "{report:?}");
    assert!(new.join("other_one").join("arcform.yaml").is_file());
    assert_eq!(win.dir(), Some(old.join("left_one")), "the open one stays");
    assert!(win.app.title().contains(UNSAVED_MARK), "with its draft");
    assert!(report.said().contains("left_one"), "{}", report.said());
}

/// Copy the folder `from` to `to`.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("a folder");
    for entry in std::fs::read_dir(from).expect("a folder").flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("a file copies");
        }
    }
}

#[test]
fn accepting_the_move_reopens_an_agent_protocol_at_its_new_place() {
    let _serial = serial();
    let (old, new) = roots("agent_held");
    let dir = old.join("co2");
    copy_dir(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/agent-protocols/co2-per-capita"),
        &dir,
    );
    let mut win = Window::new();
    win.app.open_queue().push(OpenRequest::Path(dir.clone()));
    win.settle();
    assert!(win.app.own_protocol().is_none(), "not one Brightfield made");
    assert_eq!(
        win.app.held_protocol_dir(),
        Some(dir.clone()),
        "the window holds the agent's Protocol: {}",
        win.said()
    );

    win.choose(&new);
    win.answer_offer(true);
    win.settle();

    assert!(new.join("co2").join("arcform.yaml").is_file(), "it moved");
    assert_eq!(
        win.app.held_protocol_dir(),
        Some(new.join("co2")),
        "the window holds it at its new place: {}",
        win.said()
    );
    assert!(
        win.banner(brightfield_shell::window::FOLDER_GONE_BANNER)
            .is_none(),
        "{}",
        win.said()
    );
}

#[test]
fn an_open_during_the_move_question_waits_for_the_move() {
    let _serial = serial();
    let (_old, new) = roots("open_held_behind_move");
    let mut win = Window::opened("moving_one");
    win.keep_filter();
    win.choose(&new);
    win.answer_offer(true);
    assert_eq!(
        win.app.close_question().map(|q| q.title()),
        Some(MOVE_TO_FOLDER_QUESTION_TITLE)
    );

    // A folder opened from the Finder while the question is up.
    let folder = scratch().join("input").join("held_behind_move");
    std::fs::create_dir_all(&folder).expect("a folder");
    std::fs::write(folder.join("held_people.csv"), "id,name\n1,Ada\n2,Bo\n").unwrap();
    std::fs::write(folder.join("held_visits.csv"), "id,person_id\n10,1\n11,2\n").unwrap();
    win.app.open_queue().push(OpenRequest::Path(folder));
    win.settle();
    assert_eq!(
        win.app.close_question().map(|q| q.title()),
        Some(MOVE_TO_FOLDER_QUESTION_TITLE),
        "the question still asks before the move: {}",
        win.said()
    );

    win.app
        .answer_close_question(&win.ctx.clone(), CloseAnswer::SaveAndClose);
    win.settle();
    win.settle();
    assert!(
        new.join("moving_one").join("arcform.yaml").is_file(),
        "the move went ahead: {}",
        win.said()
    );
    let opened = win
        .app
        .model_pane()
        .map(|pane| pane.protocol.dir.clone())
        .unwrap_or_else(|| panic!("then the folder opened: {}", win.said()));
    assert!(opened.starts_with(&new), "{}", opened.display());
}

#[test]
fn the_door_tree_reads_the_new_folder() {
    let _serial = serial();
    let (old, new) = roots("tree");
    brightfield_shell::own_files::create_empty(&old).expect("an empty Protocol");
    std::fs::create_dir_all(new.join("waiting")).expect("a group");
    brightfield_shell::own_files::create_empty(&new.join("waiting")).expect("an empty Protocol");
    let mut win = Window::new();
    assert_eq!(win.tree_rows(), vec!["Untitled".to_string()]);
    std::fs::rename(
        new.join("waiting").join("Untitled"),
        new.join("waiting").join("over_here"),
    )
    .expect("renamed in the Finder");
    win.choose(&new);
    win.answer_offer(false);
    win.app.leave(&win.ctx.clone(), Leaving::Close);
    win.settle();
    assert_eq!(
        win.tree_rows(),
        vec!["over_here".to_string()],
        "the door reads the new folder at once"
    );
}

#[cfg(unix)]
#[test]
fn a_refused_new_folder_says_how_and_keeps_the_old() {
    use std::os::unix::fs::PermissionsExt;
    let _serial = serial();
    let (old, new) = roots("refused");
    std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o500)).unwrap();
    let mut win = Window::new();
    win.choose(&new);
    std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o700)).unwrap();
    let banner = win
        .app
        .notifications()
        .iter()
        .find(|n| n.id == meridian_egui::NotificationId::new(PREFERENCES_BANNER))
        .map(|n| format!("{} {}", n.title, n.body.clone().unwrap_or_default()))
        .unwrap_or_else(|| panic!("a banner says why: {}", win.said()));
    assert!(
        banner.contains("System Settings → Privacy & Security → Files and Folders"),
        "{banner}"
    );
    assert_eq!(protocols_dir::root(), old, "the old setting is kept");
    assert_eq!(win.app.move_offer(), None);
}

#[test]
fn a_folder_inside_the_old_one_or_around_it_is_refused() {
    let _serial = serial();
    let (old, _new) = roots("nested");
    let inner = old.join("inner");
    std::fs::create_dir_all(&inner).unwrap();
    let inside = protocols_dir::set_root(&inner).expect_err("a folder inside the old one");
    assert!(inside.contains("inside"), "{inside}");
    let around = protocols_dir::set_root(old.parent().unwrap()).expect_err("a folder around it");
    assert!(around.contains("inside"), "{around}");
    assert_eq!(protocols_dir::root(), old);
}

/// The Preferences file in this file's config folder.
fn preferences_file() -> PathBuf {
    scratch()
        .join("config")
        .join(protocols_dir::PREFERENCES_FILE)
}

/// The folder new Protocols go to when no setting reads.
fn default_root() -> PathBuf {
    std::env::home_dir()
        .unwrap()
        .join("Documents")
        .join("Protocols")
}

#[test]
fn a_protocol_a_run_is_writing_into_is_left_and_named_while_the_others_move() {
    let _serial = serial();
    let (old, new) = roots("running");
    let mut win = Window::opened("moves_on");
    win.open("still_running");
    let running = win.dir().expect("a Protocol");
    win.start_run();
    // The run goes on after Home, writing its record into the folder.
    win.home();
    assert!(win.app.run_outstanding(), "the run is still out");
    win.choose(&new);
    win.answer_offer(true);
    let report = win.report();
    assert_eq!(
        report.left,
        vec![(running.clone(), Left::Running)],
        "{report:?}"
    );
    assert!(running.join("arcform.yaml").is_file(), "it was not moved");
    assert!(
        new.join("moves_on").join("arcform.yaml").is_file(),
        "the other moved"
    );
    assert!(!old.join("moves_on").exists());
    let said = report.said();
    assert!(
        said.contains("still_running") && said.contains("a run is writing into its folder"),
        "it is named with its reason: {said}"
    );
    assert!(!said.contains("unsaved"), "{said}");
    win.wait_for_runs();
}

#[test]
fn a_protocol_left_is_worded_for_why() {
    let report = MoveReport {
        left: vec![
            (PathBuf::from("/p/drafted"), Left::Draft),
            (PathBuf::from("/p/running"), Left::Running),
        ],
        ..MoveReport::default()
    };
    let said = report.said();
    assert!(
        said.contains("drafted is open with unsaved edits"),
        "{said}"
    );
    assert!(
        said.contains("running stayed in /p: a run is writing into its folder"),
        "{said}"
    );
}

#[test]
fn an_unreadable_setting_says_so_and_new_protocols_go_to_the_default_for_now() {
    let _serial = serial();
    let (old, _new) = roots("unreadable");
    std::fs::write(preferences_file(), "{ not json").unwrap();
    let read = protocols_dir::root();
    let problem = protocols_dir::setting_problem();
    let win = Window::new();
    let banner = win.banner(PREFERENCES_BANNER);
    // Put a readable setting back before asserting, so a failure leaves the file whole.
    std::fs::remove_file(preferences_file()).unwrap();
    protocols_dir::set_root(&old).unwrap();
    assert_eq!(read, default_root(), "the default stands in for now");
    let problem = problem.expect("the problem is said");
    let banner = banner.unwrap_or_else(|| panic!("a banner says so: {}", win.said()));
    assert!(banner.contains("could not be read"), "{banner}");
    assert!(
        banner.contains(&preferences_file().display().to_string()),
        "it names the file: {banner}"
    );
    assert!(problem.contains("could not be read"), "{problem}");
}

#[test]
fn choosing_again_sets_an_unreadable_setting_aside_and_says_so() {
    let _serial = serial();
    let (_old, new) = roots("set_aside");
    std::fs::write(preferences_file(), "[1, 2").unwrap();
    assert!(
        protocols_dir::set_root(&preferences_file()).is_err(),
        "a file is not a folder"
    );
    assert_eq!(
        std::fs::read_to_string(preferences_file()).unwrap(),
        "[1, 2",
        "a refused choice leaves the unreadable file as it was"
    );
    let mut win = Window::new();
    win.choose(&new);
    let aside = scratch().join("config").join("preferences.json.unreadable");
    assert_eq!(
        std::fs::read_to_string(&aside).ok().as_deref(),
        Some("[1, 2"),
        "the unreadable file is kept beside: {}",
        win.said()
    );
    assert_eq!(protocols_dir::root(), new);
    let banner = win.banner(PREFERENCES_BANNER).unwrap_or_default();
    assert!(
        banner.contains("preferences.json.unreadable"),
        "{}",
        win.said()
    );
    let _ = std::fs::remove_file(aside);
}

#[test]
fn esc_on_the_move_question_cancels_the_whole_move() {
    let _serial = serial();
    let (old, new) = roots("esc_cancels");
    let mut win = Window::opened("waits_here");
    win.open("drafted_here");
    win.keep_filter();
    win.choose(&new);
    win.answer_offer(true);
    let question = win.app.close_question().expect("the close question is up");
    assert_eq!(question.title(), MOVE_TO_FOLDER_QUESTION_TITLE);
    win.run(vec![press(egui::Key::Escape, egui::Modifiers::NONE)]);
    win.settle();
    assert!(win.app.close_question().is_none(), "the question is down");
    assert!(win.app.move_report().is_none(), "nothing was moved");
    assert!(old.join("waits_here").join("arcform.yaml").is_file());
    assert!(old.join("drafted_here").join("arcform.yaml").is_file());
    assert!(!new.join("waits_here").exists());
    assert_eq!(
        protocols_dir::root(),
        new,
        "the setting changed all the same"
    );
    assert!(win.app.title().contains(UNSAVED_MARK), "the draft is kept");
    let banner = win
        .banner(MOVE_ALL_BANNER)
        .unwrap_or_else(|| panic!("a banner says so: {}", win.said()));
    assert!(
        banner.contains("are where they were")
            && banner.contains(&format!("New Protocols now go to {}", new.display())),
        "{banner}"
    );
}

#[cfg(unix)]
#[test]
fn a_failed_save_before_the_move_moves_nothing_and_says_so() {
    let _serial = serial();
    let (old, new) = roots("save_fails");
    let mut win = Window::opened("cannot_save");
    let dir = win.dir().expect("a Protocol");
    win.keep_filter();
    win.choose(&new);
    win.answer_offer(true);
    let chmod = |mode: &str| {
        let ok = std::process::Command::new("chmod")
            .args(["-R", mode])
            .arg(&dir)
            .status()
            .expect("chmod runs")
            .success();
        assert!(ok, "chmod {mode}");
    };
    chmod("a-w");
    win.app
        .answer_close_question(&win.ctx.clone(), CloseAnswer::SaveAndClose);
    win.settle();
    chmod("u+w");
    assert!(win.app.move_report().is_none(), "nothing was moved");
    assert!(old.join("cannot_save").join("arcform.yaml").is_file());
    assert!(!new.join("cannot_save").exists());
    let banner = win
        .banner(MOVE_ALL_BANNER)
        .unwrap_or_else(|| panic!("a banner says so: {}", win.said()));
    assert!(
        banner.contains("could not be saved") && banner.contains("no Protocol was moved"),
        "{banner}"
    );
    assert!(
        banner.contains(&format!("New Protocols now go to {}", new.display()))
            && !banner.contains("Nothing else changed"),
        "it says the setting changed: {banner}"
    );
}

#[test]
fn the_variable_shows_its_folder_and_the_setting_is_not_changed() {
    let _serial = serial();
    let (old, new) = roots("variable");
    let elsewhere = scratch().join("roots").join("variable").join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::env::set_var(protocols_dir::PROTOCOLS_DIR_VAR, &elsewhere);
    let mut win = Window::new();
    win.app.open_preferences();
    win.settle();
    let shown = win.app.preferences_folder();
    let can_choose = win.app.preferences_can_choose();
    win.app.choose_protocols_folder(&win.ctx.clone(), &new);
    win.settle();
    let banner = win.banner(PREFERENCES_BANNER);
    std::env::remove_var(protocols_dir::PROTOCOLS_DIR_VAR);
    assert_eq!(
        shown,
        Some(elsewhere),
        "the sheet shows the variable's folder"
    );
    assert_eq!(can_choose, Some(false), "and offers no Choose…");
    assert_eq!(protocols_dir::root(), old, "the setting is unchanged");
    assert!(
        banner.is_some_and(|b| b.contains(protocols_dir::PROTOCOLS_DIR_VAR)),
        "{}",
        win.said()
    );
}

#[test]
fn a_second_unreadable_file_is_kept_beside_the_first() {
    let _serial = serial();
    let (_old, new) = roots("second_aside");
    let config = scratch().join("config");
    let first = config.join("preferences.json.unreadable");
    let second = config.join("preferences.json.unreadable-2");
    std::fs::write(&first, "kept before").unwrap();
    std::fs::write(preferences_file(), "{ broken").unwrap();
    let mut win = Window::new();
    win.choose(&new);
    let kept = std::fs::read_to_string(&first).ok();
    let beside = std::fs::read_to_string(&second).ok();
    let banner = win.banner(PREFERENCES_BANNER).unwrap_or_default();
    let _ = std::fs::remove_file(&first);
    let _ = std::fs::remove_file(&second);
    assert_eq!(
        kept.as_deref(),
        Some("kept before"),
        "the earlier one is kept"
    );
    assert_eq!(beside.as_deref(), Some("{ broken"), "{}", win.said());
    assert!(banner.contains("preferences.json.unreadable-2"), "{banner}");
    assert_eq!(protocols_dir::root(), new);
}

#[cfg(unix)]
#[test]
fn a_folder_running_and_drafted_is_left_once() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("protocols").join("busy");
    std::fs::create_dir_all(&dir).unwrap();
    let link = tmp.path().join("linked");
    std::os::unix::fs::symlink(tmp.path().join("protocols"), &link).unwrap();
    let written_otherwise = tmp.path().join("protocols").join(".").join("busy");
    let left = protocols_dir::left_behind(&[link.join("busy")], Some(&written_otherwise));
    assert_eq!(left, vec![(link.join("busy"), Left::Running)]);
    let other = tmp.path().join("protocols").join("other");
    std::fs::create_dir_all(&other).unwrap();
    let left = protocols_dir::left_behind(&[link.join("busy")], Some(&other));
    assert_eq!(
        left.len(),
        2,
        "a drafted folder of its own is listed: {left:?}"
    );
    assert_eq!(left[1], (other, Left::Draft));
}

/// Take the default-app question's answer out of the Preferences file, as on a Mac that has not
/// been asked yet, keeping the file's other settings.
fn unasked() {
    let path = preferences_file();
    let mut fields: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    fields.remove(default_app::ASKED_FIELD);
    std::fs::write(&path, serde_json::to_string_pretty(&fields).unwrap()).unwrap();
}

/// The default-app question's answer as the Preferences file keeps it, and the Protocols folder
/// it holds beside it.
fn kept_answer() -> (Option<String>, Option<String>) {
    let text = std::fs::read_to_string(preferences_file()).unwrap_or_default();
    let fields: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let field = |name: &str| fields.get(name).and_then(|v| v.as_str()).map(str::to_owned);
    (field(default_app::ASKED_FIELD), field("protocols_dir"))
}

impl Window {
    /// One headless window built as a launch from the app builds it, which may ask the
    /// default-app question.
    fn from_the_app() -> Self {
        scratch();
        let mut win = Self {
            app: MeridianApp::headless(Boot::empty(), Mode::Light).offering_default_app(true),
            ctx: egui::Context::default(),
            time: 0.0,
        };
        win.settle();
        win
    }
}

#[test]
fn the_default_app_question_is_asked_once_and_remembered() {
    let _serial = serial();
    let (old, _new) = roots("default_app_once");
    unasked();
    let mut first = Window::from_the_app();
    assert_eq!(
        first.app.open_overlay(),
        Some("default-app"),
        "the first launch asks"
    );
    // Esc is Not now.
    first.run(vec![press(egui::Key::Escape, egui::Modifiers::NONE)]);
    first.settle();
    assert_eq!(first.app.open_overlay(), None, "the answer closes it");
    assert_eq!(
        first.app.default_app_asks(),
        0,
        "Not now asks macOS nothing"
    );
    let (answer, folder) = kept_answer();
    assert_eq!(answer.as_deref(), Some("declined"), "the answer is kept");
    assert_eq!(
        folder,
        Some(old.display().to_string()),
        "beside the Protocols folder, which is kept"
    );
    assert_eq!(protocols_dir::root(), old);

    let second = Window::from_the_app();
    assert_eq!(
        second.app.open_overlay(),
        None,
        "the next launch does not ask"
    );
    assert_eq!(kept_answer().0.as_deref(), Some("declined"));
}

#[test]
fn accepting_the_default_app_question_asks_macos_and_is_remembered() {
    let _serial = serial();
    let _ = roots("default_app_accept");
    unasked();
    let mut win = Window::from_the_app();
    assert_eq!(win.app.open_overlay(), Some("default-app"));
    // Enter is Make Brightfield the Default.
    win.run(vec![press(egui::Key::Enter, egui::Modifiers::NONE)]);
    win.settle();
    assert_eq!(win.app.open_overlay(), None);
    assert_eq!(win.app.default_app_asks(), 1, "macOS is asked once");
    assert_eq!(kept_answer().0.as_deref(), Some("accepted"));
    let again = Window::from_the_app();
    assert_eq!(again.app.open_overlay(), None, "and not asked again");
}

/// A window that is not a launch from the app, such as a build run from the repo, cannot be made
/// the default, so it does not ask, and leaves the question for the app's first launch.
#[test]
fn a_launch_not_from_the_app_leaves_the_question_unasked() {
    let _serial = serial();
    let _ = roots("default_app_not_app");
    unasked();
    let win = Window::new();
    assert_eq!(win.app.open_overlay(), None);
    assert_eq!(kept_answer().0, None, "nothing is kept");
}

#[test]
fn preferences_makes_brightfield_the_default_at_any_time() {
    let _serial = serial();
    let _ = roots("default_app_preferences");
    unasked();
    let mut win = Window::from_the_app();
    win.run(vec![press(egui::Key::Escape, egui::Modifiers::NONE)]);
    win.settle();
    assert_eq!(kept_answer().0.as_deref(), Some("declined"));
    win.app.open_preferences();
    win.settle();
    win.app.make_default_app(&win.ctx.clone());
    win.settle();
    assert_eq!(win.app.default_app_asks(), 1, "Preferences asks macOS");
    win.app.make_default_app(&win.ctx.clone());
    assert_eq!(win.app.default_app_asks(), 2, "each time it is pressed");
    assert_eq!(
        kept_answer().0.as_deref(),
        Some("declined"),
        "the first launch's answer is kept as it was"
    );
}

/// Opening Preferences over the question and making Brightfield the default there answers the
/// question too, so it does not come back once Preferences closes.
#[test]
fn making_the_default_from_preferences_answers_the_question() {
    let _serial = serial();
    let _ = roots("default_app_answered_there");
    unasked();
    let mut win = Window::from_the_app();
    assert_eq!(win.app.open_overlay(), Some("default-app"));
    win.app.open_preferences();
    win.settle();
    win.app.make_default_app(&win.ctx.clone());
    win.run(vec![press(egui::Key::Escape, egui::Modifiers::NONE)]);
    win.settle();
    assert_eq!(win.app.open_overlay(), None, "the question stays answered");
    assert_eq!(kept_answer().0.as_deref(), Some("accepted"));
}

/// Another sheet taking the window's one modal slot puts the question off until it closes,
/// rather than losing it for the launch.
#[test]
fn the_question_waits_out_another_sheet() {
    let _serial = serial();
    let _ = roots("default_app_waits");
    unasked();
    let mut win = Window::from_the_app();
    win.app.open_preferences();
    win.settle();
    assert_eq!(win.app.open_overlay(), Some("preferences"));
    assert_eq!(kept_answer().0, None, "not answered by being covered");
    win.run(vec![press(egui::Key::Escape, egui::Modifiers::NONE)]);
    win.settle();
    assert_eq!(
        win.app.open_overlay(),
        Some("default-app"),
        "it is asked once the sheet closes"
    );
}
