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

/// A problem in an element's meaning, with the element's name so the text can be searched for a line.
struct Bad {
    message: String,
    name: String,
}

fn bad(name: &str) -> impl Fn(String) -> Bad + '_ {
    move |message| Bad {
        message,
        name: name.to_string(),
    }
}

/// The line and column of the first `name: <name>` in `yaml`, or (0, 0).
fn locate(yaml: &str, name: &str) -> (usize, usize) {
    let key = format!("name: {name}");
    yaml.lines()
        .enumerate()
        .find_map(|(i, l)| {
            let t = l.trim_start();
            let t = t.strip_prefix("- ").unwrap_or(t);
            (t == key).then(|| (i + 1, l.len() - t.len() + 1))
        })
        .unwrap_or((0, 0))
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
        let (line, column) = locate(yaml, &b.name);
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
        verified: Verified::Sample,
        decided_by: DecidedBy::Analyst,
        warning: None,
    }
}

fn from_wire(w: WireModel) -> Result<Model, Bad> {
    let (meta, other_extensions) =
        meridian::split::<ModelMeta>(w.custom_extensions, "model", default_model_meta)
            .map_err(bad(&w.name))?;
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
        .map_err(bad(&d.name))?;
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
            .map_err(bad(&f.name))?;
    let dialects_only = f.expression.sql.is_none();
    let expression_sql = match f.expression.sql {
        Some(sql) => sql,
        None => preferred(&f.expression.dialects)
            .map(|d| d.expression.clone())
            .ok_or_else(|| Bad {
                message: format!("{what}: no expression"),
                name: f.name.clone(),
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
    .map_err(bad(&r.name))?;
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
    let e = &mut r.meta.evidence;
    for v in [&mut e.value_overlap, &mut e.name_similarity] {
        if !v.is_finite() {
            *v = 0.0;
        }
    }
    r
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
