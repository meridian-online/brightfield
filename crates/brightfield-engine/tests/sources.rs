//! Finding the tables in a folder, a workbook or a database file, and loading them with row
//! lineage. The folder, `book.xlsx` and `shop.sqlite` fixtures live under `tests/data`, and the
//! scripts beside them regenerate the binary ones.
//!
//! Workbooks and SQLite files are read through DuckDB's `excel` and `sqlite` extensions. A test
//! that needs one prints why and returns when the machine has neither installed nor a network to
//! fetch it.

use std::path::{Path, PathBuf};

use brightfield_engine::sources::{
    discover, extensions_needed, load_sql, Reader, SourceSet, SourceTable,
};
use duckdb::Connection;

fn data(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(rel)
}

/// A copy of the folder fixture with the `.DS_Store` a file manager would leave in it: the
/// checked-in tree ignores that name, so it is created here.
fn folder_with_junk() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for entry in std::fs::read_dir(data("own_files")).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), dir.path().join(entry.file_name())).unwrap();
    }
    std::fs::write(dir.path().join(".DS_Store"), b"\0\0\0\x01Bud1").unwrap();
    dir
}

/// Whether `ext` can be installed and loaded here; prints the reason when it cannot.
fn extension_available(ext: &str) -> bool {
    let conn = Connection::open_in_memory().unwrap();
    match conn.execute_batch(&format!("INSTALL {ext}; LOAD {ext};")) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("skipping: the {ext} extension is neither installed nor reachable: {e}");
            false
        }
    }
}

fn names(set: &SourceSet) -> Vec<&str> {
    set.tables.iter().map(|t| t.name.as_str()).collect()
}

/// Runs `table`'s load step in a fresh DuckDB and returns `(source_file, source_part, rows)`
/// ordered by `__source_row`, with the text of the columns in `cols`.
fn load(
    table: &SourceTable,
    protocol_dir: &Path,
    cols: &str,
) -> Vec<(String, Option<String>, i64, String)> {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&load_sql(table, protocol_dir)).unwrap();
    let sql = format!(
        "SELECT __source_file, __source_part, __source_row, concat_ws('|', {cols}) \
         FROM \"raw_{}\" ORDER BY __source_row",
        table.name
    );
    let mut stmt = conn.prepare(&sql).unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn folder_takes_readable_and_names_skipped() {
    let dir = folder_with_junk();
    let set = discover(dir.path()).unwrap();
    assert_eq!(set.tables.len(), 3, "{:?}", set.tables);
    assert_eq!(names(&set), ["items", "sales", "sales_2"]);
    let mut skipped: Vec<String> = set
        .skipped
        .iter()
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    skipped.sort();
    // The `.DS_Store` beside them is a hidden entry, passed over without being named.
    assert_eq!(skipped, ["broken.csv", "notes.txt"]);
    assert!(set.skipped.iter().all(|(_, why)| !why.is_empty()));
}

#[test]
fn hidden_entries_are_neither_read_nor_named() {
    let dir = folder_with_junk();
    // A hidden file that would read as a table, and a hidden folder.
    std::fs::write(dir.path().join(".scratch.csv"), "id\n1\n").unwrap();
    std::fs::create_dir(dir.path().join(".cache")).unwrap();
    let set = discover(dir.path()).unwrap();
    assert_eq!(names(&set), ["items", "sales", "sales_2"]);
    assert!(
        set.skipped
            .iter()
            .all(|(p, _)| !p.file_name().unwrap().to_string_lossy().starts_with('.')),
        "{:?}",
        set.skipped
    );
}

#[test]
fn name_collision_is_deterministic() {
    let dir = folder_with_junk();
    let first = discover(dir.path()).unwrap();
    let second = discover(dir.path()).unwrap();
    assert_eq!(first.tables, second.tables);
    let sales = &first.tables[1];
    let sales_2 = &first.tables[2];
    assert_eq!((sales.name.as_str(), sales.reader), ("sales", Reader::Csv));
    assert_eq!(
        (sales_2.name.as_str(), sales_2.reader),
        ("sales_2", Reader::Parquet)
    );
}

#[test]
fn a_table_named_as_another_tables_raw_or_failed_table_is_numbered() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["orders.csv", "raw_orders.csv", "orders__failed.csv"] {
        std::fs::write(dir.path().join(name), "id\n1\n").unwrap();
    }
    let set = discover(dir.path()).unwrap();
    // `orders` loads into `raw_orders` and keeps its failed values in `orders__failed`, so the
    // files named so are moved on to names of their own.
    assert_eq!(names(&set), ["orders", "orders__failed_2", "raw_orders_2"]);
}

#[test]
fn workbook_sheets_and_empty_sheet() {
    if !extension_available("excel") {
        return;
    }
    let set = discover(&data("book.xlsx")).unwrap();
    assert_eq!(names(&set), ["book_orders"]);
    assert_eq!(set.tables[0].part.as_deref(), Some("Orders"));
    assert_eq!(set.tables[0].reader, Reader::XlsxSheet);
    assert_eq!(set.skipped.len(), 1);
    assert!(
        set.skipped[0].1.contains("empty sheet"),
        "{:?}",
        set.skipped
    );
}

#[test]
fn blank_first_row_is_not_header() {
    if !extension_available("excel") {
        return;
    }
    let set = discover(&data("book.xlsx")).unwrap();
    let rows = load(&set.tables[0], &data(""), "id, item, qty");
    let got: Vec<&str> = rows.iter().map(|r| r.3.as_str()).collect();
    assert_eq!(got, ["1|apple|5", "2|pear|7", "3|plum|9"]);
}

#[test]
fn sqlite_file_lists_its_tables() {
    if !extension_available("sqlite") {
        return;
    }
    let set = discover(&data("shop.sqlite")).unwrap();
    assert_eq!(names(&set), ["shop_customers", "shop_orders"]);
    assert!(set.tables.iter().all(|t| t.reader == Reader::SqliteTable));
    assert_eq!(set.tables[0].part.as_deref(), Some("customers"));
    let rows = load(&set.tables[0], &data(""), "id, name");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].1.as_deref(), Some("customers"));
    assert_eq!(rows[0].3, "1|Ana");
}

#[test]
fn lineage_rows_are_one_based() {
    let dir = folder_with_junk();
    let set = discover(dir.path()).unwrap();
    let sales = set.tables.iter().find(|t| t.name == "sales").unwrap();
    let rows = load(sales, dir.path(), "id, name");
    let numbers: Vec<i64> = rows.iter().map(|r| r.2).collect();
    assert_eq!(numbers, [1, 2, 3]);
    let got: Vec<&str> = rows.iter().map(|r| r.3.as_str()).collect();
    assert_eq!(got, ["1|a", "2|b", "3|c"]);
    // A file inside the Protocol folder is named relative to it; a flat file has no part.
    assert_eq!(rows[0].0, "sales.csv");
    assert_eq!(rows[0].1, None);

    let parquet = set.tables.iter().find(|t| t.name == "sales_2").unwrap();
    let rows = load(parquet, Path::new("/elsewhere"), "id, name");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].2, 1);
    assert!(Path::new(&rows[0].0).is_absolute(), "{}", rows[0].0);
    assert_eq!(rows[0].3, "7|z");

    if extension_available("excel") {
        let set = discover(&data("book.xlsx")).unwrap();
        let rows = load(&set.tables[0], &data(""), "id");
        let numbers: Vec<i64> = rows.iter().map(|r| r.2).collect();
        assert_eq!(numbers, [1, 2, 3]);
        assert_eq!(rows[0].1.as_deref(), Some("Orders"));
        assert_eq!(rows[0].0, "book.xlsx");
    }
}

#[test]
fn duckdb_file_lists_its_tables() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("warehouse.duckdb");
    {
        let conn = Connection::open(&file).unwrap();
        conn.execute_batch(
            "CREATE TABLE trips (id INTEGER, city VARCHAR); \
             INSERT INTO trips VALUES (1, 'Oslo'), (2, 'Lima'); \
             CREATE TABLE stops (code VARCHAR); INSERT INTO stops VALUES ('A');",
        )
        .unwrap();
    }
    let set = discover(&file).unwrap();
    assert_eq!(names(&set), ["warehouse_stops", "warehouse_trips"]);
    assert!(set.tables.iter().all(|t| t.reader == Reader::DuckdbTable));
    let trips = set
        .tables
        .iter()
        .find(|t| t.part.as_deref() == Some("trips"))
        .unwrap();
    let rows = load(trips, dir.path(), "id, city");
    let got: Vec<&str> = rows.iter().map(|r| r.3.as_str()).collect();
    assert_eq!(got, ["1|Oslo", "2|Lima"]);
    assert_eq!(rows[1].2, 2);
    assert_eq!(rows[0].0, "warehouse.duckdb");
    assert_eq!(rows[0].1.as_deref(), Some("trips"));
}

#[test]
fn xlsx_load_step_installs_excel() {
    let table = SourceTable {
        name: "book_orders".into(),
        file: data("book.xlsx"),
        part: Some("Orders".into()),
        schema: None,
        reader: Reader::XlsxSheet,
    };
    assert!(load_sql(&table, &data("")).starts_with("INSTALL excel; LOAD excel;"));
}

#[test]
fn extensions_needed_names_excel_and_sqlite() {
    let table = |reader| SourceTable {
        name: "t".into(),
        file: PathBuf::from("/x"),
        part: None,
        schema: None,
        reader,
    };
    let set = |readers: &[Reader]| SourceSet {
        tables: readers.iter().map(|r| table(*r)).collect(),
        skipped: vec![],
    };
    assert!(
        extensions_needed(&set(&[Reader::Csv, Reader::Parquet, Reader::DuckdbTable])).is_empty()
    );
    assert_eq!(
        extensions_needed(&set(&[Reader::XlsxSheet, Reader::XlsxSheet])),
        ["excel"]
    );
    assert_eq!(
        extensions_needed(&set(&[Reader::SqliteTable, Reader::XlsxSheet, Reader::Csv])),
        ["excel", "sqlite"]
    );
}

#[test]
fn dotted_and_quoted_names_survive_the_load_sql() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("odd.duckdb");
    {
        let conn = Connection::open(&file).unwrap();
        conn.execute_batch(
            "CREATE TABLE \"sales.2024\" (v INTEGER); INSERT INTO \"sales.2024\" VALUES (1), (2); \
             CREATE TABLE \"a\"\"b\" (v INTEGER); INSERT INTO \"a\"\"b\" VALUES (3); \
             CREATE SCHEMA \"s.x\"; CREATE TABLE \"s.x\".t (v INTEGER); INSERT INTO \"s.x\".t VALUES (4);",
        )
        .unwrap();
    }
    let set = discover(&file).unwrap();
    assert_eq!(set.tables.len(), 3, "{:?}", set.tables);
    let rows_of = |part: &str, schema: Option<&str>| {
        let t = set
            .tables
            .iter()
            .find(|t| t.part.as_deref() == Some(part) && t.schema.as_deref() == schema)
            .unwrap();
        load(t, dir.path(), "v")
    };
    let dotted = rows_of("sales.2024", None);
    assert_eq!(
        dotted.iter().map(|r| r.3.as_str()).collect::<Vec<_>>(),
        ["1", "2"]
    );
    assert_eq!(dotted[0].1.as_deref(), Some("\"sales.2024\""));
    assert_eq!(rows_of("a\"b", None)[0].3, "3");
    assert_eq!(rows_of("t", Some("s.x"))[0].3, "4");
}

#[test]
fn apostrophes_survive_the_load_sql() {
    if extension_available("sqlite") {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("it's.sqlite");
        Connection::open_in_memory()
            .unwrap()
            .execute_batch(&format!(
                "LOAD sqlite; ATTACH '{}' AS w (TYPE sqlite); \
                 CREATE TABLE w.\"It's\" (v INTEGER); INSERT INTO w.\"It's\" VALUES (5); DETACH w;",
                file.display().to_string().replace('\'', "''")
            ))
            .unwrap();
        let set = discover(&file).unwrap();
        assert_eq!(set.tables[0].part.as_deref(), Some("It's"));
        let rows = load(&set.tables[0], dir.path(), "v");
        assert_eq!(
            (rows[0].0.as_str(), rows[0].1.as_deref(), rows[0].3.as_str()),
            ("it's.sqlite", Some("It's"), "5")
        );
    }
    // A sheet with an apostrophe in its name: book.xlsx's "Orders" re-pointed is not possible
    // without a second fixture, so the sheet case goes through the SQL text alone.
    let table = SourceTable {
        name: "b_it_s".into(),
        file: PathBuf::from("/x/o'brien.xlsx"),
        part: Some("It's".into()),
        schema: None,
        reader: Reader::XlsxSheet,
    };
    let sql = load_sql(&table, Path::new("/x"));
    assert!(
        sql.contains("sheet = 'It''s'") && sql.contains("'o''brien.xlsx'"),
        "{sql}"
    );
}
