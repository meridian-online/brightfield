//! `cast-column` in the window: the type typed in place over a column's
//! storage label in its header, starting from the type FineType reads. A
//! click on the header's type, `shift-t` on a cell, or "Change type" on a
//! header's menu opens the field, each edit previews arc's model, and Enter
//! keeps the step.
//!
//! Each window is over a copy of a fixture in a folder of the test's own, and
//! arc is the `brightfield-shell` binary cargo built for the suite. A column
//! is given a FineType label by a record nominating it, written before the
//! window opens, as an analyst's choice of label is kept.

#[path = "support/operations.rs"]
mod operations;

use brightfield_shell::operation_ui::header::Gesture;
use brightfield_shell::operation_ui::HeaderPart;
use brightfield_shell::{column_record, data_file};
use operations::OpWindow;

const SAMPLE: &str = "california_housing_sample.csv";
const WITH_NULLS: &str = "with_nulls.csv";
const DECIMAL: &str = "representation.numeric.decimal_number";

/// A copy of `fixture` whose record nominates each of `labels` for its
/// column, opened.
fn labelled(fixture: &str, labels: &[(&str, &str)]) -> OpWindow {
    let labels: Vec<(String, String)> = labels
        .iter()
        .map(|(column, label)| ((*column).to_string(), (*label).to_string()))
        .collect();
    OpWindow::on_copy_prepared(fixture, move |dir| {
        let mut record = column_record::SourceRecord::default();
        for (column, label) in labels {
            record.fields.insert(
                column,
                column_record::FieldRecord {
                    label: Some(label),
                    nominated: true,
                    ..column_record::FieldRecord::default()
                },
            );
        }
        let package = column_record::flat_record_path(&dir.join(fixture)).expect("a record path");
        column_record::write_record(&package, data_file::SOURCE, &record)
            .expect("the record writes");
    })
}

/// The housing sample with `house_age`, stored as `BIGINT`, labelled a
/// decimal number, which FineType reads as `DOUBLE`.
fn house_age_as_decimal() -> OpWindow {
    labelled(SAMPLE, &[("house_age", DECIMAL)])
}

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

/// Open the cast's field with a click on `column`'s type.
fn cast_from_header(win: &mut OpWindow, column: &str) {
    until_making(win, |win| {
        win.header(column, HeaderPart::Type, Gesture::Click);
    });
}

/// Open the cast's field with `shift-t` on `column`'s cell.
fn cast_with_key(win: &mut OpWindow, column: &str) {
    win.cursor_on(column);
    until_making(win, |win| win.press("shift-t"));
}

/// Type `text` into the field in place and wait for the draft's answer.
fn type_in(win: &mut OpWindow, text: &str) {
    win.type_text(text);
    win.settle_reads();
    win.wait_for_preview();
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

/// The DuckDB type of `column` in the table `step`, which `model` makes over
/// the data file `fixture` in `dir`, loaded as `table`.
fn type_in_model(
    dir: &std::path::Path,
    fixture: &str,
    table: &str,
    (step, model): (&str, &str),
    column: &str,
) -> String {
    let conn = duckdb::Connection::open_in_memory().expect("an in-memory DuckDB");
    let csv = dir.join(fixture);
    conn.execute_batch(&format!(
        "CREATE TABLE \"{table}\" AS SELECT * FROM read_csv_auto('{}')",
        csv.to_string_lossy().replace('\'', "''")
    ))
    .expect("the data file loads");
    conn.execute_batch(model)
        .unwrap_or_else(|e| panic!("the model runs: {e}\n{model}"));
    conn.query_row(
        &format!("SELECT typeof(\"{column}\") FROM \"{step}\" LIMIT 1"),
        [],
        |row| row.get::<_, String>(0),
    )
    .unwrap_or_else(|e| panic!("{step} reads: {e}"))
}

#[test]
fn cast_column_starts_from_a_header_with_its_defaults() {
    let mut win = house_age_as_decimal();
    cast_from_header(&mut win, "house_age");
    let draft = win.making().expect("the cast is being made");
    assert_eq!(draft.long_name, "cast-column");
    assert_eq!(draft.on, Some(win.table()));
    assert_eq!(draft.value("column"), Some("house_age"));
    assert_eq!(draft.value("type"), Some("DOUBLE"));
    assert_eq!(win.field_text().as_deref(), Some("DOUBLE"));
    assert!(selected_whole(&win), "FineType's type opens selected whole");
    // FineType's type first, then the common types, less the storage type.
    assert_eq!(
        win.suggestions(),
        [
            "DOUBLE",
            "VARCHAR",
            "DECIMAL(18, 3)",
            "DATE",
            "TIMESTAMP",
            "BOOLEAN"
        ]
    );
}

#[test]
fn cast_column_starts_from_a_cell_and_the_menu_with_the_same_defaults() {
    let mut win = house_age_as_decimal();
    cast_with_key(&mut win, "house_age");
    assert_eq!(win.field_text().as_deref(), Some("DOUBLE"));
    let from_key = win.making().expect("the cast is being made");

    let mut win = house_age_as_decimal();
    win.cursor_on("house_age");
    until_making(&mut win, |win| win.header_menu("house_age", "Change type"));
    assert_eq!(win.making(), Some(from_key));
    assert_eq!(win.field_text().as_deref(), Some("DOUBLE"));
}

#[test]
fn cast_column_on_a_column_fine_type_agrees_with_opens_empty() {
    // `median_income` is stored as `DOUBLE`, and FineType reads its label as
    // `DOUBLE` too, so the field opens with no type in it.
    let mut win = labelled(SAMPLE, &[("median_income", DECIMAL)]);
    cast_from_header(&mut win, "median_income");
    let draft = win.making().expect("the cast is being made");
    assert_eq!(draft.value("type"), Some(""));
    assert_eq!(win.field_text().as_deref(), Some(""));
    assert!(!selected_whole(&win), "an empty field has nothing selected");
    assert_eq!(
        win.suggestions(),
        [
            "VARCHAR",
            "BIGINT",
            "DECIMAL(18, 3)",
            "DATE",
            "TIMESTAMP",
            "BOOLEAN"
        ]
    );
    // An empty type is not asked of arc, so Enter says it is still needed.
    win.key(egui::Key::Enter);
    assert_eq!(win.said().as_deref(), Some("cast-column needs type"));
    assert!(win.kept().is_empty(), "{:?}", win.kept());
}

#[test]
fn cast_column_previews_arcs_model_as_it_is_edited() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    cast_with_key(&mut win, "median_income");
    // The type it is stored as reads as it is.
    type_in(&mut win, "DOUBLE");
    assert_eq!(win.grid_cell(0, "median_income").as_deref(), Some("0.9302"));
    for _ in 0.."DOUBLE".len() {
        win.key(egui::Key::Backspace);
    }
    // What is typed filters the types offered.
    win.type_text("BIG");
    assert_eq!(win.suggestions(), ["BIGINT"]);
    type_in(&mut win, "INT");
    assert_eq!(win.field_text().as_deref(), Some("BIGINT"));
    // The column's cells re-read as the type.
    assert_eq!(win.grid_cell(0, "median_income").as_deref(), Some("1"));
    assert_eq!(win.grid_columns().len(), 9);
    let editor = win.editor_text();
    assert!(
        editor.contains("CAST(") && editor.contains("BIGINT"),
        "{editor}"
    );
}

#[test]
fn cast_column_takes_a_suggestion_whole() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    cast_with_key(&mut win, "median_income");
    win.type_text("dec");
    assert_eq!(win.suggestions(), ["DECIMAL(18, 3)"]);
    win.key(egui::Key::ArrowDown);
    win.key(egui::Key::Tab);
    assert_eq!(win.field_text().as_deref(), Some("DECIMAL(18, 3)"));
}

#[test]
fn cast_column_keeps_a_step_named_by_its_effect_and_save_records_arcs_model() {
    let mut win = house_age_as_decimal();
    cast_from_header(&mut win, "house_age");
    win.settle_reads();
    win.wait_for_preview();
    win.key(egui::Key::Enter);
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    let name = format!("{}_cast_house_age", win.table());
    assert_eq!(name, "california_housing_sample_cast_house_age");
    assert_eq!(kept[0].name, name);
    assert!(win.making().is_none());

    let statement = kept[0].statement.clone();
    win.save()
        .unwrap_or_else(|e| panic!("the Save records: {e}"));
    assert!(win.manifest_steps().contains(&name));
    let model = win.model_of(&name);
    assert_eq!(model, statement);
    // The rows the step makes hold house_age as the type FineType read.
    assert_eq!(
        type_in_model(
            win.dir(),
            SAMPLE,
            &win.table(),
            (&name, &model),
            "house_age"
        ),
        "DOUBLE"
    );
}

#[test]
fn cast_column_reopens_as_a_step_not_run_that_previews_the_same() {
    let mut win = house_age_as_decimal();
    cast_from_header(&mut win, "house_age");
    win.settle_reads();
    win.wait_for_preview();
    win.key(egui::Key::Enter);
    let name = "california_housing_sample_cast_house_age";
    assert_eq!(win.kept().len(), 1, "{:?}", win.said());
    win.wait_for_preview();
    let columns = win.grid_columns();
    assert_eq!(columns.len(), 9);
    win.save()
        .unwrap_or_else(|e| panic!("the Save records: {e}"));
    let mut again = win.reopen();
    assert!(!again.step_ran(name));
    again.show_grid_of(name);
    assert_eq!(again.grid_columns(), columns);
}

#[test]
fn cast_column_gives_a_column_needing_quotes_bare_and_names_it_whole() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    cast_with_key(&mut win, "median income");
    type_in(&mut win, "VARCHAR");
    win.key(egui::Key::Enter);
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, "with_nulls_cast_median_income");
    assert!(
        kept[0].statement.contains("\"median income\""),
        "{}",
        kept[0].statement
    );
}

#[test]
fn cast_column_says_the_refusal_and_keeps_nothing() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    let before = win.snapshot();
    cast_with_key(&mut win, "city");
    // A good type first, so there are rows to fade.
    type_in(&mut win, "VARCHAR");
    assert!(!win.grid_faded());

    // Not one SQL type: arc's reason, the rows faded.
    for _ in 0.."VARCHAR".len() {
        win.key(egui::Key::Backspace);
    }
    win.type_text("DECIMAL(10");
    win.settle_reads();
    let said = win.said().unwrap_or_default();
    assert!(
        said.contains("the type `type` of `cast-column` is not one SQL type"),
        "{said}"
    );
    assert!(win.grid_faded(), "the last good rows stay, faded");
    win.key(egui::Key::Enter);
    assert!(win.kept().is_empty(), "{:?}", win.kept());
    assert!(win.making().is_some(), "the field stays open on a refusal");

    // A type the values will not take: DuckDB's reason, since arc's cast is a
    // `CAST` and the run would fail too.
    for _ in 0.."DECIMAL(10".len() {
        win.key(egui::Key::Backspace);
    }
    win.type_text("BIGINT");
    win.settle_reads();
    let said = win.said().unwrap_or_default();
    assert!(said.contains("Alameda"), "{said}");
    assert!(win.grid_faded());
    win.key(egui::Key::Enter);
    assert!(win.kept().is_empty(), "{:?}", win.kept());
    assert_eq!(win.snapshot(), before);
}
