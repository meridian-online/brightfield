//! The Mac's own ways in reach the window's one open queue: a file dropped on the front door, a
//! request the operating system delivered before the window drew its first frame, and Cmd+O. The
//! bundle's `Info.plist` declares the documents and the link scheme Launch Services hands over.
//! A drop onto an open Protocol adds what it holds to that Protocol as sources, or opens the
//! Protocol it holds after the close question.
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
use brightfield_shell::overlays::CloseAnswer;
use brightfield_shell::run::Runner;
use brightfield_shell::sources_browser::Pane;
use brightfield_shell::window::{
    Boot, Leaving, MeridianApp, ADD_SOURCE_REFUSED_BANNER, DROP_REST_NOT_ADDED_BANNER,
    DROP_REST_NOT_ADDED_TITLE, MENU_QUIT_UNROUTED_BANNER, MENU_QUIT_UNROUTED_TITLE,
};

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

/// What dropping `paths` together onto the window hands it.
fn dropped(paths: &[PathBuf]) -> egui::RawInput {
    egui::RawInput {
        dropped_files: paths
            .iter()
            .map(|p| egui::DroppedFile {
                path: Some(p.clone()),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

/// The names of the tables the model pane's Protocol reads, in order.
fn model_tables(app: &MeridianApp) -> Vec<String> {
    app.model_pane()
        .unwrap_or_else(|| panic!("the model is open: {}", said(app)))
        .protocol
        .sources
        .tables
        .iter()
        .map(|t| t.name.clone())
        .collect()
}

/// `files` of the retail extract that dovetail joins, a visit's customer and store, copied into
/// `dir`.
fn retail_copy(dir: &Path, files: &[&str]) -> PathBuf {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../brightfield-ossie/fixtures/retail_extract");
    std::fs::create_dir_all(dir).expect("a folder for the retail tables");
    for file in files {
        std::fs::copy(fixture.join(file), dir.join(file)).expect("the fixture copies");
    }
    dir.to_path_buf()
}

/// A copy of the committed Protocol Brightfield did not make, named `name`, in `folder`.
fn shipped_copy(folder: &str, name: &str) -> PathBuf {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/shipped-protocol");
    let dir = scratch().join("input").join(folder).join(name);
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
    dir
}

#[test]
fn a_csv_dropped_on_an_open_protocol_is_added_as_a_source() {
    let csv = housing_copy("dropped_on_open", "dropped_on_open.csv");
    let other = housing_copy("dropped_on_open", "dropped_after.csv");
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());
    app.open_queue().push(OpenRequest::Path(csv));
    frame(&mut app, &ctx, egui::RawInput::default());
    assert_opened(&app, "dropped_on_open");

    frame(&mut app, &ctx, dropped(&[other]));
    frame(&mut app, &ctx, egui::RawInput::default());

    let dir = scratch().join("protocols").join("dropped_on_open");
    assert_eq!(
        model_tables(&app),
        ["dropped_on_open", "dropped_after"],
        "a second source shows the model"
    );
    assert_eq!(
        app.open_protocol_dir(),
        Some(dir.clone()),
        "added to the Protocol that was open"
    );
    assert!(
        !scratch().join("protocols").join("dropped_after").exists(),
        "no Protocol is made of the drop"
    );
    let manifest = std::fs::read_to_string(dir.join("arcform.yaml")).expect("the manifest reads");
    assert!(
        manifest.contains("dropped_after"),
        "the source is kept at once: {manifest}"
    );
    assert!(!app.carries_unsaved_mark(), "with nothing left to save");
    assert_eq!(app.open_overlay(), None, "a file opens no browser");
}

#[test]
fn a_folder_with_a_join_dropped_on_an_open_protocol_opens_the_browser_on_it() {
    let csv = housing_copy("dropped_joined", "dropped_joined.csv");
    let folder = retail_copy(
        &scratch()
            .join("input")
            .join("dropped_joined")
            .join("retail"),
        &["customers.csv", "loyalty_visits.csv", "stores.csv"],
    );
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());
    app.open_queue().push(OpenRequest::Path(csv));
    frame(&mut app, &ctx, egui::RawInput::default());
    assert_opened(&app, "dropped_joined");

    frame(&mut app, &ctx, dropped(std::slice::from_ref(&folder)));
    frame(&mut app, &ctx, egui::RawInput::default());

    assert_eq!(
        app.open_overlay(),
        Some("sources-browser"),
        "the browser opens on the dropped folder: {}",
        said(&app)
    );
    assert_eq!(
        app.open_protocol_dir(),
        Some(scratch().join("protocols").join("dropped_joined")),
        "the folder is added to the Protocol that was open"
    );
    assert_eq!(
        model_tables(&app),
        ["dropped_joined", "customers", "loyalty_visits", "stores"],
        "the folder's tables are added at once, before its joins are decided"
    );
    let b = &app
        .sources_browser()
        .unwrap_or_else(|| panic!("the browser is open: {}", said(&app)))
        .browser;
    assert_eq!(b.canvas_folder(), Some(folder.as_path()));
    assert_eq!(b.focus, Pane::Canvas, "its canvas holds the keys");
    let inside = ["customers", "loyalty_visits", "stores"];
    assert!(
        !b.canvas_joins().is_empty(),
        "dovetail's joins are on its canvas"
    );
    assert!(
        b.canvas_joins().iter().all(|(join, accepted)| {
            !accepted
                && inside.contains(&join.left.0.as_str())
                && inside.contains(&join.right.0.as_str())
        }),
        "each a join between the folder's tables, none accepted yet: {:?}",
        b.canvas_joins()
    );
}

#[test]
fn a_folder_of_one_table_dropped_on_an_open_protocol_is_added_at_once() {
    // The Protocol holds the customers a visit names, so dovetail proposes a join from the dropped
    // folder's one table to the Protocol's: a join the folder makes with the Protocol, not one in
    // the folder, and no reason to open the browser.
    let place = scratch().join("input").join("dropped_one_table");
    let customers = retail_copy(&place.join("members"), &["customers.csv"]).join("customers.csv");
    let visits = retail_copy(&place.join("visits"), &["loyalty_visits.csv"]);
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());
    app.open_queue().push(OpenRequest::Path(customers));
    frame(&mut app, &ctx, egui::RawInput::default());
    let dir = app
        .own_protocol()
        .map(|p| p.dir.clone())
        .unwrap_or_else(|| panic!("the customers open as a Protocol: {}", said(&app)));

    frame(&mut app, &ctx, dropped(&[visits]));
    frame(&mut app, &ctx, egui::RawInput::default());

    assert_eq!(
        app.open_overlay(),
        None,
        "a folder of one table opens no browser: {}",
        said(&app)
    );
    assert_eq!(model_tables(&app), ["customers", "loyalty_visits"]);
    assert_eq!(
        app.open_protocol_dir(),
        Some(dir),
        "added to the open Protocol"
    );
    let pane = app.model_pane().expect("the model is open");
    assert!(
        pane.model.relationships.iter().any(|r| {
            let ends = [r.from.as_str(), r.to.as_str()];
            ends.contains(&"customers") && ends.contains(&"loyalty_visits")
        }),
        "dovetail proposes the visit's customer, a join to the Protocol's table: {:?}",
        pane.model.relationships
    );
}

#[test]
fn a_protocol_dropped_on_an_open_protocol_opens_it_after_the_question() {
    let first = housing_copy("dropped_protocol", "dropped_protocol.csv");
    let target = housing_copy("drop_target", "drop_target.csv");
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());
    app.open_queue().push(OpenRequest::Path(first));
    frame(&mut app, &ctx, egui::RawInput::default());
    assert_opened(&app, "dropped_protocol");
    let protocol = scratch().join("protocols").join("dropped_protocol");
    // Another Protocol, open over it, with an unsaved order on its grid.
    app.open_queue().push(OpenRequest::Path(target));
    frame(&mut app, &ctx, egui::RawInput::default());
    assert_opened(&app, "drop_target");
    order_the_grid(&mut app);

    frame(&mut app, &ctx, dropped(std::slice::from_ref(&protocol)));
    frame(&mut app, &ctx, egui::RawInput::default());

    // Nothing replaces the unsaved edits unasked, and the Protocol is not taken as a source.
    assert_opened(&app, "drop_target");
    assert!(
        app.chart_doc().has_unsaved_work(),
        "the edits are still there"
    );
    assert_eq!(
        app.leaving(),
        Some(&Leaving::Open(OpenRequest::Path(protocol.clone()))),
        "the dropped Protocol opens once the question is answered: {}",
        said(&app)
    );
    answer_and_frame_until(
        &mut app,
        &ctx,
        CloseAnswer::SaveAndClose,
        "the dropped Protocol never opened",
        |app| app.open_protocol_dir() == Some(protocol.clone()),
    );
    let over = scratch().join("protocols").join("drop_target");
    let saved =
        std::fs::read_to_string(over.join("arcform.yaml")).expect("the target's manifest reads");
    assert!(
        saved.contains("median_income"),
        "the order was saved before the open: {saved}"
    );
    assert!(
        !saved.contains("dropped_protocol"),
        "the dropped Protocol was not added to the one it was dropped on: {saved}"
    );
}

#[test]
fn a_protocol_dropped_with_files_on_an_open_protocol_opens_alone_and_adds_nothing() {
    let first = housing_copy("dropped_with", "dropped_with.csv");
    let target = housing_copy("dropped_with_target", "with_target.csv");
    let beside = housing_copy("dropped_with", "dropped_beside.csv");
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());
    app.open_queue().push(OpenRequest::Path(first));
    frame(&mut app, &ctx, egui::RawInput::default());
    assert_opened(&app, "dropped_with");
    let protocol = scratch().join("protocols").join("dropped_with");
    app.open_queue().push(OpenRequest::Path(target));
    frame(&mut app, &ctx, egui::RawInput::default());
    assert_opened(&app, "with_target");

    frame(&mut app, &ctx, dropped(&[beside, protocol.clone()]));
    frame(&mut app, &ctx, egui::RawInput::default());

    assert_eq!(
        app.open_protocol_dir(),
        Some(protocol),
        "the Protocol in the drop opens: {}",
        said(&app)
    );
    let target = std::fs::read_to_string(
        scratch()
            .join("protocols")
            .join("with_target")
            .join("arcform.yaml"),
    )
    .expect("the target's manifest reads");
    assert!(
        !target.contains("dropped_beside"),
        "the file dropped beside the Protocol was not added: {target}"
    );
    assert!(
        !scratch().join("protocols").join("dropped_beside").exists(),
        "and nothing was made of it"
    );
    let banner = app
        .notifications()
        .iter()
        .find(|n| n.id == meridian_egui::NotificationId::new(DROP_REST_NOT_ADDED_BANNER))
        .unwrap_or_else(|| panic!("a banner says the file was not added: {}", said(&app)));
    assert_eq!(banner.title, DROP_REST_NOT_ADDED_TITLE);
    assert!(
        banner
            .body
            .as_deref()
            .is_some_and(|b| b.contains("dropped_with is a Protocol")
                && b.contains("The other 1 was not added; drop it by itself to add it.")),
        "naming the Protocol and what was not added: {:?}",
        banner.body
    );
}

#[test]
fn a_file_dropped_on_a_protocol_brightfield_did_not_make_is_refused_without_asking() {
    let shipped = shipped_copy("dropped_refused", "shipped_refused");
    let csv = housing_copy("dropped_refused", "refused_houses.csv");
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());
    app.open_queue().push(OpenRequest::Path(shipped.clone()));
    frame(&mut app, &ctx, egui::RawInput::default());
    assert_eq!(
        app.panel_tiles().map(|t| t.dir().to_path_buf()),
        Some(shipped.clone()),
        "the Protocol opens where it is: {}",
        said(&app)
    );
    order_the_grid(&mut app);
    assert!(app.carries_unsaved_mark(), "with a draft pending");
    let held = std::fs::read_to_string(shipped.join("arcform.yaml")).expect("the manifest");

    frame(&mut app, &ctx, dropped(&[csv]));
    frame(&mut app, &ctx, egui::RawInput::default());

    // The add cannot happen, so the draft is not asked about for it.
    assert_eq!(app.leaving(), None, "nothing waits on a question");
    assert_eq!(app.open_overlay(), None, "no close question is up");
    assert!(
        app.notifications()
            .iter()
            .any(|n| n.id == meridian_egui::NotificationId::new(ADD_SOURCE_REFUSED_BANNER)),
        "a banner says sources are added only to Protocols Brightfield made: {}",
        said(&app)
    );
    assert!(app.carries_unsaved_mark(), "the draft is still there");
    assert_eq!(
        std::fs::read_to_string(shipped.join("arcform.yaml")).expect("the manifest"),
        held,
        "and the Protocol is as it was"
    );
}

#[test]
fn a_data_file_in_a_protocols_folder_is_added_and_its_manifest_opens_it() {
    let shipped = shipped_copy("dropped_inside", "shipped_inside");
    let target = housing_copy("dropped_inside", "inside_target.csv");
    let inside = shipped.join("inside_houses.csv");
    std::fs::copy(&target, &inside).expect("a data file in the Protocol's folder");
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());
    app.open_queue().push(OpenRequest::Path(target));
    frame(&mut app, &ctx, egui::RawInput::default());
    assert_opened(&app, "inside_target");

    // A data file is data wherever it sits: the drop adds it.
    frame(&mut app, &ctx, dropped(&[inside]));
    frame(&mut app, &ctx, egui::RawInput::default());
    assert_eq!(
        model_tables(&app),
        ["inside_target", "inside_houses"],
        "the file is added to the open Protocol: {}",
        said(&app)
    );
    assert!(
        app.panel_tiles().is_none(),
        "the Protocol it sits in is not opened"
    );

    // The manifest names the Protocol itself: the drop opens it.
    frame(&mut app, &ctx, dropped(&[shipped.join("arcform.yaml")]));
    frame(&mut app, &ctx, egui::RawInput::default());
    assert_eq!(
        app.panel_tiles().map(|t| t.dir().to_path_buf()),
        Some(shipped),
        "the Protocol opens where it is: {}",
        said(&app)
    );
}

#[test]
fn the_menu_quit_banner_says_quit_will_not_ask() {
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());
    let shown = |app: &MeridianApp| {
        app.notifications()
            .iter()
            .any(|n| n.id == meridian_egui::NotificationId::new(MENU_QUIT_UNROUTED_BANNER))
    };
    app.say_menu_quit_unrouted(true);
    frame(&mut app, &ctx, egui::RawInput::default());
    assert!(shown(&app), "the banner is up: {}", said(&app));
    assert!(app
        .notifications()
        .iter()
        .any(|n| n.title == MENU_QUIT_UNROUTED_TITLE));
    app.say_menu_quit_unrouted(false);
    frame(&mut app, &ctx, egui::RawInput::default());
    assert!(!shown(&app), "routed late, it comes down");
}

#[test]
fn an_archive_among_several_files_says_nothing_opened() {
    use std::io::Write as _;
    let dir = scratch().join("input").join("archive_among");
    std::fs::create_dir_all(&dir).expect("a folder");
    let archive = dir.join("bundle.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).expect("the zip"));
    zip.start_file(
        "bundle/arcform.yaml",
        zip::write::SimpleFileOptions::default(),
    )
    .expect("an entry");
    zip.write_all(b"name: bundle\nengine: duckdb\nsteps: []\n")
        .expect("the manifest");
    zip.finish().expect("the zip is written");
    let csv = housing_copy("archive_among", "beside_the_archive.csv");

    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());
    app.open_queue()
        .push(OpenRequest::Paths(vec![csv, archive]));
    frame(&mut app, &ctx, egui::RawInput::default());

    let titles: Vec<String> = app
        .notifications()
        .iter()
        .map(|n| n.title.clone())
        .collect();
    assert!(
        !titles.iter().any(|t| t == "Only the Protocol was opened"),
        "nothing is said to have opened: {titles:?}"
    );
    let refusal = app
        .notifications()
        .iter()
        .find(|n| n.title == "Could not open that data file")
        .unwrap_or_else(|| panic!("the archive is refused: {}", said(&app)));
    assert!(
        refusal
            .body
            .as_deref()
            .is_some_and(|b| b.contains("The other 1 was not opened")),
        "and says the other was not opened: {:?}",
        refusal.body
    );
    assert!(app.front_door_is_live(), "nothing opened");
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
/// fires whether the runner names the key Command or Control.
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

    // Brightfield is offered for the types other apps own, and is the default only for the two no
    // app on a Mac owns.
    let ranks: Vec<(&str, &str)> = info
        .get("CFBundleDocumentTypes")
        .and_then(plist::Value::as_array)
        .expect("CFBundleDocumentTypes is an array")
        .iter()
        .filter_map(|t| {
            let t = t.as_dictionary()?;
            let rank = t.get("LSHandlerRank")?.as_string()?;
            let types = t.get("LSItemContentTypes")?.as_array()?;
            Some(
                types
                    .iter()
                    .filter_map(plist::Value::as_string)
                    .map(move |uti| (uti, rank))
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect();
    for (uti, rank) in [
        ("public.comma-separated-values-text", "Alternate"),
        ("public.tab-separated-values-text", "Alternate"),
        ("org.openxmlformats.spreadsheetml.sheet", "Alternate"),
        ("public.database", "Alternate"),
        ("public.folder", "Alternate"),
        ("org.apache.parquet", "Default"),
        ("org.duckdb.database", "Default"),
    ] {
        assert!(
            ranks.contains(&(uti, rank)),
            "{uti} is declared at {rank}: {ranks:?}"
        );
    }
    let defaults: Vec<&str> = ranks
        .iter()
        .filter(|(_, rank)| *rank == "Default")
        .map(|(uti, _)| *uti)
        .collect();
    assert_eq!(
        defaults,
        ["org.apache.parquet", "org.duckdb.database"],
        "only Parquet and DuckDB take the double-click"
    );

    // The three types macOS does not know are imported, each with the extension that names it.
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
        ("org.sqlite.sqlite3", "sqlite"),
    ] {
        assert!(
            imported
                .iter()
                .any(|(id, exts)| *id == uti && exts.contains(&extension)),
            "{uti} is imported with .{extension}: {imported:?}"
        );
    }
}

/// Rows of the housing sample ordered by a column, applied to the grid and saved nowhere: an edit
/// the window holds and its files do not.
fn order_the_grid(app: &mut MeridianApp) {
    app.chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: None,
            order: Some("median_income".into()),
        });
    assert!(app.chart_doc().has_unsaved_work(), "the order is unsaved");
}

#[test]
fn an_open_over_unsaved_edits_asks_and_goes_once_they_are_saved() {
    let first = housing_copy("held_first", "held_first.csv");
    let second = housing_copy("held_second", "held_second.csv");
    let mut app = app();
    let ctx = egui::Context::default();
    app.open_queue().push(OpenRequest::Path(first));
    frame(&mut app, &ctx, egui::RawInput::default());
    assert_opened(&app, "held_first");
    order_the_grid(&mut app);

    app.open_queue().push(OpenRequest::Path(second.clone()));
    frame(&mut app, &ctx, egui::RawInput::default());
    frame(&mut app, &ctx, egui::RawInput::default());

    // Nothing replaces the unsaved edits unasked: the close question is up, and the open waits
    // for its answer.
    assert_opened(&app, "held_first");
    assert!(
        app.chart_doc().has_unsaved_work(),
        "the edits are still there"
    );
    assert_eq!(app.open_overlay(), Some("close-question"), "{}", said(&app));
    assert_eq!(
        app.leaving(),
        Some(&Leaving::Open(OpenRequest::Path(second)))
    );
    assert!(
        !scratch().join("protocols").join("held_second").exists(),
        "nothing was made of the open before the answer"
    );

    answer_and_frame_until(
        &mut app,
        &ctx,
        CloseAnswer::SaveAndClose,
        "the open never went ahead after the save",
        |app| {
            app.own_protocol().map(|p| p.dir.clone())
                == Some(scratch().join("protocols").join("held_second"))
        },
    );
    assert_opened(&app, "held_second");
    assert!(app.leaving().is_none());
    let saved = std::fs::read_to_string(
        scratch()
            .join("protocols")
            .join("held_first")
            .join("arcform.yaml"),
    )
    .expect("the first Protocol's manifest reads");
    assert!(
        saved.contains("median_income"),
        "the first Protocol's order was saved before the open: {saved}"
    );
}

/// Two copies of the housing sample, dropped together.
fn two_dropped(folder: &str) -> egui::RawInput {
    let files = ["north_houses.csv", "south_houses.csv"]
        .into_iter()
        .map(|name| egui::DroppedFile {
            path: Some(housing_copy(folder, name)),
            ..Default::default()
        })
        .collect();
    egui::RawInput {
        dropped_files: files,
        ..Default::default()
    }
}

#[test]
fn two_files_dropped_together_make_one_protocol_of_both() {
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());
    frame(&mut app, &ctx, two_dropped("dropped_two"));
    frame(&mut app, &ctx, egui::RawInput::default());

    let pane = app
        .model_pane()
        .unwrap_or_else(|| panic!("two tables open the model: {}", said(&app)));
    let tables: Vec<&str> = pane
        .protocol
        .sources
        .tables
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(tables, ["north_houses", "south_houses"]);
    assert_eq!(
        pane.protocol.dir,
        scratch().join("protocols").join("north_houses"),
        "one Protocol, named after the first"
    );
    assert!(
        !scratch().join("protocols").join("south_houses").exists(),
        "no Protocol is made of the second on its own"
    );
}

#[test]
fn files_handed_over_together_by_the_mac_make_one_protocol() {
    // What an open-documents event of two files pushes, as `mac_open`'s tests show.
    let paths = vec![
        housing_copy("event_two", "east_houses.csv"),
        housing_copy("event_two", "west_houses.csv"),
    ];
    let mut app = app();
    app.open_queue().push(OpenRequest::Paths(paths));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());

    let pane = app
        .model_pane()
        .unwrap_or_else(|| panic!("two tables open the model: {}", said(&app)));
    assert_eq!(pane.protocol.sources.tables.len(), 2);
    assert_eq!(
        pane.protocol.dir,
        scratch().join("protocols").join("east_houses")
    );
}

#[test]
fn a_protocol_handed_over_with_files_opens_alone_and_says_so() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/shipped-protocol");
    let dir = scratch()
        .join("input")
        .join("batch_protocol")
        .join("shipped_batch");
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
    let csv = housing_copy("batch_protocol", "beside_houses.csv");
    // A file inside the Protocol's folder routes to that Protocol.
    let inside = dir.join("inside_houses.csv");
    std::fs::copy(&csv, &inside).unwrap();
    let mut app = app();
    app.open_queue().push(OpenRequest::Paths(vec![csv, inside]));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());
    frame(&mut app, &ctx, egui::RawInput::default());

    assert_eq!(
        app.panel_tiles().map(|t| t.dir().to_path_buf()),
        Some(dir),
        "the Protocol opens where it is: {}",
        said(&app)
    );
    assert!(
        !scratch().join("protocols").join("beside_houses").exists(),
        "nothing is made of the file beside it"
    );
    assert!(
        app.notifications()
            .iter()
            .any(|n| n.body.as_deref().is_some_and(|b| {
                b.contains(
                "inside_houses.csv belongs to the Protocol shipped_batch, which opened on its own; \
                 the other 1 was not opened"
            )
            })),
        "a banner names the Protocol the file belongs to and says the rest were not opened: {}",
        said(&app)
    );
}

/// Answer the close question with `answer`, then run frames until `done` holds of the window, or
/// fail saying `what`.
fn answer_and_frame_until(
    app: &mut MeridianApp,
    ctx: &egui::Context,
    answer: CloseAnswer,
    what: &str,
    done: impl Fn(&MeridianApp) -> bool,
) {
    assert_eq!(app.open_overlay(), Some("close-question"), "{}", said(app));
    app.answer_close_question(ctx, answer);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !done(app) {
        assert!(
            std::time::Instant::now() < deadline,
            "{what}: {}",
            said(app)
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        frame(app, ctx, egui::RawInput::default());
    }
}

/// A window holding the one-source Protocol made of `name`, with an unsaved order on its grid.
fn window_with_unsaved_edits(name: &str) -> (MeridianApp, egui::Context) {
    let csv = housing_copy(name, &format!("{name}.csv"));
    let mut app = app();
    let ctx = egui::Context::default();
    app.open_queue().push(OpenRequest::Path(csv));
    frame(&mut app, &ctx, egui::RawInput::default());
    assert_opened(&app, name);
    order_the_grid(&mut app);
    (app, ctx)
}

#[test]
fn a_newer_open_waits_behind_the_one_asked_about() {
    let (mut app, ctx) = window_with_unsaved_edits("held_over");
    let older = housing_copy("held_over_a", "held_older.csv");
    let newer = housing_copy("held_over_b", "held_newer.csv");

    app.open_queue().push(OpenRequest::Path(older.clone()));
    frame(&mut app, &ctx, egui::RawInput::default());
    app.open_queue().push(OpenRequest::Path(newer));
    frame(&mut app, &ctx, egui::RawInput::default());
    assert_eq!(
        app.leaving(),
        Some(&Leaving::Open(OpenRequest::Path(older))),
        "the open asked about stands"
    );

    let newer_dir = scratch().join("protocols").join("held_newer");
    answer_and_frame_until(
        &mut app,
        &ctx,
        CloseAnswer::SaveAndClose,
        "the held open never went ahead",
        |app| app.own_protocol().map(|p| p.dir.clone()) == Some(newer_dir.clone()),
    );
    for _ in 0..5 {
        frame(&mut app, &ctx, egui::RawInput::default());
    }
    assert_opened(&app, "held_newer");
    assert!(
        scratch()
            .join("protocols")
            .join("held_older")
            .join("arcform.yaml")
            .is_file(),
        "the open asked about went first"
    );
}

#[test]
fn keeping_editing_cancels_the_open() {
    let (mut app, ctx) = window_with_unsaved_edits("held_cancel");
    let cancelled = housing_copy("held_cancel_a", "held_cancelled.csv");
    app.open_queue().push(OpenRequest::Path(cancelled));
    frame(&mut app, &ctx, egui::RawInput::default());
    assert!(app.leaving().is_some(), "{}", said(&app));

    answer_and_frame_until(
        &mut app,
        &ctx,
        CloseAnswer::KeepEditing,
        "the question never went down",
        |app| app.open_overlay().is_none(),
    );
    assert!(app.leaving().is_none(), "keeping editing cancels the open");

    app.save_protocol(&ctx)
        .expect("there is a Protocol to save")
        .expect("the save is written");
    for _ in 0..10 {
        frame(&mut app, &ctx, egui::RawInput::default());
    }
    assert_opened(&app, "held_cancel");
    assert!(
        !scratch().join("protocols").join("held_cancelled").exists(),
        "nothing is made of a cancelled open"
    );
}

#[test]
fn a_file_brightfield_does_not_read_among_several_is_skipped() {
    let names = ["mixed_one.csv", "mixed_two.csv", "mixed_three.csv"];
    let mut paths: Vec<PathBuf> = names.iter().map(|n| housing_copy("mixed", n)).collect();
    let notes = scratch().join("input").join("mixed").join("notes.txt");
    std::fs::write(&notes, "not a table\n").unwrap();
    paths.insert(1, notes);
    let mut app = app();
    app.open_queue().push(OpenRequest::Paths(paths));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());

    let pane = app
        .model_pane()
        .unwrap_or_else(|| panic!("three tables open the model: {}", said(&app)));
    assert_eq!(pane.protocol.sources.tables.len(), 3);
    assert_eq!(
        pane.protocol.dir,
        scratch().join("protocols").join("mixed_one")
    );
    let skipped = pane.skipped_line().unwrap_or_default();
    assert!(skipped.contains("notes.txt ("), "{skipped:?}");
    assert!(
        !skipped.contains("/input/mixed/"),
        "the reason reads without the path the list already names: {skipped:?}"
    );
}

#[test]
fn a_file_skipped_beside_one_table_is_said() {
    let csv = housing_copy("one_and_notes", "lone_sales.csv");
    let notes = csv.with_file_name("notes.txt");
    std::fs::write(&notes, "not a table\n").unwrap();
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());
    let dropped = egui::RawInput {
        dropped_files: [csv, notes]
            .into_iter()
            .map(|path| egui::DroppedFile {
                path: Some(path),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    frame(&mut app, &ctx, dropped);
    frame(&mut app, &ctx, egui::RawInput::default());

    assert_opened(&app, "lone_sales");
    let skipped = &app.own_protocol().expect("a Protocol").sources.skipped;
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    assert!(
        !skipped[0].1.contains("notes.txt"),
        "the recorded reason does not repeat the path: {:?}",
        skipped[0].1
    );
    assert!(
        app.notifications().iter().any(|n| n
            .body
            .as_deref()
            .is_some_and(|b| b.starts_with("1 file skipped: notes.txt ("))),
        "a banner says the file beside the table was not opened: {}",
        said(&app)
    );
}

#[test]
fn a_batch_brightfield_reads_none_of_names_each_file() {
    let dir = scratch().join("input").join("none_read");
    std::fs::create_dir_all(&dir).unwrap();
    let paths: Vec<PathBuf> = ["minutes.txt", "photo.jpg"]
        .into_iter()
        .map(|name| {
            let path = dir.join(name);
            std::fs::write(&path, "x\n").unwrap();
            path
        })
        .collect();
    let mut app = app();
    app.open_queue().push(OpenRequest::Paths(paths));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, egui::RawInput::default());

    assert!(app.front_door_is_live(), "nothing opened");
    let words = said(&app);
    assert!(
        words.contains("Brightfield opens none of these files. 2 files skipped: minutes.txt (")
            && words.contains("photo.jpg ("),
        "the refusal names each file: {words}"
    );
}
