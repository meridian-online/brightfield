//! `fill-nulls` in the window: the value a column's missing values are filled
//! with, typed in place over the header's validity band.
//!
//! It starts from a click on the header's missing count, from `f` with the
//! grid's cursor on a cell, or from "Fill missing values" on a header's menu,
//! and only on a column that has missing values: on any other it says so
//! (`Condition::ColumnHasNulls` on its key-registry entry,
//! `fill_nulls_is_greyed_on_a_column_with_no_missing_values`). `column` is
//! given bare, since arc takes it as a name; `value` is SQL, recorded as typed.
//!
//! The value opens on a fill that suits the column's storage type, selected
//! whole: `0` for a number, `'unknown'` for text, `false` for a boolean, and
//! nothing for any other type. For a number the field also offers the column's
//! median. Each edit runs arc's model, so the cells fill and the preview's
//! missing count for the column falls to none as the value is typed. A value
//! DuckDB cannot combine with the column, or one arc refuses, keeps the last
//! good rows faded with the reason on the status band, and Enter keeps nothing
//! until the value is one both take. The step is named for the column it
//! filled (`<table>_filled_<column>`).
//!
//! Which columns have missing values is known for the loaded table's grid
//! alone, so a fill does not start over a step's preview, a kept fill's
//! included (`fill_nulls_over_a_kept_fills_grid_says_it_needs_missing_values`).

use super::{Begin, By, Draft, HeaderPart, InPlace, Keeps, NameRule, OperationUi, Stage, Start};
use crate::one_step::ColumnFacts;

/// What a DuckDB storage type is, as far as a fill's default goes.
#[derive(Debug, PartialEq, Eq)]
enum Kind {
    /// A whole number: a fill with a fraction would widen it.
    Integer,
    /// A number that holds fractions.
    Fractional,
    Text,
    Boolean,
    Other,
}

/// The kind of the storage type `storage`, as DuckDB names it.
fn kind(storage: &str) -> Kind {
    let upper = storage.trim().to_ascii_uppercase();
    let base = upper.split('(').next().unwrap_or_default().trim();
    match base {
        "TINYINT" | "SMALLINT" | "INTEGER" | "INT" | "BIGINT" | "HUGEINT" | "UTINYINT"
        | "USMALLINT" | "UINTEGER" | "UBIGINT" | "UHUGEINT" => Kind::Integer,
        "FLOAT" | "REAL" | "DOUBLE" | "DECIMAL" | "NUMERIC" => Kind::Fractional,
        "VARCHAR" | "TEXT" | "STRING" => Kind::Text,
        "BOOLEAN" | "BOOL" => Kind::Boolean,
        _ => Kind::Other,
    }
}

/// The fill a column of `storage` opens on.
fn default_fill(storage: &str) -> &'static str {
    match kind(storage) {
        Kind::Integer | Kind::Fractional => "0",
        Kind::Text => "'unknown'",
        Kind::Boolean => "false",
        Kind::Other => "",
    }
}

/// The median a column of `storage` is offered as a fill, as SQL writes it:
/// rounded to a whole number for an integer column, so taking it keeps the
/// column's type, and exact for any other number. `None` for a column that is
/// not a number, and for a median that is not finite.
fn median_fill(storage: &str, median: f64) -> Option<String> {
    if !median.is_finite() {
        return None;
    }
    match kind(storage) {
        Kind::Integer => Some(median.round().to_string()),
        Kind::Fractional => Some(median.to_string()),
        Kind::Text | Kind::Boolean | Kind::Other => None,
    }
}

/// What Brightfield knows of the column the draft fills.
fn facts_of<'a>(b: &'a Begin<'_>, draft: &Draft) -> Option<&'a ColumnFacts> {
    let column = draft.value("column")?;
    b.facts.iter().find(|f| f.column == column)
}

fn begin(b: &Begin<'_>) -> Result<Draft, String> {
    let column = b
        .column
        .ok_or_else(|| "Fill nulls needs a column.".to_string())?;
    let mut draft = Draft::empty("fill-nulls");
    draft.on = b.table.map(str::to_string);
    draft.set("column", column.to_string());
    let storage = facts_of(b, &draft).map_or("", |f| f.storage.as_str());
    draft.set("value", default_fill(storage).to_string());
    draft.focus = Some("value");
    Ok(draft)
}

/// The fills offered under the field: the storage type's default, then a
/// number's median, less the value as it stands.
fn suggestions(b: &Begin<'_>, draft: &Draft) -> Vec<String> {
    let Some(facts) = facts_of(b, draft) else {
        return Vec::new();
    };
    let mut offered = Vec::new();
    let default = default_fill(&facts.storage);
    if !default.is_empty() {
        offered.push(default.to_string());
    }
    if let Some(median) = facts
        .moments
        .as_ref()
        .and_then(|m| median_fill(&facts.storage, m.median))
    {
        if !offered.contains(&median) {
            offered.push(median);
        }
    }
    let typed = draft.value("value").unwrap_or_default().trim();
    offered.retain(|fill| fill != typed);
    offered
}

static UI: OperationUi = OperationUi {
    long_name: "fill-nulls",
    verb: "fill-nulls",
    stage: Stage::Shape,
    icon: "bucket-droplet",
    starts: &[
        Start::Cell,
        Start::Header(HeaderPart::Missing),
        Start::Header(HeaderPart::Menu),
    ],
    menu_words: "Fill missing values",
    begin,
    keeps: Keeps::OnEnter,
    field: Some(InPlace {
        part: HeaderPart::Missing,
        keys: &["value"],
        suggestions,
        follows: None,
    }),
    form: None,
    name: NameRule {
        word: "filled",
        from: By::Name("column"),
        with_by: false,
    },
};

/// `fill-nulls`'s slot.
#[must_use]
pub fn ui() -> Option<&'static OperationUi> {
    Some(&UI)
}

#[cfg(test)]
mod tests {
    use super::{default_fill, median_fill};

    #[test]
    fn a_fill_opens_on_a_value_its_storage_type_takes() {
        for (storage, fill) in [
            ("BIGINT", "0"),
            ("DOUBLE", "0"),
            ("DECIMAL(18,3)", "0"),
            ("integer", "0"),
            ("VARCHAR", "'unknown'"),
            ("BOOLEAN", "false"),
            ("DATE", ""),
            ("TIMESTAMP", ""),
            ("", ""),
        ] {
            assert_eq!(default_fill(storage), fill, "{storage}");
        }
    }

    #[test]
    fn a_median_is_offered_whole_for_an_integer_column_and_exact_otherwise() {
        assert_eq!(median_fill("BIGINT", 34.0).as_deref(), Some("34"));
        // An even count of values can give an integer column a fractional median.
        assert_eq!(median_fill("BIGINT", 2.5).as_deref(), Some("3"));
        assert_eq!(median_fill("INTEGER", -2.0).as_deref(), Some("-2"));
        assert_eq!(median_fill("DOUBLE", 2.3125).as_deref(), Some("2.3125"));
        assert_eq!(median_fill("DECIMAL(18,3)", 34.0).as_deref(), Some("34"));
        assert_eq!(median_fill("VARCHAR", 1.0), None);
        assert_eq!(median_fill("DOUBLE", f64::NAN), None);
        assert_eq!(median_fill("DOUBLE", f64::INFINITY), None);
    }
}
