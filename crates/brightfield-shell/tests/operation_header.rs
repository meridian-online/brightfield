//! A column's header and the inspector's Operation section as places to act:
//! a gesture on a part of a header starts the operation that starts there, a
//! secondary click lists them, an operation asks in place on the header, and
//! one with a form draws it in the Operation section.
//!
//! No operation that starts from a header is built yet, so the operations
//! here are specs local to this file, handed to the header in place of the
//! built ones.

#[path = "support/operations.rs"]
mod operations;

use brightfield_shell::operation_ui::header::Gesture;
use brightfield_shell::operation_ui::{
    Begin, By, Draft, FormCx, FormEvent, HeaderPart, InPlace, Keeps, NameRule, Need, OperationUi,
    Stage, Start,
};
use operations::OpWindow;

const SAMPLE: &str = "california_housing_sample.csv";
const WITH_NULLS: &str = "with_nulls.csv";

fn on_column(long_name: &'static str, b: &Begin<'_>, value: &'static str) -> Result<Draft, String> {
    let mut draft = Draft::empty(long_name);
    draft.on = b.table.map(str::to_string);
    draft.set("column", b.column.unwrap_or_default().to_string());
    draft.set(value, String::new());
    draft.focus = Some(value);
    Ok(draft)
}

fn begin_rename(b: &Begin<'_>) -> Result<Draft, String> {
    on_column("rename-column", b, "to")
}

fn begin_cast(b: &Begin<'_>) -> Result<Draft, String> {
    on_column("cast-column", b, "type")
}

fn begin_fill(b: &Begin<'_>) -> Result<Draft, String> {
    on_column("fill-nulls", b, "value")
}

fn begin_derive(b: &Begin<'_>) -> Result<Draft, String> {
    let mut draft = Draft::empty("derive-column");
    draft.on = b.table.map(str::to_string);
    draft.set("column", String::new());
    draft.set("expression", String::new());
    draft.focus = Some("column");
    Ok(draft)
}

fn begin_summarise(b: &Begin<'_>) -> Result<Draft, String> {
    let mut draft = Draft::empty("summarise-rows");
    draft.on = b.table.map(str::to_string);
    draft.set("aggregates", "count(*) AS n".to_string());
    draft.set("group_by", (b.quote)(b.column.unwrap_or_default()));
    Ok(draft)
}

fn none(_: &Begin<'_>, _: &Draft) -> Vec<String> {
    Vec::new()
}

/// Two names for the column, whatever is typed.
fn two_names(b: &Begin<'_>, _: &Draft) -> Vec<String> {
    let column = b.column.unwrap_or_default();
    vec![format!("{column}_usd"), format!("{column}_eur")]
}

/// The summary's form: one line saying what it groups by.
fn summary_form(draft: &mut Draft, ui: &mut egui::Ui, cx: &FormCx<'_>) -> FormEvent {
    ui.label(format!(
        "Group {} by {}",
        cx.begin.table.unwrap_or_default(),
        draft.value("group_by").unwrap_or_default()
    ));
    FormEvent::None
}

const NEEDS: &[Need] = &[Need::ProtocolSource, Need::Rows, Need::Column];

const fn spec(
    long_name: &'static str,
    menu_words: &'static str,
    starts: &'static [Start],
    begin: fn(&Begin<'_>) -> Result<Draft, String>,
    field: Option<InPlace>,
    word: &'static str,
) -> OperationUi {
    OperationUi {
        long_name,
        verb: long_name,
        stage: Stage::Shape,
        icon: "pencil",
        starts,
        menu_words,
        needs: NEEDS,
        begin,
        keeps: Keeps::OnEnter,
        field,
        form: None,
        name: NameRule {
            word,
            from: By::Value("column"),
            with_by: false,
        },
    }
}

static RENAME: OperationUi = spec(
    "rename-column",
    "Rename column",
    &[
        Start::Header(HeaderPart::Name),
        Start::Header(HeaderPart::Menu),
    ],
    begin_rename,
    Some(InPlace {
        part: HeaderPart::Name,
        keys: &["to"],
        suggestions: two_names,
    }),
    "renamed",
);

/// A rename whose field opens on the column's own name.
fn begin_rename_named(b: &Begin<'_>) -> Result<Draft, String> {
    let mut draft = on_column("rename-column", b, "to")?;
    draft.set("to", b.column.unwrap_or_default().to_string());
    Ok(draft)
}

static RENAME_NAMED: OperationUi = spec(
    "rename-column",
    "Rename column",
    &[Start::Header(HeaderPart::Name)],
    begin_rename_named,
    Some(InPlace {
        part: HeaderPart::Name,
        keys: &["to"],
        suggestions: none,
    }),
    "renamed",
);

/// An operation that starts from a header's menu and from no part of it.
static MENU_ONLY: OperationUi = spec(
    "hide-column",
    "Hide column",
    &[Start::Header(HeaderPart::Menu)],
    begin_rename,
    None,
    "without",
);

static CAST: OperationUi = spec(
    "cast-column",
    "Cast column",
    &[Start::Header(HeaderPart::Type)],
    begin_cast,
    Some(InPlace {
        part: HeaderPart::Type,
        keys: &["type"],
        suggestions: none,
    }),
    "cast",
);

static FILL: OperationUi = OperationUi {
    needs: &[
        Need::ProtocolSource,
        Need::Rows,
        Need::Column,
        Need::ColumnHasNulls,
    ],
    ..spec(
        "fill-nulls",
        "Fill nulls",
        &[
            Start::Header(HeaderPart::Missing),
            Start::Header(HeaderPart::Menu),
        ],
        begin_fill,
        Some(InPlace {
            part: HeaderPart::Missing,
            keys: &["value"],
            suggestions: none,
        }),
        "filled",
    )
};

static DERIVE: OperationUi = OperationUi {
    needs: &[Need::ProtocolSource, Need::Rows],
    ..derive()
};

const fn derive() -> OperationUi {
    spec(
        "derive-column",
        "Derive column",
        &[Start::Header(HeaderPart::NewColumn)],
        begin_derive,
        Some(InPlace {
            part: HeaderPart::NewColumn,
            keys: &["column", "expression"],
            suggestions: none,
        }),
        "with",
    )
}

static SUMMARISE: OperationUi = OperationUi {
    form: Some(summary_form),
    name: NameRule {
        word: "summarised",
        from: By::Value("group_by"),
        with_by: true,
    },
    ..spec(
        "summarise-rows",
        "Summarise by this column",
        &[Start::Header(HeaderPart::Menu)],
        begin_summarise,
        None,
        "summarised",
    )
};

/// A window on `fixture` whose header offers `ops`, with the loaded table's
/// grid in front and arc's description of each operation read.
fn window(fixture: &str, ops: &[&'static OperationUi]) -> OpWindow {
    let mut win = OpWindow::on_copy_of(fixture);
    win.app.chart_doc_mut().set_header_operations(ops.to_vec());
    let table = win.table();
    win.show_grid_of(&table);
    win.focus_grid();
    // Each description is asked for and waited on, so a gesture below starts
    // its operation on the first try.
    let ctx = win.ctx.clone();
    for op in ops {
        for _ in 0..600 {
            let started = win
                .app
                .start_operation_spec(&ctx, op, Start::Cell, Some("x"));
            let pending = win
                .app
                .operation_said()
                .is_some_and(|said| said.starts_with("Reading arc's description"));
            if started {
                win.key(egui::Key::Escape);
            }
            if !pending {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
            win.run(Vec::new());
        }
    }
    win.key(egui::Key::Escape);
    win.settle();
    win
}

fn parts_of(win: &OpWindow, column: &str) -> Vec<HeaderPart> {
    win.app
        .chart_doc()
        .header_drawn()
        .parts
        .iter()
        .filter(|(name, ..)| name == column)
        .map(|(_, part, _)| *part)
        .collect()
}

#[test]
fn a_double_click_on_a_name_starts_the_operation_that_starts_there() {
    let mut win = window(SAMPLE, &[&RENAME, &CAST]);
    win.header("median_income", HeaderPart::Name, Gesture::DoubleClick);
    let making = win.app.making().expect("the rename started");
    assert_eq!(making.spec.long_name, "rename-column");
    assert_eq!(making.column.as_deref(), Some("median_income"));
    assert_eq!(making.start, Start::Header(HeaderPart::Name));
    win.key(egui::Key::Escape);

    // One click on the name is not the gesture, and starts no operation.
    win.header("median_income", HeaderPart::Name, Gesture::Click);
    assert!(win.app.making().is_none(), "{:?}", win.said());
}

#[test]
fn a_click_on_the_type_and_on_the_missing_count_start_theirs() {
    let mut win = window(WITH_NULLS, &[&RENAME, &CAST, &FILL]);
    win.header("house_age", HeaderPart::Type, Gesture::Click);
    let making = win.app.making().expect("the cast started");
    assert_eq!(making.spec.long_name, "cast-column");
    assert_eq!(making.column.as_deref(), Some("house_age"));
    win.key(egui::Key::Escape);

    win.header("house_age", HeaderPart::Missing, Gesture::Click);
    let making = win.app.making().expect("the fill started");
    assert_eq!(making.spec.long_name, "fill-nulls");
    assert_eq!(making.column.as_deref(), Some("house_age"));
    win.key(egui::Key::Escape);

    // A column with no missing value says what the fill needs.
    win.header("city", HeaderPart::Missing, Gesture::Click);
    assert!(win.app.making().is_none());
    assert_eq!(
        win.said().as_deref(),
        Some("Fill nulls needs a column with missing values.")
    );
}

#[test]
fn the_header_menu_lists_each_operation_that_starts_from_a_header_with_its_key() {
    let mut win = window(WITH_NULLS, &[&RENAME, &CAST, &FILL, &SUMMARISE]);
    win.header("house_age", HeaderPart::Menu, Gesture::Secondary);
    let rows: Vec<(String, Option<String>)> = win
        .app
        .chart_doc()
        .header_drawn()
        .menu
        .iter()
        .map(|row| (row.words.clone(), row.key.clone()))
        .collect();
    let key = |k: &str| Some(k.to_string());
    assert_eq!(
        rows,
        [
            ("Rename column".to_string(), key("shift-r")),
            ("Cast column".to_string(), key("shift-t")),
            ("Fill nulls".to_string(), key("f")),
            ("Summarise by this column".to_string(), key("shift-f")),
        ]
    );
    assert!(
        win.app.making().is_none(),
        "the secondary click starts none"
    );
    // The menu opens from a secondary click on a part too: on the name, which
    // a double-click renames from, and on the missing count.
    win.key(egui::Key::Escape);
    win.settle();
    for part in [HeaderPart::Name, HeaderPart::Missing] {
        win.header("house_age", part, Gesture::Secondary);
        assert!(
            win.texts.iter().any(|t| t.starts_with("Rename column")),
            "a secondary click on the {part:?} opens the menu: {:?}",
            win.texts
        );
        assert!(win.app.making().is_none());
        win.key(egui::Key::Escape);
        win.settle();
    }
    win.header("house_age", HeaderPart::Menu, Gesture::Secondary);
    // Choosing a row starts its operation on that column.
    win.click_text("Rename column");
    win.settle();
    let making = win.app.making().expect("the rename started from the menu");
    assert_eq!(making.spec.long_name, "rename-column");
    assert_eq!(making.column.as_deref(), Some("house_age"));
}

#[test]
fn an_operation_the_column_cannot_take_is_greyed_in_the_menu_with_its_sentence() {
    let mut win = window(WITH_NULLS, &[&RENAME, &FILL]);
    let sentence = |win: &OpWindow| {
        win.app
            .chart_doc()
            .header_drawn()
            .menu
            .iter()
            .find(|row| row.words == "Fill nulls")
            .expect("the menu lists the fill")
            .sentence
            .clone()
    };
    win.header("city", HeaderPart::Menu, Gesture::Secondary);
    assert_eq!(
        sentence(&win).as_deref(),
        Some("Fill nulls needs a column with missing values.")
    );
    assert!(
        win.texts
            .iter()
            .any(|t| t.contains("needs a column with missing values")),
        "the sentence is drawn in the menu: {:?}",
        win.texts
    );
    // The greyed row starts no operation.
    win.click_text("Fill nulls");
    win.settle();
    assert!(win.app.making().is_none());

    win.key(egui::Key::Escape);
    win.header("house_age", HeaderPart::Menu, Gesture::Secondary);
    assert_eq!(sentence(&win), None);
}

#[test]
fn a_part_no_operation_starts_from_senses_nothing() {
    // An operation that starts from a header's menu alone, as hiding a column does:
    // each cell is sensed for the menu and no part is.
    let win = window(WITH_NULLS, &[&MENU_ONLY]);
    let drawn = win.app.chart_doc().header_drawn().clone();
    assert!(
        drawn.parts.is_empty() && !drawn.cells.is_empty(),
        "{drawn:?}"
    );
    assert_eq!(drawn.new_column, None);

    // With the rename alone, the name is sensed and the type and count are not.
    let win = window(WITH_NULLS, &[&RENAME]);
    assert_eq!(parts_of(&win, "house_age"), [HeaderPart::Name]);
    let win = window(WITH_NULLS, &[&RENAME, &CAST, &FILL]);
    assert_eq!(
        parts_of(&win, "house_age"),
        [HeaderPart::Name, HeaderPart::Type, HeaderPart::Missing]
    );
}

#[test]
fn a_field_in_place_takes_typing_tab_and_its_suggestions() {
    let mut win = window(SAMPLE, &[&RENAME, &DERIVE]);
    win.header("median_income", HeaderPart::Name, Gesture::DoubleClick);
    assert_eq!(win.field_text().as_deref(), Some(""));
    assert_eq!(
        win.suggestions(),
        ["median_income_usd", "median_income_eur"]
    );
    win.type_text("inc");
    assert_eq!(win.field_text().as_deref(), Some("inc"));
    // The preview arrives with the column under its new name, and the field
    // stays on the column it was opened on.
    win.settle_reads();
    assert!(win.grid_columns().contains(&"inc".to_string()));
    assert_eq!(win.field_text().as_deref(), Some("inc"));
    assert_eq!(win.suggestions().len(), 2);
    win.key(egui::Key::Backspace);
    assert_eq!(win.field_text().as_deref(), Some("in"));
    // Down, Down, Up is the first again; Tab takes it.
    win.key(egui::Key::ArrowDown);
    win.key(egui::Key::ArrowDown);
    win.key(egui::Key::ArrowUp);
    let selected = win
        .app
        .chart_doc()
        .header_drawn()
        .field
        .as_ref()
        .and_then(|field| field.selected);
    assert_eq!(selected, Some(0));
    win.key(egui::Key::Tab);
    assert_eq!(win.field_text().as_deref(), Some("median_income_usd"));
    win.settle_reads();
    win.key(egui::Key::Enter);
    assert_eq!(win.kept().len(), 1, "{:?}", win.said());
    assert_eq!(
        win.field_text(),
        None,
        "the field closes with the operation"
    );

    // Tab moves between the values a field of two edits.
    win.cursor_on("house_age");
    win.header_new_column();
    assert_eq!(win.making().and_then(|d| d.focus), Some("column"));
    win.type_text("double");
    win.key(egui::Key::Tab);
    assert_eq!(win.making().and_then(|d| d.focus), Some("expression"));
    win.type_text("house_age * 2");
    let draft = win.making().expect("the derive is being made");
    assert_eq!(draft.value("column"), Some("double"));
    assert_eq!(draft.value("expression"), Some("house_age * 2"));
    assert_eq!(win.field_text().as_deref(), Some("house_age * 2"));
}

/// Whether the field in place draws its text selected whole.
fn field_whole(win: &OpWindow) -> bool {
    win.app
        .chart_doc()
        .header_drawn()
        .field
        .as_ref()
        .is_some_and(|field| field.whole)
}

#[test]
fn a_field_that_opens_on_a_value_selects_it_whole() {
    let mut win = window(SAMPLE, &[&RENAME_NAMED]);
    win.header("median_income", HeaderPart::Name, Gesture::DoubleClick);
    assert_eq!(win.field_text().as_deref(), Some("median_income"));
    assert!(field_whole(&win));
    // The first text typed replaces it, and then the typing adds to it.
    win.type_text("inc");
    assert_eq!(win.field_text().as_deref(), Some("inc"));
    assert!(!field_whole(&win));
    win.type_text("ome");
    assert_eq!(win.field_text().as_deref(), Some("income"));
    win.key(egui::Key::Escape);

    // Backspace over it empties the value, and a second takes nothing more.
    win.header("median_income", HeaderPart::Name, Gesture::DoubleClick);
    assert!(field_whole(&win));
    win.key(egui::Key::Backspace);
    assert_eq!(win.field_text().as_deref(), Some(""));
    assert!(!field_whole(&win));
    win.type_text("x");
    assert_eq!(win.field_text().as_deref(), Some("x"));
    win.key(egui::Key::Escape);

    // An arrow or End lets go of it, and the typing adds to the name.
    for key in [
        egui::Key::ArrowRight,
        egui::Key::End,
        egui::Key::ArrowLeft,
        egui::Key::Home,
    ] {
        win.header("median_income", HeaderPart::Name, Gesture::DoubleClick);
        assert!(field_whole(&win));
        win.key(key);
        assert!(!field_whole(&win), "{key:?}");
        win.type_text("_usd");
        assert_eq!(
            win.field_text().as_deref(),
            Some("median_income_usd"),
            "{key:?}"
        );
        win.key(egui::Key::Escape);
    }
}

#[test]
fn the_new_column_cell_shows_only_when_an_operation_starts_there() {
    let win = window(SAMPLE, &[&RENAME]);
    assert_eq!(win.app.chart_doc().header_drawn().new_column, None);

    let mut win = window(SAMPLE, &[&RENAME, &DERIVE]);
    assert!(win.app.chart_doc().header_drawn().new_column.is_some());
    win.header_new_column();
    let making = win.app.making().expect("the derive started");
    assert_eq!(making.spec.long_name, "derive-column");
    assert_eq!(making.column, None);
    // While its field is open the cell is the field's.
    assert_eq!(win.app.chart_doc().header_drawn().new_column, None);
    assert!(win.app.chart_doc().header_drawn().field.is_some());
    win.key(egui::Key::Escape);
    assert!(win.app.chart_doc().header_drawn().new_column.is_some());
}

#[test]
fn the_operation_section_draws_the_form_of_the_operation_being_made() {
    let mut win = window(SAMPLE, &[&RENAME, &SUMMARISE]);
    win.header_menu("house_age", "Summarise by this column");
    win.settle();
    let making = win.app.making().expect("the summary started");
    assert_eq!(making.spec.long_name, "summarise-rows");
    let table = win.table();
    let line = format!("Group {table} by house_age");
    assert!(
        win.texts.iter().any(|t| t == &line),
        "the Operation section drew the form's line {line:?}: {:?}",
        win.texts
    );
    assert!(win.texts.iter().any(|t| t == "Summarise rows"));
    // Kept on Enter, as an operation typed in place is.
    win.settle_reads();
    win.key(egui::Key::Enter);
    assert_eq!(win.kept().len(), 1, "{:?}", win.said());
    assert_eq!(
        win.kept()[0].name,
        format!("{table}_summarised_by_house_age")
    );
}
