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
