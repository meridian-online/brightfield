//! **A column put on a generated tile's axis takes out what the tile wrote for the old column.**
//!
//! A generated tile writes its column's tick format, its title with the unit and, for a year, the
//! listed run of years. Each describes that column and no other: `$` on a count, `d` on a
//! fraction, `revenue (£bn)` over downloads, and a band pinned to years no value of the new column
//! matches, so every bar goes. The shelf takes each of them out in the edit that puts the column,
//! so one `u` puts them all back, and leaves a format the analyst typed where it is.
//!
//! The fixture is `data/typed_formats.csv`, its columns labelled through a column record that
//! nominates each label, so no FineType bundle is needed.

use std::path::PathBuf;

use brightfield_engine::ColumnProfile;
use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::column_record;
use brightfield_shell::data_file::{self, OpenOptions};
use brightfield_shell::design::Mode;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::ast::{PlotNode, SpecValue};
use brightfield_spec::edit::plot_at_path;
use brightfield_workbench::channel::ShelfChannel;

const TYPED_FORMATS: &str = include_str!("data/typed_formats.csv");

const LABELS: &[(&str, &str)] = &[
    ("revenue", "finance.currency.amount"),
    ("price_usd", "finance.currency.amount"),
    ("share", "representation.numeric.percentage"),
    ("yield", "finance.rate.yield"),
    ("year", "datetime.component.year"),
    ("downloads", "representation.numeric.integer_number"),
    ("day", "datetime.date.iso"),
];

fn scratch_config() {
    use std::sync::OnceLock;
    static CONFIG: OnceLock<PathBuf> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("bf-put-formats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch config folder");
        std::env::set_var(brightfield_shell::startup::CONFIG_DIR_VAR, &dir);
        dir
    });
}

/// The fixture in a folder of the calling test's own, with its labels nominated.
fn fixture() -> PathBuf {
    scratch_config();
    let thread = std::thread::current();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("shelf_column_formats")
        .join(thread.name().unwrap_or("main").replace("::", "-"));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join("typed_formats.csv");
    std::fs::write(&path, TYPED_FORMATS).expect("write fixture");
    let mut record = column_record::SourceRecord::default();
    for (column, label) in LABELS {
        record.fields.insert(
            (*column).to_string(),
            column_record::FieldRecord {
                label: Some((*label).to_string()),
                nominated: true,
                ..column_record::FieldRecord::default()
            },
        );
    }
    let package = column_record::flat_record_path(&path).expect("a record path");
    column_record::write_record(&package, data_file::SOURCE, &record).expect("write record");
    path
}

/// A window over the fixture's generated dashboard, and the table's profile the shelf checks a
/// column against.
fn window() -> (MeridianApp, Vec<ColumnProfile>) {
    let path = fixture();
    let options = OpenOptions {
        type_source: None,
        ..OpenOptions::default()
    };
    let boot = Boot::data_file_with(&path.to_string_lossy(), &options).expect("the file opens");
    let app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
    let table = app
        .protocol_model()
        .source()
        .expect("a data file's protocol")
        .profiles
        .clone();
    (app, table)
}

/// The page's plot number of the tile drawn for `column`.
fn tile_of(doc: &ChartDoc, column: &str) -> usize {
    doc.tile_columns()
        .iter()
        .position(|c| c.column == column)
        .unwrap_or_else(|| panic!("no tile for {column}"))
}

/// The plot numbered `plot` in the page the window draws now.
fn plot_now(doc: &ChartDoc, plot: usize) -> PlotNode {
    let path = &doc.composed.plots[plot].path;
    plot_at_path(doc.live_spec().expect("a live page"), path)
        .expect("the plot is in the spec")
        .clone()
}

fn attribute<'a>(plot: &'a PlotNode, key: &str) -> Option<&'a SpecValue> {
    plot.attributes.get(key)
}

fn text(value: &str) -> SpecValue {
    SpecValue::String(value.to_string())
}

#[test]
fn a_column_put_on_the_year_tile_drops_its_listed_years_and_draws_its_own_bars() {
    let (mut app, table) = window();
    let doc = app.chart_doc_mut();
    let year = tile_of(doc, "year");
    let before = plot_now(doc, year);
    assert!(
        matches!(attribute(&before, "xDomain"), Some(SpecValue::Array(years)) if years.len() > 2),
        "fixture check: the year tile lists its years"
    );

    assert!(doc.preview_shelf_column(year, ShelfChannel::X, "notes", &table));
    let after = plot_now(doc, year);
    assert_eq!(
        attribute(&after, "xDomain"),
        None,
        "the run of years goes with the year"
    );
    let Some(Scale::Band { categories, .. }) = doc.composed.plots[year].scales.get(Channel::X)
    else {
        panic!("the notes draw on a band");
    };
    assert!(
        categories.iter().any(|c| c.starts_with("note ")),
        "the band holds the notes, not the years: {categories:?}"
    );
}

#[test]
fn a_count_put_on_the_dollar_histogram_loses_the_dollar() {
    let (mut app, table) = window();
    let doc = app.chart_doc_mut();
    let price = tile_of(doc, "price_usd");
    assert_eq!(
        attribute(&plot_now(doc, price), "xTickFormat"),
        Some(&text("$,~f")),
        "fixture check: the price tile writes dollars"
    );
    assert!(doc.preview_shelf_column(price, ShelfChannel::X, "downloads", &table));
    let after = plot_now(doc, price);
    assert_eq!(attribute(&after, "xTickFormat"), None);
    assert_eq!(
        attribute(&after, "xLabel"),
        None,
        "the old column's name goes too"
    );
}

#[test]
fn a_fraction_put_on_the_year_tile_is_not_rounded_by_the_years_format() {
    let (mut app, table) = window();
    let doc = app.chart_doc_mut();
    let year = tile_of(doc, "year");
    assert_eq!(
        attribute(&plot_now(doc, year), "xTickFormat"),
        Some(&text("d"))
    );
    assert!(doc.preview_shelf_column(year, ShelfChannel::X, "share", &table));
    let after = plot_now(doc, year);
    assert_eq!(attribute(&after, "xTickFormat"), None);
    assert_eq!(attribute(&after, "xDomain"), None);
}

#[test]
fn a_column_put_on_the_scaled_tile_drops_the_unit_and_one_undo_puts_everything_back() {
    let (mut app, table) = window();
    let doc = app.chart_doc_mut();
    let revenue = tile_of(doc, "revenue");
    let before = plot_now(doc, revenue);
    assert_eq!(
        attribute(&before, "xLabel"),
        Some(&text("revenue (£bn)")),
        "fixture check: the scaled tile names its unit"
    );
    assert!(doc.keep_shelf_column(revenue, ShelfChannel::X, "downloads", &table));
    let after = plot_now(doc, revenue);
    assert_eq!(attribute(&after, "xLabel"), None);
    assert_eq!(attribute(&after, "xTickFormat"), None);

    assert!(doc.undo_shelf_edit().is_some(), "the edit is taken back");
    let undone = plot_now(doc, revenue);
    for key in ["xLabel", "xTickFormat", "xDomain"] {
        assert_eq!(
            attribute(&undone, key),
            attribute(&before, key),
            "one `u` puts {key} back"
        );
    }
}

#[test]
fn a_format_the_analyst_typed_stays_when_the_column_changes() {
    let (mut app, table) = window();
    let doc = app.chart_doc_mut();
    let price = tile_of(doc, "price_usd");
    let mut spec = doc.live_spec().expect("a live page").clone();
    let path = doc.composed.plots[price].path.clone();
    for (key, value) in [("xTickFormat", "~s"), ("xLabel", "price in dollars")] {
        brightfield_spec::edit::apply_for_fresh_load(
            &mut spec,
            &brightfield_spec::edit::ChartEdit::SetPlotAttribute {
                plot: brightfield_spec::analysis::ComponentPath(path.clone()),
                key: key.to_string(),
                value: text(value),
            },
        )
        .expect("the attribute is written");
    }
    let mut put = spec.clone();
    let edits = brightfield_shell::shelf_edit::put_column_over(
        &mut put,
        &brightfield_spec::analysis::ComponentPath(path.clone()),
        brightfield_spec::layout::PlotAxis::X,
        "downloads",
        &table,
        &brightfield_shell::shelf_edit::RuleWritten {
            tick_format: Some("$,~f".to_string()),
            titles: vec!["price_usd".to_string()],
        },
    )
    .expect("the column goes on x");
    assert!(!edits.is_empty());
    let plot = plot_at_path(&put, &path).expect("the plot");
    assert_eq!(attribute(plot, "xTickFormat"), Some(&text("~s")));
    assert_eq!(attribute(plot, "xLabel"), Some(&text("price in dollars")));
}
