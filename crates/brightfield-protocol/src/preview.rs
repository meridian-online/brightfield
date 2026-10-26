//! The SQL that shows a relationship's joined rows, for the model inspector's preview and for
//! opening them in the grid.
//!
//! Each side reads its dataset's rows as cast by the silver contract: the silver table, named as
//! the dataset, or, for a left-out or folded dataset without a silver table of its own, its raw
//! table read through silver's own `TRY` cast projection ([`cast_rows_sql`]). So both
//! sides' keys are compared as silver compares them, and a raw value that does not cast reads
//! NULL rather than failing the statement. The lineage columns are left out of both, and each
//! column is named `<dataset>.<column>` so the two sides' columns stay apart.

use std::sync::OnceLock;

use brightfield_ossie::{Model, LINEAGE};

use crate::silver::{cast_rows_sql, FinetypeTransforms};

/// `name` as a SQL identifier.
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `s` as a SQL string literal.
fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn transforms() -> &'static FinetypeTransforms {
    static VENDORED: OnceLock<FinetypeTransforms> = OnceLock::new();
    VENDORED.get_or_init(FinetypeTransforms::vendored)
}

/// What the Protocol holds `dataset`'s cast rows in: its silver table, or a subquery casting its
/// raw table when it has none.
fn rows_of(model: &Model, dataset: &str) -> String {
    match model.datasets.iter().find(|d| d.name == dataset) {
        Some(d) if !d.meta.excluded && d.meta.folded_into.is_none() => quoted(dataset),
        Some(d) => format!("({})", cast_rows_sql(d, transforms())),
        None => quoted(dataset),
    }
}

/// The joined rows of `relationship`, the child side read through `child`, or an empty string when
/// the model has no relationship of that name or it names no key columns.
fn join_sql(model: &Model, relationship: &str, child: impl Fn(String) -> String) -> String {
    let Some(r) = model.relationships.iter().find(|r| r.name == relationship) else {
        return String::new();
    };
    if r.from_columns.is_empty() || r.from_columns.len() != r.to_columns.len() {
        return String::new();
    }
    // A relationship from a table to itself names its sides by their part in the join.
    let (child_prefix, parent_prefix) = if r.from == r.to {
        ("from".to_string(), "to".to_string())
    } else {
        (r.from.clone(), r.to.clone())
    };
    let lineage = LINEAGE.join(", ");
    let on: Vec<String> = r
        .from_columns
        .iter()
        .zip(&r.to_columns)
        .map(|(c, p)| format!("child.{} = parent.{}", quoted(c), quoted(p)))
        .collect();
    format!(
        "SELECT COLUMNS(child.* EXCLUDE ({lineage})) AS {}, \
         COLUMNS(parent.* EXCLUDE ({lineage})) AS {} \
         FROM {} AS child LEFT JOIN {} AS parent ON {}",
        literal(&format!("{child_prefix}.\\0")),
        literal(&format!("{parent_prefix}.\\0")),
        child(rows_of(model, &r.from)),
        rows_of(model, &r.to),
        on.join(" AND ")
    )
}

/// The SQL that previews `relationship`'s joined rows: at most `limit` rows of its `from` (child)
/// side, sampled with `USING SAMPLE` before the join, each joined to its `to` (parent) row. The
/// parent side is read whole, so a sampled key finds its parent if it has one; a child row whose
/// key has none is kept with the parent's columns empty, so orphans show. Empty when the model
/// has no relationship of that name.
#[must_use]
pub fn join_preview_sql(model: &Model, relationship: &str, limit: usize) -> String {
    let joined = join_sql(model, relationship, |table| {
        format!("(SELECT * FROM {table} USING SAMPLE {limit} ROWS)")
    });
    if joined.is_empty() {
        return joined;
    }
    // A parent key that repeats would give a sampled child row more than one match.
    format!("{joined} LIMIT {limit}")
}

/// [`join_preview_sql`] without the sample or the limit: the child rows of `relationship`, each
/// joined to its parent row.
#[must_use]
pub fn join_rows_sql(model: &Model, relationship: &str) -> String {
    join_sql(model, relationship, |table| table)
}
