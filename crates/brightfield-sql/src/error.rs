//! Emission errors for the data-source SQL emitter.

/// Errors that can occur during data-source SQL emission.
///
/// These are emitter-specific errors — parse errors live in `brightfield-spec`.
#[derive(Debug, thiserror::Error)]
pub enum EmitError {
    /// A file source has an unrecognised extension.
    #[error("unknown format: file '{path}' has unrecognised extension '.{extension}'")]
    UnknownFormat {
        /// The file path or URL that could not be dispatched.
        path: String,
        /// The extension that was not recognised.
        extension: String,
    },

    /// Inline row count exceeds the 1000-row cap.
    #[error("inline row count {count} exceeds 1000 — use a file source")]
    InlineRowLimit {
        /// The actual row count.
        count: usize,
    },

    /// An internal invariant was violated — a code path that should not be
    /// reachable was reached.
    #[error("invariant violation: {detail}")]
    InvariantViolation {
        /// What went wrong.
        detail: String,
    },

    /// A mark with an unimplemented `MarkKind` reached the emitter.
    ///
    /// Defence in depth — preflight should reject specs with unsupported marks
    /// before the emitter runs. If this error surfaces in production, the
    /// preflight gate has a gap.
    #[error("unsupported mark kind: {kind}")]
    UnsupportedMark {
        /// The wire name of the unsupported `MarkKind`.
        kind: String,
    },

    /// SQL parsing failed during structural conformance comparison.
    #[error("SQL parse error: {detail}")]
    SqlParseError {
        /// The parser's diagnostic.
        detail: String,
    },
}
