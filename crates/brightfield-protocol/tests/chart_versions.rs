//! A chart file's versions listed newest first, with what changed in each, and
//! one read back as text: `HistoryStore::versions` and `read_version`.
//!
//! The store is pointed at a temporary folder outside the chart file's own, so
//! no test reads or writes the home directory.

use std::fs;
use std::path::{Path, PathBuf};

use brightfield_protocol::chart_history::{HistoryKind, HistoryWay, LocalHistory};
use brightfield_protocol::{ChartChange, ChartVersions, HistoryStore, VersionChange, VersionList};
use brightfield_spec::{parse_spec, Format};

/// A scratch folder holding a chart file's own folder and, beside it, the
/// store's.
struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("bf-versions-{}-{test}-{seq}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("charts")).expect("charts folder");
        Self(dir)
    }

    /// The chart file, which need not exist.
    fn file(&self) -> PathBuf {
        self.0.join("charts").join("chart.yaml")
    }

    fn root(&self) -> PathBuf {
        self.0.join("store")
    }

    fn store(&self) -> HistoryStore {
        HistoryStore::At(self.root())
    }

    /// One Save, as the window makes it: begin before the write, write, finish.
    fn save(&self, text: &str) {
        let file = self.file();
        let replaced = fs::read_to_string(&file).ok();
        let versions = ChartVersions::begin(&self.store(), &file, replaced.as_deref());
        fs::write(&file, text).expect("write chart");
        versions.finish(text).expect("version recorded");
    }

    fn list(&self) -> VersionList {
        self.store().versions(&self.file()).expect("versions list")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A chart of two plots in a column, with a note above the file, above each
/// plot, and optionally a log scale on the second plot's y.
struct Chart<'a> {
    /// The column on the second plot's `x`.
    x: &'a str,
    /// The second plot's `yScale`, when it carries one.
    y_scale: Option<&'a str>,
    notes: [&'a str; 3],
}

impl Default for Chart<'static> {
    fn default() -> Self {
        Self {
            x: "income",
            y_scale: None,
            notes: ["a note", "the first plot", "the second plot"],
        }
    }
}

impl Chart<'_> {
    fn text(&self) -> String {
        let y_scale = self
            .y_scale
            .map(|scale| format!("    yScale: {scale}\n"))
            .unwrap_or_default();
        format!(
            "# {}\n\
             meta:\n  title: \"versions\"\n\
             data:\n  opened:\n    file: 'data.csv'\n\
             vconcat:\n\
             \x20 # {}\n\
             \x20 - plot:\n\
             \x20   - mark: dot\n\
             \x20     data: {{ from: opened }}\n\
             \x20     x: 'longitude'\n\
             \x20     y: 'latitude'\n\
             \x20   width: 400\n\
             \x20 # {}\n\
             \x20 - plot:\n\
             \x20   - mark: dot\n\
             \x20     data: {{ from: opened }}\n\
             \x20     x: '{}'\n\
             \x20     y: 'value'\n\
             \x20   width: 400\n\
             {y_scale}",
            self.notes[0], self.notes[1], self.notes[2], self.x
        )
    }
}

const SECOND_PLOT: &str = "root/vconcat[1]";

/// The changes of the newest version of `list`, which has a version before it.
fn newest_changes(list: &VersionList) -> Vec<ChartChange> {
    match &list.versions[0].change {
        VersionChange::Changes(changes) => changes.clone(),
        VersionChange::NoPredecessor => panic!("the newest version has a version before it"),
    }
}

#[test]
fn two_saves_list_two_versions_newest_first_and_each_reads_back_as_the_text_saved() {
    let scratch = Scratch::new("two_saves");
    let first = Chart::default().text();
    let second = Chart {
        x: "age",
        ..Chart::default()
    }
    .text();
    scratch.save(&first);
    scratch.save(&second);

    let list = scratch.list();
    assert_eq!(list.kept(), 2, "two Saves keep two versions");
    let store = scratch.store();
    let read = |i: usize| {
        store
            .read_version(&scratch.file(), &list.versions[i].id)
            .expect("a listed version reads back")
    };
    assert_eq!(read(0), second, "the newest version is the second Save's");
    assert_eq!(read(1), first, "the version after it is the first Save's");

    // The time is the one the store recorded, not one the listing took.
    let recorded = store
        .open()
        .expect("store opens")
        .entries_for_file(&scratch.file())
        .expect("entries");
    assert_eq!(list.versions[0].id, recorded[1].id);
    assert_eq!(list.versions[0].at, recorded[1].at);
    assert_eq!(list.versions[1].at, recorded[0].at);
    assert!(list.versions[0].at >= list.versions[1].at);
}

#[test]
fn a_version_names_its_kind_and_the_way_it_was_reached() {
    let scratch = Scratch::new("kind_and_way");
    let file = scratch.file();
    let named = |way: HistoryWay| LocalHistory::at_root(scratch.root()).reached_by(way);
    // An entry that names no way: a handle that was given none.
    let unnamed = LocalHistory::at_root(scratch.root());

    let brightfield = scratch.store().open().expect("store opens");
    brightfield
        .record_save_for_file(&file, "a: 1\n")
        .expect("1");
    unnamed
        .record_checkpoint_for_file(&file, "a: 2\n")
        .expect("2");
    named(HistoryWay::TERMINAL)
        .record_save_for_file(&file, "a: 3\n")
        .expect("3");
    named(HistoryWay::MCP)
        .record_checkpoint_for_file(&file, "a: 4\n")
        .expect("4");

    let seen: Vec<(HistoryKind, Option<String>)> = scratch
        .list()
        .versions
        .iter()
        .map(|v| (v.kind, v.way.clone()))
        .collect();
    assert_eq!(
        seen,
        vec![
            (HistoryKind::Checkpoint, Some("mcp".to_string())),
            (HistoryKind::Save, Some("terminal".to_string())),
            (HistoryKind::Checkpoint, None),
            (HistoryKind::Save, Some("brightfield".to_string())),
        ],
        "newest first, each with the kind and way the store recorded"
    );
}

#[test]
fn a_column_moved_on_a_plots_x_names_the_plot_the_channel_and_both_columns() {
    let scratch = Scratch::new("channel");
    scratch.save(&Chart::default().text());
    scratch.save(
        &Chart {
            x: "age",
            ..Chart::default()
        }
        .text(),
    );

    assert_eq!(
        newest_changes(&scratch.list()),
        vec![ChartChange::Channel {
            plot: SECOND_PLOT.to_string(),
            mark: 0,
            channel: "x".to_string(),
            before: Some("income".to_string()),
            after: Some("age".to_string()),
        }]
    );
}

#[test]
fn a_plot_attribute_added_is_named_with_its_value_before_absent() {
    let scratch = Scratch::new("attribute");
    scratch.save(&Chart::default().text());
    scratch.save(
        &Chart {
            y_scale: Some("log"),
            ..Chart::default()
        }
        .text(),
    );

    assert_eq!(
        newest_changes(&scratch.list()),
        vec![ChartChange::PlotAttribute {
            plot: SECOND_PLOT.to_string(),
            key: "yScale".to_string(),
            before: None,
            after: Some("log".to_string()),
        }]
    );
}

#[test]
fn a_version_that_differs_by_two_changes_names_both() {
    let scratch = Scratch::new("two_changes");
    scratch.save(&Chart::default().text());
    scratch.save(
        &Chart {
            x: "age",
            y_scale: Some("log"),
            ..Chart::default()
        }
        .text(),
    );

    assert_eq!(
        newest_changes(&scratch.list()),
        vec![
            ChartChange::PlotAttribute {
                plot: SECOND_PLOT.to_string(),
                key: "yScale".to_string(),
                before: None,
                after: Some("log".to_string()),
            },
            ChartChange::Channel {
                plot: SECOND_PLOT.to_string(),
                mark: 0,
                channel: "x".to_string(),
                before: Some("income".to_string()),
                after: Some("age".to_string()),
            },
        ],
        "the plot's attributes, then its marks' channels"
    );
}

#[test]
fn three_comment_lines_changed_in_place_are_an_edit_outside_the_channels_with_a_count_of_three() {
    let scratch = Scratch::new("three_comments");
    scratch.save(&Chart::default().text());
    scratch.save(
        &Chart {
            notes: [
                "a changed note",
                "the first, changed",
                "the second, changed",
            ],
            ..Chart::default()
        }
        .text(),
    );

    assert_eq!(
        newest_changes(&scratch.list()),
        vec![ChartChange::EditedOutsideChannels { lines: 3 }]
    );
}

#[test]
fn a_block_of_three_lines_changed_in_place_counts_three_and_not_six() {
    let scratch = Scratch::new("a_block");
    let before = "meta:\n  title: \"versions\"\n# one\n# two\n# three\n";
    let after = "meta:\n  title: \"versions\"\n# 1\n# 2\n# 3\n";
    scratch.save(before);
    scratch.save(after);

    assert_eq!(
        newest_changes(&scratch.list()),
        vec![ChartChange::EditedOutsideChannels { lines: 3 }]
    );
}

#[test]
fn a_line_added_above_a_long_chart_counts_one_and_not_every_line_below_it() {
    let scratch = Scratch::new("a_line_above");
    let chart = Chart::default().text();
    scratch.save(&chart);
    scratch.save(&format!("# a note above everything\n{chart}"));

    assert_eq!(
        newest_changes(&scratch.list()),
        vec![ChartChange::EditedOutsideChannels { lines: 1 }]
    );
}

#[test]
fn a_text_that_does_not_parse_as_a_chart_reports_its_count_of_lines_from_either_side() {
    let scratch = Scratch::new("unparseable");
    let chart = Chart::default().text();
    let broken = format!("{chart}this: is: not: yaml: [\n");
    assert!(
        parse_spec(&broken, Format::Yaml).is_err(),
        "the broken text is one the parser refuses"
    );
    assert!(parse_spec(&chart, Format::Yaml).is_ok());
    scratch.save(&chart);
    scratch.save(&broken);
    scratch.save(&chart);

    let list = scratch.list();
    let outside = VersionChange::Changes(vec![ChartChange::EditedOutsideChannels { lines: 1 }]);
    assert_eq!(
        list.versions[1].change, outside,
        "the version that does not parse"
    );
    assert_eq!(
        list.versions[0].change, outside,
        "the version whose predecessor does not parse"
    );
}

#[test]
fn a_text_too_long_to_compare_line_by_line_counts_its_changed_middle_as_one_run() {
    let scratch = Scratch::new("too_long");
    let body = |first: &str, last: &str| {
        let mut lines = vec![first.to_string()];
        lines.extend((1..2099).map(|i| format!("# line {i}")));
        lines.push(last.to_string());
        lines.join("\n") + "\n"
    };
    let history = scratch.store().open().expect("store opens");
    history
        .record_checkpoint_for_file(&scratch.file(), &body("# first", "# last"))
        .expect("before");
    history
        .record_checkpoint_for_file(&scratch.file(), &body("# 1st", "# 2100th"))
        .expect("after");

    assert_eq!(
        newest_changes(&scratch.list()),
        vec![ChartChange::EditedOutsideChannels { lines: 2100 }],
        "past the comparison's bound the lines between the first and last that \
         differ count as one run"
    );
}

#[test]
fn the_oldest_version_kept_has_no_version_before_it_to_compare_with() {
    let scratch = Scratch::new("oldest");
    scratch.save(&Chart::default().text());
    let alone = scratch.list();
    assert_eq!(alone.versions[0].change, VersionChange::NoPredecessor);

    scratch.save(
        &Chart {
            x: "age",
            ..Chart::default()
        }
        .text(),
    );
    let list = scratch.list();
    assert_eq!(list.versions[1].change, VersionChange::NoPredecessor);
    assert_ne!(
        list.versions[0].change,
        VersionChange::NoPredecessor,
        "a version with one before it is compared with it"
    );
}

#[test]
fn the_list_reports_how_many_are_kept_the_stores_bound_and_the_stores_folder() {
    let scratch = Scratch::new("bound");
    let history = scratch.store().open().expect("store opens");
    let empty = scratch.list();
    assert_eq!(empty.kept(), 0);
    assert_eq!(empty.folder, scratch.root());

    // The bound is what the store does: recording past it drops the oldest.
    let bound = empty.bound;
    for i in 0..bound + 3 {
        history
            .record_checkpoint_for_file(&scratch.file(), &format!("# version {i}\n"))
            .expect("record");
    }
    let full = scratch.list();
    assert_eq!(full.bound, bound);
    assert_eq!(
        full.kept(),
        bound,
        "recording {} versions keeps the store's bound of them",
        bound + 3
    );
    assert_eq!(full.versions.len(), full.kept());
    assert_eq!(full.folder, scratch.root());
}

#[test]
fn a_file_with_no_recorded_version_lists_none_and_returns_no_error() {
    let scratch = Scratch::new("none_recorded");
    // The chart file is not there, and its folder is.
    let list = scratch
        .store()
        .versions(&scratch.file())
        .expect("no version is not an error");
    assert!(list.versions.is_empty());
    assert_eq!(list.kept(), 0);
}

#[test]
fn a_store_that_cannot_be_opened_returns_the_reason_opening_it_gives() {
    let scratch = Scratch::new("unopenable");
    // A root that is a file cannot be made a folder.
    let root = scratch.0.join("not-a-folder");
    fs::write(&root, "a file").expect("file");
    let store = HistoryStore::At(root);
    let reason = store
        .open()
        .expect_err("a file cannot be the store's folder")
        .reason()
        .to_string();
    assert!(!reason.is_empty());

    let listed = store.versions(&scratch.file()).expect_err("cannot list");
    assert_eq!(listed.reason(), reason);
    let read = store
        .read_version(&scratch.file(), "any-id")
        .expect_err("cannot read");
    assert_eq!(read.reason(), reason);
}

#[test]
fn an_id_the_file_has_no_version_under_is_refused_with_a_reason() {
    let scratch = Scratch::new("unknown_id");
    scratch.save(&Chart::default().text());
    let refused = scratch
        .store()
        .read_version(&scratch.file(), "no-such-version")
        .expect_err("no version has that id");
    assert!(!refused.reason().is_empty());
}

/// Every entry in `dir` with its bytes, so a file added, removed or rewritten
/// shows.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut found: Vec<(String, Vec<u8>)> = fs::read_dir(dir)
        .expect("read folder")
        .map(|entry| {
            let entry = entry.expect("entry");
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).unwrap_or_default(),
            )
        })
        .collect();
    found.sort();
    found
}

#[test]
fn a_listing_writes_no_file_under_the_chart_files_folder() {
    let scratch = Scratch::new("no_write");
    scratch.save(&Chart::default().text());
    scratch.save(
        &Chart {
            x: "age",
            ..Chart::default()
        }
        .text(),
    );
    let folder = scratch.file().parent().expect("folder").to_path_buf();
    let before = snapshot(&folder);

    let list = scratch.list();
    for version in &list.versions {
        scratch
            .store()
            .read_version(&scratch.file(), &version.id)
            .expect("reads");
    }
    assert_eq!(snapshot(&folder), before);
    assert_eq!(
        before
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        vec!["chart.yaml"],
        "the folder holds the chart file and nothing else"
    );
}

// ------------------------------------------------------------ a close without saving

/// The unsaved chart is recorded as arcform's unsaved kind, the newest version,
/// and reads back as the text given; the chart file keeps its bytes.
#[test]
fn an_unsaved_chart_is_the_newest_version_of_the_unsaved_kind_and_the_file_keeps_its_bytes() {
    let scratch = Scratch::new("unsaved_kept");
    scratch.save("a: 1\n");
    let on_disk = fs::read(scratch.file()).expect("the chart file");

    brightfield_protocol::record_unsaved(&scratch.store(), &scratch.file(), "a: 2\n")
        .expect("the unsaved chart is recorded");

    assert_eq!(fs::read(scratch.file()).expect("the chart file"), on_disk);
    let list = scratch.list();
    assert_eq!(
        list.versions.iter().map(|v| v.kind).collect::<Vec<_>>(),
        [HistoryKind::Unsaved, HistoryKind::Save]
    );
    assert_eq!(
        scratch
            .store()
            .read_version(&scratch.file(), &list.versions[0].id)
            .expect("reads back"),
        "a: 2\n"
    );
}

/// Before a chart file's folder is there, the folder is made empty to key the
/// version by, and the chart file is not made.
#[test]
fn an_unsaved_chart_with_no_folder_yet_makes_the_folder_and_no_file() {
    let scratch = Scratch::new("unsaved_unmade");
    let file = scratch.0.join("panels").join("chart.yaml");

    brightfield_protocol::record_unsaved(&scratch.store(), &file, "a: 1\n")
        .expect("the unsaved chart is recorded");

    assert!(file.parent().expect("a folder").is_dir());
    assert!(!file.exists(), "the chart file was made");
    let list = scratch.store().versions(&file).expect("versions list");
    assert_eq!(list.versions.len(), 1);
    assert_eq!(list.versions[0].kind, HistoryKind::Unsaved);
}

/// A record that fails takes away the folder it made, so the data folder is
/// left as it was.
#[cfg(unix)]
#[test]
fn an_unsaved_chart_that_cannot_be_recorded_takes_away_the_folder_it_made() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new("unsaved_refused");
    let root = scratch.root();
    fs::create_dir_all(&root).expect("the store's root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o555)).expect("read-only root");
    if fs::create_dir(root.join("probe")).is_ok() {
        eprintln!("this process ignores directory permissions; the record cannot be refused");
        return;
    }
    let file = scratch.0.join("panels").join("chart.yaml");

    let refused = brightfield_protocol::record_unsaved(&scratch.store(), &file, "a: 1\n");

    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).expect("writable again");
    assert!(refused.is_err(), "a read-only store took the version");
    assert!(
        !file.parent().expect("a folder").exists(),
        "the folder made to key the version by was left behind"
    );
}
