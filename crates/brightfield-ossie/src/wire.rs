//! The YAML shape of an Ossie document, and the conversion to and from [`Model`].
//!
//! Struct field order here is the key order `write` emits, so output is deterministic.

use serde::{Deserialize, Serialize};

use crate::meridian;
use crate::model::{
    Dataset, DatasetMeta, Datatype, DecidedBy, Dialect, Evidence, Extension, Extras, Field,
    FieldMeta, Grade, Metric, Model, ModelMeta, Relationship, RelationshipMeta, Role, Verified,
};
use crate::OSSIE_VERSION;

/// A document that could not be read. `line` and `column` are 1-based, and 0 when
/// the problem is in a value's meaning rather than at a place in the text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message} (line {line}, column {column})")]
pub struct ReadError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Serialize, Deserialize)]
struct WireModel {
    version: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    datasets: Vec<WireDataset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    relationships: Vec<WireRelationship>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    metrics: Vec<Metric>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    custom_extensions: Vec<Extension>,
    #[serde(flatten)]
    extras: Extras,
}

#[derive(Serialize, Deserialize)]
struct WireDataset {
    name: String,
    source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    primary_key: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    fields: Vec<WireField>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    metrics: Vec<Metric>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    custom_extensions: Vec<Extension>,
    #[serde(flatten)]
    extras: Extras,
}

#[derive(Serialize, Deserialize)]
struct WireField {
    name: String,
    expression: WireExpression,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    datatype: Option<Datatype>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    custom_extensions: Vec<Extension>,
    #[serde(flatten)]
    extras: Extras,
}

#[derive(Serialize, Deserialize)]
struct WireExpression {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sql: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    dialects: Vec<Dialect>,
}

#[derive(Serialize, Deserialize)]
struct WireRelationship {
    name: String,
    from: String,
    to: String,
    from_columns: Vec<String>,
    to_columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    custom_extensions: Vec<Extension>,
    #[serde(flatten)]
    extras: Extras,
}

/// Where in the document a problem sits, by path rather than by bare name, so
/// two datasets that share a field name are told apart.
enum At {
    Model,
    Dataset(String),
    Field(String, String),
    Relationship(String),
}

/// A problem in an element's meaning, and the element it is in.
struct Bad {
    message: String,
    at: At,
}

fn bad(at: At) -> impl FnOnce(String) -> Bad {
    move |message| Bad { message, at }
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn is_blank(line: &str) -> bool {
    let t = line.trim();
    t.is_empty() || t.starts_with('#')
}

/// A scalar as written: trailing comment dropped, one layer of quotes removed.
fn scalar(raw: &str) -> String {
    let raw = raw.trim();
    let raw = match raw.find(" #") {
        Some(i) if !raw.starts_with(['\'', '"']) => &raw[..i],
        _ => raw,
    };
    let raw = raw.trim();
    // A quoted scalar may be followed by a comment after the closing quote.
    for q in ['\'', '"'] {
        if let Some(rest) = raw.strip_prefix(q) {
            if let Some(end) = rest.find(q) {
                return rest[..end].to_string();
            }
        }
    }
    raw.to_string()
}

/// The lines after `key_line` that belong to its value: up to the next line that
/// is at or left of the key's indent (a `- ` item at the key's own indent still belongs).
fn block_end(lines: &[&str], key_line: usize) -> usize {
    let ki = indent(lines[key_line]);
    (key_line + 1..lines.len())
        .find(|&i| {
            let l = lines[i];
            !is_blank(l)
                && (indent(l) < ki || (indent(l) == ki && !l.trim_start().starts_with("- ")))
        })
        .unwrap_or(lines.len())
}

/// The line of `key:` at exactly `at_indent` within `range`, if any.
fn find_key(
    lines: &[&str],
    range: std::ops::Range<usize>,
    at_indent: usize,
    key: &str,
) -> Option<usize> {
    let want = format!("{key}:");
    range.into_iter().find(|&i| {
        let l = lines[i];
        !is_blank(l) && indent(l) == at_indent && l.trim_start().starts_with(&want)
    })
}

/// Each item of the sequence in `range` whose `name` is `name`: its line range
/// and the line and 0-based column of its `name:` key.
fn find_item(
    lines: &[&str],
    range: std::ops::Range<usize>,
    name: &str,
) -> Option<(std::ops::Range<usize>, usize, usize)> {
    let seq = range
        .clone()
        .find(|&i| !is_blank(lines[i]) && lines[i].trim_start().starts_with("- "))
        .map(|i| indent(lines[i]))?;
    let starts: Vec<usize> = range
        .clone()
        .filter(|&i| {
            !is_blank(lines[i])
                && indent(lines[i]) == seq
                && lines[i].trim_start().starts_with("- ")
        })
        .collect();
    for (k, &st) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(range.end);
        for (i, l) in lines.iter().enumerate().take(end).skip(st) {
            let l = *l;
            let (col, body) = if i == st {
                (seq + 2, l.trim_start().trim_start_matches("- "))
            } else {
                (indent(l), l.trim_start())
            };
            if col == seq + 2 && !is_blank(l) {
                if let Some(v) = body.strip_prefix("name:") {
                    if scalar(v) == name {
                        return Some((st..end, i, col));
                    }
                    break;
                }
            }
        }
    }
    None
}

/// Line and column (1-based) of the element, or (0, 0) when the text has no such element.
fn locate(yaml: &str, at: &At) -> (usize, usize) {
    let lines: Vec<&str> = yaml.lines().collect();
    let all = 0..lines.len();
    let found = (|| match at {
        At::Model => {
            let i = find_key(&lines, all, 0, "name")?;
            Some((i, 0))
        }
        At::Relationship(r) => {
            let k = find_key(&lines, all, 0, "relationships")?;
            let (_, i, col) = find_item(&lines, k + 1..block_end(&lines, k), r)?;
            Some((i, col))
        }
        At::Dataset(d) | At::Field(d, _) => {
            let k = find_key(&lines, all, 0, "datasets")?;
            let (dr, i, col) = find_item(&lines, k + 1..block_end(&lines, k), d)?;
            let At::Field(_, f) = at else {
                return Some((i, col));
            };
            let fk = find_key(&lines, dr.clone(), col, "fields")?;
            let end = block_end(&lines, fk).min(dr.end);
            let (_, fi, fcol) = find_item(&lines, fk + 1..end, f)?;
            Some((fi, fcol))
        }
    })();
    found.map_or((0, 0), |(i, col)| (i + 1, col + 1))
}

/// Parse a `model.ossie.yaml`. An element with no Meridian extension is read
/// with default metadata (see the `default_*_meta` functions); the document's
/// version is checked by [`crate::validate`], not here.
pub fn read(yaml: &str) -> Result<Model, ReadError> {
    let w: WireModel = serde_yaml::from_str(yaml).map_err(|e| {
        let loc = e.location();
        // A document cut off mid-value is reported at the end of input, one line
        // past the text; point at its last line instead.
        let last = yaml.lines().count().max(1);
        ReadError {
            message: e.to_string(),
            line: loc.as_ref().map_or(0, |l| l.line().min(last)),
            column: loc.as_ref().map_or(0, |l| l.column()),
        }
    })?;
    from_wire(w).map_err(|b| {
        let (line, column) = locate(yaml, &b.at);
        ReadError {
            message: b.message,
            line,
            column,
        }
    })
}

fn default_model_meta() -> ModelMeta {
    ModelMeta {
        generator: "hand-written".to_string(),
        rejected: Vec::new(),
        skipped_sources: Vec::new(),
    }
}

fn default_dataset_meta(source: &str) -> DatasetMeta {
    DatasetMeta {
        role: Role::Unclear,
        role_evidence: Vec::new(),
        folded_into: None,
        excluded: false,
        decided_by: DecidedBy::Analyst,
        source_file: source.to_string(),
        source_part: None,
        rows: None,
    }
}

fn default_field_meta() -> FieldMeta {
    FieldMeta {
        finetype: None,
        decided_by: DecidedBy::Analyst,
    }
}

fn default_relationship_meta() -> RelationshipMeta {
    RelationshipMeta {
        grade: Grade::Possible,
        evidence: Evidence {
            value_overlap: 0.0,
            orphans: 0,
            child_rows: 0,
            child_type: None,
            parent_type: None,
            types_agree: false,
            name_similarity: 0.0,
        },
        cardinality: "N:1".to_string(),
        verified: Verified::Unmeasured,
        decided_by: DecidedBy::Analyst,
        warning: None,
    }
}

fn from_wire(w: WireModel) -> Result<Model, Bad> {
    let (meta, other_extensions) =
        meridian::split::<ModelMeta>(w.custom_extensions, "model", default_model_meta)
            .map_err(bad(At::Model))?;
    let datasets = w
        .datasets
        .into_iter()
        .map(dataset_from_wire)
        .collect::<Result<_, _>>()?;
    let relationships = w
        .relationships
        .into_iter()
        .map(relationship_from_wire)
        .collect::<Result<_, _>>()?;
    Ok(Model {
        name: w.name,
        description: w.description,
        metrics: w.metrics,
        extras: w.extras,
        datasets,
        relationships,
        meta,
        other_extensions,
    })
}

fn dataset_from_wire(d: WireDataset) -> Result<Dataset, Bad> {
    let (meta, other_extensions) =
        meridian::split::<DatasetMeta>(d.custom_extensions, &format!("dataset {}", d.name), || {
            default_dataset_meta(&d.source)
        })
        .map_err(bad(At::Dataset(d.name.clone())))?;
    let fields = d
        .fields
        .into_iter()
        .map(|f| field_from_wire(&d.name, f))
        .collect::<Result<_, _>>()?;
    Ok(Dataset {
        name: d.name,
        description: d.description,
        metrics: d.metrics,
        extras: d.extras,
        source: d.source,
        primary_key: d.primary_key,
        fields,
        meta,
        other_extensions,
    })
}

/// The dialect entry that stands for the expression when no `sql` is written.
fn preferred(dialects: &[Dialect]) -> Option<&Dialect> {
    ["OSSIE_SQL_2026", "ANSI_SQL", "DUCKDB"]
        .iter()
        .find_map(|want| {
            dialects
                .iter()
                .find(|d| d.dialect.eq_ignore_ascii_case(want))
        })
        .or_else(|| dialects.first())
}

fn field_from_wire(dataset: &str, f: WireField) -> Result<Field, Bad> {
    let what = format!("field {dataset}.{}", f.name);
    let (meta, other_extensions) =
        meridian::split::<FieldMeta>(f.custom_extensions, &what, default_field_meta)
            .map_err(bad(At::Field(dataset.to_string(), f.name.clone())))?;
    let dialects_only = f.expression.sql.is_none();
    let expression_sql = match f.expression.sql {
        Some(sql) => sql,
        None => preferred(&f.expression.dialects)
            .map(|d| d.expression.clone())
            .ok_or_else(|| Bad {
                message: format!("{what}: no expression"),
                at: At::Field(dataset.to_string(), f.name.clone()),
            })?,
    };
    Ok(Field {
        name: f.name,
        expression_sql,
        dialects: f.expression.dialects,
        dialects_only,
        extras: f.extras,
        datatype: f.datatype,
        meta,
        other_extensions,
    })
}

fn relationship_from_wire(r: WireRelationship) -> Result<Relationship, Bad> {
    let (meta, other_extensions) = meridian::split::<RelationshipMeta>(
        r.custom_extensions,
        &format!("relationship {}", r.name),
        default_relationship_meta,
    )
    .map_err(bad(At::Relationship(r.name.clone())))?;
    Ok(Relationship {
        name: r.name,
        from: r.from,
        to: r.to,
        from_columns: r.from_columns,
        to_columns: r.to_columns,
        meta,
        extras: r.extras,
        other_extensions,
    })
}

/// Serialise a model as an Ossie document. Keys come out in a fixed order, so
/// the same model always gives the same bytes. A non-finite evidence number
/// (NaN or infinite) is written as 0.0, since YAML and JSON cannot carry it
/// and a null would make the file unreadable.
pub fn write(model: &Model) -> String {
    let mut meta = model.meta.clone();
    meta.rejected = meta.rejected.iter().map(finite_relationship).collect();
    let w = WireModel {
        version: OSSIE_VERSION.to_string(),
        name: model.name.clone(),
        description: model.description.clone(),
        datasets: model.datasets.iter().map(dataset_to_wire).collect(),
        relationships: model
            .relationships
            .iter()
            .map(|r| relationship_to_wire(&finite_relationship(r)))
            .collect(),
        metrics: model.metrics.clone(),
        custom_extensions: meridian::join(&meta, &model.other_extensions),
        extras: model.extras.clone(),
    };
    serde_yaml::to_string(&w).expect("a model serialises to YAML")
}

fn finite_relationship(r: &Relationship) -> Relationship {
    let mut r = r.clone();
    r.extras = r
        .extras
        .into_iter()
        .map(|(k, v)| (k, string_keys(v)))
        .collect();
    let e = &mut r.meta.evidence;
    for v in [&mut e.value_overlap, &mut e.name_similarity] {
        if !v.is_finite() {
            *v = 0.0;
        }
    }
    r
}

/// `v` with every mapping key as a string (a non-string key becomes its YAML
/// text), since the `rejected` list is stored as JSON, which has only string keys.
fn string_keys(v: serde_yaml::Value) -> serde_yaml::Value {
    use serde_yaml::Value;
    match v {
        Value::Mapping(m) => Value::Mapping(
            m.into_iter()
                .map(|(k, v)| {
                    let k = match k {
                        Value::String(s) => s,
                        other => serde_yaml::to_string(&other)
                            .map(|s| s.trim().to_string())
                            .unwrap_or_default(),
                    };
                    (Value::String(k), string_keys(v))
                })
                .collect(),
        ),
        Value::Sequence(s) => Value::Sequence(s.into_iter().map(string_keys).collect()),
        Value::Tagged(t) => Value::Tagged(Box::new(serde_yaml::value::TaggedValue {
            tag: t.tag,
            value: string_keys(t.value),
        })),
        other => other,
    }
}

fn dataset_to_wire(d: &Dataset) -> WireDataset {
    WireDataset {
        name: d.name.clone(),
        source: d.source.clone(),
        description: d.description.clone(),
        primary_key: d.primary_key.clone(),
        fields: d
            .fields
            .iter()
            .map(|f| WireField {
                name: f.name.clone(),
                expression: WireExpression {
                    sql: (!f.dialects_only).then(|| f.expression_sql.clone()),
                    dialects: f.dialects.clone(),
                },
                datatype: f.datatype,
                custom_extensions: meridian::join(&f.meta, &f.other_extensions),
                extras: f.extras.clone(),
            })
            .collect(),
        metrics: d.metrics.clone(),
        custom_extensions: meridian::join(&d.meta, &d.other_extensions),
        extras: d.extras.clone(),
    }
}

fn relationship_to_wire(r: &Relationship) -> WireRelationship {
    WireRelationship {
        name: r.name.clone(),
        from: r.from.clone(),
        to: r.to.clone(),
        from_columns: r.from_columns.clone(),
        to_columns: r.to_columns.clone(),
        custom_extensions: meridian::join(&r.meta, &r.other_extensions),
        extras: r.extras.clone(),
    }
}
