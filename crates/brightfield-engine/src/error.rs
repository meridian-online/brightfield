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
}
