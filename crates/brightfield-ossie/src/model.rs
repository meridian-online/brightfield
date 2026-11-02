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

impl Dataset {
    /// Whether the dataset has tables of its own in the Protocol's database: it is not left out,
    /// folded into another dataset, or one part of a combination, whose silver table is the
    /// combined dataset's.
    #[must_use]
    pub fn has_own_table(&self) -> bool {
        !self.meta.excluded && self.meta.folded_into.is_none() && self.meta.combined_into.is_none()
    }
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
    /// No discovery pass has verified the join: it was added by hand. Its evidence is what
    /// Brightfield measured over the silver tables since, or empty where nothing could be measured.
    Unmeasured,
    Sample,
    Full,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetMeta {
    pub role: Role,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub role_evidence: Vec<String>,
    /// How well each role fitted the table, as dovetail scored it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_scores: Option<RoleScores>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folded_into: Option<String>,
    /// The column of the host that tells apart the rows folded in from this table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fold_column: Option<String>,
    /// The combined table this one is a part of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combined_into: Option<String>,
    /// For a combined table, the parts it was built from and how.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combination: Option<Combination>,
    #[serde(default)]
    pub excluded: bool,
    pub decided_by: DecidedBy,
    pub source_file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_part: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u64>,
    /// Keys Brightfield does not type, carried through a read and write unchanged.
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

/// The score of each role for a table, in dovetail's order. A role dovetail did not score reads
/// as 0, and a role Brightfield does not know is kept in `other`, so scores from a newer dovetail
/// still read and are written back as they came.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleScores {
    #[serde(default)]
    pub entity: f64,
    #[serde(default)]
    pub event: f64,
    #[serde(default)]
    pub reference: f64,
    #[serde(default)]
    pub bridge: f64,
    #[serde(default)]
    pub snapshot: f64,
    #[serde(default)]
    pub unclear: f64,
    /// Roles Brightfield does not type, carried through a read and write unchanged.
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

/// How the parts of a combined table relate. A kind Brightfield does not know reads as
/// [`Self::Other`] with its text, and is written back as it came.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum CombinationKind {
    Partition,
    Snapshot,
    Duplicate,
    Other(String),
}

impl From<String> for CombinationKind {
    fn from(text: String) -> Self {
        match text.as_str() {
            "partition" => Self::Partition,
            "snapshot" => Self::Snapshot,
            "duplicate" => Self::Duplicate,
            _ => Self::Other(text),
        }
    }
}

impl From<CombinationKind> for String {
    fn from(kind: CombinationKind) -> Self {
        match kind {
            CombinationKind::Partition => "partition".to_string(),
            CombinationKind::Snapshot => "snapshot".to_string(),
            CombinationKind::Duplicate => "duplicate".to_string(),
            CombinationKind::Other(text) => text,
        }
    }
}

/// A table dovetail built from several files that share one shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Combination {
    pub kind: CombinationKind,
    pub parts: Vec<String>,
    /// The column that holds what told the parts apart, such as a year taken from the file name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_column: Option<String>,
    /// Keys Brightfield does not type, carried through a read and write unchanged.
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finetype: Option<String>,
    pub decided_by: DecidedBy,
    /// Keys Brightfield does not type, carried through a read and write unchanged.
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
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
    /// Parent key values held by more than one parent row: a join that should find one parent
    /// per child row finds several for these.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub parent_duplicates: u64,
    /// Keys Brightfield does not type, carried through a read and write unchanged.
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if passes a reference
fn is_zero(n: &u64) -> bool {
    *n == 0
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
    /// Keys Brightfield does not type, carried through a read and write unchanged.
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
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
    /// The discovery pass that wrote the model, such as `sample` or `full`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass: Option<String>,
    /// How the columns were typed, kept as dovetail wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typing: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rows: Option<u64>,
    /// Dropped relationships live here, not in `relationships`, so the core
    /// document only asserts joins the model holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected: Vec<Relationship>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped_sources: Vec<SkippedSource>,
    /// Keys Brightfield does not type, carried through a read and write unchanged.
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}
