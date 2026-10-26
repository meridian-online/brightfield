//! The Apache Ossie semantic model as Rust types, with Meridian's vendor extension.
//!
//! A `model.ossie.yaml` is one Ossie document. The datasets, fields and
//! relationships Ossie defines are core; what Meridian adds (roles, grades, evidence,
//! who decided) rides in `custom_extensions` entries whose `vendor_name` is
//! [`VENDOR`] and whose `data` is a JSON string, so a foreign Ossie reader carries
//! it without interpreting it. Extensions from other vendors are kept verbatim.
//!
//! No UI framework: could this run headless? yes.

mod edit;
mod layout;
mod merge;
mod meridian;
mod model;
mod validate;
mod wire;

pub use edit::{apply, Edit, EditError};
pub use layout::{
    edge_text, is_proposal, layout, list_rows, summary, Band, Card, EdgeLabel, ListRow, ModelLayout,
};
pub use merge::merge_discovery;
pub use model::{
    Dataset, DatasetMeta, Datatype, DecidedBy, Evidence, Extension, Field, FieldMeta, Grade, Model,
    ModelMeta, Relationship, RelationshipMeta, Role, SkippedSource, Verified,
};
pub use validate::{validate, SchemaIssue};
pub use wire::{read, write, ReadError};

/// The Ossie specification version this crate reads and writes.
pub const OSSIE_VERSION: &str = "0.2.0.dev0";

/// The lineage columns every loaded and silver table starts with, in order: the file a row came
/// from, the sheet or table inside it (NULL for a flat file), and its 1-based data row.
pub const LINEAGE: [&str; 3] = ["__source_file", "__source_part", "__source_row"];

/// The `vendor_name` of Meridian's extension entries.
pub const VENDOR: &str = "MERIDIAN";
