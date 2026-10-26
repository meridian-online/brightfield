//! The front door opens folders, workbooks and database files as well as single files: a single
//! flat file keeps its route, and everything else becomes an own-files Protocol.
//!
//! Every test points the config directory and arc's directories at a scratch folder of its own
//! before anything reads them, so no Protocol lands under the developer's real data directory.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use brightfield_shell::data_file::{self, Opened};
use brightfield_shell::run::Runner;
use brightfield_shell::window::names_a_data_file;

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-front-door-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "input"] {
            std::fs::create_dir_all(root.join(sub)).expect("a scratch folder");
        }
        // Set once, before any test in this file reads them, and never changed.
        std::env::set_var(
            brightfield_shell::startup::CONFIG_DIR_VAR,
            root.join("config"),
        );
        std::env::set_var("ARCFORM_HISTORY_DIR", root.join("history"));
        std::env::set_var("ARCFORM_DB_DIR", root.join("db"));
        root
    })
}

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

fn input(name: &str) -> PathBuf {
    let dir = scratch().join("input").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const READINGS: &str = "id,reading\n1,2.5\n2,3.5\n3,4.5\n4,5.5\n";

fn csv_folder(name: &str) -> PathBuf {
    let dir = input(name);
    std::fs::write(dir.join("customers.csv"), "id,name\n1,Ada\n2,Bo\n3,Cy\n").unwrap();
    std::fs::write(dir.join("orders.csv"), "id,customer_id\n10,1\n11,2\n").unwrap();
    dir
}

/// A DuckDB file holding two tables, made with the linked engine.
fn database(name: &str, file: &str, ext_sql: &str) -> PathBuf {
    let path = input(name).join(file);
    let conn = duckdb::Connection::open(&path).unwrap();
    conn.execute_batch(ext_sql).unwrap();
    path
}

#[test]
fn single_csv_keeps_todays_route() {
    let dir = input("single");
    let csv = dir.join("readings.csv");
    std::fs::write(&csv, READINGS).unwrap();
    match data_file::open_any(&csv, &runner()).expect("a csv opens") {
        Opened::OneTable(opened) => assert!(!opened.dashboard.tiles().is_empty()),
        Opened::Model(_) => panic!("a single flat file must not become a model"),
    }
}

#[test]
fn folder_opens_a_model() {
    let dir = csv_folder("folder");
    let Opened::Model(p) = data_file::open_any(&dir, &runner()).expect("a folder opens") else {
        panic!("two tables are a model");
    };
    assert_eq!(p.sources.tables.len(), 2);
    let model = std::fs::read_to_string(p.dir.join("model.ossie.yaml")).expect("a model file");
    assert!(
        model.contains("customers") && model.contains("orders"),
        "{model}"
    );
    // The silver steps ran, so the manifest names them.
    let manifest = std::fs::read_to_string(p.dir.join("arcform.yaml")).unwrap();
    assert!(manifest.contains("silver_customers"), "{manifest}");
}

#[test]
fn workbook_opens_a_model() {
    // A workbook needs the excel extension and an authoring path; the fixture is written by
    // DuckDB itself.
    let dir = input("workbook");
    let book = dir.join("book.xlsx");
    let conn = duckdb::Connection::open_in_memory().unwrap();
    if conn
        .execute_batch("INSTALL excel; LOAD excel;")
        .and_then(|()| {
            conn.execute_batch(&format!(
                "COPY (SELECT 1 AS id, 'a' AS name) TO '{}' (FORMAT xlsx, SHEET 'People'); ",
                book.display()
            ))
        })
        .is_err()
    {
        eprintln!("skipped: the excel extension is not available here");
        return;
    }
    let Opened::Model(p) = data_file::open_any(&book, &runner()).expect("a workbook opens") else {
        panic!("a workbook is a model");
    };
    assert_eq!(p.sources.tables.len(), 1);
    assert!(p.dir.join("model.ossie.yaml").is_file());
}

#[test]
fn sqlite_opens_a_model() {
    let dir = input("sqlite");
    let file = dir.join("shop.sqlite");
    let conn = duckdb::Connection::open_in_memory().unwrap();
    if conn
        .execute_batch(&format!(
            "INSTALL sqlite; LOAD sqlite; ATTACH '{}' AS s (TYPE sqlite); \
             CREATE TABLE s.a AS SELECT 1 AS id; CREATE TABLE s.b AS SELECT 1 AS a_id;",
            file.display()
        ))
        .is_err()
    {
        eprintln!("skipped: the sqlite extension is not available here");
        return;
    }
    drop(conn);
    let Opened::Model(p) = data_file::open_any(&file, &runner()).expect("a sqlite file opens")
    else {
        panic!("two tables are a model");
    };
    assert_eq!(p.sources.tables.len(), 2);
    assert!(p.dir.join("model.ossie.yaml").is_file());
}

#[test]
fn duckdb_file_opens_a_model() {
    let path = database(
        "duck",
        "shop.duckdb",
        "CREATE TABLE a AS SELECT 1 AS id; CREATE TABLE b AS SELECT 1 AS a_id;",
    );
    let Opened::Model(p) = data_file::open_any(&path, &runner()).expect("a duckdb file opens")
    else {
        panic!("two tables are a model");
    };
    assert_eq!(p.sources.tables.len(), 2);
}

#[test]
fn accept_still_refuses_urls() {
    for url in [
        "https://example.com/a.csv",
        "s3://bucket/a.xlsx",
        "ducklake:x.db",
    ] {
        let refusal = data_file::accept(url).expect_err("a URL is not a local path");
        assert!(refusal.contains("URL"), "{url}: {refusal}");
        assert!(data_file::open_any(Path::new(url), &runner()).is_err());
    }
}

#[test]
fn typed_path_to_folder_is_recognised() {
    let dir = csv_folder("typed");
    assert!(names_a_data_file(&dir.to_string_lossy()));
    assert!(names_a_data_file("/tmp/no-such-book.XLSX"));
    assert!(data_file::accept(&dir.to_string_lossy()).is_ok());
}

#[test]
fn the_flat_route_refuses_what_it_cannot_read_as_one_table() {
    let dir = csv_folder("flat-refuses");
    let refusal = data_file::open(&dir.to_string_lossy())
        .err()
        .expect("a folder is not a table");
    assert!(refusal.contains("front door"), "{refusal}");
}

#[test]
fn fetch_message_names_the_extension() {
    let installed = vec!["json".to_string()];
    let message = data_file::fetch_message(&["excel"], &installed).expect("excel is missing");
    assert!(message.contains("excel"), "{message}");
    assert!(
        message.contains("first time a workbook is opened"),
        "{message}"
    );
    assert_eq!(
        data_file::fetch_message(&["excel"], &["excel".to_string()]),
        None
    );
    assert_eq!(data_file::fetch_message(&[], &installed), None);
    let both = data_file::fetch_message(&["excel", "sqlite"], &[]).unwrap();
    assert!(both.contains("excel") && both.contains("sqlite"), "{both}");
}

#[test]
fn a_folder_member_named_with_pattern_syntax_is_left_out_not_read_as_another_file() {
    let dir = input("glob-names");
    std::fs::write(dir.join("sales1.csv"), "id\n1\n2\n").unwrap();
    std::fs::write(dir.join("sales[1].csv"), "id\n9\n").unwrap();
    std::fs::write(dir.join("other.csv"), "k\n1\n").unwrap();
    let Opened::Model(p) = data_file::open_any(&dir, &runner()).expect("the folder opens") else {
        panic!("two readable tables are a model");
    };
    let names: Vec<&str> = p.sources.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["other", "sales1"]);
    assert!(p
        .sources
        .skipped
        .iter()
        .any(|(f, why)| f.ends_with("sales[1].csv") && why.contains("pattern")));
    let rows = p
        .profiles
        .iter()
        .find(|s| s.name == "sales1")
        .map(|s| &s.outcome);
    assert!(
        matches!(
            rows,
            Some(brightfield_engine::ProfileOutcome::Profiled { row_count: 2, .. })
        ),
        "{rows:?}"
    );
}
