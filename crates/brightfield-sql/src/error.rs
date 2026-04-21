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
}
