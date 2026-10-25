//! An agent-written Protocol's panel versions, listed from its version log.
//!
//! The CO2 fixture's `arcform-log.txt` is committed with the folder, and the
//! snapshots its lines name are not: they stayed in the store of the machine
//! the agent ran on. A list read from a fresh store still shows the agent's
//! version, made through arc's command line, and marks it as not on this
//! machine. A Save in Brightfield then joins the log under its own word.

#[path = "support/agent_protocol.rs"]
mod agent_protocol;

use std::path::Path;

use agent_protocol::ran_fixture;
use arc::spec::{read_log, LOG_FILENAME};
use brightfield_protocol::chart_history::{HistoryKind, Who};
use brightfield_protocol::layout::Flow;
use brightfield_protocol::{HistoryStore, VersionChange};
use brightfield_shell::design::Mode;
use brightfield_shell::shelf::{RowEdit, SettingValue, TITLE_ROW};
use brightfield_shell::startup::default_layout;
use brightfield_shell::versions::{
    Listing, Source, Versions, NOT_ON_THIS_MACHINE, NOT_ON_THIS_MACHINE_REASON,
};
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::channel::ShelfChannel;

const CO2: &str = "co2-per-capita";
const SINCE: &str = "panels/co2_per_capita_since_1990.yaml";

/// Run three frames of a device-free window.
fn settle(app: &mut MeridianApp, ctx: &egui::Context) {
    for _ in 0..3 {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 820.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| app.draw(ui));
    }
}

/// A store of the test's own, holding nothing yet.
fn empty_store() -> (tempfile::TempDir, HistoryStore) {
    let tmp = tempfile::Builder::new()
        .prefix("bf-versions-log-")
        .tempdir()
        .expect("a temp store");
    let store = HistoryStore::At(tmp.path().join("history"));
    (tmp, store)
}

#[test]
fn the_agents_versions_show_as_arc() {
    let fixture = ran_fixture(CO2);
    let (_tmp, store) = empty_store();

    let list = store
        .versions(&fixture.dir.join(SINCE))
        .expect("the versions list");
    assert_eq!(list.versions.len(), 1, "{:#?}", list.versions);
    let version = &list.versions[0];
    assert_eq!(version.id, "1792185321622-000-save");
    assert_eq!(version.who, Who::ArcCommandLine);
    assert!(!version.on_this_machine);
}

#[test]
fn a_save_in_brightfield_joins_the_log() {
    let fixture = ran_fixture(CO2);
    let (_tmp, store) = empty_store();
    let panel = fixture.dir.join(SINCE);
    let lines_before = read_log(&fixture.dir).expect("the log reads").len();

    // The panel open from its tile, with its x title edited, as an analyst
    // makes a change before saving.
    let mut app = MeridianApp::headless_with_layout(
        Boot::open(
            &fixture.dir.join("arcform.yaml").to_string_lossy(),
            Flow::Vertical,
            None,
        )
        .expect("the Protocol opens"),
        default_layout(),
        Mode::Light,
    );
    let ctx = egui::Context::default();
    settle(&mut app, &ctx);
    let at = app
        .panel_tiles()
        .and_then(|tiles| tiles.panels())
        .expect("the Protocol opened as tiles")
        .tiles
        .iter()
        .position(|t| t.file == Path::new(SINCE))
        .expect("a tile for the panel");
    assert!(app.open_panel_with_grid(at), "the tile opens as the chart");
    settle(&mut app, &ctx);
    let title = RowEdit {
        channel: ShelfChannel::X,
        row: TITLE_ROW,
        value: SettingValue::Text("year".to_owned()),
    };
    assert!(
        app.chart_doc_mut().set_axis_row(0, &title),
        "the edit holds"
    );
    let not_recorded = app
        .chart_doc_mut()
        .save_chart_at(&panel, Some(&store))
        .expect("the panel saves");
    assert_eq!(not_recorded, None, "the Save recorded its version");

    let log = read_log(&fixture.dir).expect("the log reads");
    assert!(
        log.len() > lines_before,
        "the Save added a line to {LOG_FILENAME}"
    );
    let added = log.last().expect("a line");
    assert_eq!(added.interface.as_deref(), Some("brightfield"));
    assert_eq!(added.file, SINCE);

    let list = store.versions(&panel).expect("the versions list");
    let newest = &list.versions[0];
    assert_eq!(newest.who, Who::Brightfield, "{:#?}", list.versions);
    assert_eq!(
        newest.kind,
        HistoryKind::Save,
        "a Save, not the text it replaced"
    );
    assert!(newest.on_this_machine);
    assert!(
        matches!(&newest.change, VersionChange::Changes(changes) if !changes.is_empty()),
        "the Save names its edit: {:?}",
        newest.change
    );
    assert_eq!(newest.id, added.version);
    let agents = list.versions.last().expect("the agent's version");
    assert_eq!(agents.who, Who::ArcCommandLine);
}

#[test]
fn the_panel_marks_a_version_not_on_this_machine_and_will_not_read_it() {
    let fixture = ran_fixture(CO2);
    let (_tmp, store) = empty_store();
    let mut versions = Versions::default();
    versions.sync(
        Some(Source {
            store,
            file: fixture.dir.join(SINCE),
            dir: fixture.dir.clone(),
        }),
        true,
        &str::to_string,
        &[],
        None,
        Vec::new,
    );

    let Listing::Listed { rows, .. } = versions.listing() else {
        panic!("the panel lists nothing: {:?}", versions.listing());
    };
    assert_eq!(rows.len(), 1, "{rows:#?}");
    let row = &rows[0];
    assert_eq!(row.who, "arc (command line)");
    assert!(!row.on_this_machine);
    assert_eq!(row.changed, NOT_ON_THIS_MACHINE);
    assert_eq!(
        versions.read_version(&row.id),
        Err(NOT_ON_THIS_MACHINE_REASON.to_string()),
        "a version whose text is elsewhere is not drawn or stepped back to"
    );
}
