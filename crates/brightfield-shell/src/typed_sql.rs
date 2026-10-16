//! Typed SQL in the chart palette: text on the query line that reads as a
//! statement is previewed and, when kept, becomes a step. This module holds the
//! parts of that which need no window.

use std::collections::BTreeSet;

use brightfield_engine::StatementReading;

use crate::generated::sql_step_name_for;

/// The name for a step made from the statement `reading` describes: the first
/// table it reads and the column it turns on, made free of `taken`.
#[must_use]
pub fn typed_step_name(reading: &StatementReading, taken: &BTreeSet<String>) -> String {
    sql_step_name_for(
        reading.tables.first().map(String::as_str),
        reading.name_column.as_deref(),
        taken,
    )
}
