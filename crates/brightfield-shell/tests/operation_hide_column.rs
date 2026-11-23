//! `hide-column` in the window: a direct action on a column. `-` on a cell, or
//! "Hide column" on a header's menu, asks nothing and keeps the step as soon as
//! arc admits it and DuckDB has run it.
//!
//! Each window is over a copy of a fixture in a folder of the test's own, and
//! arc is the `brightfield-shell` binary cargo built for the suite.

#[path = "support/operations.rs"]
mod operations;

use operations::OpWindow;

const SAMPLE: &str = "california_housing_sample.csv";
const WITH_NULLS: &str = "with_nulls.csv";

/// Wait until arc's description has come and the key starts a draft, then
/// until the draft is answered and kept.
fn hide_with_key(win: &mut OpWindow, column: &str) {
    win.cursor_on(column);
    for _ in 0..600 {
        win.press("-");
        let pending = win
            .app
            .operation_said()
            .is_some_and(|said| said.starts_with("Reading arc's description"));
        if !pending {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        win.run(Vec::new());
    }
    win.settle_reads();
}

#[test]
fn hide_column_starts_from_a_cell_with_its_defaults() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    win.cursor_on("house_age");
    win.press("-");
    // Either the draft is being made, with the bare column, or it has already been kept.
    match win.making() {
        Some(draft) => {
            assert_eq!(draft.long_name, "hide-column");
            assert_eq!(draft.value("column"), Some("house_age"));
            assert_eq!(draft.on, Some(win.table()));
        }
        None => assert_eq!(win.kept().len(), 1, "{:?}", win.said()),
    }
}

#[test]
fn hide_column_previews_arcs_model_as_it_is_edited() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    hide_with_key(&mut win, "house_age");
    win.wait_for_preview();
    let columns = win.grid_columns();
    assert_eq!(columns.len(), 8, "{columns:?}");
    assert!(!columns.contains(&"house_age".to_string()), "{columns:?}");
    let editor = win.editor_text();
    assert!(editor.contains("EXCLUDE (house_age)"), "{editor}");
}

#[test]
fn hide_column_keeps_a_step_named_by_its_effect_and_save_records_arcs_model() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    let before = win.snapshot();
    hide_with_key(&mut win, "house_age");
    let kept = win.kept();
    let name = format!("{}_without_house_age", win.table());
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, name);
    assert!(kept[0].statement.contains("EXCLUDE (house_age)"));
    // The status band says what was done and how to take it back.
    let said = win.said().unwrap_or_default();
    assert!(said.starts_with("Hid house_age"), "{said}");
    assert!(said.contains("takes it back"), "{said}");
    // Kept, and no file written.
    assert_eq!(win.snapshot(), before);

    let statement = kept[0].statement.clone();
    win.save()
        .unwrap_or_else(|e| panic!("the Save records: {e}"));
    assert!(win.manifest_steps().contains(&name));
    assert_eq!(win.model_of(&name), statement);
}

#[test]
fn hide_column_reopens_as_a_step_not_run_that_previews_the_same() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    hide_with_key(&mut win, "house_age");
    let name = format!("{}_without_house_age", win.table());
    win.wait_for_preview();
    let columns = win.grid_columns();
    win.save()
        .unwrap_or_else(|e| panic!("the Save records: {e}"));
    let mut again = win.reopen();
    assert!(!again.step_ran(&name));
    again.show_grid_of(&name);
    assert_eq!(again.grid_columns(), columns);
}

#[test]
fn hide_column_from_the_header_menu_keeps_the_same_step() {
    let mut by_key = OpWindow::on_copy_of(SAMPLE);
    hide_with_key(&mut by_key, "house_age");
    let from_key = by_key.kept();

    let mut win = OpWindow::on_copy_of(SAMPLE);
    win.cursor_on("house_age");
    // Wait for arc's description, which the header's menu reads.
    for _ in 0..600 {
        win.header_menu("house_age", "Hide column");
        if win.making().is_some() || !win.kept().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        win.key(egui::Key::Escape);
    }
    win.settle_reads();
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, from_key[0].name);
    assert_eq!(kept[0].statement, from_key[0].statement);
    assert_eq!(kept[0].operation, from_key[0].operation);
}

#[test]
fn hide_column_over_a_hidden_columns_step_hides_another_from_it() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    hide_with_key(&mut win, "house_age");
    let first = format!("{}_without_house_age", win.table());
    win.wait_for_preview();
    // The first step's grid is in front with the cursor on it: `-` on `population` there.
    let at = win
        .grid_columns()
        .iter()
        .position(|c| c == "population")
        .expect("the first step keeps population");
    win.app.chart_doc_mut().grid_cursor =
        Some(brightfield_shell::data_grid::GridCursor { row: 0, col: at });
    win.settle();
    win.press("-");
    win.settle_reads();
    let kept = win.kept();
    assert_eq!(kept.len(), 2, "{:?} {:?}", win.said(), kept);
    assert_eq!(kept[0].name, first);
    assert_eq!(kept[1].name, format!("{first}_without_population"));
    assert_eq!(
        kept[1].operation.as_ref().and_then(|r| r.on.clone()),
        Some(first.clone())
    );
    let said = win.said().unwrap_or_default();
    assert!(said.starts_with("Hid population"), "{said}");
    win.wait_for_preview();
    let columns = win.grid_columns();
    assert_eq!(columns.len(), 7, "{columns:?}");
    assert!(!columns.contains(&"house_age".to_string()), "{columns:?}");
    assert!(!columns.contains(&"population".to_string()), "{columns:?}");
}

#[test]
fn hide_column_quotes_a_column_named_order() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    hide_with_key(&mut win, "order");
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, "with_nulls_without_order");
    assert!(
        kept[0].statement.contains("EXCLUDE (\"order\")"),
        "{}",
        kept[0].statement
    );
}

#[test]
fn hide_column_says_the_refusal_and_keeps_nothing() {
    // A table of one column: hiding it leaves a select of nothing, which DuckDB refuses.
    let mut win = OpWindow::on_copy_prepared(WITH_NULLS, |dir| {
        std::fs::write(dir.join(WITH_NULLS), "city\nBerkeley\nOakland\n")
            .expect("the one-column table writes");
    });
    let before = win.snapshot();
    hide_with_key(&mut win, "city");
    assert!(win.kept().is_empty(), "{:?}", win.kept());
    assert!(win.making().is_none());
    let said = win.said().unwrap_or_default();
    assert!(!said.is_empty() && !said.starts_with("Hid"), "{said}");
    win.key(egui::Key::Enter);
    assert!(win.kept().is_empty());
    assert_eq!(win.snapshot(), before);
}
