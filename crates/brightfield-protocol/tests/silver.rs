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
