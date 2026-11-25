//! `limit-rows` in the window: the top rows by a column, asked in the
//! inspector's Operation section. `shift-l` on a cell, "Top rows by this
//! column" on a header's menu, or the palette opens ten rows, largest first;
//! the count field takes Up and Down, the direction words flip the order, each
//! change previews arc's model, and Enter keeps the step.
//!
//! Each window is over a copy of a fixture in a folder of the test's own, and
//! arc is the `brightfield-shell` binary cargo built for the suite.

#[path = "support/operations.rs"]
mod operations;

use operations::OpWindow;

const SAMPLE: &str = "california_housing_sample.csv";

/// Whether the status band says arc's description is still on its way, which
/// a start waits out.
fn reading_description(win: &OpWindow) -> bool {
    win.app
        .operation_said()
        .is_some_and(|said| said.starts_with("Reading arc's description"))
}

/// Run `start` until arc's description has come and it begins a draft.
fn until_making(win: &mut OpWindow, mut start: impl FnMut(&mut OpWindow)) {
    for _ in 0..600 {
        start(win);
        if win.making().is_some() {
            return;
        }
        assert!(reading_description(win), "{:?}", win.said());
        std::thread::sleep(std::time::Duration::from_millis(20));
        win.run(Vec::new());
    }
    panic!("arc's description never came");
}

/// Open the limit with `shift-l` on `column`'s cell, and wait for its rows.
fn limit_with_key(win: &mut OpWindow, column: &str) {
    win.cursor_on(column);
    until_making(win, |win| win.press("shift-l"));
    win.settle_reads();
}

/// How many rows the grid's preview page holds.
fn preview_rows(win: &OpWindow) -> usize {
    win.app
        .chart_doc()
        .grid_preview()
        .and_then(|p| p.page())
        .map_or(0, |page| page.rows.len())
}

/// The value of `column` in row `row` of the preview, as a number.
fn number(win: &OpWindow, row: usize, column: &str) -> f64 {
    let cell = win.grid_cell(row, column).expect("the cell is in the page");
    cell.parse()
        .unwrap_or_else(|_| panic!("{cell:?} is not a number"))
}

/// Every value of `column` in the fixture, as numbers.
fn fixture_values(win: &OpWindow, column: &str) -> Vec<f64> {
    let text = std::fs::read_to_string(win.dir().join(SAMPLE)).expect("the fixture reads");
    let mut lines = text.lines();
    let at = lines
        .next()
        .unwrap_or_default()
        .split(',')
        .position(|name| name == column)
        .expect("the column is in the fixture");
    lines
        .map(|line| {
            line.split(',')
                .nth(at)
                .expect("a value")
                .parse()
                .expect("a number")
        })
        .collect()
}

/// Empty the count field, which holds the keyboard, and type `text` into it.
fn type_count(win: &mut OpWindow, text: &str) {
    win.key(egui::Key::End);
    for _ in 0..8 {
        win.key(egui::Key::Backspace);
    }
    win.type_text(text);
}

#[test]
fn limit_rows_opens_on_ten_rows_largest_first_and_keeps_on_enter() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    limit_with_key(&mut win, "median_house_value");
    let draft = win.making().expect("the limit is being made");
    assert_eq!(draft.long_name, "limit-rows");
    assert_eq!(draft.on, Some(win.table()));
    assert_eq!(draft.value("count"), Some("10"));
    assert_eq!(draft.value("order_by"), Some("median_house_value desc"));

    let values = fixture_values(&win, "median_house_value");
    let greatest = values.iter().copied().fold(f64::MIN, f64::max);
    let least = values.iter().copied().fold(f64::MAX, f64::min);
    assert_eq!(preview_rows(&win), 10, "{:?}", win.said());
    assert_eq!(number(&win, 0, "median_house_value"), greatest);

    // Up in the count field gives one row more.
    win.key(egui::Key::ArrowUp);
    win.settle_reads();
    assert_eq!(win.making().unwrap().value("count"), Some("11"));
    assert_eq!(preview_rows(&win), 11, "{:?}", win.said());

    // Smallest first puts the least value at the top.
    win.click_text("smallest first");
    win.settle_reads();
    assert_eq!(
        win.making().unwrap().value("order_by"),
        Some("median_house_value asc")
    );
    assert_eq!(number(&win, 0, "median_house_value"), least);

    let before = win.snapshot();
    win.key(egui::Key::Enter);
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, format!("{}_first_11", win.table()));
    assert!(win.making().is_none());
    // Kept, and no file written.
    assert_eq!(win.snapshot(), before);
}

#[test]
fn shift_with_up_and_down_steps_the_count_by_ten_and_stops_at_none() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    limit_with_key(&mut win, "house_age");
    win.key_with(egui::Key::ArrowUp, egui::Modifiers::SHIFT);
    assert_eq!(win.making().unwrap().value("count"), Some("20"));
    win.key(egui::Key::ArrowDown);
    assert_eq!(win.making().unwrap().value("count"), Some("19"));
    for _ in 0..3 {
        win.key_with(egui::Key::ArrowDown, egui::Modifiers::SHIFT);
    }
    assert_eq!(win.making().unwrap().value("count"), Some("0"));
    win.settle_reads();
    assert_eq!(preview_rows(&win), 0, "{:?}", win.said());
}

#[test]
fn limit_rows_starts_from_the_header_menu_and_the_palette_with_the_same_draft() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    limit_with_key(&mut win, "house_age");
    let from_key = win.making().expect("the limit is being made");
    assert_eq!(from_key.value("order_by"), Some("house_age desc"));

    let mut win = OpWindow::on_copy_of(SAMPLE);
    win.cursor_on("house_age");
    until_making(&mut win, |win| {
        win.header_menu("house_age", "Top rows by this column");
    });
    assert_eq!(win.making(), Some(from_key.clone()));

    let mut win = OpWindow::on_copy_of(SAMPLE);
    win.cursor_on("house_age");
    until_making(&mut win, |win| win.palette("limit-rows"));
    assert_eq!(win.making(), Some(from_key));
}

#[test]
fn a_count_arc_refuses_keeps_the_rows_faded_with_arcs_reason() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    limit_with_key(&mut win, "median_house_value");

    type_count(&mut win, "ten");
    win.settle_reads();
    assert_eq!(win.making().unwrap().value("count"), Some("ten"));
    let said = win.said().unwrap_or_default();
    assert!(said.contains("is an integer, not \"ten\""), "{said}");
    assert!(win.grid_faded());
    // Up leaves a count that is not a number as typed.
    win.key(egui::Key::ArrowUp);
    assert_eq!(win.making().unwrap().value("count"), Some("ten"));

    type_count(&mut win, "-1");
    win.settle_reads();
    let said = win.said().unwrap_or_default();
    assert!(said.contains("has a minimum of 0, not -1"), "{said}");
    win.key(egui::Key::Enter);
    assert!(win.kept().is_empty());
    assert!(win.making().is_some());
}

#[test]
fn the_order_is_typed_over_several_columns_and_names_no_step_by_it() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    limit_with_key(&mut win, "house_age");
    // Click into the order and type a second column after it.
    win.click_text("house_age desc");
    win.key(egui::Key::End);
    win.type_text(", median_income");
    win.settle_reads();
    assert_eq!(
        win.making().unwrap().value("order_by"),
        Some("house_age desc, median_income")
    );
    assert_eq!(preview_rows(&win), 10, "{:?}", win.said());
    // The direction words read the first item.
    win.click_text("smallest first");
    assert_eq!(
        win.making().unwrap().value("order_by"),
        Some("house_age asc, median_income")
    );
    win.settle_reads();
    win.key(egui::Key::Enter);
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, format!("{}_first_10", win.table()));
}
