//! A draft read as it is edited: arc's model for it, asked for in a folder
//! that is thrown away, and run on a preview connection kept open.
//!
//! arc is the `brightfield-shell` binary cargo built for this suite, and the
//! rows are a copy of the housing sample in a folder of the test's own.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use brightfield_engine::FileReader;
use brightfield_shell::data_file;
use brightfield_shell::data_grid::{use_own_preview_gate_on_this_thread, PREVIEW_ROWS};
use brightfield_shell::operation::{self, StepRequest};
use brightfield_shell::operation_ui::draft::draft_model;
use brightfield_shell::operation_ui::reads::DraftReads;
use brightfield_shell::run::Runner;
use brightfield_shell::typed_sql::TypedSource;

/// The table the sample's load makes.
const ROOT: &str = "california_housing_sample";

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-operation-reads-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// A copy of the housing sample in this folder.
    fn sample(&self) -> PathBuf {
        let data = self.0.join("california_housing_sample.csv");
        std::fs::write(&data, include_bytes!("data/california_housing_sample.csv"))
            .expect("the sample writes");
        data
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn request(long_name: &str, other: &[&str], args: &[(&str, &str)]) -> StepRequest {
    StepRequest {
        long_name: long_name.to_string(),
        on: (long_name != "read-file").then(|| ROOT.to_string()),
        reads: other
            .iter()
            .map(|t| ("other".to_string(), (*t).to_string()))
            .collect(),
        args: args
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
    }
}

fn hide() -> StepRequest {
    request("hide-column", &[], &[("column", "house_age")])
}

fn rename(to: &str) -> StepRequest {
    request(
        "rename-column",
        &[],
        &[("column", "median_income"), ("to", to)],
    )
}

fn limit(count: &str) -> StepRequest {
    request("limit-rows", &[], &[("count", count)])
}

fn source(dir: &TempDir) -> TypedSource {
    use_own_preview_gate_on_this_thread();
    TypedSource {
        data: dir.sample(),
        reader: FileReader::Csv,
        root_table: ROOT.into(),
        steps: Vec::new(),
        order: None,
        runner: Some(runner()),
        shown: Default::default(),
        casts: Vec::new(),
    }
}

fn reads(dir: &TempDir) -> DraftReads {
    DraftReads::start(source(dir), vec![ROOT.to_string()], || {})
}

/// Poll until the request as it stands is answered or refused.
fn settle(reads: &mut DraftReads) {
    let until = Instant::now() + Duration::from_secs(120);
    loop {
        reads.poll();
        if reads.settled() {
            return;
        }
        assert!(Instant::now() < until, "the draft was never answered");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn arg<'a>(request: &'a StepRequest, key: &str) -> Option<&'a str> {
    request
        .args
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// The model arc writes for `request` as the step `name` in a Protocol saved
/// over the sample, with a second table `by_age` recorded in it.
fn recorded_model(name: &str, request: &StepRequest) -> String {
    let dir = TempDir::new("recorded");
    let data = dir.sample();
    let opened = data_file::open(&data.to_string_lossy()).expect("the sample opens");
    opened
        .protocol
        .save_to(dir.path())
        .expect("the Protocol saves");
    operation::record_sql(
        &runner(),
        dir.path(),
        &format!("SELECT house_age, count(*) AS n FROM {ROOT} GROUP BY house_age"),
        "by_age",
    )
    .expect("arc records by_age");
    operation::record_step(&runner(), dir.path(), name, request)
        .unwrap_or_else(|e| panic!("arc records {name}: {e}"));
    let text = std::fs::read_to_string(dir.path().join("arcform.yaml")).expect("the manifest");
    let manifest = brightfield_protocol::parse_manifest_str(&text).expect("the manifest parses");
    let step = manifest
        .steps
        .iter()
        .find(|s| s.name == name)
        .expect("the step is in the manifest");
    std::fs::read_to_string(dir.path().join(step.sql.as_ref().expect("a sql step")))
        .expect("the model reads")
}

#[test]
fn a_drafts_model_is_what_arc_records_in_the_protocol() {
    let join = request("join-tables", &["by_age"], &[("using", "house_age")]);
    let read = request("read-file", &[], &[("path", "https://example.com/x.csv")]);
    for (name, request) in [
        ("h", hide()),
        ("r", rename("my income")),
        ("j", join),
        ("f", read),
    ] {
        let draft = draft_model(&runner(), name, &request)
            .unwrap_or_else(|e| panic!("arc drafts {name}: {e}"));
        assert_eq!(draft.model, recorded_model(name, &request), "{name}");
    }
    // arc's notes come with the draft, and its refusal in its own words.
    let read = request("read-file", &[], &[("path", "https://example.com/x.csv")]);
    let draft = draft_model(&runner(), "f", &read).expect("arc drafts f");
    assert!(
        draft.notes[0].contains("is read live at each run"),
        "{:?}",
        draft.notes
    );
    let err = draft_model(&runner(), "l", &limit("ten")).expect_err("ten is no whole number");
    assert!(err.contains("is an integer, not \"ten\""), "{err}");
}

#[test]
fn an_edit_is_read_with_no_pause() {
    let dir = TempDir::new("no-pause");
    let mut reads = reads(&dir);
    assert_eq!(reads.reads_started(), 0);
    reads.edit("h", hide());
    // No further call is made on the reads: no tick and no wait for the text
    // to rest. The worker starts the read on its own.
    let until = Instant::now() + Duration::from_secs(120);
    while reads.reads_started() == 0 {
        assert!(Instant::now() < until, "the read never started");
        std::thread::sleep(Duration::from_millis(2));
    }
    settle(&mut reads);
    let answer = reads.current().expect("the draft is answered");
    assert_eq!(answer.request, hide());
    assert_eq!(answer.name, "h");
    assert_eq!(answer.page.rows.len() as u64, PREVIEW_ROWS.min(answer.rows));
    assert_eq!(answer.rows, answer.rows_in);
    assert!(answer.rows > 0);
    assert!(answer.page.columns.iter().all(|c| c.name != "house_age"));
    assert!(answer.missing.iter().all(|(c, _)| c != "house_age"));
    assert!(answer.unread.is_none());
    assert!(!reads.faded());

    // The same request again is not read again.
    let started = reads.reads_started();
    reads.edit("h", hide());
    assert!(reads.settled());
    assert_eq!(reads.reads_started(), started);
}

#[test]
fn an_answer_for_text_since_changed_is_never_shown() {
    let dir = TempDir::new("changed");
    let mut reads = reads(&dir);
    reads.edit("r", rename("a"));
    reads.edit("r", rename("ab"));
    let until = Instant::now() + Duration::from_secs(120);
    loop {
        reads.poll();
        // Whatever frame this is, an answer shown as current is for `ab`.
        for answer in [reads.current(), reads.shown()].into_iter().flatten() {
            assert_eq!(arg(&answer.request, "to"), Some("ab"));
        }
        if reads.settled() {
            break;
        }
        assert!(Instant::now() < until, "the draft was never answered");
        std::thread::sleep(Duration::from_millis(2));
    }
    let answer = reads.current().expect("the draft is answered");
    assert!(answer.page.columns.iter().any(|c| c.name == "ab"));
    assert!(
        answer.model.model.contains("TO ab"),
        "{}",
        answer.model.model
    );
}

#[test]
fn the_last_edit_of_a_burst_is_always_read() {
    let dir = TempDir::new("burst");
    let mut reads = reads(&dir);
    for n in 1..=10 {
        reads.edit("l", limit(&n.to_string()));
    }
    settle(&mut reads);
    let answer = reads.current().expect("the tenth edit is answered");
    assert_eq!(arg(&answer.request, "count"), Some("10"));
    assert_eq!(answer.rows, 10);
    // The worker read the newest waiting each time it came back, and not each
    // of the ten.
    assert!(reads.reads_started() < 10, "{}", reads.reads_started());
}

#[test]
fn a_refused_edit_keeps_the_last_good_rows_faded() {
    let dir = TempDir::new("refused");
    let mut reads = reads(&dir);
    reads.edit("l", limit("5"));
    settle(&mut reads);
    assert_eq!(reads.current().map(|a| a.rows), Some(5));
    assert!(!reads.faded());

    // arc refuses.
    reads.edit("l", limit("ten"));
    settle(&mut reads);
    let why = reads.refusal().expect("arc refuses ten");
    assert!(why.contains("is an integer, not \"ten\""), "{why}");
    assert!(reads.current().is_none());
    assert_eq!(reads.shown().map(|a| a.rows), Some(5));
    assert!(reads.faded());

    // DuckDB refuses: arc writes the model, and a value will not take the type.
    let cast = request(
        "cast-column",
        &[],
        &[("column", "median_income"), ("type", "DATE")],
    );
    reads.edit("l", cast);
    settle(&mut reads);
    let why = reads.refusal().expect("DuckDB refuses the cast");
    assert!(!why.is_empty());
    assert!(reads.current().is_none());
    assert_eq!(reads.shown().map(|a| a.rows), Some(5));
    assert!(reads.faded());

    // A good edit after it is shown, and no longer faded.
    reads.edit("l", limit("3"));
    settle(&mut reads);
    assert_eq!(reads.current().map(|a| a.rows), Some(3));
    assert!(reads.refusal().is_none());
    assert!(!reads.faded());
}

#[test]
fn a_preview_too_large_still_answers_with_arcs_model() {
    let dir = TempDir::new("too-large");
    let mut reads = DraftReads::start_under(source(&dir), vec![ROOT.to_string()], 1, || {});
    reads.edit("h", hide());
    settle(&mut reads);
    let answer = reads.current().expect("the draft is answered");
    assert_eq!(answer.model.model, recorded_model("h", &hide()));
    assert!(answer.page.rows.is_empty());
    let unread = answer.unread.as_deref().expect("it says why no rows");
    assert!(unread.contains("too large to preview"), "{unread}");
    assert!(reads.refusal().is_none());
}

#[test]
fn a_draft_that_reads_a_file_answers_with_arcs_model_and_says_when_it_is_read() {
    use brightfield_shell::operation_ui::reads::READ_WHEN_KEPT;
    let dir = TempDir::new("read-file");
    let mut reads = DraftReads::start(source(&dir), Vec::new(), || {});
    let local = request("read-file", &[], &[("path", "extra.csv")]);
    reads.edit("extra", local.clone());
    settle(&mut reads);
    let answer = reads.current().expect("the draft is answered");
    assert_eq!(answer.model.model, recorded_model("extra", &local));
    assert!(answer.page.rows.is_empty());
    assert_eq!(answer.unread.as_deref(), Some(READ_WHEN_KEPT));

    let remote = request("read-file", &[], &[("path", "https://example.com/x.csv")]);
    reads.edit("extra", remote);
    settle(&mut reads);
    let answer = reads.current().expect("the draft is answered");
    assert_eq!(
        answer.unread.as_deref(),
        Some("Brightfield previews a local CSV or Parquet file; Run reads https://example.com/x.csv.")
    );
    assert!(reads.refusal().is_none());
}

#[test]
fn dropping_the_reads_ends_the_work_under_way() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let dir = TempDir::new("dropped");
    let woken = Arc::new(AtomicUsize::new(0));
    let counter = woken.clone();
    let mut first = DraftReads::start(source(&dir), vec![ROOT.to_string()], move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    first.edit("h", hide());
    drop(first);
    // A second draft's reads share the first's gate, and are answered: the
    // first let go of it.
    let mut second = reads(&dir);
    second.edit("h", hide());
    settle(&mut second);
    assert!(second.current().is_some());
    // The first asked arc for no model and sent no answer.
    assert_eq!(woken.load(Ordering::SeqCst), 0);
}
