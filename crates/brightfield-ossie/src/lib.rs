//! The Apache Ossie semantic model as Rust types, with Meridian's vendor extension.
//!
//! A `model.ossie.yaml` is one Ossie document. Everything Ossie defines (datasets,
//! fields, relationships) is core; everything Meridian adds (roles, grades, evidence,
//! who decided) rides in `custom_extensions` entries whose `vendor_name` is
//! [`VENDOR`] and whose `data` is a JSON string, so a foreign Ossie reader carries
//! it without interpreting it. Extensions from other vendors are kept verbatim.
//!
//! No UI framework: could this run headless? yes.

mod meridian;
mod model;
mod validate;
mod wire;

pub use model::{
    Dataset, DatasetMeta, Datatype, DecidedBy, Evidence, Extension, Field, FieldMeta, Grade, Model,
    ModelMeta, Relationship, RelationshipMeta, Role, SkippedSource, Verified,
};
pub use validate::{validate, SchemaIssue};
pub use wire::{read, write, ReadError};

/// The Ossie specification version this crate reads and writes.
pub const OSSIE_VERSION: &str = "0.2.0.dev0";

/// The `vendor_name` of Meridian's extension entries.
pub const VENDOR: &str = "MERIDIAN";
