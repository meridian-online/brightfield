//! An analyst's own files written as a Protocol folder: the load steps arc runs, the raw tables
//! they leave, the profile and the data packages.
//!
//! Every test points the config directory and arc's directories at a scratch folder of its own
//! before anything reads them, so no Protocol lands under the developer's real data directory.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use brightfield_engine::{ProfileOutcome, SemanticType};
use brightfield_shell::run::Runner;
use brightfield_shell::{column_record, own_files};

/// The scratch root every test in this file shares, with the environment pointed into it.
fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-own-files-{}", std::process::id()));
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

/// A folder of two CSVs, named `name` so no two tests number the same Protocol.
fn csv_folder(name: &str) -> PathBuf {
    let dir = scratch().join("input").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("customers.csv"),
        "id,name,email\n1,Ada,ada@example.com\n2,Bo,bo@example.org\n3,Cy,cy@example.net\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("orders.csv"),
        "id,customer_id,total\n10,1,9.5\n11,2,20.0\n",
    )
    .unwrap();
    dir
}

fn query_strings(db: &Path, sql: &str) -> Vec<String> {
    let conn = duckdb::Connection::open_with_flags(
        db,
        duckdb::Config::default()
            .access_mode(duckdb::AccessMode::ReadOnly)
            .unwrap(),
    )
    .unwrap();
    let mut stmt = conn.prepare(sql).unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn creates_numbered_folder() {
    let root = scratch();
    let input = csv_folder("numbered");
    let one = own_files::create(&input).unwrap();
    let two = own_files::create(&input).unwrap();
    let own = root.join("config");
    assert!(one.dir.starts_with(&own), "{}", one.dir.display());
    assert_eq!(one.dir.file_name().unwrap(), "numbered-1");
    assert_eq!(two.dir.file_name().unwrap(), "numbered-2");
    assert_eq!(
        one.dir.parent().unwrap().file_name().unwrap(),
        own_files::OWN_DIR
    );
}

#[test]
fn manifest_has_one_load_step_per_table() {
    let p = own_files::create(&csv_folder("manifest")).unwrap();
    let text = std::fs::read_to_string(p.dir.join("arcform.yaml")).unwrap();
    let manifest: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
    assert_eq!(manifest["engine"], "duckdb");
    assert_eq!(
        manifest["name"],
        *p.dir.file_name().unwrap().to_str().unwrap()
    );
    let steps = manifest["steps"].as_sequence().unwrap();
    let names: Vec<_> = steps.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["load_customers", "load_orders"]);
    assert_eq!(steps[0]["sql"], "models/01_load_customers.sql");
    let sql = std::fs::read_to_string(p.dir.join("models/01_load_customers.sql")).unwrap();
    assert!(sql.contains("customers.csv"), "{sql}");
}

#[test]
fn raw_tables_carry_lineage() {
    let input = csv_folder("lineage");
    let p = own_files::create(&input).unwrap();
    own_files::run_loads(&p, &runner()).unwrap();
    let db = arc::spec::protocol_db_path(&p.dir).unwrap();
    let rows = query_strings(
        &db,
        "SELECT __source_file || '|' || coalesce(__source_part, 'NULL') || '|' || __source_row \
         FROM raw_customers ORDER BY __source_row",
    );
    let file = input.join("customers.csv");
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0],
        format!("{}|NULL|1", std::path::absolute(&file).unwrap().display())
    );
    assert!(rows[2].ends_with("|NULL|3"));
}

/// Profile and package a CSV folder; the shared setup of the two tests below.
fn packaged(name: &str) -> (own_files::OwnProtocol, Vec<PathBuf>) {
    let mut p = own_files::create(&csv_folder(name)).unwrap();
    own_files::run_loads(&p, &runner()).unwrap();
    own_files::profile(&mut p).unwrap();
    let written = own_files::write_packages(&p).unwrap();
    (p, written)
}

#[test]
fn packages_name_each_field() {
    let (p, written) = packaged("packages");
    assert_eq!(written.len(), 2);
    let text = std::fs::read_to_string(p.dir.join("packages/customers/datapackage.json")).unwrap();
    let package: serde_json::Value = serde_json::from_str(&text).unwrap();
    let fields = package["resources"][0]["schema"]["fields"]
        .as_array()
        .unwrap();
    let names: Vec<_> = fields.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["id", "name", "email"]);
    assert_eq!(fields[0]["type"], "integer");
    let orders = std::fs::read_to_string(p.dir.join("packages/orders/datapackage.json")).unwrap();
    let orders: serde_json::Value = serde_json::from_str(&orders).unwrap();
    let total = &orders["resources"][0]["schema"]["fields"][2];
    assert_eq!(
        (total["name"].as_str(), total["type"].as_str()),
        (Some("total"), Some("number"))
    );
    // The resource points back at the file the analyst gave, relative to the package.
    let path = package["resources"][0]["path"].as_str().unwrap();
    assert!(
        p.dir
            .join("packages/customers")
            .join(path)
            .canonicalize()
            .is_ok(),
        "{path}"
    );
}

#[test]
fn protocol_package_carries_x_brightfield() {
    let (p, _) = packaged("x_brightfield");
    let path = p.dir.join("packages/orders/datapackage.json");
    let mut record = column_record::read_record(&path, "orders")
        .unwrap()
        .unwrap();
    let total = record.fields.get_mut("total").unwrap();
    total.written.currency_symbol = Some("£".into());
    total.prefer.number_format = Some(",.0f".into());
    column_record::write_record(&path, "orders", &record).unwrap();

    // Writing the packages again, as a later open of the Protocol does, keeps the analyst's
    // preference. How the column was written is read again from the values, and `total` is not
    // a cast currency column, so the placed symbol goes.
    own_files::write_packages(&p).unwrap();

    let again = column_record::read_record(&path, "orders")
        .unwrap()
        .unwrap();
    let total = &again.fields["total"];
    assert_eq!(total.written, column_record::Written::default());
    assert_eq!(total.prefer.number_format.as_deref(), Some(",.0f"));
}

const CURRENCY: &str = "finance.currency.amount";

/// Set `column`'s label on `resource` in the package at `package` as an analyst's accepted one, or
/// take its label away when `label` is `None`; `number_format` is put on it as a preference when
/// given.
fn nominate(
    package: &Path,
    resource: &str,
    column: &str,
    label: Option<&str>,
    number_format: Option<&str>,
) {
    let mut record = column_record::read_record(package, resource)
        .unwrap()
        .unwrap();
    let field = record.fields.get_mut(column).unwrap();
    field.label = label.map(str::to_string);
    field.nominated = label.is_some();
    if let Some(format) = number_format {
        field.prefer.number_format = Some(format.to_string());
    }
    column_record::write_record(package, resource, &record).unwrap();
}

fn written_of(package: &Path, resource: &str, column: &str) -> column_record::FieldRecord {
    column_record::read_record(package, resource)
        .unwrap()
        .unwrap()
        .fields
        .remove(column)
        .unwrap()
}

/// Write a one-sheet workbook at `book` whose `amount` column holds `amounts`, beside a `region`
/// column; false when the excel extension cannot be had here.
fn write_book(book: &Path, amounts: [&str; 2]) -> bool {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    if let Err(e) = conn.execute_batch("INSTALL excel; LOAD excel;") {
        eprintln!("skipping: the excel extension cannot be installed here: {e}");
        return false;
    }
    conn.execute_batch(&format!(
        "COPY (SELECT * FROM (VALUES ('{}', 'North'), ('{}', 'South')) t(amount, region)) \
         TO '{}' (FORMAT xlsx, HEADER true)",
        amounts[0],
        amounts[1],
        book.display()
    ))
    .unwrap();
    true
}

#[test]
fn protocol_package_records_how_a_column_was_written() {
    let dir = scratch().join("input").join("written_book");
    std::fs::create_dir_all(&dir).unwrap();
    let book = dir.join("book.xlsx");
    if !write_book(&book, ["£1,234.50", "(£12.00)"]) {
        return;
    }
    let mut p = own_files::create(&book).unwrap();
    own_files::run_loads(&p, &runner()).unwrap();
    own_files::profile(&mut p).unwrap();
    own_files::write_packages(&p).unwrap();
    let package = p.dir.join("packages/book/datapackage.json");
    // The label FineType would give, accepted, so the test needs no FineType bundle.
    nominate(&package, "book_sheet1", "amount", Some(CURRENCY), None);
    own_files::write_packages(&p).unwrap();

    let amount = written_of(&package, "book_sheet1", "amount");
    assert_eq!(amount.written.currency_symbol.as_deref(), Some("£"));
    assert_eq!(
        amount.written.negatives,
        Some(column_record::Negatives::Parentheses)
    );
    let region = written_of(&package, "book_sheet1", "region");
    assert_eq!(region.written, column_record::Written::default());

    // The sheet is re-exported in dollars with minus signs, and the analyst prefers a format.
    assert!(write_book(&book, ["$1,234.50", "-$12.00"]));
    nominate(
        &package,
        "book_sheet1",
        "amount",
        Some(CURRENCY),
        Some(",.0f"),
    );
    own_files::run_loads(&p, &runner()).unwrap();
    own_files::write_packages(&p).unwrap();

    let amount = written_of(&package, "book_sheet1", "amount");
    assert_eq!(amount.written.currency_symbol.as_deref(), Some("$"));
    assert_eq!(
        amount.written.negatives,
        Some(column_record::Negatives::Minus)
    );
    assert_eq!(amount.prefer.number_format.as_deref(), Some(",.0f"));
}

/// A folder holding `sales.csv`, whose `amount` column is written in sterling with parentheses.
fn sterling_folder(name: &str) -> PathBuf {
    let dir = scratch().join("input").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("sales.csv"),
        "id,amount\n1,\"£1,234.50\"\n2,(£12.00)\n",
    )
    .unwrap();
    dir
}

#[test]
fn a_column_no_longer_cast_loses_its_written() {
    let mut p = own_files::create(&sterling_folder("written_uncast")).unwrap();
    own_files::run_loads(&p, &runner()).unwrap();
    own_files::profile(&mut p).unwrap();
    own_files::write_packages(&p).unwrap();
    let package = p.dir.join("packages/sales/datapackage.json");
    nominate(&package, "sales", "amount", Some(CURRENCY), None);
    own_files::write_packages(&p).unwrap();
    assert_eq!(
        written_of(&package, "sales", "amount")
            .written
            .currency_symbol
            .as_deref(),
        Some("£")
    );

    // The label is taken away, so the silver table no longer casts `amount`.
    nominate(&package, "sales", "amount", None, None);
    own_files::write_packages(&p).unwrap();
    assert_eq!(
        written_of(&package, "sales", "amount").written,
        column_record::Written::default()
    );
}

#[test]
fn protocol_written_is_read_from_the_raw_table() {
    let mut p = own_files::create(&sterling_folder("written_raw")).unwrap();
    own_files::run_loads(&p, &runner()).unwrap();
    own_files::profile(&mut p).unwrap();
    own_files::write_packages(&p).unwrap();
    let package = p.dir.join("packages/sales/datapackage.json");
    nominate(&package, "sales", "amount", Some(CURRENCY), None);

    // The silver table casts `amount` to a number, whose text holds no symbol.
    let model = brightfield_ossie::read(&format!(
        r#"version: 0.2.0.dev0
name: written
datasets:
- name: sales
  source: sales.csv
  fields:
  - name: id
    expression: {{sql: id}}
  - name: amount
    expression: {{sql: amount}}
    custom_extensions:
    - vendor_name: MERIDIAN
      data: '{{"finetype":"{CURRENCY}","decided_by":"dovetail"}}'
  custom_extensions:
  - vendor_name: MERIDIAN
    data: '{{"role":"event","excluded":false,"decided_by":"dovetail","source_file":"sales.csv"}}'
custom_extensions:
- vendor_name: MERIDIAN
  data: '{{"generator":"test"}}'
"#
    ))
    .unwrap();
    own_files::write_silver(&p, &model).unwrap();
    own_files::run_loads(&p, &runner()).unwrap();
    let db = arc::spec::protocol_db_path(&p.dir).unwrap();
    assert_eq!(
        query_strings(&db, "SELECT CAST(amount AS VARCHAR) FROM sales ORDER BY id"),
        ["1234.50", "-12.00"]
    );

    own_files::write_packages(&p).unwrap();
    let amount = written_of(&package, "sales", "amount");
    assert_eq!(amount.written.currency_symbol.as_deref(), Some("£"));
    assert_eq!(
        amount.written.negatives,
        Some(column_record::Negatives::Parentheses)
    );
}

#[test]
fn protocol_package_keeps_a_nominated_label() {
    let (p, _) = packaged("nominated");
    let path = p.dir.join("packages/customers/datapackage.json");
    let mut record = column_record::read_record(&path, "customers")
        .unwrap()
        .unwrap();
    let name = record.fields.get_mut("name").unwrap();
    name.label = Some("identity.person.full_name".into());
    name.nominated = true;
    column_record::write_record(&path, "customers", &record).unwrap();

    own_files::write_packages(&p).unwrap();

    let again = column_record::read_record(&path, "customers")
        .unwrap()
        .unwrap();
    assert!(again.fields["name"].nominated);
    assert_eq!(
        again.fields["name"].label.as_deref(),
        Some("identity.person.full_name")
    );
    assert!(!again.fields["email"].nominated);
}

#[test]
fn broken_package_is_regenerated() {
    let (p, _) = packaged("broken_package");
    let path = p.dir.join("packages/customers/datapackage.json");
    std::fs::write(&path, "{\"resources\": [").unwrap();
    own_files::write_packages(&p).unwrap();
    let package: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(package["resources"][0]["name"], "customers");
}

#[test]
fn profile_labels_email_column() {
    let (p, _) = packaged("labels");
    let customers = p.profiles.iter().find(|s| s.name == "customers").unwrap();
    let ProfileOutcome::Profiled { columns, .. } = &customers.outcome else {
        panic!("not profiled: {:?}", customers.outcome);
    };
    assert!(columns.iter().all(|c| !c.name.starts_with("__source_")));
    let email = columns.iter().find(|c| c.name == "email").unwrap();
    if matches!(email.semantic, SemanticType::NotAsked) {
        eprintln!("skipping the label assertion: no FineType bundle beside the test binary");
        return;
    }
    let label = email
        .semantic
        .label()
        .expect("a label for the email column");
    assert!(label.ends_with(".email"), "{label}");
    let text = std::fs::read_to_string(p.dir.join("packages/customers/datapackage.json")).unwrap();
    assert!(
        text.contains(&format!("\"x-finetype-label\": \"{label}\"")),
        "{text}"
    );
}

/// A workbook and a SQLite file load under arc's own DuckDB, which installs each extension.
#[test]
fn workbook_and_sqlite_files_load_under_arc() {
    let dir = scratch().join("input").join("mixed");
    std::fs::create_dir_all(&dir).unwrap();
    let conn = duckdb::Connection::open_in_memory().unwrap();
    if let Err(e) = conn.execute_batch("INSTALL excel; LOAD excel; INSTALL sqlite; LOAD sqlite;") {
        eprintln!("skipping: the excel and sqlite extensions cannot be installed here: {e}");
        return;
    }
    conn.execute_batch(&format!(
        "COPY (SELECT 1 AS a, 'x' AS b) TO '{0}/book.xlsx' (FORMAT xlsx, HEADER true); \
         ATTACH '{0}/lite.sqlite' AS lite (TYPE sqlite); \
         CREATE TABLE lite.t AS SELECT 7 AS n;",
        dir.display()
    ))
    .unwrap();
    drop(conn);
    let p = own_files::create(&dir).unwrap();
    let names: Vec<_> = p.sources.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["book_sheet1", "lite_t"]);
    own_files::run_loads(&p, &runner()).unwrap();
    let db = arc::spec::protocol_db_path(&p.dir).unwrap();
    assert_eq!(query_strings(&db, "SELECT b FROM raw_book_sheet1"), ["x"]);
    assert_eq!(
        query_strings(&db, "SELECT CAST(n AS VARCHAR) FROM raw_lite_t"),
        ["7"]
    );
}

/// Package folders are lower-cased and distinct: `a.csv`, `a.xlsx` and `a_xlsx.csv` are three
/// folders, and `Sales.csv` and `sales.parquet` are two.
#[test]
fn package_folders_never_collide() {
    let dir = scratch().join("input").join("collide");
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["a.csv", "a_xlsx.csv", "Sales.csv"] {
        std::fs::write(dir.join(name), "x\n1\n").unwrap();
    }
    let conn = duckdb::Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!(
        "COPY (SELECT 1 AS x) TO '{0}/sales.parquet' (FORMAT parquet);",
        dir.display()
    ))
    .unwrap();
    // The workbook is only needed when the excel extension can be had; the others still collide.
    let workbook = conn
        .execute_batch(&format!(
            "INSTALL excel; LOAD excel; COPY (SELECT 1 AS x) TO '{0}/a.xlsx' (FORMAT xlsx, HEADER true);",
            dir.display()
        ))
        .is_ok();
    drop(conn);
    let mut p = own_files::create(&dir).unwrap();
    own_files::run_loads(&p, &runner()).unwrap();
    own_files::profile(&mut p).unwrap();
    let written = own_files::write_packages(&p).unwrap();
    let mut folders: Vec<String> = written
        .iter()
        .map(|w| {
            w.parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    folders.sort();
    let mut expected = vec!["a", "a_xlsx", "sales", "sales_parquet"];
    if workbook {
        expected.push("a_xlsx_csv");
        expected.sort();
    }
    assert_eq!(folders, expected);
}

/// A model over the folder `silver_folder` makes: `events.when` is labelled a US-style date, and
/// `notes` is left out of the silver tables when `exclude_notes` is set.
fn silver_model(exclude_notes: bool) -> brightfield_ossie::Model {
    let notes_meta = format!(
        r#"{{"role":"unclear","excluded":{exclude_notes},"decided_by":"dovetail","source_file":"notes.csv"}}"#
    );
    let yaml = format!(
        r#"version: 0.2.0.dev0
name: silver
datasets:
- name: events
  source: events.csv
  fields:
  - name: id
    expression: {{sql: id}}
  - name: when
    expression: {{sql: when}}
    custom_extensions:
    - vendor_name: MERIDIAN
      data: '{{"finetype":"datetime.date.mdy_slash","decided_by":"dovetail"}}'
  custom_extensions:
  - vendor_name: MERIDIAN
    data: '{{"role":"event","excluded":false,"decided_by":"dovetail","source_file":"events.csv"}}'
- name: notes
  source: notes.csv
  fields:
  - name: text
    expression: {{sql: text}}
  custom_extensions:
  - vendor_name: MERIDIAN
    data: '{notes_meta}'
custom_extensions:
- vendor_name: MERIDIAN
  data: '{{"generator":"test"}}'
"#
    );
    brightfield_ossie::read(&yaml).unwrap()
}

fn silver_folder(name: &str) -> PathBuf {
    let dir = scratch().join("input").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("events.csv"),
        "id,when\n1,01/02/2024\n2,not a date\n3,03/04/2024\n",
    )
    .unwrap();
    std::fs::write(dir.join("notes.csv"), "text\nhello\n").unwrap();
    dir
}

#[test]
fn silver_runs_and_keeps_failures() {
    let p = own_files::create(&silver_folder("silver_runs")).unwrap();
    own_files::write_silver(&p, &silver_model(false)).unwrap();
    own_files::run_loads(&p, &runner()).unwrap();
    let db = arc::spec::protocol_db_path(&p.dir).unwrap();
    let dates = query_strings(
        &db,
        "SELECT coalesce(CAST(\"when\" AS VARCHAR), 'NULL') FROM events ORDER BY __source_row",
    );
    assert_eq!(dates, ["2024-01-02", "NULL", "2024-03-04"]);
    let failed = query_strings(
        &db,
        "SELECT __source_row || '|' || \"column\" || '|' || \"value\" FROM events__failed",
    );
    assert_eq!(failed, ["2|when|not a date"]);
    assert_eq!(
        query_strings(&db, "SELECT count(*)::VARCHAR FROM notes__failed"),
        ["0"]
    );
}

#[test]
fn silver_steps_are_rewritten_not_accumulated() {
    let p = own_files::create(&silver_folder("silver_rewrite")).unwrap();
    let names = |p: &own_files::OwnProtocol| -> Vec<String> {
        let text = std::fs::read_to_string(p.dir.join("arcform.yaml")).unwrap();
        let manifest: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
        manifest["steps"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap().to_string())
            .collect()
    };
    own_files::write_silver(&p, &silver_model(false)).unwrap();
    own_files::write_silver(&p, &silver_model(false)).unwrap();
    assert_eq!(
        names(&p),
        [
            "load_events",
            "load_notes",
            "silver_events",
            "failed_events",
            "silver_notes",
            "failed_notes"
        ]
    );
    own_files::write_silver(&p, &silver_model(true)).unwrap();
    assert_eq!(
        names(&p),
        [
            "load_events",
            "load_notes",
            "silver_events",
            "failed_events"
        ]
    );
    assert!(!p.dir.join("models/02_silver_notes.sql").exists());
    assert!(!p.dir.join("models/02_failed_notes.sql").exists());
    assert!(p.dir.join("models/02_silver_events.sql").exists());
}

#[test]
fn a_currency_column_stored_as_a_number_records_no_written() {
    let dir = scratch().join("input").join("written_numeric");
    std::fs::create_dir_all(&dir).unwrap();
    let conn = duckdb::Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!(
        "COPY (SELECT * FROM (VALUES (1, 1234.5::DOUBLE), (2, -12.0::DOUBLE)) t(id, amount)) \
         TO '{}' (FORMAT parquet)",
        dir.join("ledger.parquet").display()
    ))
    .unwrap();
    drop(conn);
    let mut p = own_files::create(&dir).unwrap();
    own_files::run_loads(&p, &runner()).unwrap();
    own_files::profile(&mut p).unwrap();
    own_files::write_packages(&p).unwrap();
    let package = p.dir.join("packages/ledger/datapackage.json");
    nominate(&package, "ledger", "amount", Some(CURRENCY), None);
    own_files::write_packages(&p).unwrap();
    let amount = written_of(&package, "ledger", "amount");
    assert_eq!(amount.label.as_deref(), Some(CURRENCY));
    // DuckDB's `-12.0` is not how a source wrote a negative.
    assert_eq!(amount.written, column_record::Written::default());
}

#[test]
fn a_raw_table_that_cannot_be_read_keeps_the_written_recorded() {
    let mut p = own_files::create(&sterling_folder("written_unread")).unwrap();
    own_files::run_loads(&p, &runner()).unwrap();
    own_files::profile(&mut p).unwrap();
    own_files::write_packages(&p).unwrap();
    let package = p.dir.join("packages/sales/datapackage.json");
    nominate(&package, "sales", "amount", Some(CURRENCY), None);
    own_files::write_packages(&p).unwrap();
    assert_eq!(
        written_of(&package, "sales", "amount")
            .written
            .currency_symbol
            .as_deref(),
        Some("£")
    );

    // The raw table is gone, so the aggregate fails; that is no news about the column.
    let db = arc::spec::protocol_db_path(&p.dir).unwrap();
    duckdb::Connection::open(&db)
        .unwrap()
        .execute_batch("DROP TABLE raw_sales")
        .unwrap();
    own_files::write_packages(&p).unwrap();
    let amount = written_of(&package, "sales", "amount");
    assert_eq!(amount.written.currency_symbol.as_deref(), Some("£"));
    assert_eq!(
        amount.written.negatives,
        Some(column_record::Negatives::Parentheses)
    );
}
