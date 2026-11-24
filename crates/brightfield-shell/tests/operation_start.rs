//! Starting an operation, making it and keeping it: from a key or the
//! palette, with a sentence when it cannot start, a preview as it is made,
//! Enter to keep and Esc to drop.
//!
//! No operation other than the filter and the sort is built yet, so the
//! operations here are specs local to this file, started through the
//! window's own entry point. They ask arc for real operations, so each
//! preview and each kept step is arc's.

#[path = "support/operations.rs"]
mod operations;

use std::path::PathBuf;

use brightfield_shell::grid_foot::FootField;
use brightfield_shell::operation_ui::open::{
    availability, sentence, Availability, InFront, OperationContext,
};
use brightfield_shell::operation_ui::{
    Begin, By, Draft, HeaderPart, InPlace, Keeps, NameRule, Need, OperationUi, Stage, Start,
};
use brightfield_shell::overlays::chart_palette_verbs;
use operations::OpWindow;

const SAMPLE: &str = "california_housing_sample.csv";
const WITH_NULLS: &str = "with_nulls.csv";

fn on_column(long_name: &'static str, b: &Begin<'_>) -> Result<Draft, String> {
    let mut draft = Draft::empty(long_name);
    draft.on = b.table.map(str::to_string);
    draft.set("column", b.column.unwrap_or_default().to_string());
    Ok(draft)
}

fn begin_hide(b: &Begin<'_>) -> Result<Draft, String> {
    on_column("hide-column", b)
}

fn begin_rename(b: &Begin<'_>) -> Result<Draft, String> {
    let mut draft = on_column("rename-column", b)?;
    draft.set("to", String::new());
    draft.focus = Some("to");
    Ok(draft)
}

/// A new column `copy` whose expression is the column it started on, as SQL
/// spells it.
fn begin_derive(b: &Begin<'_>) -> Result<Draft, String> {
    let mut draft = Draft::empty("derive-column");
    draft.on = b.table.map(str::to_string);
    draft.set("column", "copy".to_string());
    draft.set("expression", (b.quote)(b.column.unwrap_or_default()));
    Ok(draft)
}

fn no_suggestions(_: &Begin<'_>, _: &Draft) -> Vec<String> {
    Vec::new()
}

const NEEDS: &[Need] = &[
    Need::ProtocolSource,
    Need::Rows,
    Need::Column,
    Need::ViewRecorded,
];

/// An operation that asks for no value and is kept at once, as hiding a column is.
static HIDE_LIKE: OperationUi = OperationUi {
    long_name: "hide-column",
    verb: "hide-column",
    stage: Stage::Shape,
    icon: "eye-off",
    starts: &[Start::Cell],
    menu_words: "Hide column",
    needs: NEEDS,
    begin: begin_hide,
    keeps: Keeps::AtOnce,
    field: None,
    form: None,
    name: NameRule {
        word: "without",
        from: By::Value("column"),
        with_by: false,
    },
};

/// An operation typed in place and kept on Enter, as renaming a column is.
static RENAME_LIKE: OperationUi = OperationUi {
    long_name: "rename-column",
    verb: "rename-column",
    stage: Stage::Shape,
    icon: "pencil",
    starts: &[Start::Cell],
    menu_words: "Rename column",
    needs: NEEDS,
    begin: begin_rename,
    keeps: Keeps::OnEnter,
    field: Some(InPlace {
        part: HeaderPart::Name,
        keys: &["to"],
        suggestions: no_suggestions,
    }),
    form: None,
    name: NameRule {
        word: "renamed",
        from: By::Value("column"),
        with_by: false,
    },
};

/// An operation whose SQL value takes the column it started on.
static DERIVE_LIKE: OperationUi = OperationUi {
    long_name: "derive-column",
    verb: "derive-column",
    stage: Stage::Shape,
    icon: "plus",
    starts: &[Start::Cell],
    menu_words: "Derive column",
    needs: NEEDS,
    begin: begin_derive,
    keeps: Keeps::OnEnter,
    field: Some(InPlace {
        part: HeaderPart::NewColumn,
        keys: &["expression"],
        suggestions: no_suggestions,
    }),
    form: None,
    name: NameRule {
        word: "with",
        from: By::Value("column"),
        with_by: false,
    },
};

/// Start `spec` on `column` of the loaded table, and wait for arc's
/// description when the first ask finds it still on its way.
fn start(win: &mut OpWindow, spec: &'static OperationUi, column: &str) -> bool {
    win.cursor_on(column);
    let ctx = win.ctx.clone();
    for _ in 0..600 {
        if win
            .app
            .start_operation_spec(&ctx, spec, Start::Cell, Some(column))
        {
            win.settle();
            return true;
        }
        let pending = win
            .app
            .operation_said()
            .is_some_and(|said| said.starts_with("Reading arc's description"));
        if !pending {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        win.run(Vec::new());
    }
    panic!("arc never described {}", spec.long_name);
}

/// The table in front of `cx`.
fn front(cx: &mut OperationContext) -> &mut InFront {
    cx.in_front.as_mut().expect("a table")
}

/// A context that meets each need of [`NEEDS`], with a second table.
fn full_context() -> OperationContext {
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
            facts: Vec::new(),
            cursor_column: Some("house_age".to_string()),
            step_offer: None,
        }),
        tables: vec![
            ("housing".to_string(), Some(vec!["house_age".to_string()])),
            ("by_age".to_string(), Some(vec!["house_age".to_string()])),
        ],
    }
}

#[test]
fn the_palette_opens_filter_rows_in_the_where_field() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    win.cursor_on("house_age");
    win.palette("filter-rows-at-cursor");
    let asking = win.app.chart_doc().foot_asking().map(|(field, _)| field);
    assert_eq!(asking, Some(FootField::Where), "{:?}", win.said());
}

#[test]
fn the_palette_opens_sort_rows_in_the_order_by_field() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    win.cursor_on("house_age");
    win.palette("sort-rows-at-cursor");
    let asking = win
        .app
        .chart_doc()
        .foot_asking()
        .map(|(field, text)| (field, text.to_string()));
    assert_eq!(asking, Some((FootField::OrderBy, "house_age".to_string())));
}

#[test]
fn a_key_off_a_cell_says_why_instead_of_nothing() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    win.cursor_on("house_age");
    win.app
        .set_grid_layout(brightfield_shell::app::GridLayout::Columns);
    win.settle();
    win.press("w");
    assert_eq!(win.app.chart_doc().foot_asking(), None);
    assert_eq!(
        win.app.operation_said(),
        Some("Filter rows needs the grid's cursor on a cell.")
    );
    // The sentence is on the status band, and goes at the next press.
    assert!(
        win.texts
            .iter()
            .any(|t| t.contains("needs the grid's cursor")),
        "{:?}",
        win.texts
    );
    win.key(egui::Key::ArrowDown);
    assert_eq!(win.app.operation_said(), None);
}

#[test]
fn a_window_with_no_protocol_still_opens_the_where_field() {
    let mut win = OpWindow::on_live_chart("scatter.yaml");
    assert!(!win.app.has_protocol_to_save());
    // The grid in its other spot, where this layout draws it.
    if win.app.chart_doc().grid_drawn().is_none() {
        win.palette("move-grid");
    }
    win.focus_grid();
    win.app.chart_doc_mut().grid_cursor =
        Some(brightfield_shell::data_grid::GridCursor { row: 0, col: 0 });
    win.settle();
    win.press("w");
    let asking = win.app.chart_doc().foot_asking().map(|(field, _)| field);
    assert_eq!(
        asking,
        Some(FootField::Where),
        "{:?}",
        win.app.operation_said()
    );
    assert_eq!(win.app.operation_said(), None);
}

#[test]
fn filter_rows_on_a_steps_grid_says_the_steps_offer() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some("house_age > 28".to_string()),
            order: None,
        });
    win.settle();
    win.save()
        .unwrap_or_else(|e| panic!("the Save records: {e}"));
    let step = win
        .manifest_steps()
        .into_iter()
        .find(|s| s.contains("filtered"))
        .expect("the filter is recorded");
    win.show_grid_of(&step);
    win.focus_grid();
    win.app.chart_doc_mut().grid_cursor =
        Some(brightfield_shell::data_grid::GridCursor { row: 0, col: 0 });
    win.settle();
    let offer = win
        .app
        .chart_doc()
        .grid_preview()
        .expect("the step's grid is a preview")
        .offer();
    win.palette("filter-rows-at-cursor");
    assert_eq!(win.app.chart_doc().foot_asking(), None);
    assert_eq!(win.said(), Some(offer));
}

#[test]
fn an_operation_over_an_unrecorded_view_says_save_or_clear_it_first() {
    let mut cx = full_context();
    cx.in_front.as_mut().expect("a table").view_unrecorded = true;
    assert_eq!(
        availability(&HIDE_LIKE, &cx, Some("house_age")),
        Availability::Unavailable(
            "Hide column reads the table as recorded: save or clear the where and order by first."
                .to_string()
        )
    );
}

#[test]
fn availability_names_the_first_need_it_lacks() {
    static EACH: OperationUi = OperationUi {
        long_name: "append-rows",
        verb: "append-rows",
        stage: Stage::Model,
        icon: "stack",
        starts: &[Start::Cell],
        menu_words: "Append rows",
        needs: &[
            Need::ProtocolSource,
            Need::TakesSources,
            Need::Rows,
            Need::GridCell,
            Need::Column,
            Need::ColumnHasNulls,
            Need::ViewRecorded,
            Need::LoadedTable,
            Need::AnotherTable,
            Need::AnotherTableSharingAColumn,
        ],
        begin: begin_hide,
        keeps: Keeps::OnEnter,
        field: None,
        form: None,
        name: NameRule {
            word: "appended",
            from: By::Nothing,
            with_by: false,
        },
    };
    let mut met = full_context();
    let mut facts = OpWindow::on_copy_of(WITH_NULLS)
        .app
        .protocol_model()
        .columns()
        .to_vec();
    facts.retain(|c| c.column == "house_age");
    assert_eq!(facts[0].nulls, 1, "the fixture's house_age has one missing");
    front(&mut met).facts = facts;
    assert_eq!(availability(&EACH, &met, None), Availability::Available);

    let unmet = |change: &dyn Fn(&mut OperationContext), need: Need| {
        let mut cx = met.clone();
        change(&mut cx);
        assert_eq!(
            availability(&EACH, &cx, None),
            Availability::Unavailable(sentence(need, "Append rows", None)),
            "{need:?}"
        );
    };
    unmet(&|cx| cx.protocol = None, Need::ProtocolSource);
    unmet(&|cx| cx.takes_sources = false, Need::TakesSources);
    unmet(&|cx| front(cx).rows = false, Need::Rows);
    unmet(&|cx| front(cx).cell = false, Need::GridCell);
    unmet(&|cx| front(cx).cursor_column = None, Need::Column);
    unmet(&|cx| front(cx).facts.clear(), Need::ColumnHasNulls);
    unmet(&|cx| front(cx).view_unrecorded = true, Need::ViewRecorded);
    unmet(&|cx| front(cx).loaded = false, Need::LoadedTable);
    unmet(&|cx| cx.tables.truncate(1), Need::AnotherTable);
    unmet(
        &|cx| cx.tables[1].1 = Some(vec!["other".to_string()]),
        Need::AnotherTableSharingAColumn,
    );
    // A step's preview has its own words for a table that is not loaded.
    let mut cx = met.clone();
    front(&mut cx).loaded = false;
    front(&mut cx).step_offer = Some("the step has not been run".to_string());
    assert_eq!(
        availability(&EACH, &cx, None).sentence(),
        Some("the step has not been run")
    );
}

#[test]
fn an_at_once_operation_keeps_on_its_first_good_answer() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    assert!(start(&mut win, &HIDE_LIKE, "house_age"), "{:?}", win.said());
    win.settle_reads();
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, format!("{}_without_house_age", win.table()));
    assert!(kept[0].operation.is_some());
    assert!(win.making().is_none());
    win.wait_for_preview();
    assert!(!win.grid_columns().contains(&"house_age".to_string()));
    // Kept, and no file written.
    assert!(win.manifest_steps().is_empty());
}

#[test]
fn a_column_is_given_bare_to_a_name_and_quoted_to_sql() {
    let mut win = OpWindow::on_copy_of(WITH_NULLS);
    assert!(
        start(&mut win, &RENAME_LIKE, "median income"),
        "{:?}",
        win.said()
    );
    let draft = win.making().expect("the rename is being made");
    assert_eq!(draft.value("column"), Some("median income"));
    win.key(egui::Key::Escape);

    assert!(
        start(&mut win, &DERIVE_LIKE, "median income"),
        "{:?}",
        win.said()
    );
    let draft = win.making().expect("the derive is being made");
    assert_eq!(draft.value("expression"), Some("\"median income\""));
    win.key(egui::Key::Escape);
    // A word DuckDB reads as a keyword is quoted too, and a plain name is bare.
    assert!(start(&mut win, &DERIVE_LIKE, "order"));
    assert_eq!(
        win.making().expect("made").value("expression"),
        Some("\"order\"")
    );
    win.key(egui::Key::Escape);
    assert!(start(&mut win, &DERIVE_LIKE, "house_age"));
    assert_eq!(
        win.making().expect("made").value("expression"),
        Some("house_age")
    );
    // The preview reads the right column: the copy holds its values.
    win.settle_reads();
    assert!(win.grid_columns().contains(&"copy".to_string()));
    assert_eq!(win.preview_missing("copy"), Some(1));
    assert_eq!(win.preview_missing("city"), Some(0));
}

#[test]
fn enter_keeps_and_esc_keeps_nothing() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    assert!(
        start(&mut win, &RENAME_LIKE, "median_income"),
        "{:?}",
        win.said()
    );
    // Typed as a keyboard types it: `o` is the grid's sort key, `w` its
    // filter's, and here each is a letter of the name.
    win.type_keys("income");
    assert_eq!(
        win.making().expect("the draft is still open").value("to"),
        Some("income")
    );
    assert_eq!(win.app.chart_doc().foot_asking(), None);
    assert_eq!(win.app.open_overlay(), None);
    win.type_keys("w");
    assert_eq!(
        win.making().expect("still open").value("to"),
        Some("incomew")
    );
    win.key(egui::Key::Backspace);
    win.settle_reads();
    assert!(win.grid_columns().contains(&"income".to_string()));
    assert!(!win.grid_faded());
    win.key(egui::Key::Enter);
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(
        kept[0].name,
        format!("{}_renamed_median_income", win.table())
    );
    assert!(win.making().is_none());

    assert!(
        start(&mut win, &RENAME_LIKE, "house_age"),
        "{:?}",
        win.said()
    );
    win.type_text("age");
    win.settle_reads();
    win.key(egui::Key::Escape);
    assert!(win.making().is_none());
    assert_eq!(win.kept().len(), 1, "Esc kept no step");
    assert!(win.manifest_steps().is_empty(), "and no file was written");
}

#[test]
fn a_draft_goes_when_the_grid_it_began_over_does() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    assert!(start(&mut win, &HIDE_LIKE, "house_age"), "{:?}", win.said());
    win.settle_reads();
    let hidden = win.kept()[0].name.clone();
    // A rename begun over the loaded table's grid.
    assert!(
        start(&mut win, &RENAME_LIKE, "median_income"),
        "{:?}",
        win.said()
    );
    win.type_keys("inc");
    assert!(win.making().is_some());
    // Another step's grid is brought to the front: the draft goes with the
    // grid it was made over, and a key typed now is the grid's own.
    win.show_grid_of(&hidden);
    assert!(win.making().is_none(), "the draft went with its grid");
    assert_eq!(
        win.app
            .chart_doc()
            .grid_preview()
            .map(|p| p.step().to_string()),
        Some(hidden.clone()),
        "the grid shows the step brought forward"
    );
    win.focus_grid();
    win.type_keys("j");
    assert!(win.making().is_none());
    assert_eq!(win.kept().len(), 1, "no step was kept unseen");
}

#[test]
fn enter_with_a_required_value_empty_says_what_is_missing() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    assert!(
        start(&mut win, &RENAME_LIKE, "median_income"),
        "{:?}",
        win.said()
    );
    win.key(egui::Key::Enter);
    assert_eq!(win.said().as_deref(), Some("rename-column needs to"));
    assert!(win.making().is_some(), "the draft stays open");
    assert!(win.kept().is_empty());
}

#[test]
fn a_refused_edit_fades_the_grid_and_says_why() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    assert!(
        start(&mut win, &RENAME_LIKE, "median_income"),
        "{:?}",
        win.said()
    );
    win.type_text("income");
    win.settle_reads();
    assert!(!win.grid_faded());
    // A second column of a name the table has is DuckDB's to refuse.
    for _ in 0.."income".len() {
        win.key(egui::Key::Backspace);
    }
    win.type_text("house_age");
    win.settle_reads();
    assert!(win.grid_faded(), "the last good rows stay, faded");
    assert!(win.grid_columns().contains(&"income".to_string()));
    let band: Vec<&String> = win
        .texts
        .iter()
        .filter(|t| t.contains("house_age"))
        .collect();
    assert!(!band.is_empty(), "the reason is on the status band");
    win.key(egui::Key::Enter);
    assert!(win.kept().is_empty(), "Enter keeps no refused draft");
    assert!(win.making().is_some());
    assert!(win.manifest_steps().is_empty());
}

#[test]
fn cmd_s_keeps_an_admitted_draft_first() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    assert!(
        start(&mut win, &RENAME_LIKE, "median_income"),
        "{:?}",
        win.said()
    );
    win.type_text("income");
    win.settle_reads();
    win.press("cmd-s");
    let step = format!("{}_renamed_median_income", win.table());
    assert!(win.making().is_none(), "{:?}", win.said());
    assert!(
        win.manifest_steps().contains(&step),
        "{:?}; {}",
        win.manifest_steps(),
        win.banners()
    );
    assert!(win.kept().is_empty());
    assert!(win.model_of(&step).contains("TO income"));
}

#[test]
fn cmd_s_saves_the_rest_and_leaves_an_unfinished_draft_open() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    assert!(start(&mut win, &HIDE_LIKE, "house_age"), "{:?}", win.said());
    win.settle_reads();
    let hidden = win.kept()[0].name.clone();
    // The rename is started on the loaded table and given no name yet.
    assert!(
        start(&mut win, &RENAME_LIKE, "median_income"),
        "{:?}",
        win.said()
    );
    win.press("cmd-s");
    assert!(win.manifest_steps().contains(&hidden), "{}", win.banners());
    assert!(win.making().is_some(), "the unfinished draft stays open");
    assert_eq!(win.said().as_deref(), Some("rename-column needs to"));
    assert!(win.kept().is_empty(), "the kept step was recorded");
}

#[test]
fn the_editor_follows_the_draft_as_it_is_made() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    assert!(
        start(&mut win, &RENAME_LIKE, "median_income"),
        "{:?}",
        win.said()
    );
    win.type_text("inc");
    win.settle_reads();
    let drawn = win.editor();
    assert!(drawn.text.contains("TO inc;"), "{}", drawn.text);
    assert!(drawn.read_only);
    win.type_text("ome");
    win.settle_reads();
    let text = win.editor_text();
    assert!(text.contains("TO income;"), "{text}");
}

#[test]
fn starting_over_a_kept_draft_of_the_same_operation_corrects_it() {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    assert!(
        start(&mut win, &RENAME_LIKE, "median_income"),
        "{:?}",
        win.said()
    );
    win.type_text("income");
    win.settle_reads();
    win.key(egui::Key::Enter);
    let name = win.kept()[0].name.clone();
    win.wait_for_preview();

    // Its grid is in front; the same operation started there begins from it.
    let ctx = win.ctx.clone();
    assert!(
        win.app
            .start_operation_spec(&ctx, &RENAME_LIKE, Start::Cell, Some("income")),
        "{:?}",
        win.said()
    );
    win.settle();
    let making = win.app.making().expect("the kept rename is corrected");
    assert_eq!(making.corrects.as_deref(), Some(name.as_str()));
    assert_eq!(making.draft.value("column"), Some("median_income"));
    assert_eq!(making.draft.value("to"), Some("income"));
    // The kept name opens selected whole, so the new one is typed over it.
    assert!(making.whole);
    win.type_text("income_2");
    win.settle_reads();
    win.key(egui::Key::Enter);
    let kept = win.kept();
    assert_eq!(kept.len(), 1, "{:?}", win.said());
    assert_eq!(kept[0].name, name);
    let to = kept[0]
        .operation
        .as_ref()
        .and_then(|r| r.args.iter().find(|(k, _)| k == "to"))
        .map(|(_, v)| v.as_str());
    assert_eq!(to, Some("income_2"));
}

/// The chart palette lists each built operation, and reads it against what
/// the window holds: with nothing held, an operation's row is greyed with the
/// sentence its command declares, where a live row would confirm and then say
/// what it needs.
#[test]
fn the_chart_palette_lists_built_operations_and_greys_them_where_they_cannot_start() {
    use brightfield_keys::{availability, registry, Availability, Situation};

    let listed = chart_palette_verbs();
    let reg = registry();
    for spec in brightfield_shell::operation_ui::built() {
        assert!(listed.contains(&spec.verb), "{} is not listed", spec.verb);
        let verb = reg
            .iter()
            .find(|v| v.longname == spec.verb)
            .unwrap_or_else(|| panic!("{} is not in the registry", spec.verb));
        assert!(
            matches!(
                availability(verb, &Situation::default()),
                Availability::Missing(_)
            ),
            "{} reads available with nothing held",
            spec.verb
        );
    }
    assert!(listed.contains(&"filter-rows-at-cursor"));
    assert!(listed.contains(&"sort-rows-at-cursor"));
}
