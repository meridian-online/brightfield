//! Every way a path arrives asks for its folder: in the managed edition an open whose folders are
//! not all granted asks for them, through the panel, before anything is read, and opens nothing
//! half-read meanwhile.
//!
//! The panel is replaced by a stand-in that records the folder it was opened on and answers as
//! the test says. Every test runs in either edition's build and checks the half that applies: in
//! the full edition nothing is asked and every open goes ahead as before.
//!
//! ```text
//! cargo test -p brightfield-shell --features managed-edition --test grant_on_open
//! cargo test -p brightfield-shell --test grant_on_open
//! ```

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use brightfield_shell::design::Mode;
use brightfield_shell::edition::{self, Edition};
use brightfield_shell::grants::{self, Grants};
use brightfield_shell::open_queue::{admit, OpenOutcome, OpenRequest};
use brightfield_shell::run::Runner;
use brightfield_shell::sources_browser::{granted_places, places, GRANT_A_FOLDER};
use brightfield_shell::window::{Boot, MeridianApp, NEEDS_GRANT_BANNER};

fn managed() -> bool {
    edition::current() == Edition::Managed
}

/// This file's scratch folder, with the config folder, the Protocols folder and arc's folders in
/// it, so the process's grant store is `config/grants.json` here and nothing is written under the
/// home directory. The Protocols folder is granted, as the first launch will have granted it.
fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-grant-on-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "protocols", "input"] {
            std::fs::create_dir_all(root.join(sub)).expect("a scratch folder");
        }
        let root = std::fs::canonicalize(&root).expect("the scratch folder resolves");
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
        grants::store()
            .grant(&root.join("protocols"))
            .expect("the Protocols folder is granted");
        root
    })
}

/// A new empty folder of this test's, under `input`.
fn folder(name: &str) -> PathBuf {
    let dir = scratch().join("input").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a folder");
    dir
}

/// A Protocol folder `dir`, holding a manifest and, when `source` is given, a record of its
/// sources naming that CSV.
fn protocol_in(dir: &Path, source: Option<&Path>) {
    std::fs::create_dir_all(dir).expect("the Protocol folder");
    std::fs::write(dir.join("arcform.yaml"), "name: granted\n").expect("its manifest");
    if let Some(csv) = source {
        let record = serde_json::json!({
            "dir": dir.to_string_lossy(),
            "tables": [{ "name": "x", "file": csv.to_string_lossy(), "reader": "csv" }],
            "skipped": [],
        });
        std::fs::write(dir.join(".sources.json"), record.to_string()).expect("its sources");
    }
}

/// A copy of the committed California Housing sample, named `name`, in `dir`.
fn housing_in(dir: &Path, name: &str) -> PathBuf {
    let csv = dir.join(name);
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv"),
        &csv,
    )
    .expect("the housing sample copies");
    csv
}

/// The folders the panel's stand-in was opened on, and whether it chooses the folder it was
/// opened on or cancels.
#[derive(Clone, Default)]
struct Panel {
    asked: Arc<Mutex<Vec<PathBuf>>>,
    choose: Arc<Mutex<bool>>,
}

impl Panel {
    fn asked(&self) -> Vec<PathBuf> {
        self.asked.lock().expect("the record").clone()
    }

    fn choose(&self, choose: bool) {
        *self.choose.lock().expect("the answer") = choose;
    }

    fn asker(&self) -> Box<brightfield_shell::ask_grant::Asker> {
        let panel = self.clone();
        Box::new(move |folder: &Path, _why: &str| {
            panel
                .asked
                .lock()
                .expect("the record")
                .push(folder.to_path_buf());
            (*panel.choose.lock().expect("the answer")).then(|| folder.to_path_buf())
        })
    }
}

/// A headless window whose grant asks `panel` answers.
fn app(panel: &Panel) -> MeridianApp {
    scratch();
    MeridianApp::headless(Boot::empty(), Mode::Light)
        .running_with(Some(Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))))
        .asking_grants_with(panel.asker())
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

/// What the window raised, so a failed open's own words reach the assertion.
fn said(app: &MeridianApp) -> String {
    app.notifications()
        .iter()
        .map(|n| format!("{}: {}", n.title, n.body.clone().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("; ")
}

fn needs_grant_raised(app: &MeridianApp) -> bool {
    app.notifications()
        .iter()
        .any(|n| n.id == meridian_egui::NotificationId::new(NEEDS_GRANT_BANNER))
}

/// What [`admit`] answers in the managed edition, and `Opened` in the full one.
fn expected(managed_outcome: OpenOutcome) -> OpenOutcome {
    if managed() {
        managed_outcome
    } else {
        OpenOutcome::Opened
    }
}

#[test]
fn a_lone_manifest_asks_for_its_folder() {
    let dir = folder("lone-manifest").join("quakes");
    protocol_in(&dir, None);
    let manifest = dir.join("arcform.yaml");
    let store = Grants::in_memory();
    store
        .grant(&manifest)
        .expect("the manifest alone is granted");

    assert_eq!(
        admit(std::slice::from_ref(&manifest), &store),
        expected(OpenOutcome::NeedsGrant(vec![dir.clone()])),
        "a grant on arcform.yaml alone does not reach its models"
    );
    store.grant(&dir).expect("its folder is granted");
    assert_eq!(admit(&[manifest], &store), OpenOutcome::Opened);
}

#[test]
fn a_protocol_with_an_ungranted_source_folder_asks_for_it() {
    let root = folder("two-folders");
    let other = root.join("other");
    std::fs::create_dir_all(&other).expect("the other folder");
    let csv = housing_in(&other, "x.csv");
    let dir = root.join("protocol");
    protocol_in(&dir, Some(&csv));

    let store = Grants::in_memory();
    store.grant(&dir).expect("the Protocol's folder is granted");
    assert_eq!(
        admit(std::slice::from_ref(&dir), &store),
        expected(OpenOutcome::NeedsGrant(vec![other.clone()])),
    );
    store.grant(&other).expect("the source's folder is granted");
    assert_eq!(
        admit(std::slice::from_ref(&dir), &store),
        OpenOutcome::Opened
    );

    // In the window: the Protocol's folder is granted, its source's is not, and the analyst
    // cancels the panel. Nothing is drawn.
    grants::store()
        .grant(&dir)
        .expect("the Protocol's folder is granted");
    let panel = Panel::default();
    let mut app = app(&panel);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    app.open_protocol_path(&ctx, &dir.join("arcform.yaml").to_string_lossy());
    frame(&mut app, &ctx);
    if managed() {
        assert_eq!(
            panel.asked(),
            vec![other],
            "the panel opens on the ungranted folder"
        );
        assert!(app.front_door_is_live(), "nothing is drawn: {}", said(&app));
        assert!(
            needs_grant_raised(&app),
            "the banner says why: {}",
            said(&app)
        );
    } else {
        assert!(
            panel.asked().is_empty(),
            "the full edition asks for nothing"
        );
        assert!(!needs_grant_raised(&app));
    }
}

#[test]
fn a_lone_data_file_opens_with_its_own_grant() {
    let dir = folder("lone-file");
    let csv = housing_in(&dir, "granted_housing.csv");
    // What the Finder's double-click hands over: a grant on the file alone.
    grants::store().grant(&csv).expect("the file is granted");
    let before: Vec<_> = std::fs::read_dir(&dir)
        .expect("its folder")
        .flatten()
        .map(|e| e.path())
        .collect();

    let panel = Panel::default();
    let mut app = app(&panel);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    app.open_queue().push(OpenRequest::Path(csv));
    frame(&mut app, &ctx);
    frame(&mut app, &ctx);

    assert!(panel.asked().is_empty(), "nothing is asked: {}", said(&app));
    let made = scratch().join("protocols").join("granted_housing");
    assert_eq!(
        app.own_protocol().map(|p| p.dir.clone()),
        Some(made.clone()),
        "its Protocol is made in the Protocols folder: {}",
        said(&app)
    );
    assert!(made.join("arcform.yaml").is_file());
    let after: Vec<_> = std::fs::read_dir(&dir)
        .expect("its folder")
        .flatten()
        .map(|e| e.path())
        .collect();
    assert_eq!(after, before, "nothing is written beside the file");
}

#[test]
fn recents_without_a_grant_ask_rather_than_open_half_read() {
    // A Protocol made as the app makes one, from a file granted on its own, then moved in the
    // Finder out of the Protocols folder into a folder no grant covers.
    let data = folder("recent-data");
    let csv = housing_in(&data, "recent_housing.csv");
    grants::store().grant(&csv).expect("the file is granted");
    let panel = Panel::default();
    let ctx = egui::Context::default();
    let mut maker = app(&panel);
    frame(&mut maker, &ctx);
    maker.open_queue().push(OpenRequest::Path(csv));
    frame(&mut maker, &ctx);
    let made = scratch().join("protocols").join("recent_housing");
    assert_eq!(
        maker.own_protocol().map(|p| p.dir.clone()),
        Some(made.clone()),
        "{}",
        said(&maker)
    );
    drop(maker);
    let dir = folder("recent").join("recent_housing");
    std::fs::rename(&made, &dir).expect("the Protocol is moved");
    let manifest = dir.join("arcform.yaml").to_string_lossy().into_owned();

    let mut app = app(&panel);
    frame(&mut app, &ctx);
    // A Recents row opens by this route.
    app.open_protocol_path(&ctx, &manifest);
    frame(&mut app, &ctx);
    if !managed() {
        assert!(
            panel.asked().is_empty(),
            "the full edition asks for nothing"
        );
        assert!(!app.front_door_is_live(), "and opens it: {}", said(&app));
        return;
    }
    assert_eq!(
        panel.asked(),
        vec![dir.clone()],
        "the panel opens on its folder"
    );
    assert!(
        app.front_door_is_live(),
        "nothing is opened half-read: {}",
        said(&app)
    );
    assert!(
        needs_grant_raised(&app),
        "the banner says why: {}",
        said(&app)
    );

    // Asked again, the analyst grants it, and the Protocol opens once, with no second ask.
    panel.choose(true);
    app.open_protocol_path(&ctx, &manifest);
    frame(&mut app, &ctx);
    assert_eq!(panel.asked(), vec![dir.clone(), dir.clone()]);
    assert_eq!(
        grants::store().covering(&dir),
        Some(dir.clone()),
        "the grant is kept"
    );
    assert!(!app.front_door_is_live(), "it opens: {}", said(&app));
    assert!(
        !needs_grant_raised(&app),
        "and the banner goes: {}",
        said(&app)
    );
}

#[test]
fn the_location_field_accepts_only_a_granted_folder() {
    let input = folder("location");
    let csv = housing_in(&input, "moving_housing.csv");
    grants::store().grant(&csv).expect("the file is granted");
    let elsewhere = folder("location-elsewhere");

    let panel = Panel::default();
    let mut app = app(&panel);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    app.open_queue().push(OpenRequest::Path(csv));
    frame(&mut app, &ctx);
    let made = scratch().join("protocols").join("moving_housing");
    assert_eq!(
        app.open_protocol_dir(),
        Some(made.clone()),
        "{}",
        said(&app)
    );

    // Typed into the Location field: a folder no grant covers.
    app.relocate_protocol(&ctx, &elsewhere);
    frame(&mut app, &ctx);
    if !managed() {
        assert!(
            panel.asked().is_empty(),
            "the full edition asks for nothing"
        );
        assert!(
            elsewhere.join("moving_housing").is_dir(),
            "and moves it: {}",
            said(&app)
        );
        return;
    }
    assert_eq!(
        panel.asked(),
        vec![elsewhere.clone()],
        "the panel opens on what was typed"
    );
    assert!(made.is_dir(), "cancelled, the Protocol stays where it was");
    assert!(!elsewhere.join("moving_housing").exists());

    panel.choose(true);
    app.relocate_protocol(&ctx, &elsewhere);
    frame(&mut app, &ctx);
    frame(&mut app, &ctx);
    assert!(
        elsewhere.join("moving_housing").is_dir(),
        "granted, the move goes ahead: {}",
        said(&app)
    );
}

#[test]
fn the_sources_browser_lists_granted_places_and_downloads() {
    let a = folder("place-a");
    let b = folder("place-b");
    let downloads = folder("Downloads");
    let listed = granted_places(&[a.clone(), b.clone(), a.clone()], &downloads);
    let labels: Vec<&str> = listed.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, ["place-a", "place-b", "Downloads", GRANT_A_FOLDER]);
    assert!(listed.last().is_some_and(|p| p.is_grant_entry()));

    grants::store().grant(&a).expect("a folder is granted");
    let offered = places();
    let has_grant_entry = offered.iter().any(|p| p.is_grant_entry());
    let has_a = offered.iter().any(|p| p.path == a);
    if managed() {
        assert!(
            has_grant_entry && has_a,
            "the granted folder and Grant a folder… are offered"
        );
        assert!(
            !offered.iter().any(|p| p.label == "Home"),
            "the home folder, not granted, is not a place"
        );
    } else {
        assert!(!has_grant_entry, "the full edition offers its usual places");
        assert!(offered.iter().any(|p| p.label == "Home"));
    }
}

#[test]
fn a_database_that_needs_its_folder_asks_for_it() {
    let dir = folder("database");
    let db = dir.join("shop.duckdb");
    std::fs::write(&db, b"").expect("a database file");
    let refused = format!(
        "IO Error: Cannot open file \"{}.wal\": Operation not permitted",
        db.display()
    );
    let asked = brightfield_shell::ask_grant::database_needs_folder(&db, &refused);
    assert_eq!(asked, managed().then(|| dir.clone()));
    assert_eq!(
        brightfield_shell::ask_grant::database_needs_folder(&db, "Catalog Error: no such table"),
        None,
        "a failure of another kind asks for nothing"
    );
}

#[test]
fn a_protocol_and_its_source_in_two_ungranted_folders_are_asked_for_in_turn() {
    // A Protocol made as the app makes one, then moved in the Finder into a folder no grant
    // covers, with the record of its sources pointed at a copy of its source in a second folder
    // no grant covers.
    let data = folder("two-asks-data");
    let csv = housing_in(&data, "two_asks_housing.csv");
    grants::store().grant(&csv).expect("the file is granted");
    let panel = Panel::default();
    let ctx = egui::Context::default();
    let mut maker = app(&panel);
    frame(&mut maker, &ctx);
    maker.open_queue().push(OpenRequest::Path(csv.clone()));
    frame(&mut maker, &ctx);
    let made = scratch().join("protocols").join("two_asks_housing");
    assert_eq!(
        maker.own_protocol().map(|p| p.dir.clone()),
        Some(made.clone()),
        "{}",
        said(&maker)
    );
    drop(maker);
    let dir = folder("two-asks").join("two_asks_housing");
    std::fs::rename(&made, &dir).expect("the Protocol is moved");
    let sources = folder("two-asks-sources");
    let copy = housing_in(&sources, "two_asks_housing.csv");
    let record = dir.join(".sources.json");
    let text = std::fs::read_to_string(&record).expect("its record of sources");
    std::fs::write(
        &record,
        text.replace(&*csv.to_string_lossy(), &copy.to_string_lossy()),
    )
    .expect("the record names the copy");

    panel.choose(true);
    let mut app = app(&panel);
    frame(&mut app, &ctx);
    app.open_protocol_path(&ctx, &dir.join("arcform.yaml").to_string_lossy());
    frame(&mut app, &ctx);
    frame(&mut app, &ctx);
    if !managed() {
        assert!(
            panel.asked().is_empty(),
            "the full edition asks for nothing"
        );
        return;
    }
    assert_eq!(
        panel.asked(),
        vec![dir, sources],
        "the Protocol's folder, then the source's folder found once the first was granted"
    );
    assert!(
        !app.front_door_is_live(),
        "one gesture opens it: {}",
        said(&app)
    );
    assert!(
        !needs_grant_raised(&app),
        "and the banner goes: {}",
        said(&app)
    );
}
