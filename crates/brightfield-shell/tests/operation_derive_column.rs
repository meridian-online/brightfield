//! `derive-column` in the window: an expression typed in a new header at the
//! table's end, its values filling under it at each keystroke. `=` on a cell,
//! the header's new-column cell, or "Add a column from this one" on a header's
//! menu starts it; Tab moves from the expression to the new column's name, and
//! Enter keeps the step.
//!
//! Each window is over a copy of a fixture in a folder of the test's own, and
//! arc is the `brightfield-shell` binary cargo built for the suite.

#[path = "support/operations.rs"]
mod operations;

use operations::OpWindow;

const SAMPLE: &str = "california_housing_sample.csv";
const WITH_NULLS: &str = "with_nulls.csv";

/// Run `start` until arc's description has come and it begins a draft.
fn until_making(win: &mut OpWindow, mut start: impl FnMut(&mut OpWindow)) {
    for _ in 0..600 {
        start(win);
        if win.making().is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        win.key(egui::Key::Escape);
    }
    panic!("the draft never began: {:?}", win.said());
}

/// `=` with the grid's cursor on `column`, once arc's description has come.
fn derive_with_key(win: &mut OpWindow, column: &str) {
    win.cursor_on(column);
    until_making(win, |win| win.press("="));
}

/// Type `text` into the field in place and wait for the draft's answer.
fn type_in(win: &mut OpWindow, text: &str) {
    win.type_text(text);
    win.settle_reads();
    win.wait_for_preview();
}

fn last(columns: &[String]) -> String {
    columns.last().cloned().unwrap_or_default()
}

#[test]
fn derive_column_starts_from_a_cell_with_its_defaults() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    derive_with_key(&mut win, "median_income");
    let draft = win.making().expect("the draft is being made");
    assert_eq!(draft.long_name, "derive-column");
    assert_eq!(draft.on, Some(win.table()));
    assert_eq!(draft.value("expression"), Some("median_income"));
    // The name follows the expression, free of the table's columns.
    assert_eq!(draft.value("column"), Some("median_income_2"));
    assert_eq!(win.field_text().as_deref(), Some("median_income"));
}

#[test]
fn derive_column_previews_arcs_model_as_it_is_edited() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    derive_with_key(&mut win, "median_income");
    // The default is selected whole, so typing replaces it.
    type_in(&mut win, "median_income * 2");
    assert_eq!(win.field_text().as_deref(), Some("median_income * 2"));
    let columns = win.grid_columns();
    assert_eq!(columns.len(), 10, "{columns:?}");
    assert_eq!(last(&columns), "median_income_2");
    assert_eq!(
        win.grid_cell(0, "median_income_2").as_deref(),
        Some("1.8604")
    );
    let editor = win.editor_text();
    assert!(editor.contains("median_income * 2"), "{editor}");

    // Tab moves to the name, selected whole, and the column is renamed live.
    win.key(egui::Key::Tab);
    assert_eq!(win.field_text().as_deref(), Some("median_income_2"));
    type_in(&mut win, "double_income");
    assert_eq!(last(&win.grid_columns()), "double_income");
    assert_eq!(win.grid_cell(0, "double_income").as_deref(), Some("1.8604"));
}

#[test]
fn derive_column_keeps_a_step_named_by_its_effect_and_save_records_arcs_model() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    let before = win.snapshot();
    derive_with_key(&mut win, "median_income");
    type_in(&mut win, "median_income * 2");
    win.key(egui::Key::Tab);
    type_in(&mut win, "double_income");
    win.key(egui::Key::Enter);
    win.settle_reads();
    let kept = win.kept();
    let name = format!("{}_with_double_income", win.table());
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, name);
    assert!(
        kept[0].statement.contains("median_income * 2"),
        "{}",
        kept[0].statement
    );
    // Kept, and no file written.
    assert_eq!(win.snapshot(), before);

    let statement = kept[0].statement.clone();
    win.save()
        .unwrap_or_else(|e| panic!("the Save records: {e}"));
    assert!(win.manifest_steps().contains(&name));
    assert_eq!(win.model_of(&name), statement);
}

#[test]
fn derive_column_reopens_as_a_step_not_run_that_previews_the_same() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    derive_with_key(&mut win, "median_income");
    type_in(&mut win, "median_income * 2");
    win.key(egui::Key::Enter);
    win.settle_reads();
    let name = format!("{}_with_median_income_2", win.table());
    assert_eq!(win.kept().len(), 1, "{:?}", win.said());
    win.wait_for_preview();
    let columns = win.grid_columns();
    assert_eq!(last(&columns), "median_income_2");
    win.save()
        .unwrap_or_else(|e| panic!("the Save records: {e}"));
    let mut again = win.reopen();
    assert!(!again.step_ran(&name));
    again.show_grid_of(&name);
    assert_eq!(again.grid_columns(), columns);
}

#[test]
fn derive_column_says_the_refusal_and_keeps_nothing() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    let before = win.snapshot();
    derive_with_key(&mut win, "median_income");
    type_in(&mut win, "median_income * 2");
    assert!(!win.grid_faded());
    win.key(egui::Key::Backspace);
    win.key(egui::Key::Backspace);
    win.key(egui::Key::Backspace);
    type_in(&mut win, " + sum(");
    assert_eq!(win.field_text().as_deref(), Some("median_income  + sum("));
    // The reason is the status band's, drawn while the draft stands refused.
    let refusal = win
        .app
        .draft_reads()
        .and_then(|reads| reads.refusal())
        .unwrap_or_default()
        .to_string();
    assert!(refusal.contains("is not one SQL expression"), "{refusal}");
    assert!(
        win.texts.iter().any(|text| text.contains(&refusal)),
        "the status band drew no {refusal:?}: {:?}",
        win.texts
    );
    assert!(win.grid_faded());
    win.key(egui::Key::Enter);
    win.settle();
    assert!(win.kept().is_empty(), "{:?}", win.kept());
    assert_eq!(win.snapshot(), before);
}

#[test]
fn derive_column_from_the_new_column_cell_starts_empty_and_completes_a_column() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    win.settle();
    until_making(&mut win, OpWindow::header_new_column);
    let draft = win.making().expect("the draft is being made");
    assert_eq!(draft.value("expression"), Some(""));
    assert_eq!(win.field_text().as_deref(), Some(""));
    // A column is completed as SQL spells it, quoted where it needs quotes.
    win.type_text("house_age + med");
    assert_eq!(win.suggestions(), ["house_age + \"median income\""]);
    win.key(egui::Key::ArrowDown);
    win.key(egui::Key::Tab);
    assert_eq!(
        win.field_text().as_deref(),
        Some("house_age + \"median income\"")
    );
    win.settle_reads();
    let draft = win.making().expect("the draft is being made");
    assert_eq!(draft.value("column"), Some("house_age_median_income"));
    // The name has no suggestions.
    win.key(egui::Key::Tab);
    assert!(win.suggestions().is_empty(), "{:?}", win.suggestions());
}

#[test]
fn derive_column_from_the_header_menu_keeps_the_same_step() {
    let mut by_key = OpWindow::on_copy_of(SAMPLE);
    derive_with_key(&mut by_key, "house_age");
    by_key.settle_reads();
    by_key.key(egui::Key::Enter);
    by_key.settle_reads();
    let from_key = by_key.kept();
    assert_eq!(from_key.len(), 1, "{:?}", by_key.said());

    let mut win = OpWindow::on_copy_of(SAMPLE);
    win.cursor_on("house_age");
    until_making(&mut win, |win| {
        win.header_menu("house_age", "Add a column from this one");
    });
    win.settle_reads();
    win.key(egui::Key::Enter);
    win.settle_reads();
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, from_key[0].name);
    assert_eq!(kept[0].statement, from_key[0].statement);
    assert_eq!(kept[0].operation, from_key[0].operation);
}

#[test]
fn derive_column_quotes_a_column_whose_name_needs_quotes() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    derive_with_key(&mut win, "median income");
    assert_eq!(win.field_text().as_deref(), Some("\"median income\""));
    let draft = win.making().expect("the draft is being made");
    assert_eq!(draft.value("column"), Some("median_income"));
    win.settle_reads();
    win.key(egui::Key::Enter);
    win.settle_reads();
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, "with_nulls_with_median_income");
    assert!(
        kept[0].statement.contains("\"median income\""),
        "{}",
        kept[0].statement
    );
}

#[test]
fn a_name_typed_into_stops_following_the_expression() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    derive_with_key(&mut win, "median_income");
    win.key(egui::Key::Tab);
    win.type_text("income");
    // Back to the expression, which no longer renames the column.
    win.key(egui::Key::Tab);
    win.type_text("house_age * 2");
    let draft = win.making().expect("the draft is being made");
    assert_eq!(draft.value("expression"), Some("house_age * 2"));
    assert_eq!(draft.value("column"), Some("income"));
}
