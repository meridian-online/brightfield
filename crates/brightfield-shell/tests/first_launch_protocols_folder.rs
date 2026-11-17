//! The first launch asks for the Protocols folder: the managed edition reads only folders the
//! analyst granted, so its first launch asks one thing, where the Protocols folder is, in a panel
//! opened on Documents with a folder named Protocols proposed. A cancel leaves the door as it is,
//! and New Protocol asks again. The full edition always has its Protocols folder and never asks.
//!
//! The panel is replaced by a stand-in that records where it was opened and the name it proposed,
//! and answers as the test says. Every test runs in either edition's build and checks the half
//! that applies.
//!
//! ```text
//! cargo test -p brightfield-shell --features managed-edition --test first_launch_protocols_folder
//! cargo test -p brightfield-shell --test first_launch_protocols_folder
//! ```

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use brightfield_shell::design::Mode;
use brightfield_shell::edition::{self, Edition};
use brightfield_shell::protocols_dir;
use brightfield_shell::window::{
    Boot, MeridianApp, NO_PROTOCOLS_FOLDER_BANNER, PROTOCOLS_FOLDER_OUT_OF_REACH_BANNER,
};

fn managed() -> bool {
    edition::current() == Edition::Managed
}

/// This file's scratch folder, with the config folder and arc's folders in it, so the process's
/// grant store and Preferences are written here and nothing under the home directory. The
/// Protocols folder variable is cleared, since it would decide the folder and nothing would ask.
fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-first-launch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "chosen"] {
            std::fs::create_dir_all(root.join(sub)).expect("a scratch folder");
        }
        let root = std::fs::canonicalize(&root).expect("the scratch folder resolves");
        // Set once, before any test in this file reads them, and never changed.
        std::env::set_var(
            brightfield_shell::startup::CONFIG_DIR_VAR,
            root.join("config"),
        );
        std::env::remove_var(protocols_dir::PROTOCOLS_DIR_VAR);
        std::env::set_var("ARCFORM_HISTORY_DIR", root.join("history"));
        std::env::set_var("ARCFORM_DB_DIR", root.join("db"));
        root
    })
}

/// One test at a time, each starting from a first launch: no Protocols folder in Preferences.
/// The process's grant store keeps what earlier tests granted, so each test names a folder of its
/// own, which no earlier test granted.
fn first_launch() -> MutexGuard<'static, ()> {
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    let guard = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let preferences = scratch()
        .join("config")
        .join(protocols_dir::PREFERENCES_FILE);
    let _ = std::fs::remove_file(preferences);
    guard
}

/// Where the panel's stand-in was opened and the name it proposed each time, and the folder it
/// answers, or `None` for a cancel.
#[derive(Clone, Default)]
struct Panel {
    asked: Arc<Mutex<Vec<(PathBuf, String)>>>,
    answer: Arc<Mutex<Option<PathBuf>>>,
}

impl Panel {
    fn asked(&self) -> Vec<(PathBuf, String)> {
        self.asked.lock().expect("the record").clone()
    }

    fn answer(&self, folder: Option<PathBuf>) {
        *self.answer.lock().expect("the answer") = folder;
    }

    fn asker(&self) -> Box<brightfield_shell::ask_grant::ProtocolsAsker> {
        let panel = self.clone();
        Box::new(move |on: &Path, proposed: &str| {
            panel
                .asked
                .lock()
                .expect("the record")
                .push((on.to_path_buf(), proposed.to_string()));
            panel.answer.lock().expect("the answer").clone()
        })
    }
}

/// A headless window, as a launch makes it, whose Protocols-folder panel `panel` answers.
fn launch(panel: &Panel) -> MeridianApp {
    scratch();
    MeridianApp::headless(Boot::empty(), Mode::Light).asking_protocols_folder_with(panel.asker())
}

fn frame(app: &mut MeridianApp, ctx: &egui::Context) {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1440.0, 900.0),
        )),
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| app.draw(ui));
}

fn raised(app: &MeridianApp, id: &str) -> bool {
    app.notifications()
        .iter()
        .any(|n| n.id == meridian_egui::NotificationId::new(id))
}

/// What the panel is expected to have been asked: on the analyst's Documents folder, with
/// Protocols proposed, `times` times.
fn asked_on_documents(times: usize) -> Vec<(PathBuf, String)> {
    vec![(protocols_dir::documents(), "Protocols".to_string()); times]
}

#[test]
fn the_first_managed_launch_asks_once_with_protocols_proposed() {
    let _first = first_launch();
    let chosen = scratch().join("chosen").join("first").join("Protocols");
    let panel = Panel::default();
    panel.answer(Some(chosen.clone()));
    let ctx = egui::Context::default();

    let mut app = launch(&panel);
    for _ in 0..3 {
        frame(&mut app, &ctx);
    }
    if !managed() {
        assert!(panel.asked().is_empty(), "the full edition asked");
        return;
    }
    assert_eq!(
        panel.asked(),
        asked_on_documents(1),
        "the first launch asks once, on Documents, proposing Protocols"
    );
    assert!(chosen.is_dir(), "the folder named is made");
    assert_eq!(
        protocols_dir::granted_root(),
        Some(chosen.clone()),
        "the folder named is the Protocols folder, granted"
    );
    assert!(
        brightfield_shell::grants::store()
            .covering(&chosen.join("quakes").join("arcform.yaml"))
            .is_some(),
        "its grant covers the Protocols made in it"
    );
    assert!(app.front_door_is_live(), "the door stays");

    let again = Panel::default();
    let mut second = launch(&again);
    for _ in 0..3 {
        frame(&mut second, &ctx);
    }
    assert!(
        again.asked().is_empty(),
        "a launch with the folder granted asks nothing"
    );
    assert_eq!(protocols_dir::root(), chosen);
}

#[test]
fn cancelling_leaves_the_door_open_and_asks_again_on_new_protocol() {
    let _first = first_launch();
    let panel = Panel::default();
    let ctx = egui::Context::default();

    let mut app = launch(&panel);
    for _ in 0..3 {
        frame(&mut app, &ctx);
    }
    if !managed() {
        assert!(panel.asked().is_empty(), "the full edition asked");
        app.ask_new_protocol();
        frame(&mut app, &ctx);
        assert_eq!(app.open_overlay(), Some("new-protocol"));
        assert!(panel.asked().is_empty(), "the full edition asked");
        return;
    }
    assert_eq!(panel.asked(), asked_on_documents(1));
    assert!(app.front_door_is_live(), "a cancel leaves the door");
    assert_eq!(app.open_overlay(), None, "and opens nothing over it");
    assert!(
        raised(&app, NO_PROTOCOLS_FOLDER_BANNER),
        "the window says there is no Protocols folder yet"
    );
    assert_eq!(protocols_dir::granted_root(), None);

    // The door does not ask a second time this launch.
    for _ in 0..3 {
        frame(&mut app, &ctx);
    }
    assert_eq!(panel.asked(), asked_on_documents(1));

    // New Protocol asks again, and cancelled, opens no sheet.
    app.ask_new_protocol();
    frame(&mut app, &ctx);
    assert_eq!(panel.asked(), asked_on_documents(2));
    assert_eq!(app.open_overlay(), None);

    // Answered, the folder is kept and New Protocol's sheet opens on it.
    let chosen = scratch().join("chosen").join("cancelled").join("Protocols");
    panel.answer(Some(chosen.clone()));
    app.ask_new_protocol();
    frame(&mut app, &ctx);
    assert_eq!(panel.asked(), asked_on_documents(3));
    assert_eq!(protocols_dir::granted_root(), Some(chosen));
    assert_eq!(app.open_overlay(), Some("new-protocol"));
    assert!(!raised(&app, NO_PROTOCOLS_FOLDER_BANNER));
}

#[test]
#[cfg_attr(
    feature = "managed-edition",
    ignore = "a managed-edition build, where the other tests check the managed half"
)]
fn the_full_edition_never_asks() {
    let _first = first_launch();
    let panel = Panel::default();
    let ctx = egui::Context::default();
    let mut app = launch(&panel);
    for _ in 0..3 {
        frame(&mut app, &ctx);
    }
    app.ask_new_protocol();
    frame(&mut app, &ctx);
    assert!(panel.asked().is_empty(), "the full edition asked");
    assert_eq!(app.open_overlay(), Some("new-protocol"));
    assert!(
        protocols_dir::granted_root().is_some(),
        "the full edition always has a Protocols folder"
    );
    assert!(!raised(&app, NO_PROTOCOLS_FOLDER_BANNER));
}

#[test]
fn a_chosen_folder_out_of_reach_is_asked_for_again_not_replaced() {
    let _first = first_launch();
    // A Protocols folder chosen on an earlier launch whose grant this launch cannot reach: as
    // Preferences holds it, with no grant covering it.
    let kept = scratch()
        .join("chosen")
        .join("out-of-reach")
        .join("Protocols");
    std::fs::create_dir_all(&kept).expect("the folder");
    let preferences = scratch()
        .join("config")
        .join(protocols_dir::PREFERENCES_FILE);
    std::fs::write(
        &preferences,
        serde_json::json!({ "protocols_dir": kept.to_string_lossy() }).to_string(),
    )
    .expect("the Preferences file");

    let panel = Panel::default();
    let regrant: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    let answer: Arc<Mutex<bool>> = Arc::default();
    let (asked, choose) = (regrant.clone(), answer.clone());
    let ctx = egui::Context::default();
    let mut app = launch(&panel).asking_grants_with(Box::new(move |folder: &Path, _why: &str| {
        asked.lock().expect("the record").push(folder.to_path_buf());
        (*choose.lock().expect("the answer")).then(|| folder.to_path_buf())
    }));
    for _ in 0..3 {
        frame(&mut app, &ctx);
    }
    assert!(
        panel.asked().is_empty(),
        "a folder chosen before is not replaced by the first launch's proposal"
    );
    if !managed() {
        assert!(regrant.lock().expect("the record").is_empty());
        assert_eq!(protocols_dir::granted_root(), Some(kept));
        return;
    }
    assert!(
        regrant.lock().expect("the record").is_empty(),
        "the launch asks nothing"
    );
    assert!(raised(&app, PROTOCOLS_FOLDER_OUT_OF_REACH_BANNER));
    assert_eq!(protocols_dir::out_of_reach(), Some(kept.clone()));

    // New Protocol asks for that folder again; cancelled, the setting stays.
    app.ask_new_protocol();
    frame(&mut app, &ctx);
    assert_eq!(*regrant.lock().expect("the record"), vec![kept.clone()]);
    assert_eq!(protocols_dir::chosen(), Some(kept.clone()));
    assert_eq!(app.open_overlay(), None);

    // Granted, the sheet opens on it, still the same folder.
    *answer.lock().expect("the answer") = true;
    app.ask_new_protocol();
    frame(&mut app, &ctx);
    assert_eq!(protocols_dir::granted_root(), Some(kept));
    assert_eq!(app.open_overlay(), Some("new-protocol"));
    assert!(panel.asked().is_empty());
    assert!(!raised(&app, PROTOCOLS_FOLDER_OUT_OF_REACH_BANNER));
}

#[test]
fn a_window_with_only_a_grant_stand_in_asks_nothing_at_first_launch() {
    let _first = first_launch();
    let ctx = egui::Context::default();
    let mut app = MeridianApp::headless(Boot::empty(), Mode::Light)
        .asking_grants_with(Box::new(|_: &Path, _: &str| None));
    for _ in 0..3 {
        frame(&mut app, &ctx);
    }
    app.ask_new_protocol();
    frame(&mut app, &ctx);
    assert!(
        !raised(&app, NO_PROTOCOLS_FOLDER_BANNER),
        "nothing was asked, so nothing was cancelled"
    );
}

#[test]
fn a_containers_home_is_the_home_it_sits_in() {
    assert_eq!(
        protocols_dir::home_outside_container(Path::new(
            "/Users/analyst/Library/Containers/online.meridian.brightfield/Data"
        )),
        PathBuf::from("/Users/analyst")
    );
    assert_eq!(
        protocols_dir::home_outside_container(Path::new("/Users/analyst")),
        PathBuf::from("/Users/analyst")
    );
}
