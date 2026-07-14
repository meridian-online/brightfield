//! Parser for the Mosaic declarative visualisation spec.
//!
//! The Mosaic spec (YAML/JSON) is the stable contract that drives brightfield's
//! query engine, coordinator, and renderer. This crate turns spec source into a
//! typed Rust AST — total over the full Mosaic 0.24.x vocabulary, fails with
//! located diagnostics on malformed input, and lifts `$param` / `$selection`
//! references into structured AST nodes at parse time.
//!
//! See `README.md` for the Option Z vocabulary contract and v1 non-goals.

pub mod analysis;
pub mod ast;
pub mod edit;
pub mod error;
pub mod expr;
pub mod layout;
pub mod parse;
pub mod vocab;

pub use crate::ast::{
    Component, DataSource, ExpressionNode, Input, Interactor, Mark, Meta, ParamNode, ParamRef,
    PlotNode, SelectionNode, Spec, SpecValue, ValueOrParamRef,
};
pub use crate::error::{NameSurface, ParseError, SourceSpan};
pub use crate::parse::{parse_spec, parse_spec_path, serialise_spec, Format, ParseOutput, ParseWarning};
pub use crate::vocab::{ComponentKind, ImplStatus, InputKind, InteractorKind, MarkKind};

/// Mosaic spec version this crate targets. The parser accepts any spec whose
/// `meta.version` shares the same major.minor as `SUPPORTED_MOSAIC_MAJOR_MINOR`;
/// other versions produce `ParseWarning::VersionMismatch`.
pub const SUPPORTED_MOSAIC_VERSION: &str = "0.24.x";

/// Major and minor version of the supported Mosaic spec. A spec's
/// `meta.version` is compared against this pair for mismatch detection.
pub const SUPPORTED_MOSAIC_MAJOR_MINOR: (u16, u16) = (0, 24);
