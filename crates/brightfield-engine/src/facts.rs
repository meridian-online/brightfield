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
    /// Empty for a channel the mark does not name, one it names as a literal
    /// or a `$param`, one whose column is not a string, and one whose query
    /// failed — every case in which no restoration is on offer and the caller
    /// must go on refusing.
    pub categories: Vec<(String, Vec<String>)>,
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

/// Run one channel's distinct-values query and read its column.
///
/// The rows come back in whatever order the scan produced them. That is
/// deliberate: ordering them here would put the ordering rule in a SQL
/// collation, while the renderer needs the same rule applied to the categories
/// it infers from an Arrow batch. The caller that owns the palette owns the
/// order.
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
    })
}
