//! Whether an operation can start, read from its verb's needs in the key
//! registry: what each operation declares against what arc says it reads, the
//! facts the window adds for an operation, and one sentence wherever the
//! operation is asked for, whether by its key, a palette row, a menu bar item
//! or a row of a header's menu.

#[path = "support/operations.rs"]
mod operations;

use std::path::PathBuf;

use brightfield_keys::{Condition, Need, Selected, Situation};
use brightfield_shell::operation;
use brightfield_shell::operation_ui::header::Gesture;
use brightfield_shell::operation_ui::open::{self, InFront, OperationContext};
use brightfield_shell::operation_ui::{self, HeaderPart, Keeps, Start};
use operations::OpWindow;

const WITH_NULLS: &str = "with_nulls.csv";

/// What fill-nulls says on a column with no missing values.
const NO_NULLS: &str = "Fill missing values needs a column with missing values.";

#[test]
fn each_operations_declared_needs_match_what_arc_says_it_reads() {
    let runner = operations::runner();
    for ui in operation_ui::built() {
        let description = operation::describe(&runner, ui.long_name)
            .unwrap_or_else(|e| panic!("{} describes: {e}", ui.long_name));
        let needs = open::needs(ui);
        let has = |need: Need| needs.iter().any(|r| r.need == need);
        // An operation that asks in the foot row applies a view, which a
        // window with no Protocol of its own shows too. Any other is recorded,
        // and needs a Protocol to record into.
        let in_the_foot_row = ui.starts.iter().any(|s| matches!(s, Start::FootField(_)));
        assert!(
            in_the_foot_row
                || has(Need::Condition(Condition::ProtocolSource))
                || has(Need::Condition(Condition::TakesSources)),
            "{} records into a Protocol",
            ui.long_name
        );
        let role = |name: &str| description.reads.iter().find(|r| r.name == name);
        let table = role("table").is_some_and(|r| r.required);
        let other = role("other").filter(|r| r.required);
        // Filter and sort ask at the cursor's cell, which is a table's rows.
        assert_eq!(
            has(Need::Selected(Selected::GridRows)) || has(Need::Selected(Selected::GridCell)),
            table,
            "{} and a table's rows",
            ui.long_name
        );
        // A recorded operation reads the table as recorded, not the rows a
        // view leaves on screen.
        assert_eq!(
            has(Need::Condition(Condition::ViewRecorded)),
            table && !in_the_foot_row,
            "{} and an unrecorded view",
            ui.long_name
        );
        assert_eq!(
            has(Need::Condition(Condition::AnotherTable)),
            other.is_some_and(|r| !r.list),
            "{} and a second table",
            ui.long_name
        );
        assert_eq!(
            has(Need::Condition(Condition::AnotherTableSharingAColumn)),
            other.is_some_and(|r| r.list),
            "{} and tables to append",
            ui.long_name
        );
        // A Data-menu item ends in "…" exactly when its operation asks before
        // it keeps a step.
        if brightfield_keys::operations::is_operation_verb(ui.verb) {
            assert_eq!(
                brightfield_keys::operations::operation_asks(ui.verb),
                ui.keeps == Keeps::OnEnter,
                "{} on the Data menu",
                ui.long_name
            );
        }
        // The ribbon's and the menus' stage is the operation's own.
        if let Some(stage) = brightfield_keys::operations::operation_stage(ui.verb) {
            assert_eq!(stage, ui.stage, "{}'s stage", ui.long_name);
        }
    }
}

/// A context in which every fact an operation asks after holds, with a second
/// table sharing a column and a column with one missing value.
fn full_context() -> OperationContext {
    let facts = OpWindow::on_copy_of(WITH_NULLS)
        .app
        .protocol_model()
        .columns()
        .iter()
        .filter(|c| c.column == "house_age")
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(facts[0].nulls, 1, "the fixture's house_age has one missing");
    OperationContext {
        runner: true,
        protocol: Some(PathBuf::from("protocols/p")),
        takes_sources: true,
        in_front: Some(InFront {
            table: "housing".to_string(),
            loaded: true,
            rows: true,
            cell: true,
            view_unrecorded: false,
            columns: vec!["house_age".to_string(), "rooms".to_string()],
            facts,
            cursor_column: Some("house_age".to_string()),
            step_offer: None,
        }),
        tables: vec![
            ("housing".to_string(), Some(vec!["house_age".to_string()])),
            ("by_age".to_string(), Some(vec!["house_age".to_string()])),
        ],
    }
}

fn front(cx: &mut OperationContext) -> &mut InFront {
    cx.in_front.as_mut().expect("a table")
}

/// The situation the window reports for `cx`, on the cursor's column.
fn held(cx: &OperationContext) -> Situation {
    let mut situation = Situation::default();
    open::hold(&mut situation, cx, None);
    situation
}

#[test]
fn availability_names_the_first_need_it_lacks() {
    let every = [
        Need::Condition(Condition::ProtocolSource),
        Need::Selected(Selected::GridRows),
        Need::Selected(Selected::GridColumn),
        Need::Condition(Condition::ColumnHasNulls),
        Need::Condition(Condition::ViewRecorded),
        Need::Condition(Condition::LoadedTable),
        Need::Condition(Condition::AnotherTable),
        Need::Condition(Condition::AnotherTableSharingAColumn),
    ];
    let met = full_context();
    for need in every {
        assert!(held(&met).meets(need), "{need:?} is met");
    }
    // Each fact the window gathers is the one need it says.
    let unmet = |change: &dyn Fn(&mut OperationContext), lost: Need| {
        let mut cx = met.clone();
        change(&mut cx);
        let situation = held(&cx);
        for need in every {
            assert_eq!(
                situation.meets(need),
                need != lost,
                "{lost:?} lost: {need:?}"
            );
        }
    };
    unmet(
        &|cx| cx.protocol = None,
        Need::Condition(Condition::ProtocolSource),
    );
    unmet(
        &|cx| front(cx).facts.clear(),
        Need::Condition(Condition::ColumnHasNulls),
    );
    unmet(
        &|cx| front(cx).view_unrecorded = true,
        Need::Condition(Condition::ViewRecorded),
    );
    unmet(
        &|cx| front(cx).loaded = false,
        Need::Condition(Condition::LoadedTable),
    );
    unmet(
        &|cx| cx.tables[1].1 = Some(vec!["other".to_string()]),
        Need::Condition(Condition::AnotherTableSharingAColumn),
    );
    // A table whose columns are not read yet is not ruled out.
    let mut cx = met.clone();
    cx.tables[1].1 = None;
    assert!(held(&cx).meets(Need::Condition(Condition::AnotherTableSharingAColumn)));
    // No rows drawn, no column, no second table: each loses what reads it.
    let mut cx = met.clone();
    front(&mut cx).rows = false;
    assert!(!held(&cx).meets(Need::Selected(Selected::GridRows)));
    let mut cx = met.clone();
    front(&mut cx).cursor_column = None;
    let situation = held(&cx);
    assert!(!situation.meets(Need::Selected(Selected::GridColumn)));
    assert!(!situation.meets(Need::Condition(Condition::ColumnHasNulls)));
    // A header's column stands in for the cursor's.
    let mut on_header = Situation::default();
    open::hold(&mut on_header, &cx, Some("house_age"));
    assert!(on_header.meets(Need::Condition(Condition::ColumnHasNulls)));
    let mut cx = met.clone();
    cx.tables.truncate(1);
    let situation = held(&cx);
    assert!(!situation.meets(Need::Condition(Condition::AnotherTable)));
    assert!(!situation.meets(Need::Condition(Condition::AnotherTableSharingAColumn)));

    // Read against an operation's verb, the first unmet need says its sentence.
    let fill = operation_ui::spec("fill-nulls").expect("fill-nulls is built");
    let mut cx = met.clone();
    cx.protocol = None;
    front(&mut cx).facts.clear();
    assert_eq!(
        open::availability(fill, &held(&cx)).missing(),
        Some("Fill missing values needs a Protocol of your own: open a data file or a folder.")
    );
    cx.protocol = met.protocol.clone();
    assert_eq!(
        open::availability(fill, &held(&cx)).missing(),
        Some(NO_NULLS)
    );

    // A step's preview has its own words for a table that is not loaded.
    let filter = operation_ui::spec("filter-rows").expect("filter-rows is built");
    let mut cx = met.clone();
    front(&mut cx).loaded = false;
    front(&mut cx).step_offer = Some("the step has not been run".to_string());
    let mut situation = held(&cx);
    situation.selected.push(Selected::GridCell);
    assert_eq!(
        open::availability(filter, &situation).missing(),
        Some("the step has not been run")
    );
}

/// The window over a copy of the fixture with the grid's cursor on `column`.
fn on(column: &str) -> OpWindow {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    win.cursor_on(column);
    win
}

/// The open palette's sentence for `verb`, opened and closed again.
fn palette_says(win: &mut OpWindow, verb: &str) -> Option<String> {
    win.key(egui::Key::Space);
    assert_eq!(win.app.open_overlay(), Some("palette"), "space opened it");
    win.settle();
    let said = win
        .app
        .open_palette_missing()
        .into_iter()
        .find(|(longname, _)| *longname == verb)
        .map(|(_, sentence)| sentence);
    assert!(
        win.app.open_palette_rows().iter().any(|row| row == verb),
        "the palette lists {verb}, greyed or not"
    );
    win.key(egui::Key::Escape);
    win.settle();
    said
}

#[test]
fn an_operation_unavailable_here_is_greyed_in_the_palette_with_its_sentence() {
    // `city` has no missing values, so filling it is greyed and says why;
    // hiding it is not.
    let mut win = on("city");
    assert_eq!(
        palette_says(&mut win, "fill-nulls").as_deref(),
        Some(NO_NULLS)
    );
    assert_eq!(palette_says(&mut win, "hide-column"), None);
    // On `house_age`, which has one, it can start.
    let mut win = on("house_age");
    assert_eq!(palette_says(&mut win, "fill-nulls"), None);
}

#[test]
fn its_key_its_menu_row_and_its_palette_row_say_the_same_sentence() {
    let mut win = on("city");
    let palette = palette_says(&mut win, "fill-nulls");
    let menu_bar = win
        .app
        .menu_rows()
        .into_iter()
        .find(|row| row.longname == "fill-nulls")
        .expect("fill-nulls is on the Data menu")
        .missing;
    win.header("city", HeaderPart::Menu, Gesture::Secondary);
    let header = win
        .app
        .chart_doc()
        .header_drawn()
        .menu
        .iter()
        .find(|row| row.key.as_deref() == Some("f"))
        .unwrap_or_else(|| panic!("the header's menu has fill-nulls' row"))
        .sentence
        .clone();
    win.key(egui::Key::Escape);
    win.cursor_on("city");
    win.press("f");
    assert!(win.making().is_none(), "{:?}", win.making());
    let key = win.said();
    for (place, said) in [
        ("palette", palette),
        ("menu bar", menu_bar),
        ("header menu", header),
        ("key", key),
    ] {
        assert_eq!(said.as_deref(), Some(NO_NULLS), "the {place}");
    }
}
