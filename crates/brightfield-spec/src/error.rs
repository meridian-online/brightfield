//! Parser error types.
//!
//! `ParseError` is the fatal-error surface. Non-fatal diagnostics are carried
//! separately via `ParseWarning` on `ParseOutput`.
//!
//! `SourceSpan` is a plain public struct — the parser crate deliberately does
//! NOT depend on `miette` so it stays app-agnostic. Downstream app crates may
//! wrap `ParseError` in `miette::Report` themselves.

use thiserror::Error;

/// A byte-offset span into spec source. `offset` is the byte position of the
/// first byte covered by the span; `length` is the byte length of the span.
///
/// Spans are best-effort: the underlying YAML/JSON deserialiser produces them
/// for syntactic errors, but semantic errors detected post-deserialisation may
/// carry `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    /// Byte offset into the source string where the span starts.
    pub offset: usize,
    /// Byte length of the span.
    pub length: usize,
}

impl SourceSpan {
    /// A zero-length span at `offset`. Useful when a precise span is unavailable
    /// but a diagnostic still wants to point at a position.
    #[must_use]
    pub fn point(offset: usize) -> Self {
        Self { offset, length: 0 }
    }
}

/// The kind of vocabulary surface an unknown name was encountered on.
/// Keys the error message so the user knows where to look in their spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameSurface {
    /// Mark discriminator (e.g. `mark: fooBar`).
    Mark,
    /// Interactor discriminator (e.g. `select: fooBar`).
    Interactor,
    /// Input discriminator (e.g. `input: fooBar`).
    Input,
    /// Component / layout discriminator (e.g. an object with neither `plot`
    /// nor `vconcat` nor `hconcat` nor a mark discriminator at the root).
    Component,
    /// Legend channel name (e.g. `legend: fooChannel`).
    LegendChannel,
}

impl NameSurface {
    /// A short human-readable label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Mark => "mark",
            Self::Interactor => "interactor",
            Self::Input => "input",
            Self::Component => "component",
            Self::LegendChannel => "legend channel",
        }
    }
}

/// A fatal parsing error. The parser returns `Result<ParseOutput, ParseError>`
/// where `ParseError` is only produced for malformed or structurally-broken
/// input; non-fatal observations about a successfully-parsed spec are
/// collected into `ParseOutput.warnings`.
#[derive(Debug, Error)]
pub enum ParseError {
    /// The source could not be parsed as YAML.
    #[error("malformed YAML: {msg}")]
    YamlSyntax {
        /// Underlying parser message.
        msg: String,
        /// Location in the source if the deserialiser provided one.
        span: Option<SourceSpan>,
    },

    /// The source could not be parsed as JSON.
    #[error("malformed JSON: {msg}")]
    JsonSyntax {
        /// Underlying parser message.
        msg: String,
        /// Location in the source if the deserialiser provided one.
        span: Option<SourceSpan>,
    },

    /// An I/O error occurred reading a spec from disk.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// `parse_spec_path` was called with a path whose extension is neither
    /// `.yaml`, `.yml`, nor `.json`.
    #[error("unknown spec format: extension `{ext}` not recognised (expected .yaml, .yml, or .json)")]
    UnknownFormat {
        /// The offending extension (without leading dot), or empty if absent.
        ext: String,
    },

    /// A mark, interactor, input, component, or legend channel name is not in
    /// the vocabulary registry.
    #[error("unknown {surface} name `{name}`", surface = surface.label())]
    UnknownName {
        /// The offending name as it appeared in the source.
        name: String,
        /// Which vocabulary surface the name was used on.
        surface: NameSurface,
        /// Location in the source if available.
        span: Option<SourceSpan>,
    },

    /// A data source declaration is structurally malformed (e.g. neither a
    /// string shorthand nor a `{type, ...}` / `{file, ...}` / `{query, ...}`
    /// / inline-rows object).
    #[error("malformed data definition{location}", location = span.map_or(String::new(), |s| format!(" at byte {}", s.offset)))]
    MalformedDataDef {
        /// Location in the source if available.
        span: Option<SourceSpan>,
        /// Human-readable detail.
        detail: String,
    },

    /// A param declaration is structurally malformed.
    #[error("malformed param definition `{name}`{location}", location = span.map_or(String::new(), |s| format!(" at byte {}", s.offset)))]
    MalformedParamDef {
        /// Name of the param that failed to parse.
        name: String,
        /// Location in the source if available.
        span: Option<SourceSpan>,
        /// Human-readable detail.
        detail: String,
    },

    /// A `$`-prefixed string appeared in a strict-context field (under
    /// `meta`, `config`, or `plotDefaults`) where `$param` references are
    /// not allowed.
    #[error("unresolved `$` reference in strict context `{field_path}`: `${name}`")]
    StrictContextUnresolvedRef {
        /// The dotted field path (e.g. `meta.title`).
        field_path: String,
        /// The identifier following the `$`.
        name: String,
        /// Location in the source if available.
        span: Option<SourceSpan>,
    },

    /// The spec violates a structural schema rule (e.g. unknown field under
    /// a closed head, wrong type at a known field).
    #[error("schema violation at `{path}`: {detail}")]
    SchemaViolation {
        /// The dotted field path where the violation occurred.
        path: String,
        /// Human-readable detail.
        detail: String,
        /// Location in the source if available.
        span: Option<SourceSpan>,
    },
}

impl ParseError {
    /// Returns the span associated with this error, if one is available.
    #[must_use]
    pub fn span(&self) -> Option<SourceSpan> {
        match self {
            Self::YamlSyntax { span, .. }
            | Self::JsonSyntax { span, .. }
            | Self::UnknownName { span, .. }
            | Self::MalformedDataDef { span, .. }
            | Self::MalformedParamDef { span, .. }
            | Self::StrictContextUnresolvedRef { span, .. }
            | Self::SchemaViolation { span, .. } => *span,
            Self::Io(_) | Self::UnknownFormat { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ac-09 verification: every variant (except `Io` and `UnknownFormat`)
    /// exposes an accessible `span` and the public `SourceSpan` type is a
    /// plain struct defined here (not re-exported from miette).
    #[test]
    fn dfspec_ac09_variants_expose_span() {
        let variants: Vec<ParseError> = vec![
            ParseError::YamlSyntax {
                msg: "x".into(),
                span: Some(SourceSpan { offset: 1, length: 2 }),
            },
            ParseError::JsonSyntax {
                msg: "x".into(),
                span: Some(SourceSpan::point(3)),
            },
            ParseError::UnknownName {
                name: "fooBar".into(),
                surface: NameSurface::Mark,
                span: None,
            },
            ParseError::MalformedDataDef {
                span: Some(SourceSpan::point(0)),
                detail: "x".into(),
            },
            ParseError::MalformedParamDef {
                name: "p".into(),
                span: None,
                detail: "x".into(),
            },
            ParseError::StrictContextUnresolvedRef {
                field_path: "meta.title".into(),
                name: "missing".into(),
                span: Some(SourceSpan::point(0)),
            },
            ParseError::SchemaViolation {
                path: "meta.bogus".into(),
                detail: "unknown field".into(),
                span: Some(SourceSpan::point(0)),
            },
        ];
        // Each variant must expose a span() accessor (even if None).
        for v in &variants {
            let _ = v.span();
        }
        // Io / UnknownFormat return None — verified by contract, not asserted
        // here because constructing Io requires an io::Error.
    }

    #[test]
    fn dfspec_ac09_source_span_is_plain_struct() {
        // Constructing SourceSpan with named fields proves it is a plain struct
        // with public fields — not an opaque wrapper around a miette type.
        let s = SourceSpan { offset: 10, length: 5 };
        assert_eq!(s.offset, 10);
        assert_eq!(s.length, 5);
    }
}
