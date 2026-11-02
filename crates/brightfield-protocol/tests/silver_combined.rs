//! A dataset dovetail combined from several tables reads its own query rather than a raw table of
//! its name, and its parts get no silver tables.

use brightfield_ossie::{read, Model};
use brightfield_protocol::preview::join_rows_sql;
use brightfield_protocol::silver::{failed_sql, has_silver, silver_sql, FinetypeTransforms};
use duckdb::Connection;

fn transforms() -> FinetypeTransforms {
    FinetypeTransforms::vendored()
}

/// Parts `a` and `b` combined into `ab`, which tells them apart by `region`, and `ab` folded into
/// `host` on `id` when `fold` is set.
fn model(fold: bool) -> Model {
    let host = if fold {
        r#"
- name: host
  source: h.csv
  fields:
  - {name: id, expression: {sql: x}}
  custom_extensions:
  - {vendor_name: MERIDIAN, data: '{"role":"event","decided_by":"dovetail","source_file":"h"}'}"#
    } else {
        ""
    };
    let folded = if fold {
        r#""role":"reference","folded_into":"host","#
    } else {
        r#""role":"event","#
    };
    let relationship = if fold {
        r#"
relationships:
- name: host_ab
  from: host
  to: ab
  from_columns: [id]
  to_columns: [id]
  custom_extensions:
  - vendor_name: MERIDIAN
    data: '{"grade":"likely","evidence":{"value_overlap":1.0,"orphans":0,"child_rows":1,"child_type":"BIGINT","parent_type":"BIGINT","types_agree":true,"name_similarity":1.0},"cardinality":"N:1","verified":"full","decided_by":"dovetail"}'"#
    } else {
        ""
    };
    read(&format!(
        r#"version: 0.2.0.dev0
name: combined
datasets:{host}
- name: a
  source: a.csv
  fields:
  - {{name: id, expression: {{sql: x}}}}
  custom_extensions:
  - {{vendor_name: MERIDIAN, data: '{{"role":"event","combined_into":"ab","decided_by":"dovetail","source_file":"a"}}'}}
- name: b
  source: b.csv
  fields:
  - {{name: id, expression: {{sql: x}}}}
  custom_extensions:
  - {{vendor_name: MERIDIAN, data: '{{"role":"event","combined_into":"ab","decided_by":"dovetail","source_file":"b"}}'}}
- name: ab
  source: SELECT "id", 'x' AS "region" FROM "raw_a" UNION ALL BY NAME SELECT "id", 'y' AS "region" FROM "raw_b"
  fields:
  - {{name: id, expression: {{sql: x}}}}
  - {{name: region, expression: {{sql: x}}}}
  custom_extensions:
  - {{vendor_name: MERIDIAN, data: '{{{folded}"combination":{{"kind":"partition","parts":["a","b"],"token_column":"region"}},"decided_by":"dovetail","source_file":"ab"}}'}}{relationship}
"#
    ))
    .expect("the model reads")
}

fn db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE raw_a AS SELECT * FROM (VALUES ('a', NULL::VARCHAR, 1::BIGINT, 1::BIGINT), \
           ('a', NULL::VARCHAR, 2::BIGINT, 2::BIGINT)) t(__source_file, __source_part, __source_row, id); \
         CREATE TABLE raw_b AS SELECT * FROM (VALUES ('b', NULL::VARCHAR, 1::BIGINT, 1::BIGINT), \
           ('b', NULL::VARCHAR, 2::BIGINT, 3::BIGINT), \
           ('b', NULL::VARCHAR, 3::BIGINT, 4::BIGINT)) t(__source_file, __source_part, __source_row, id);",
    )
    .unwrap();
    conn
}

#[test]
fn a_combined_dataset_reads_its_query_with_lineage() {
    let m = model(false);
    let t = transforms();
    assert!(silver_sql(&m, "a", &t).is_empty());
    assert!(silver_sql(&m, "b", &t).is_empty());
    let conn = db();
    conn.execute_batch(&silver_sql(&m, "ab", &t)).unwrap();
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM ab", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 5);
    let columns: Vec<String> = conn
        .prepare("SELECT column_name FROM duckdb_columns() WHERE table_name = 'ab' ORDER BY column_index")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        columns,
        [
            "__source_file",
            "__source_part",
            "__source_row",
            "id",
            "region"
        ]
    );

    // Its failed-value table is made over the same query.
    conn.execute_batch(&failed_sql(&m, "ab", &t)).unwrap();
    let failed: i64 = conn
        .query_row("SELECT count(*) FROM ab__failed", [], |r| r.get(0))
        .unwrap();
    assert_eq!(failed, 0);
}

#[test]
fn a_folded_combination_picks_the_same_row_on_every_build() {
    // Both parts hold id 1, so the folded dataset's lineage (NULL here) cannot say which row
    // wins: the token column, then the fields, do.
    let m = model(true);
    let t = transforms();
    let conn = db();
    conn.execute_batch(
        "CREATE TABLE raw_host AS SELECT * FROM (VALUES ('h', NULL::VARCHAR, 1::BIGINT, 1::BIGINT)) \
         t(__source_file, __source_part, __source_row, id);",
    )
    .unwrap();
    let sql = silver_sql(&m, "host", &t);
    assert!(
        sql.contains("\"region\""),
        "the token column orders the pick: {sql}"
    );
    let mut picks = Vec::new();
    for _ in 0..2 {
        conn.execute_batch(&sql).unwrap();
        let (rows, region): (i64, String) = conn
            .query_row("SELECT count(*), any_value(ab_region) FROM host", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(rows, 1);
        picks.push(region);
    }
    assert_eq!(picks, ["x", "x"]);
}

#[test]
fn a_part_reads_through_the_cast_subquery_not_a_table_of_its_own() {
    // A join from a part to its combination, which is folded here: neither has a silver table, so
    // the preview casts each from its raw tables.
    let mut m = model(true);
    assert!(!has_silver(
        m.datasets.iter().find(|d| d.name == "a").unwrap()
    ));
    m.relationships[0].from = "a".into();
    let conn = db();
    let sql = join_rows_sql(&m, "host_ab");
    assert!(!sql.is_empty());
    let rows: i64 = conn
        .query_row(&format!("SELECT count(*) FROM ({sql})"), [], |r| r.get(0))
        .unwrap();
    assert!(rows > 0);
}

#[test]
fn a_cast_field_casts_over_the_combined_query() {
    let t = transforms();
    let mut m = model(false);
    let ab = m.datasets.iter_mut().find(|d| d.name == "ab").unwrap();
    ab.fields[0].meta.finetype = Some("representation.numeric.integer_number".into());
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE raw_a AS SELECT 'a' AS __source_file, NULL::VARCHAR AS __source_part, 1::BIGINT AS __source_row, '7' AS id; \
         CREATE TABLE raw_b AS SELECT 'b' AS __source_file, NULL::VARCHAR AS __source_part, 1::BIGINT AS __source_row, 'x' AS id;",
    )
    .unwrap();
    conn.execute_batch(&silver_sql(&m, "ab", &t)).unwrap();
    conn.execute_batch(&failed_sql(&m, "ab", &t)).unwrap();
    let typed: String = conn
        .query_row("SELECT typeof(id) FROM ab LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert_ne!(typed, "VARCHAR");
    let nulls: i64 = conn
        .query_row("SELECT count(*) FROM ab WHERE id IS NULL", [], |r| r.get(0))
        .unwrap();
    assert_eq!(nulls, 1);
    let failed: i64 = conn
        .query_row(
            "SELECT count(*) FROM ab__failed WHERE value = 'x'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(failed, 1);
}
