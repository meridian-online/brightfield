//! The YAML shape of an Ossie document, and the conversion to and from [`Model`].
//!
//! Struct field order here is the key order `write` emits, so output is deterministic.

use serde::{Deserialize, Serialize};

use crate::meridian;
use crate::model::{
    Dataset, DatasetMeta, Datatype, Extension, Field, FieldMeta, Model, ModelMeta, Relationship,
    RelationshipMeta,
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
    datasets: Vec<WireDataset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    relationships: Vec<WireRelationship>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    custom_extensions: Vec<Extension>,
}

#[derive(Serialize, Deserialize)]
struct WireDataset {
    name: String,
    source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    primary_key: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    fields: Vec<WireField>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    custom_extensions: Vec<Extension>,
}

#[derive(Serialize, Deserialize)]
struct WireField {
    name: String,
    expression: WireExpression,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    datatype: Option<Datatype>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    custom_extensions: Vec<Extension>,
}

#[derive(Serialize, Deserialize)]
struct WireExpression {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sql: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    dialects: Vec<WireDialect>,
}

#[derive(Serialize, Deserialize)]
struct WireDialect {
    dialect: String,
    expression: String,
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
}

fn unlocated(message: String) -> ReadError {
    ReadError {
        message,
        line: 0,
        column: 0,
    }
}

/// Parse a `model.ossie.yaml`. Every element must carry Meridian's extension;
/// the document's version is checked by [`crate::validate`], not here.
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
    from_wire(w).map_err(unlocated)
}

fn from_wire(w: WireModel) -> Result<Model, String> {
    let (meta, other_extensions) = meridian::split::<ModelMeta>(w.custom_extensions, "model")?;
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
        datasets,
        relationships,
        meta,
        other_extensions,
    })
}

fn dataset_from_wire(d: WireDataset) -> Result<Dataset, String> {
    let (meta, other_extensions) =
        meridian::split::<DatasetMeta>(d.custom_extensions, &format!("dataset {}", d.name))?;
    let fields = d
        .fields
        .into_iter()
        .map(|f| field_from_wire(&d.name, f))
        .collect::<Result<_, _>>()?;
    Ok(Dataset {
        name: d.name,
        source: d.source,
        primary_key: d.primary_key,
        fields,
        meta,
        other_extensions,
    })
}

fn field_from_wire(dataset: &str, f: WireField) -> Result<Field, String> {
    let what = format!("field {dataset}.{}", f.name);
    let (meta, other_extensions) = meridian::split::<FieldMeta>(f.custom_extensions, &what)?;
    // `sql` is authoritative when present; otherwise the first dialect entry stands in.
    let expression_sql = f
        .expression
        .sql
        .or_else(|| {
            f.expression
                .dialects
                .into_iter()
                .next()
                .map(|d| d.expression)
        })
        .ok_or_else(|| format!("{what}: no expression"))?;
    Ok(Field {
        name: f.name,
        expression_sql,
        datatype: f.datatype,
        meta,
        other_extensions,
    })
}

fn relationship_from_wire(r: WireRelationship) -> Result<Relationship, String> {
    let (meta, other_extensions) = meridian::split::<RelationshipMeta>(
        r.custom_extensions,
        &format!("relationship {}", r.name),
    )?;
    Ok(Relationship {
        name: r.name,
        from: r.from,
        to: r.to,
        from_columns: r.from_columns,
        to_columns: r.to_columns,
        meta,
        other_extensions,
    })
}

/// Serialise a model as an Ossie document. Keys come out in a fixed order, so
/// the same model always gives the same bytes.
pub fn write(model: &Model) -> String {
    let w = WireModel {
        version: OSSIE_VERSION.to_string(),
        name: model.name.clone(),
        datasets: model.datasets.iter().map(dataset_to_wire).collect(),
        relationships: model
            .relationships
            .iter()
            .map(relationship_to_wire)
            .collect(),
        custom_extensions: meridian::join(&model.meta, &model.other_extensions),
    };
    serde_yaml::to_string(&w).expect("a model serialises to YAML")
}

fn dataset_to_wire(d: &Dataset) -> WireDataset {
    WireDataset {
        name: d.name.clone(),
        source: d.source.clone(),
        primary_key: d.primary_key.clone(),
        fields: d
            .fields
            .iter()
            .map(|f| WireField {
                name: f.name.clone(),
                expression: WireExpression {
                    sql: Some(f.expression_sql.clone()),
                    dialects: Vec::new(),
                },
                datatype: f.datatype,
                custom_extensions: meridian::join(&f.meta, &f.other_extensions),
            })
            .collect(),
        custom_extensions: meridian::join(&d.meta, &d.other_extensions),
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
    }
}
