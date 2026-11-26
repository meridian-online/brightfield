//! `read-file` in the window: a file is brought in through the sources
//! browser, so the operation's command opens the browser and begins no draft.
//! A `read-file` step an agent records through arc still opens and previews.
//!
//! Each window is over a copy of a fixture in a folder of the test's own, and
//! arc is the `brightfield-shell` binary cargo built for the suite.

#[path = "support/operations.rs"]
mod operations;

use std::path::Path;

use brightfield_shell::data_file;
use brightfield_shell::operation::{self, StepRequest};
use brightfield_shell::operation_ui::{self, Start};
use operations::{runner, OpWindow};

const SAMPLE: &str = "california_housing_sample.csv";
const WITH_NULLS: &str = "with_nulls.csv";

/// Save the sample's Protocol into `dir`, so its load is written, and record
/// `name` as arc's `read-file` of `path`.
fn read_file_step(dir: &Path, name: &str, path: &str) {
    let opened = data_file::open(&dir.join(SAMPLE).to_string_lossy()).expect("the sample opens");
    opened.protocol.save_to(dir).expect("the Protocol saves");
    let request = StepRequest {
        long_name: "read-file".to_string(),
        on: None,
        reads: Vec::new(),
        args: vec![("path".to_string(), path.to_string())],
    };
    operation::record_step(&runner(), dir, name, &request)
        .unwrap_or_else(|e| panic!("arc records {name}: {e}"));
}

#[test]
fn read_file_is_built_as_the_sources_browsers_command() {
    let ui = operation_ui::spec("read-file").expect("read-file is built");
    assert_eq!(ui.verb, "open-sources-browser");
    assert_eq!(ui.starts, [Start::SourcesBrowser]);
    assert_eq!(
        operation_ui::by_verb("open-sources-browser").map(|ui| ui.long_name),
        Some("read-file")
    );
}

/// Whether the window's palette offers `open-sources-browser`, or the sentence
/// it greys the row with.
fn offered(win: &OpWindow) -> Result<(), String> {
    let registry = brightfield_keys::registry();
    let verb = registry
        .iter()
        .find(|v| v.longname == "open-sources-browser")
        .expect("the verb is in the registry");
    match brightfield_keys::availability(verb, &win.app.situation()).missing() {
        Some(why) => Err(why.to_string()),
        None => Ok(()),
    }
}

#[test]
fn the_bring_in_command_opens_the_sources_browser() {
    let mut win = OpWindow::on_own_protocol(SAMPLE);
    assert_eq!(offered(&win), Ok(()));
    let table = win.table();
    win.show_grid_of(&table);
    win.focus_grid();
    win.palette("open-sources-browser");
    assert_eq!(win.app.open_overlay(), Some("sources-browser"));
    assert!(win.making().is_none(), "the browser begins no draft");
    assert!(win.kept().is_empty());
}

#[test]
fn a_protocol_brightfield_did_not_make_says_sources_are_not_added_to_it() {
    const SENTENCE: &str = "Sources are added to Protocols Brightfield made, and this one was not.";
    let mut win = OpWindow::on_protocol_folder("shipped-protocol");
    let before = win.snapshot();
    // The palette greys the row with the sentence.
    assert_eq!(offered(&win), Err(SENTENCE.to_string()));
    // Started anyway, as a key an unavailable command keeps would start it, it
    // raises the browser's own banner, opens nothing and begins no draft.
    let spec = operation_ui::spec("read-file").expect("read-file is built");
    let ctx = win.ctx.clone();
    let started = win
        .app
        .start_operation_spec(&ctx, spec, Start::SourcesBrowser, None);
    win.settle();
    assert!(!started, "the browser did not open");
    let banners = win.banners();
    assert!(banners.contains(SENTENCE), "{banners}");
    assert!(win.making().is_none());
    assert_ne!(win.app.open_overlay(), Some("sources-browser"));
    assert_eq!(win.snapshot(), before);
}

#[test]
fn an_agents_read_file_step_opens_and_previews_its_file() {
    let win = OpWindow::on_copy_prepared(SAMPLE, |dir| {
        let from = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(WITH_NULLS);
        std::fs::copy(&from, dir.join(WITH_NULLS)).expect("the second file copies");
        read_file_step(dir, "cities", WITH_NULLS);
    });
    let mut win = win.reopen();
    assert!(!win.step_ran("cities"), "the step is not run yet");
    win.show_grid_of("cities");
    assert_eq!(
        win.grid_columns(),
        ["city", "house_age", "order", "median income"]
    );
    let rows = win
        .app
        .chart_doc()
        .grid_preview()
        .and_then(|p| p.page())
        .map(|page| page.rows.len());
    assert_eq!(rows, Some(3));
}

#[test]
fn an_agents_read_file_of_a_url_says_it_previews_local_files_only() {
    let mut win = OpWindow::on_copy_prepared(SAMPLE, |dir| {
        read_file_step(dir, "remote", "https://example.com/x.csv");
    });
    win.show_grid_of("remote");
    let refusal = win
        .app
        .chart_doc()
        .grid_preview()
        .and_then(|p| p.refusal())
        .map(ToString::to_string);
    assert!(win.making().is_none());
    assert_eq!(
        refusal.as_deref(),
        Some("Brightfield previews a local CSV or Parquet file; Run reads https://example.com/x.csv.")
    );
}
