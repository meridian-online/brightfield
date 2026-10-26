use brightfield_ossie::{read, validate, write, Role, VENDOR};

const STAND_IN: &str = include_str!("../fixtures/retail_extract/model.ossie.yaml");

fn dataset<'a>(m: &'a brightfield_ossie::Model, name: &str) -> &'a brightfield_ossie::Dataset {
    m.datasets.iter().find(|d| d.name == name).unwrap()
}

#[test]
fn stand_in_reads_and_validates() {
    let m = read(STAND_IN).expect("stand-in reads");
    assert_eq!(m.datasets.len(), 8);
    assert_eq!(m.relationships.len(), 7);
    assert_eq!(dataset(&m, "transactions").meta.role, Role::Event);
    assert_eq!(
        dataset(&m, "categories").meta.folded_into.as_deref(),
        Some("products")
    );
    assert_eq!(dataset(&m, "promo_codes").meta.role, Role::Unclear);
    assert_eq!(validate(STAND_IN), vec![]);
}

#[test]
fn write_round_trips() {
    let m = read(STAND_IN).unwrap();
    let out = write(&m);
    assert_eq!(read(&out).unwrap(), m);
    assert_eq!(write(&m), out);
    assert_eq!(validate(&out), vec![]);
}

#[test]
fn meridian_extension_survives_a_foreign_reader() {
    let out = write(&read(STAND_IN).unwrap());
    let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
    let mut seen = 0;
    for d in v["datasets"].as_sequence().unwrap() {
        for e in d["custom_extensions"].as_sequence().unwrap() {
            assert_eq!(e["vendor_name"], VENDOR);
            let data = e["data"].as_str().expect("data is a string");
            serde_json::from_str::<serde_json::Value>(data).expect("data is JSON");
            seen += 1;
        }
    }
    assert_eq!(seen, 8);
}

#[test]
fn unknown_vendor_extensions_are_kept() {
    let mut m = read(STAND_IN).unwrap();
    m.relationships[0].other_extensions = vec![brightfield_ossie::Extension {
        vendor_name: "DBT".into(),
        data: "{\"ref\":\"stg_customers\"}".into(),
    }];
    let out = write(&m);
    assert!(validate(&out).is_empty());
    let back = read(&out).unwrap();
    assert_eq!(back, m);
    assert_eq!(write(&back), out);
}

#[test]
fn bad_yaml_names_the_line() {
    let err = read("datasets: [").unwrap_err();
    assert_eq!(err.line, 1);
}

#[test]
fn schema_violations_are_reported() {
    let issues = validate("version: 9.9\nname: x\ndatasets: []\n");
    assert!(issues.len() >= 2, "{issues:?}");
}

#[test]
fn stand_in_is_in_the_form_write_emits() {
    assert_eq!(write(&read(STAND_IN).unwrap()), STAND_IN);
}

const HAND_WRITTEN: &str = "\
version: 0.2.0.dev0
name: shop
description: A small shop
ai_context: Ask about sales
datasets:
- name: sales
  source: sales.csv
  description: One row per sale
  unique_keys:
  - - sale_id
  fields:
  - name: sale_id
    expression:
      sql: sale_id
    label: Sale
    dimension:
      is_time: false
  - name: sold_at
    expression:
      dialects:
      - dialect: DUCKDB
        expression: CAST(sold_at AS DATE)
  metrics:
  - name: revenue
    expression:
      sql: SUM(sales.amount)
relationships:
- name: sales_shops
  from: sales
  to: shops
  from_columns:
  - shop_id
  to_columns:
  - shop_id
  ai_context: Each sale belongs to one shop
metrics:
- name: total_revenue
  expression:
    sql: SUM(sales.amount)
";

#[test]
fn unmodelled_content_and_dialects_round_trip() {
    let m = read(HAND_WRITTEN).unwrap();
    assert_eq!(m.description.as_deref(), Some("A small shop"));
    assert_eq!(m.metrics.len(), 1);
    assert_eq!(m.metrics[0].name, "total_revenue");
    let sales = dataset(&m, "sales");
    assert_eq!(sales.metrics[0].name, "revenue");
    let sold_at = &sales.fields[1];
    assert!(sold_at.dialects_only);
    assert_eq!(sold_at.expression_sql, "CAST(sold_at AS DATE)");

    let out = write(&m);
    assert_eq!(validate(&out), vec![]);
    assert!(
        !out.contains("sql: CAST"),
        "dialect relabelled as sql: {out}"
    );
    let again = read(&out).unwrap();
    assert_eq!(again, m);
    // Nothing the file said is lost: the same document, read as plain YAML, matches
    // once the Meridian entries the writer adds are set aside.
    let strip = |mut v: serde_yaml::Value| {
        fn walk(v: &mut serde_yaml::Value) {
            match v {
                serde_yaml::Value::Mapping(m) => {
                    m.remove("custom_extensions");
                    m.values_mut().for_each(walk);
                }
                serde_yaml::Value::Sequence(s) => s.iter_mut().for_each(walk),
                _ => {}
            }
        }
        walk(&mut v);
        v
    };
    let a: serde_yaml::Value = serde_yaml::from_str(HAND_WRITTEN).unwrap();
    let b: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
    assert_eq!(strip(a), strip(b));
    assert_eq!(write(&again), out);
}

#[test]
fn elements_without_the_extension_read_with_defaults() {
    use brightfield_ossie::{DecidedBy, Grade, Role, Verified};
    let m = read(HAND_WRITTEN).unwrap();
    let sales = dataset(&m, "sales");
    assert_eq!(sales.meta.role, Role::Unclear);
    assert_eq!(sales.meta.decided_by, DecidedBy::Analyst);
    assert_eq!(sales.meta.source_file, "sales.csv");
    assert_eq!(sales.meta.source_part, None);
    assert_eq!(sales.meta.rows, None);
    assert_eq!(sales.fields[0].meta.finetype, None);
    assert_eq!(sales.fields[0].meta.decided_by, DecidedBy::Analyst);
    let r = &m.relationships[0];
    assert_eq!(r.meta.grade, Grade::Possible);
    assert_eq!(r.meta.verified, Verified::Sample);
    assert_eq!(r.meta.decided_by, DecidedBy::Analyst);
    assert_eq!(r.meta.evidence.child_rows, 0);
}

#[test]
fn bad_extension_data_names_the_element_line() {
    let yaml = "\
version: 0.2.0.dev0
name: x
datasets:
- name: a
  source: a.csv
  custom_extensions:
  - vendor_name: MERIDIAN
    data: not json
";
    let err = read(yaml).unwrap_err();
    assert_eq!(err.line, 4, "{err}");
}

#[test]
fn non_finite_evidence_is_written_as_zero_and_reads_back() {
    let mut m = read(STAND_IN).unwrap();
    m.relationships[0].meta.evidence.value_overlap = f64::NAN;
    m.relationships[0].meta.evidence.name_similarity = f64::INFINITY;
    m.relationships[0].meta.evidence.child_rows = 0;
    let mut rejected = m.relationships[1].clone();
    rejected.meta.evidence.value_overlap = f64::NEG_INFINITY;
    m.meta.rejected.push(rejected);
    let back = read(&write(&m)).expect("writes a readable file");
    assert_eq!(back.relationships[0].meta.evidence.value_overlap, 0.0);
    assert_eq!(back.relationships[0].meta.evidence.name_similarity, 0.0);
    assert_eq!(back.relationships[0].meta.evidence.child_rows, 0);
    assert_eq!(back.meta.rejected[0].meta.evidence.value_overlap, 0.0);
}
