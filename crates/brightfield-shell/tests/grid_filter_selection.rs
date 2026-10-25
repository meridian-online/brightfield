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

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::RowsView;
use brightfield_engine::SqlPredicate;
use brightfield_shell::app::GRID_CONTRIBUTOR;
use brightfield_shell::data_file;
use brightfield_shell::pipeline::RowCount;
use brightfield_shell::starts::{self, Opened};
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::layout::{PlotAxis, ScaleType};
use brightfield_workbench::channel::ShelfChannel;

const WHOLE_TABLE: u64 = 20_640;

/// The generated dashboard's one selection.
const SEL: &str = brightfield_shell::dashboard::SELECTION;

fn grid() -> ComponentPath {
    ComponentPath(GRID_CONTRIBUTOR.to_string())
}

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
    assert!(
        doc.chart_fault().is_none(),
        "a filter that binds is no fault"
    );
}

#[test]
fn a_typed_filter_that_matches_nothing_draws_the_charts_empty_not_an_error() {
    let (path, mut doc) = open();
    assert_eq!(rows_meeting(&path, "housing_median_age > 280"), 0);

    doc.apply_grid_view(view("housing_median_age > 280"));

    assert_eq!(
        charts_hold(&doc),
        0,
        "no row matches, so the map holds none"
    );
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
    assert!(
        doc.held_clause(SEL, &grid()).is_none(),
        "the grid's clause is still held after its condition was cleared"
    );
}

/// Recording the grid's condition as a step empties the grid's where field,
/// and the charts follow it.
#[test]
fn recording_the_grid_condition_lets_the_charts_follow_the_empty_where_field() {
    let (_path, mut doc) = open();
    doc.apply_grid_view(view("housing_median_age > 28"));
    assert!(charts_hold(&doc) < WHOLE_TABLE);

    doc.mark_grid_condition_recorded("kept_step".to_string());

    assert_eq!(charts_hold(&doc), WHOLE_TABLE, "the charts stayed filtered");
    assert!(doc.held_clause(SEL, &grid()).is_none());
}

/// A rebuilt page starts with an empty session; the grid still shows its
/// condition, so the charts must still be narrowed.
#[test]
fn a_rebuilt_page_keeps_the_charts_narrowed_to_what_the_grid_shows() {
    let (path, mut doc) = open();
    let want = rows_meeting(&path, "housing_median_age > 28");
    doc.apply_grid_view(view("housing_median_age > 28"));

    assert!(
        doc.set_plot_scale(0, PlotAxis::Y, ScaleType::Log),
        "the rebuild-triggering change was refused"
    );

    assert_eq!(
        charts_hold(&doc),
        want,
        "the rebuild dropped the grid's clause"
    );
}

/// A grid-only filter is not a selection somebody made, so the grid's foot
/// does not say the rows are under one.
#[test]
fn a_grid_only_filter_is_not_counted_as_a_selection() {
    let (_path, mut doc) = open();
    doc.apply_grid_view(view("housing_median_age > 28"));
    assert!(
        !doc.live_dashboard().expect("live").rows_narrowed(),
        "the grid's own clause was counted as a selection"
    );
}

fn brush_hero(doc: &mut brightfield_shell::app::ChartDoc) {
    let hero = ComponentPath(doc.composed.plots[0].path.clone());
    assert!(doc.apply_interaction(Interaction::Select {
        name: SEL.to_string(),
        contributor: hero,
        predicate: SqlPredicate::Interval {
            column: "median_income".to_string(),
            lo: brightfield_sql::ir::ScalarValue::Float(2.0),
            hi: brightfield_sql::ir::ScalarValue::Float(4.0),
            meta: None,
        },
    }));
}

const BRUSH: &str = "median_income >= 2.0 AND median_income <= 4.0";

/// Clear-selection retracts the brush and leaves the grid's filter; clearing
/// the grid leaves the brush.
#[test]
fn the_brush_and_the_grid_filter_are_cleared_each_by_its_own_owner() {
    let (path, mut doc) = open();
    brush_hero(&mut doc);
    doc.apply_grid_view(view("housing_median_age > 28"));
    assert_eq!(
        charts_hold(&doc),
        rows_meeting(&path, &format!("({BRUSH}) AND housing_median_age > 28"))
    );

    doc.clear_selection();
    assert_eq!(
        charts_hold(&doc),
        rows_meeting(&path, "housing_median_age > 28"),
        "clear-selection took the grid's filter with the brush"
    );

    brush_hero(&mut doc);
    doc.apply_grid_view(RowsView::default());
    assert_eq!(
        charts_hold(&doc),
        rows_meeting(&path, BRUSH),
        "clearing the grid took the brush with it"
    );
}

/// A refused gesture, then a good pick under a grid filter: the page drawn is
/// new and the fault belonged to the last one.
#[test]
fn a_good_pick_under_a_grid_filter_clears_the_last_refusal() {
    let (_path, mut doc) = open();
    doc.apply_grid_view(view("housing_median_age > 28"));
    assert!(!doc.preview_shelf_column(0, ShelfChannel::X, "no_such_column", &[]));
    assert!(doc.chart_fault().is_some(), "the refusal left no fault");

    assert!(doc.set_plot_scale(0, PlotAxis::Y, ScaleType::Log));

    assert!(
        doc.chart_fault().is_none(),
        "the old refusal is still over the new page"
    );
}

/// The same with no grid filter: nothing is published after the rebuild, so
/// the rebuild itself must clear the refusal.
#[test]
fn a_good_pick_with_no_grid_filter_clears_the_last_refusal() {
    let (_path, mut doc) = open();
    assert!(!doc.preview_shelf_column(0, ShelfChannel::X, "no_such_column", &[]));
    assert!(doc.chart_fault().is_some(), "the refusal left no fault");

    assert!(doc.set_plot_scale(0, PlotAxis::Y, ScaleType::Log));

    assert!(
        doc.chart_fault().is_none(),
        "the old refusal is still over the new page"
    );
}
