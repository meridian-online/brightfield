//! `hide-column` in the window: a direct action on a column.
//!
//! It starts from `-` with the grid's cursor on a cell, or from "Hide column"
//! on a column header's menu. It asks nothing: its one value is the column it
//! started on, given bare since arc takes it as a name, and the draft is kept
//! as soon as arc admits it and DuckDB has run it ([`Keeps::AtOnce`]). The
//! step's table then comes to the front without the column, and the status
//! band says what was hidden and the key that takes it back.

use super::{Begin, By, Draft, HeaderPart, Keeps, NameRule, Need, OperationUi, Stage, Start};

fn begin(b: &Begin<'_>) -> Result<Draft, String> {
    let column = b
        .column
        .ok_or_else(|| "Hide column needs a column.".to_string())?;
    let mut draft = Draft::empty("hide-column");
    draft.on = b.table.map(str::to_string);
    draft.set("column", column.to_string());
    Ok(draft)
}

static UI: OperationUi = OperationUi {
    long_name: "hide-column",
    verb: "hide-column",
    stage: Stage::Shape,
    icon: "eye-off",
    starts: &[Start::Cell, Start::Header(HeaderPart::Menu)],
    menu_words: "Hide column",
    needs: &[
        Need::ProtocolSource,
        Need::Rows,
        Need::Column,
        Need::ViewRecorded,
    ],
    begin,
    keeps: Keeps::AtOnce,
    field: None,
    form: None,
    name: NameRule {
        word: "without",
        from: By::Value("column"),
        with_by: false,
    },
};

/// `hide-column`'s slot.
#[must_use]
pub fn ui() -> Option<&'static OperationUi> {
    Some(&UI)
}
