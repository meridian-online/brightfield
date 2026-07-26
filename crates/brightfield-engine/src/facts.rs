//! The unsampled facts a sampled plot needs to stay honest.

use duckdb::arrow::array::{Array, Float64Array, Int64Array};
use duckdb::Connection;

use crate::error::EngineError;
use brightfield_spec::ast::{Spec, ValueOrParamRef};

/// What one mark's query would have returned had it not been sampled.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MarkFacts {
    /// The unsampled row count — the `of` half of the notice. Counted, not
    /// inferred from the modulus: a hash sample is not a uniform partition.
    pub rows: u64,
    /// The unsampled continuous extent of the x channel's column, when that
    /// column has one (a categorical positional column has none).
    pub x_domain: Option<(f64, f64)>,
    /// The same for y.
    pub y_domain: Option<(f64, f64)>,
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
    })
}
