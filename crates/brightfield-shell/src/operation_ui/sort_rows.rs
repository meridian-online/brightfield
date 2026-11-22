//! `sort-rows` in the window: an order typed in the `order by` field of the
//! grid's foot row, opened with the cursor's column in it.
//!
//! As with `filter-rows`, the foot row is where a simple `order by` is
//! written: the field applies the order to the grid as a view, and a Save
//! records the view as the step.

use super::{By, Draft, Keeps, NameRule, Need, OperationUi, Stage, Start};
use crate::grid_foot::FootField;

fn begin(_: &super::Begin<'_>) -> Result<Draft, String> {
    Ok(Draft::empty("sort-rows"))
}

static UI: OperationUi = OperationUi {
    long_name: "sort-rows",
    verb: "sort-rows-at-cursor",
    stage: Stage::Shape,
    icon: "arrows-sort",
    starts: &[Start::FootField(FootField::OrderBy)],
    menu_words: "Sort rows",
    needs: &[Need::ProtocolSource, Need::GridCell, Need::LoadedTable],
    begin,
    keeps: Keeps::OnEnter,
    field: None,
    form: None,
    name: NameRule {
        word: "sorted",
        from: By::Value("order_by"),
        with_by: true,
    },
};

/// `sort-rows`' slot.
#[must_use]
pub fn ui() -> Option<&'static OperationUi> {
    Some(&UI)
}
