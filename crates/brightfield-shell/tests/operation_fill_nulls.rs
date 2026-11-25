//! `fill-nulls` in the window: the value a column's missing values are filled
//! with, typed in place over its header's validity band. A click on the missing
//! count, `f` on a cell, or "Fill missing values" on a header's menu opens the
//! field on a fill that suits the column's storage type, each edit previews
//! arc's model, and Enter keeps the step.
//!
//! Each window is over a copy of a fixture in a folder of the test's own, and
//! arc is the `brightfield-shell` binary cargo built for the suite.

#[path = "support/operations.rs"]
mod operations;

use brightfield_shell::operation_ui::header::Gesture;
use brightfield_shell::operation_ui::HeaderPart;
use operations::OpWindow;

const WITH_NULLS: &str = "with_nulls.csv";

/// `with_nulls` with a missing value in each of `city` (text), `house_age`
/// (a whole number) and `median income` (a decimal, whose name needs quotes).
const MORE_NULLS: &str = "city,house_age,order,median income\n\
                          Alameda,28,1,1.5\n\
                          Berkeley,,2,\n\
                          ,40,3,3.125\n";

/// A window over `with_nulls` rewritten as [`MORE_NULLS`].
fn more_nulls() -> OpWindow {
    OpWindow::on_copy_prepared(WITH_NULLS, |dir| {
        std::fs::write(dir.join(WITH_NULLS), MORE_NULLS).expect("the fixture writes");
    })
}

/// Whether the status band says arc's description is still on its way, which
/// a start waits out.
fn reading_description(win: &OpWindow) -> bool {
    win.app
        .operation_said()
        .is_some_and(|said| said.starts_with("Reading arc's description"))
}

/// Open the fill's field with a click on `column`'s missing count, once arc's
/// description has come.
fn fill_from_header(win: &mut OpWindow, column: &str) {
    for _ in 0..600 {
        win.header(column, HeaderPart::Missing, Gesture::Click);
        if win.making().is_some() {
            return;
        }
        assert!(reading_description(win), "{:?}", win.said());
        std::thread::sleep(std::time::Duration::from_millis(20));
        win.run(Vec::new());
    }
    panic!("arc's description never came");
}

/// Open the fill's field with `f` on `column`'s cell, once arc's description
/// has come.
fn fill_with_key(win: &mut OpWindow, column: &str) {
    win.cursor_on(column);
    for _ in 0..600 {
        win.press("f");
        if win.making().is_some() {
            return;
        }
        assert!(reading_description(win), "{:?}", win.said());
        std::thread::sleep(std::time::Duration::from_millis(20));
        win.run(Vec::new());
    }
    panic!("arc's description never came");
}

/// Wait until arc's description has come, by asking for it with `f` on
/// `column` until the band says something other than that it is on its way.
fn description_read(win: &mut OpWindow, column: &str) {
    win.cursor_on(column);
    for _ in 0..600 {
        win.press("f");
        if win.making().is_some() || !reading_description(win) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        win.run(Vec::new());
    }
    panic!("arc's description never came");
}

/// Clear the field's text, one Backspace a character.
fn clear(win: &mut OpWindow) {
    let typed = win.field_text().unwrap_or_default();
    for _ in 0..typed.chars().count().max(1) {
        win.key(egui::Key::Backspace);
    }
}

#[test]
fn fill_nulls_starts_from_the_missing_count_with_its_defaults() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    fill_from_header(&mut win, "house_age");
    let draft = win.making().expect("the fill is being made");
    assert_eq!(draft.long_name, "fill-nulls");
    assert_eq!(draft.on, Some(win.table()));
    // The column is a name to arc, so it is given bare; a whole number opens on 0.
    assert_eq!(draft.value("column"), Some("house_age"));
    assert_eq!(draft.value("value"), Some("0"));
    assert_eq!(win.field_text().as_deref(), Some("0"));

    win.settle_reads();
    assert_eq!(win.preview_missing("house_age"), Some(0));
    assert_eq!(win.grid_cell(1, "city").as_deref(), Some("Berkeley"));
    assert_eq!(win.grid_cell(1, "house_age").as_deref(), Some("0"));
    win.key(egui::Key::Enter);
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, "with_nulls_filled_house_age");
}

#[test]
fn fill_nulls_starts_from_a_cell_and_the_menu_with_the_same_defaults() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    fill_with_key(&mut win, "house_age");
    assert_eq!(win.field_text().as_deref(), Some("0"));
    let from_key = win.making().expect("the fill is being made");

    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    win.cursor_on("house_age");
    for _ in 0..600 {
        win.header_menu("house_age", "Fill missing values");
        if win.making().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        win.key(egui::Key::Escape);
    }
    assert_eq!(win.making(), Some(from_key));
    assert_eq!(win.field_text().as_deref(), Some("0"));
}

#[test]
fn fill_nulls_offers_a_numbers_median() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    fill_from_header(&mut win, "house_age");
    // The default is the value as it stands, so the median alone is offered.
    assert_eq!(win.suggestions(), ["34"]);
    win.key(egui::Key::ArrowDown);
    win.key(egui::Key::Tab);
    assert_eq!(win.field_text().as_deref(), Some("34"));
    assert_eq!(win.suggestions(), ["0"]);
    win.settle_reads();
    assert_eq!(win.grid_cell(1, "house_age").as_deref(), Some("34"));
    assert_eq!(win.preview_missing("house_age"), Some(0));
}

#[test]
fn fill_nulls_previews_arcs_model_as_it_is_edited() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    fill_from_header(&mut win, "house_age");
    // The first text typed replaces the value selected whole.
    win.type_keys("7");
    assert_eq!(win.field_text().as_deref(), Some("7"));
    win.settle_reads();
    assert_eq!(
        win.grid_columns(),
        ["city", "house_age", "order", "median income"]
    );
    assert_eq!(win.grid_cell(1, "house_age").as_deref(), Some("7"));
    // A value already there is kept.
    assert_eq!(win.grid_cell(0, "house_age").as_deref(), Some("28"));
    let editor = win.editor_text();
    assert!(
        editor.contains("coalesce(house_age, 7) AS house_age"),
        "{editor}"
    );

    // Another edit, and the cells follow it.
    win.type_keys("0");
    win.settle_reads();
    assert_eq!(win.grid_cell(1, "house_age").as_deref(), Some("70"));
}

#[test]
fn fill_nulls_keeps_a_step_named_by_its_effect_and_save_records_arcs_model() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    let before = win.snapshot();
    fill_from_header(&mut win, "house_age");
    win.settle_reads();
    win.key(egui::Key::Enter);
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, "with_nulls_filled_house_age");
    assert!(
        kept[0]
            .statement
            .contains("coalesce(house_age, 0) AS house_age"),
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
        .contains(&"with_nulls_filled_house_age".to_string()));
    assert_eq!(win.model_of("with_nulls_filled_house_age"), statement);
}

#[test]
fn fill_nulls_reopens_as_a_step_not_run_that_previews_the_same() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    fill_from_header(&mut win, "house_age");
    win.settle_reads();
    win.key(egui::Key::Enter);
    let name = "with_nulls_filled_house_age";
    assert_eq!(win.kept().len(), 1, "{:?}", win.said());
    win.wait_for_preview();
    let columns = win.grid_columns();
    assert_eq!(columns, ["city", "house_age", "order", "median income"]);
    win.save()
        .unwrap_or_else(|e| panic!("the Save records: {e}"));
    let mut again = win.reopen();
    assert!(!again.step_ran(name));
    again.show_grid_of(name);
    assert_eq!(again.grid_columns(), columns);
}

#[test]
fn fill_nulls_quotes_a_column_whose_name_needs_quotes() {
    let mut win = more_nulls();
    fill_from_header(&mut win, "median income");
    let draft = win.making().expect("the fill is being made");
    // Bare to arc, which quotes it in the SQL it writes.
    assert_eq!(draft.value("column"), Some("median income"));
    assert_eq!(win.field_text().as_deref(), Some("0"));
    assert_eq!(win.suggestions(), ["2.3125"]);
    win.settle_reads();
    assert_eq!(win.preview_missing("median income"), Some(0));
    win.key(egui::Key::Enter);
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, "with_nulls_filled_median_income");
    assert!(
        kept[0]
            .statement
            .contains("coalesce(\"median income\", 0) AS \"median income\""),
        "{}",
        kept[0].statement
    );
}

#[test]
fn fill_nulls_opens_a_text_column_on_unknown() {
    let mut win = more_nulls();
    fill_with_key(&mut win, "city");
    assert_eq!(win.field_text().as_deref(), Some("'unknown'"));
    // Text has no median, and the default is the value as it stands.
    assert!(win.suggestions().is_empty(), "{:?}", win.suggestions());
    win.settle_reads();
    assert_eq!(win.preview_missing("city"), Some(0));
    assert_eq!(win.grid_cell(2, "city").as_deref(), Some("unknown"));
    // The other columns' missing values are left as they are.
    assert_eq!(win.preview_missing("house_age"), Some(1));
}

#[test]
fn fill_nulls_is_greyed_on_a_column_with_no_missing_values() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    description_read(&mut win, "city");
    assert!(win.making().is_none(), "{:?}", win.making());
    assert_eq!(
        win.said().as_deref(),
        Some("Fill nulls needs a column with missing values.")
    );
}

#[test]
fn fill_nulls_says_the_refusal_and_keeps_nothing() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    let before = win.snapshot();
    fill_from_header(&mut win, "house_age");
    // The default first, so there are rows to fade.
    win.settle_reads();
    assert_eq!(win.grid_cell(1, "house_age").as_deref(), Some("0"));

    // A string DuckDB cannot read as the column's number: its reason, the rows faded.
    win.type_text("'x'");
    assert_eq!(win.field_text().as_deref(), Some("'x'"));
    win.settle_reads();
    let said = win.said().unwrap_or_default();
    assert!(said.contains("Could not convert string 'x'"), "{said}");
    assert!(win.grid_faded(), "the last good rows stay, faded");
    assert_eq!(win.grid_cell(1, "house_age").as_deref(), Some("0"));
    win.key(egui::Key::Enter);
    assert!(win.kept().is_empty(), "{:?}", win.kept());
    assert!(win.making().is_some(), "the field stays open on a refusal");

    // A `;` that would end the step early: arc's reason.
    clear(&mut win);
    win.type_text("1;");
    win.settle_reads();
    let said = win.said().unwrap_or_default();
    assert!(said.contains("holds a `;`"), "{said}");
    assert!(win.grid_faded());
    win.key(egui::Key::Enter);
    assert!(win.kept().is_empty(), "{:?}", win.kept());
    assert_eq!(win.snapshot(), before);
}

#[test]
fn fill_nulls_over_a_kept_fills_grid_says_it_needs_missing_values() {
    // Which columns have missing values is known for the loaded table alone, so
    // a kept fill is not corrected from its own grid.
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    fill_from_header(&mut win, "house_age");
    win.settle_reads();
    win.key(egui::Key::Enter);
    assert_eq!(win.kept().len(), 1, "{:?}", win.said());
    win.wait_for_preview();
    win.focus_grid();
    win.press("f");
    assert!(win.making().is_none(), "{:?}", win.making());
    assert_eq!(
        win.said().as_deref(),
        Some("Fill nulls needs a column with missing values.")
    );
    assert_eq!(win.kept().len(), 1);
}
