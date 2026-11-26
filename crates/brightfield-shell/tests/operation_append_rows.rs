//! `append-rows` in the window: columns matched by name. `shift-a` on a cell,
//! or "Append rows of other tables" on a header's menu, asks the spine for
//! the tables whose rows follow, `Space` adding each; the Operation section
//! then shows how their columns match, and Enter keeps the step.
//!
//! Each window is over a copy of the housing sample in a folder of the test's
//! own, with steps recorded through arc before it opens.

#[path = "support/operations.rs"]
mod operations;

use std::path::Path;
use std::time::{Duration, Instant};

use brightfield_shell::data_file;
use brightfield_shell::operation::{self, StepRequest};
use brightfield_shell::operation_ui::append_rows::{HELD, LACKED};
use brightfield_shell::protocol::SpineRole;
use brightfield_shell::spine_pick::SHARES_NO_COLUMN;
use operations::{runner, OpWindow};

const SAMPLE: &str = "california_housing_sample.csv";
const ROOT: &str = "california_housing_sample";

fn record(dir: &Path, name: &str, long_name: &str, args: &[(&str, &str)]) {
    let request = StepRequest {
        long_name: long_name.to_string(),
        on: Some(ROOT.to_string()),
        reads: Vec::new(),
        args: args
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
    };
    operation::record_step(&runner(), dir, name, &request)
        .unwrap_or_else(|e| panic!("arc records {name}: {e}"));
}

/// A window on the sample, saved so its load is written, with the steps
/// `steps` records on the loaded table.
fn window(steps: impl FnOnce(&Path)) -> OpWindow {
    OpWindow::on_copy_prepared(SAMPLE, |dir| {
        let opened =
            data_file::open(&dir.join(SAMPLE).to_string_lossy()).expect("the sample opens");
        opened.protocol.save_to(dir).expect("the Protocol saves");
        steps(dir);
    })
}

/// `top`, ten rows of the sample, and `without_age`, the sample without
/// `house_age`.
fn top_and_without_age(dir: &Path) {
    record(dir, "top", "limit-rows", &[("count", "10")]);
    record(
        dir,
        "without_age",
        "hide-column",
        &[("column", "house_age")],
    );
}

/// Press `shift-a` with the cursor on `house_age` until arc's description has
/// come, then draw frames until the pick knows each table's columns or has
/// closed saying why.
fn start(win: &mut OpWindow) {
    win.cursor_on("house_age");
    for _ in 0..600 {
        win.press("shift-a");
        let pending = win
            .app
            .operation_said()
            .is_some_and(|said| said.starts_with("Reading arc's description"));
        if !pending {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
        win.run(Vec::new());
    }
    wait_for_columns(win);
}

/// Draw frames until the open pick, if any, knows each candidate's columns.
fn wait_for_columns(win: &mut OpWindow) {
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
}

/// The table under the pick's cursor.
fn cursor(win: &OpWindow) -> String {
    win.app
        .spine_pick()
        .and_then(|pick| pick.under_cursor())
        .map(|c| c.table.clone())
        .expect("the pick is open, with a candidate")
}

#[test]
fn append_rows_takes_the_tables_added_and_keeps_their_rows_matched_by_name() {
    let mut win = window(top_and_without_age);
    let before = win.snapshot();
    start(&mut win);
    let pick = win.app.spine_pick().unwrap_or_else(|| {
        panic!("shift-a opened no pick: {:?}", win.said());
    });
    assert!(pick.list(), "append-rows reads a list of tables");
    assert_eq!(cursor(&win), "top");
    win.key(egui::Key::Space);
    win.letter(egui::Key::J, "j");
    assert_eq!(cursor(&win), "without_age");
    win.key(egui::Key::Space);
    win.key(egui::Key::Enter);
    assert!(win.app.spine_pick().is_none(), "{:?}", win.said());
    let draft = win
        .making()
        .unwrap_or_else(|| panic!("no draft: {:?}", win.said()));
    assert_eq!(
        draft.reads,
        [
            ("other".to_string(), "top".to_string()),
            ("other".to_string(), "without_age".to_string())
        ]
    );
    win.settle_reads();
    let read = win
        .app
        .making()
        .and_then(|making| making.answer.clone())
        .unwrap_or_else(|| panic!("the draft is answered: {:?}", win.said()));
    assert_eq!((read.rows_in, read.rows), (240, 490), "{:?}", win.said());
    // `without_age` lacks `house_age`, so its rows hold it null.
    assert_eq!(
        read.missing
            .iter()
            .find(|(column, _)| column == "house_age")
            .map(|(_, n)| *n),
        Some(240)
    );

    // The form shows a column per table and a row per name, each cell held or null.
    win.settle();
    for words in ["top", "without_age", "house_age", HELD, LACKED] {
        assert!(
            win.texts.iter().any(|t| t == words),
            "the form lacks {words}: {:?}",
            win.texts
        );
    }
    assert!(
        win.texts
            .iter()
            .any(|t| t.contains("then of top, then of without_age")),
        "{:?}",
        win.texts
    );

    win.key(egui::Key::Enter);
    let name = format!("{ROOT}_appended");
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?} {}", win.said(), win.banners());
    assert_eq!(kept[0].name, name);
    assert_eq!(win.snapshot(), before, "keeping writes no file");

    win.save()
        .unwrap_or_else(|e| panic!("the save records the step: {e}"));
    assert!(win.manifest_steps().contains(&name));
    let model = win.model_of(&name);
    assert_eq!(model.matches("UNION ALL BY NAME").count(), 2, "{model}");
    let (top, without) = (
        model.find("top").expect("the model reads top"),
        model
            .find("without_age")
            .expect("the model reads without_age"),
    );
    assert!(top < without, "top's rows come first: {model}");
}

#[test]
fn append_rows_starts_from_the_header_menu_and_dims_a_table_sharing_no_column() {
    let mut win = window(|dir| {
        record(dir, "top", "limit-rows", &[("count", "10")]);
        record(
            dir,
            "counted",
            "summarise-rows",
            &[("aggregates", "count(*) AS n")],
        );
    });
    win.cursor_on("house_age");
    // The first ask may find arc's description still on its way.
    for _ in 0..600 {
        win.header_menu("house_age", "Append rows of other tables");
        if win.app.spine_pick().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
        win.run(Vec::new());
    }
    wait_for_columns(&mut win);
    assert!(win.app.spine_pick().is_some(), "{:?}", win.said());
    let row = win
        .rows()
        .into_iter()
        .find(|row| row.role == SpineRole::Asset && row.label == "counted")
        .expect("the spine draws counted");
    assert_eq!(row.kind, SHARES_NO_COLUMN);
    assert!(row.muted, "a table sharing no column is dimmed");
    assert_eq!(cursor(&win), "top", "the cursor starts past it");

    // Neither added nor taken.
    win.letter(egui::Key::J, "j");
    assert_eq!(cursor(&win), "counted");
    win.key(egui::Key::Space);
    assert!(win.app.spine_pick().is_some_and(|p| p.added().is_empty()));
    let said = win.said().unwrap_or_default();
    assert!(said.starts_with("counted shares no column with"), "{said}");
    win.key(egui::Key::Enter);
    assert!(win.making().is_none());
    assert!(win.app.spine_pick().is_some(), "the pick stays open");

    // A click reaches it too, though it is drawn faded: the cursor goes onto
    // it, which is where the window says why it cannot be taken.
    win.letter(egui::Key::K, "k");
    assert_eq!(cursor(&win), "top");
    let at = win
        .rows()
        .into_iter()
        .find(|row| row.role == SpineRole::Asset && row.label == "counted")
        .expect("the spine still draws counted")
        .rect
        .center();
    win.click(at);
    assert_eq!(
        cursor(&win),
        "counted",
        "a click on a dimmed table puts the pick's cursor on it"
    );
}

#[test]
fn append_rows_needs_a_table_sharing_a_column() {
    let mut win = window(|dir| {
        record(
            dir,
            "counted",
            "summarise-rows",
            &[("aggregates", "count(*) AS n")],
        );
    });
    start(&mut win);
    assert!(win.app.spine_pick().is_none(), "the pick closed");
    assert!(win.making().is_none());
    assert_eq!(
        win.said().as_deref(),
        Some("Append rows needs a second table with matching columns.")
    );
}
