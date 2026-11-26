//! `summarise-rows` in the window: the rows grouped by a column and counted at
//! once, then shaped in the form the inspector's Operation section draws.
//! `shift-f` on a cell, or "Summarise by this column" on a header's menu,
//! begins a count of each of the column's values; *+ group* and *+ value* add
//! to it, each edit previews arc's model, and Enter keeps the step.
//!
//! Each window is over a copy of a fixture in a folder of the test's own, and
//! arc is the `brightfield-shell` binary cargo built for the suite.

#[path = "support/operations.rs"]
mod operations;

use brightfield_shell::operation_ui::summarise_rows::{AGGREGATES_FIELD, GROUP_BY_FIELD};
use operations::OpWindow;

const SAMPLE: &str = "california_housing_sample.csv";

/// The rows of the housing sample.
const SAMPLE_ROWS: u64 = 240;

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

/// Begin the summary with `shift-f` on `column`'s cell, and wait for its
/// preview.
fn summarise_with_key(win: &mut OpWindow, column: &str) {
    win.cursor_on(column);
    until_making(win, |win| win.press("shift-f"));
    answered(win);
}

/// Wait for the draft's answer and the preview it gives.
fn answered(win: &mut OpWindow) {
    win.settle_reads();
    win.wait_for_preview();
}

/// Click the text drawn as exactly `text`.
fn click_exact(win: &mut OpWindow, text: &str) {
    let at = win
        .text_rects
        .iter()
        .find(|(drawn, _)| drawn == text)
        .unwrap_or_else(|| panic!("no text {text:?} was drawn: {:?}", win.texts))
        .1
        .center();
    win.click(at);
}

/// Replace the text of the form's field `id` with `text`.
fn retype(win: &mut OpWindow, id: &str, text: &str) {
    let rect = win
        .ctx
        .read_response(egui::Id::new(id))
        .unwrap_or_else(|| panic!("the form drew no field {id}"))
        .rect;
    win.click(rect.center());
    win.key_with(egui::Key::A, egui::Modifiers::COMMAND);
    win.type_text(text);
    answered(win);
}

/// The sum of the preview's column `column` over the page it holds.
fn sum_of(win: &OpWindow, column: &str) -> u64 {
    (0..)
        .map_while(|row| win.grid_cell(row, column))
        .map(|cell| cell.replace(',', "").parse::<u64>().expect("a count"))
        .sum()
}

#[test]
fn summarise_rows_starts_from_a_cell_as_a_count_of_each_value() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    summarise_with_key(&mut win, "house_age");
    let draft = win.making().expect("the summary is being made");
    assert_eq!(draft.long_name, "summarise-rows");
    assert_eq!(draft.on, Some(win.table()));
    assert_eq!(draft.value("group_by"), Some("house_age"));
    assert_eq!(draft.value("aggregates"), Some("count(*) AS n"));
    // The form is in the Operation section, under the operation's name.
    assert!(
        win.texts.iter().any(|t| t == "Summarise rows"),
        "{:?}",
        win.texts
    );
    assert!(win.texts.iter().any(|t| t == "GROUP BY"), "{:?}", win.texts);

    assert_eq!(win.preview_columns(), ["house_age", "n"]);
    // The warm preview holds the whole table, so the counts are every row's.
    assert_eq!(sum_of(&win, "n"), SAMPLE_ROWS);
    win.key(egui::Key::Enter);
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(
        kept[0].name,
        format!("{}_summarised_by_house_age", win.table())
    );
}

#[test]
fn summarise_rows_starts_from_the_menu_and_the_palette_with_the_same_defaults() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    win.cursor_on("house_age");
    until_making(&mut win, |win| {
        win.header_menu("house_age", "Summarise by this column");
        win.settle();
    });
    let draft = win.making().expect("the menu began it");
    assert_eq!(draft.value("group_by"), Some("house_age"));
    assert_eq!(draft.value("aggregates"), Some("count(*) AS n"));
    win.key(egui::Key::Escape);
    assert!(win.making().is_none());

    win.focus_grid();
    until_making(&mut win, |win| win.palette("summarise-rows"));
    let draft = win.making().expect("the palette began it");
    assert_eq!(draft.value("group_by"), Some("house_age"));
    assert_eq!(draft.value("aggregates"), Some("count(*) AS n"));
}

#[test]
fn the_form_adds_a_value_and_a_group_and_the_name_stays() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    summarise_with_key(&mut win, "house_age");

    win.click_text("+ value");
    win.settle();
    click_exact(&mut win, "avg(median_income)");
    answered(&mut win);
    let draft = win.making().expect("still being made");
    assert_eq!(
        draft.value("aggregates"),
        Some("count(*) AS n, avg(median_income) AS avg_median_income")
    );
    assert_eq!(
        win.preview_columns(),
        ["house_age", "n", "avg_median_income"]
    );

    // The value's function is a chip of its own.
    click_exact(&mut win, "avg");
    win.settle();
    click_exact(&mut win, "sum");
    answered(&mut win);
    assert_eq!(
        win.making().expect("still being made").value("aggregates"),
        Some("count(*) AS n, sum(median_income) AS sum_median_income")
    );

    win.click_text("+ group");
    win.settle();
    click_exact(&mut win, "by latitude");
    answered(&mut win);
    assert_eq!(
        win.making().expect("still being made").value("group_by"),
        Some("house_age, latitude")
    );
    assert_eq!(
        win.preview_columns(),
        ["house_age", "latitude", "n", "sum_median_income"]
    );

    // A group's chip takes it off again.
    click_exact(&mut win, "latitude \u{00d7}");
    answered(&mut win);
    assert_eq!(
        win.making().expect("still being made").value("group_by"),
        Some("house_age")
    );
    click_exact(&mut win, "+ group");
    win.settle();
    click_exact(&mut win, "by latitude");
    answered(&mut win);

    win.key(egui::Key::Enter);
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(
        kept[0].name,
        format!("{}_summarised_by_house_age", win.table())
    );
}

#[test]
fn the_select_lists_are_typed_as_text_and_a_constant_is_refused() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    summarise_with_key(&mut win, "house_age");
    assert!(!win.grid_faded());

    retype(
        &mut win,
        AGGREGATES_FIELD,
        "count(*) AS rows, max(population) AS most",
    );
    assert_eq!(win.preview_columns(), ["house_age", "rows", "most"]);
    assert_eq!(sum_of(&win, "rows"), SAMPLE_ROWS);

    retype(&mut win, GROUP_BY_FIELD, "2");
    let said = win.said().unwrap_or_default();
    assert!(said.contains("constant"), "arc's reason: {said:?}");
    assert!(win.grid_faded(), "the last good rows stay, faded");
    win.key(egui::Key::Enter);
    assert!(win.kept().is_empty(), "a refused draft keeps nothing");
    assert!(win.making().is_some(), "the form stays open on a refusal");
}
