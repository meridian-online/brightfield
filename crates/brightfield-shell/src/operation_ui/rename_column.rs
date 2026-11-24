//! `rename-column` in the window: the new name typed in place over the column's
//! name in its header.
//!
//! It starts from a double-click on a header's name, from `shift-r` with the
//! grid's cursor on a cell, or from "Rename column" on a header's menu. Its
//! field opens over the name with the column's own name in it, selected whole,
//! so the first letter typed replaces it. `column` is given bare, since arc
//! takes it as a name, and so is `to`.
//!
//! Each edit runs arc's model, so the header follows the text as typed. A name
//! the table already has, or one left blank, keeps the last good rows faded
//! with the reason on the status band, and Enter keeps nothing until the name
//! is one arc and DuckDB take. The step is named for the column it renamed
//! (`<table>_renamed_<column>`).

use super::{
    Begin, By, Draft, HeaderPart, InPlace, Keeps, NameRule, Need, OperationUi, Stage, Start,
};

fn begin(b: &Begin<'_>) -> Result<Draft, String> {
    let column = b
        .column
        .ok_or_else(|| "Rename column needs a column.".to_string())?;
    let mut draft = Draft::empty("rename-column");
    draft.on = b.table.map(str::to_string);
    draft.set("column", column.to_string());
    draft.set("to", column.to_string());
    draft.focus = Some("to");
    Ok(draft)
}

/// A rename offers nothing under its field: the name is the analyst's own.
fn no_suggestions(_: &Begin<'_>, _: &Draft) -> Vec<String> {
    Vec::new()
}

static UI: OperationUi = OperationUi {
    long_name: "rename-column",
    verb: "rename-column",
    stage: Stage::Shape,
    icon: "cursor-text",
    starts: &[
        Start::Cell,
        Start::Header(HeaderPart::Name),
        Start::Header(HeaderPart::Menu),
    ],
    menu_words: "Rename column",
    needs: &[
        Need::ProtocolSource,
        Need::Rows,
        Need::Column,
        Need::ViewRecorded,
    ],
    begin,
    keeps: Keeps::OnEnter,
    field: Some(InPlace {
        part: HeaderPart::Name,
        keys: &["to"],
        suggestions: no_suggestions,
        follows: None,
    }),
    form: None,
    name: NameRule {
        word: "renamed",
        from: By::Name("column"),
        with_by: false,
    },
};

/// `rename-column`'s slot.
#[must_use]
pub fn ui() -> Option<&'static OperationUi> {
    Some(&UI)
}
