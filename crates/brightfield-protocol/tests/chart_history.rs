//! A chart file's versions joined with its Protocol's version log: who made
//! each version, and whether its text is on this machine.
//!
//! The log (`arcform-log.txt`) travels with the Protocol's folder and the
//! store's snapshots do not, so the list is the union of the two. The store is
//! pointed at a temporary folder, so no test reads or writes the home
//! directory.

use std::fs;
use std::path::{Path, PathBuf};

use arc::spec::{LOG_FILENAME, MANIFEST_FILENAME};
use brightfield_protocol::chart_history::{HistoryKind, Who};
use brightfield_protocol::{ChartVersions, HistoryStore, VersionChange};

/// A Protocol folder with a `panels/` folder, and a store beside it, removed
/// when it drops.
struct Protocol {
    root: PathBuf,
    dir: PathBuf,
    store: HistoryStore,
}

impl Protocol {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("bf-chart-history-{}-{seq}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dir = root.join("protocol");
        fs::create_dir_all(dir.join("panels")).expect("the panels folder");
        fs::write(dir.join(MANIFEST_FILENAME), "name: p\nsteps: []\n").expect("the manifest");
        let store = HistoryStore::At(root.join("store"));
        Self {
            dir: fs::canonicalize(&dir).expect("the folder canonicalises"),
            root,
            store,
        }
    }

    fn panel(&self) -> PathBuf {
        self.dir.join("panels").join("chart.yaml")
    }

    /// One Save, as the window makes it.
    fn save(&self, text: &str) {
        let file = self.panel();
        let replaced = fs::read_to_string(&file).ok();
        let versions = ChartVersions::begin(&self.store, &file, replaced.as_deref());
        fs::write(&file, text).expect("write chart");
        versions.finish(text).expect("version recorded");
    }

    fn log(&self) -> PathBuf {
        self.dir.join(LOG_FILENAME)
    }
}

impl Drop for Protocol {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn append(log: &Path, line: &str) {
    let mut text = fs::read_to_string(log).unwrap_or_default();
    text.push_str(line);
    text.push('\n');
    fs::write(log, text).expect("the log writes");
}

#[test]
fn a_store_entry_missing_from_the_log_is_listed() {
    let protocol = Protocol::new();
    protocol.save("plot: []\n");
    // An older arc kept the snapshot and wrote no log.
    fs::remove_file(protocol.log()).expect("the save wrote a log line");

    let list = protocol
        .store
        .versions(&protocol.panel())
        .expect("the list");
    assert_eq!(list.versions.len(), 1, "{:#?}", list.versions);
    let version = &list.versions[0];
    assert_eq!(
        version.who,
        Who::Brightfield,
        "who is read from the store's way"
    );
    assert!(version.on_this_machine);
}

#[test]
fn a_version_only_the_log_names_is_listed_as_not_on_this_machine() {
    let protocol = Protocol::new();
    append(
        &protocol.log(),
        r#"{"at":"2026-10-16T21:15:21.622Z","file":"panels/chart.yaml","kind":"save","interface":"mcp","version":"1792185321622-000-save"}"#,
    );
    append(
        &protocol.log(),
        r#"{"at":"2026-10-16T21:15:22.000Z","file":"panels/other.yaml","kind":"save","interface":"mcp","version":"1792185322000-000-save"}"#,
    );

    let list = protocol
        .store
        .versions(&protocol.panel())
        .expect("the list");
    assert_eq!(
        list.versions.len(),
        1,
        "only this panel's line: {:#?}",
        list.versions
    );
    let version = &list.versions[0];
    assert_eq!(version.id, "1792185321622-000-save");
    assert_eq!(version.who, Who::ArcMcp);
    assert_eq!(version.kind, HistoryKind::Save);
    assert!(!version.on_this_machine);
    assert_eq!(version.change, VersionChange::NoPredecessor);
    assert_eq!(
        version
            .at
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after the epoch")
            .as_millis(),
        1_792_185_321_622
    );
}

#[test]
fn a_save_here_is_listed_once_newest_first_after_the_logs_older_version() {
    let protocol = Protocol::new();
    append(
        &protocol.log(),
        r#"{"at":"2026-10-16T21:15:21.622Z","file":"panels/chart.yaml","kind":"save","interface":"terminal","version":"1792185321622-000-save"}"#,
    );
    protocol.save("plot: []\n");

    let list = protocol
        .store
        .versions(&protocol.panel())
        .expect("the list");
    let who: Vec<(&Who, bool)> = list
        .versions
        .iter()
        .map(|v| (&v.who, v.on_this_machine))
        .collect();
    assert_eq!(
        who,
        [(&Who::Brightfield, true), (&Who::ArcCommandLine, false)],
        "{:#?}",
        list.versions
    );
}

#[test]
fn the_interface_word_names_who() {
    assert_eq!(Who::from_interface(Some("brightfield")), Who::Brightfield);
    assert_eq!(Who::from_interface(Some("terminal")), Who::ArcCommandLine);
    assert_eq!(Who::from_interface(Some("mcp")), Who::ArcMcp);
    assert_eq!(
        Who::from_interface(Some("notebook")),
        Who::Other("notebook".to_string())
    );
    assert_eq!(Who::from_interface(None), Who::Unknown);
}
