//! `filter-rows` in the window: a condition typed in the `where` field of the
//! grid's foot row, opened with the cursor's column equal to its cell's value.
//!
//! The foot row is where a simple `where` is written, so this operation has no
//! form and no draft of the kind the others have: its field applies the
//! condition to the grid as a view, and a Save records the view as the step
//! ([`crate::window::MeridianApp::record_pending`]), named by the rule the grid's views are named by.

use super::{By, Draft, Keeps, NameRule, OperationUi, Stage, Start};
use crate::grid_foot::FootField;

fn begin(_: &super::Begin<'_>) -> Result<Draft, String> {
    Ok(Draft::empty("filter-rows"))
}

static UI: OperationUi = OperationUi {
    long_name: "filter-rows",
    verb: "filter-rows-at-cursor",
    stage: Stage::Shape,
    icon: "filter",
    starts: &[Start::FootField(FootField::Where)],
    menu_words: "Filter rows",
    // No Protocol is asked for: the field applies a view, which a window with no Protocol
    // of its own shows too, and a click on the field opens it there.
    begin,
    keeps: Keeps::OnEnter,
    field: None,
    form: None,
    name: NameRule {
        word: "filtered",
        from: By::Value("where"),
        with_by: true,
    },
};

/// `filter-rows`' slot.
#[must_use]
pub fn ui() -> Option<&'static OperationUi> {
    Some(&UI)
}
