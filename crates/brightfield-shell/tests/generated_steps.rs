//! Folders holding a data-file load and steps arc generated, recorded through
//! arc itself so the parser is held to what arc writes.

use std::path::{Path, PathBuf};

use brightfield_shell::generated::{
    generated_protocol_in, parse_generated, KnownOperation, StepEffect,
};
use brightfield_shell::run::Runner;
use brightfield_shell::{data_file, operation, starts};

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
            "bf-generated-{name}-{nanos}-{}",
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

fn manifest(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("arcform.yaml")).expect("the manifest reads")
}

fn record_filter(dir: &Path, on: &str, name: &str) {
    operation::record(
        &runner(),
        dir,
        "filter-rows",
        on,
        name,
        &[("where", "house_age > 28")],
    )
    .expect("arc records the filter");
}

fn record_sort(dir: &Path, on: &str, name: &str) {
    operation::record(
        &runner(),
        dir,
        "sort-rows",
        on,
        name,
        &[("order_by", "median_income DESC")],
    )
    .expect("arc records the sort");
}

fn model_of(dir: &Path, name: &str) -> PathBuf {
    let text = manifest(dir);
    let parsed = brightfield_protocol::parse_manifest_str(&text).expect("the manifest parses");
    let step = parsed
        .steps
        .iter()
        .find(|s| s.name == name)
        .expect("the step is in the manifest");
    dir.join(step.sql.as_ref().expect("a sql step"))
}

#[test]
fn parses_a_model_arc_actually_wrote() {
    let dir = saved_housing("wrote");
    record_filter(dir.path(), "california_housing", "kept");
    let model = std::fs::read_to_string(model_of(dir.path(), "kept")).expect("the model reads");
    let step = parse_generated("kept", &model).expect("arc's own model parses");
    assert_eq!(
        step.effect,
        StepEffect::Operation(KnownOperation::FilterRows)
    );
    assert_eq!(step.on, "california_housing");
    assert_eq!(step.argument, "house_age > 28");

    record_sort(dir.path(), "kept", "ordered");
    let model = std::fs::read_to_string(model_of(dir.path(), "ordered")).expect("the model reads");
    let step = parse_generated("ordered", &model).expect("arc's sort parses");
    assert_eq!(step.effect, StepEffect::Operation(KnownOperation::SortRows));
    assert_eq!(step.on, "kept");
    assert_eq!(step.argument, "median_income DESC");
}

#[test]
fn a_load_plus_generated_folder_is_recognised() {
    let dir = saved_housing("chain");
    record_filter(dir.path(), "california_housing", "kept");
    record_sort(dir.path(), "kept", "ordered");
    let found = generated_protocol_in(&manifest(dir.path()), dir.path())
        .expect("the folder reads")
        .expect("the folder is this shape");
    assert_eq!(found.root_table, "california_housing");
    assert!(
        found.data.ends_with(starts::HOUSING_FILE),
        "{:?}",
        found.data
    );
    let names: Vec<&str> = found.steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["kept", "ordered"]);
    assert_eq!(found.steps[0].on, "california_housing");
    assert_eq!(found.steps[1].on, "kept");
}

#[test]
fn a_generated_step_on_a_table_no_earlier_step_makes_is_refused() {
    let dir = saved_housing("orphan");
    record_filter(dir.path(), "california_housing", "kept");
    let path = model_of(dir.path(), "kept");
    let text = std::fs::read_to_string(&path).expect("the model reads");
    let moved = text
        .replace("on california_housing", "on nowhere")
        .replace("FROM california_housing", "FROM nowhere");
    std::fs::write(&path, moved).expect("the model writes");
    let err = generated_protocol_in(&manifest(dir.path()), dir.path())
        .expect_err("nothing makes nowhere");
    assert!(err.contains("kept") && err.contains("nowhere"), "{err}");
}

#[test]
fn a_hand_edited_step_refuses_the_folder() {
    let dir = saved_housing("edited");
    record_filter(dir.path(), "california_housing", "kept");
    let path = model_of(dir.path(), "kept");
    let text = std::fs::read_to_string(&path).expect("the model reads");
    std::fs::write(&path, text.replace("SELECT *", "SELECT house_age")).expect("writes");
    let err = generated_protocol_in(&manifest(dir.path()), dir.path()).expect_err("edited");
    assert!(err.contains("kept"), "{err}");
}

#[test]
fn a_one_step_folder_is_not_this_shape() {
    let dir = saved_housing("one");
    let found = generated_protocol_in(&manifest(dir.path()), dir.path()).expect("reads");
    assert_eq!(found, None);
}
