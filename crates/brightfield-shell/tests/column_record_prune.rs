//! The first flat open of a process prunes the column records of lone files that have gone.
//!
//! One test in a binary of its own, since the prune runs once per process: a record planted after
//! another test's open would never meet it.

use brightfield_shell::column_record::{self, SourceRecord, SourceStamp};
use brightfield_shell::data_file::{self, OpenOptions};

#[test]
fn the_first_flat_open_prunes_a_gone_files_record() {
    let root = tempfile::tempdir().unwrap();
    std::env::set_var(
        brightfield_shell::startup::CONFIG_DIR_VAR,
        root.path().join("config"),
    );
    let datasets = brightfield_shell::starts::datasets_dir();
    assert!(datasets.starts_with(root.path()), "{}", datasets.display());

    let input = root.path().join("input");
    std::fs::create_dir_all(&input).unwrap();
    let opened = input.join("opened.csv");
    std::fs::write(&opened, "region,units\nNorth,3\nSouth,5\n").unwrap();
    let stale = datasets.join("records").join("stale");
    let record = SourceRecord {
        source: Some(SourceStamp {
            size: 1,
            modified: 1,
            path: Some(input.join("deleted.csv")),
        }),
        ..SourceRecord::default()
    };
    column_record::write_record(&stale.join("datapackage.json"), "source", &record).unwrap();
    assert!(stale.exists());

    let options = OpenOptions {
        type_source: None,
        ..OpenOptions::default()
    };
    data_file::open_traced(&opened.to_string_lossy(), &options).expect("the file opens");
    assert!(!stale.exists(), "the gone file's record was pruned on open");
}
