//! **A column put on the map's x or y moves both its layers, and one that is
//! not a coordinate draws a dot plot. A column put on its colour paints the
//! highlighted points, and a legend follows it.**
//!
//! The tests here start from the dashboard the generator writes for a table
//! whose columns hold a coordinate pair, opened the way a data file is opened,
//! and edit its hero map through `shelf_edit::put_column` and
//! `shelf_edit::put_colour` with the table's own profile. The spec half reads
//! the edited AST; the page half loads a page from it the way the shell's tile
//! controls do, and sweeps a rectangle over it in a headless window.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use brightfield_engine::{ColumnProfile, ProfileOutcome, RowsAudience, SqlPredicate};
use brightfield_protocol::write_chart_edit;
use brightfield_render::layout::Margins;
use brightfield_render::title::TITLE_BAND;
use brightfield_render::VelloRenderer;
use brightfield_shell::chart_kinds;
use brightfield_shell::data_file::{self, OpenedFile};
use brightfield_shell::design::Mode;
use brightfield_shell::legend::{band_width, LegendSpec};
use brightfield_shell::pipeline::LiveDashboard;
use brightfield_shell::shelf_edit::{put_colour, put_column, ShelfRefusal};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::ast::{Component, LegendNode, PlotNode, Spec, SpecValue, ValueOrParamRef};
use brightfield_spec::edit::{self, plot_at_path, plot_at_path_mut, ChartEdit, RefuseReason};
use brightfield_spec::layout::{resolve_axis_titles, AxisTitle, PlotAxis};
use brightfield_spec::vocab::LegendChannel;
use brightfield_spec::{parse_spec, Format, MarkKind};
use brightfield_sql::ir::ScalarValue;

// ---------------------------------------------------------------------------
// The fixture
// ---------------------------------------------------------------------------

const LON: &str = "longitude";
const LAT: &str = "latitude";
/// The column the prototype put on the map's x first: quantitative, as the
/// longitude is, and not a coordinate.
const INCOME: &str = "median_income";
/// A fourth measure, so the tile that narrows under a sweep is not a redraw
/// of a column the swept plot binds.
const VALUE: &str = "house_value";

const ROWS: i64 = 24;

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "bf-shelf-edit-{name}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The table: a coordinate pair the generator finds by name, and two measures
/// that rise with the row, so a band swept across the middle of `median_income`
/// empties both ends of `house_value`'s histogram. The latitude is a V in the
/// row so that no measure is a copy of it.
fn csv() -> String {
    let mut out = format!("{LON},{LAT},{INCOME},{VALUE}\n");
    for row in 0..ROWS {
        let lon = -124.0 + row as f64 * 0.25;
        let lat = 32.0 + (row - 12).abs() as f64 * 0.5;
        let income = 1.0 + row as f64 * 0.5;
        let value = row * 10;
        let _ = writeln!(out, "{lon},{lat},{income},{value}");
    }
    out
}

/// The table opened as a data file is: profiled, a dashboard generated for
/// it, and that dashboard's page composed.
struct Opened {
    _dir: TempDir,
    file: OpenedFile,
    /// The table's profile, as the engine read it.
    table: Vec<ColumnProfile>,
    /// The spec the generator wrote.
    generated: Spec,
    /// The hero map's plot path.
    hero: ComponentPath,
}

fn open(name: &str) -> Opened {
    let dir = TempDir::new(name);
    let path = dir.0.join("housing.csv");
    std::fs::write(&path, csv()).expect("the fixture writes");
    let mut file = data_file::open(path.to_str().expect("utf-8 path"))
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));

    let drawn: Vec<(&str, &str)> = file
        .dashboard
        .tiles()
        .iter()
        .map(|t| (t.column(), t.kind().as_str()))
        .collect();
    assert_eq!(
        drawn,
        vec![
            (LON, chart_kinds::POINT_MAP.as_str()),
            (LON, chart_kinds::BINNED_HISTOGRAM.as_str()),
            (LAT, chart_kinds::BINNED_HISTOGRAM.as_str()),
            (INCOME, chart_kinds::BINNED_HISTOGRAM.as_str()),
            (VALUE, chart_kinds::BINNED_HISTOGRAM.as_str()),
        ],
        "the generator's tile choices for this table moved, so the mark \
         numbering the page tests read is no longer the map's two layers \
         and then two per histogram"
    );

    let table = file
        .live
        .coordinator()
        .session()
        .profile_sources()
        .into_iter()
        .find(|p| p.name == data_file::SOURCE)
        .map(|p| match p.outcome {
            ProfileOutcome::Profiled { columns, .. } => columns,
            other => panic!("the table did not profile: {other:?}"),
        })
        .expect("the opened file has a source to profile");

    let generated = file.live.spec().clone();
    let hero = file.composed.plots[0].path.clone();
    assert!(
        plot_at_path(&generated, &hero)
            .expect("the first composed plot is in the spec")
            .attributes
            .contains_key("projectionType"),
        "the first plot the generator placed is not the projected map"
    );
    Opened {
        _dir: dir,
        file,
        table,
        generated,
        hero: ComponentPath(hero),
    }
}

// ---------------------------------------------------------------------------
// Readings
// ---------------------------------------------------------------------------

fn plot<'a>(spec: &'a Spec, path: &ComponentPath) -> &'a PlotNode {
    plot_at_path(spec, &path.0).expect("the hero's path names a plot")
}

/// Each mark's kind and the column it binds `channel` to, in mark order.
fn layers(spec: &Spec, path: &ComponentPath, channel: &str) -> Vec<(MarkKind, String)> {
    plot(spec, path)
        .items
        .iter()
        .filter_map(|c| match c {
            Component::Mark(m) => Some((
                m.kind,
                match m.options.get(channel) {
                    Some(ValueOrParamRef::Value(SpecValue::String(s))) => s.clone(),
                    other => format!("{other:?}"),
                },
            )),
            _ => None,
        })
        .collect()
}

fn projection<'a>(spec: &'a Spec, path: &ComponentPath) -> Option<&'a SpecValue> {
    plot(spec, path).attributes.get("projectionType")
}

fn set(path: &ComponentPath, mark_ordinal: usize, channel: &str, column: &str) -> ChartEdit {
    ChartEdit::SetChannel {
        plot: path.clone(),
        mark_ordinal,
        channel: channel.to_string(),
        column: column.to_string(),
    }
}

fn scheme(path: &ComponentPath, name: &str) -> ChartEdit {
    ChartEdit::SetPlotAttribute {
        plot: path.clone(),
        key: "colorScheme".to_string(),
        value: SpecValue::String(name.to_string()),
    }
}

fn legend(path: &ComponentPath) -> ChartEdit {
    ChartEdit::AddColourLegend { plot: path.clone() }
}

/// The channel each legend item of the plot draws, in item order.
fn legend_items(spec: &Spec, path: &ComponentPath) -> Vec<LegendChannel> {
    plot(spec, path)
        .items
        .iter()
        .filter_map(|c| match c {
            Component::Legend(l) => Some(l.channel),
            _ => None,
        })
        .collect()
}

/// `spec` with a standalone colour legend in the column the hero sits in, after
/// the hero, that names a plot `names` by `for:`; the hero is called `hero`.
fn with_standalone_legend(spec: &mut Spec, hero: &ComponentPath, names: &str) {
    plot_at_path_mut(spec, &hero.0)
        .expect("the hero")
        .attributes
        .insert("name".to_string(), SpecValue::String("hero".to_string()));
    let Some(Component::HConcat(row)) = &mut spec.root else {
        panic!("the generated dashboard's root is not a row");
    };
    let Some(Component::VConcat(column)) = row.items.first_mut() else {
        panic!("the hero's column is not first in the row");
    };
    let mut standalone = LegendNode {
        channel: LegendChannel::Color,
        status: LegendChannel::Color.status(),
        options: Default::default(),
    };
    standalone.options.insert(
        "for".to_string(),
        ValueOrParamRef::Value(SpecValue::String(names.to_string())),
    );
    column.items.push(Component::Legend(standalone));
}

fn drop_projection(path: &ComponentPath) -> ChartEdit {
    ChartEdit::RemovePlotAttribute {
        plot: path.clone(),
        key: "projectionType".to_string(),
    }
}

// ---------------------------------------------------------------------------
// AC1 and AC5 — the spec: both layers move, the projection goes, and the list
// ---------------------------------------------------------------------------

/// **`median_income` on the map's x**: both dot layers read it, their y is
/// untouched, the plot has no `projectionType`, and the size the map was drawn
/// at is kept.
#[test]
fn a_column_put_on_the_maps_x_moves_both_layers_and_takes_the_projection_out() {
    let o = open("x-both-layers");
    let mut spec = o.generated.clone();

    put_column(&mut spec, &o.hero, PlotAxis::X, INCOME, &o.table)
        .expect("the table has the column");

    assert_eq!(
        layers(&spec, &o.hero, "x"),
        [
            (MarkKind::Dot, INCOME.to_string()),
            (MarkKind::Dot, INCOME.to_string())
        ],
        "both dot layers should read x: {INCOME}"
    );
    assert_eq!(
        layers(&spec, &o.hero, "y"),
        [
            (MarkKind::Dot, LAT.to_string()),
            (MarkKind::Dot, LAT.to_string())
        ],
        "y moved on a gesture that put a column on x"
    );
    assert_eq!(
        projection(&spec, &o.hero),
        None,
        "the plot is still projected"
    );
    assert_eq!(
        plot(&spec, &o.hero).attributes.get("width"),
        plot(&o.generated, &o.hero).attributes.get("width"),
        "the map's declared width went with the projection"
    );
}

/// **The edit is the list of edits applied, in order, and replaying it on the
/// generated spec gives the edited one.** Save writes that list into the
/// file's text one edit at a time, so a list that differs from what was
/// applied writes a file that differs from the page.
#[test]
fn the_edit_is_the_list_of_chart_edits_applied_and_replays_to_the_same_spec() {
    let o = open("x-list");
    let mut spec = o.generated.clone();

    let edits = put_column(&mut spec, &o.hero, PlotAxis::X, INCOME, &o.table)
        .expect("the table has the column");

    assert_eq!(
        edits,
        [
            set(&o.hero, 0, "x", INCOME),
            set(&o.hero, 1, "x", INCOME),
            drop_projection(&o.hero),
        ],
        "the list should be the ghost's x, the subset's x, then the projection"
    );
    let mut replayed = o.generated.clone();
    for e in &edits {
        edit::apply_for_fresh_load(&mut replayed, e).expect("each edit has its target");
    }
    assert_eq!(
        replayed, spec,
        "the listed edits, replayed on the generated spec, give another spec"
    );
}

// ---------------------------------------------------------------------------
// AC2 — the pair put back is the generator's map
// ---------------------------------------------------------------------------

/// **`longitude` back on x, with `latitude` on y, is the generator's spec.**
/// The projection comes back because the plot comes to hold the pair again.
#[test]
fn putting_longitude_back_on_x_gives_the_spec_the_generator_wrote() {
    let o = open("x-back");
    let mut spec = o.generated.clone();

    put_column(&mut spec, &o.hero, PlotAxis::X, INCOME, &o.table)
        .expect("the table has the column");
    let edits = put_column(&mut spec, &o.hero, PlotAxis::X, LON, &o.table)
        .expect("the table has the column");

    assert_eq!(
        spec, o.generated,
        "the pair put back is not the map the generator wrote"
    );
    assert_eq!(
        edits.last(),
        Some(&ChartEdit::SetPlotAttribute {
            plot: o.hero.clone(),
            key: "projectionType".to_string(),
            value: SpecValue::String(chart_kinds::POINT_MAP_PROJECTION.to_string()),
        }),
        "the projection came back by some edit other than the one Save writes"
    );
}

// ---------------------------------------------------------------------------
// AC3 — y moves both layers the same way
// ---------------------------------------------------------------------------

/// **A column on y moves both layers' y, and one that is not the pair's
/// latitude takes the projection out** — `median_income`, and `longitude`,
/// which is a coordinate but not the latitude.
#[test]
fn a_column_put_on_the_maps_y_moves_both_layers_and_takes_the_projection_out() {
    let o = open("y-both-layers");
    for column in [INCOME, LON] {
        let mut spec = o.generated.clone();

        let edits = put_column(&mut spec, &o.hero, PlotAxis::Y, column, &o.table)
            .expect("the table has the column");

        assert_eq!(
            layers(&spec, &o.hero, "y"),
            [
                (MarkKind::Dot, column.to_string()),
                (MarkKind::Dot, column.to_string())
            ],
            "both dot layers should read y: {column}"
        );
        assert_eq!(
            layers(&spec, &o.hero, "x"),
            [
                (MarkKind::Dot, LON.to_string()),
                (MarkKind::Dot, LON.to_string())
            ],
            "x moved on a gesture that put {column} on y"
        );
        assert_eq!(
            projection(&spec, &o.hero),
            None,
            "{column} on y left the plot projected"
        );
        assert_eq!(
            edits,
            [
                set(&o.hero, 0, "y", column),
                set(&o.hero, 1, "y", column),
                drop_projection(&o.hero),
            ]
        );
    }
}

/// **The latitude put on y, where it already is, is no edit**: the map keeps
/// its projection and the spec is equal.
#[test]
fn the_latitude_put_on_the_maps_y_leaves_the_map_as_it_is() {
    let o = open("y-latitude");
    let mut spec = o.generated.clone();

    let edits = put_column(&mut spec, &o.hero, PlotAxis::Y, LAT, &o.table)
        .expect("the table has the column");

    assert_eq!(edits, [], "a column put where it already is made edits");
    assert_eq!(spec, o.generated);
}

// ---------------------------------------------------------------------------
// AC4 — a column the table does not have
// ---------------------------------------------------------------------------

/// **A column the table does not have is refused in words that name it**, and
/// the spec is left as it was.
#[test]
fn a_column_the_table_does_not_have_is_refused_by_name() {
    let o = open("no-such-column");
    let mut spec = o.generated.clone();

    let refused = put_column(&mut spec, &o.hero, PlotAxis::X, "median_incme", &o.table)
        .expect_err("the table has no column median_incme");

    assert_eq!(
        refused,
        ShelfRefusal::NoSuchColumn("median_incme".to_string())
    );
    assert!(
        refused.to_string().contains("median_incme"),
        "the refusal does not name the column: {refused}"
    );
    assert_eq!(spec, o.generated, "a refused column changed the spec");
}

// ---------------------------------------------------------------------------
// AC6 — the reload-from-disk path is unchanged
// ---------------------------------------------------------------------------

/// **The reload-from-disk path still refuses the same change.** `edit::apply`
/// is that path's entry, and the first edit of the list changes the map's
/// derived x title from `longitude` to `median_income`.
#[test]
fn the_reload_from_disk_path_still_refuses_the_edit_that_changes_an_axis_title() {
    let o = open("reload-path");
    let mut spec = o.generated.clone();

    assert_eq!(
        edit::apply(&mut spec, &set(&o.hero, 0, "x", INCOME)),
        Err(RefuseReason::WouldChangeAxisTitle),
        "the reload-from-disk path let a derived axis title change through"
    );
    assert_eq!(spec, o.generated, "a refused edit changed the spec");
}

// ---------------------------------------------------------------------------
// AC1 — the page: a dot plot of median_income, and a sweep that narrows
// ---------------------------------------------------------------------------

fn screen() -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0))
}

fn frame(app: &mut MeridianApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput {
        screen_rect: Some(screen()),
        events,
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| app.draw(ui));
}

/// A pointer position at `(fx, fy)` of plot `plot`'s data area.
fn at(app: &MeridianApp, plot: usize, fx: f64, fy: f64) -> egui::Pos2 {
    let doc = app.chart_doc();
    let raster = doc
        .raster_rect
        .expect("a settled frame presented the raster");
    let p = &doc.composed.plots[plot];
    let l = &p.layout;
    let x = p.rect.x + l.plot_x_start() + (l.plot_x_end() - l.plot_x_start()) * fx;
    let y = p.rect.y + l.plot_y_start() + (l.plot_y_end() - l.plot_y_start()) * fy;
    egui::pos2(raster.min.x + x as f32, raster.min.y + y as f32)
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// Press at one corner, move to the opposite one, release.
fn sweep(
    app: &mut MeridianApp,
    ctx: &egui::Context,
    plot: usize,
    from: (f64, f64),
    to: (f64, f64),
) {
    let start = at(app, plot, from.0, from.1);
    frame(
        app,
        ctx,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    let end = at(app, plot, to.0, to.1);
    frame(app, ctx, vec![egui::Event::PointerMoved(end)]);
    frame(app, ctx, vec![button(end, false)]);
    frame(app, ctx, Vec::new());
}

/// The interval clauses the document's selections hold, as `(column, lo, hi)`.
fn held(app: &MeridianApp) -> Vec<(String, f64, f64)> {
    fn intervals(predicate: &SqlPredicate) -> Vec<(String, f64, f64)> {
        match predicate {
            SqlPredicate::Interval { column, lo, hi, .. } => match (lo, hi) {
                (ScalarValue::Float(lo), ScalarValue::Float(hi)) => {
                    vec![(column.trim_matches('"').to_string(), *lo, *hi)]
                }
                _ => Vec::new(),
            },
            SqlPredicate::And(parts) | SqlPredicate::Or(parts) => {
                parts.iter().flat_map(intervals).collect()
            }
            _ => Vec::new(),
        }
    }
    app.chart_doc()
        .live_dashboard()
        .expect("the page has a live session")
        .selection_clauses()
        .iter()
        .flat_map(|(_, p)| intervals(p))
        .collect()
}

/// How many rows the step behind mark `mark` returns under the current
/// selection.
fn step_rows(app: &mut MeridianApp, mark: usize) -> u64 {
    app.chart_doc_mut()
        .live_coordinator()
        .expect("the page has a live session")
        .session()
        .step_rows_count(mark, RowsAudience::Plot)
        .expect("the step counts")
}

/// A window over a page loaded from `spec` the way the shell's tile controls
/// load one: `LiveDashboard::load` over the edited spec, from the base the
/// first load resolved its sources against.
fn window_over(spec: Spec, base: Option<&Path>, ctx: &egui::Context) -> MeridianApp {
    let mut live = LiveDashboard::load(spec, base).expect("the edited spec loads");
    let composed = live.present().expect("the edited page presents");
    let mut boot = Boot::charts(composed);
    boot.live = Some(live);
    let mut app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
    frame(&mut app, ctx, Vec::new());
    frame(&mut app, ctx, Vec::new());
    app
}

/// **A page loaded from the edited spec draws a dot plot of `median_income`
/// against `latitude`, with its x axis titled `median_income`, and a rectangle
/// swept over it narrows the other tiles.**
///
/// The page is read at the composition: the hero's two marks are dots, its
/// scales carry no projection, its x domain lies over `median_income`'s values
/// and not `longitude`'s, and its bottom margin holds a title band. The
/// generated map reserves that band too, for a derived title its projected
/// frame never draws, so the band says an x title is laid out and not which
/// one. The title's text is the spec's derived one, which the render resolves
/// from the first layer's x column.
///
/// Marks are numbered across the page: the hero's ghost (0) and subset (1),
/// then each histogram's ghost and subset in tile order, which puts
/// `house_value`'s subset at 9.
#[test]
fn a_page_loaded_from_it_draws_a_dot_plot_and_a_swept_rectangle_narrows_the_other_tiles() {
    let o = open("page");
    let mut spec = o.generated.clone();
    put_column(&mut spec, &o.hero, PlotAxis::X, INCOME, &o.table)
        .expect("the table has the column");

    let titles = resolve_axis_titles(plot(&spec, &o.hero));
    assert_eq!(
        titles.x,
        AxisTitle::Derive,
        "the hero's x title is no longer derived"
    );
    assert_eq!(
        layers(&spec, &o.hero, "x").first().map(|(_, c)| c.as_str()),
        Some(INCOME),
        "the derived x title reads the first layer's column"
    );

    let base = o.file.live.base_dir().map(Path::to_path_buf);
    let ctx = egui::Context::default();
    let mut app = window_over(spec, base.as_deref(), &ctx);

    let hero = &app.chart_doc().composed.plots[0];
    assert_eq!(
        hero.path, o.hero.0,
        "the hero is no longer the first plot placed"
    );
    assert_eq!(
        hero.marks,
        [MarkKind::Dot, MarkKind::Dot],
        "the hero is not two dot layers"
    );
    assert!(
        hero.scales.projection().is_none(),
        "the page still draws the hero through a projection"
    );
    let x = hero
        .scales
        .get(brightfield_render::channel::Channel::X)
        .expect("the dot plot has an x scale");
    let (lo, hi) = (
        x.domain_min().expect("a numeric x domain"),
        x.domain_max().expect("a numeric x domain"),
    );
    assert!(
        lo >= 0.0 && hi <= 13.5 && lo < hi,
        "the x axis spans [{lo}, {hi}], which is not {INCOME}'s 1..=12.5 — \
         longitude runs -124..-118"
    );
    let bottom = hero.layout.margins.bottom;
    assert!(
        bottom >= Margins::default().bottom + TITLE_BAND,
        "the dot plot's bottom margin is {bottom}: no band was reserved for \
         an x title"
    );

    let (hero_rest, value_ghost_rest, value_rest) = (
        step_rows(&mut app, 1),
        step_rows(&mut app, 8),
        step_rows(&mut app, 9),
    );
    assert_eq!(
        hero_rest, ROWS as u64,
        "the hero's subset draws every row at rest"
    );
    assert!(
        value_rest > 2,
        "{VALUE}'s histogram fills {value_rest} bin(s) at rest"
    );

    // A band across the middle of median_income and most of latitude.
    sweep(&mut app, &ctx, 0, (0.30, 0.05), (0.60, 0.95));

    let clauses = held(&app);
    let mut columns: Vec<&str> = clauses.iter().map(|(c, _, _)| c.as_str()).collect();
    columns.sort_unstable();
    assert_eq!(
        columns,
        [LAT, INCOME],
        "the sweep committed {clauses:?}; the rectangle should constrain the \
         two columns the dot plot draws"
    );
    let (hero_after, value_ghost_after, value_after) = (
        step_rows(&mut app, 1),
        step_rows(&mut app, 8),
        step_rows(&mut app, 9),
    );
    assert!(
        value_after < value_rest,
        "{VALUE}'s histogram fills {value_after} bin(s) after the sweep and \
         {value_rest} before: the rectangle narrowed nothing"
    );
    assert_eq!(
        value_ghost_after, value_ghost_rest,
        "an unfiltered layer narrowed"
    );
    assert_eq!(
        hero_after, hero_rest,
        "the hero narrowed on its own selection"
    );
}

// ---------------------------------------------------------------------------
// Colour — the spec: the highlighted layer takes the column, the ghost keeps
// its ink
// ---------------------------------------------------------------------------

/// The highlighted layer's ordinal on the hero: the second of the map's two.
const HIGHLIGHTED: usize = 1;

/// The ink the generated map's ghost layer is drawn in, which is the fill it
/// carries before any edit.
fn ghost_ink(o: &Opened) -> String {
    let ink = layers(&o.generated, &o.hero, "fill")[0].1.clone();
    assert!(
        ink.starts_with('#'),
        "the generated ghost layer's fill is {ink}, not a literal ink"
    );
    ink
}

/// **`house_value` on the map's colour**: the highlighted layer reads
/// `fill: house_value`, the ghost layer keeps its literal ink, and x, y and the
/// plot's attributes stay as they were but for the `colorScheme` a number
/// column names — the map is still projected at the size it was drawn at.
#[test]
fn a_column_put_on_the_maps_colour_paints_the_highlighted_layer_and_keeps_the_ghost_ink() {
    let o = open("colour-highlighted");
    let mut spec = o.generated.clone();

    put_colour(&mut spec, &o.hero, VALUE, &o.table).expect("the table has the column");

    assert_eq!(
        layers(&spec, &o.hero, "fill"),
        [
            (MarkKind::Dot, ghost_ink(&o)),
            (MarkKind::Dot, VALUE.to_string())
        ],
        "the highlighted layer should read fill: {VALUE} and the ghost keep its ink"
    );
    for channel in ["x", "y"] {
        assert_eq!(
            layers(&spec, &o.hero, channel),
            layers(&o.generated, &o.hero, channel),
            "a gesture that put a column on colour moved {channel}"
        );
    }
    let mut attributes = plot(&spec, &o.hero).attributes.clone();
    assert_eq!(
        attributes.shift_remove("colorScheme"),
        Some(SpecValue::String("viridis".to_string())),
        "a number column put on colour should name the scheme the dot draws it in"
    );
    assert_eq!(
        attributes,
        plot(&o.generated, &o.hero).attributes,
        "a gesture that put a column on colour changed the plot's attributes \
         but for its colorScheme, which are the map's projection and its size"
    );
}

/// **The edit is the list of edits applied, and replaying it on the generated
/// spec gives the edited one** — the highlighted layer's fill, then the scheme
/// a number column names, then the legend, as x's and y's lists are the marks
/// they moved.
#[test]
fn the_colour_edit_is_the_list_of_chart_edits_applied_and_replays_to_the_same_spec() {
    let o = open("colour-list");
    let mut spec = o.generated.clone();

    let edits = put_colour(&mut spec, &o.hero, VALUE, &o.table).expect("the table has the column");

    assert_eq!(
        edits,
        [
            set(&o.hero, HIGHLIGHTED, "fill", VALUE),
            scheme(&o.hero, "viridis"),
            legend(&o.hero)
        ],
        "the list should be the highlighted layer's fill, the scheme and then the legend"
    );
    let mut replayed = o.generated.clone();
    for e in &edits {
        edit::apply_for_fresh_load(&mut replayed, e).expect("each edit has its target");
    }
    assert_eq!(
        replayed, spec,
        "the listed edits, replayed on the generated spec, give another spec"
    );
}

/// **Another column put on colour replaces the first**, on the highlighted
/// layer alone, and the ghost keeps its ink. The same column put again is no
/// edit.
#[test]
fn another_column_put_on_the_maps_colour_replaces_the_first() {
    let o = open("colour-replaced");
    let mut spec = o.generated.clone();
    put_colour(&mut spec, &o.hero, VALUE, &o.table).expect("the table has the column");

    let edits = put_colour(&mut spec, &o.hero, INCOME, &o.table).expect("the table has the column");

    assert_eq!(
        edits,
        [set(&o.hero, HIGHLIGHTED, "fill", INCOME)],
        "replacing the colour should be one edit, on the highlighted layer"
    );
    assert_eq!(
        layers(&spec, &o.hero, "fill"),
        [
            (MarkKind::Dot, ghost_ink(&o)),
            (MarkKind::Dot, INCOME.to_string())
        ],
        "the second column did not replace the first"
    );

    let before = spec.clone();
    let again = put_colour(&mut spec, &o.hero, INCOME, &o.table).expect("the table has the column");
    assert_eq!(again, [], "a column put where it already is made edits");
    assert_eq!(spec, before);
}

/// **A plot with no layer reading through a selection takes the colour on its
/// first mark**, as x and y take a column when no mark binds them.
#[test]
fn a_plot_with_no_selected_layer_takes_the_colour_on_its_first_mark() {
    let o = open("colour-first-mark");
    let mut spec = o.generated.clone();
    let path = o.hero.0.clone();
    let hero = brightfield_spec::edit::plot_at_path_mut(&mut spec, &path).expect("the hero");
    for c in &mut hero.items {
        if let Component::Mark(m) = c {
            if let Some(brightfield_spec::ast::MarkData::From { filter_by, .. }) = &mut m.data {
                *filter_by = None;
            }
        }
    }

    let edits = put_colour(&mut spec, &o.hero, VALUE, &o.table).expect("the table has the column");

    assert_eq!(
        edits,
        [
            set(&o.hero, 0, "fill", VALUE),
            scheme(&o.hero, "viridis"),
            legend(&o.hero)
        ]
    );
}

/// **A column put on a plot's colour ends with the edit that puts a colour
/// legend among the plot's items**, after the other edits, and the spec
/// afterwards holds that legend as the plot's last item, once.
#[test]
fn a_column_put_on_the_maps_colour_ends_with_a_colour_legend_as_the_plots_last_item() {
    let o = open("colour-legend-item");
    let mut spec = o.generated.clone();
    assert_eq!(
        legend_items(&spec, &o.hero),
        [],
        "the generated map already holds a legend"
    );

    let edits = put_colour(&mut spec, &o.hero, VALUE, &o.table).expect("the table has the column");

    assert_eq!(
        edits.last(),
        Some(&legend(&o.hero)),
        "the colour edit does not end with the legend: {edits:?}"
    );
    assert_eq!(
        edits
            .iter()
            .filter(|e| matches!(e, ChartEdit::AddColourLegend { .. }))
            .count(),
        1
    );
    assert_eq!(legend_items(&spec, &o.hero), [LegendChannel::Color]);
    assert!(
        matches!(
            plot(&spec, &o.hero).items.last(),
            Some(Component::Legend(_))
        ),
        "the legend is not the plot's last item: {:?}",
        plot(&spec, &o.hero).items.last()
    );
}

/// **A plot with a colour legend drawn for it already is given no second**: not
/// when its items hold one, as they do after an earlier colour, and not when a
/// standalone one names it with `for:`. A standalone legend that names another
/// plot draws nothing for this one, so the legend is written.
#[test]
fn a_plot_with_a_colour_legend_drawn_for_it_already_is_given_no_second() {
    let o = open("colour-legend-covered");
    let has_legend_edit = |edits: &[ChartEdit]| {
        edits
            .iter()
            .any(|e| matches!(e, ChartEdit::AddColourLegend { .. }))
    };

    let mut in_items = o.generated.clone();
    put_colour(&mut in_items, &o.hero, VALUE, &o.table).expect("the table has the column");
    let edits =
        put_colour(&mut in_items, &o.hero, INCOME, &o.table).expect("the table has the column");
    assert!(!has_legend_edit(&edits), "a second legend edit: {edits:?}");
    assert_eq!(legend_items(&in_items, &o.hero), [LegendChannel::Color]);

    let mut named = o.generated.clone();
    with_standalone_legend(&mut named, &o.hero, "hero");
    let edits =
        put_colour(&mut named, &o.hero, INCOME, &o.table).expect("the table has the column");
    assert!(
        !has_legend_edit(&edits),
        "a legend edit over a standalone legend that names the plot: {edits:?}"
    );
    assert_eq!(
        legend_items(&named, &o.hero),
        [],
        "the plot was given a legend beside the standalone one"
    );

    let mut elsewhere = o.generated.clone();
    with_standalone_legend(&mut elsewhere, &o.hero, "another");
    let edits =
        put_colour(&mut elsewhere, &o.hero, INCOME, &o.table).expect("the table has the column");
    assert_eq!(
        edits.last(),
        Some(&legend(&o.hero)),
        "a standalone legend for another plot stopped this one's legend"
    );
}

/// **A column the table does not have is refused in words that name it**, and
/// the spec is left as it was.
#[test]
fn a_colour_the_table_does_not_have_is_refused_by_name() {
    let o = open("colour-no-such-column");
    let mut spec = o.generated.clone();

    let refused = put_colour(&mut spec, &o.hero, "house_valu", &o.table)
        .expect_err("the table has no column house_valu");

    assert_eq!(
        refused,
        ShelfRefusal::NoSuchColumn("house_valu".to_string())
    );
    assert!(
        refused.to_string().contains("house_valu"),
        "the refusal does not name the column: {refused}"
    );
    assert_eq!(spec, o.generated, "a refused column changed the spec");
}

// ---------------------------------------------------------------------------
// Colour — the page: the legend follows the column, and the points wear it
// ---------------------------------------------------------------------------

/// The legend the page draws for the hero: the one its scales call for, when
/// the hero's file puts a legend on it, which is what the page reserves a band
/// for. Read here and not the scale's domain: a page whose fill scale is a
/// `Linear` over the column has a domain and draws no legend, and a test that
/// read the domain passed on it.
fn hero_legend(app: &MeridianApp) -> Option<LegendSpec> {
    LegendSpec::of_plot(&app.chart_doc().composed.plots[0])
}

/// **A page loaded from the edited spec draws a sequential legend for the colour
/// column beside the plot, and a column put on colour afterwards moves the
/// legend's ends.** The legend spans the column, the band the page reserved for
/// it holds it, and the band is clear of the raster the map is drawn on.
#[test]
fn a_column_put_on_the_maps_colour_draws_a_legend_beside_the_plot_and_a_replaced_colour_moves_it() {
    let o = open("colour-legend");
    let ctx = egui::Context::default();
    let base = o.file.live.base_dir().map(Path::to_path_buf);
    let page = |columns: &[&str]| {
        let mut spec = o.generated.clone();
        for column in columns {
            put_colour(&mut spec, &o.hero, column, &o.table).expect("the table has the column");
        }
        window_over(spec, base.as_deref(), &ctx)
    };
    let beside_the_plot = |app: &MeridianApp| {
        let doc = app.chart_doc();
        let (legend, raster) = (
            doc.legend_rect.expect("the page recorded no legend band"),
            doc.raster_rect.expect("the page recorded no raster"),
        );
        assert!(
            band_width(&doc.composed) > 0.0,
            "a legend was derived and no band was reserved for it"
        );
        assert!(
            !legend.intersects(raster),
            "the legend band {legend:?} overlaps the raster {raster:?}: a legend is on the data"
        );
    };

    let plain = page(&[]);
    assert_eq!(
        hero_legend(&plain),
        None,
        "the generated map already draws a legend"
    );
    assert_eq!(
        plain.chart_doc().legend_rect,
        None,
        "the generated map already reserves a legend band"
    );

    let by_value = page(&[VALUE]);
    let Some(LegendSpec::Sequential { min, max, stops }) = hero_legend(&by_value) else {
        panic!(
            "{VALUE} on colour drew no sequential legend; the fill scale is {:?}",
            by_value.chart_doc().composed.plots[0]
                .scales
                .get(brightfield_render::channel::Channel::Fill)
        );
    };
    assert!(
        min <= 0.0 && max >= 230.0,
        "the legend spans [{min}, {max}], which does not span {VALUE}'s 0..=230"
    );
    assert!(stops.len() >= 2, "a ramp needs two stops to be a gradient");
    beside_the_plot(&by_value);

    let by_income = page(&[VALUE, INCOME]);
    let Some(LegendSpec::Sequential { min, max, .. }) = hero_legend(&by_income) else {
        panic!("{INCOME} put on colour after {VALUE} drew no sequential legend");
    };
    assert!(
        min <= 1.0 && (12.5..13.5).contains(&max),
        "the legend spans [{min}, {max}] after {INCOME} replaced {VALUE}; it should span \
         {INCOME}'s 1..=12.5 and not stay on {VALUE}'s 0..=230"
    );
    beside_the_plot(&by_income);
}

/// **A page loaded from a chart that holds the legend item draws the legend,
/// and the same chart with the item taken out draws none**: the legend over the
/// column's range in a band clear of the raster with the item, and with it
/// gone no legend and no band, the raster wider by the band's width.
#[test]
fn a_page_loaded_from_a_chart_holding_the_legend_item_draws_it_and_one_without_the_item_draws_none()
{
    let o = open("colour-legend-page");
    let ctx = egui::Context::default();
    let base = o.file.live.base_dir().map(Path::to_path_buf);
    let mut with = o.generated.clone();
    put_colour(&mut with, &o.hero, VALUE, &o.table).expect("the table has the column");
    let mut without = with.clone();
    let taken = plot_at_path_mut(&mut without, &o.hero.0)
        .expect("the hero")
        .items
        .pop();
    assert!(
        matches!(taken, Some(Component::Legend(_))),
        "the last item of the edited hero is {taken:?}, not the legend"
    );

    let with = window_over(with, base.as_deref(), &ctx);
    let without = window_over(without, base.as_deref(), &ctx);

    assert!(
        matches!(hero_legend(&with), Some(LegendSpec::Sequential { .. })),
        "the chart holding the item drew no sequential legend: {:?}",
        hero_legend(&with)
    );
    let band = band_width(&with.chart_doc().composed);
    assert!(band > 0.0, "the chart holding the item reserved no band");
    assert!(
        with.chart_doc().legend_rect.is_some(),
        "the chart holding the item recorded no legend band"
    );

    assert_eq!(
        hero_legend(&without),
        None,
        "the chart with the item taken out drew a legend"
    );
    assert_eq!(
        band_width(&without.chart_doc().composed),
        0.0,
        "the chart with the item taken out reserved a band"
    );
    assert_eq!(
        without.chart_doc().legend_rect,
        None,
        "the chart with the item taken out recorded a legend band"
    );
    // The colour is still on the plot: it is the legend that went, not the
    // scale, so the page is the same picture with the room the band held.
    assert!(
        LegendSpec::from_scales(&without.chart_doc().composed.plots[0].scales).is_some(),
        "taking the item out took the colour scale with it"
    );
    let (r_with, r_without) = (
        with.chart_doc().raster_rect.expect("a raster"),
        without.chart_doc().raster_rect.expect("a raster"),
    );
    assert!(
        r_without.width() >= r_with.width() + band - 1.0,
        "the plot did not take the width the band held: {} with the item, {} without, \
         band {band}",
        r_with.width(),
        r_without.width()
    );
}

/// **A chart coloured from the shelf draws its legend after the edit, and again
/// after the edit is written into the chart's text and the text read back.** The
/// edit is the shelf's, the text is the one the generator wrote with each edit
/// written into it the way Save writes it, and the page the text reads back as
/// is loaded the way a reopened file is.
#[test]
fn a_chart_coloured_from_the_shelf_draws_its_legend_after_the_edit_and_after_the_text_is_read_back()
{
    let o = open("colour-legend-text");
    let ctx = egui::Context::default();
    let base = o.file.live.base_dir().map(Path::to_path_buf);

    let mut edited = o.generated.clone();
    let edits =
        put_colour(&mut edited, &o.hero, VALUE, &o.table).expect("the table has the column");
    let after_the_edit = window_over(edited, base.as_deref(), &ctx);

    let generated_text = std::fs::read_to_string(
        o.file
            .spec_file
            .as_ref()
            .expect("the generator wrote its text to a scratch file"),
    )
    .expect("the generated text reads");
    let written = edits.iter().fold(generated_text, |text, edit| {
        write_chart_edit(&text, edit).unwrap_or_else(|e| panic!("{edit:?} is written: {e}"))
    });
    assert!(
        written.lines().any(|l| l.trim() == "- legend: color"),
        "the text holds no legend item after the edit:\n{written}"
    );
    let read_back = parse_spec(&written, Format::Yaml)
        .expect("the written text parses")
        .spec;
    let after_the_text = window_over(read_back, base.as_deref(), &ctx);

    for (when, page) in [
        ("after the edit", &after_the_edit),
        ("after the text is read back", &after_the_text),
    ] {
        assert!(
            matches!(hero_legend(page), Some(LegendSpec::Sequential { .. })),
            "{when}: the chart coloured by {VALUE} drew no sequential legend: {:?}",
            hero_legend(page)
        );
        assert!(
            band_width(&page.chart_doc().composed) > 0.0,
            "{when}: no band was reserved for the legend"
        );
        assert!(
            page.chart_doc().legend_rect.is_some(),
            "{when}: no legend band was recorded"
        );
    }
    assert_eq!(
        hero_legend(&after_the_edit),
        hero_legend(&after_the_text),
        "the legend moved when the chart was written and read back"
    );
}

/// The hero's tile that draws `median_income`'s histogram, among the plots the
/// page composes: the map is 0, then `longitude`'s, `latitude`'s and this one.
const INCOME_HISTOGRAM: usize = 3;

/// How far a pixel channel may sit from an ink and still be that ink: the
/// anti-aliased edge of a dot is a blend and is not counted.
const INK_TOLERANCE: f32 = 6.0;

/// `#rrggbb` as three channels in 0..=255.
fn rgb(hex: &str) -> [f32; 3] {
    let byte = |i: usize| f32::from(u8::from_str_radix(&hex[i..i + 2], 16).expect("a hex byte"));
    [byte(1), byte(3), byte(5)]
}

/// The colour a ramp's control points give at `t` along it, in 0..=255.
fn ramp_at(stops: &[[f32; 4]], t: f32) -> [f32; 3] {
    let last = stops.len() - 1;
    let at = t * last as f32;
    let i = (at.floor() as usize).min(last - 1);
    let u = at - i as f32;
    let channel = |c: usize| (stops[i][c] + (stops[i + 1][c] - stops[i][c]) * u) * 255.0;
    [channel(0), channel(1), channel(2)]
}

fn is_ink(pixel: [f32; 3], ink: [f32; 3]) -> bool {
    (0..3).all(|c| (pixel[c] - ink[c]).abs() <= INK_TOLERANCE)
}

/// The pixels of the hero's data area as the page presents them, rendered off
/// the composed scene through the renderer the window and the PNG export use.
fn hero_pixels(app: &MeridianApp) -> Vec<[f32; 3]> {
    let doc = app.chart_doc();
    let (w, h) = (doc.composed.width, doc.composed.height);
    let buffer = VelloRenderer::new()
        .lock()
        .expect("renderer poisoned")
        .render_to_pixels(&doc.composed.scene, w, h);
    let hero = &doc.composed.plots[0];
    let (x0, x1) = (
        (hero.rect.x + hero.layout.plot_x_start()) as usize,
        (hero.rect.x + hero.layout.plot_x_end()) as usize,
    );
    let (y0, y1) = (
        (hero.rect.y + hero.layout.plot_y_start()) as usize,
        (hero.rect.y + hero.layout.plot_y_end()) as usize,
    );
    let mut out = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * w as usize + x) * 4;
            out.push([
                f32::from(buffer[i]),
                f32::from(buffer[i + 1]),
                f32::from(buffer[i + 2]),
            ]);
        }
    }
    out
}

/// **The highlighted points are painted by the column's ramp and the ghost
/// points keep their ink.** At rest the highlighted layer covers the ghost, so
/// the ghost is seen where a range swept on another tile narrows the highlighted
/// layer: the rows outside the range are drawn by the ghost layer alone. The
/// page is read in pixels, and the assertions are made in colour: a spec that
/// names the column and a scale that holds it were both true of the page whose
/// picture the column did not change.
#[test]
fn the_highlighted_points_wear_the_ramp_and_the_ghost_points_keep_their_ink() {
    let o = open("colour-pixels");
    let ctx = egui::Context::default();
    let base = o.file.live.base_dir().map(Path::to_path_buf);
    let swept = |colour: Option<&str>| {
        let mut spec = o.generated.clone();
        if let Some(column) = colour {
            put_colour(&mut spec, &o.hero, column, &o.table).expect("the table has the column");
        }
        let mut app = window_over(spec, base.as_deref(), &ctx);
        sweep(&mut app, &ctx, INCOME_HISTOGRAM, (0.0, 0.05), (0.5, 0.95));
        let kept = step_rows(&mut app, HIGHLIGHTED);
        assert!(
            0 < kept && kept < ROWS as u64,
            "the range swept on {INCOME}'s histogram left {kept} of {ROWS} rows in the highlighted layer"
        );
        app
    };
    let ghost = rgb(&ghost_ink(&o));

    let plain = swept(None);
    let coloured = swept(Some(VALUE));
    let Some(LegendSpec::Sequential { stops, .. }) = hero_legend(&coloured) else {
        panic!("{VALUE} on colour drew no sequential legend, so there is no ramp to read");
    };
    let ramp: Vec<[f32; 3]> = (0..=100)
        .map(|k| ramp_at(&stops, k as f32 / 100.0))
        .collect();
    // Which of the hundred-odd samples along the ramp a pixel is, if any.
    let along = |pixel: [f32; 3]| ramp.iter().position(|&c| is_ink(pixel, c));

    let (plain_picture, picture) = (hero_pixels(&plain), hero_pixels(&coloured));
    let wearing = |picture: &[[f32; 3]]| picture.iter().filter(|&&p| along(p).is_some()).count();
    let ghosts = |picture: &[[f32; 3]]| picture.iter().filter(|&&p| is_ink(p, ghost)).count();

    // The floor: the page without the colour edit has ghost ink and no ramp
    // colour, so a measurement that finds ramp colours everywhere says so here.
    assert_eq!(
        wearing(&plain_picture),
        0,
        "the page without a colour edit already draws in ramp colours"
    );
    let ghost_before = ghosts(&plain_picture);
    assert!(
        ghost_before > 0,
        "the swept page draws no ghost ink to keep"
    );

    let mut positions: Vec<usize> = picture.iter().filter_map(|&p| along(p)).collect();
    positions.sort_unstable();
    positions.dedup();
    assert!(
        positions.len() > 1,
        "the highlighted points wear {} position(s) along the ramp: {VALUE} on colour \
         should paint them by value",
        positions.len()
    );
    // The same rows are outside the range in both pages, so the same ghost
    // points are drawn. Their pixels are not identical: the legend's band
    // narrows the raster, and a dot lands on other sub-pixels.
    let ghost_after = ghosts(&picture);
    assert!(
        ghost_after * 10 >= ghost_before * 9,
        "the ghost layer kept {ghost_after} pixels of its ink with {VALUE} on colour and \
         {ghost_before} without: the column painted the ghost points too"
    );
}
