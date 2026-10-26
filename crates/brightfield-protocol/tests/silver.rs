//! The silver cast: each column through FineType's transformation contract for its label, folded
//! references joined in, and the values that did not cast kept in a companion table.

use brightfield_ossie::{read, Model};
use brightfield_protocol::silver::{cast_projection, failed_sql, silver_sql, FinetypeTransforms};
use duckdb::Connection;

const RETAIL: &str =
    include_str!("../../brightfield-ossie/fixtures/retail_extract/model.ossie.yaml");

fn transforms() -> FinetypeTransforms {
    FinetypeTransforms::vendored()
}

fn retail() -> Model {
    read(RETAIL).expect("the fixture reads")
}

#[test]
fn varchar_passes_through() {
    // `representation.text.word` is a VARCHAR label: no cast, whatever its transform says.
    let t = transforms();
    let label = "representation.text.word";
    assert_eq!(t.duckdb_type(label), Some("VARCHAR"));
    assert_eq!(cast_projection("w", Some(label), &t), "\"w\"");
}

#[test]
fn transform_is_try_wrapped() {
    let sql = cast_projection("d", Some("datetime.date.mdy_slash"), &transforms());
    assert_eq!(sql, "TRY(strptime(\"d\", '%m/%d/%Y')::DATE) AS \"d\"");
}

#[test]
fn no_transform_uses_try_cast() {
    let t = FinetypeTransforms::from_json(
        r#"{"tag":"v0","source":"","labels":{"x.y.z":{"duckdb_type":"BIGINT","transform":null}}}"#,
    )
    .unwrap();
    assert_eq!(
        cast_projection("n", Some("x.y.z"), &t),
        "TRY_CAST(\"n\" AS BIGINT) AS \"n\""
    );
}

#[test]
fn unlabelled_passes_through() {
    let t = transforms();
    assert_eq!(cast_projection("c", None, &t), "\"c\"");
    assert_eq!(cast_projection("c", Some("no.such.label"), &t), "\"c\"");
}

#[test]
fn awkward_names_are_quoted() {
    let t = transforms();
    let label = Some("datetime.date.iso");
    let sql = [
        cast_projection("Order ID", label, &t),
        cast_projection("select", label, &t),
        cast_projection("a\"b", label, &t),
    ];
    assert!(sql[0].contains("\"Order ID\""), "{}", sql[0]);
    assert!(sql[1].contains("\"select\""), "{}", sql[1]);
    assert!(sql[2].contains("\"a\"\"b\""), "{}", sql[2]);
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE r (\"Order ID\" VARCHAR, \"select\" VARCHAR, \"a\"\"b\" VARCHAR); \
         INSERT INTO r VALUES ('2024-01-02', '2024-02-03', 'nope')",
    )
    .unwrap();
    let query = format!(
        "SELECT CAST(\"Order ID\" AS VARCHAR), CAST(\"select\" AS VARCHAR), CAST(\"a\"\"b\" AS VARCHAR) \
         FROM (SELECT {} FROM r)",
        sql.join(", ")
    );
    let mut stmt = conn.prepare(&query).unwrap();
    let got: (String, String, Option<String>) = stmt
        .query_row([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap();
    assert_eq!(got, ("2024-01-02".into(), "2024-02-03".into(), None));
}

#[test]
fn fold_left_joins_with_prefix() {
    let sql = silver_sql(&retail(), "products", &transforms());
    assert!(
        sql.starts_with("CREATE OR REPLACE TABLE \"products\" AS"),
        "{sql}"
    );
    assert!(sql.contains("LEFT JOIN"), "{sql}");
    assert!(sql.contains("AS \"categories_name\""), "{sql}");
    // The join key is not repeated.
    assert!(!sql.contains("categories_category_id"), "{sql}");

    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE raw_products AS SELECT 'f' AS __source_file, NULL::VARCHAR AS __source_part, \
           1::BIGINT AS __source_row, 'BEV-001' AS sku, 'Cold brew' AS name, 10 AS category_id, '$4.50' AS unit_price; \
         CREATE TABLE raw_categories AS SELECT 'f' AS __source_file, NULL::VARCHAR AS __source_part, \
           1::BIGINT AS __source_row, 10 AS category_id, 'Beverages' AS name;",
    )
    .unwrap();
    conn.execute_batch(&sql).unwrap();
    let (name, cat): (String, String) = conn
        .query_row("SELECT name, categories_name FROM products", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((name.as_str(), cat.as_str()), ("Cold brew", "Beverages"));
}

#[test]
fn folded_and_excluded_datasets_have_no_silver() {
    let mut model = retail();
    assert_eq!(silver_sql(&model, "categories", &transforms()), "");
    let customers = model
        .datasets
        .iter_mut()
        .find(|d| d.name == "customers")
        .unwrap();
    customers.meta.excluded = true;
    assert_eq!(silver_sql(&model, "customers", &transforms()), "");
    assert_eq!(failed_sql(&model, "customers", &transforms()), "");
}

#[test]
fn failed_table_shape() {
    let mut model = retail();
    let products = model
        .datasets
        .iter_mut()
        .find(|d| d.name == "products")
        .unwrap();
    products.fields[3].meta.finetype = Some("datetime.date.mdy_slash".into());
    let sql = failed_sql(&model, "products", &transforms());
    assert!(
        sql.starts_with("CREATE OR REPLACE TABLE \"products__failed\" AS"),
        "{sql}"
    );

    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE raw_products AS SELECT * FROM (VALUES \
           ('f', NULL::VARCHAR, 1::BIGINT, 'A', 'a', 10, '01/02/2024'), \
           ('f', NULL::VARCHAR, 2::BIGINT, 'B', 'b', 10, 'yesterday'), \
           ('f', NULL::VARCHAR, 3::BIGINT, 'C', 'c', 10, NULL)) \
         t(__source_file, __source_part, __source_row, sku, name, category_id, unit_price);",
    )
    .unwrap();
    conn.execute_batch(&sql).unwrap();
    let cols: Vec<String> = conn
        .prepare("SELECT column_name FROM information_schema.columns WHERE table_name = 'products__failed' ORDER BY ordinal_position")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        cols,
        [
            "__source_file",
            "__source_part",
            "__source_row",
            "column",
            "value"
        ]
    );
    let rows: Vec<(i64, String, String)> = conn
        .prepare("SELECT __source_row, \"column\", \"value\" FROM products__failed")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows, [(2, "unit_price".into(), "yesterday".into())]);
}

#[test]
fn vendored_tag_is_the_pin() {
    let pin = include_str!("../../../packaging/finetype-pin.env");
    let tag = pin
        .lines()
        .find_map(|l| l.strip_prefix("FINETYPE_TAG="))
        .unwrap();
    assert_eq!(transforms().tag(), tag);
}

/// A tiny model: `items` (columns `Order ID`, `select`, `a"b`, each labelled a date) with the
/// dataset `look ups` folded into it on `select`.
fn awkward_model() -> Model {
    read(
        r#"version: 0.2.0.dev0
name: awkward
datasets:
- name: items
  source: items.csv
  fields:
  - name: Order ID
    expression: {sql: x}
    custom_extensions:
    - {vendor_name: MERIDIAN, data: '{"finetype":"datetime.date.iso","decided_by":"dovetail"}'}
  - name: select
    expression: {sql: x}
  - name: a"b
    expression: {sql: x}
    custom_extensions:
    - {vendor_name: MERIDIAN, data: '{"finetype":"datetime.date.iso","decided_by":"dovetail"}'}
  - name: look ups_note
    expression: {sql: x}
  custom_extensions:
  - {vendor_name: MERIDIAN, data: '{"role":"event","decided_by":"dovetail","source_file":"i"}'}
- name: look ups
  source: l.csv
  fields:
  - name: select
    expression: {sql: x}
  - name: note
    expression: {sql: x}
  - name: seen
    expression: {sql: x}
    custom_extensions:
    - {vendor_name: MERIDIAN, data: '{"finetype":"datetime.date.iso","decided_by":"dovetail"}'}
  custom_extensions:
  - {vendor_name: MERIDIAN, data: '{"role":"reference","folded_into":"items","decided_by":"dovetail","source_file":"l"}'}
relationships:
- name: items_look_ups
  from: items
  to: look ups
  from_columns: [select]
  to_columns: [select]
  custom_extensions:
  - vendor_name: MERIDIAN
    data: '{"grade":"likely","evidence":{"value_overlap":1.0,"orphans":0,"child_rows":1,"child_type":"VARCHAR","parent_type":"VARCHAR","types_agree":true,"name_similarity":1.0},"cardinality":"N:1","verified":"full","decided_by":"dovetail"}'
custom_extensions:
- {vendor_name: MERIDIAN, data: '{"generator":"test"}'}
"#,
    )
    .expect("the model reads")
}

fn awkward_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE raw_items AS SELECT * FROM (VALUES \
           ('i', NULL::VARCHAR, 1::BIGINT, '2024-01-02', 'k1', 'nope', 'mine'), \
           ('i', NULL::VARCHAR, 2::BIGINT, '2024-01-03', 'k2', '2024-05-06', 'mine'), \
           ('i', NULL::VARCHAR, 3::BIGINT, '2024-01-04', 'k9', NULL, 'mine')) \
           t(__source_file, __source_part, __source_row, \"Order ID\", \"select\", \"a\"\"b\", \"look ups_note\"); \
         CREATE TABLE \"raw_look ups\" AS SELECT * FROM (VALUES \
           ('l', NULL::VARCHAR, 1::BIGINT, 'k1', 'first', '2024-02-02'), \
           ('l', NULL::VARCHAR, 2::BIGINT, 'k1', 'duplicate', 'garbled'), \
           ('l', NULL::VARCHAR, 3::BIGINT, 'k2', 'other', NULL)) \
           t(__source_file, __source_part, __source_row, \"select\", note, seen);",
    )
    .unwrap();
    conn
}

#[test]
fn awkward_names_run_through_silver_and_failed() {
    let (model, t) = (awkward_model(), transforms());
    let conn = awkward_db();
    conn.execute_batch(&silver_sql(&model, "items", &t))
        .unwrap();
    conn.execute_batch(&failed_sql(&model, "items", &t))
        .unwrap();
    // A folded name that repeats a column of the host takes a suffix.
    let cols: Vec<String> = conn
        .prepare("SELECT column_name FROM information_schema.columns WHERE table_name = 'items' ORDER BY ordinal_position")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        cols[cols.len() - 3..],
        ["look ups_note", "look ups_note_2", "look ups_seen"]
    );
}

#[test]
fn fold_keeps_one_row_per_host_row_when_folded_keys_repeat() {
    let (model, t) = (awkward_model(), transforms());
    let conn = awkward_db();
    conn.execute_batch(&silver_sql(&model, "items", &t))
        .unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 3);
    let note: String = conn
        .query_row(
            "SELECT \"look ups_note_2\" FROM items WHERE __source_row = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(note, "first");
}

#[test]
fn failures_inside_a_folded_dataset_are_kept() {
    let (model, t) = (awkward_model(), transforms());
    let conn = awkward_db();
    conn.execute_batch(&failed_sql(&model, "items", &t))
        .unwrap();
    let mut rows: Vec<(String, i64, String, String)> = conn
        .prepare("SELECT __source_file, __source_row, \"column\", \"value\" FROM items__failed")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows.sort();
    assert_eq!(
        rows,
        [
            ("i".into(), 1, "a\"b".into(), "nope".into()),
            ("l".into(), 2, "look ups_seen".into(), "garbled".into()),
        ]
    );
}

#[test]
fn colliding_folded_names_agree_in_silver_and_failed() {
    // The folded `note` and `seen` columns are `look ups_note` and `look ups_seen`; a host column
    // named `look ups_seen` in another case takes the first, so the folded one is suffixed.
    let mut model = awkward_model();
    let items = model
        .datasets
        .iter_mut()
        .find(|d| d.name == "items")
        .unwrap();
    items.fields[3].name = "Look Ups_Seen".into();
    let t = transforms();
    let conn = awkward_db();
    conn.execute_batch("ALTER TABLE raw_items RENAME \"look ups_note\" TO \"Look Ups_Seen\"")
        .unwrap();
    conn.execute_batch(&silver_sql(&model, "items", &t))
        .unwrap();
    conn.execute_batch(&failed_sql(&model, "items", &t))
        .unwrap();
    let seen: Option<String> = conn
        .query_row(
            "SELECT CAST(\"look ups_seen_2\" AS VARCHAR) FROM items WHERE __source_row = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(seen.as_deref(), Some("2024-02-02"));
    let column: String = conn
        .query_row(
            "SELECT \"column\" FROM items__failed WHERE __source_file = 'l'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(column, "look ups_seen_2");
}
