//! The gate on SQL the user types: exactly one statement, and one that reads rows.
//!
//! These tests hold the gate to its two refusals, and to refusing before anything
//! runs: a `COPY` or `ATTACH` it turns away must leave no file behind.

use std::path::{Path, PathBuf};

use brightfield_engine::read_only_statement;

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("bf-typed-statement-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory");
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

#[test]
fn a_select_a_with_and_a_from_first_statement_pass_the_gate() {
    for sql in [
        "select 1",
        "with t as (select 1 as a) select a from t",
        "from range(3)",
        "select 1;",
        "select 1 -- a note",
        "SELECT\n  1\n",
    ] {
        assert_eq!(read_only_statement(sql), Ok(()), "{sql}");
    }
}

#[test]
fn a_second_statement_is_refused() {
    let e = read_only_statement("select 1; select 2").unwrap_err();
    assert!(e.starts_with("it reads as 2 statements, not one"), "{e}");
}

#[test]
fn each_side_effecting_kind_is_refused() {
    for sql in [
        "copy (select 1) to 'x.csv'",
        "attach 'x.db' as x",
        "install httpfs",
        "pragma version",
        "set threads = 1",
        "create table t as select 1",
        "insert into t values (1)",
        "update t set a = 1",
        "delete from t",
        "drop table t",
        "with t as (select 1) insert into u select * from t",
    ] {
        let e = read_only_statement(sql).unwrap_err();
        assert!(
            e.starts_with("it is not one query that reads rows: "),
            "{sql}: {e}"
        );
    }
}

#[test]
fn refusing_runs_nothing() {
    let dir = TempDir::new("refusing");
    let csv = dir.path().join("out.csv");
    let db = dir.path().join("x.db");
    assert!(read_only_statement(&format!("copy (select 1) to '{}'", csv.display())).is_err());
    assert!(read_only_statement(&format!("attach '{}' as x", db.display())).is_err());
    assert!(!csv.exists() && !db.exists());
}

#[test]
fn empty_and_comment_only_text_read_as_no_statements() {
    for sql in ["", "  \n", "-- note", ";"] {
        let e = read_only_statement(sql).unwrap_err();
        assert!(
            e.starts_with("it reads as 0 statements, not one"),
            "{sql:?}: {e}"
        );
    }
}

#[test]
fn a_syntax_error_is_refused_in_the_not_a_query_words() {
    let e = read_only_statement("selec 1").unwrap_err();
    assert!(
        e.starts_with("it is not one query that reads rows: "),
        "{e}"
    );
}
