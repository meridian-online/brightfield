//! `append-rows` in the window: the rows of tables picked in the spine,
//! appended to the table in front's, their columns matched by name.
//!
//! It starts from `shift-a` with the grid's cursor on a cell, or from "Append
//! rows of other tables" on a column header's menu, and first asks the spine
//! for the tables: `Space` adds each, in the order the rows will follow, and
//! Enter takes them. A table that shares no column's name with the table in
//! front is dimmed in the pick and reads *shares no column*, since arc's
//! `UNION ALL BY NAME` would append its rows with every column of the table
//! in front null.
//!
//! Once the tables are taken, Enter keeps the step ([`Keeps::OnEnter`]). Its form in the
//! Operation section shows how the columns match: the tables numbered in the
//! order their rows come, then a row per column name, in the order the result
//! takes them (the table in front's first, then each name a later table
//! adds), and a column per table under its number, each cell *✓* where the
//! table holds the column and *null* where its rows will lack it. The
//! preview is the appended rows, and the step is named `<table>_appended`.

use brightfield_workbench::chrome;
use meridian_design::{semantic, spacing};

use super::{
    Begin, By, Draft, FormCx, FormEvent, HeaderPart, Keeps, NameRule, Need, OperationUi, Stage,
    Start,
};

/// arc's role for the tables whose rows follow the table in front's.
const OTHER: &str = "other";

/// The cell of a table that holds the column.
pub const HELD: &str = "\u{2713}";

/// The cell of a table whose rows lack the column.
pub const LACKED: &str = "null";

fn begin(b: &Begin<'_>) -> Result<Draft, String> {
    if b.taken.is_empty() {
        return Err("Append rows needs a second table with matching columns.".to_string());
    }
    let mut draft = Draft::empty("append-rows");
    draft.on = b.table.map(str::to_string);
    draft.reads = b
        .taken
        .iter()
        .map(|(table, _)| (OTHER.to_string(), table.clone()))
        .collect();
    Ok(draft)
}

/// **How `UNION ALL BY NAME` matches the columns of `tables`**, the first of
/// them the table in front: each column name of the result, in the order it
/// takes them, with whether each table holds it. A name is matched without
/// regard to case, as DuckDB matches it, and is spelled as the first table
/// to hold it spells it.
#[must_use]
pub fn matching(tables: &[&[String]]) -> Vec<(String, Vec<bool>)> {
    let mut rows: Vec<(String, Vec<bool>)> = Vec::new();
    for (at, columns) in tables.iter().enumerate() {
        for column in *columns {
            let row = match rows
                .iter()
                .position(|(name, _)| name.eq_ignore_ascii_case(column))
            {
                Some(row) => row,
                None => {
                    rows.push((column.clone(), vec![false; tables.len()]));
                    rows.len() - 1
                }
            };
            rows[row].1[at] = true;
        }
    }
    rows
}

/// The form: which table's rows come in which order, and how their columns
/// match.
fn form(draft: &mut Draft, ui: &mut egui::Ui, cx: &FormCx<'_>) -> FormEvent {
    let sem = semantic(cx.mode.is_dark());
    let font = crate::protocol::ui_font();
    let primary = chrome::colour(sem.text.primary);
    let secondary = chrome::colour(sem.text.secondary);
    let muted = chrome::colour(sem.text.muted);
    let text = |words: &str, colour| egui::RichText::new(words).font(font.clone()).color(colour);

    let front = draft.on.clone().unwrap_or_default();
    let others: Vec<&str> = draft
        .reads
        .iter()
        .filter(|(role, _)| role == OTHER)
        .map(|(_, table)| table.as_str())
        .collect();
    // A kept step being corrected is begun from its record, which holds the
    // tables it reads but not their columns.
    let known: Vec<(&str, &[String])> = others
        .iter()
        .filter_map(|table| {
            cx.begin
                .taken
                .iter()
                .find(|(taken, _)| taken == table)
                .map(|(_, columns)| (*table, columns.as_slice()))
        })
        .collect();
    ui.add_space(spacing::SPACE_2);
    ui.label(text(
        &format!(
            "The rows of {front}, then of {}, matched by column name",
            others.join(", then of ")
        ),
        secondary,
    ));
    // The tables are keyed by number, so the grid stays as narrow as the rail
    // however long their names are.
    let mut named: Vec<&str> = vec![front.as_str()];
    named.extend(known.iter().map(|(table, _)| *table));
    egui::Grid::new("append-rows-tables")
        .spacing(egui::vec2(spacing::SPACE_2, spacing::SPACE_1))
        .show(ui, |ui| {
            for (at, table) in named.iter().enumerate() {
                ui.label(text(&(at + 1).to_string(), secondary));
                ui.label(text(table, primary));
                ui.end_row();
            }
        });
    ui.add_space(spacing::SPACE_2);
    let mut tables: Vec<&[String]> = vec![cx.begin.columns];
    tables.extend(known.iter().map(|(_, columns)| *columns));
    let rows = matching(&tables);
    egui::Grid::new("append-rows-matching")
        .spacing(egui::vec2(spacing::SPACE_3, spacing::SPACE_1))
        .show(ui, |ui| {
            ui.label("");
            for at in 1..=named.len() {
                ui.label(text(&at.to_string(), secondary));
            }
            ui.end_row();
            for (name, held) in &rows {
                ui.label(text(name, primary));
                for holds in held {
                    if *holds {
                        ui.label(text(HELD, primary));
                    } else {
                        ui.label(text(LACKED, muted));
                    }
                }
                ui.end_row();
            }
        });
    for table in others
        .iter()
        .filter(|table| !known.iter().any(|(k, _)| k == *table))
    {
        ui.label(text(
            &format!("{table}'s columns are not read here."),
            muted,
        ));
    }
    FormEvent::None
}

static UI: OperationUi = OperationUi {
    long_name: "append-rows",
    verb: "append-rows",
    stage: Stage::Model,
    icon: "row-insert-bottom",
    starts: &[
        Start::Spine { role: OTHER },
        Start::Cell,
        Start::Header(HeaderPart::Menu),
    ],
    menu_words: "Append rows of other tables",
    needs: &[
        Need::ProtocolSource,
        Need::Rows,
        Need::ViewRecorded,
        Need::AnotherTableSharingAColumn,
    ],
    begin,
    keeps: Keeps::OnEnter,
    field: None,
    form: Some(form),
    name: NameRule {
        word: "appended",
        from: By::Nothing,
        with_by: false,
    },
};

/// `append-rows`'s slot.
#[must_use]
pub fn ui() -> Option<&'static OperationUi> {
    Some(&UI)
}

#[cfg(test)]
mod tests {
    use super::matching;

    fn names(of: &[&str]) -> Vec<String> {
        of.iter().map(|n| (*n).to_string()).collect()
    }

    #[test]
    fn the_result_takes_the_front_tables_columns_then_each_name_a_later_one_adds() {
        let front = names(&["median_income", "house_age", "rooms"]);
        let without_age = names(&["median_income", "rooms"]);
        let counted = names(&["House_Age", "n"]);
        let rows = matching(&[&front, &without_age, &counted]);
        assert_eq!(
            rows,
            [
                ("median_income".to_string(), vec![true, true, false]),
                ("house_age".to_string(), vec![true, false, true]),
                ("rooms".to_string(), vec![true, true, false]),
                ("n".to_string(), vec![false, false, true]),
            ]
        );
    }

    #[test]
    fn a_table_with_no_columns_holds_none_of_them() {
        let front = names(&["a"]);
        assert_eq!(
            matching(&[&front, &[]]),
            [("a".to_string(), vec![true, false])]
        );
        assert!(matching(&[]).is_empty());
    }
}
