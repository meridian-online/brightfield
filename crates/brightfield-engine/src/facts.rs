//! The unsampled facts a sampled plot needs to stay honest.

use duckdb::arrow::array::{Array, Float64Array, Int64Array, StringArray};
use duckdb::Connection;

use crate::error::EngineError;
use brightfield_spec::ast::{Spec, ValueOrParamRef};

/// What one mark's query would have returned had it not been sampled.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MarkFacts {
    /// The unsampled row count — the `of` half of the notice. Counted, not
    /// inferred from the modulus: a hash sample is not a uniform partition.
    pub rows: u64,
    /// The unsampled continuous extent of the x channel's column, when that
    /// column has one (a categorical positional column has none).
    pub x_domain: Option<(f64, f64)>,
    /// The same for y.
    pub y_domain: Option<(f64, f64)>,
    /// The unsampled VALUE SET of each colour-bearing channel whose column is
    /// a string, keyed by the channel's Mosaic wire name (`fill`, `stroke`).
    ///
    /// A categorical domain is a list, and a category's palette slot is its
    /// index in that list, so a sample that drops a category outright shifts
    /// every later one. This is the set the complete render would have seen;
    /// the renderer orders it, so the ordering rule lives in one place rather
    /// than being split across a SQL collation and a Rust comparator.
    ///
    /// A channel with no entry has no measured set — it may not be named, be
    /// named as a literal colour or a `$param`, sit on a column that holds
    /// something other than strings, or have had its query fail. The caller
    /// treats the absence itself as the answer and goes on refusing, so which
    /// of those it was does not have to be told apart here.
    pub categories: Vec<(String, Vec<String>)>,
    /// The unsampled category ORDER of each non-colour channel whose column is
    /// a string, keyed by the channel's Mosaic wire name (`x`, `y`, …), in the
    /// order the unsampled query first produces each category.
    ///
    /// A separate field from [`Self::categories`] because it is a separate
    /// quantity. That one is a SET whose order the renderer discards; this is
    /// the order itself, and it is the whole payload — a band scale gives each
    /// category a slot by its index here, so the list decides where the marks
    /// are. Merging the two would let a colour set be read as an order.
    ///
    /// Absent for the same reasons [`Self::categories`] is absent, and treated
    /// the same way by the caller: no entry means the channel goes on being
    /// refused.
    pub band_categories: Vec<(String, Vec<String>)>,
}

/// The x and y channel column expressions of the mark at `index`, quoted for
/// SQL. `None` for a channel the mark does not name, or names as a `$param`
/// (a param-valued channel is a constant, and has no column to measure).
pub(crate) fn positional_columns(spec: &Spec, index: usize) -> (Option<String>, Option<String>) {
    let marks = brightfield_sql::emit::collect_marks(spec);
    let Some(mark) = marks.get(index) else {
        return (None, None);
    };
    let col = |key: &str| match mark.options.get(key) {
        Some(ValueOrParamRef::Value(v)) => v
            .as_str()
            .map(|c| format!("\"{}\"", c.replace('"', "\"\""))),
        _ => None,
    };
    (col("x"), col("y"))
}

/// The colour-bearing channels of the mark at `index` that name a plain string
/// option, as `(wire name, quoted SQL identifier)`.
///
/// The option is taken at face value as a column name — which is also how
/// `fill: steelblue` reads here, since a literal colour and a column are the
/// same YAML scalar. That ambiguity is resolved by the DATABASE rather than by
/// a colour-name table: the distinct-values query binds `"steelblue"` as an
/// identifier, DuckDB refuses it, and [`read_categories`] returns the error the
/// caller drops the channel on. A colour-name table would have to stay in step
/// with CSS, and being wrong about a name there would mean silently skipping a
/// real column.
pub(crate) fn categorical_columns(spec: &Spec, index: usize) -> Vec<(String, String)> {
    let marks = brightfield_sql::emit::collect_marks(spec);
    let Some(mark) = marks.get(index) else {
        return Vec::new();
    };
    ["fill", "stroke"]
        .into_iter()
        .filter_map(|key| match mark.options.get(key) {
            Some(ValueOrParamRef::Value(v)) => v
                .as_str()
                .map(|c| (key.to_string(), format!("\"{}\"", c.replace('"', "\"\"")))),
            _ => None,
        })
        .collect()
}

/// The non-colour channels of the mark at `index` that name a plain string
/// option, as `(wire name, quoted SQL identifier)` — the channels a band scale
/// can be inferred for.
///
/// The complement of [`categorical_columns`] over the channel vocabulary
/// `brightfield-render` maps a column to. Scale inference sends a string column
/// to a colour scale on `fill` and `stroke` and to a band scale everywhere
/// else, so the split here is the same split, made at the same place in the
/// name list.
///
/// A key the mark does not name, or names as a `$param`, contributes nothing —
/// as in [`categorical_columns`], and a non-column value is left for the
/// database to reject rather than screened here.
pub(crate) fn band_columns(spec: &Spec, index: usize) -> Vec<(String, String)> {
    let marks = brightfield_sql::emit::collect_marks(spec);
    let Some(mark) = marks.get(index) else {
        return Vec::new();
    };
    ["x", "y", "x1", "y1", "x2", "y2", "size", "text"]
        .into_iter()
        .filter_map(|key| match mark.options.get(key) {
            Some(ValueOrParamRef::Value(v)) => v
                .as_str()
                .map(|c| (key.to_string(), format!("\"{}\"", c.replace('"', "\"\"")))),
            _ => None,
        })
        .collect()
}

/// Run one channel's category query and read its column, row by row.
///
/// The values arrive in the order the statement produced them, and what that
/// order means is the STATEMENT's to decide. The distinct-values query behind a
/// colour scale leaves it to the scan and the renderer sorts it, so the
/// ordering rule is not split across a SQL collation and a Rust comparator; the
/// band-order query orders in SQL, because the quantity it is measuring IS the
/// order the unsampled rows arrive in and no client-side rule reconstructs it.
pub(crate) fn read_categories(
    conn: &Connection,
    sql: &str,
    mark_index: usize,
) -> Result<Vec<String>, EngineError> {
    let fail = |cause: duckdb::Error| EngineError::QueryFailed {
        mark_index,
        mark_kind: "unsampled-categories".to_string(),
        sql: sql.to_string(),
        cause,
    };
    let mut stmt = conn.prepare(sql).map_err(fail)?;
    let batches: Vec<duckdb::arrow::record_batch::RecordBatch> =
        stmt.query_arrow(duckdb::params![]).map_err(fail)?.collect();
    let mut out = Vec::new();
    for batch in batches {
        let Some(col) = batch.column(0).as_any().downcast_ref::<StringArray>() else {
            continue;
        };
        for i in 0..col.len() {
            if !col.is_null(i) {
                out.push(col.value(i).to_string());
            }
        }
    }
    Ok(out)
}

/// Run the facts query and read its one row.
pub(crate) fn read_mark_facts(
    conn: &Connection,
    sql: &str,
    mark_index: usize,
    has_x: bool,
    has_y: bool,
) -> Result<MarkFacts, EngineError> {
    let fail = |cause: duckdb::Error| EngineError::QueryFailed {
        mark_index,
        mark_kind: "unsampled-facts".to_string(),
        sql: sql.to_string(),
        cause,
    };
    let mut stmt = conn.prepare(sql).map_err(fail)?;
    let batches: Vec<duckdb::arrow::record_batch::RecordBatch> =
        stmt.query_arrow(duckdb::params![]).map_err(fail)?.collect();
    let Some(batch) = batches.into_iter().find(|b| b.num_rows() > 0) else {
        return Ok(MarkFacts::default());
    };
    let rows = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .filter(|a| !a.is_null(0))
        .map_or(0, |a| a.value(0).max(0) as u64);

    let pair = |base: usize| -> Option<(f64, f64)> {
        let lo = batch.column_by_name(&format!("__bf_lo{base}"))?;
        let hi = batch.column_by_name(&format!("__bf_hi{base}"))?;
        let lo = lo.as_any().downcast_ref::<Float64Array>()?;
        let hi = hi.as_any().downcast_ref::<Float64Array>()?;
        if lo.is_null(0) || hi.is_null(0) {
            return None;
        }
        Some((lo.value(0), hi.value(0)))
    };

    Ok(MarkFacts {
        rows,
        x_domain: if has_x { pair(0) } else { None },
        y_domain: if has_y { pair(1) } else { None },
        categories: Vec::new(),
        band_categories: Vec::new(),
    })
}
