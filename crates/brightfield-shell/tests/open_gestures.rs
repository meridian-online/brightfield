//! The Mac's own ways in reach the window's one open queue: a file dropped on the front door, a
//! request the operating system delivered before the window drew its first frame, and Cmd+O. The
//! bundle's `Info.plist` declares the documents and the link scheme Launch Services hands over.
//!
//! The Apple Events themselves cannot be sent to a headless test, so what is proved here is the
//! queue's side of them: whatever arrives before the first frame is opened on it. Every window is
//! headless, with the `brightfield-shell` binary cargo built as its runner, and the config
//! directory, the Protocols folder and arc's directories point at a scratch folder of this file's,
//! so nothing is written under the home directory.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use brightfield_shell::design::Mode;
use brightfield_shell::open_queue::OpenRequest;
use brightfield_shell::run::Runner;
use brightfield_shell::window::{Boot, MeridianApp};

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-open-gestures-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "protocols", "input"] {
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

/// A copy of the committed California Housing sample, named `name`, in a folder of its own.
fn housing_copy(folder: &str, name: &str) -> PathBuf {
    let dir = scratch().join("input").join(folder);
    std::fs::create_dir_all(&dir).expect("an input folder");
    let csv = dir.join(name);
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv"),
        &csv,
    )
    .expect("the housing sample copies");
    csv
}

/// A headless window, before it has drawn anything.
fn app() -> MeridianApp {
    scratch();
    MeridianApp::headless(Boot::empty(), Mode::Light)
        .running_with(Some(Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))))
}

/// One frame of `app`, with `raw`'s files dropped and events delivered.
fn frame(app: &mut MeridianApp, ctx: &egui::Context, raw: egui::RawInput) {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1440.0, 900.0),
        )),
        ..raw
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

/// Assert `app` holds the one-source Protocol named `name`, made in the Protocols folder.
fn assert_opened(app: &MeridianApp, name: &str) {
    let dir = scratch().join("protocols").join(name);
    assert!(
        dir.join("arcform.yaml").is_file(),
        "the Protocol is made in the Protocols folder: {}",
        said(app)
    );
    assert_eq!(
        app.own_protocol().map(|p| p.dir.clone()),
        Some(dir),
        "the window holds the Protocol it made: {}",
        said(app)
    );
    assert!(!app.front_door_is_live(), "the open replaced the door");
    assert!(app.chart_doc().is_live(), "the dashboard is drawn");
}

#[test]
fn a_file_dropped_on_the_door_opens_as_a_protocol() {
    let csv = housing_copy("dropped", "dropped_housing.csv");
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());
    assert!(app.front_door_is_live(), "the window starts on the door");

    let dropped = egui::RawInput {
        dropped_files: vec![egui::DroppedFile {
            path: Some(csv),
            ..Default::default()
        }],
        ..Default::default()
    };
    frame(&mut app, &ctx, dropped);
    frame(&mut app, &ctx, egui::RawInput::default());

    assert_opened(&app, "dropped_housing");
}

#[test]
fn requests_queued_before_the_first_frame_are_opened() {
    let csv = housing_copy("early", "early_housing.csv");
    let mut app = app();
    // As an Apple Event delivered while the app launches does: pushed before any frame, so no
    // context exists yet to ask for one.
    app.open_queue().push(OpenRequest::Path(csv));

    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());

    assert_opened(&app, "early_housing");
}

/// One frame's worth of Cmd+O. `command` is what `consume_key`'s logical match reads, so this
/// fires the same on every runner.
fn cmd_o() -> egui::RawInput {
    let modifiers = egui::Modifiers {
        command: true,
        ..Default::default()
    };
    egui::RawInput {
        modifiers,
        events: [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key: egui::Key::O,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn cmd_o_raises_the_picker() {
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());
    assert!(
        !app.pick_requested(),
        "nothing has asked for the picker yet"
    );

    frame(&mut app, &ctx, cmd_o());
    assert!(app.pick_requested(), "Cmd+O on the door raises the picker");

    frame(&mut app, &ctx, egui::RawInput::default());
    assert!(
        !app.pick_requested(),
        "the ask lasts the frame it was made in"
    );

    // Away from the door too: with a Protocol open, Cmd+O still raises the picker.
    let csv = housing_copy("cmd_o", "cmd_o_housing.csv");
    app.open_queue().push(OpenRequest::Path(csv));
    frame(&mut app, &ctx, egui::RawInput::default());
    frame(&mut app, &ctx, egui::RawInput::default());
    assert!(!app.front_door_is_live(), "{}", said(&app));
    frame(&mut app, &ctx, cmd_o());
    assert!(
        app.pick_requested(),
        "Cmd+O over an open Protocol raises the picker"
    );
}

#[test]
fn info_plist_declares_the_documents_and_the_scheme() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packaging/Info.plist");
    let plist = plist::Value::from_file(&path).expect("Info.plist parses");
    let info = plist.as_dictionary().expect("Info.plist is a dictionary");

    let schemes: Vec<&str> = info
        .get("CFBundleURLTypes")
        .and_then(plist::Value::as_array)
        .expect("CFBundleURLTypes is an array")
        .iter()
        .filter_map(|t| t.as_dictionary()?.get("CFBundleURLSchemes")?.as_array())
        .flatten()
        .filter_map(plist::Value::as_string)
        .collect();
    assert!(schemes.contains(&"brightfield"), "{schemes:?}");

    let types: Vec<&str> = info
        .get("CFBundleDocumentTypes")
        .and_then(plist::Value::as_array)
        .expect("CFBundleDocumentTypes is an array")
        .iter()
        .filter_map(|t| t.as_dictionary()?.get("LSItemContentTypes")?.as_array())
        .flatten()
        .filter_map(plist::Value::as_string)
        .collect();
    for uti in [
        "public.comma-separated-values-text",
        "public.tab-separated-values-text",
        "org.apache.parquet",
        "org.openxmlformats.spreadsheetml.sheet",
        "org.duckdb.database",
        "public.database",
        "public.folder",
    ] {
        assert!(types.contains(&uti), "{uti} is not declared: {types:?}");
    }
    assert!(
        !types.contains(&"public.zip-archive"),
        "every zip on the Mac is not Brightfield's to open"
    );

    // The two types macOS does not know are imported, each with the extension that names it.
    let imported: Vec<(&str, Vec<&str>)> = info
        .get("UTImportedTypeDeclarations")
        .and_then(plist::Value::as_array)
        .expect("UTImportedTypeDeclarations is an array")
        .iter()
        .filter_map(|t| {
            let t = t.as_dictionary()?;
            let id = t.get("UTTypeIdentifier")?.as_string()?;
            let extensions = t
                .get("UTTypeTagSpecification")?
                .as_dictionary()?
                .get("public.filename-extension")?
                .as_array()?
                .iter()
                .filter_map(plist::Value::as_string)
                .collect();
            Some((id, extensions))
        })
        .collect();
    for (uti, extension) in [
        ("org.apache.parquet", "parquet"),
        ("org.duckdb.database", "duckdb"),
    ] {
        assert!(
            imported
                .iter()
                .any(|(id, exts)| *id == uti && exts.contains(&extension)),
            "{uti} is imported with .{extension}: {imported:?}"
        );
    }
}
