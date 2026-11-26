//! `join-tables` in the window: the table in front joined to a second table
//! picked in the spine. `&` on a cell, "Join on this column" on a header's
//! menu, or the palette opens the pick; Enter on a table begins the join with
//! its keys proposed, the form in the inspector's Operation section shapes it,
//! each edit previews arc's model, and Enter keeps the step.
//!
//! The folder is a copy of the housing sample with `by_age` recorded through
//! arc, which counts the rows by `house_age`, and arc is the `brightfield-shell`
//! binary cargo built for the suite.

#[path = "support/operations.rs"]
mod operations;

use std::path::Path;
use std::time::{Duration, Instant};

use brightfield_shell::data_file;
use brightfield_shell::operation::{self, StepRequest};
use brightfield_shell::operation_ui::join_tables::{CONDITION_FIELD, PREFIX_FIELD};
use operations::{runner, OpWindow};

const SAMPLE: &str = "california_housing_sample.csv";
const ROOT: &str = "california_housing_sample";

/// Save the sample's Protocol into `dir`, so its load is written, and record
/// `by_age` on the loaded table.
fn by_age(dir: &Path) {
    let opened = data_file::open(&dir.join(SAMPLE).to_string_lossy()).expect("the sample opens");
    opened.protocol.save_to(dir).expect("the Protocol saves");
    let request = StepRequest {
        long_name: "summarise-rows".to_string(),
        on: Some(ROOT.to_string()),
        reads: Vec::new(),
        args: vec![
            ("aggregates".to_string(), "count(*) AS n".to_string()),
            ("group_by".to_string(), "house_age".to_string()),
        ],
    };
    operation::record_step(&runner(), dir, "by_age", &request)
        .unwrap_or_else(|e| panic!("arc records by_age: {e}"));
}

/// A window on the sample with `by_age` recorded.
fn window() -> OpWindow {
    OpWindow::on_copy_prepared(SAMPLE, by_age)
}

/// Whether the status band says arc's description is still on its way.
fn reading_description(win: &OpWindow) -> bool {
    win.app
        .operation_said()
        .is_some_and(|said| said.starts_with("Reading arc's description"))
}

/// Run `start` until arc's description has come and the spine's pick opens,
/// then draw frames until it knows each candidate's columns.
fn until_picking(win: &mut OpWindow, mut start: impl FnMut(&mut OpWindow)) {
    for _ in 0..600 {
        start(win);
        if win.app.spine_pick().is_some() {
            let deadline = Instant::now() + Duration::from_secs(60);
            while win
                .app
                .spine_pick()
                .is_some_and(|pick| pick.candidates().iter().any(|c| c.columns.is_none()))
            {
                assert!(Instant::now() < deadline, "the columns never arrived");
                std::thread::sleep(Duration::from_millis(5));
                win.run(Vec::new());
            }
            win.settle();
            return;
        }
        assert!(reading_description(win), "{:?}", win.said());
        std::thread::sleep(Duration::from_millis(20));
        win.run(Vec::new());
    }
    panic!("arc's description never came");
}

/// Take the table under the pick's cursor, which must be `table`, and wait
/// for the join's preview.
fn take(win: &mut OpWindow, table: &str) {
    let under = win
        .app
        .spine_pick()
        .and_then(|pick| pick.under_cursor())
        .map(|c| c.table.clone());
    assert_eq!(under.as_deref(), Some(table));
    win.key(egui::Key::Enter);
    assert!(win.making().is_some(), "{:?}", win.said());
    win.settle_reads();
    win.wait_for_preview();
}

/// Join with `&` on `column`'s cell, taking `by_age`.
fn join_with_key(win: &mut OpWindow, column: &str) {
    win.cursor_on(column);
    until_picking(win, |win| win.press("&"));
    take(win, "by_age");
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
    win.settle_reads();
}

#[test]
fn join_tables_starts_from_a_cell_with_the_shared_column_as_its_key() {
    let mut win = window();
    join_with_key(&mut win, "house_age");
    let draft = win.making().expect("the join is being made");
    assert_eq!(draft.long_name, "join-tables");
    assert_eq!(draft.on.as_deref(), Some(ROOT));
    assert_eq!(draft.reads, [("other".to_string(), "by_age".to_string())]);
    assert_eq!(draft.value("using"), Some("house_age"));
    assert_eq!(draft.value("condition"), None);
    // The form is in the Operation section, and reads the join as SQL with
    // the column it adds.
    assert!(win.texts.iter().any(|t| t == "JOIN"), "{:?}", win.texts);
    assert!(
        win.texts
            .iter()
            .any(|t| t == "left join by_age using (house_age)"),
        "{:?}",
        win.texts
    );
    assert!(
        win.texts.iter().any(|t| t == "Adds 1 column"),
        "{:?}",
        win.texts
    );
}

#[test]
fn join_tables_previews_arcs_model_as_it_is_edited() {
    let mut win = window();
    join_with_key(&mut win, "house_age");
    let columns = win.grid_columns();
    assert_eq!(columns[0], "house_age", "{columns:?}");
    assert!(columns.contains(&"by_age_n".to_string()), "{columns:?}");
    let editor = win.editor_text();
    assert!(
        editor.contains("LEFT JOIN by_age USING (house_age)"),
        "{editor}"
    );

    // A prefix typed renames the other table's columns.
    retype(&mut win, PREFIX_FIELD, "age_");
    let columns = win.grid_columns();
    assert!(columns.contains(&"age_n".to_string()), "{columns:?}");

    // A condition typed replaces the columns the join was on.
    retype(
        &mut win,
        CONDITION_FIELD,
        &format!("{ROOT}.house_age = by_age.house_age"),
    );
    let draft = win.making().expect("still being made");
    assert_eq!(draft.value("using"), Some(""));
    let editor = win.editor_text();
    assert!(
        editor.contains("ON california_housing_sample.house_age = by_age.house_age"),
        "{editor}"
    );
    // The condition's join brings `by_age`'s key through as well.
    let columns = win.grid_columns();
    assert!(
        columns.contains(&"age_house_age".to_string()),
        "{columns:?}"
    );

    // Inner keeps the rows that match: every row, since each age is counted.
    win.click_text("inner");
    win.settle_reads();
    assert_eq!(win.making().unwrap().value("kind"), Some("inner"));
    assert!(win.editor_text().contains("INNER JOIN"));
}

#[test]
fn join_tables_keeps_a_step_named_by_its_effect_and_save_records_arcs_model() {
    let mut win = window();
    join_with_key(&mut win, "house_age");
    let before = win.snapshot();
    win.key(egui::Key::Enter);
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?} {}", win.said(), win.banners());
    let name = format!("{ROOT}_joined_by_age");
    assert_eq!(kept[0].name, name);
    assert!(
        kept[0]
            .statement
            .contains("LEFT JOIN by_age USING (house_age)"),
        "{}",
        kept[0].statement
    );
    assert!(win.making().is_none());
    // Kept, and no file written.
    assert_eq!(win.snapshot(), before);

    let statement = kept[0].statement.clone();
    win.save()
        .unwrap_or_else(|e| panic!("the Save records: {e}"));
    assert!(win.manifest_steps().contains(&name));
    assert_eq!(win.model_of(&name), statement);
}

#[test]
fn a_kept_join_is_corrected_in_its_form_and_read_as_it_was_made() {
    let mut win = window();
    join_with_key(&mut win, "house_age");
    win.key(egui::Key::Enter);
    assert_eq!(win.kept().len(), 1, "{:?}", win.said());
    win.wait_for_preview();

    // `&` over the kept join's own grid corrects it, with no pick.
    for _ in 0..600 {
        win.press("&");
        if win.making().is_some() {
            break;
        }
        assert!(reading_description(&win), "{:?}", win.said());
        std::thread::sleep(Duration::from_millis(20));
        win.run(Vec::new());
    }
    assert!(win.app.spine_pick().is_none());
    assert_eq!(
        win.app.making().and_then(|m| m.corrects.clone()),
        Some(format!("{ROOT}_joined_by_age"))
    );
    win.settle_reads();
    assert!(
        win.texts
            .iter()
            .any(|t| t == "left join by_age using (house_age)"),
        "{:?}",
        win.texts
    );
    assert!(
        !win.texts.iter().any(|t| t.starts_with("Adds ")),
        "{:?}",
        win.texts
    );
}

#[test]
fn join_tables_reopens_as_a_step_not_run_that_previews_the_same() {
    let mut win = window();
    join_with_key(&mut win, "house_age");
    let columns = win.grid_columns();
    win.key(egui::Key::Enter);
    win.save()
        .unwrap_or_else(|e| panic!("the Save records: {e}"));
    let name = format!("{ROOT}_joined_by_age");

    let mut again = win.reopen();
    assert!(!again.step_ran(&name));
    again.show_grid_of(&name);
    assert_eq!(again.preview_columns(), columns);
}

#[test]
fn join_tables_starts_from_the_header_menu_and_the_palette_with_the_same_draft() {
    let mut win = window();
    join_with_key(&mut win, "house_age");
    let from_key = win.making().expect("the join is being made");

    let mut win = window();
    win.cursor_on("house_age");
    until_picking(&mut win, |win| {
        win.header_menu("house_age", "Join on this column");
        win.settle();
    });
    take(&mut win, "by_age");
    assert_eq!(win.making(), Some(from_key.clone()));

    let mut win = window();
    win.cursor_on("house_age");
    until_picking(&mut win, |win| win.palette("join-tables"));
    take(&mut win, "by_age");
    assert_eq!(win.making(), Some(from_key));
}

#[test]
fn a_join_started_on_a_column_the_other_lacks_is_on_the_first_both_hold() {
    let mut win = window();
    join_with_key(&mut win, "median_income");
    assert_eq!(win.making().unwrap().value("using"), Some("house_age"));
}

#[test]
fn join_tables_with_no_other_table_says_what_it_needs() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    win.cursor_on("house_age");
    for _ in 0..600 {
        win.press("&");
        if !reading_description(&win) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
        win.run(Vec::new());
    }
    assert!(win.app.spine_pick().is_none());
    assert!(win.making().is_none());
    assert_eq!(
        win.said().as_deref(),
        Some("Join tables needs a second table in the Protocol.")
    );
}

#[test]
fn join_tables_says_the_refusal_and_keeps_nothing() {
    let mut win = window();
    join_with_key(&mut win, "house_age");
    let before = win.snapshot();

    // A cross join is on no key, and the key is kept.
    win.click_text("cross");
    win.settle_reads();
    let draft = win.making().expect("still being made");
    assert_eq!(draft.value("kind"), Some("cross"));
    assert_eq!(draft.value("using"), Some("house_age"));
    let said = win.said().unwrap_or_default();
    assert!(
        said.contains(
            "exactly one of `using` or `condition` is required for every kind but `cross`"
        ),
        "{said}"
    );
    assert!(win.grid_faded());
    win.key(egui::Key::Enter);
    assert!(win.kept().is_empty());
    assert!(win.making().is_some());
    assert_eq!(win.snapshot(), before);
}
