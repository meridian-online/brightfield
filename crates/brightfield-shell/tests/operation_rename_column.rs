//! `rename-column` in the window: the new name typed in place over the column's
//! name in its header. A double-click on the name, `shift-r` on a cell, or
//! "Rename column" on a header's menu opens the field with the column's name
//! selected whole, each edit previews arc's model, and Enter keeps the step.
//!
//! Each window is over a copy of a fixture in a folder of the test's own, and
//! arc is the `brightfield-shell` binary cargo built for the suite.

#[path = "support/operations.rs"]
mod operations;

use brightfield_shell::operation_ui::header::Gesture;
use brightfield_shell::operation_ui::HeaderPart;
use operations::OpWindow;

const WITH_NULLS: &str = "with_nulls.csv";
const SAMPLE: &str = "california_housing_sample.csv";

/// Whether the status band says arc's description is still on its way, which
/// a start waits out.
fn reading_description(win: &OpWindow) -> bool {
    win.app
        .operation_said()
        .is_some_and(|said| said.starts_with("Reading arc's description"))
}

/// Open the rename's field with a double-click on `column`'s name, once arc's
/// description has come.
fn rename_from_header(win: &mut OpWindow, column: &str) {
    for _ in 0..600 {
        win.header(column, HeaderPart::Name, Gesture::DoubleClick);
        if win.making().is_some() {
            return;
        }
        assert!(reading_description(win), "{:?}", win.said());
        std::thread::sleep(std::time::Duration::from_millis(20));
        win.run(Vec::new());
    }
    panic!("arc's description never came");
}

/// Open the rename's field with `shift-r` on `column`'s cell, once arc's
/// description has come.
fn rename_with_key(win: &mut OpWindow, column: &str) {
    win.cursor_on(column);
    for _ in 0..600 {
        win.press("shift-r");
        if win.making().is_some() {
            return;
        }
        assert!(reading_description(win), "{:?}", win.said());
        std::thread::sleep(std::time::Duration::from_millis(20));
        win.run(Vec::new());
    }
    panic!("arc's description never came");
}

/// Whether the field in place draws its text selected whole.
fn selected_whole(win: &OpWindow) -> bool {
    win.app
        .chart_doc()
        .header_drawn()
        .field
        .as_ref()
        .is_some_and(|field| field.whole)
}

#[test]
fn rename_column_starts_from_a_header_with_its_defaults() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    rename_from_header(&mut win, "median income");
    let draft = win.making().expect("the rename is being made");
    assert_eq!(draft.long_name, "rename-column");
    assert_eq!(draft.on, Some(win.table()));
    // Both are names to arc, so both are given bare.
    assert_eq!(draft.value("column"), Some("median income"));
    assert_eq!(draft.value("to"), Some("median income"));
    assert_eq!(win.field_text().as_deref(), Some("median income"));
    assert!(selected_whole(&win), "the name opens selected whole");
}

#[test]
fn rename_column_starts_from_a_cell_and_the_menu_with_the_same_defaults() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    rename_with_key(&mut win, "median income");
    assert_eq!(win.field_text().as_deref(), Some("median income"));
    let from_key = win.making().expect("the rename is being made");

    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    win.cursor_on("median income");
    for _ in 0..600 {
        win.header_menu("median income", "Rename column");
        if win.making().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        win.key(egui::Key::Escape);
    }
    assert_eq!(win.making(), Some(from_key));
    assert_eq!(win.field_text().as_deref(), Some("median income"));
}

#[test]
fn rename_column_previews_arcs_model_as_it_is_edited() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    rename_from_header(&mut win, "median income");
    // The first letters replace the name selected whole.
    win.type_keys("income");
    assert_eq!(win.field_text().as_deref(), Some("income"));
    assert!(!selected_whole(&win));
    win.settle_reads();
    assert_eq!(win.grid_columns(), ["city", "house_age", "order", "income"]);
    // The field stays over the column it opened on, under its new name.
    assert_eq!(win.field_text().as_deref(), Some("income"));
    let editor = win.editor_text();
    assert!(
        editor.contains("RENAME COLUMN \"median income\" TO income"),
        "{editor}"
    );

    // Another edit, and the header follows it.
    win.type_keys("_usd");
    win.settle_reads();
    assert_eq!(
        win.grid_columns(),
        ["city", "house_age", "order", "income_usd"]
    );
}

#[test]
fn rename_column_keeps_a_step_named_by_its_effect_and_save_records_arcs_model() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    let before = win.snapshot();
    rename_from_header(&mut win, "median income");
    win.type_keys("income");
    win.settle_reads();
    win.key(egui::Key::Enter);
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, "with_nulls_renamed_median_income");
    assert!(
        kept[0]
            .statement
            .contains("RENAME COLUMN \"median income\" TO income"),
        "{}",
        kept[0].statement
    );
    assert!(win.making().is_none());
    assert_eq!(
        win.field_text(),
        None,
        "the field closes with the operation"
    );
    // Kept, and no file written.
    assert_eq!(win.snapshot(), before);

    let statement = kept[0].statement.clone();
    win.save()
        .unwrap_or_else(|e| panic!("the Save records: {e}"));
    assert!(win
        .manifest_steps()
        .contains(&"with_nulls_renamed_median_income".to_string()));
    assert_eq!(win.model_of("with_nulls_renamed_median_income"), statement);
}

#[test]
fn rename_column_reopens_as_a_step_not_run_that_previews_the_same() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    rename_from_header(&mut win, "median income");
    win.type_keys("income");
    win.settle_reads();
    win.key(egui::Key::Enter);
    let name = "with_nulls_renamed_median_income";
    assert_eq!(win.kept().len(), 1, "{:?}", win.said());
    win.wait_for_preview();
    let columns = win.grid_columns();
    assert_eq!(columns, ["city", "house_age", "order", "income"]);
    win.save()
        .unwrap_or_else(|e| panic!("the Save records: {e}"));
    let mut again = win.reopen();
    assert!(!again.step_ran(name));
    again.show_grid_of(name);
    assert_eq!(again.grid_columns(), columns);
}

#[test]
fn rename_column_gives_a_plain_column_bare_on_the_housing_sample() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    rename_with_key(&mut win, "house_age");
    win.type_keys("age");
    win.settle_reads();
    win.key(egui::Key::Enter);
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, format!("{}_renamed_house_age", win.table()));
    assert!(
        kept[0].statement.contains("RENAME COLUMN house_age TO age"),
        "{}",
        kept[0].statement
    );
}

#[test]
fn rename_column_says_the_refusal_and_keeps_nothing() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    let before = win.snapshot();
    rename_from_header(&mut win, "median income");
    // A good name first, so there are rows to fade.
    win.type_keys("income");
    win.settle_reads();
    assert_eq!(win.grid_columns(), ["city", "house_age", "order", "income"]);

    // A name the table already has: DuckDB's reason, the rows faded.
    for _ in 0.."income".len() {
        win.key(egui::Key::Backspace);
    }
    win.type_keys("city");
    win.settle_reads();
    let said = win.said().unwrap_or_default();
    assert!(
        said.contains("Column with name \"city\" already exists"),
        "{said}"
    );
    assert!(win.grid_faded(), "the last good rows stay, faded");
    assert_eq!(win.grid_columns(), ["city", "house_age", "order", "income"]);
    win.key(egui::Key::Enter);
    assert!(win.kept().is_empty(), "{:?}", win.kept());
    assert!(win.making().is_some(), "the field stays open on a refusal");

    // A blank name: the draft asks arc for no blank value, so Enter says the
    // name is still needed.
    for _ in 0.."city".len() {
        win.key(egui::Key::Backspace);
    }
    win.type_text(" ");
    win.settle_reads();
    win.key(egui::Key::Enter);
    let said = win.said().unwrap_or_default();
    assert_eq!(said, "rename-column needs to");
    assert!(win.kept().is_empty(), "{:?}", win.kept());
    assert_eq!(win.snapshot(), before);
}
