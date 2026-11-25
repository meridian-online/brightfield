//! `dedupe-rows` in the window: one action with a sensible default. `shift-u`
//! on a cell, or "Remove duplicate rows" on a header's menu, asks nothing and
//! keeps `SELECT DISTINCT *` as soon as arc admits it and DuckDB has run it.
//! The kept step then opens in its form in the Operation section, where a key
//! and an order refine it in place.
//!
//! Each window is over a copy of a fixture in a folder of the test's own, and
//! arc is the `brightfield-shell` binary cargo built for the suite.

#[path = "support/operations.rs"]
mod operations;

use operations::OpWindow;

const SAMPLE: &str = "california_housing_sample.csv";
const WITH_NULLS: &str = "with_nulls.csv";

/// `with_nulls.csv` with Alameda's row twice: four rows, three of them distinct.
fn with_a_duplicate() -> OpWindow {
    OpWindow::on_copy_prepared(WITH_NULLS, |dir| {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join(WITH_NULLS))
            .expect("the fixture's copy opens");
        writeln!(file, "Alameda,28,1,1.5").expect("the row is written");
    })
}

/// Wait until arc's description has come and the key starts a draft, then
/// until the draft is answered and kept.
fn dedupe_with_key(win: &mut OpWindow) {
    win.cursor_on("house_age");
    for _ in 0..600 {
        win.press("shift-u");
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

/// The answer to the draft being made, once it has come.
fn answer(win: &OpWindow) -> brightfield_shell::operation_ui::DraftAnswer {
    win.app
        .making()
        .and_then(|making| making.answer.clone())
        .unwrap_or_else(|| panic!("the draft is answered: {:?}", win.said()))
}

/// Set the value `key` of the draft being made, as its form does, and wait
/// for the draft as it stands to be read.
fn set(win: &mut OpWindow, key: &'static str, value: &str) {
    win.app
        .chart_doc_mut()
        .making_mut()
        .expect("a draft is being made")
        .draft
        .set(key, value.to_string());
    win.settle_reads();
}

#[test]
fn dedupe_rows_keeps_select_distinct_at_once_and_opens_its_form() {
    let mut win = with_a_duplicate();
    let before = win.snapshot();
    dedupe_with_key(&mut win);
    let name = format!("{}_deduped", win.table());
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, name);
    assert!(
        kept[0].statement.contains("SELECT DISTINCT *"),
        "{}",
        kept[0].statement
    );
    let said = win.said().unwrap_or_default();
    assert!(said.starts_with(&format!("Kept {name}")), "{said}");
    // Kept, and no file written.
    assert_eq!(win.snapshot(), before);
    // The kept step is open again in its form, as a correction of itself.
    let making = win
        .app
        .making()
        .expect("the form is open over the kept step");
    assert_eq!(making.corrects.as_deref(), Some(name.as_str()));
    assert_eq!(making.draft.long_name, "dedupe-rows");
    win.settle_reads();
    let read = answer(&win);
    assert_eq!((read.rows_in, read.rows), (4, 3));
    win.settle();
    assert!(
        win.texts.iter().any(|t| t.contains("One row per")),
        "{:?}",
        win.texts
    );
    assert!(win.texts.iter().any(|t| t.contains("The first by")));
}

#[test]
fn dedupe_rows_shows_no_form_before_its_step_is_kept() {
    let mut win = with_a_duplicate();
    win.cursor_on("house_age");
    // Whenever the first draft is open, before its step is kept, it asks with no form.
    for _ in 0..600 {
        win.press("shift-u");
        if let Some(making) = win.app.making() {
            if making.corrects.is_none() {
                assert!(making.form().is_none(), "the first draft asks nothing");
                assert!(!win.texts.iter().any(|t| t.contains("One row per")));
            }
        }
        if !win.kept().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        win.run(Vec::new());
    }
    win.settle_reads();
    let making = win.app.making().expect("the kept step opens in its form");
    assert!(making.form().is_some());
}

#[test]
fn dedupe_rows_form_refines_the_kept_step_in_place_under_its_name() {
    let mut win = with_a_duplicate();
    dedupe_with_key(&mut win);
    let name = format!("{}_deduped", win.table());
    set(&mut win, "key", "city");
    set(&mut win, "order_by", "house_age desc");
    let editor = win.editor_text();
    assert!(editor.contains("QUALIFY"), "{editor}");
    win.key(egui::Key::Enter);
    win.settle();
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "replaced in place: {kept:?}");
    assert_eq!(kept[0].name, name, "a name is fixed when a draft is kept");
    assert!(
        kept[0].statement.contains("QUALIFY"),
        "{}",
        kept[0].statement
    );
    assert!(win.app.making().is_none(), "Enter kept the correction");

    let statement = kept[0].statement.clone();
    win.save()
        .unwrap_or_else(|e| panic!("the Save records: {e}"));
    assert!(win.manifest_steps().contains(&name));
    assert_eq!(win.model_of(&name), statement);
}

#[test]
fn dedupe_rows_order_emptied_with_the_key_kept_is_refused() {
    let mut win = with_a_duplicate();
    dedupe_with_key(&mut win);
    set(&mut win, "key", "city");
    set(&mut win, "order_by", "");
    win.key(egui::Key::Enter);
    win.settle();
    let said = win.said().unwrap_or_default();
    assert!(said.contains("needs order_by"), "{said}");
    // The draft stays open and the kept step is as it was.
    assert!(win.app.making().is_some());
    let kept = win.kept();
    assert_eq!(kept.len(), 1);
    assert!(kept[0].statement.contains("SELECT DISTINCT *"));
}

#[test]
fn dedupe_rows_escape_leaves_the_step_kept_at_once() {
    let mut win = with_a_duplicate();
    dedupe_with_key(&mut win);
    set(&mut win, "key", "city");
    win.key(egui::Key::Escape);
    win.settle();
    assert!(win.app.making().is_none());
    let kept = win.kept();
    assert_eq!(kept.len(), 1);
    assert!(kept[0].statement.contains("SELECT DISTINCT *"));
}

#[test]
fn dedupe_rows_on_a_table_without_duplicates_says_so_and_keeps_nothing() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    dedupe_with_key(&mut win);
    assert!(win.kept().is_empty(), "{:?}", win.kept());
    assert!(win.app.making().is_none());
    assert_eq!(
        win.said().as_deref(),
        Some(brightfield_shell::window::NO_DUPLICATE_ROWS)
    );
}

#[test]
fn dedupe_rows_from_the_header_menu_keeps_the_same_step() {
    let mut by_key = with_a_duplicate();
    dedupe_with_key(&mut by_key);
    let from_key = by_key.kept();

    let mut win = with_a_duplicate();
    win.cursor_on("city");
    // Wait for arc's description, which the header's menu reads.
    for _ in 0..600 {
        win.header_menu("city", "Remove duplicate rows");
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
fn dedupe_rows_over_its_kept_step_corrects_it() {
    let mut win = with_a_duplicate();
    dedupe_with_key(&mut win);
    let name = format!("{}_deduped", win.table());
    win.key(egui::Key::Escape);
    win.settle();
    win.wait_for_preview();
    // The kept step's grid is in front: `shift-u` there opens its form again.
    win.focus_grid();
    win.press("shift-u");
    win.settle_reads();
    let making = win
        .app
        .making()
        .unwrap_or_else(|| panic!("the form opens over the kept step: {:?}", win.said()));
    assert_eq!(making.corrects.as_deref(), Some(name.as_str()));
    assert_eq!(win.kept().len(), 1);
}
