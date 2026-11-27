//! The spine picking a table for an operation: an operation that reads a
//! second table opens the pick in the navigator rail, which marks the table in
//! front, says what each other table has in common with it, and hands the
//! table taken, or the list of them, to the operation's draft.
//!
//! The two operations here are specs local to this file, started through the
//! window's own entry point, so the pick is tested apart from any one
//! operation's form.
//! They ask arc for `join-tables` and `append-rows`, so the roles, the preview
//! and the draft are arc's. The folder is a copy of the housing sample with two
//! steps recorded through arc: `by_age`, which counts the rows by `house_age`,
//! and `top`, which keeps ten rows.

#[path = "support/operations.rs"]
mod operations;

use std::cell::RefCell;
use std::path::Path;
use std::time::{Duration, Instant};

use brightfield_shell::data_file;
use brightfield_shell::operation::{self, StepRequest};
use brightfield_shell::operation_ui::{
    Begin, By, Draft, Keeps, NameRule, OperationUi, Stage, Start,
};
use brightfield_shell::protocol::{SpineRole, SpineRowDrawn, ADDED, IN_FRONT, OUTLINE};
use brightfield_workbench::PaneKey;
use operations::{runner, OpWindow};

const SAMPLE: &str = "california_housing_sample.csv";
const ROOT: &str = "california_housing_sample";

thread_local! {
    /// The tables the last `begin` on this thread was handed, each with its columns.
    static TAKEN: RefCell<Vec<(String, Vec<String>)>> = const { RefCell::new(Vec::new()) };
    /// Whether the last `begin` on this thread was handed the Protocol's model.
    static HAD_MODEL: RefCell<bool> = const { RefCell::new(false) };
}

/// A draft on the table in front that reads each table the spine took.
fn reading_taken(long_name: &'static str, b: &Begin<'_>) -> Draft {
    TAKEN.with(|taken| *taken.borrow_mut() = b.taken.to_vec());
    HAD_MODEL.with(|had| *had.borrow_mut() = b.model.is_some());
    let mut draft = Draft::empty(long_name);
    draft.on = b.table.map(str::to_string);
    draft.reads = b
        .taken
        .iter()
        .map(|(table, _)| ("other".to_string(), table.clone()))
        .collect();
    draft
}

fn begin_join(b: &Begin<'_>) -> Result<Draft, String> {
    let mut draft = reading_taken("join-tables", b);
    draft.set("using", b.column.unwrap_or_default().to_string());
    Ok(draft)
}

fn begin_append(b: &Begin<'_>) -> Result<Draft, String> {
    Ok(reading_taken("append-rows", b))
}

/// An operation that reads one other table, as joining two tables does.
static JOIN_LIKE: OperationUi = OperationUi {
    long_name: "join-tables",
    verb: "join-tables",
    stage: Stage::Model,
    icon: "arrows-join",
    starts: &[Start::Spine { role: "other" }],
    menu_words: "Join tables",
    begin: begin_join,
    keeps: Keeps::OnEnter,
    field: None,
    form: None,
    name: NameRule {
        word: "joined",
        from: By::Nothing,
        with_by: false,
    },
};

/// An operation that reads a list of other tables, as appending rows does.
static APPEND_LIKE: OperationUi = OperationUi {
    long_name: "append-rows",
    verb: "append-rows",
    stage: Stage::Model,
    icon: "row-insert-bottom",
    starts: &[Start::Spine { role: "other" }],
    menu_words: "Append rows",
    begin: begin_append,
    keeps: Keeps::OnEnter,
    field: None,
    form: None,
    name: NameRule {
        word: "appended",
        from: By::Nothing,
        with_by: false,
    },
};

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

/// Save the sample's Protocol into `dir`, so its load is written, and record
/// `by_age` and `top` on the loaded table.
fn two_steps(dir: &Path) {
    let opened = data_file::open(&dir.join(SAMPLE).to_string_lossy()).expect("the sample opens");
    opened.protocol.save_to(dir).expect("the Protocol saves");
    record(
        dir,
        "by_age",
        "summarise-rows",
        &[("aggregates", "count(*) AS n"), ("group_by", "house_age")],
    );
    record(dir, "top", "limit-rows", &[("count", "10")]);
}

/// A window on the sample with `by_age` and `top` recorded.
fn window() -> OpWindow {
    OpWindow::on_copy_prepared(SAMPLE, two_steps)
}

/// Start `spec` from the spine with the grid's cursor on `column` of the
/// loaded table, waiting for arc's description when the first ask finds it
/// still on its way, and then for each candidate's columns.
fn start(win: &mut OpWindow, spec: &'static OperationUi, column: &str) -> bool {
    win.cursor_on(column);
    let ctx = win.ctx.clone();
    let from = Start::Spine { role: "other" };
    for _ in 0..600 {
        if win.app.start_operation_spec(&ctx, spec, from, Some(column)) {
            win.settle();
            wait_for_columns(win);
            return true;
        }
        let pending = win
            .app
            .operation_said()
            .is_some_and(|said| said.starts_with("Reading arc's description"));
        if !pending {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
        win.run(Vec::new());
    }
    panic!("arc never described {}", spec.long_name);
}

/// Draw frames until the pick knows each candidate's columns.
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

/// The row the spine drew for the table `table`.
fn table_row(win: &OpWindow, table: &str) -> SpineRowDrawn {
    win.rows()
        .into_iter()
        .find(|row| row.role == SpineRole::Asset && row.label == table)
        .unwrap_or_else(|| panic!("no row for {table}: {:?}", win.spine_labels()))
}

/// The table under the pick's cursor.
fn cursor(win: &OpWindow) -> String {
    win.app
        .spine_pick()
        .and_then(|pick| pick.under_cursor())
        .map(|c| c.table.clone())
        .expect("the pick is open, with a candidate")
}

fn grid() -> PaneKey {
    PaneKey::new(brightfield_shell::data_grid::DATA)
}

#[test]
fn the_spine_marks_the_table_in_front_and_starts_on_the_best_candidate() {
    let mut win = window();
    // `median_income` is a column of `top`, which keeps the table's columns,
    // and not of `by_age`, which holds `house_age` and its count.
    assert!(
        start(&mut win, &JOIN_LIKE, "median_income"),
        "{:?}",
        win.said()
    );

    let pick = win.app.spine_pick().expect("the pick is open");
    let tables: Vec<&str> = pick.candidates().iter().map(|c| c.table.as_str()).collect();
    assert_eq!(
        tables,
        ["by_age", "top"],
        "every table but the one in front"
    );
    assert_eq!(pick.in_front(), ROOT);
    assert!(!pick.list(), "join-tables reads one other table");
    assert_eq!(
        cursor(&win),
        "top",
        "the first that shares the cursor's column"
    );
    // The draft waits on the take.
    assert!(win.making().is_none());

    // The rail holds the keys, and with them the window's focus ring.
    assert_eq!(win.app.focused_pane(), Some(PaneKey::new(OUTLINE)));
    // The table in front says so and is not the cursor's row.
    let front = table_row(&win, ROOT);
    assert_eq!(front.kind, IN_FRONT);
    assert!(!front.washed && !front.muted);
    // The cursor's table is the one selected row.
    assert!(table_row(&win, "top").washed);
    assert!(!table_row(&win, "by_age").washed);
    // Every row that is not a table is named in the muted ink and answers no
    // click: each step, each view, and the data file. No table is.
    let tables = [ROOT, "by_age", "top"];
    let mut muted = 0;
    for row in win.rows() {
        let table = row.role == SpineRole::Asset && tables.contains(&row.label.as_str());
        let listed = matches!(
            row.role,
            SpineRole::Asset | SpineRole::Step | SpineRole::View | SpineRole::Model
        );
        if table {
            assert!(!row.muted, "the table {} is muted", row.label);
        } else if listed {
            assert!(row.muted, "{:?} {} is not muted", row.role, row.label);
            assert!(!row.control, "{:?} {} answers a click", row.role, row.label);
            muted += 1;
        }
    }
    assert!(
        muted >= 5,
        "two steps, a file and the views: {:?}",
        win.spine_labels()
    );
    // The status band names the pick's five keys, as the registry binds them.
    let band = win
        .texts
        .iter()
        .find(|text| text.contains("pick a table in the spine"))
        .unwrap_or_else(|| panic!("the band says nothing of the pick: {:?}", win.texts));
    for keys in [
        "j/down next",
        "k/up previous",
        "space add",
        "enter take",
        "escape back",
    ] {
        assert!(band.contains(keys), "{band}");
    }
    assert!(band.starts_with("Join tables"), "{band}");
}

#[test]
fn each_table_says_whether_it_shares_the_cursors_column() {
    let mut win = window();
    assert!(start(&mut win, &JOIN_LIKE, "house_age"), "{:?}", win.said());
    assert_eq!(table_row(&win, "by_age").kind, "shares house_age");
    assert_eq!(table_row(&win, "top").kind, "shares house_age");
    assert_eq!(cursor(&win), "by_age", "the first of the two");

    // The cursor on `latitude`, which `top` holds and `by_age` lacks: the row of `top` says it
    // shares the column, and the row of `by_age` trails an empty string.
    let mut win = window();
    assert!(start(&mut win, &JOIN_LIKE, "latitude"), "{:?}", win.said());
    assert_eq!(table_row(&win, "by_age").kind, "");
    assert_eq!(table_row(&win, "top").kind, "shares latitude");
}

#[test]
fn a_step_that_makes_no_rows_still_says_what_it_shares() {
    // `none` keeps no row of the table, and holds each of its columns.
    let mut win = OpWindow::on_copy_prepared(SAMPLE, |dir| {
        two_steps(dir);
        record(dir, "none", "filter-rows", &[("where", "house_age > 1000")]);
    });
    assert!(start(&mut win, &JOIN_LIKE, "latitude"), "{:?}", win.said());
    assert_eq!(table_row(&win, "none").kind, "shares latitude");
    let columns = win
        .app
        .spine_pick()
        .and_then(|pick| pick.columns_of("none").map(<[String]>::to_vec))
        .expect("the columns of none are read");
    assert_eq!(columns.len(), 9, "{columns:?}");
    assert_eq!(columns[0], "median_income");
}

#[test]
fn a_table_the_model_relates_comes_first_and_says_so() {
    // A model in the Protocol's folder that joins the loaded table to `top`.
    let model = format!(
        "version: 0.2.0.dev0\nname: housing\ndatasets:\n{}{}relationships:\n- name: sample_top\n  \
         from: {ROOT}\n  to: top\n  from_columns:\n  - house_age\n  to_columns:\n  - house_age\n  \
         custom_extensions:\n  - vendor_name: MERIDIAN\n    data: \
         '{{\"grade\":\"likely\",\"evidence\":{{\"value_overlap\":1.0,\"orphans\":0,\
         \"child_rows\":8,\"child_type\":\"BIGINT\",\"parent_type\":\"BIGINT\",\
         \"types_agree\":true,\"name_similarity\":1.0}},\"cardinality\":\"N:1\",\
         \"verified\":\"full\",\"decided_by\":\"analyst\"}}'\n",
        dataset(ROOT),
        dataset("top"),
    );
    brightfield_ossie::read(&model).unwrap_or_else(|e| panic!("the model reads: {e:?}"));
    let mut win = OpWindow::on_copy_prepared(SAMPLE, |dir| {
        two_steps(dir);
        std::fs::write(dir.join("model.ossie.yaml"), &model).expect("the model writes");
    });
    // Both tables share `house_age`, and `by_age` is listed first: the
    // relationship is what puts the cursor on `top`.
    assert!(start(&mut win, &JOIN_LIKE, "house_age"), "{:?}", win.said());
    assert_eq!(cursor(&win), "top");
    assert_eq!(table_row(&win, "top").kind, "related");
    assert_eq!(table_row(&win, "by_age").kind, "shares house_age");

    // The operation's draft is begun with the model the relationship is in.
    win.key(egui::Key::Enter);
    assert!(win.making().is_some(), "{:?}", win.said());
    assert!(HAD_MODEL.with(|had| *had.borrow()));
}

/// One dataset of a model file, with the one column the tables here share.
fn dataset(name: &str) -> String {
    format!(
        "- name: {name}\n  source: {name}.csv\n  primary_key:\n  - house_age\n  fields:\n  - name: \
         house_age\n    expression:\n      sql: house_age\n    custom_extensions:\n    - \
         vendor_name: MERIDIAN\n      data: '{{\"decided_by\":\"dovetail\"}}'\n  \
         custom_extensions:\n  - vendor_name: MERIDIAN\n    data: \
         '{{\"role\":\"entity\",\"role_evidence\":[],\"excluded\":false,\
         \"decided_by\":\"dovetail\",\"source_file\":\"{name}.csv\",\"rows\":5}}'\n"
    )
}

#[test]
fn enter_takes_the_table_and_begins_the_operation_with_it() {
    let mut win = window();
    assert!(start(&mut win, &JOIN_LIKE, "house_age"), "{:?}", win.said());
    assert_eq!(cursor(&win), "by_age");
    win.key(egui::Key::Enter);

    // The pick is over, and the grid has the keys back.
    assert!(win.app.spine_pick().is_none());
    assert_eq!(win.app.focused_pane(), Some(grid()));
    assert!(!win
        .rows()
        .iter()
        .any(|row| row.muted || row.kind == IN_FRONT));
    // The operation was begun with the table and its columns.
    let taken = TAKEN.with(|taken| taken.borrow().clone());
    assert_eq!(
        taken,
        [(
            "by_age".to_string(),
            vec!["house_age".to_string(), "n".to_string()]
        )]
    );
    let draft = win
        .making()
        .unwrap_or_else(|| panic!("no draft: {:?}", win.said()));
    assert_eq!(draft.on.as_deref(), Some(ROOT));
    assert_eq!(draft.reads, [("other".to_string(), "by_age".to_string())]);
    // Its preview reads the table taken: the join carries `by_age`'s count,
    // under the table's name as arc's join spells it.
    win.settle_reads();
    let columns = win.grid_columns();
    assert!(
        columns.contains(&"by_age_n".to_string()),
        "{columns:?} {:?}",
        win.said()
    );
    assert!(
        columns.contains(&"median_income".to_string()),
        "{columns:?}"
    );
    // And Enter keeps it, as it keeps any operation.
    win.key(egui::Key::Enter);
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?} {}", win.said(), win.banners());
    assert_eq!(
        kept[0].operation.as_ref().map(|r| r.reads.clone()),
        Some(vec![("other".to_string(), "by_age".to_string())])
    );
}

#[test]
fn the_keys_move_the_cursor_and_a_click_puts_it_on_a_table() {
    let mut win = window();
    assert!(start(&mut win, &JOIN_LIKE, "house_age"), "{:?}", win.said());
    assert_eq!(cursor(&win), "by_age");
    win.letter(egui::Key::J, "j");
    assert_eq!(cursor(&win), "top");
    assert!(table_row(&win, "top").washed && !table_row(&win, "by_age").washed);
    // The cursor stops at the last table, and the key reaches no other pane.
    win.key(egui::Key::ArrowDown);
    assert_eq!(cursor(&win), "top");
    win.letter(egui::Key::K, "k");
    assert_eq!(cursor(&win), "by_age");
    win.key(egui::Key::ArrowUp);
    assert_eq!(cursor(&win), "by_age");
    assert!(win.app.spine_pick().is_some(), "a move closed the pick");
    assert!(!win.app.graph_on_canvas());

    // A click on a table the operation can read puts the cursor on it, and
    // takes nothing.
    let top = table_row(&win, "top");
    assert!(top.control);
    assert!(
        !table_row(&win, ROOT).control,
        "the table in front answers a click"
    );
    win.click(top.rect.center());
    assert_eq!(cursor(&win), "top");
    assert!(win.making().is_none());

    // Space adds no table to an operation that reads one, and says so.
    win.key(egui::Key::Space);
    assert!(win
        .app
        .spine_pick()
        .is_some_and(|pick| pick.added().is_empty()));
    assert_eq!(
        win.said().as_deref(),
        Some("Join tables reads one table: press Enter to take the one under the cursor.")
    );
    assert_eq!(win.app.open_overlay(), None, "space opened the palette");
}

#[test]
fn space_adds_to_a_list_and_enter_takes_them_in_order() {
    let mut win = window();
    assert!(
        start(&mut win, &APPEND_LIKE, "house_age"),
        "{:?}",
        win.said()
    );
    assert!(win.app.spine_pick().is_some_and(|pick| pick.list()));
    // `top` first, then `by_age`: the order added, not the spine's.
    win.letter(egui::Key::J, "j");
    win.key(egui::Key::Space);
    win.letter(egui::Key::K, "k");
    win.key(egui::Key::Space);
    let added = win.app.spine_pick().expect("still open").added().to_vec();
    assert_eq!(added, ["top", "by_age"]);
    assert_eq!(win.app.open_overlay(), None, "space opened the palette");
    // An added table is drawn selected and says it is added, which is what
    // tells it from the row under the cursor, selected too.
    let top = table_row(&win, "top");
    assert!(top.washed);
    assert_eq!(top.kind, ADDED);
    // Space on an added table takes it off the list, and adds it again at the end.
    win.key(egui::Key::Space);
    // Off the list and under the cursor, by_age is drawn selected and does not
    // say it is added: the word is what tells the two apart.
    let by_age = table_row(&win, "by_age");
    assert!(by_age.washed, "the cursor is on by_age");
    assert_ne!(by_age.kind, ADDED);
    assert_eq!(table_row(&win, "top").kind, ADDED);
    win.key(egui::Key::Space);
    assert_eq!(
        win.app.spine_pick().expect("still open").added(),
        ["top", "by_age"]
    );

    win.key(egui::Key::Enter);
    assert!(win.app.spine_pick().is_none());
    let draft = win
        .making()
        .unwrap_or_else(|| panic!("no draft: {:?}", win.said()));
    assert_eq!(
        draft.reads,
        [
            ("other".to_string(), "top".to_string()),
            ("other".to_string(), "by_age".to_string())
        ]
    );
    let taken: Vec<String> =
        TAKEN.with(|taken| taken.borrow().iter().map(|t| t.0.clone()).collect());
    assert_eq!(taken, ["top", "by_age"]);
}

#[test]
fn esc_goes_back_to_the_grid_and_keeps_nothing() {
    let mut win = window();
    let before = win.snapshot();
    assert!(
        start(&mut win, &APPEND_LIKE, "house_age"),
        "{:?}",
        win.said()
    );
    win.key(egui::Key::Space);
    assert_eq!(win.app.spine_pick().expect("open").added(), ["by_age"]);
    win.key(egui::Key::Escape);

    assert!(win.app.spine_pick().is_none());
    assert!(win.making().is_none());
    assert!(win.kept().is_empty());
    assert_eq!(win.app.focused_pane(), Some(grid()));
    assert!(!win.app.has_unsaved_work());
    let rows = win.rows();
    assert!(!rows.iter().any(|row| row.muted), "a row stayed muted");
    assert!(!rows
        .iter()
        .any(|row| row.kind == IN_FRONT || row.kind.starts_with(ADDED)));
    assert!(
        table_row(&win, "top").control,
        "the rows answer a click again"
    );
    assert_eq!(win.snapshot(), before, "the pick wrote a file");
    // The grid's keys are the grid's again.
    assert!(!win
        .texts
        .iter()
        .any(|t| t.contains("pick a table in the spine")));
}

#[test]
fn the_pick_closes_when_another_pane_takes_the_keys() {
    let mut win = window();
    assert!(start(&mut win, &JOIN_LIKE, "house_age"), "{:?}", win.said());
    // A press on the grid gives it the keys: the pick's keys were the rail's.
    win.focus_grid();
    assert!(win.app.spine_pick().is_none());
    assert!(win.making().is_none());
    assert_eq!(win.app.focused_pane(), Some(grid()));
    assert!(!win.rows().iter().any(|row| row.muted));
    // `j` is the grid's again, and moves its cursor a row.
    win.letter(egui::Key::J, "j");
    assert_eq!(win.app.chart_doc().grid_cursor.map(|at| at.row), Some(1));
}

#[test]
fn the_pick_closes_when_its_rail_is_put_away() {
    use brightfield_workbench::arrangement::NAVIGATOR_RAIL;
    let mut win = window();
    assert!(start(&mut win, &JOIN_LIKE, "house_age"), "{:?}", win.said());
    let control = win
        .app
        .rail_collapse_rect(NAVIGATOR_RAIL)
        .expect("the navigator rail drew its collapse control");
    win.click(control.center());
    assert!(win.app.rail_is_collapsed(NAVIGATOR_RAIL));
    // The spine is not drawn, so the pick is over, and Enter takes no table unseen.
    assert!(win.app.spine_pick().is_none());
    win.key(egui::Key::Enter);
    assert!(win.making().is_none());
    // The rail stays as the click left it.
    assert!(win.app.rail_is_collapsed(NAVIGATOR_RAIL));
}

#[test]
fn a_protocol_with_no_other_table_says_so_and_opens_no_pick() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    assert!(!start(&mut win, &JOIN_LIKE, "house_age"));
    assert_eq!(
        win.said().as_deref(),
        Some("Join tables needs a second table in the Protocol.")
    );
    assert!(win.app.spine_pick().is_none());
    assert_eq!(win.app.focused_pane(), Some(grid()));
}

/// Each key the registry lists for the spine's pick reaches the pick's
/// handler: the window records the command the key performed.
#[test]
fn every_spine_pick_key_is_answered() {
    use brightfield_keys::{registry, BindingContext};

    let mut swept = 0;
    for verb in registry().iter().filter(|v| !v.is_reserved()) {
        for binding in verb
            .binding_specs
            .iter()
            .filter(|b| b.context == BindingContext::SpinePick)
        {
            swept += 1;
            let key = match binding.keystrokes {
                "j" => egui::Key::J,
                "k" => egui::Key::K,
                "down" => egui::Key::ArrowDown,
                "up" => egui::Key::ArrowUp,
                "space" => egui::Key::Space,
                "enter" => egui::Key::Enter,
                "escape" => egui::Key::Escape,
                other => panic!("no key here for the pick's `{other}`"),
            };
            let mut win = window();
            assert!(start(&mut win, &JOIN_LIKE, "house_age"), "{:?}", win.said());
            win.run(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            assert!(
                win.answered.contains(&verb.longname),
                "{} is listed for {} in the spine's pick and no handler took it: {:?}",
                binding.keystrokes,
                verb.longname,
                win.answered
            );
        }
    }
    assert!(
        swept > 0,
        "no built command binds a key in the spine's pick"
    );
}
