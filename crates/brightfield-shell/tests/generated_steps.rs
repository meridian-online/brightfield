//! Folders holding a data-file load and steps arc generated, recorded through
//! arc itself so the parser is held to what arc writes.

use std::path::{Path, PathBuf};

use brightfield_shell::generated::{
    generated_protocol_in, parse_generated, parse_sql_step, KnownOperation, StepEffect,
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

/// Append a step named `name` whose model, at `models/<name>.sql`, is `sql`,
/// as a hand writes one.
fn append_hand_step(dir: &Path, name: &str, sql: &str) {
    let mut text = manifest(dir);
    text.push_str(&format!("  - name: {name}\n    sql: models/{name}.sql\n"));
    std::fs::write(dir.join("arcform.yaml"), text).expect("the manifest writes");
    std::fs::write(dir.join("models").join(format!("{name}.sql")), sql).expect("the model writes");
}

#[test]
fn arcs_sql_step_model_parses() {
    let s = parse_sql_step(
        "t",
        "CREATE OR REPLACE TABLE \"t\" AS\nfrom california_housing select house_age\n",
    )
    .unwrap();
    assert_eq!(
        (s.effect, s.reads.clone()),
        (StepEffect::Sql, vec!["california_housing".to_string()])
    );
    assert_eq!(s.argument, "from california_housing select house_age");
}

#[test]
fn a_sql_step_model_naming_another_table_is_refused() {
    assert!(
        parse_sql_step("t", "CREATE OR REPLACE TABLE \"u\" AS\nselect 1\n")
            .unwrap_err()
            .contains("\"t\"")
    );
}

#[test]
fn a_sql_step_model_with_a_second_statement_is_refused() {
    assert!(parse_sql_step(
        "t",
        "CREATE OR REPLACE TABLE \"t\" AS\nselect 1;\ndrop table x;\n"
    )
    .is_err());
}

#[test]
fn a_hand_written_step_after_the_load_still_opens_as_it_did() {
    let dir = saved_housing("hand");
    append_hand_step(dir.path(), "extra", "select 1\n");
    let found = generated_protocol_in(&manifest(dir.path()), dir.path()).expect("reads");
    assert_eq!(found, None);
}

#[test]
fn a_folder_of_recorded_sql_steps_reads_with_what_each_reads() {
    let dir = saved_housing("sql");
    operation::record_sql(
        &runner(),
        dir.path(),
        "select house_age, count(*) as n from california_housing group by house_age",
        "by_age",
    )
    .expect("arc records the statement");
    record_filter(dir.path(), "by_age", "kept");
    let found = generated_protocol_in(&manifest(dir.path()), dir.path())
        .expect("the folder reads")
        .expect("the folder is this shape");
    let read: Vec<(&str, StepEffect, Vec<String>)> = found
        .steps
        .iter()
        .map(|s| (s.name.as_str(), s.effect, s.reads.clone()))
        .collect();
    assert_eq!(
        read,
        [
            (
                "by_age",
                StepEffect::Sql,
                vec!["california_housing".to_string()]
            ),
            (
                "kept",
                StepEffect::Operation(KnownOperation::FilterRows),
                vec!["by_age".to_string()]
            ),
        ]
    );
}

#[test]
fn a_sql_step_reading_a_table_no_earlier_step_makes_is_refused() {
    let dir = saved_housing("sql-orphan");
    operation::record_sql(&runner(), dir.path(), "select * from nowhere", "lost")
        .expect("arc records the statement");
    let err = generated_protocol_in(&manifest(dir.path()), dir.path())
        .expect_err("nothing makes nowhere");
    assert!(err.contains("lost") && err.contains("nowhere"), "{err}");
}

#[test]
fn a_hand_written_step_beside_a_recorded_sql_step_is_refused_by_name() {
    let dir = saved_housing("sql-mixed");
    operation::record_sql(
        &runner(),
        dir.path(),
        "select house_age from california_housing",
        "ages",
    )
    .expect("arc records the statement");
    append_hand_step(dir.path(), "extra", "select 1\n");
    let err = generated_protocol_in(&manifest(dir.path()), dir.path())
        .expect_err("a step of neither shape");
    assert!(err.contains("\"extra\""), "{err}");
}

#[test]
fn a_sql_step_naming_a_table_in_another_case_reads_as_duckdb_resolves_it() {
    let dir = saved_housing("sql-case");
    operation::record_sql(
        &runner(),
        dir.path(),
        "select * from California_Housing",
        "cased",
    )
    .expect("arc records the statement");
    let found = generated_protocol_in(&manifest(dir.path()), dir.path())
        .expect("DuckDB reads the name without regard to case")
        .expect("the folder is this shape");
    assert_eq!(found.steps[0].reads, ["California_Housing"]);
}
