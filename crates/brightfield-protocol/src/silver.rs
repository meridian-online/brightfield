//! The silver tables: an analyst's raw tables cast through FineType's transformation contracts.
//!
//! Each column the model labels is cast by the SQL its label carries, wrapped in `TRY(...)` so a
//! value that does not cast becomes NULL and the table still builds. The raw value of every such
//! failure is kept in a `<dataset>__failed` companion table, so nothing is lost by the cast. A
//! dataset folded into another is joined onto it, its columns prefixed with its name.
//!
//! The contracts are FineType's, vendored at the tag the rest of the app stages FineType from (see
//! `vendor/finetype/README.md`), so the cast and the classifier that chose the labels agree.

use std::collections::BTreeMap;

use brightfield_ossie::{Dataset, Model, Relationship};
use serde::Deserialize;

/// The text of the vendored file, which `scripts/vendor-finetype-transforms.sh` writes.
const VENDORED: &str = include_str!("../vendor/finetype/transforms.json");

/// The lineage columns every raw and silver table starts with.
const LINEAGE: [&str; 3] = ["__source_file", "__source_part", "__source_row"];

/// What FineType's definition of one label says about casting it.
#[derive(Debug, Clone, Deserialize)]
struct Contract {
    duckdb_type: String,
    transform: Option<String>,
}

/// FineType's transformation contract for each label: the DuckDB type and the SQL that casts a
/// value to it, with `{col}` standing for the column.
#[derive(Debug, Clone, Deserialize)]
pub struct FinetypeTransforms {
    tag: String,
    labels: BTreeMap<String, Contract>,
}

impl FinetypeTransforms {
    /// The contracts vendored into this crate.
    ///
    /// # Panics
    ///
    /// If the vendored file does not parse, which `vendored_file_reads` in the tests rules out.
    #[must_use]
    pub fn vendored() -> Self {
        Self::from_json(VENDORED).expect("the vendored transforms parse")
    }

    /// Read contracts from the JSON `scripts/vendor-finetype-transforms.sh` writes.
    ///
    /// # Errors
    ///
    /// The parse error, when `json` is not that shape.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// The FineType tag the contracts were taken from.
    #[must_use]
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// The DuckDB type `label` casts to, or `None` for a label FineType does not define.
    #[must_use]
    pub fn duckdb_type(&self, label: &str) -> Option<&str> {
        self.labels.get(label).map(|c| c.duckdb_type.as_str())
    }

    /// The SQL that casts a value of `label`, or `None` for a label with no transform or an unknown one.
    #[must_use]
    pub fn transform(&self, label: &str) -> Option<&str> {
        self.labels.get(label)?.transform.as_deref()
    }
}

/// `name` as a SQL identifier.
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `s` as a SQL string literal.
fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The SQL that casts `column` for `finetype`, without the name it is selected as; `None` when
/// the column passes through.
///
/// This is FineType's own five-branch projection with the cast wrapped in `TRY`: no label, a
/// label the contracts do not define, or a VARCHAR label passes the column through; a label with a
/// transform takes it with `{col}` replaced; any other is a `TRY_CAST` to its type.
fn cast_expr(
    column: &str,
    finetype: Option<&str>,
    transforms: &FinetypeTransforms,
) -> Option<String> {
    let label = finetype?;
    let ty = transforms.duckdb_type(label)?;
    if ty == "VARCHAR" {
        return None;
    }
    let col = quoted(column);
    Some(match transforms.transform(label) {
        Some(transform) => format!("TRY({})", transform.replace("{col}", &col)),
        None => format!("TRY_CAST({col} AS {ty})"),
    })
}

/// The projection of `column` into a silver table: the cast named as the column, or the column
/// itself when it passes through.
#[must_use]
pub fn cast_projection(
    column: &str,
    finetype: Option<&str>,
    transforms: &FinetypeTransforms,
) -> String {
    match cast_expr(column, finetype, transforms) {
        Some(expr) => format!("{expr} AS {}", quoted(column)),
        None => quoted(column),
    }
}

/// A dataset gets silver tables unless it is excluded or folded into another.
fn has_silver(d: &Dataset) -> bool {
    !d.meta.excluded && d.meta.folded_into.is_none()
}

/// The raw table of `dataset` with the columns that will be cast read as text, so a column DuckDB
/// typed on load (a date, say) meets a transform written for text.
fn source(d: &Dataset, transforms: &FinetypeTransforms) -> String {
    let mut columns: Vec<String> = LINEAGE.iter().map(|c| quoted(c)).collect();
    for f in &d.fields {
        let col = quoted(&f.name);
        if cast_expr(&f.name, f.meta.finetype.as_deref(), transforms).is_some() {
            columns.push(format!("CAST({col} AS VARCHAR) AS {col}"));
        } else {
            columns.push(col);
        }
    }
    format!(
        "SELECT {} FROM {}",
        columns.join(", "),
        quoted(&format!("raw_{}", d.name))
    )
}

/// `d`'s columns as cast, lineage first, to select from [`source`].
fn projections(d: &Dataset, transforms: &FinetypeTransforms) -> Vec<String> {
    let mut out: Vec<String> = LINEAGE.iter().map(|c| quoted(c)).collect();
    out.extend(
        d.fields
            .iter()
            .map(|f| cast_projection(&f.name, f.meta.finetype.as_deref(), transforms)),
    );
    out
}

/// The join between a host and a dataset folded into it: the host's key columns and the folded
/// dataset's, which the relationship may state in either direction.
fn fold_keys<'m>(
    model: &'m Model,
    host: &str,
    folded: &str,
) -> Option<(&'m [String], &'m [String])> {
    model.relationships.iter().find_map(|r: &Relationship| {
        if r.from == host && r.to == folded {
            Some((r.from_columns.as_slice(), r.to_columns.as_slice()))
        } else if r.from == folded && r.to == host {
            Some((r.to_columns.as_slice(), r.from_columns.as_slice()))
        } else {
            None
        }
    })
}

/// The statement that makes `dataset`'s silver table from its raw table, or an empty string for a
/// dataset that is excluded, folded into another, or not in the model.
///
/// The table has the lineage columns, then each field's cast under its own name, then each
/// dataset folded into this one, joined on the relationship between them with its cast columns
/// named `<folded>_<column>` and its join key left out.
#[must_use]
pub fn silver_sql(model: &Model, dataset: &str, transforms: &FinetypeTransforms) -> String {
    let Some(d) = model
        .datasets
        .iter()
        .find(|d| d.name == dataset)
        .filter(|d| has_silver(d))
    else {
        return String::new();
    };
    let mut ctes = vec![format!(
        "host AS (SELECT {} FROM ({}))",
        projections(d, transforms).join(", "),
        source(d, transforms)
    )];
    let mut select = vec!["host.*".to_string()];
    let mut joins = Vec::new();
    let folds = model
        .datasets
        .iter()
        .filter(|f| f.meta.folded_into.as_deref() == Some(dataset) && !f.meta.excluded);
    for (n, folded) in folds.enumerate() {
        let Some((host_keys, folded_keys)) = fold_keys(model, dataset, &folded.name) else {
            continue;
        };
        if host_keys.is_empty() || host_keys.len() != folded_keys.len() {
            continue;
        }
        let alias = format!("fold_{n}");
        ctes.push(format!(
            "{alias} AS (SELECT {} FROM ({}))",
            projections(folded, transforms).join(", "),
            source(folded, transforms)
        ));
        let on: Vec<String> = host_keys
            .iter()
            .zip(folded_keys)
            .map(|(h, f)| format!("host.{} = {alias}.{}", quoted(h), quoted(f)))
            .collect();
        joins.push(format!("LEFT JOIN {alias} ON {}", on.join(" AND ")));
        for f in folded
            .fields
            .iter()
            .filter(|f| !folded_keys.contains(&f.name))
        {
            select.push(format!(
                "{alias}.{} AS {}",
                quoted(&f.name),
                quoted(&format!("{}_{}", folded.name, f.name))
            ));
        }
    }
    format!(
        "CREATE OR REPLACE TABLE {} AS WITH {} SELECT {} FROM host {}",
        quoted(dataset),
        ctes.join(", "),
        select.join(", "),
        joins.join(" ")
    )
}

/// The statement that makes `<dataset>__failed`, or an empty string where [`silver_sql`] makes no
/// table. It holds one row per value that was present in the raw table and did not cast: its
/// lineage columns, the `column` it was in and its raw `value`.
#[must_use]
pub fn failed_sql(model: &Model, dataset: &str, transforms: &FinetypeTransforms) -> String {
    let Some(d) = model
        .datasets
        .iter()
        .find(|d| d.name == dataset)
        .filter(|d| has_silver(d))
    else {
        return String::new();
    };
    let target = quoted(&format!("{dataset}__failed"));
    let lineage = LINEAGE
        .iter()
        .map(|c| quoted(c))
        .collect::<Vec<_>>()
        .join(", ");
    let parts: Vec<String> = d
        .fields
        .iter()
        .filter_map(|f| {
            let expr = cast_expr(&f.name, f.meta.finetype.as_deref(), transforms)?;
            let col = quoted(&f.name);
            Some(format!(
                "SELECT {lineage}, {} AS \"column\", {col} AS \"value\" FROM src \
                 WHERE {col} IS NOT NULL AND {expr} IS NULL",
                literal(&f.name)
            ))
        })
        .collect();
    if parts.is_empty() {
        return format!(
            "CREATE OR REPLACE TABLE {target} AS SELECT NULL::VARCHAR AS \"__source_file\", \
             NULL::VARCHAR AS \"__source_part\", NULL::BIGINT AS \"__source_row\", \
             NULL::VARCHAR AS \"column\", NULL::VARCHAR AS \"value\" WHERE false"
        );
    }
    format!(
        "CREATE OR REPLACE TABLE {target} AS WITH src AS ({}) {}",
        source(d, transforms),
        parts.join(" UNION ALL ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendored_file_reads() {
        let t = FinetypeTransforms::vendored();
        assert!(t.labels.len() > 100);
        assert!(t.tag().starts_with('v'));
    }
}
