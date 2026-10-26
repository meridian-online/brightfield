//! The in-memory model: Ossie's core plus Meridian's extension, as plain data.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Schema-valid Ossie content Brightfield does not model (`ai_context`,
/// `unique_keys`, a field's `label` and `dimension`, and so on), keyed by its
/// YAML key and carried through a read and write unchanged.
pub type Extras = BTreeMap<String, serde_yaml::Value>;

/// A metric, kept as written: its name, so each one can be listed, and the rest of it opaque.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub name: String,
    #[serde(flatten)]
    pub body: Extras,
}

/// One entry of an expression's `dialects` list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dialect {
    pub dialect: String,
    pub expression: String,
}

/// A vendor extension entry this crate does not interpret: carried and written back unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extension {
    pub vendor_name: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub name: String,
    pub description: Option<String>,
    pub metrics: Vec<Metric>,
    pub extras: Extras,
    pub datasets: Vec<Dataset>,
    pub relationships: Vec<Relationship>,
    pub meta: ModelMeta,
    pub other_extensions: Vec<Extension>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dataset {
    pub name: String,
    pub description: Option<String>,
    pub metrics: Vec<Metric>,
    pub extras: Extras,
    pub source: String,
    pub primary_key: Vec<String>,
    pub fields: Vec<Field>,
    pub meta: DatasetMeta,
    pub other_extensions: Vec<Extension>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    /// The expression Brightfield acts on: the written `sql`, else the
    /// `OSSIE_SQL_2026`, `ANSI_SQL` or `DUCKDB` dialect entry, else the first one.
    pub expression_sql: String,
    /// The written `expression.dialects`, kept as written.
    pub dialects: Vec<Dialect>,
    /// True when no `sql` was written and `expression_sql` was taken from
    /// `dialects`; `write` then emits the dialects alone. Clear it to write `expression_sql` as `sql`.
    pub dialects_only: bool,
    pub extras: Extras,
    pub datatype: Option<Datatype>,
    pub meta: FieldMeta,
    pub other_extensions: Vec<Extension>,
}

/// A join between two datasets: `from` is the many side, `to` the one side.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relationship {
    pub name: String,
    pub from: String,
    pub to: String,
    pub from_columns: Vec<String>,
    pub to_columns: Vec<String>,
    pub meta: RelationshipMeta,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extras: Extras,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other_extensions: Vec<Extension>,
}

/// Ossie's logical data types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Datatype {
    String,
    Integer,
    Decimal,
    Float,
    Boolean,
    Date,
    Time,
    DateTime,
    DateTimeTz,
    Opaque,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Entity,
    Event,
    Reference,
    Bridge,
    Snapshot,
    Unclear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Grade {
    Likely,
    Possible,
}

/// Who settled an element. Discovery, verification and rerun leave an `Analyst` element as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DecidedBy {
    Dovetail,
    Analyst,
}

/// How much of the data a relationship's evidence was measured on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verified {
    Sample,
    Full,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetMeta {
    pub role: Role,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub role_evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folded_into: Option<String>,
    #[serde(default)]
    pub excluded: bool,
    pub decided_by: DecidedBy,
    pub source_file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_part: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finetype: Option<String>,
    pub decided_by: DecidedBy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub value_overlap: f64,
    pub orphans: u64,
    pub child_rows: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_type: Option<String>,
    pub types_agree: bool,
    pub name_similarity: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipMeta {
    pub grade: Grade,
    pub evidence: Evidence,
    /// For example `N:1`.
    pub cardinality: String,
    pub verified: Verified,
    pub decided_by: DecidedBy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// A source file the discovery pass left out, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedSource {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelMeta {
    pub generator: String,
    /// Dropped relationships live here, not in `relationships`, so the core
    /// document only asserts joins the model holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected: Vec<Relationship>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped_sources: Vec<SkippedSource>,
}
