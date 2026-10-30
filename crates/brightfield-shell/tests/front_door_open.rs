//! The front door opens folders, workbooks and database files as well as single files: a single
//! flat file keeps its route, and everything else becomes an own-files Protocol.
//!
//! Every test points the config directory and arc's directories at a scratch folder of its own
//! before anything reads them, so no Protocol lands under the developer's real data directory.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use brightfield_engine::semantic::{OpenTypeSource, TypeSource, TypeSourceSpec};
use brightfield_engine::SemanticType;
use brightfield_shell::column_record;
use brightfield_shell::data_file::{self, Opened};
use brightfield_shell::run::Runner;
use brightfield_shell::window::{names_a_data_file, Boot};

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
fn a_file_named_as_another_tables_raw_or_failed_table_still_opens() {
    let dir = input("derived-names");
    std::fs::write(dir.join("orders.csv"), "id,amount\n1,2.5\n2,3.5\n").unwrap();
    std::fs::write(dir.join("raw_orders.csv"), "id,note\n1,a\n").unwrap();
    std::fs::write(dir.join("orders__failed.csv"), "id,why\n1,b\n").unwrap();
    // The open runs the Protocol twice, the second time with the silver steps, so arc has
    // accepted its manifest when it lands.
    let Opened::Model(p) = data_file::open_any(&dir, &runner()).expect("the folder opens") else {
        panic!("three tables are a model");
    };
    let names: Vec<&str> = p.sources.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["orders", "orders__failed_2", "raw_orders_2"]);
    let manifest = std::fs::read_to_string(p.dir.join("arcform.yaml")).unwrap();
    for step in [
        "silver_orders",
        "silver_raw_orders_2",
        "failed_orders__failed_2",
    ] {
        assert!(manifest.contains(step), "{step}: {manifest}");
    }
    let tables = brightfield_shell::model_inspector::query(
        &p.dir,
        "SELECT table_name FROM duckdb_tables() ORDER BY table_name",
    )
    .expect("the run's database reads");
    let made: Vec<&str> = tables
        .rows
        .iter()
        .filter_map(|row| row.first().and_then(Option::as_deref))
        .collect();
    for table in [
        "orders",
        "raw_orders",
        "orders__failed",
        "raw_orders_2",
        "orders__failed_2",
    ] {
        assert!(made.contains(&table), "{table}: {made:?}");
    }
}

#[test]
fn single_sheet_workbook_opens_one_table() {
    // A workbook needs the excel extension and an authoring path; the fixture is written by
    // DuckDB itself.
    let dir = input("workbook");
    let book = dir.join("book.xlsx");
    let conn = duckdb::Connection::open_in_memory().unwrap();
    if conn
        .execute_batch("INSTALL excel; LOAD excel;")
        .and_then(|()| {
            conn.execute_batch(&format!(
                "COPY (SELECT * FROM (VALUES (1, 'a'), (2, 'b'), (3, 'c')) t(id, name)) \
                 TO '{}' (FORMAT xlsx, SHEET 'People'); ",
                book.display()
            ))
        })
        .is_err()
    {
        eprintln!("skipped: the excel extension is not available here");
        return;
    }
    match data_file::open_any(&book, &runner()).expect("a workbook opens") {
        Opened::OneTable(opened) => {
            assert!(!opened.dashboard.tiles().is_empty());
            // The sheet's rows, without the lineage the load gave them.
            let columns: Vec<String> = opened
                .dashboard
                .tiles()
                .iter()
                .map(|t| t.column().to_string())
                .collect();
            assert!(
                !columns.iter().any(|c| c.starts_with("__source")),
                "{columns:?}"
            );
        }
        Opened::Model(_) => panic!("a workbook of one sheet opens on the grid, not as a model"),
    }
}

#[test]
fn single_table_database_opens_one_table() {
    let path = database(
        "duck-one",
        "one.duckdb",
        "CREATE TABLE readings AS SELECT * FROM (VALUES (1, 2.5), (2, 3.5), (3, 4.5)) t(id, reading);",
    );
    match data_file::open_any(&path, &runner()).expect("a duckdb file opens") {
        Opened::OneTable(opened) => assert!(!opened.dashboard.tiles().is_empty()),
        Opened::Model(_) => panic!("a database of one table opens on the grid, not as a model"),
    }
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
fn a_typed_folder_or_workbook_waits_for_the_window_and_a_flat_file_does_not() {
    let flow = brightfield_protocol::layout::Flow::Vertical;
    let dir = csv_folder("typed-route");
    // A folder, workbook or database opens through the door's own open on the window's first
    // frame, not through the flat-file route, which refuses them.
    for typed in [
        dir.to_string_lossy().into_owned(),
        "/tmp/bf-no-such-book.xlsx".to_string(),
        "/tmp/bf-no-such-shop.sqlite".to_string(),
    ] {
        let boot = Boot::open(&typed, flow, None).expect("it waits for the window");
        assert_eq!(boot.open_later.as_deref(), Some(typed.as_str()));
        assert!(
            boot.live.is_none(),
            "{typed}: nothing is opened before the window"
        );
    }
    let csv = input("typed-flat").join("readings.csv");
    std::fs::write(&csv, READINGS).unwrap();
    let boot = Boot::open(&csv.to_string_lossy(), flow, None).expect("a csv opens");
    assert_eq!(boot.open_later, None);
    assert!(
        boot.live.is_some(),
        "a flat file opens at once, as it always has"
    );
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

/// A one-sheet workbook whose sheet holds `amount` written as `values`, or `None` when the excel
/// extension cannot be had here.
fn workbook(name: &str, values: &[&str]) -> Option<PathBuf> {
    let book = input(name).join("book.xlsx");
    let rows: Vec<String> = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!("({i}, '{v}')"))
        .collect();
    let conn = duckdb::Connection::open_in_memory().unwrap();
    let made = conn
        .execute_batch("INSTALL excel; LOAD excel;")
        .and_then(|()| {
            conn.execute_batch(&format!(
                "COPY (SELECT * FROM (VALUES {}) t(id, amount)) TO '{}' (FORMAT xlsx, HEADER true, SHEET 'Sales')",
                rows.join(", "),
                book.display()
            ))
        });
    match made {
        Ok(()) => Some(book),
        Err(e) => {
            eprintln!("skipped: the excel extension is not available here: {e}");
            None
        }
    }
}

/// The Protocol folder a grid copy at `copy` (`<protocol>/.grid/<table>.parquet`) belongs to.
fn protocol_of(copy: &Path) -> PathBuf {
    copy.parent().unwrap().parent().unwrap().to_path_buf()
}

/// The label the Protocol's data package records for `column` of the table `copy` is a grid copy
/// of.
fn package_label(copy: &Path, column: &str) -> Option<String> {
    let resource = copy.file_stem().unwrap().to_string_lossy().into_owned();
    std::fs::read_dir(protocol_of(copy).join("packages"))
        .unwrap()
        .flatten()
        .find_map(|e| {
            column_record::read_record(&e.path().join("datapackage.json"), &resource).unwrap()
        })
        .and_then(|r| r.fields.get(column).and_then(|f| f.label.clone()))
}

fn semantic_of(opened: &data_file::OpenedFile, column: &str) -> SemanticType {
    opened
        .protocol
        .profiles
        .iter()
        .find(|c| c.name == column)
        .unwrap_or_else(|| panic!("no column {column}"))
        .semantic
        .clone()
}

const CURRENCY: &str = "finance.currency.amount";

fn given(label: &str) -> SemanticType {
    SemanticType::Labelled {
        label: label.to_string(),
        confidence: 1.0,
        check: brightfield_engine::ValueCheck::Given,
    }
}

#[test]
fn single_sheet_cast_copy_keeps_its_currency_label() {
    let Some(book) = workbook(
        "cast-label",
        &[
            "$1,234.50",
            "$12.00",
            "$980.25",
            "$45.10",
            "$3,210.00",
            "$7.75",
        ],
    ) else {
        return;
    };
    let Opened::OneTable(opened) = data_file::open_any(&book, &runner()).expect("it opens") else {
        panic!("a workbook of one sheet opens on the grid");
    };
    // The label comes from FineType when the Protocol is profiled; without a bundle beside the
    // test binary there is none to keep.
    let recorded = package_label(&opened.protocol.data, "amount");
    if recorded.as_deref() != Some(CURRENCY) {
        eprintln!("skipping: the Protocol's package labels amount {recorded:?}, not {CURRENCY}");
        return;
    }
    assert_eq!(semantic_of(&opened, "amount"), given(CURRENCY));
}

#[test]
fn cast_copy_is_not_cast_again() {
    let Some(book) = workbook(
        "cast-again",
        &[
            "£1,234.50",
            "(£12.00)",
            "£980.25",
            "£45.10",
            "£3,210.00",
            "£7.75",
        ],
    ) else {
        return;
    };
    let Opened::OneTable(opened) = data_file::open_any(&book, &runner()).expect("it opens") else {
        panic!("a workbook of one sheet opens on the grid");
    };
    let copy = opened.protocol.data.clone();
    let recorded = package_label(&copy, "amount");
    if recorded.as_deref() != Some(CURRENCY) {
        eprintln!("skipping: the Protocol's package labels amount {recorded:?}, not {CURRENCY}");
        return;
    }
    drop(opened);
    // Reopened as a recent file is: the copy's own path, on the flat route.
    let (again, trace) =
        data_file::open_traced(&copy.to_string_lossy(), &data_file::OpenOptions::default())
            .expect("the cast copy opens again");
    assert!(trace.already_cast);
    let amount = again
        .protocol
        .profiles
        .iter()
        .find(|c| c.name == "amount")
        .unwrap();
    assert_eq!(amount.min.as_deref(), Some("-12.00"), "{amount:?}");
}

#[test]
fn copy_inside_a_protocol_folder_is_recognised() {
    let path = database(
        "duck-recognised",
        "sales.duckdb",
        "CREATE TABLE sales AS SELECT * FROM (VALUES (1, 12.50, 'North'), (2, 980.00, 'South'), \
         (3, 45.10, 'East')) t(id, amount, region);",
    );
    let Opened::OneTable(opened) = data_file::open_any(&path, &runner()).expect("it opens") else {
        panic!("a database of one table opens on the grid");
    };
    let copy = opened.protocol.data.clone();
    drop(opened);
    // The Protocol's package records a label for `amount`, as FineType would have written it.
    let resource = copy.file_stem().unwrap().to_string_lossy().into_owned();
    let package = std::fs::read_dir(protocol_of(&copy).join("packages"))
        .unwrap()
        .flatten()
        .map(|e| e.path().join("datapackage.json"))
        .find(|p| column_record::read_record(p, &resource).unwrap().is_some())
        .expect("a package naming the table");
    let mut record = column_record::read_record(&package, &resource)
        .unwrap()
        .unwrap();
    let amount = record.fields.get_mut("amount").unwrap();
    amount.label = Some(CURRENCY.into());
    amount.nominated = false;
    column_record::write_record(&package, &resource, &record).unwrap();

    let options = data_file::OpenOptions {
        type_source: None,
        ..data_file::OpenOptions::default()
    };
    let (reopened, trace) =
        data_file::open_traced(&copy.to_string_lossy(), &options).expect("the copy opens");
    assert!(trace.already_cast);
    assert_eq!(semantic_of(&reopened, "amount"), given(CURRENCY));

    // Another file in the grid folder, such as a join preview, is built from cast tables: already
    // cast, with no labels given.
    let preview = copy.with_file_name("preview.parquet");
    std::fs::copy(&copy, &preview).unwrap();
    let (other, trace) =
        data_file::open_traced(&preview.to_string_lossy(), &options).expect("a preview opens");
    assert!(trace.already_cast);
    assert_eq!(
        semantic_of(&other, "amount"),
        SemanticType::NotAsked,
        "a preview is not the table's copy"
    );

    // The raw snapshot the Protocol was profiled from is not cast.
    let raw = protocol_of(&copy)
        .join(".profile")
        .join(format!("{resource}.parquet"));
    let (_, trace) =
        data_file::open_traced(&raw.to_string_lossy(), &options).expect("the snapshot opens");
    assert!(!trace.already_cast);
}

/// A type source that answers `stub.label` for every column and remembers which it was asked.
struct Recording(Arc<Mutex<Vec<String>>>);

impl TypeSource for Recording {
    fn name(&self) -> &str {
        "recording"
    }
    fn typing_expr(&self, column: &str, _duckdb_type: &str) -> Result<String, String> {
        self.0.lock().unwrap().push(column.to_string());
        let c = column.replace('"', "\"\"");
        Ok(format!(
            "any_value('stub.label') FILTER (WHERE \"{c}\" IS NOT NULL)"
        ))
    }
    fn read_and_check(
        &self,
        _conn: &duckdb::Connection,
        _view: &str,
        _column: &str,
        _cell: &dyn duckdb::arrow::array::Array,
        _row: usize,
    ) -> SemanticType {
        SemanticType::Labelled {
            label: "stub.label".to_string(),
            confidence: 1.0,
            check: brightfield_engine::ValueCheck::NoConstraint,
        }
    }
}

struct OpenRecording(Arc<Mutex<Vec<String>>>);

impl OpenTypeSource for OpenRecording {
    fn open(&self, _conn: &duckdb::Connection) -> Result<Box<dyn TypeSource>, String> {
        Ok(Box::new(Recording(Arc::clone(&self.0))))
    }
}

#[test]
fn nominated_label_is_not_asked() {
    let csv = input("nominated").join("sales.csv");
    std::fs::write(
        &csv,
        "amount,region\n1234.50,North\n12.00,South\n980.25,East\n",
    )
    .unwrap();
    // The record nominates `amount`; `region`'s label is only inferred, so it is asked again.
    let mut record = column_record::SourceRecord::default();
    record.fields.insert(
        "amount".into(),
        column_record::FieldRecord {
            label: Some(CURRENCY.into()),
            nominated: true,
            ..column_record::FieldRecord::default()
        },
    );
    record.fields.insert(
        "region".into(),
        column_record::FieldRecord {
            label: Some("geography.location.region".into()),
            ..column_record::FieldRecord::default()
        },
    );
    let package = column_record::flat_record_path(&csv).unwrap();
    column_record::write_record(&package, data_file::SOURCE, &record).unwrap();

    let asked = Arc::new(Mutex::new(Vec::new()));
    let options = data_file::OpenOptions {
        type_source: Some(TypeSourceSpec::Other(Arc::new(OpenRecording(Arc::clone(
            &asked,
        ))))),
        ..data_file::OpenOptions::default()
    };
    let (opened, trace) =
        data_file::open_traced(&csv.to_string_lossy(), &options).expect("the csv opens");
    assert_eq!(*asked.lock().unwrap(), ["region"]);
    assert_eq!(semantic_of(&opened, "amount"), given(CURRENCY));
    assert_eq!(semantic_of(&opened, "region").label(), Some("stub.label"));
    assert!(!trace.already_cast, "a lone file is not already cast");
}
