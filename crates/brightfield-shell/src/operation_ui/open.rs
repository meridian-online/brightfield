//! What the window holds, as far as starting an operation asks, and whether
//! an operation can start.
//!
//! An operation declares what it needs to start in the key registry, on the
//! entry of the verb that starts it (`brightfield_keys::operations`), as a
//! command of the registry does. [`hold`] adds what [`OperationContext`] gathers to the
//! window's [`Situation`], and [`availability`] reads the operation's verb
//! against it. So a key press, a palette row, a header's menu row and a menu
//! item read one declaration through one evaluator, and say one sentence
//! (`its_key_its_menu_row_and_its_palette_row_say_the_same_sentence` in
//! `tests/operation_availability.rs`).
//!
//! A command that cannot start is still listed and keeps its key: what is
//! greyed is a missing input, and the sentence names it.

use std::path::PathBuf;

use brightfield_keys::{Availability, Condition, Need, Requirement, Selected, Situation};

use super::{label, OperationUi};
use crate::one_step::ColumnFacts;

/// What the window holds, as far as starting an operation asks.
#[derive(Clone, Debug, Default)]
pub struct OperationContext {
    /// Whether the window has arc to record through.
    pub runner: bool,
    /// The folder of the Protocol a Save records into, when the window holds a
    /// Protocol of the analyst's own.
    pub protocol: Option<PathBuf>,
    /// Whether the Protocol is one Brightfield made, which takes new sources.
    pub takes_sources: bool,
    /// The table whose grid is in front, when one is.
    pub in_front: Option<InFront>,
    /// Each table of the Protocol, recorded or kept, with its columns once
    /// they are known.
    pub tables: Vec<(String, Option<Vec<String>>)>,
}

/// The table whose grid is in front.
#[derive(Clone, Debug, Default)]
pub struct InFront {
    /// The table's name.
    pub table: String,
    /// Whether its rows are held: the loaded table, and not a step's preview.
    pub loaded: bool,
    /// Whether its rows are drawn as rows.
    pub rows: bool,
    /// Whether the grid's cursor is on a cell.
    pub cell: bool,
    /// Whether a `where` or an `order by` is applied and not recorded.
    pub view_unrecorded: bool,
    /// The table's columns, in order.
    pub columns: Vec<String>,
    /// What Brightfield knows of each column.
    pub facts: Vec<ColumnFacts>,
    /// The column the grid's cursor is on.
    pub cursor_column: Option<String>,
    /// What a step's preview says to a key that asks to filter or sort it,
    /// for a table that is not loaded.
    pub step_offer: Option<String>,
}

/// The needs `spec` declares in the key registry, on its verb's entry, in the
/// order their sentences are said. None for a verb the registry lacks.
#[must_use]
pub fn needs(spec: &OperationUi) -> &'static [Requirement] {
    brightfield_keys::registry()
        .iter()
        .find(|v| v.longname == spec.verb)
        .map_or(&[], |v| v.needs)
}

/// What `spec` is called where a command is listed: its verb's label in the
/// key registry, and its long name in words for a verb the registry lacks.
#[must_use]
pub fn command(spec: &OperationUi) -> String {
    brightfield_keys::registry()
        .iter()
        .find(|v| v.longname == spec.verb)
        .map_or_else(|| label(spec.long_name), |v| v.label.to_string())
}

/// The sentence `spec` says when `need` is unmet: the one its verb declares,
/// and the need's own otherwise.
#[must_use]
pub fn sentence(spec: &OperationUi, need: Need) -> String {
    brightfield_keys::sentence(spec.verb, need)
        .unwrap_or_else(|| need.default_sentence().replace("{command}", &command(spec)))
}

/// **Add to `situation` what `cx` holds**, for an operation started on
/// `column`, or on the grid cursor's column when `column` is `None`: the
/// grid's rows and column, and the conditions an operation asks after.
pub fn hold(situation: &mut Situation, cx: &OperationContext, column: Option<&str>) {
    let front = cx.in_front.as_ref();
    let column = column.or_else(|| front.and_then(|f| f.cursor_column.as_deref()));
    if front.is_some_and(|f| f.rows) {
        situation.selected.push(Selected::GridRows);
    }
    if column.is_some() {
        situation.selected.push(Selected::GridColumn);
    }
    let mut when = |holds: bool, condition: Condition| {
        if holds {
            situation.conditions.push(condition);
        }
    };
    when(cx.protocol.is_some(), Condition::ProtocolSource);
    when(
        column.is_some_and(|name| {
            front.is_some_and(|f| f.facts.iter().any(|c| c.column == name && c.nulls > 0))
        }),
        Condition::ColumnHasNulls,
    );
    when(
        !front.is_some_and(|f| f.view_unrecorded),
        Condition::ViewRecorded,
    );
    when(front.is_some_and(|f| f.loaded), Condition::LoadedTable);
    when(others(cx).next().is_some(), Condition::AnotherTable);
    let mine: Vec<String> = front
        .map(|f| f.columns.iter().map(|c| c.to_lowercase()).collect())
        .unwrap_or_default();
    // A table whose columns are not known yet is not ruled out: arc and DuckDB
    // say whether it matches once it is picked.
    when(
        others(cx).any(|(_, columns)| {
            columns
                .as_ref()
                .is_none_or(|theirs| theirs.iter().any(|c| mine.contains(&c.to_lowercase())))
        }),
        Condition::AnotherTableSharingAColumn,
    );
    // A step's preview says how to load its table in its own words.
    if let Some(offer) = front.and_then(|f| f.step_offer.as_deref()) {
        situation
            .instead
            .push((Need::Condition(Condition::LoadedTable), offer.to_string()));
    }
}

/// The Protocol's tables other than the one in front.
fn others(cx: &OperationContext) -> impl Iterator<Item = &(String, Option<Vec<String>>)> {
    let front = cx.in_front.as_ref().map(|f| f.table.to_lowercase());
    cx.tables
        .iter()
        .filter(move |(table, _)| Some(table.to_lowercase()) != front)
}

/// Whether `spec` can start where the window holds `situation`: its verb's
/// declared needs in their order, and the sentence of the first unmet
/// (`availability_names_the_first_need_it_lacks` in
/// `tests/operation_availability.rs`). An operation whose verb the registry
/// lacks needs nothing.
#[must_use]
pub fn availability(spec: &OperationUi, situation: &Situation) -> Availability {
    brightfield_keys::registry()
        .iter()
        .find(|v| v.longname == spec.verb)
        .map_or(Availability::Available, |verb| {
            brightfield_keys::availability(verb, situation)
        })
}
