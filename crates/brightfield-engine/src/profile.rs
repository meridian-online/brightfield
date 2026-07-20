//! Per-source DuckDB column profiles (sidebar profiling).
//!
//! Pure data types + framework-free helpers for [`Session::profile_sources`]
//! (implemented in `lib.rs`, where the private `conn`/`spec` fields live). No
//! gpui, no rendering: the app's `profile_model` formats these for the Data
//! sidebar. Profiles describe the SOURCE — full-table stats, not any live
//! cross-filter selection.
//!
//! [`Session::profile_sources`]: crate::Session::profile_sources

use duckdb::arrow::array::{Array, Int64Array, StringArray};
use duckdb::arrow::record_batch::RecordBatch;

/// One `data:` source's profile, emitted in spec declaration order.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceProfile {
    /// The source's declared name (the `data:` key).
    pub name: String,
    /// What profiling produced for this source.
    pub outcome: ProfileOutcome,
}

/// The three shapes a source profile can take.
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileOutcome {
    /// Real profiles: the view's full-table row count and per-column stats.
    Profiled {
        /// `count(*)` over the source view.
        row_count: u64,
        /// One entry per non-internal column, in DESCRIBE order.
        columns: Vec<ColumnProfile>,
    },
    /// An attached-database source (`.duckdb`/`.db` ATTACH) — deliberately not
    /// profiled v1 (needs table-qualified introspection). Never queried.
    Unsupported,
    /// DESCRIBE or the aggregate pass failed; carries the DuckDB reason. Other
    /// sources are unaffected (per-source failure isolation).
    Failed(String),
}

/// Per-column stats from the one aggregate pass over a source view.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnProfile {
    /// The column name (DESCRIBE `column_name`).
    pub name: String,
    /// The DuckDB type name (DESCRIBE `column_type`, e.g. "BIGINT",
    /// "VARCHAR", "DATE").
    pub type_name: String,
    /// Rows where the column is non-null (`count("col")`).
    pub non_null: u64,
    /// Rows where the column is null (`row_count - non_null`).
    pub nulls: u64,
    /// Approximate distinct count (`approx_count_distinct`).
    pub distinct: u64,
    /// Minimum, rendered by DuckDB via `CAST(min(...) AS VARCHAR)`. Present
    /// only for numeric/temporal columns; `None` for non-gated types or an
    /// all-null column (SQL NULL).
    pub min: Option<String>,
    /// Maximum (see [`ColumnProfile::min`]).
    pub max: Option<String>,
}

/// Columns Brightfield's SQL layer synthesises (e.g. a raster's `__bf_count`)
/// — implementation detail, not source schema. Mirrors the
/// `sidebar_model::is_internal_column` precedent the sidebar established.
pub(crate) fn is_internal_column(name: &str) -> bool {
    name.starts_with("__bf_")
}

/// Whether a DuckDB column type carries a meaningful min/max for the sidebar:
/// numeric + temporal only. Varchar/blob/boolean show count/distinct/nulls;
/// universal min/max is a formatting swamp (long strings, blobs) for marginal
/// insight (tabletop trade-off).
pub(crate) fn is_min_max_type(duckdb_type: &str) -> bool {
    let upper = duckdb_type.trim().to_ascii_uppercase();
    // Strip any parameter list (DECIMAL(18,3), TIMESTAMP(6), ...).
    let base = upper.split('(').next().unwrap_or(&upper).trim();
    if base.starts_with("TIMESTAMP") || base.starts_with("TIME") || base.starts_with("DATE") {
        return true;
    }
    matches!(
        base,
        "TINYINT"
            | "SMALLINT"
            | "INTEGER"
            | "BIGINT"
            | "HUGEINT"
            | "UTINYINT"
            | "USMALLINT"
            | "UINTEGER"
            | "UBIGINT"
            | "UHUGEINT"
            | "FLOAT"
            | "REAL"
            | "DOUBLE"
            | "DECIMAL"
            | "NUMERIC"
    )
}

/// Read a BIGINT cell from row 0 of a single-row aggregate result. The
/// aggregate SELECT casts every count to BIGINT (→ Arrow `Int64`), so a
/// non-Int64 array or a null is a surprise the caller treats as 0 (counts are
/// never legitimately null here). Clamps negatives to 0.
pub(crate) fn read_count(batch: &RecordBatch, col: usize) -> u64 {
    batch
        .column(col)
        .as_any()
        .downcast_ref::<Int64Array>()
        .filter(|a| !a.is_null(0))
        .map(|a| a.value(0).max(0) as u64)
        .unwrap_or(0)
}

/// Read a nullable VARCHAR cell (min/max cast to VARCHAR) from row 0. `None`
/// on SQL NULL (all-null gated column) or a non-string array.
pub(crate) fn read_text(batch: &RecordBatch, col: usize) -> Option<String> {
    let arr = batch.column(col).as_any().downcast_ref::<StringArray>()?;
    if arr.is_null(0) {
        None
    } else {
        Some(arr.value(0).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_min_max_type_gates_numeric_and_temporal() {
        for t in [
            "BIGINT",
            "INTEGER",
            "DOUBLE",
            "FLOAT",
            "DECIMAL(18,3)",
            "HUGEINT",
            "UBIGINT",
            "DATE",
            "TIMESTAMP",
            "TIMESTAMP WITH TIME ZONE",
            "TIME",
        ] {
            assert!(is_min_max_type(t), "{t} should be gated on");
        }
        for t in ["VARCHAR", "BOOLEAN", "BLOB", "VARCHAR(10)", "UUID"] {
            assert!(!is_min_max_type(t), "{t} should be gated off");
        }
    }

    #[test]
    fn is_internal_column_filters_bf_prefix() {
        assert!(is_internal_column("__bf_count"));
        assert!(!is_internal_column("delay"));
        assert!(!is_internal_column("bf_count"));
    }
}
