//! **A column put on the map's x or y moves both its layers, and one that is
//! not a coordinate draws a dot plot.**
//!
//! The tests here start from the dashboard the generator writes for a table
//! whose columns hold a coordinate pair, opened the way a data file is opened,
//! and edit its hero map through `shelf_edit::put_column` with the table's
//! own profile. The spec half reads the edited AST; the page half loads a page
//! from it the way the shell's tile controls do, and sweeps a rectangle over
//! it in a headless window.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use brightfield_engine::{ColumnProfile, ProfileOutcome, RowsAudience, SqlPredicate};
use brightfield_render::layout::Margins;
use brightfield_render::title::TITLE_BAND;
use brightfield_shell::chart_kinds;
use brightfield_shell::data_file::{self, OpenedFile};
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::LiveDashboard;
use brightfield_shell::shelf_edit::{put_column, ShelfRefusal};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::ast::{Component, PlotNode, Spec, SpecValue, ValueOrParamRef};
use brightfield_spec::edit::{self, plot_at_path, ChartEdit, RefuseReason};
use brightfield_spec::layout::{resolve_axis_titles, AxisTitle, PlotAxis};
use brightfield_spec::MarkKind;
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
