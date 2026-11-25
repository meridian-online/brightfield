//! `cast-column` in the window: the type typed in place over the column's
//! storage label in its header, starting from the type FineType reads.
//!
//! It starts from a click on a header's type (the FineType glyph and the
//! storage label), from `shift-t` with the grid's cursor on a cell, or from
//! "Change type" on a header's menu. Its field opens over the storage label
//! with the DuckDB type FineType gives the column's label, selected whole,
//! when that differs from the type the column is stored as; otherwise it
//! opens empty. Under it are that type, then the common types, less the one
//! the column already has and filtered by what is typed. `column` is given
//! bare, since arc takes it as a name; `type` is SQL.
//!
//! Each edit runs arc's model, so the column's cells re-read as the type at
//! each keystroke. arc's cast is a `CAST`, not a `TRY_CAST`, so a value that
//! will not convert gives DuckDB's reason, with the last good rows faded, as
//! running the step would. The step is named for the column it cast
//! (`<table>_cast_<column>`).

use super::{
    Begin, By, Draft, HeaderPart, InPlace, Keeps, NameRule, Need, OperationUi, Stage, Start,
};

/// The types offered after FineType's, in the order they are listed.
const COMMON: &[&str] = &[
    "VARCHAR",
    "BIGINT",
    "DOUBLE",
    "DECIMAL(18, 3)",
    "DATE",
    "TIMESTAMP",
    "BOOLEAN",
];

/// A type's spelling for comparing two: upper case, with no spaces, so
/// `decimal(18,3)` and `DECIMAL(18, 3)` are one type.
fn spelled(sql_type: &str) -> String {
    sql_type
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase()
}

/// The column's storage type, and the DuckDB type FineType gives its label,
/// as far as the window knows them.
fn types_of<'b>(b: &'b Begin<'_>) -> (Option<&'b str>, Option<&'b str>) {
    let Some(facts) = b
        .column
        .and_then(|column| b.facts.iter().find(|f| f.column == column))
    else {
        return (None, None);
    };
    let fine_type = facts
        .label
        .as_deref()
        .and_then(|label| b.transforms.duckdb_type(label));
    (Some(facts.storage.as_str()), fine_type)
}

/// The type FineType reads for the column, when it is not the one the column
/// is stored as.
fn fine_type_default(b: &Begin<'_>) -> Option<String> {
    let (storage, fine_type) = types_of(b);
    let fine_type = fine_type?;
    (storage.map(spelled) != Some(spelled(fine_type))).then(|| fine_type.to_string())
}

fn begin(b: &Begin<'_>) -> Result<Draft, String> {
    let column = b
        .column
        .ok_or_else(|| "Cast column needs a column.".to_string())?;
    let mut draft = Draft::empty("cast-column");
    draft.on = b.table.map(str::to_string);
    draft.set("column", column.to_string());
    draft.set("type", fine_type_default(b).unwrap_or_default());
    draft.focus = Some("type");
    Ok(draft)
}

/// FineType's type first, then the common types, less the column's storage
/// type, each once. While the field holds no text, or FineType's type as it
/// opened, the whole list is offered; once something else is typed, the
/// types that hold it.
fn suggestions(b: &Begin<'_>, draft: &Draft) -> Vec<String> {
    let storage = types_of(b).0.map(spelled);
    let first = fine_type_default(b);
    let typed = spelled(draft.value("type").unwrap_or_default());
    let whole_list = typed.is_empty() || first.as_deref().map(spelled).as_ref() == Some(&typed);
    let mut offered: Vec<String> = Vec::new();
    for candidate in first
        .iter()
        .map(String::as_str)
        .chain(COMMON.iter().copied())
    {
        let candidate_spelled = spelled(candidate);
        if storage.as_ref() == Some(&candidate_spelled)
            || offered.iter().any(|o| spelled(o) == candidate_spelled)
        {
            continue;
        }
        if whole_list || candidate_spelled.contains(&typed) {
            offered.push(candidate.to_string());
        }
    }
    offered
}

static UI: OperationUi = OperationUi {
    long_name: "cast-column",
    verb: "cast-column",
    stage: Stage::Shape,
    icon: "transform",
    starts: &[
        Start::Cell,
        Start::Header(HeaderPart::Type),
        Start::Header(HeaderPart::Menu),
    ],
    menu_words: "Change type",
    needs: &[
        Need::ProtocolSource,
        Need::Rows,
        Need::Column,
        Need::ViewRecorded,
    ],
    begin,
    keeps: Keeps::OnEnter,
    field: Some(InPlace {
        part: HeaderPart::Type,
        keys: &["type"],
        suggestions,
        follows: None,
    }),
    form: None,
    name: NameRule {
        word: "cast",
        from: By::Name("column"),
        with_by: false,
    },
};

/// `cast-column`'s slot.
#[must_use]
pub fn ui() -> Option<&'static OperationUi> {
    Some(&UI)
}
