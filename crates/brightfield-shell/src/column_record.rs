//! What a source taught Brightfield about its columns, kept on the `datapackage.json` fields.
//!
//! A field carries FineType's label (`x-finetype-label`) and whether an analyst accepted it
//! (`x-finetype-nominated`). Everything Brightfield adds sits in one object, `x-brightfield`:
//! `written` is how the source wrote the column (the currency symbol its values share, whether
//! its negatives are in parentheses) and `prefer` is the analyst's preferred number and date
//! format. A resource carries `x-brightfield-source`, the size and modification time of the file
//! the record was derived from, so an edited file is noticed.
//!
//! The package is edited as JSON, so a property this module does not model (such as
//! `x-finetype-confidence`) survives a write.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

const LABEL: &str = "x-finetype-label";
const NOMINATED: &str = "x-finetype-nominated";
const BRIGHTFIELD: &str = "x-brightfield";
const SOURCE: &str = "x-brightfield-source";

/// How a negative amount is written in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Negatives {
    Minus,
    Parentheses,
}

/// What the source was written with, read from the raw values when a column is cast.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Written {
    /// The symbol every non-null value shares; none when they use more than one.
    pub currency_symbol: Option<String>,
    pub negatives: Option<Negatives>,
}

/// An analyst's preferred representation of a column, over FineType's default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prefer {
    pub number_format: Option<String>,
    pub date_format: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldRecord {
    /// `x-finetype-label`.
    pub label: Option<String>,
    /// `x-finetype-nominated`: the label was accepted by an analyst and is taken as given.
    pub nominated: bool,
    pub written: Written,
    pub prefer: Prefer,
}

/// The size and modification time of the file a record was derived from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceStamp {
    pub size: u64,
    /// Nanoseconds since the Unix epoch.
    pub modified: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceRecord {
    pub fields: BTreeMap<String, FieldRecord>,
    /// The file the record was derived from, when it has been stamped.
    pub source: Option<SourceStamp>,
}

impl FieldRecord {
    fn is_empty(&self) -> bool {
        *self == FieldRecord::default()
    }
}

/// The record of `resource` in the `datapackage.json` at `package`: `None` when the file or the
/// resource is not there. An absent `x-brightfield` reads as empty `Written` and `Prefer`.
///
/// # Errors
///
/// The reason when the package cannot be read or is not JSON.
pub fn read_record(package: &Path, resource: &str) -> Result<Option<SourceRecord>, String> {
    let Some(doc) = read_package(package)? else {
        return Ok(None);
    };
    let Some(res) = doc["resources"]
        .as_array()
        .and_then(|rs| rs.iter().find(|r| r["name"] == resource))
    else {
        return Ok(None);
    };
    let mut record = SourceRecord {
        source: res.get(SOURCE).and_then(|s| {
            Some(SourceStamp {
                size: s.get("size")?.as_u64()?,
                modified: s.get("modified")?.as_u64()?,
            })
        }),
        ..SourceRecord::default()
    };
    for field in res["schema"]["fields"].as_array().into_iter().flatten() {
        let Some(name) = field["name"].as_str() else {
            continue;
        };
        record.fields.insert(name.to_string(), read_field(field));
    }
    Ok(Some(record))
}

fn read_field(field: &Value) -> FieldRecord {
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    let own = field.get(BRIGHTFIELD).unwrap_or(&Value::Null);
    let written = own.get("written").unwrap_or(&Value::Null);
    let prefer = own.get("prefer").unwrap_or(&Value::Null);
    FieldRecord {
        label: text(field, LABEL),
        nominated: field.get(NOMINATED).and_then(Value::as_bool) == Some(true),
        written: Written {
            currency_symbol: text(written, "currency_symbol"),
            negatives: match written.get("negatives").and_then(Value::as_str) {
                Some("minus") => Some(Negatives::Minus),
                Some("parentheses") => Some(Negatives::Parentheses),
                _ => None,
            },
        },
        prefer: Prefer {
            number_format: text(prefer, "number_format"),
            date_format: text(prefer, "date_format"),
        },
    }
}

/// Write `record` onto `resource` in the `datapackage.json` at `package`, creating the package,
/// the resource and its fields when they are not there. Every other property of the package is
/// left as it was.
///
/// # Errors
///
/// The reason when the package cannot be read, is not JSON, or cannot be written.
pub fn write_record(package: &Path, resource: &str, record: &SourceRecord) -> Result<(), String> {
    let mut doc = read_package(package)?.unwrap_or_else(|| {
        json!({
            "$schema": "https://datapackage.org/profiles/2.0/datapackage.json",
            "resources": [],
        })
    });
    check_shape(&doc, package)?;
    let resources = array_at(&mut doc, "resources")?;
    let at = match resources.iter().position(|r| r["name"] == resource) {
        Some(at) => at,
        None => {
            resources.push(json!({ "name": resource, "schema": { "fields": [] } }));
            resources.len() - 1
        }
    };
    let res = &mut resources[at];
    match record.source {
        Some(s) => res[SOURCE] = json!({ "size": s.size, "modified": s.modified }),
        None => {
            if let Some(o) = res.as_object_mut() {
                o.remove(SOURCE);
            }
        }
    }
    if !res["schema"].is_object() {
        res["schema"] = json!({});
    }
    let fields = array_at(&mut res["schema"], "fields")?;
    for (name, field_record) in &record.fields {
        let at = match fields.iter().position(|f| f["name"] == name.as_str()) {
            Some(at) => at,
            None if field_record.is_empty() => continue,
            None => {
                fields.push(json!({ "name": name }));
                fields.len() - 1
            }
        };
        write_field(&mut fields[at], field_record);
    }
    let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    if let Some(dir) = package.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    // Written beside the package and renamed over it, so a reader never sees half a file.
    let partial = package.with_extension("json.partial");
    std::fs::write(&partial, text + "\n").map_err(|e| format!("{}: {e}", partial.display()))?;
    std::fs::rename(&partial, package).map_err(|e| format!("{}: {e}", package.display()))
}

/// Refuse a package whose shape this module cannot edit, naming what is wrong, so an edit never
/// has to index into a value of the wrong kind.
fn check_shape(doc: &Value, package: &Path) -> Result<(), String> {
    let wrong = |what: &str| Err(format!("{}: {what}", package.display()));
    let Some(root) = doc.as_object() else {
        return wrong("the package is not a JSON object");
    };
    let Some(resources) = root.get("resources") else {
        return Ok(());
    };
    let Some(resources) = resources.as_array() else {
        return wrong("`resources` is not a list");
    };
    for res in resources {
        let Some(res) = res.as_object() else {
            return wrong("a resource is not an object");
        };
        match res.get("schema") {
            None => {}
            Some(Value::Object(schema)) => match schema.get("fields") {
                None => {}
                Some(Value::Array(_)) => {}
                Some(_) => return wrong("a resource's `fields` is not a list"),
            },
            // A schema of the wrong kind is replaced by an empty one on write.
            Some(_) => {}
        }
    }
    Ok(())
}

fn array_at<'a>(parent: &'a mut Value, key: &str) -> Result<&'a mut Vec<Value>, String> {
    if parent.get(key).is_none() {
        parent[key] = json!([]);
    }
    parent[key]
        .as_array_mut()
        .ok_or_else(|| format!("`{key}` is not a list"))
}

/// Set `key` on `object` to `value`, or remove it when there is none.
fn set(object: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    match value {
        Some(v) => {
            object.insert(key.to_string(), v);
        }
        None => {
            object.remove(key);
        }
    }
}

fn write_field(field: &mut Value, record: &FieldRecord) {
    let Some(object) = field.as_object_mut() else {
        return;
    };
    set(object, LABEL, record.label.clone().map(Value::String));
    set(
        object,
        NOMINATED,
        record.nominated.then_some(Value::Bool(true)),
    );
    let mut own = match object.remove(BRIGHTFIELD) {
        Some(Value::Object(o)) => o,
        _ => Map::new(),
    };
    let section =
        |own: &mut Map<String, Value>, key: &str, entries: [(&str, Option<String>); 2]| {
            let mut section = match own.remove(key) {
                Some(Value::Object(o)) => o,
                _ => Map::new(),
            };
            for (name, value) in entries {
                set(&mut section, name, value.map(Value::String));
            }
            set(
                own,
                key,
                (!section.is_empty()).then_some(Value::Object(section)),
            );
        };
    section(
        &mut own,
        "written",
        [
            ("currency_symbol", record.written.currency_symbol.clone()),
            (
                "negatives",
                record.written.negatives.map(|n| {
                    match n {
                        Negatives::Minus => "minus",
                        Negatives::Parentheses => "parentheses",
                    }
                    .to_string()
                }),
            ),
        ],
    );
    section(
        &mut own,
        "prefer",
        [
            ("number_format", record.prefer.number_format.clone()),
            ("date_format", record.prefer.date_format.clone()),
        ],
    );
    set(
        object,
        BRIGHTFIELD,
        (!own.is_empty()).then_some(Value::Object(own)),
    );
}

fn read_package(package: &Path) -> Result<Option<Value>, String> {
    let text = match std::fs::read_to_string(package) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", package.display())),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("{}: {e}", package.display()))
}

/// `fresh`, a package built from a profile, with the `x-brightfield` objects and
/// `x-brightfield-source` stamps of the package already at `existing` put back on the resources
/// and fields of the same names. Nothing else of the old package is kept: the profile's reading
/// of the columns is the new one.
///
/// A field the analyst nominated keeps its nominated marker and its recorded label, not the
/// profile's.
pub(crate) fn carry_over(mut fresh: Value, existing: &Path) -> Value {
    // A package that is missing, does not parse or has the wrong shape carries nothing over: the
    // fresh one replaces it, as a write always did before records were kept.
    let Some(old) = read_package(existing)
        .ok()
        .flatten()
        .filter(|old| check_shape(old, existing).is_ok())
    else {
        return fresh;
    };
    let old_resources = old["resources"].as_array().cloned().unwrap_or_default();
    for res in fresh["resources"].as_array_mut().into_iter().flatten() {
        let Some(old_res) = old_resources.iter().find(|r| r["name"] == res["name"]) else {
            continue;
        };
        if let Some(stamp) = old_res.get(SOURCE) {
            res[SOURCE] = stamp.clone();
        }
        for field in res["schema"]["fields"].as_array_mut().into_iter().flatten() {
            let Some(old_field) = old_res["schema"]["fields"]
                .as_array()
                .and_then(|fs| fs.iter().find(|f| f["name"] == field["name"]))
            else {
                continue;
            };
            if let Some(kept) = old_field.get(BRIGHTFIELD) {
                field[BRIGHTFIELD] = kept.clone();
            }
            if old_field.get(NOMINATED).and_then(Value::as_bool) == Some(true) {
                field[NOMINATED] = Value::Bool(true);
                if let Some(label) = old_field.get(LABEL) {
                    field[LABEL] = label.clone();
                }
            }
        }
    }
    fresh
}

/// Where a lone file's record lives: `records/<key>/datapackage.json` under Brightfield's data
/// folder, where `<key>` hashes the file's canonical path and nothing else, so an edited file
/// keeps its record.
///
/// # Errors
///
/// The reason when the file's path cannot be resolved.
pub fn flat_record_path(file: &Path) -> Result<PathBuf, String> {
    record_path_under(&crate::starts::datasets_dir(), file)
}

fn record_path_under(datasets: &Path, file: &Path) -> Result<PathBuf, String> {
    let canonical = file
        .canonicalize()
        .map_err(|e| format!("{}: {e}", file.display()))?;
    // SHA-256 rather than std's hasher, whose output is not stable across Rust releases.
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    let key: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    Ok(datasets.join("records").join(key).join("datapackage.json"))
}

/// The size and modification time of `file` now.
///
/// # Errors
///
/// The reason when the file cannot be read.
pub fn stamp(file: &Path) -> Result<SourceStamp, String> {
    let meta = std::fs::metadata(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let modified = meta
        .modified()
        .map_err(|e| format!("{}: {e}", file.display()))?
        // A time before the epoch is taken as the epoch; it still differs from any later edit.
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Ok(SourceStamp {
        size: meta.len(),
        modified: u64::try_from(modified.as_nanos()).unwrap_or(u64::MAX),
    })
}

/// Compare the record's stamp with `file` now. A record never stamped takes the file's stamp
/// and is left as it is. When the size or modification time differs, the inferred labels and
/// `written` are cleared (the file is read afresh) while `nominated` labels and `prefer` are
/// kept, and the record takes the new stamp. True when it differed.
///
/// # Errors
///
/// The reason when the file cannot be read.
pub fn refresh_if_changed(record: &mut SourceRecord, file: &Path) -> Result<bool, String> {
    let now = stamp(file)?;
    let before = record.source.replace(now);
    if before.is_none_or(|b| b == now) {
        return Ok(false);
    }
    for field in record.fields.values_mut() {
        if !field.nominated {
            field.label = None;
        }
        field.written = Written::default();
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn revenue() -> SourceRecord {
        let mut record = SourceRecord::default();
        record.fields.insert(
            "revenue".into(),
            FieldRecord {
                label: Some("finance.currency.amount".into()),
                nominated: false,
                written: Written {
                    currency_symbol: Some("£".into()),
                    negatives: Some(Negatives::Parentheses),
                },
                prefer: Prefer {
                    number_format: Some(",.0f".into()),
                    date_format: None,
                },
            },
        );
        record
    }

    #[test]
    fn record_round_trips_the_spec_example() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("datapackage.json");
        write_record(&path, "sales", &revenue()).unwrap();
        let got = read_record(&path, "sales").unwrap().unwrap();
        let f = &got.fields["revenue"];
        assert_eq!(f.written.currency_symbol.as_deref(), Some("£"));
        assert_eq!(f.written.negatives, Some(Negatives::Parentheses));
        assert_eq!(f.prefer.number_format.as_deref(), Some(",.0f"));
        assert_eq!(f.label.as_deref(), Some("finance.currency.amount"));
        assert!(!f.nominated);
        // The file says it the way the spec does.
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            doc["resources"][0]["schema"]["fields"][0]["x-brightfield"],
            json!({
                "written": { "currency_symbol": "£", "negatives": "parentheses" },
                "prefer": { "number_format": ",.0f" },
            })
        );
        assert_eq!(read_record(&path, "other").unwrap(), None);
        assert_eq!(
            read_record(&dir.path().join("none.json"), "sales").unwrap(),
            None
        );
    }

    #[test]
    fn absent_x_brightfield_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("datapackage.json");
        std::fs::write(
            &path,
            r#"{"resources":[{"name":"t","schema":{"fields":[{"name":"a","x-finetype-label":"x.y"}]}}]}"#,
        )
        .unwrap();
        let got = read_record(&path, "t").unwrap().unwrap();
        assert_eq!(got.fields["a"].label.as_deref(), Some("x.y"));
        assert_eq!(got.fields["a"].written, Written::default());
        assert_eq!(got.fields["a"].prefer, Prefer::default());
    }

    #[test]
    fn record_keeps_other_properties() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("datapackage.json");
        std::fs::write(
            &path,
            r#"{"name":"p","resources":[{"name":"t","path":"t.csv","schema":{"fields":[
                {"name":"revenue","type":"number","x-finetype-confidence":0.93}]}}]}"#,
        )
        .unwrap();
        write_record(&path, "t", &revenue()).unwrap();
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let field = &doc["resources"][0]["schema"]["fields"][0];
        assert_eq!(field["x-finetype-confidence"], 0.93);
        assert_eq!(field["type"], "number");
        assert_eq!(field["x-finetype-label"], "finance.currency.amount");
        assert_eq!(doc["name"], "p");
        assert_eq!(doc["resources"][0]["path"], "t.csv");
    }

    #[test]
    fn edited_file_keeps_choices() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sales.csv");
        std::fs::write(&file, "revenue\n(1)\n").unwrap();
        let datasets = dir.path().join("datasets");
        let before = record_path_under(&datasets, &file).unwrap();

        let mut record = revenue();
        record.fields.insert(
            "region".into(),
            FieldRecord {
                label: Some("geography.location.region".into()),
                nominated: true,
                ..FieldRecord::default()
            },
        );
        assert!(
            !refresh_if_changed(&mut record, &file).unwrap(),
            "first stamp"
        );
        write_record(&before, "sales", &record).unwrap();
        let mut record = read_record(&before, "sales").unwrap().unwrap();
        assert!(
            !refresh_if_changed(&mut record, &file).unwrap(),
            "unchanged"
        );

        let handle = std::fs::File::options().write(true).open(&file).unwrap();
        handle
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60))
            .unwrap();
        drop(handle);

        assert_eq!(record_path_under(&datasets, &file).unwrap(), before);
        assert!(refresh_if_changed(&mut record, &file).unwrap());
        let revenue = &record.fields["revenue"];
        assert_eq!(revenue.label, None);
        assert_eq!(revenue.written, Written::default());
        assert_eq!(revenue.prefer.number_format.as_deref(), Some(",.0f"));
        let region = &record.fields["region"];
        assert_eq!(region.label.as_deref(), Some("geography.location.region"));
        assert!(region.nominated);
        assert!(
            !refresh_if_changed(&mut record, &file).unwrap(),
            "restamped"
        );
    }

    #[test]
    fn write_refuses_a_package_of_the_wrong_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("datapackage.json");
        for (text, what) in [
            ("[1]", "not a JSON object"),
            (r#"{"resources":{}}"#, "`resources` is not a list"),
            (r#"{"resources":[3]}"#, "a resource is not an object"),
            (
                r#"{"resources":[{"name":"t","schema":{"fields":{}}}]}"#,
                "`fields` is not a list",
            ),
        ] {
            std::fs::write(&path, text).unwrap();
            let err = write_record(&path, "t", &revenue()).unwrap_err();
            assert!(err.contains(what), "{err}");
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                text,
                "left as it was"
            );
        }
    }

    #[test]
    fn carry_over_keeps_nominations_and_drops_a_broken_package() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("datapackage.json");
        std::fs::write(
            &path,
            r#"{"resources":[{"name":"t","schema":{"fields":[
                {"name":"a","x-finetype-label":"mine.label","x-finetype-nominated":true},
                {"name":"b","x-finetype-label":"old.label"}]}}]}"#,
        )
        .unwrap();
        let fresh = || {
            json!({"resources":[{"name":"t","schema":{"fields":[
                {"name":"a","x-finetype-label":"profile.label"},
                {"name":"b","x-finetype-label":"profile.label"}]}}]})
        };
        let got = carry_over(fresh(), &path);
        let fields = &got["resources"][0]["schema"]["fields"];
        assert_eq!(fields[0]["x-finetype-label"], "mine.label");
        assert_eq!(fields[0]["x-finetype-nominated"], true);
        assert_eq!(fields[1]["x-finetype-label"], "profile.label");
        assert!(fields[1].get("x-finetype-nominated").is_none());
        for broken in ["{\"resources\": [", "[1]", r#"{"resources":{}}"#] {
            std::fs::write(&path, broken).unwrap();
            assert_eq!(carry_over(fresh(), &path), fresh(), "{broken}");
        }
    }

    #[test]
    fn key_follows_the_canonical_path_only() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.csv");
        let b = dir.path().join("b.csv");
        std::fs::write(&a, "x\n").unwrap();
        std::fs::write(&b, "x\n").unwrap();
        let datasets = dir.path().join("d");
        let via_dots = dir.path().join(".").join("a.csv");
        assert_eq!(
            record_path_under(&datasets, &a).unwrap(),
            record_path_under(&datasets, &via_dots).unwrap()
        );
        assert_ne!(
            record_path_under(&datasets, &a).unwrap(),
            record_path_under(&datasets, &b).unwrap()
        );
        assert!(record_path_under(&datasets, &a)
            .unwrap()
            .starts_with(datasets.join("records")));
    }
}
