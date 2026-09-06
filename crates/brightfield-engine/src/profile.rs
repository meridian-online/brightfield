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
    /// a VARCHAR, a temporal column, or a numeric one with no non-null row.
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

/// The most times DuckDB reads a source while [`Session::profile_sources`]
/// profiles it, whatever the source's column count.
///
/// Three, and each one is a statement: the DESCRIBE that names the columns,
/// the one aggregate SELECT that counts and measures every one of them, and
/// the one `GROUP BY` that counts every numeric column's distribution
/// together. The third used to be one statement per numeric column — a read
/// apiece, so a table's distributions cost whatever its numeric column count
/// happened to be — and this number is what keeps it from becoming that
/// again. No count is quoted here on purpose: the figure would be a property
/// of whichever table somebody had in mind, and a `pub const`'s documentation
/// is the wrong place to keep one true.
///
/// A *read* here is a leaf of DuckDB's physical plan: an operator with no
/// children, which is where rows enter a query. Counting leaves rather than
/// statements is deliberate, because the shape most likely to be reached for
/// — a `UNION ALL` branch per column — is one statement that reads the table
/// once per branch, and a statement count would call that free.
///
/// [`Session::profile_sources`]: crate::Session::profile_sources
pub const SCANS_PER_SOURCE: u32 = 3;

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

/// One statement a profiling pass issued, and how many times DuckDB's plan
/// for it reads a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementScans {
    /// The statement, verbatim.
    pub sql: String,
    /// Leaves in the statement's physical plan, or `None` where DuckDB
    /// declined to explain it.
    ///
    /// `None` rather than zero, and [`ScanTally::scans`] carries the absence
    /// out to its caller, because a statement whose cost could not be read is
    /// not a statement that cost zero — and a gate handed a silent zero passes
    /// over exactly the case it was written for.
    pub scans: Option<u32>,
    /// Leaves of the same plan that read a **file** rather than a relation
    /// this session holds in memory, or `None` where DuckDB declined to
    /// explain the statement.
    ///
    /// It cannot exceed [`StatementScans::scans`], and it equals it for a
    /// statement over a source that is still a view on `read_csv` /
    /// `read_parquet` —
    /// `the_profile_pass_reads_the_file_at_every_leaf_it_plans` in the
    /// open-scan harness is what holds that. It is the smaller number when a
    /// source has been materialised, which is how this session acquires a base
    /// table.
    pub file_reads: Option<u32>,
}

/// How many times profiling read the table, statement by statement.
///
/// Collected by [`Session::profile_sources_counting_scans`], which is what
/// turns the `EXPLAIN` behind these numbers on; a profile pass nobody asked to
/// count leaves it off.
///
/// [`Session::profile_sources_counting_scans`]: crate::Session::profile_sources_counting_scans
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanTally {
    /// Every statement the pass issued, in the order it issued them.
    pub statements: Vec<StatementScans>,
}

impl ScanTally {
    /// The sum over the statements, or `None` if DuckDB declined to explain
    /// one of them.
    #[must_use]
    pub fn scans(&self) -> Option<u32> {
        self.statements
            .iter()
            .try_fold(0u32, |acc, s| Some(acc.saturating_add(s.scans?)))
    }

    /// The sum over the statements of the leaves that read a **file**, or
    /// `None` if DuckDB declined to explain one of them.
    ///
    /// **This is the number the claim about opening a data file is made in.**
    /// [`ScanTally::scans`] counts the leaves, which is the right instrument
    /// while a leaf is a read of the file — as it is for the profile pass,
    /// where the source is a view over `read_csv`. It stops being the right
    /// instrument the moment a relation is materialised: a scan of a
    /// session-scoped table is a leaf and is not a read of the file, and the
    /// two cost different orders of magnitude.
    #[must_use]
    pub fn file_reads(&self) -> Option<u32> {
        self.statements
            .iter()
            .try_fold(0u32, |acc, s| Some(acc.saturating_add(s.file_reads?)))
    }

    /// The statements DuckDB declined to explain — empty when
    /// [`ScanTally::scans`] answers.
    #[must_use]
    pub fn unexplained(&self) -> Vec<&str> {
        self.statements
            .iter()
            .filter(|s| s.scans.is_none())
            .map(|s| s.sql.as_str())
            .collect()
    }
}

/// How many leaves DuckDB's `EXPLAIN (FORMAT json)` tree carries, or `None`
/// where the text is not a plan.
///
/// A leaf is a node with no children, which in a physical plan is where rows
/// enter it — a `READ_CSV`, a `READ_PARQUET`, a `SEQ_SCAN` over a table, the
/// `COLUMN_DATA_SCAN` a DESCRIBE reads its answer out of.
///
/// **Every leaf counts, and no operator name is enumerated here.** An
/// inclusion list would have to name each spelling DuckDB gives a scan, and
/// the one it gains next release is the one that would go uncounted — which is
/// the direction that fails silently. Counting leaves has the opposite bias: a
/// leaf this code has never seen makes the number go up, and a bound that
/// reddens is a bound somebody reads.
pub(crate) fn plan_scans(explained: &str) -> Option<u32> {
    let plan: serde_json::Value = serde_json::from_str(explained).ok()?;
    let roots = plan.as_array()?;
    let mut total = 0u32;
    for root in roots {
        total = total.saturating_add(count_leaves(root)?);
    }
    Some(total)
}

/// Leaves under `node`, counting `node` itself when it has none.
fn count_leaves(node: &serde_json::Value) -> Option<u32> {
    count_matching_leaves(node, &|_| true)
}

/// How many of the plan's leaves read a **file** rather than a relation held
/// in memory, or `None` where the text is not a plan.
///
/// **The rule is stated as an exclusion and that is the whole of its
/// robustness.** A leaf counts as a file read *unless* DuckDB's own plan says
/// what else it reads. There are two such statements and they are both
/// positive claims the plan makes about the leaf, not names this code has
/// guessed:
///
/// | key in `extra_info` | what the leaf reads | example |
/// |---|---|---|
/// | `Table` | a base table in this database | `SEQ_SCAN` over a materialised source |
/// | `CTE Index` | a CTE materialised inside the same statement | `CTE_SCAN` under a repeated scalar subquery |
///
/// An inclusion list would have to name each spelling DuckDB gives a file
/// reader — `READ_CSV`, `READ_PARQUET`, and the one it gains next release —
/// and a spelling this code has never seen would go uncounted, which is the
/// direction that fails silently. Excluding what the plan names has the
/// opposite bias: a leaf nobody anticipated makes the number go **up**, and a
/// bound that reddens is a bound somebody reads. A `DUMMY_SCAN` and a
/// `COLUMN_DATA_SCAN` are therefore counted as file reads, which over-counts a
/// constant SELECT and is correct for the `DESCRIBE` of a `read_csv` view.
///
/// **The unsafe direction is closed by a test rather than by this rule.** If
/// DuckDB stopped carrying `Table`, every leaf would count and nothing would
/// be hidden. If it stopped carrying `CTE Index` the same. The failure that
/// would matter is this function returning zero when the file *is* being read,
/// and `the_profile_pass_reads_the_file_at_every_leaf_it_plans` in the
/// open-scan harness is what refuses it: the profile pass runs over a source
/// that is still a view on `read_csv`, so its file reads must equal its
/// leaves.
///
/// It cannot exceed [`plan_scans`] on the same plan, and it equals it for a
/// plan whose leaves are file readers —
/// `a_leaf_reads_a_file_unless_the_plan_names_a_table_or_a_cte` reads both
/// directions off literal plans.
pub(crate) fn plan_file_reads(explained: &str) -> Option<u32> {
    let plan: serde_json::Value = serde_json::from_str(explained).ok()?;
    let roots = plan.as_array()?;
    let mut total = 0u32;
    for root in roots {
        total = total.saturating_add(count_matching_leaves(root, &|leaf| {
            let named = leaf
                .get("extra_info")
                .is_some_and(|info| info.get("Table").is_some() || info.get("CTE Index").is_some());
            !named
        })?);
    }
    Some(total)
}

/// Leaves under `node` that `keep` accepts, counting `node` itself when it has
/// no children.
fn count_matching_leaves(
    node: &serde_json::Value,
    keep: &dyn Fn(&serde_json::Value) -> bool,
) -> Option<u32> {
    let children = node.get("children")?.as_array()?;
    if children.is_empty() {
        return Some(u32::from(keep(node)));
    }
    let mut total = 0u32;
    for child in children {
        total = total.saturating_add(count_matching_leaves(child, keep)?);
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // The leaf counter, against literal plans rather than against DuckDB.
    //
    // [`SCANS_PER_SOURCE`] is a bound on what these two functions return, so a
    // counter that had stopped counting would take the bound with it: a guard
    // over it would pass over a table being read once per column, which is the
    // defect the bound exists for. Pinning it against DuckDB's answer
    // for the statement the pass writes today cannot catch that, because that
    // statement genuinely plans to one leaf: a counter hard-wired to 1 agrees
    // with it. So the cases below are literals, and each names the wrong
    // counter it rules out.
    //
    // The shapes are `EXPLAIN (FORMAT json)`'s: a top-level array of roots,
    // each node an object with `name` and `children`.
    // ---------------------------------------------------------------------

    /// One root, three deep, one leaf — an aggregate over a projection over a
    /// scan, which is what the profile pass's aggregate SELECT plans to.
    ///
    /// Rules out a counter that counts NODES: this plan has three of them and
    /// one leaf.
    const PLAN_ONE_LEAF_THREE_DEEP: &str = r#"[
      { "name": "UNGROUPED_AGGREGATE", "children": [
        { "name": "PROJECTION", "children": [
          { "name": "READ_CSV", "children": [] }
        ] }
      ] }
    ]"#;

    /// One root over a UNION of three scans.
    ///
    /// **The shape the bound exists for.** A `UNION ALL` branch per column is
    /// ONE statement that reads the table once per branch, so this is what
    /// tells a leaf count apart from a statement count. Rules out a counter
    /// that returns the root's immediate child count: that is 1 here and the
    /// answer is 3.
    const PLAN_UNION_OF_THREE_SCANS: &str = r#"[
      { "name": "UNGROUPED_AGGREGATE", "children": [
        { "name": "UNION", "children": [
          { "name": "READ_CSV", "children": [] },
          { "name": "READ_CSV", "children": [] },
          { "name": "READ_CSV", "children": [] }
        ] }
      ] }
    ]"#;

    /// A leaf sitting beside a non-leaf, with two leaves under the non-leaf.
    ///
    /// Rules out the same immediate-child counter from the other direction —
    /// the root has two children and three leaves — and rules out a counter
    /// that walks only the first child, which would answer 1.
    const PLAN_LEAF_BESIDE_A_SUBTREE: &str = r#"[
      { "name": "HASH_JOIN", "children": [
        { "name": "READ_CSV", "children": [] },
        { "name": "PROJECTION", "children": [
          { "name": "UNION", "children": [
            { "name": "READ_PARQUET", "children": [] },
            { "name": "SEQ_SCAN", "children": [] }
          ] }
        ] }
      ] }
    ]"#;

    /// Two roots in the array. Rules out a counter that reads the first and
    /// stops.
    const PLAN_TWO_ROOTS: &str = r#"[
      { "name": "PROJECTION", "children": [
        { "name": "READ_CSV", "children": [] }
      ] },
      { "name": "COLUMN_DATA_SCAN", "children": [] }
    ]"#;

    /// The plan a composition takes once its source has been read into a
    /// session-scoped table: DuckDB folds the two identical bin-scheme
    /// subqueries into one CTE, so the leaves are the rows' scan, the CTE's
    /// own scan, and two reads of the CTE. **Four leaves, no file read.**
    ///
    /// The extra-info keys are DuckDB v1.5's, taken from a real
    /// `EXPLAIN (FORMAT json)` rather than invented — a `SEQ_SCAN` carries
    /// `Table` and a `CTE_SCAN` carries `CTE Index`.
    const PLAN_MATERIALISED_HISTOGRAM: &str = r#"[
      { "name": "ORDER_BY", "children": [
        { "name": "HASH_GROUP_BY", "children": [
          { "name": "SEQ_SCAN", "extra_info": { "Table": "\"temp\".main.opened__bf_materialised" }, "children": [] },
          { "name": "PROJECTION", "children": [
            { "name": "SEQ_SCAN", "extra_info": { "Table": "\"temp\".main.opened__bf_materialised" }, "children": [] }
          ] },
          { "name": "CTE_SCAN", "extra_info": { "CTE Index": "174" }, "children": [] },
          { "name": "CTE_SCAN", "extra_info": { "CTE Index": "174" }, "children": [] }
        ] }
      ] }
    ]"#;

    /// The same statement before the source was materialised: the reads are
    /// `READ_CSV`, which carry a `Function` and no `Table`. **Three leaves,
    /// all three file reads.**
    const PLAN_FILE_BACKED_HISTOGRAM: &str = r#"[
      { "name": "ORDER_BY", "children": [
        { "name": "HASH_GROUP_BY", "children": [
          { "name": "READ_CSV", "extra_info": { "Function": "READ_CSV" }, "children": [] },
          { "name": "READ_CSV", "extra_info": { "Function": "READ_CSV" }, "children": [] },
          { "name": "READ_CSV", "extra_info": { "Function": "READ_CSV" }, "children": [] }
        ] }
      ] }
    ]"#;

    /// A file read beside a table read under one root — the shape a
    /// half-materialised session would produce, and the one a rule that
    /// answered per *statement* rather than per *leaf* would get wrong.
    const PLAN_ONE_FILE_ONE_TABLE: &str = r#"[
      { "name": "HASH_JOIN", "children": [
        { "name": "READ_PARQUET", "extra_info": { "Function": "READ_PARQUET" }, "children": [] },
        { "name": "SEQ_SCAN", "extra_info": { "Table": "\"temp\".main.mat" }, "children": [] }
      ] }
    ]"#;

    /// **A leaf is a file read unless the plan says what else it reads, and
    /// the two things it can say are a table and a CTE.**
    ///
    /// The bound this feeds is **zero**, and a function returning zero passes
    /// that bound on every plan there is. So the cases that matter most here
    /// are the ones that must NOT be zero: a rule that had inverted, or a walk
    /// that had stopped finding leaves, agrees with the shipped composition
    /// exactly and disagrees with these.
    ///
    /// `PLAN_MATERIALISED_HISTOGRAM` is the other direction and the reason the
    /// `CTE Index` arm is there: DuckDB plans the histogram's two identical
    /// bin-scheme subqueries as one CTE read twice, and a rule excluding
    /// `Table` alone counted those two reads of an in-statement CTE as
    /// reads of the file — two per histogram tile, which is a count that grows
    /// with the tile count and a bound that could never be met.
    #[test]
    fn a_leaf_reads_a_file_unless_the_plan_names_a_table_or_a_cte() {
        assert_eq!(
            plan_file_reads(PLAN_FILE_BACKED_HISTOGRAM),
            Some(3),
            "three READ_CSV leaves are three reads of the file — a rule that              answered zero here would meet the composition's bound by              measuring nothing"
        );
        assert_eq!(
            plan_scans(PLAN_FILE_BACKED_HISTOGRAM),
            Some(3),
            "and every one of them is a leaf, so the two counts agree while              nothing is materialised"
        );

        assert_eq!(
            plan_file_reads(PLAN_MATERIALISED_HISTOGRAM),
            Some(0),
            "a scan of a table and a read of an in-statement CTE are not reads              of the file"
        );
        assert_eq!(
            plan_scans(PLAN_MATERIALISED_HISTOGRAM),
            Some(4),
            "the leaves are still there — the composition still issues the              statement, and only what it reads has changed. A leaf count that              fell to zero here would mean the walk had stopped walking"
        );

        assert_eq!(
            plan_file_reads(PLAN_ONE_FILE_ONE_TABLE),
            Some(1),
            "one of the two leaves reads a file, so the answer is per leaf and              not per statement"
        );

        assert_eq!(
            plan_file_reads(PLAN_ONE_LEAF_THREE_DEEP),
            Some(1),
            "a leaf carrying no extra_info at all is counted as a file read —              the unknown leaf makes the number go up, which is the direction a              bound can catch"
        );
        assert_eq!(
            plan_file_reads(PLAN_TWO_ROOTS),
            Some(2),
            "both roots are walked, as they are for the leaf count"
        );
        assert_eq!(
            plan_file_reads("not a plan"),
            None,
            "text that is not a plan is an absence, not a zero"
        );
    }

    /// **The counter counts leaves, and each case rules out a different
    /// counter that would agree with the shipped statement.**
    ///
    /// The shipped distribution statement plans to one leaf, so a counter that
    /// answers 1 passes every guard in the repository while the file is read
    /// once per column. These four literals are what stands between that and a
    /// green suite.
    #[test]
    fn the_leaf_count_is_the_leaves_and_not_the_nodes_or_the_children() {
        assert_eq!(
            plan_scans(PLAN_ONE_LEAF_THREE_DEEP),
            Some(1),
            "three nested operators with one scan under them is one leaf"
        );
        assert_eq!(
            plan_scans(PLAN_UNION_OF_THREE_SCANS),
            Some(3),
            "a UNION of three scans is three leaves — one statement, three \
             reads, which is the whole reason this counts leaves"
        );
        assert_eq!(
            plan_scans(PLAN_LEAF_BESIDE_A_SUBTREE),
            Some(3),
            "a leaf beside a subtree holding two more is three leaves, not the \
             root's two children"
        );
        assert_eq!(
            plan_scans(PLAN_TWO_ROOTS),
            Some(2),
            "both roots in the array are counted"
        );

        // Three different answers on purpose: a counter returning any single
        // constant fails at least two of them, and a constant satisfying all
        // three would have to be three values at once.
        let answers = [
            plan_scans(PLAN_ONE_LEAF_THREE_DEEP),
            plan_scans(PLAN_UNION_OF_THREE_SCANS),
            plan_scans(PLAN_TWO_ROOTS),
        ];
        assert!(
            answers[0] != answers[1] && answers[1] != answers[2] && answers[0] != answers[2],
            "the cases answer {answers:?} — two that agree cannot both be \
             ruling out a constant"
        );
    }

    /// **A plan this cannot read is an absence, not a zero.**
    ///
    /// [`ScanTally::scans`] carries the `None` out and the guards compare
    /// against `Some(_)`, so this is where the absence is decided. A zero here
    /// would let a statement whose plan went unread pass a bound it was never
    /// measured against.
    #[test]
    fn a_plan_that_cannot_be_read_answers_none_rather_than_zero() {
        for (what, text) in [
            ("not JSON at all", "a drawn plan tree, not JSON"),
            (
                "an object rather than the array of roots",
                r#"{"name":"READ_CSV"}"#,
            ),
            ("a node with no children key", r#"[{"name":"READ_CSV"}]"#),
            (
                "a node whose children is not an array",
                r#"[{"name":"READ_CSV","children":"none"}]"#,
            ),
            (
                "a node deep in the tree with no children key",
                r#"[{"name":"PROJECTION","children":[{"name":"READ_CSV"}]}]"#,
            ),
        ] {
            assert_eq!(plan_scans(text), None, "{what} should answer None");
        }
    }

    /// **The tally refuses to total a statement it could not read.**
    ///
    /// The pair to the test above, one level up: an unreadable plan reaches
    /// [`ScanTally`] as a `None` on one statement, and the total has to go
    /// with it rather than quietly summing the rest.
    #[test]
    fn a_tally_holding_one_unexplained_statement_has_no_total() {
        let readable = StatementScans {
            sql: "SELECT count(*) FROM t".to_string(),
            scans: Some(1),
            file_reads: Some(1),
        };
        let unreadable = StatementScans {
            sql: "DESCRIBE t".to_string(),
            scans: None,
            file_reads: None,
        };

        let whole = ScanTally {
            statements: vec![readable.clone(), readable.clone()],
        };
        assert_eq!(whole.scans(), Some(2));
        assert_eq!(whole.file_reads(), Some(2));
        assert!(whole.unexplained().is_empty());

        let holed = ScanTally {
            statements: vec![readable, unreadable],
        };
        assert_eq!(
            holed.scans(),
            None,
            "one unread statement beside a read one totalled to a number, so a \
             bound would be compared against a partial sum"
        );
        assert_eq!(
            holed.file_reads(),
            None,
            "the file-read total is the one under a bound of zero, so an \
             unread statement summing to zero beside it is the exact shape of \
             a bound met by not looking"
        );
        assert_eq!(holed.unexplained(), vec!["DESCRIBE t"]);
    }

    /// **The two totals are read off the same statements and are not the same
    /// number.**
    ///
    /// A composition over a materialised source plans leaves and reads no
    /// file, which is the case a single total cannot express — so a
    /// [`ScanTally`] whose `file_reads` merely echoed `scans` would report a
    /// bound of zero as unreachable and one of "a read per tile" as met.
    #[test]
    fn a_tally_totals_leaves_and_file_reads_apart() {
        let tally = ScanTally {
            statements: vec![
                StatementScans {
                    sql: "SELECT * FROM opened".to_string(),
                    scans: Some(4),
                    file_reads: Some(0),
                },
                StatementScans {
                    sql: "SELECT count(*) FROM opened".to_string(),
                    scans: Some(1),
                    file_reads: Some(0),
                },
            ],
        };
        assert_eq!(
            tally.scans(),
            Some(5),
            "five leaves were planned across the two statements"
        );
        assert_eq!(
            tally.file_reads(),
            Some(0),
            "and none of them read the file"
        );
    }

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
