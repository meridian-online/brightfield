//! Structured errors for the DuckDB execution engine.

use brightfield_sql::error::EmitError;

/// Errors that can occur during spec execution.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// DuckDB connection setup failed.
    #[error("connection failed: {cause}")]
    ConnectionFailed {
        /// The underlying DuckDB error.
        cause: duckdb::Error,
    },

    /// A data source DDL statement failed to execute.
    #[error("DDL failed for source '{source_name}': {cause}\n  SQL: {sql}")]
    DdlFailed {
        /// The data source name from the spec.
        source_name: String,
        /// The SQL statement that failed.
        sql: String,
        /// The underlying DuckDB error.
        cause: duckdb::Error,
    },

    /// A per-mark query failed to execute.
    #[error("query failed for mark {mark_index} ({mark_kind}): {cause}\n  SQL: {sql}")]
    QueryFailed {
        /// The depth-first mark index.
        mark_index: usize,
        /// The mark kind wire name (e.g. "dot", "lineY").
        mark_kind: String,
        /// The SQL statement that failed.
        sql: String,
        /// The underlying DuckDB error.
        cause: duckdb::Error,
    },

    /// Upstream SQL emission failed.
    #[error("emit failed: {cause}")]
    EmitFailed {
        /// The emission error from brightfield-sql.
        cause: EmitError,
    },

    /// A spec declares a remote (network-reached) data source, but the
    /// DuckDB extension it needs could not be loaded — so remote sources
    /// are disabled rather than served wrong. Local file specs are
    /// unaffected: they never attempt an extension load and never touch
    /// the network.
    #[error(
        "remote source '{source_name}' ({location}) is disabled: {reason} — \
         remote data needs the network; local file specs still work offline"
    )]
    RemoteDisabled {
        /// The data source name from the spec.
        source_name: String,
        /// The remote location the spec pointed at.
        location: String,
        /// Why the extension is unavailable (the DuckDB load error).
        reason: String,
    },

    /// A LOCAL data source (a `.ducklake` catalog on disk) needs a DuckDB
    /// extension that could not be loaded. Distinct from
    /// [`Self::RemoteDisabled`] because the data itself is local — it is
    /// the extension INSTALL, not the data, that needs the network, and
    /// the message must not claim otherwise.
    #[error(
        "source '{source_name}' ({location}) is disabled: {reason} — \
         the catalog is local, but reading it needs the DuckDB \
         '{extension}' extension; sources that don't need it still work"
    )]
    ExtensionUnavailable {
        /// The data source name from the spec.
        source_name: String,
        /// The local catalog location the spec pointed at.
        location: String,
        /// The extension this source needs (e.g. `ducklake`).
        extension: String,
        /// Why the extension is unavailable (the DuckDB load error).
        reason: String,
    },

    /// A remote data source's SQL executed and failed — the network fetch
    /// itself is the cause (unreachable host, connection refused, an HTTP
    /// error from the far end). Raised for a failing source DDL at load
    /// AND for a failing query over a remote-backed mark mid-session (a
    /// remote view re-fetches on every execution, so the network can drop
    /// long after a successful load). Named separately from
    /// [`Self::DdlFailed`] / [`Self::QueryFailed`] so the surface names
    /// the network, never showing plausible-and-wrong local data instead.
    #[error(
        "remote source '{source_name}' could not be reached over the \
         network ({location}): {cause}"
    )]
    RemoteSourceFailed {
        /// The data source name from the spec.
        source_name: String,
        /// The remote location that could not be fetched.
        location: String,
        /// The underlying DuckDB error.
        cause: duckdb::Error,
    },

    /// A distinct-values options query failed (input widgets) —
    /// a bad column name, a vanished source, or a column type with no
    /// [`brightfield_spec::ast::SpecValue`] mapping. Per-input isolated:
    /// the caller warns and skips the one widget, never the dashboard.
    #[error("distinct values failed for {source_name}.{column}: {reason}")]
    DistinctFailed {
        /// The data source name from the spec.
        source_name: String,
        /// The column whose distinct values were requested.
        column: String,
        /// The underlying failure, stringified (a DuckDB error or an
        /// unsupported-type explanation).
        reason: String,
    },
}
