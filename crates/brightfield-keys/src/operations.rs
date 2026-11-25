//! One key-registry entry for each of arc's operations that starts from the
//! grid, each behind a constant of its own.
//!
//! An operation's verb is arc's long name for it (`hide-column`), and its key
//! is chosen here once, for every operation at the same time, so that two
//! operations built side by side cannot pick the same key. An entry joins
//! [`crate::registry::registry`] when its `*_BUILT` constant is true, which is
//! when the window can perform it: a key the registry lists is a key that
//! acts. Until then the long name is no verb at all, so neither the palette
//! nor the help sheet shows a command that does nothing.
//!
//! Each operation is a block of its own, its constant directly above its entry
//! function, so that building one operation changes one line and two built at
//! once never change neighbouring lines.
//!
//! Three of arc's operations have no entry here. `filter-rows` and `sort-rows`
//! start from the registry's own `filter-rows-at-cursor` and
//! `sort-rows-at-cursor`, and `read-file` from `open-sources-browser`.
//!
//! An entry is a [`CommandTier::View`] command
//! (`an_operations_verb_is_its_long_name_and_writes_nothing`): starting an
//! operation previews it and keeps a draft, and a Save does the writing.

use crate::altitude::Altitude;
use crate::availability::{Condition, Need, Requirement, Selected};
use crate::registry::{
    BindingContext, BindingSpec, CommandTier, Drives, Scores, VerbEntry, VerbStatus,
};

/// A key in the grid's context.
fn grid(keystrokes: &'static str) -> BindingSpec {
    BindingSpec {
        keystrokes,
        context: BindingContext::Grid,
    }
}

/// What an operation started from the grid needs: a Protocol its step is
/// saved into, and the grid's cursor on a cell.
const NEEDS_OPERATION: &[Requirement] = &[
    Requirement::of(Need::Condition(Condition::ProtocolToSave)),
    Requirement::of(Need::Selected(Selected::GridCell)),
];

/// An operation's entry: its long name as the verb, one key in the grid's
/// context, its help line, and the scores that defend the key.
fn entry(
    longname: &'static str,
    label: &'static str,
    key: &'static str,
    help: &'static str,
    (frequency, mnemonic, convention): (u8, u8, u8),
    motor_note: &'static str,
) -> VerbEntry {
    VerbEntry {
        longname,
        label,
        needs: NEEDS_OPERATION,
        tier: CommandTier::View,
        binding_specs: vec![grid(key)],
        scope_applicability: vec![Altitude::View],
        drives: Drives::Navigation,
        status: VerbStatus::Built,
        reserved_reason: None,
        help,
        scores: Some(Scores {
            frequency,
            mnemonic,
            convention,
            motor_note,
        }),
    }
}

// ---- hide-column ----
const HIDE_COLUMN_BUILT: bool = true;
fn hide_column() -> VerbEntry {
    entry(
        "hide-column",
        "Hide column",
        "-",
        "Hide the cursor's column; its header's menu does the same",
        (3, 4, 4),
        "- is VisiData's hide; the Workspace's - zooms in its own context",
    )
}

// ---- rename-column ----
const RENAME_COLUMN_BUILT: bool = true;
fn rename_column() -> VerbEntry {
    entry(
        "rename-column",
        "Rename column",
        "shift-r",
        "Rename the cursor's column in its header; a double-click on the name does the same",
        (3, 4, 2),
        "R = rename; bare r stays free for the grid's movement",
    )
}

// ---- derive-column ----
const DERIVE_COLUMN_BUILT: bool = true;
fn derive_column() -> VerbEntry {
    entry(
        "derive-column",
        "Derive column",
        "=",
        "Add a column from an expression, typed in a new header at the table's end",
        (3, 4, 4),
        "= is VisiData's new column from an expression",
    )
}

// ---- cast-column ----
const CAST_COLUMN_BUILT: bool = true;
fn cast_column() -> VerbEntry {
    entry(
        "cast-column",
        "Cast column",
        "shift-t",
        "Change the cursor's column's type, starting from the type FineType reads",
        (2, 4, 2),
        "T = type; retype-column is the model's, in its own context",
    )
}

// ---- summarise-rows ----
const SUMMARISE_ROWS_BUILT: bool = true;
fn summarise_rows() -> VerbEntry {
    entry(
        "summarise-rows",
        "Summarise rows",
        "shift-f",
        "Summarise the rows by the cursor's column: a count of each value, then the values to add",
        (3, 3, 5),
        "F is VisiData's frequency table of the cursor's column",
    )
}

// ---- dedupe-rows ----
const DEDUPE_ROWS_BUILT: bool = false;
fn dedupe_rows() -> VerbEntry {
    entry(
        "dedupe-rows",
        "Remove duplicate rows",
        "shift-u",
        "Remove the table's duplicate rows",
        (2, 3, 2),
        "U = unique; bare u is undo elsewhere",
    )
}

// ---- limit-rows ----
const LIMIT_ROWS_BUILT: bool = false;
fn limit_rows() -> VerbEntry {
    entry(
        "limit-rows",
        "Limit rows",
        "shift-l",
        "Keep the top rows by the cursor's column: ten, largest first, until changed",
        (2, 4, 2),
        "L = limit; bare l moves the cursor right",
    )
}

// ---- fill-nulls ----
const FILL_NULLS_BUILT: bool = true;
fn fill_nulls() -> VerbEntry {
    entry(
        "fill-nulls",
        "Fill missing values",
        "f",
        "Fill the cursor's column's missing values, starting from a suggested value",
        (2, 4, 3),
        "f is VisiData's fill; the grid's filter is w",
    )
}

// ---- join-tables ----
const JOIN_TABLES_BUILT: bool = false;
fn join_tables() -> VerbEntry {
    entry(
        "join-tables",
        "Join tables",
        "&",
        "Join the table in front to one picked in the spine, on keys proposed from the model or the cursor's column",
        (2, 4, 5),
        "& is VisiData's join",
    )
}

// ---- append-rows ----
const APPEND_ROWS_BUILT: bool = false;
fn append_rows() -> VerbEntry {
    entry(
        "append-rows",
        "Append rows",
        "shift-a",
        "Append the rows of tables picked in the spine, matching columns by name",
        (2, 4, 3),
        "A = append, as vim's A",
    )
}

/// Every operation's entry with whether it is built, in the order arc lists
/// its operations. The key-collision test reads the unbuilt ones too, so a key
/// is known to be free before its operation is built.
#[must_use]
pub fn all_operation_entries() -> Vec<(bool, VerbEntry)> {
    vec![
        (HIDE_COLUMN_BUILT, hide_column()),
        (RENAME_COLUMN_BUILT, rename_column()),
        (DERIVE_COLUMN_BUILT, derive_column()),
        (CAST_COLUMN_BUILT, cast_column()),
        (SUMMARISE_ROWS_BUILT, summarise_rows()),
        (DEDUPE_ROWS_BUILT, dedupe_rows()),
        (LIMIT_ROWS_BUILT, limit_rows()),
        (FILL_NULLS_BUILT, fill_nulls()),
        (JOIN_TABLES_BUILT, join_tables()),
        (APPEND_ROWS_BUILT, append_rows()),
    ]
}

/// The entries of the operations that are built, in arc's order: the ones
/// [`crate::registry::registry`] holds.
#[must_use]
pub fn operation_verbs() -> Vec<VerbEntry> {
    all_operation_entries()
        .into_iter()
        .filter_map(|(built, entry)| built.then_some(entry))
        .collect()
}

/// Whether `longname` is one of the operation verbs here, built or not.
#[must_use]
pub fn is_operation_verb(longname: &str) -> bool {
    all_operation_entries()
        .iter()
        .any(|(_, entry)| entry.longname == longname)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::registry;

    #[test]
    fn no_operation_key_is_taken_in_its_context() {
        let entries = all_operation_entries();
        let registry = registry();
        for (_, entry) in &entries {
            for spec in &entry.binding_specs {
                let same = |other: &VerbEntry| {
                    other.longname != entry.longname
                        && other
                            .binding_specs
                            .iter()
                            .any(|b| b.context == spec.context && b.keystrokes == spec.keystrokes)
                };
                let in_registry: Vec<&str> = registry
                    .iter()
                    .filter(|v| same(v))
                    .map(|v| v.longname)
                    .collect();
                assert!(
                    in_registry.is_empty(),
                    "{}'s `{}` is {in_registry:?}'s too",
                    entry.longname,
                    spec.keystrokes
                );
                let among: Vec<&str> = entries
                    .iter()
                    .filter(|(_, other)| same(other))
                    .map(|(_, other)| other.longname)
                    .collect();
                assert!(
                    among.is_empty(),
                    "{}'s `{}` is {among:?}'s too",
                    entry.longname,
                    spec.keystrokes
                );
            }
        }
    }

    #[test]
    fn only_built_operations_join_the_registry() {
        let built: Vec<&str> = all_operation_entries()
            .iter()
            .filter(|(built, _)| *built)
            .map(|(_, entry)| entry.longname)
            .collect();
        let verbs: Vec<&str> = operation_verbs().iter().map(|v| v.longname).collect();
        assert_eq!(verbs, built);
        let registry = registry();
        for (is_built, entry) in all_operation_entries() {
            let held = registry
                .iter()
                .filter(|v| v.longname == entry.longname)
                .count();
            assert_eq!(held, usize::from(is_built), "{}", entry.longname);
        }
    }

    #[test]
    fn an_operations_verb_is_its_long_name_and_writes_nothing() {
        let long_names = [
            "hide-column",
            "rename-column",
            "derive-column",
            "cast-column",
            "summarise-rows",
            "dedupe-rows",
            "limit-rows",
            "fill-nulls",
            "join-tables",
            "append-rows",
        ];
        let entries = all_operation_entries();
        let verbs: Vec<&str> = entries.iter().map(|(_, e)| e.longname).collect();
        assert_eq!(verbs, long_names);
        for (_, entry) in &entries {
            assert_eq!(entry.tier, CommandTier::View, "{}", entry.longname);
            assert!(is_operation_verb(entry.longname));
            assert_eq!(entry.binding_specs.len(), 1, "{}", entry.longname);
            let scores = entry.scores.as_ref().expect("a bound key has scores");
            for score in [scores.frequency, scores.mnemonic, scores.convention] {
                assert!((1..=5).contains(&score), "{} score {score}", entry.longname);
            }
        }
        assert!(!is_operation_verb("filter-rows-at-cursor"));
    }
}
