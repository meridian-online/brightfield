//! Measuring a join's evidence again over the Protocol's silver tables, and saying when what is
//! measured no longer bears the join out.
//!
//! [`evidence_sql`] is the statement a run's database answers; [`apply_evidence`] writes what it
//! answered onto the join, with a warning when the join's grade or cardinality no longer holds.
//! Neither changes a join's grade, who decided it, or how it was verified: the warning says what
//! changed, and the analyst decides what to do about it.

use crate::{Evidence, Grade, Model, Relationship};

/// `name` as a SQL identifier.
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// The statement that measures `relationship`'s evidence over the silver tables of its two
/// datasets, each named as its dataset. It answers one row:
///
/// - `child_rows`: the rows of the `from` (child) table;
/// - `orphans`: the child rows whose key finds no parent row;
/// - `value_overlap`: the share of the child's distinct keys that some parent row holds, 0.0 when
///   the child holds no key;
/// - `parent_duplicates`: the parent key values held by more than one parent row.
///
/// A key with a NULL in any of its columns is no key: it is not an orphan, and it is left out of
/// the overlap and the duplicates. Empty when the model has no relationship of that name, or its
/// two sides do not name the same number of columns.
#[must_use]
pub fn evidence_sql(model: &Model, relationship: &str) -> String {
    let Some(r) = model.relationships.iter().find(|r| r.name == relationship) else {
        return String::new();
    };
    if r.from_columns.is_empty() || r.from_columns.len() != r.to_columns.len() {
        return String::new();
    }
    // Each side's key columns renamed k0, k1, … so the two sides compare position by position.
    let keys = |columns: &[String]| -> String {
        columns
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{} AS k{i}", quoted(c)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let n = r.from_columns.len();
    let all_present = (0..n)
        .map(|i| format!("k{i} IS NOT NULL"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let matched = (0..n)
        .map(|i| format!("c.k{i} = p.k{i}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let key_list = (0..n)
        .map(|i| format!("k{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "WITH child AS (SELECT {child_keys} FROM {child}), \
         parent AS (SELECT {parent_keys} FROM {parent}), \
         keyed AS (SELECT * FROM child WHERE {all_present}), \
         parent_keys AS (SELECT DISTINCT * FROM parent WHERE {all_present}), \
         child_keys AS (SELECT DISTINCT * FROM keyed) \
         SELECT \
         (SELECT count(*) FROM child) AS child_rows, \
         (SELECT count(*) FROM keyed AS c ANTI JOIN parent_keys AS p ON {matched}) AS orphans, \
         coalesce((SELECT count(*) FROM child_keys AS c SEMI JOIN parent_keys AS p ON {matched})::DOUBLE \
         / nullif((SELECT count(*) FROM child_keys), 0), 0.0) AS value_overlap, \
         (SELECT count(*) FROM (SELECT {key_list} FROM parent WHERE {all_present} \
         GROUP BY {key_list} HAVING count(*) > 1)) AS parent_duplicates",
        child_keys = keys(&r.from_columns),
        parent_keys = keys(&r.to_columns),
        child = quoted(&r.from),
        parent = quoted(&r.to),
    )
}

/// `model` with `measured` as `relationship`'s evidence, and its warning set to what the
/// measurement says is wrong with the join, or cleared when nothing is.
///
/// A join graded likely warns when child rows point at no parent row; a join whose cardinality
/// says one parent per child (`N:1` or `1:1`) warns when parent key values repeat. The grade,
/// who decided the join and how it was verified stay as they are. `model` comes back unchanged
/// when it has no relationship of that name.
#[must_use]
pub fn apply_evidence(model: &Model, relationship: &str, measured: Evidence) -> Model {
    let mut out = model.clone();
    if let Some(r) = out
        .relationships
        .iter_mut()
        .find(|r| r.name == relationship)
    {
        r.meta.warning = warning(r, &measured);
        r.meta.evidence = measured;
    }
    out
}

/// What `measured` says is wrong with `r`, in words that name its tables:
/// `12 transactions rows now point at no customers row`.
fn warning(r: &Relationship, measured: &Evidence) -> Option<String> {
    let mut said = Vec::new();
    if r.meta.grade == Grade::Likely && measured.orphans > 0 {
        let n = measured.orphans;
        said.push(format!(
            "{n} {} {} now {} at no {} row",
            r.from,
            if n == 1 { "row" } else { "rows" },
            if n == 1 { "points" } else { "point" },
            r.to
        ));
    }
    let one_parent = matches!(r.meta.cardinality.as_str(), "N:1" | "1:1");
    if one_parent && measured.parent_duplicates > 0 {
        let n = measured.parent_duplicates;
        said.push(format!(
            "{n} {} {} now {} in {}",
            r.to_columns.join(", "),
            if n == 1 { "value" } else { "values" },
            if n == 1 { "repeats" } else { "repeat" },
            r.to
        ));
    }
    (!said.is_empty()).then(|| said.join("; "))
}
