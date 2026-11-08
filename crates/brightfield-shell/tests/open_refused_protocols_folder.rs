//! A Protocols folder the system will not let Brightfield write in: opening a CSV says where to
//! allow it, opens nothing, and writes nothing beside the CSV in its place.
//!
//! A file of its own because the Protocols folder is read from the environment, which every test
//! in one binary shares: making it read-only here would refuse the other files' opens too.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use brightfield_shell::design::Mode;
use brightfield_shell::open_queue::OpenRequest;
use brightfield_shell::run::Runner;
use brightfield_shell::window::{Boot, MeridianApp};

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-open-refused-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "locked", "input"] {
            std::fs::create_dir_all(root.join(sub)).expect("a scratch folder");
        }
        std::env::set_var(
            brightfield_shell::startup::CONFIG_DIR_VAR,
            root.join("config"),
        );
        // A Protocols folder inside a folder the system refuses writes in, as when macOS has not
        // been given access to Documents.
        std::env::set_var(
            brightfield_shell::protocols_dir::PROTOCOLS_DIR_VAR,
            root.join("locked").join("Protocols"),
        );
        std::env::set_var("ARCFORM_HISTORY_DIR", root.join("history"));
        std::env::set_var("ARCFORM_DB_DIR", root.join("db"));
        root
    })
}

#[test]
fn a_refused_protocols_folder_says_where_to_allow_it_and_opens_nothing() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = scratch();
    let locked = root.join("locked");
    let folder = root.join("input");
    let csv = folder.join("refused.csv");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv"),
        &csv,
    )
    .expect("the housing sample copies");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555))
        .expect("the folder made read-only");

    let mut app = MeridianApp::headless(Boot::empty(), Mode::Light)
        .running_with(Some(Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))));
    let ctx = egui::Context::default();
    let frame = |app: &mut MeridianApp| {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440.0, 900.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| app.draw(ui));
    };
    frame(&mut app);
    app.open_queue().push(OpenRequest::Path(csv.clone()));
    frame(&mut app);
    frame(&mut app);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))
        .expect("the folder made writable");

    let said = app
        .notifications()
        .iter()
        .map(|n| format!("{}: {}", n.title, n.body.clone().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("; ");
    assert!(
        said.contains("Privacy & Security") && said.contains("Files and Folders"),
        "the refusal names where to allow it: {said}"
    );
    assert!(app.front_door_is_live(), "nothing opened");
    assert!(app.own_protocol().is_none());
    let beside: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(beside, ["refused.csv"], "nothing is written beside the CSV");
}
