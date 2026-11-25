//! `dedupe-rows` in the window: one action with a sensible default, refined
//! in a form.
//!
//! It starts from `shift-u` with the grid's cursor on a cell, or from "Remove
//! duplicate rows" on a column header's menu, and asks nothing at first: its
//! draft gives no value, so arc's model is `SELECT DISTINCT *`, and the step is
//! kept as soon as arc admits it and DuckDB has run it ([`Keeps::AtOnce`]). A
//! table with no duplicate rows keeps no step, and the status band says so.
//!
//! Once kept, the step is opened again as a correction, with its form in the
//! inspector's Operation section. "One row per" takes the key, a list of the
//! table's columns, and "The first by" takes the order, which arc needs beside
//! the key: it is the key's first column, descending, until it is typed over.
//! Each change previews arc's model, and Enter replaces the kept step in place
//! under the name it was kept as, since a name is fixed when a draft is kept.

use brightfield_workbench::chrome;
use meridian_design::{semantic, spacing};
use meridian_egui::{list_row, text_field, ListRow, RowHeight, TextField};

use super::{
    Begin, By, Draft, FormCx, FormEvent, HeaderPart, Keeps, NameRule, Need, OperationUi, Stage,
    Start,
};
use crate::protocol::ui_font;

fn begin(b: &Begin<'_>) -> Result<Draft, String> {
    let mut draft = Draft::empty("dedupe-rows");
    draft.on = b.table.map(str::to_string);
    Ok(draft)
}

/// The key's items, trimmed, without an empty one.
fn key_items(key: &str) -> Vec<&str> {
    key.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect()
}

/// The order a key is given until the order is typed over: its first item,
/// descending. Empty for an empty key.
fn order_for(key: &str) -> String {
    key_items(key)
        .first()
        .map(|first| format!("{first} desc"))
        .unwrap_or_default()
}

/// The key once `column`, as SQL spells it, is put in or taken out: the
/// table's columns in the key, in the table's order, after each item of the
/// key that is not one of the table's columns, such as an expression a
/// correction began from.
fn toggled(key: &str, column: &str, columns: &[String], quote: &dyn Fn(&str) -> String) -> String {
    let items = key_items(key);
    let spelled: Vec<String> = columns.iter().map(|c| quote(c)).collect();
    let flipped = quote(column);
    let mut out: Vec<String> = items
        .iter()
        .filter(|item| !spelled.iter().any(|s| s == *item))
        .map(|item| (*item).to_string())
        .collect();
    for name in &spelled {
        let held = items.contains(&name.as_str());
        if held != (*name == flipped) {
            out.push(name.clone());
        }
    }
    out.join(", ")
}

/// Put `column` in the draft's key or take it out, and keep the order in step
/// with the key while it is still the order the key before it gave.
fn toggle(draft: &mut Draft, column: &str, b: &Begin<'_>) {
    let before = draft.value("key").unwrap_or_default().to_string();
    let key = toggled(&before, column, b.columns, b.quote);
    let order = draft.value("order_by").unwrap_or_default();
    if order.trim() == order_for(&before) {
        draft.set("order_by", order_for(&key));
    }
    draft.set("key", key);
}

/// The form: a row for each of the table's columns, the key's drawn selected,
/// and the order as text.
fn form(draft: &mut Draft, ui: &mut egui::Ui, cx: &FormCx<'_>) -> FormEvent {
    let sem = semantic(cx.mode.is_dark());
    let heading = |ui: &mut egui::Ui, words: &str| {
        ui.add_space(spacing::SPACE_2);
        ui.label(
            egui::RichText::new(words)
                .font(ui_font())
                .color(chrome::colour(sem.text.secondary)),
        );
    };
    let mut event = FormEvent::None;
    heading(ui, "One row per");
    let key = draft.value("key").unwrap_or_default().to_string();
    let held = key_items(&key);
    let mut flip = None;
    for column in cx.begin.columns {
        let on = held.contains(&(cx.begin.quote)(column).as_str());
        let row = list_row(ui, ListRow::new(RowHeight::Dense).selected(on), |ui, _| {
            ui.label(
                egui::RichText::new(column)
                    .font(ui_font())
                    .color(chrome::colour(sem.text.primary)),
            );
        });
        if row.clicked() {
            flip = Some(column.clone());
        }
    }
    if let Some(column) = flip {
        toggle(draft, &column, cx.begin);
        event = FormEvent::Changed;
    }
    heading(ui, "The first by");
    let mut order = draft.value("order_by").unwrap_or_default().to_string();
    let field = text_field(
        ui,
        TextField::new(&mut order)
            .placeholder("which row of each key is kept")
            .dense(),
    );
    if field.changed {
        draft.set("order_by", order);
        event = FormEvent::Changed;
    }
    if field.submitted {
        event = FormEvent::Keep;
    }
    event
}

static UI: OperationUi = OperationUi {
    long_name: "dedupe-rows",
    verb: "dedupe-rows",
    stage: Stage::Shape,
    icon: "copy-off",
    starts: &[Start::Cell, Start::Header(HeaderPart::Menu)],
    menu_words: "Remove duplicate rows",
    needs: &[Need::ProtocolSource, Need::Rows, Need::ViewRecorded],
    begin,
    keeps: Keeps::AtOnce,
    field: None,
    form: Some(form),
    name: NameRule {
        word: "deduped",
        from: By::Value("key"),
        with_by: true,
    },
};

/// `dedupe-rows`'s slot.
#[must_use]
pub fn ui() -> Option<&'static OperationUi> {
    Some(&UI)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote(name: &str) -> String {
        if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            name.to_string()
        } else {
            format!("\"{name}\"")
        }
    }

    fn columns(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    #[test]
    fn a_key_takes_columns_in_the_tables_order_and_gives_them_back() {
        let columns = columns(&["city", "house_age", "median income"]);
        let key = toggled("", "house_age", &columns, &quote);
        assert_eq!(key, "house_age");
        let key = toggled(&key, "city", &columns, &quote);
        assert_eq!(key, "city, house_age");
        let key = toggled(&key, "median income", &columns, &quote);
        assert_eq!(key, "city, house_age, \"median income\"");
        let key = toggled(&key, "city", &columns, &quote);
        assert_eq!(key, "house_age, \"median income\"");
        // An item that is no column of the table stays, ahead of the columns.
        assert_eq!(
            toggled("lower(city)", "city", &columns, &quote),
            "lower(city), city"
        );
    }

    #[test]
    fn the_order_follows_the_key_until_it_is_typed_over() {
        assert_eq!(order_for(""), "");
        assert_eq!(order_for("city, house_age"), "city desc");
        let columns = columns(&["city", "house_age"]);
        let description = crate::operation::OperationDescription {
            long_name: "dedupe-rows".to_string(),
            summary: String::new(),
            reads: Vec::new(),
            parameters: Vec::new(),
            needed_together: Vec::new(),
        };
        let transforms = brightfield_protocol::silver::FinetypeTransforms::vendored();
        let b = Begin {
            start: Start::Cell,
            table: Some("t"),
            column: None,
            columns: &columns,
            facts: &[],
            description: &description,
            taken: &[],
            model: None,
            transforms: &transforms,
            quote: &quote,
        };
        let mut draft = Draft::empty("dedupe-rows");
        toggle(&mut draft, "house_age", &b);
        assert_eq!(draft.value("order_by"), Some("house_age desc"));
        toggle(&mut draft, "city", &b);
        assert_eq!(draft.value("key"), Some("city, house_age"));
        assert_eq!(draft.value("order_by"), Some("city desc"));
        // Typed over, the order stays as typed.
        draft.set("order_by", "house_age".to_string());
        toggle(&mut draft, "house_age", &b);
        assert_eq!(draft.value("key"), Some("city"));
        assert_eq!(draft.value("order_by"), Some("house_age"));
    }
}
