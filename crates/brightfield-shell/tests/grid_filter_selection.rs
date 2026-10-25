//! **A filter typed in a dashboard's grid filters the dashboard's charts.**
//!
//! Brushing a chart already narrows the grid. These tests hold the other
//! direction: a condition applied to the grid through
//! `ChartDoc::apply_grid_view` is published into the document's crossfilter
//! selection, so every chart narrows to the rows the grid shows, an empty
//! match draws the charts' empty state rather than an error banner, and
//! clearing the condition puts the whole table back.
//!
//! Driven over California Housing, written from the committed copy of its
//! source the way `tests/region_gate.rs` writes it, so nothing here reaches
//! the network.

use brightfield_engine::RowsView;
use brightfield_shell::data_file;
use brightfield_shell::pipeline::RowCount;
use brightfield_shell::starts::{self, Opened};

const WHOLE_TABLE: u64 = 20_640;

/// The data file the California Housing start writes, as a path.
fn housing_file() -> std::path::PathBuf {
    std::env::set_var(
        brightfield_shell::startup::CONFIG_DIR_VAR,
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("config"),
    );
    let data = starts::find(starts::CALIFORNIA_HOUSING)
        .and_then(|start| start.data.as_ref())
        .expect("California Housing is a data start");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/houses.zip");
    let fetched = brightfield_shell::remote::Fetched::stand_in(&[(data.url, &source)])
        .expect("stand in for the archive");
    match starts::load_with(starts::CALIFORNIA_HOUSING, fetched).expect("write the data file") {
        Opened::File(path) => path,
        _ => panic!("California Housing opens as a data file"),
    }
}

/// How many rows of `path` meet `condition`, asked of DuckDB directly.
fn rows_meeting(path: &std::path::Path, condition: &str) -> u64 {
    let conn = duckdb::Connection::open_in_memory().expect("open DuckDB");
    let n: i64 = conn
        .query_row(
            &format!("SELECT count(*) FROM read_parquet(?) WHERE {condition}"),
            [path.display().to_string()],
            |row| row.get(0),
        )
        .expect("count the matching rows");
    u64::try_from(n).expect("a count is not negative")
}

fn open() -> (std::path::PathBuf, brightfield_shell::app::ChartDoc) {
    let path = housing_file();
    let opened = data_file::open(path.to_str().expect("utf-8 path")).expect("open the data file");
    let mut doc = brightfield_shell::app::ChartDoc::headless(opened.composed);
    doc.attach_live(opened.live);
    (path, doc)
}

fn view(condition: &str) -> RowsView {
    RowsView {
        condition: Some(condition.into()),
        ..RowsView::default()
    }
}

fn charts_hold(doc: &brightfield_shell::app::ChartDoc) -> u64 {
    doc.composed
        .rows
        .as_ref()
        .map(|RowCount { selected, .. }| *selected)
        .expect("the map tile reports its rows")
}

#[test]
fn a_typed_filter_in_the_grid_filters_the_dashboards_charts() {
    let (path, mut doc) = open();
    assert_eq!(charts_hold(&doc), WHOLE_TABLE, "the dashboard opens whole");
    let want = rows_meeting(&path, "housing_median_age > 28");
    assert!(0 < want && want < WHOLE_TABLE, "the condition must narrow");

    doc.apply_grid_view(view("housing_median_age > 28"));

    assert_eq!(
        charts_hold(&doc),
        want,
        "the map still counts the rows the grid filtered out"
    );
    assert!(doc.chart_fault().is_none(), "a filter that binds is no fault");
}

#[test]
fn a_typed_filter_that_matches_nothing_draws_the_charts_empty_not_an_error() {
    let (path, mut doc) = open();
    assert_eq!(rows_meeting(&path, "housing_median_age > 280"), 0);

    doc.apply_grid_view(view("housing_median_age > 280"));

    assert_eq!(charts_hold(&doc), 0, "no row matches, so the map holds none");
    assert!(
        doc.chart_fault().is_none(),
        "an empty match raised a banner: {:?}",
        doc.chart_fault().map(|f| (f.title, f.detail))
    );
}

#[test]
fn clearing_the_typed_filter_restores_the_charts() {
    let (_path, mut doc) = open();
    doc.apply_grid_view(view("housing_median_age > 28"));
    assert!(charts_hold(&doc) < WHOLE_TABLE);

    doc.apply_grid_view(RowsView::default());

    assert_eq!(charts_hold(&doc), WHOLE_TABLE);
    assert!(!doc.selection_active(), "the grid's clause was not retracted");
}
