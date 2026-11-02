//! What dovetail writes reads, validates and survives a rewrite, so the keys Brightfield has not
//! typed yet are not lost when it saves a model.

use brightfield_ossie::{read, validate, write, Combination, CombinationKind, Model, VENDOR};
use serde_json::Value;

const SAMPLE: &str = include_str!("../fixtures/dovetail/retail.sample.ossie.yaml");
const FULL: &str = include_str!("../fixtures/dovetail/retail.full.ossie.yaml");

fn dataset<'a>(m: &'a Model, name: &str) -> &'a brightfield_ossie::Dataset {
    m.datasets.iter().find(|d| d.name == name).unwrap()
}

/// Every Meridian `data` payload of the document, keyed by the element it sits on.
fn meridian_data(text: &str) -> Vec<(String, Value)> {
    let doc: serde_yaml::Value = serde_yaml::from_str(text).unwrap();
    let mut out = Vec::new();
    let mut take = |path: String, node: &serde_yaml::Value| {
        let exts = node.get("custom_extensions").and_then(|e| e.as_sequence());
        for e in exts.into_iter().flatten() {
            if e.get("vendor_name").and_then(|v| v.as_str()) == Some(VENDOR) {
                let data = e.get("data").and_then(|d| d.as_str()).unwrap();
                out.push((path.clone(), serde_json::from_str(data).unwrap()));
            }
        }
    };
    take("model".to_string(), &doc);
    for d in doc["datasets"].as_sequence().into_iter().flatten() {
        let dn = d["name"].as_str().unwrap().to_string();
        take(format!("dataset {dn}"), d);
        for f in d["fields"].as_sequence().into_iter().flatten() {
            take(format!("field {dn}.{}", f["name"].as_str().unwrap()), f);
        }
    }
    for r in doc["relationships"].as_sequence().into_iter().flatten() {
        take(format!("relationship {}", r["name"].as_str().unwrap()), r);
    }
    out
}

#[test]
fn dovetails_goldens_read_and_validate() {
    for text in [SAMPLE, FULL] {
        assert_eq!(validate(text), vec![]);
        read(text).expect("golden reads");
    }
}

#[test]
fn dovetails_new_keys_are_typed() {
    let m = read(SAMPLE).unwrap();
    assert_eq!(
        dataset(&m, "members").meta.combination,
        Some(Combination {
            kind: CombinationKind::Snapshot,
            parts: vec!["members_2024".to_string(), "members_2025".to_string()],
            token_column: Some("year".to_string()),
            other: serde_json::Map::new(),
        })
    );
    assert_eq!(
        dataset(&m, "members_2024").meta.combined_into.as_deref(),
        Some("members")
    );
    assert!(m
        .datasets
        .iter()
        .any(|d| d.meta.fold_column.as_deref() == Some("category")));
    assert!(m.datasets.iter().all(|d| d.meta.role_scores.is_some()));
    assert_eq!(m.meta.pass.as_deref(), Some("sample"));
    assert_eq!(m.meta.sample_rows, Some(50000));
}

#[test]
fn every_meridian_key_survives_a_round_trip() {
    for text in [SAMPLE, FULL] {
        let before = meridian_data(text);
        let after = meridian_data(&write(&read(text).unwrap()));
        assert!(!before.is_empty());
        for (path, value) in &before {
            let kept = after.iter().find(|(p, _)| p == path);
            assert_eq!(kept.map(|(_, v)| v), Some(value), "{path}");
        }
        assert_eq!(before.len(), after.len());
    }
}

#[test]
fn an_unknown_meridian_key_survives() {
    let marked = SAMPLE.replacen("\"rows\":8}", "\"rows\":8,\"zz_new\":1}", 1);
    assert_ne!(marked, SAMPLE);
    let out = write(&read(&marked).unwrap());
    let kept = meridian_data(&out)
        .into_iter()
        .any(|(_, v)| v.get("zz_new") == Some(&Value::from(1)));
    assert!(kept);
}

#[test]
fn values_a_newer_dovetail_writes_read_and_are_written_back() {
    let marked = SAMPLE
        .replacen(
            "\"combination\":{\"kind\":\"snapshot\",",
            "\"combination\":{\"kind\":\"merged\",\"zz_how\":\"by name\",",
            1,
        )
        .replacen("\"unclear\":0.0}", "\"unclear\":0.0,\"ledger\":0.4}", 1)
        .replacen(
            "\"name_similarity\":0.9}",
            "\"name_similarity\":0.9,\"zz_sampled\":true}",
            1,
        );
    for new in ["merged", "ledger", "zz_sampled"] {
        assert!(marked.contains(new), "{new} was not marked in");
    }
    let m = read(&marked).expect("a newer dovetail's values read");
    assert!(m.datasets.iter().any(|d| d
        .meta
        .combination
        .as_ref()
        .is_some_and(|c| c.kind == CombinationKind::Other("merged".to_string()))));

    let before = meridian_data(&marked);
    let after = meridian_data(&write(&m));
    for (path, value) in &before {
        let kept = after.iter().find(|(p, _)| p == path);
        assert_eq!(kept.map(|(_, v)| v), Some(value), "{path}");
    }
}

#[test]
fn role_scores_without_a_role_read_it_as_zero() {
    let marked = SAMPLE.replacen(",\"unclear\":0.0}", "}", 1);
    assert_ne!(marked, SAMPLE);
    let m = read(&marked).expect("scores without a role read");
    assert!(m
        .datasets
        .iter()
        .filter_map(|d| d.meta.role_scores.as_ref())
        .all(|s| s.unclear == 0.0));
}
