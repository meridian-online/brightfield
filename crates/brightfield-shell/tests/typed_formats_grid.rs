//! **The grid shows each labelled column the way its file wrote it**: `(£12.00)` and `45%`, with
//! the unit in the column's header, and every other column as it is stored.
//!
//! The columns are labelled through a column record that nominates each label, as an analyst who
//! accepted them would have, so no FineType bundle is needed and the casts run from the transforms
//! vendored into `brightfield-protocol`.
//!
//! Regenerate the picture with `UPDATE_SNAPSHOTS=1`, with no FineType bundle beside the test
//! binary.

use std::path::PathBuf;

use brightfield_engine::{RowsAudience, RowsView};
use brightfield_shell::app::ChartDoc;
use brightfield_shell::column_header::ColumnBandFacts;
use brightfield_shell::column_record;
use brightfield_shell::data_file::{self, OpenOptions, OpenedFile};
use brightfield_shell::data_grid::{fetch_page_shown, page_of, DataGridItem, GridPage, PAGE_PAD};
use brightfield_shell::design::Mode;
use brightfield_workbench::{Item, ItemCtx, PaneKey};

/// Sterling amounts with an accounting negative, a percentage, a count nobody labelled, an ISO
/// date left as its label formats it and one the analyst chose a format for.
const GRID_CSV: &str = "\
revenue,share,count,day,booked
\"£1,234.50\",45%,1234567,2024-01-05,2024-01-05
(£12.00),12%,2,2024-02-10,2024-02-10
\"£900.00\",80%,3,2024-03-15,2024-03-15
\"£15.25\",3%,4,2024-04-20,2024-04-20
";

/// The labels the columns are given. `count` has none.
const LABELS: &[(&str, &str)] = &[
    ("revenue", "finance.currency.amount"),
    ("share", "representation.numeric.percentage"),
    ("day", "datetime.date.iso"),
    ("booked", "datetime.date.iso"),
];

/// The format the analyst chose for `booked`.
const BOOKED_FORMAT: &str = "%d %b %Y";

fn scratch_config() {
    use std::sync::OnceLock;
    static CONFIG: OnceLock<PathBuf> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("bf-grid-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch config folder");
        std::env::set_var(brightfield_shell::startup::CONFIG_DIR_VAR, &dir);
        dir
    });
}

/// `text` written as `typed_grid.csv` in a folder of the calling test's own, with a record
/// nominating `labels` and the format chosen for `booked`.
fn fixture(text: &str, labels: &[(&str, &str)]) -> PathBuf {
    fixture_named(text, labels, "typed_grid.csv")
}

/// [`fixture`], with the file called `name`.
fn fixture_named(text: &str, labels: &[(&str, &str)], name: &str) -> PathBuf {
    scratch_config();
    let thread = std::thread::current();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("typed_formats_grid")
        .join(thread.name().unwrap_or("main").replace("::", "-"));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join(name);
    std::fs::write(&path, text).expect("write fixture");
    let mut record = column_record::SourceRecord::default();
    for (column, label) in labels {
        let mut field = column_record::FieldRecord {
            label: Some((*label).to_string()),
            nominated: true,
            ..column_record::FieldRecord::default()
        };
        if *column == "booked" {
            field.prefer.date_format = Some(BOOKED_FORMAT.to_string());
        }
        record.fields.insert((*column).to_string(), field);
    }
    let package = column_record::flat_record_path(&path).expect("a record path");
    column_record::write_record(&package, data_file::SOURCE, &record).expect("write record");
    path
}

fn open(path: &std::path::Path) -> OpenedFile {
    let options = OpenOptions {
        type_source: None,
        ..OpenOptions::default()
    };
    data_file::open_traced(&path.to_string_lossy(), &options)
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
        .0
}

/// The page the grid reads over `opened`, shown by the representations its columns carry.
fn page(opened: &mut OpenedFile) -> GridPage {
    let facts = ColumnBandFacts::new(&opened.protocol.columns);
    let mark = opened.live.rows_mark();
    let session = opened.live.coordinator().session();
    fetch_page_shown(
        session,
        mark,
        0..PAGE_PAD,
        RowsAudience::Reader,
        &RowsView::default(),
        facts.shown(),
    )
    .expect("the grid reads its page")
}

/// The cell of `column` whose stored value is `stored`: the grid reads rows in no promised order,
/// so a row is found by what it holds.
fn cell_holding<'a>(
    page: &'a GridPage,
    column: &str,
    stored: &str,
) -> &'a brightfield_shell::data_grid::CellText {
    let at = page
        .columns
        .iter()
        .position(|c| c.name == column)
        .unwrap_or_else(|| panic!("no column {column}"));
    page.rows
        .iter()
        .map(|row| &row[at])
        .find(|c| c.exact.as_deref().unwrap_or(&c.text) == stored)
        .unwrap_or_else(|| {
            let held: Vec<_> = page.rows.iter().map(|row| &row[at]).collect();
            panic!("no {column} cell holding {stored}, in {held:?}")
        })
}

#[test]
fn labelled_cells_use_the_file_convention() {
    let mut opened = open(&fixture(GRID_CSV, LABELS));
    let page = page(&mut opened);

    let big = cell_holding(&page, "revenue", "1234.50");
    assert_eq!(big.text, "£1,234.50");
    assert_eq!(big.exact.as_deref(), Some("1234.50"));
    let negative = cell_holding(&page, "revenue", "-12.00");
    assert_eq!(negative.text, "(£12.00)");
    assert_eq!(negative.exact.as_deref(), Some("-12.00"));
    let share = cell_holding(&page, "share", "0.45");
    assert_eq!(share.text, "45%");
    assert_eq!(share.exact.as_deref(), Some("0.45"));
    assert_eq!(cell_holding(&page, "share", "0.03").text, "3%");
}

#[test]
fn unlabelled_columns_are_unchanged() {
    let mut opened = open(&fixture(GRID_CSV, LABELS));
    let page = page(&mut opened);
    let count = cell_holding(&page, "count", "1234567");
    assert_eq!(count.text, "1234567");
    assert_eq!(count.exact, None);
    let column = page.columns.iter().find(|c| c.name == "count").unwrap();
    assert_eq!(column.unit, None);
    assert_eq!(column.header_text(), "count");
}

#[test]
fn header_carries_the_unit() {
    let mut opened = open(&fixture(GRID_CSV, LABELS));
    let page = page(&mut opened);
    let header = |name: &str| {
        page.columns
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no column {name}"))
            .header_text()
    };
    assert_eq!(header("revenue"), "revenue · £");
    assert_eq!(header("share"), "share · %");
    assert_eq!(header("day"), "day");

    // The band header names the same unit.
    let facts = opened
        .protocol
        .columns
        .iter()
        .find(|c| c.column == "revenue")
        .unwrap();
    assert_eq!(facts.header_name(), "revenue · £");
}

/// A date column's cells are written in its format: the label's own leaves an ISO day as it is,
/// and the analyst's chosen format is applied, with the stored day kept as the exact value.
#[test]
fn a_date_columns_cells_are_unchanged_or_in_its_format() {
    let mut opened = open(&fixture(GRID_CSV, LABELS));
    let page = page(&mut opened);
    let day = cell_holding(&page, "day", "2024-01-05");
    assert_eq!(day.text, "2024-01-05");
    assert_eq!(day.exact, None);
    let booked = cell_holding(&page, "booked", "2024-01-05");
    assert_eq!(booked.text, "05 Jan 2024");
    assert_eq!(booked.exact.as_deref(), Some("2024-01-05"));
    let column = page.columns.iter().find(|c| c.name == "booked").unwrap();
    assert_eq!(column.unit, None);
}

/// A grid cell keeps the decimals the stored value has: 0.455 prints `45.5%` where the column's
/// own precision is whole percentages, and 0.45 still prints `45%`.
#[test]
fn a_cell_keeps_the_decimals_of_the_stored_value() {
    let csv = "share\n45.5%\n45%\n3%\n80%\n";
    let mut opened = open(&fixture(
        csv,
        &[("share", "representation.numeric.percentage")],
    ));
    let page = page(&mut opened);
    assert_eq!(cell_holding(&page, "share", "0.455").text, "45.5%");
    assert_eq!(cell_holding(&page, "share", "0.45").text, "45%");
    assert_eq!(cell_holding(&page, "share", "0.03").text, "3%");
}

/// A preview of typed rows is tabulated with the applied grid's representations, so the header's
/// unit sits over cells printed by it.
#[test]
fn a_preview_page_agrees_with_its_header() {
    let mut opened = open(&fixture(GRID_CSV, LABELS));
    let facts = ColumnBandFacts::new(&opened.protocol.columns);
    let mark = opened.live.rows_mark();
    let batches = opened
        .live
        .coordinator()
        .session()
        .execute_step_rows_window_view(mark, 0, 10, RowsAudience::Reader, &RowsView::default())
        .expect("rows");
    let page = page_of(&batches, facts.shown());
    let share = page.columns.iter().position(|c| c.name == "share").unwrap();
    assert_eq!(page.columns[share].header_text(), "share · %");
    assert!(page.rows.iter().any(|r| r[share].text == "45%"));
    // Without the representations the page is as stored, and its header names no unit.
    let bare = page_of(&batches, &Default::default());
    assert_eq!(bare.columns[share].header_text(), "share");
    assert!(bare.rows.iter().any(|r| r[share].text == "0.45"));
}

/// The grid pane over the fixture, drawn the way the window draws it.
fn pane(mode: Mode) -> egui_kittest::Harness<'static, (ChartDoc, DataGridItem)> {
    let mut opened = open(&fixture(GRID_CSV, LABELS));
    let columns = std::mem::take(&mut opened.protocol.columns);
    let mut doc = ChartDoc::headless(opened.composed);
    doc.set_column_facts(&columns);
    doc.attach_live(opened.live);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(560.0, 200.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .build_ui_state(
            move |ui, state: &mut (ChartDoc, DataGridItem)| {
                brightfield_shell::design::apply(ui.ctx(), mode);
                egui::CentralPanel::default().show(ui, |ui| {
                    let (doc, item) = state;
                    let mut requests = Vec::new();
                    let mut cx = ItemCtx::new(
                        mode,
                        PaneKey::new(item.item_id()),
                        egui_tiles::TileId::from_u64(1),
                        true,
                        &mut requests,
                    );
                    item.ui(doc, ui, &mut cx);
                });
            },
            (doc, DataGridItem::new()),
        );
    harness.run();
    harness
}

/// The pane reads the representations off the document's facts, so the drawn grid carries the
/// unit in its header and the file's convention in its cells.
#[test]
fn the_pane_draws_the_unit_and_the_formatted_cells() {
    use egui_kittest::kittest::Queryable;
    let harness = pane(Mode::Light);
    // The table draws a header once per scrolling region, so a label may be found more than once.
    for label in ["revenue · £", "share · %", "(£12.00)", "45%"] {
        assert!(
            harness.query_all_by_label(label).next().is_some(),
            "the grid draws no {label}"
        );
    }
}

#[test]
fn typed_grid_light_snapshot() {
    pane(Mode::Light).snapshot("typed_grid_light");
}

#[test]
fn typed_grid_dark_snapshot() {
    pane(Mode::Dark).snapshot("typed_grid_dark");
}

/// The labels of the committed typed fixture's columns. `notes` has none.
const TYPED_LABELS: &[(&str, &str)] = &[
    ("revenue", "finance.currency.amount"),
    ("price_usd", "finance.currency.amount"),
    ("share", "representation.numeric.percentage"),
    ("yield", "finance.rate.yield"),
    ("year", "datetime.component.year"),
    ("downloads", "representation.numeric.integer_number"),
    ("day", "datetime.date.iso"),
];

/// The answer a typed read of `statement` over the typed fixture, written as `name`, comes back
/// with, beside the file opened as the window opens it.
fn typed_answer(
    name: &str,
    statement: impl Fn(&str) -> String,
) -> (OpenedFile, brightfield_shell::typed_sql::TypedAnswer) {
    use brightfield_engine::FileReader;
    use brightfield_shell::run::Runner;
    use brightfield_shell::typed_sql::{TypedReads, TypedSource, TYPED_PAUSE};
    use std::time::{Duration, Instant};

    brightfield_shell::data_grid::use_own_preview_gate_on_this_thread();
    let path = fixture_named(include_str!("data/typed_formats.csv"), TYPED_LABELS, name);
    let opened = open(&path);
    assert!(
        opened.protocol.casts.iter().any(|(c, _)| c == "share"),
        "fixture check: share is cast on open"
    );
    let facts = ColumnBandFacts::new(&opened.protocol.columns);
    let source = TypedSource {
        data: path.clone(),
        reader: FileReader::Csv,
        root_table: opened.protocol.name.clone(),
        steps: Vec::new(),
        order: None,
        runner: Some(Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))),
        shown: facts.shown().clone(),
        casts: opened.protocol.casts.clone(),
    };
    let mut reads = TypedReads::new(source, TYPED_PAUSE, || {});
    reads.edit(&statement(&opened.protocol.name), Instant::now());
    reads.read_now();
    let until = Instant::now() + Duration::from_secs(60);
    while reads.current().is_none() && reads.refusal().is_none() {
        assert!(Instant::now() < until, "no answer");
        reads.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    let answer = reads
        .current()
        .unwrap_or_else(|| panic!("refused: {:?}", reads.refusal()))
        .clone();
    (opened, answer)
}

/// The headers of `page`'s columns.
fn headers(page: &GridPage) -> Vec<String> {
    page.columns.iter().map(|c| c.header_text()).collect()
}

/// A typed filter's preview reads the file through the casts the session reads it through, so
/// `share > 0.4` keeps the rows the applied `where` keeps and pages them the same way, and neither
/// the filter's page nor the statement's own shows the raw text the grid hides.
#[test]
fn a_typed_filter_previews_the_rows_the_applied_where_keeps() {
    let condition = "share > 0.4";
    let (mut opened, answer) = typed_answer("typed_grid.csv", |table| {
        format!("select * from {table} where {condition}")
    });
    assert!(
        headers(&answer.page)
            .iter()
            .all(|h| !h.starts_with(brightfield_shell::data_file::RAW_PREFIX)),
        "the statement's page shows raw text: {:?}",
        headers(&answer.page)
    );
    let facts = ColumnBandFacts::new(&opened.protocol.columns);
    let preview = answer
        .filter_page
        .expect("a filter of the root table is paged");

    let mark = opened.live.rows_mark();
    let session = opened.live.coordinator().session();
    let applied = fetch_page_shown(
        session,
        mark,
        0..PAGE_PAD,
        RowsAudience::Reader,
        &RowsView {
            condition: Some(condition.to_string()),
            order: None,
        },
        facts.shown(),
    )
    .expect("the applied grid reads its page");
    assert!(
        !applied.rows.is_empty(),
        "fixture check: some shares are over 40%"
    );
    assert!(
        (applied.rows.len() as u64) < PAGE_PAD,
        "fixture check: the applied page holds every row the condition keeps"
    );
    assert_eq!(answer.filter_rows, Some(applied.rows.len() as u64));
    assert_eq!(headers(&preview), headers(&applied));
    let cells = |page: &GridPage| -> Vec<Vec<String>> {
        page.rows
            .iter()
            .map(|row| row.iter().map(|c| c.text.clone()).collect())
            .collect()
    };
    assert_eq!(cells(&preview), cells(&applied));
}

/// A file called `opened_rows.csv`, the name the casts read the file's rows under, still previews:
/// the rows load under a name the root table does not have.
#[test]
fn a_file_named_as_the_raw_rows_still_previews() {
    let (opened, answer) = typed_answer("opened_rows.csv", |table| {
        format!("select * from {table} order by year")
    });
    assert_eq!(opened.protocol.name, "opened_rows", "fixture check");
    assert!(!answer.page.rows.is_empty(), "the statement read rows");
    let share = answer
        .page
        .columns
        .iter()
        .position(|c| c.name == "share")
        .expect("a share column");
    assert!(
        answer.page.rows[0][share].text.ends_with('%'),
        "share is read as the cast fraction and shown as a percentage: {:?}",
        answer.page.rows[0][share]
    );
}
