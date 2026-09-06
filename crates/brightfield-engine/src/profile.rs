//! Per-source DuckDB column profiles (sidebar profiling).
//!
//! Pure data types + framework-free helpers for [`Session::profile_sources`]
//! (implemented in `lib.rs`, where the private `conn`/`spec` fields live). No
//! UI framework, no rendering: `brightfield-model`'s `profile_model` formats these for the Data
//! sidebar. Profiles describe the SOURCE — full-table stats, not any live
//! cross-filter selection.
//!
//! [`Session::profile_sources`]: crate::Session::profile_sources

use duckdb::arrow::array::{Array, Int64Array, StringArray};
use duckdb::arrow::record_batch::RecordBatch;

use crate::semantic::SemanticType;

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
    /// What the column MEANS, as opposed to what DuckDB stored it as — and
    /// whether the column's own values bear that meaning out.
    ///
    /// [`SemanticType::NotAsked`] unless the session was loaded with
    /// [`LoadOptions::type_source`]. This is the one place a semantic label
    /// reaches anything downstream of the engine; see [`crate::semantic`].
    ///
    /// [`LoadOptions::type_source`]: crate::LoadOptions::type_source
    pub semantic: SemanticType,
    /// The moments and the counted shape of a numeric column — see
    /// [`ColumnMoments`]. `None` for a column with no moment defined over it:
    /// a VARCHAR, a temporal column, or a numeric one whose rows are all null.
    pub moments: Option<ColumnMoments>,
}

/// How many equal-width buckets the distribution pass counts a numeric column
/// into when its values are too many to carry one bar each.
///
/// Twenty times [`DISPLAY_BINS`] on purpose: the band's bar chart wants 24
/// bins and its rug wants one bucket per pixel of a cell that is 96 to 480
/// points wide, and one scan at this resolution answers both. Folding 480 into
/// 24 is exact — `floor(floor(t * 480) / 20)` is `floor(t * 24)` — so the bars
/// a reader sees are the bars a direct 24-bin pass would have produced, rather
/// than a resampling of them.
pub const BIN_RESOLUTION: usize = 480;

/// How many bars the binned branch of the distribution draws.
pub const DISPLAY_BINS: usize = 24;

/// At or below this many distinct values a column's distribution is one bar
/// per value; above it, [`DISPLAY_BINS`] equal-width bins.
pub const VALUE_BAR_LIMIT: u64 = 64;

/// The counted shape of one numeric column's values.
///
/// Which variant a column gets is decided by its EXACT distinct count, not by
/// [`ColumnProfile::distinct`], which is an estimate — see
/// [`ColumnMoments::distinct`].
#[derive(Debug, Clone, PartialEq)]
pub enum Distribution {
    /// One entry per distinct value, ascending by value: the value, and how
    /// many rows carry it. Emitted when the exact distinct count is at most
    /// [`VALUE_BAR_LIMIT`].
    Values(Vec<(f64, u64)>),
    /// Row counts in [`BIN_RESOLUTION`] equal-width buckets spanning
    /// [`ColumnMoments::min`]`..=`[`ColumnMoments::max`], bucket 0 first.
    /// Length is [`BIN_RESOLUTION`]; an empty bucket is a zero.
    Bins(Vec<u64>),
}

/// The bars a column header band draws, already reduced to what fits a cell.
#[derive(Debug, Clone, PartialEq)]
pub enum Bars {
    /// One bar per distinct value: its position across the range as a fraction
    /// in `0.0..=1.0`, and its row count.
    PerValue(Vec<(f64, u64)>),
    /// Exactly [`DISPLAY_BINS`] counts, bin 0 first.
    Binned(Vec<u64>),
}

impl Bars {
    /// How many bars there are — [`DISPLAY_BINS`] for the binned branch, the
    /// distinct count for the other.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::PerValue(v) => v.len(),
            Self::Binned(b) => b.len(),
        }
    }

    /// Whether there is nothing to draw.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The largest count among the bars, or 1 where there are none — the
    /// denominator a bar's height is a fraction of.
    #[must_use]
    pub fn peak(&self) -> u64 {
        match self {
            Self::PerValue(v) => v.iter().map(|(_, n)| *n).max().unwrap_or(1),
            Self::Binned(b) => b.iter().copied().max().unwrap_or(1),
        }
        .max(1)
    }
}

/// What one numeric column's values are, beyond their bounds and their counts:
/// the moments a column header band states and the shape it draws.
///
/// `None` on [`ColumnProfile::moments`] for a column no moment is defined for
/// — a VARCHAR, a temporal column, or a numeric column with no non-null row.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnMoments {
    /// `avg("col")` over the non-null rows.
    pub mean: f64,
    /// `median("col")` — DuckDB's interpolating median, so an even count of
    /// rows gives the mean of the middle pair rather than either of them.
    pub median: f64,
    /// `stddev_samp("col")`. `None` where the sample deviation is undefined,
    /// which is a column with one non-null row.
    pub sd: Option<f64>,
    /// `count(DISTINCT "col")` — the EXACT count, unlike
    /// [`ColumnProfile::distinct`], which stays the estimate the tile choice
    /// reads.
    pub distinct: u64,
    /// The minimum as a number, for placing a value across the range.
    pub min: f64,
    /// The maximum. See [`ColumnMoments::min`].
    pub max: f64,
    /// The counted shape of the column.
    pub distribution: Distribution,
}

impl ColumnMoments {
    /// The span the distribution is laid across — zero for a constant column.
    #[must_use]
    pub fn span(&self) -> f64 {
        self.max - self.min
    }

    /// Where `value` falls across the range, as a fraction in `0.0..=1.0`. A
    /// constant column puts every value at 0.0.
    #[must_use]
    pub fn position_of(&self, value: f64) -> f64 {
        let span = self.span();
        if span <= 0.0 {
            0.0
        } else {
            ((value - self.min) / span).clamp(0.0, 1.0)
        }
    }

    /// The bars the band draws: one per distinct value where there are at most
    /// [`VALUE_BAR_LIMIT`] of them, and [`DISPLAY_BINS`] equal-width bins
    /// otherwise.
    #[must_use]
    pub fn bars(&self) -> Bars {
        match &self.distribution {
            Distribution::Values(values) => Bars::PerValue(
                values
                    .iter()
                    .map(|(v, n)| (self.position_of(*v), *n))
                    .collect(),
            ),
            Distribution::Bins(bins) => Bars::Binned(fold(bins, DISPLAY_BINS)),
        }
    }

    /// The rug: `columns` equal-width buckets across the range, each carrying
    /// how many rows fall in it.
    ///
    /// Folded from whichever shape the distribution took, so a column with few
    /// distinct values draws a rug of spikes at the values it has rather than a
    /// smear.
    #[must_use]
    pub fn rug(&self, columns: usize) -> Vec<u64> {
        let columns = columns.max(1);
        match &self.distribution {
            Distribution::Values(values) => {
                let mut out = vec![0u64; columns];
                for (v, n) in values {
                    let t = self.position_of(*v);
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let at = ((t * columns as f64) as usize).min(columns - 1);
                    out[at] = out[at].saturating_add(*n);
                }
                out
            }
            Distribution::Bins(bins) => fold(bins, columns),
        }
    }
}

/// `source` re-bucketed into `want` equal-width buckets.
///
/// Each source bucket contributes its whole count to the destination bucket its
/// own centre falls in, which is exact when `want` divides `source.len()` —
/// the case the band's 24 bars take out of [`BIN_RESOLUTION`] — and an
/// approximation of the same shape otherwise.
fn fold(source: &[u64], want: usize) -> Vec<u64> {
    let want = want.max(1);
    let mut out = vec![0u64; want];
    if source.is_empty() {
        return out;
    }
    for (i, n) in source.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let centre = (i as f64 + 0.5) / source.len() as f64;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let at = ((centre * want as f64) as usize).min(want - 1);
        out[at] = out[at].saturating_add(*n);
    }
    out
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

/// Whether a DuckDB column type carries a mean, a median and a deviation:
/// the numeric subset of [`is_min_max_type`], temporal excluded.
///
/// A date has a min and a max a reader compares, and an average date is a
/// number in a unit nobody asked about — so the band states bounds for a
/// timestamp column and no moments.
pub(crate) fn is_moment_type(duckdb_type: &str) -> bool {
    let upper = duckdb_type.trim().to_ascii_uppercase();
    let base = upper.split('(').next().unwrap_or(&upper).trim();
    if base.starts_with("TIMESTAMP") || base.starts_with("TIME") || base.starts_with("DATE") {
        return false;
    }
    is_min_max_type(duckdb_type)
}

/// Read a nullable DOUBLE cell from row 0. `None` on SQL NULL — an all-null
/// column's average, or the sample deviation of a single row — or on a
/// non-Float64 array.
pub(crate) fn read_double(batch: &RecordBatch, col: usize) -> Option<f64> {
    let arr = batch
        .column(col)
        .as_any()
        .downcast_ref::<duckdb::arrow::array::Float64Array>()?;
    (!arr.is_null(0)).then(|| arr.value(0))
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
