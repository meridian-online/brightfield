//! An operation's draft kept as an unsaved step: laid out at the spine's
//! foot, recorded through arc on Save as arc writes it, taken back from the
//! spine, and its SQL shown read-only in the Editor.
//!
//! The window is over a copy of the housing sample in a folder of the test's
//! own, and arc is the `brightfield-shell` binary cargo built for the suite.

#[path = "support/operations.rs"]
mod operations;

use brightfield_shell::operation::{self, StepRequest};
use brightfield_shell::operation_ui::draft::draft_model;
use operations::{runner, OpWindow};

const SAMPLE: &str = "california_housing_sample.csv";

fn request(long_name: &str, on: &str, args: &[(&str, &str)]) -> StepRequest {
    StepRequest {
        long_name: long_name.to_string(),
        on: Some(on.to_string()),
        reads: Vec::new(),
        args: args
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
    }
}

/// `hide-column` of `house_age` on `on`.
fn hide(on: &str) -> StepRequest {
    request("hide-column", on, &[("column", "house_age")])
}

/// `rename-column` of `median_income` to `income` on `on`.
fn rename(on: &str) -> StepRequest {
    request(
        "rename-column",
        on,
        &[("column", "median_income"), ("to", "income")],
    )
}

/// arc's model for `request` as the step `name`.
fn model(name: &str, request: &StepRequest) -> String {
    draft_model(&runner(), name, request)
        .unwrap_or_else(|e| panic!("arc drafts {name}: {e}"))
        .model
}

/// Keep `request` as the step `name`, with arc's model for it.
fn keep(win: &mut OpWindow, name: &str, request: &StepRequest) -> String {
    let model = model(name, request);
    let ctx = win.ctx.clone();
    win.app
        .keep_draft(&ctx, name.to_string(), request.clone(), model.clone());
    win.settle();
    win.wait_for_preview();
    model
}

#[test]
fn a_kept_draft_is_an_unsaved_step_at_the_spines_foot() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    assert!(!win.app.has_unsaved_work());
    let before = win.snapshot();
    let request = hide(&win.table());
    let model = keep(&mut win, "h", &request);

    assert!(win.app.has_unsaved_work());
    let kept = win.kept();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].name, "h");
    assert_eq!(kept[0].operation.as_ref(), Some(&request));
    assert_eq!(kept[0].statement, model);
    assert_eq!(kept[0].reads, [win.table()]);
    assert!(win.app.protocol_model().is_typed("h"));
    assert!(
        win.spine_labels().iter().any(|l| l == "Step h"),
        "{:?}",
        win.spine_labels()
    );
    // Its grid is in front, previewed by running arc's model.
    let columns = win.preview_columns();
    assert!(!columns.is_empty(), "the grid previews h");
    assert!(!columns.contains(&"house_age".to_string()), "{columns:?}");
    assert!(
        columns.contains(&"median_income".to_string()),
        "{columns:?}"
    );
    assert_eq!(win.app.chart_doc().typed_lines(), ["hide-column \u{b7} h"]);
    // Keeping wrote no file.
    assert_eq!(win.snapshot(), before);
}

#[test]
fn save_records_a_kept_draft_as_arc_writes_it() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    let request = hide(&win.table());
    let model = keep(&mut win, "h", &request);
    win.save()
        .unwrap_or_else(|e| panic!("the Save records: {e}"));
    assert_eq!(win.model_of("h"), model);
    assert!(win.kept().is_empty());
    assert!(!win.app.has_unsaved_work());
    assert!(!win.app.protocol_model().is_typed("h"));
    // Reopened, the folder's step is the operation's, laid out and previewed.
    let mut again = win.reopen();
    again.show_grid_of("h");
    assert!(!again.preview_columns().contains(&"house_age".to_string()));
    assert!(!again.step_ran("h"));
}

#[test]
fn save_records_statements_and_drafts_in_the_order_they_were_made() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    let table = win.table();
    // A statement typed on the form and kept, then a draft.
    win.key(egui::Key::Space);
    win.type_text(&format!(
        "select house_age, count(*) as n from {table} group by house_age"
    ));
    assert!(win
        .app
        .finish_typed_reads(std::time::Duration::from_secs(60)));
    win.key_with(egui::Key::Enter, egui::Modifiers::COMMAND);
    let statement = win
        .kept()
        .last()
        .expect("the statement is kept")
        .name
        .clone();
    keep(&mut win, "h", &hide(&table));

    let before = win.manifest_steps();
    win.save()
        .unwrap_or_else(|e| panic!("the Save records: {e}"));
    let after = win.manifest_steps();
    let new: Vec<&String> = after.iter().filter(|s| !before.contains(s)).collect();
    let made: Vec<&String> = new
        .into_iter()
        .filter(|s| **s == statement || *s == "h")
        .collect();
    assert_eq!(made, [&statement, &"h".to_string()]);
}

#[test]
fn remove_step_takes_a_kept_draft_back() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    let before = win.snapshot();
    let on_table = hide(&win.table());
    keep(&mut win, "h", &on_table);
    win.select_made_by("h");
    win.d_d();
    assert!(win.kept().is_empty(), "{}", win.banners());
    assert!(!win.app.has_unsaved_work());
    assert!(!win.spine_labels().iter().any(|l| l == "Step h"));
    assert_eq!(win.snapshot(), before);
}

#[test]
fn a_kept_draft_another_reads_is_not_taken_back() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    let on_table = hide(&win.table());
    keep(&mut win, "h", &on_table);
    keep(&mut win, "r", &rename("h"));
    win.select_made_by("h");
    win.d_d();
    let names: Vec<String> = win.kept().into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["h", "r"]);
    let said = win.banners();
    assert!(said.contains("r reads what h makes"), "{said}");

    // The reader first, then the step.
    win.select_made_by("r");
    win.d_d();
    win.select_made_by("h");
    win.d_d();
    assert!(win.kept().is_empty(), "{}", win.banners());
}

#[test]
fn a_draft_arc_refuses_at_save_stays_kept_in_arcs_words() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    let table = win.table();
    // A first Save writes the Protocol's folder, as an agent's recording needs.
    keep(&mut win, "first", &rename(&table));
    win.save()
        .unwrap_or_else(|e| panic!("the first Save records: {e}"));
    keep(&mut win, "h", &hide(&table));
    // An agent records a step of the same name meanwhile.
    operation::record_step(&runner(), win.dir(), "h", &rename(&table))
        .expect("arc records the agent's step");

    let err = win.save().expect_err("arc refuses a second step named h");
    assert!(err.contains('h'), "{err}");
    assert!(!err.starts_with("error: "), "{err}");
    let names: Vec<String> = win.kept().into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["h"]);
    assert!(win.app.has_unsaved_work());
    assert!(
        win.banners().contains("Could not record the step:"),
        "the banner names a step, not SQL: {}",
        win.banners()
    );
}

#[test]
fn the_editor_tab_shows_the_step_in_fronts_sql_read_only() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    let on_table = hide(&win.table());
    let model = keep(&mut win, "h", &on_table);
    let drawn = win.editor();
    assert_eq!(drawn.text, model);
    assert_eq!(drawn.title, "h");
    assert!(drawn.read_only);
    assert!(
        win.texts.iter().any(|t| t.contains("read-only")),
        "the surface says it is read-only: {:?}",
        win.texts
    );

    win.save()
        .unwrap_or_else(|e| panic!("the Save records: {e}"));
    win.show_grid_of("h");
    let drawn = win.editor();
    assert_eq!(drawn.text, win.model_of("h"));
    assert_eq!(drawn.text, model);
    assert!(drawn.read_only);
}

#[test]
fn the_statement_form_over_a_drafts_grid_starts_empty() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    let on_table = hide(&win.table());
    keep(&mut win, "h", &on_table);
    win.key(egui::Key::Space);
    assert_eq!(win.app.open_overlay(), Some("palette"));
    assert_eq!(
        win.app.typed_statement(),
        None,
        "the palette holds no statement: arc's model is not typed over"
    );
    // A statement typed there and kept is a step of its own, and the draft stays.
    let table = win.table();
    win.type_text(&format!(
        "select house_age, count(*) as n from {table} group by house_age"
    ));
    assert!(win
        .app
        .finish_typed_reads(std::time::Duration::from_secs(60)));
    win.key_with(egui::Key::Enter, egui::Modifiers::COMMAND);
    let kept = win.kept();
    assert_eq!(kept.len(), 2, "{}", win.banners());
    assert_eq!(kept[0].name, "h");
    assert!(kept[0].operation.is_some(), "h is still the operation's");
    assert!(kept[1].operation.is_none());
}
