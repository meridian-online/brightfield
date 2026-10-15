//! arc's operations, reached through the child runner with an argument list.
//!
//! The runner is the `brightfield-shell` binary cargo built for this suite,
//! which becomes `arc` under the runner variable, so every answer here is
//! arc's own and not a fixture shaped like one.

use std::path::{Path, PathBuf};

use brightfield_shell::data_file;
use brightfield_shell::operation::{self, DescriptionState, Descriptions};
use brightfield_shell::run::Runner;
use brightfield_shell::starts;

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "bf-operation-{name}-{nanos}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The housing start saved as a one-step Protocol in a fresh directory.
fn saved_housing(name: &str) -> TempDir {
    let dir = TempDir::new(name);
    let data = dir.path().join(starts::HOUSING_FILE);
    std::fs::write(&data, starts::HOUSING_PARQUET).expect("the housing file writes");
    let opened = data_file::open(&data.to_string_lossy()).expect("the housing file opens");
    opened
        .protocol
        .save_to(dir.path())
        .expect("the Protocol saves");
    dir
}

/// Every file under `dir` with its bytes, in path order.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).expect("the folder reads") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes = std::fs::read(&path).expect("a file reads");
                out.push((path, bytes));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn the_runner_lists_both_operations() {
    let names = operation::list_operations(&runner()).expect("arc lists its operations");
    assert!(names.contains(&"filter-rows".to_string()), "{names:?}");
    assert!(names.contains(&"sort-rows".to_string()), "{names:?}");
}

#[test]
fn describing_filter_rows_yields_where_and_its_six_comparisons() {
    let d = operation::describe(&runner(), "filter-rows").expect("filter-rows describes");
    assert_eq!(d.long_name, "filter-rows");
    assert!(!d.summary.is_empty());
    let reads: Vec<&str> = d.reads.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(reads, ["table", "column"]);
    assert_eq!(d.reads[0].kind, "table");
    assert!(d.reads[0].required);
    assert!(!d.reads[1].required);
    assert!(!d.reads[0].description.is_empty());
    assert_eq!(d.parameters.len(), 1);
    assert_eq!(d.parameters[0].key, "where");
    assert!(!d.parameters[0].description.is_empty());
    let words: Vec<&str> = d.parameters[0]
        .offered
        .iter()
        .map(|o| o.word.as_str())
        .collect();
    assert_eq!(
        words,
        ["is", "is not", "over", "under", "between", "is null"]
    );
    assert_eq!(d.parameters[0].offered[1].sign, "!=");
}

#[test]
fn describing_sort_rows_yields_order_by_and_two_directions() {
    let d = operation::describe(&runner(), "sort-rows").expect("sort-rows describes");
    assert_eq!(d.parameters.len(), 1);
    assert_eq!(d.parameters[0].key, "order_by");
    let pairs: Vec<(&str, &str)> = d.parameters[0]
        .offered
        .iter()
        .map(|o| (o.word.as_str(), o.sign.as_str()))
        .collect();
    assert_eq!(pairs, [("ascending", "asc"), ("descending", "desc")]);
}

#[test]
fn an_unknown_operation_is_refused_in_arcs_words() {
    let err = operation::describe(&runner(), "nope").expect_err("nope is no operation");
    assert!(err.starts_with("no operation or operator called"), "{err}");
}

/// Poll `cache` for `name` until it is no longer pending, within a bound.
fn settled(cache: &mut Descriptions, runner: &Runner, name: &str) -> Result<String, String> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        match cache.get(runner, name) {
            DescriptionState::Ready(d) => return Ok(d.long_name.clone()),
            DescriptionState::Failed(e) => return Err(e.to_string()),
            DescriptionState::Pending => {
                assert!(std::time::Instant::now() < until, "still pending");
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }
}

#[test]
fn descriptions_are_pending_then_ready_and_a_failure_is_kept() {
    let woken = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = woken.clone();
    let mut cache = Descriptions::with_wake(move || {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    // The first call only starts the ask; it does not wait for the child.
    assert_eq!(cache.get(&runner(), "sort-rows"), DescriptionState::Pending);
    assert_eq!(
        settled(&mut cache, &runner(), "sort-rows"),
        Ok("sort-rows".to_string())
    );
    // The worker sends the answer before it wakes, so wait for the wake.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while woken.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        assert!(std::time::Instant::now() < until, "the wake never came");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let missing = Runner::at("/nonexistent/brightfield-runner");
    let first = settled(&mut cache, &missing, "filter-rows").expect_err("no runner");
    assert!(first.contains("could not start the runner"), "{first}");
    // A working runner would answer now, but the failure is the session's answer.
    let second = settled(&mut cache, &runner(), "filter-rows").expect_err("kept");
    assert_eq!(first, second);
}

#[test]
fn record_appends_a_generated_step_and_runs_nothing() {
    let dir = saved_housing("record");
    let said = operation::record(
        &runner(),
        dir.path(),
        "filter-rows",
        "california_housing",
        "california_housing_filtered_by_house_age",
        &[("where", "house_age > 28")],
    )
    .expect("arc records the step");
    assert!(!said.is_empty());
    let manifest = std::fs::read_to_string(dir.path().join("arcform.yaml")).expect("manifest");
    assert_eq!(manifest.matches("- name:").count(), 2, "{manifest}");
    let model = std::fs::read_dir(dir.path().join("models"))
        .expect("models")
        .map(|e| std::fs::read_to_string(e.expect("entry").path()).expect("model"))
        .find(|text| text.starts_with("-- generated:"))
        .expect("a generated model");
    assert_eq!(
        model.lines().next(),
        Some("-- generated: filter-rows on california_housing")
    );
    assert!(!dir.path().join("build").exists(), "recording ran nothing");
}

#[test]
fn record_refuses_a_step_as_on() {
    let dir = saved_housing("refuse");
    let before = snapshot(dir.path());
    let err = operation::record(
        &runner(),
        dir.path(),
        "filter-rows",
        "load",
        "load_filtered",
        &[("where", "house_age > 28")],
    )
    .expect_err("load is a step, not a table");
    assert!(
        err.contains("no step of the protocol makes a table called"),
        "{err}"
    );
    assert!(!err.starts_with("error:"), "{err}");
    assert_eq!(snapshot(dir.path()), before, "the folder is untouched");
}

#[test]
fn record_into_a_relative_directory_lands_in_that_directory() {
    // A relative path is resolved against this process's working directory, so
    // the folder is made under it and removed after.
    let name = format!("bf-relative-{}", std::process::id());
    let here = std::env::current_dir().expect("a working directory");
    let abs = TempDir(here.join(&name));
    std::fs::create_dir_all(abs.path()).expect("the folder");
    let data = abs.path().join(starts::HOUSING_FILE);
    std::fs::write(&data, starts::HOUSING_PARQUET).expect("the housing file writes");
    let opened = data_file::open(&data.to_string_lossy()).expect("the housing file opens");
    opened
        .protocol
        .save_to(abs.path())
        .expect("the Protocol saves");

    operation::record(
        &runner(),
        Path::new(&name),
        "filter-rows",
        "california_housing",
        "california_housing_filtered_by_house_age",
        &[("where", "house_age > 28")],
    )
    .expect("a relative folder records");
    let manifest = std::fs::read_to_string(abs.path().join("arcform.yaml")).expect("manifest");
    assert_eq!(manifest.matches("- name:").count(), 2, "{manifest}");
    assert!(
        !abs.path().join(&name).exists(),
        "no folder inside the folder"
    );
}
