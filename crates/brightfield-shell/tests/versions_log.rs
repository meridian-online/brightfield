//! An agent-written Protocol's panel versions, listed from its version log.
//!
//! The CO2 fixture's `arcform-log.txt` is committed with the folder, and the
//! snapshots its lines name are not: they stayed in the store of the machine
//! the agent ran on. A list read from a fresh store still shows the agent's
//! version, made through arc's command line, and marks it as not on this
//! machine. A Save in Brightfield then joins the log under its own word.

#[path = "support/agent_protocol.rs"]
mod agent_protocol;

use agent_protocol::ran_fixture;
use arc::spec::{read_log, LOG_FILENAME};
use brightfield_protocol::chart_history::Who;
use brightfield_protocol::HistoryStore;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::versions::{
    Listing, Source, Versions, NOT_ON_THIS_MACHINE, NOT_ON_THIS_MACHINE_REASON,
};

const CO2: &str = "co2-per-capita";
const SINCE: &str = "panels/co2_per_capita_since_1990.yaml";

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

    let mut doc = ChartDoc::empty();
    let not_recorded = doc
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
    assert!(newest.on_this_machine);
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
