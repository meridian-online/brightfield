//! Whether an operation can start, and the sentence it says when it cannot.
//!
//! An operation declares what the window must hold for it to start
//! ([`Need`]). [`availability`] checks those against what the window holds
//! now ([`OperationContext`]), in the order the operation lists them, and
//! answers with the sentence for the first one unmet. The same answer serves
//! a key press, a palette row, a header's menu row and a menu item, so a
//! command says one thing wherever it is asked.
//!
//! A command that cannot start is still listed and keeps its key: what is
//! greyed is a missing input, and the sentence names it.

use std::path::PathBuf;

use super::{label, Need, OperationUi};
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

/// Whether an operation can start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    /// It can.
    Available,
    /// It cannot, and this sentence says what it needs.
    Unavailable(String),
}

impl Availability {
    /// The sentence, when the operation cannot start.
    #[must_use]
    pub fn sentence(&self) -> Option<&str> {
        match self {
            Self::Available => None,
            Self::Unavailable(why) => Some(why),
        }
    }
}

/// The sentence the need `need` says for the command `command` when it is
/// unmet, with `offer` in place of the loaded-table sentence when a step's
/// preview has its own.
#[must_use]
pub fn sentence(need: Need, command: &str, offer: Option<&str>) -> String {
    match need {
        Need::ProtocolSource => {
            format!("{command} needs a Protocol of your own: open a data file or a folder.")
        }
        Need::TakesSources => {
            "Sources are added to Protocols Brightfield made, and this one was not.".to_string()
        }
        Need::Rows => format!("{command} needs a table's rows."),
        Need::GridCell => format!("{command} needs the grid's cursor on a cell."),
        Need::Column => format!(
            "{command} needs a column: put the grid's cursor on one, or start from its header."
        ),
        Need::ColumnHasNulls => format!("{command} needs a column with missing values."),
        Need::ViewRecorded => format!(
            "{command} reads the table as recorded: save or clear the where and order by first."
        ),
        Need::LoadedTable => offer.map_or_else(
            || format!("{command} needs a table that has been loaded or run."),
            str::to_string,
        ),
        Need::AnotherTable => format!("{command} needs a second table in the Protocol."),
        Need::AnotherTableSharingAColumn => {
            format!("{command} needs a second table with matching columns.")
        }
    }
}

/// Whether `cx` meets `need`, for an operation started on `column`.
fn met(need: Need, cx: &OperationContext, column: Option<&str>) -> bool {
    let front = cx.in_front.as_ref();
    let on_column = || column.or_else(|| front.and_then(|f| f.cursor_column.as_deref()));
    match need {
        Need::ProtocolSource => cx.protocol.is_some(),
        Need::TakesSources => cx.takes_sources,
        Need::Rows => front.is_some_and(|f| f.rows),
        Need::GridCell => front.is_some_and(|f| f.cell),
        Need::Column => on_column().is_some(),
        Need::ColumnHasNulls => on_column().is_some_and(|name| {
            front.is_some_and(|f| f.facts.iter().any(|c| c.column == name && c.nulls > 0))
        }),
        Need::ViewRecorded => !front.is_some_and(|f| f.view_unrecorded),
        Need::LoadedTable => front.is_some_and(|f| f.loaded),
        Need::AnotherTable => others(cx).next().is_some(),
        Need::AnotherTableSharingAColumn => {
            let mine: Vec<String> = front
                .map(|f| f.columns.iter().map(|c| c.to_lowercase()).collect())
                .unwrap_or_default();
            // A table whose columns are not known yet is not ruled out: arc and
            // DuckDB say whether it matches once it is picked.
            others(cx).any(|(_, columns)| {
                columns
                    .as_ref()
                    .is_none_or(|theirs| theirs.iter().any(|c| mine.contains(&c.to_lowercase())))
            })
        }
    }
}

/// The Protocol's tables other than the one in front.
fn others(cx: &OperationContext) -> impl Iterator<Item = &(String, Option<Vec<String>>)> {
    let front = cx.in_front.as_ref().map(|f| f.table.to_lowercase());
    cx.tables
        .iter()
        .filter(move |(table, _)| Some(table.to_lowercase()) != front)
}

/// Whether `spec` can start in `cx`, on `column` when it is started on one:
/// its needs in the order it lists them, and the sentence of the first unmet
/// (`availability_names_the_first_need_it_lacks` in
/// `tests/operation_start.rs`).
#[must_use]
pub fn availability(
    spec: &OperationUi,
    cx: &OperationContext,
    column: Option<&str>,
) -> Availability {
    let command = label(spec.long_name);
    let offer = cx.in_front.as_ref().and_then(|f| f.step_offer.as_deref());
    spec.needs
        .iter()
        .find(|need| !met(**need, cx, column))
        .map_or(Availability::Available, |need| {
            Availability::Unavailable(sentence(*need, &command, offer))
        })
}
