//! Typed SQL, asked of arc.
//!
//! The runner is the `brightfield-shell` binary cargo built for this suite,
//! which becomes `arc` under the runner variable, so every answer here is
//! arc's own and not a fixture shaped like one.

use std::path::{Path, PathBuf};

use brightfield_shell::operation::{recognise, record_sql, Recognised};
use brightfield_shell::run::Runner;
use brightfield_shell::{data_file, starts};

const BY_AGE: &str = "select house_age, count(*) as n\nfrom california_housing\ngroup by house_age";
const STEP: &str = "california_housing_sql_by_house_age";

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("bf-sql-through-arc-{name}-{}", std::process::id()));
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
    std::fs::write(&data, include_bytes!("data/california_housing.parquet"))
        .expect("the housing file writes");
    let opened = data_file::open(&data.to_string_lossy()).expect("the housing file opens");
    opened
        .protocol
        .save_to(dir.path())
        .expect("the Protocol saves");
    dir
}

/// The model of the step `name` in the folder's manifest.
fn model(dir: &Path, name: &str) -> String {
    let text = std::fs::read_to_string(dir.join("arcform.yaml")).expect("the manifest reads");
    let manifest = brightfield_protocol::parse_manifest_str(&text).expect("the manifest parses");
    let step = manifest
        .steps
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no step {name} in the manifest"));
    std::fs::read_to_string(dir.join(step.sql.as_ref().expect("a sql step")))
        .expect("the model reads")
}

#[test]
fn record_sql_writes_the_statement_as_typed_under_the_tables_create_line() {
    let dir = saved_housing("record");
    record_sql(&runner(), dir.path(), BY_AGE, STEP).expect("arc records the statement");
    assert_eq!(
        model(dir.path(), STEP),
        format!("CREATE OR REPLACE TABLE \"{STEP}\" AS\n{BY_AGE}\n")
    );
}

#[test]
fn record_sql_answers_arcs_refusal_in_its_words() {
    let dir = saved_housing("refused");
    record_sql(&runner(), dir.path(), BY_AGE, STEP).expect("arc records the statement");
    let e = record_sql(&runner(), dir.path(), BY_AGE, STEP).unwrap_err();
    assert!(!e.starts_with("error: ") && e.contains(STEP), "{e}");
}

#[test]
fn a_filter_is_read_as_filter_rows_with_its_condition_as_typed() {
    assert_eq!(
        recognise(
            &runner(),
            "select * from california_housing where house_age > 28"
        )
        .unwrap(),
        Recognised::FilterRows {
            on: "california_housing".into(),
            condition: "house_age > 28".into()
        }
    );
    assert_eq!(
        recognise(
            &runner(),
            "select *\nfrom california_housing\nwhere house_age > 28"
        )
        .unwrap(),
        Recognised::FilterRows {
            on: "california_housing".into(),
            condition: "house_age > 28".into()
        }
    );
}

#[test]
fn a_sort_a_projection_and_a_group_are_read_as_no_operation() {
    for sql in [
        "select * from california_housing order by house_age",
        "select house_age from california_housing",
        "select house_age, count(*) from california_housing group by house_age",
    ] {
        assert_eq!(
            recognise(&runner(), sql).unwrap(),
            Recognised::Other,
            "{sql}"
        );
    }
}

#[test]
fn text_arc_cannot_parse_is_refused_in_arcs_words() {
    let e = recognise(&runner(), "select from where").unwrap_err();
    assert!(!e.starts_with("error: ") && !e.is_empty(), "{e}");
}
