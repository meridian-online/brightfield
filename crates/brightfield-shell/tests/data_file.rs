//! Opening a data file the user chose, through the entry points a user
//! actually reaches.
//!
//! Two of them, and both are here on purpose. `data_file::open` is the seam the
//! window calls; `MeridianApp::open_data_file` is what the front door's control
//! reaches through, and it is where "a failure must never be a blank frame"
//! either holds or does not — the function under it can return a perfectly
//! worded `Err` while the window swallows it.
//!
//! **No dialog is opened anywhere in this file.** `data_file::pick` is one call
//! into rfd and nothing else; every decision worth gating sits on the other side
//! of it and takes a string. A test that raised an operating-system modal would
//! hijack the desktop of whoever ran the suite.
//!
//! The fixtures are written to a temp directory at test time rather than
//! committed, because what is being gated is DuckDB reading a real file off a
//! real filesystem — a fixture path that resolved relative to the checkout
//! would pass from the repo root and nowhere else, which is the exact defect
//! `starts.rs` exists to remember.

use std::path::{Path, PathBuf};

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_protocol::layout::Flow;
use brightfield_shell::chart_kinds;
use brightfield_shell::dashboard;
use brightfield_shell::data_file;
use brightfield_shell::design::Mode;
use brightfield_shell::editor::{EditorPane, SaveReport};
use brightfield_shell::resample::Step;
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::analysis::ComponentPath;
use brightfield_sql::ir::ScalarValue;
use brightfield_workbench::registry::ChartKindId;
use brightfield_workbench::ViewKind;

/// A directory of this test's own, removed when the test ends.
///
/// `std::env::temp_dir` plus the test's name plus the process id: unique per
/// run, so a suite running its tests concurrently cannot have two of these
/// collide, and readable in a failure message.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "bf-data-file-{name}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }

    /// Write `contents` to `name` inside this directory and hand back the path.
    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, contents).expect("the fixture writes");
        path
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A CSV with a numeric column and a categorical one — the ordinary shape.
const READINGS_CSV: &str = "region,reading\n\
                            north,12\n\
                            north,18\n\
                            south,31\n\
                            south,44\n\
                            east,7\n\
                            east,25\n\
                            west,52\n\
                            west,63\n";

/// A window under test, with one `egui::Context` for its whole life — the same
/// arrangement `front_door.rs` uses, and for the same reason: egui resolves a
/// click against a widget id registered on a previous frame.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    fn open() -> Self {
        Self {
            app: MeridianApp::headless_with_layout(Boot::empty(), default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
        }
    }

    fn run(&mut self, frames: Vec<Vec<egui::Event>>) {
        for events in frames {
            let raw = egui::RawInput {
                screen_rect: Some(self.screen),
                events,
                ..Default::default()
            };
            let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        }
    }

    fn settle(&mut self) {
        self.run(vec![Vec::new(), Vec::new()]);
    }

    /// A pointer position at `fraction` across plot `index`'s **data area**,
    /// halfway down it, in the window coordinates the raster was presented at.
    ///
    /// The data area rather than the plot rect: the margins carry the axis and
    /// its labels, and a pixel out there inverts through the x scale to a value
    /// off the end of the domain — so a sweep measured over the rect would commit
    /// bounds the column never reaches.
    fn at(&self, index: usize, fraction: f64) -> egui::Pos2 {
        let doc = self.app.chart_doc();
        let raster = doc
            .raster_rect
            .expect("a settled frame presented the raster");
        let plot = &doc.composed.plots[index];
        let l = &plot.layout;
        let x = plot.rect.x + l.plot_x_start() + (l.plot_x_end() - l.plot_x_start()) * fraction;
        let y = plot.rect.y + (l.plot_y_start() + l.plot_y_end()) / 2.0;
        egui::pos2(raster.min.x + x as f32, raster.min.y + y as f32)
    }

    /// Press at `from`, move to `to`, release — the frames a real sweep across
    /// plot `index` occupies.
    ///
    /// Three frames and not one: the gesture machine is edge-triggered on the
    /// button, so a press and a release in the same frame is a click, and the
    /// move between them is what makes this a sweep.
    fn sweep(&mut self, index: usize, from: f64, to: f64) {
        let start = self.at(index, from);
        self.run(vec![vec![
            egui::Event::PointerMoved(start),
            button_at(start, true),
        ]]);
        let end = self.at(index, to);
        self.run(vec![vec![egui::Event::PointerMoved(end)]]);
        self.run(vec![vec![button_at(end, false)]]);
        self.settle();
    }

    /// A press and a release on one pixel of plot `index` — which an interval
    /// binding reads as *retract this plot's contribution*.
    ///
    /// Two frames, and not [`click_at`]'s one. `click_at` is for an egui widget,
    /// which resolves a press and a release inside the same frame; the chart's
    /// gesture machine is edge-triggered on the button state at the END of a
    /// frame, so a press and a release in one frame leaves that state unchanged
    /// and no gesture ever begins.
    fn click(&mut self, index: usize, fraction: f64) {
        let pos = self.at(index, fraction);
        self.run(vec![vec![
            egui::Event::PointerMoved(pos),
            button_at(pos, true),
        ]]);
        self.run(vec![vec![button_at(pos, false)]]);
        self.settle();
    }
}

/// A primary-button press or release at `pos`.
fn button_at(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// One frame's worth of a pointer move and a primary click at `pos`.
fn click_at(pos: egui::Pos2) -> Vec<egui::Event> {
    let mut events = vec![egui::Event::PointerMoved(pos)];
    for pressed in [true, false] {
        events.push(button_at(pos, pressed));
    }
    events
}

// ---------------------------------------------------------------------------
// AC1 — the file becomes a queryable table
// ---------------------------------------------------------------------------

/// A CSV the user chose opens as a table the engine can be **queried** for, not
/// as a picture of one.
///
/// The assertion is the row count and the schema the live session returns for
/// the step the Data pane tabulates — `SELECT * FROM <the file's view>` — read
/// back through the same windowed seam that pane reads through. Asserting that
/// the composition has a plot would not have caught a chart drawn over a
/// rolled-up view with the file nowhere in the session.
#[test]
fn a_chosen_csv_becomes_a_table_the_session_can_be_queried_for() {
    let dir = TempDir::new("csv-table");
    let path = dir.write("readings.csv", READINGS_CSV);

    let data_file::OpenedFile {
        mut live, composed, ..
    } = data_file::open(&path.to_string_lossy()).expect("an ordinary CSV opens");

    assert!(
        composed.width > 0 && composed.height > 0,
        "the open has to land on a drawn result, not an empty frame"
    );

    // The step the grid reads is mark 0's SOURCE — the file's own view. Eight
    // data rows in, eight rows out.
    let session = live.coordinator().session();
    assert_eq!(
        session.step_rows_count(0).expect("the step counts"),
        8,
        "every row of the file is in the table, unaggregated"
    );

    // …and its columns are the file's columns, in the file's order.
    let batches = session
        .execute_step_rows_window(0, 0, 8)
        .expect("the windowed read the Data pane makes");
    let schema = batches.first().expect("a batch of rows").schema();
    let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    assert_eq!(
        names,
        ["region", "reading"],
        "the grid shows the file's schema, not a rolled-up one"
    );
}

/// A Parquet opens on the same path as a CSV, and the file is *read* rather
/// than copied into memory — the whole reason the engine holds it as a view.
///
/// The Parquet is written by the same DuckDB the engine reads it back with, so
/// this gate needs no fixture committed to the tree and no second writer to
/// keep in step with the reader.
#[test]
fn a_chosen_parquet_opens_on_the_same_path() {
    let dir = TempDir::new("parquet-table");
    let csv = dir.write("readings.csv", READINGS_CSV);
    let parquet = dir.path().join("readings.parquet");

    write_parquet(&csv, &parquet);

    let data_file::OpenedFile { mut live, .. } =
        data_file::open(&parquet.to_string_lossy()).expect("a Parquet opens");
    assert_eq!(
        live.coordinator()
            .session()
            .step_rows_count(0)
            .expect("the step counts"),
        8
    );
}

/// Write `csv` out as a Parquet at `parquet`, through the **same DuckDB** the
/// engine reads it back with — `brightfield-engine`'s own dependency, on the
/// same version line, so this gate cannot pass because a second writer's idea
/// of Parquet happens to agree with the reader's.
fn write_parquet(csv: &Path, parquet: &Path) {
    let conn = duckdb::Connection::open_in_memory().expect("an in-memory DuckDB");
    conn.execute_batch(&format!(
        "COPY (SELECT * FROM read_csv('{}')) TO '{}' (FORMAT PARQUET)",
        csv.display(),
        parquet.display()
    ))
    .expect("the COPY runs");
    assert!(parquet.is_file(), "the Parquet was written");
}

/// **A table opens as an analysis: one tile per column.**
///
/// The whole claim in one assertion, through the door a user comes in by. Two
/// columns in the file, two plots in the composition, and each plot is of one of
/// them — a numeric column binned, a categorical one ranked. The version of this
/// route that shipped before drew *one* picture over whichever columns the first
/// applicable kind's slots swallowed, and said nothing about the rest.
#[test]
fn a_table_opens_as_a_tile_per_column() {
    let dir = TempDir::new("tile-per-column");
    let path = dir.write("readings.csv", READINGS_CSV);

    let data_file::OpenedFile {
        composed,
        dashboard,
        ..
    } = data_file::open(&path.to_string_lossy()).expect("an ordinary CSV opens");

    let chosen: Vec<(&str, String)> = dashboard
        .tiles()
        .iter()
        .map(|t| (t.column(), t.kind().to_string()))
        .collect();
    assert_eq!(
        chosen,
        vec![
            ("region", "ranked-category-bars".to_string()),
            ("reading", "binned-histogram".to_string()),
        ],
        "each column gets the picture its own type admits, in the file's order"
    );
    assert!(dashboard.omitted().is_empty(), "{:?}", dashboard.omitted());
    assert_eq!(
        composed.plots.len(),
        dashboard.tiles().len(),
        "every tile has to reach the composition as a plot of its own"
    );
    assert!(
        composed.mark_faults.is_empty(),
        "the engine refused a tile's own mark: {:?}",
        composed.mark_faults
    );
    assert!(composed.width > 0 && composed.height > 0);
}

/// A table of **two categorical columns** is two rankings, and neither is the
/// count grid: `count-grid` declares two required categorical slots, so no
/// single column can fill it and no per-column tile is ever one.
///
/// The consequence is recorded here rather than left to be rediscovered: the
/// open-a-data-file route no longer draws that kind at all. It keeps its
/// registry entry and its unit tests.
#[test]
fn a_table_of_two_categories_is_two_rankings_and_not_a_grid() {
    let dir = TempDir::new("two-categories");
    let path = dir.write(
        "links.csv",
        "tier,method\n\
         authoritative,sec-registration\n\
         authoritative,sec-ncen\n\
         candidate,jaro_winkler\n\
         candidate,exact_name\n\
         authoritative,sec-registration\n\
         candidate,jaro_winkler\n",
    );

    let data_file::OpenedFile {
        mut live,
        composed,
        dashboard,
        ..
    } = data_file::open(&path.to_string_lossy()).expect("a table of two categorical columns opens");
    assert_eq!(dashboard.tiles().len(), 2, "{dashboard:?}");
    for tile in dashboard.tiles() {
        assert_eq!(tile.kind(), brightfield_shell::ranked_bars::KIND_ID);
    }
    assert!(
        composed.mark_faults.is_empty(),
        "the engine refused a tile's own mark: {:?}",
        composed.mark_faults
    );
    assert_eq!(
        live.coordinator()
            .session()
            .step_rows_count(0)
            .expect("the step counts"),
        6,
        "a tile aggregates in its own query, so the table behind it is still the \
         file"
    );
}

/// A table of one free-text column **is** a picture now: the registry carries a
/// kind whose single slot a category fills, so the column that used to be
/// refused opens on its ranking.
///
/// Asserted through `open`, so what is pinned is what a person gets — a live
/// session over their file with something drawn on it — rather than which
/// branch chose it.
#[test]
fn a_table_of_one_category_opens_on_its_ranking() {
    let dir = TempDir::new("one-category");
    let path = dir.write("names.csv", "name\nada\ngrace\nbarbara\nkaren\nada\n");

    let data_file::OpenedFile {
        mut live,
        composed,
        dashboard,
        ..
    } = data_file::open(&path.to_string_lossy()).expect("one category is a ranking, not a refusal");
    let tile = dashboard.sole_tile().expect("one column is one tile");
    assert_eq!(tile.kind(), brightfield_shell::ranked_bars::KIND_ID);
    assert!(
        !composed.plots.is_empty(),
        "the ranking composed no plot at all"
    );
    // …and the Data pane beside it still holds the file's own rows, which is
    // the property every kind in the registry is held to.
    assert_eq!(
        live.coordinator()
            .session()
            .step_rows_count(0)
            .expect("the step counts"),
        5,
        "the ranking aggregates in its own query, so the table behind it is \
         still the file"
    );
}

/// **A file that opened cleanly says nothing.** The window carries no banner
/// over a picture it drew from a table the user merely opened.
///
/// Asserted at the window rather than on the diagnostics, because the banner is
/// the artefact: `MeridianApp::say_load_diagnostics` turns a load's advisories
/// into one `Severity::Warning` reading *"… had no effect"*, and the user has
/// no spec of their own to go and correct — the spec was synthesised by the
/// chart kind. The one-category table is the case that reached this: its block
/// binds `$sel` from a `toggleY` and a `highlight`, and a block that binds a
/// selection it does not declare earns exactly that advisory.
#[test]
fn a_one_category_table_opens_without_a_banner_over_its_picture() {
    let dir = TempDir::new("one-category-banner");
    let path = dir.write("names.csv", "name\nada\ngrace\nbarbara\nkaren\nada\n");

    let mut win = Window::open();
    win.settle();
    let ctx = win.ctx.clone();
    win.app.open_data_file(&ctx, &path.to_string_lossy());
    win.settle();

    let said: Vec<String> = win
        .app
        .load_diagnostics()
        .diagnostics
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        win.app.notifications().len(),
        0,
        "the window put a banner over a file that opened cleanly: {said:?}"
    );
}

/// **Which chart documents carry the kind that chose their picture** —
/// enumerated over the routes that open one, rather than swept.
///
/// The chart pane draws a document through that kind's `ChartModule` exactly
/// when the document carries an `Authored` record; with none it presents
/// directly. So a route that synthesises a picture *from a chart kind* and does
/// not record which one draws that picture around the module rather than
/// through it — and nothing on screen changes when it happens, which is why
/// the routes are listed here instead of grepped for.
///
/// The routes this build opens a chart document on:
///
/// - `MeridianApp::open_data_file` over a table with **one** tile. That tile's
///   picture is the document's picture, so the kind is recorded and it is one
///   this build has.
/// - `MeridianApp::open_data_file` over a table with several. The dashboard is
///   one picture no single kind built — there is no one kind to record and no
///   one binding — so the pane presents it directly, by the arm a written spec
///   takes.
/// - `Boot::start` — the shipped starts, `include_str!`-ed spec source. The
///   registry was never asked, so there is no kind and no binding to record,
///   and the pane presents these directly.
/// - `Boot::open` / the spec editor — a spec someone wrote, which is the same
///   answer as the starts for the same reason. Covered by the start arm below;
///   both reach `ChartDoc::open`, which clears the record.
///
/// `starts::CROSSWALK_CHART` is deliberately not in the list: its spec reads a
/// source over https, and a test that opens it fails on a train.
#[test]
fn a_chart_kinds_picture_carries_its_kind_and_a_written_spec_carries_none() {
    let dir = TempDir::new("authored-routes");
    let one_tile = dir.write("names.csv", ONE_CATEGORY_CSV);
    let many_tiles = dir.write("readings.csv", READINGS_CSV);

    let mut win = Window::open();
    win.settle();
    let ctx = win.ctx.clone();
    win.app.open_data_file(&ctx, &one_tile.to_string_lossy());
    win.settle();

    let authored = win.app.chart_doc().authored().cloned().expect(
        "a one-tile dashboard IS one chart kind's picture and recorded no kind, \
         so the pane draws it around the module instead of through it",
    );
    assert!(
        chart_kinds::find(authored.kind).is_some(),
        "the recorded kind {} is not in this build's registry, so the pane has \
         nothing to draw the picture with",
        authored.kind
    );

    // Several tiles: no single kind built the picture, so nothing claims one.
    win.app.open_data_file(&ctx, &many_tiles.to_string_lossy());
    win.settle();
    assert_eq!(
        win.app.chart_doc().authored(),
        None,
        "a dashboard of several tiles was recorded as one chart kind's picture, \
         which would put a two-plot raster under a one-plot module"
    );

    for id in [starts::DASHBOARD, starts::DISTRIBUTION, starts::BREAKDOWN] {
        let boot = Boot::start(id, Flow::Vertical).unwrap_or_else(|e| panic!("{id}: {e}"));
        let app = MeridianApp::headless(boot, Mode::Light);
        assert_eq!(
            app.chart_doc().authored(),
            None,
            "{id}: a spec someone wrote was recorded as a chart kind's picture"
        );
    }
}

/// Two categorical columns and nothing to bin — the shape `count-grid` takes.
///
/// A column's `distinct` comes back from `approx_count_distinct` (see
/// `Session::profile_sources`), so which kind a fixture reaches is decided by an
/// estimate rather than by counting its rows: a two-valued column in a
/// five-row file was estimated at one and dropped the fixture to a single
/// field. Which kind each file below actually reaches is left to the coverage
/// assertion at the foot of the test rather than asserted per fixture here.
const CROSSED_CSV: &str = "tier,method\n\
                           authoritative,sec-registration\n\
                           authoritative,sec-ncen\n\
                           candidate,jaro_winkler\n\
                           candidate,exact_name\n\
                           authoritative,sec-registration\n\
                           candidate,jaro_winkler\n";

/// One categorical column — the shape ranked category bars take.
const ONE_CATEGORY_CSV: &str = "name\nada\ngrace\nbarbara\nkaren\nada\n";

/// One numeric column — one tile, and that tile is the binned histogram.
const ONE_MEASURE_CSV: &str = "reading\n12\n18\n31\n44\n7\n25\n52\n63\n";

/// One dated column holding a **day per row for three months** — the shape
/// `counts-over-time` takes, and the shape that used to reach no kind at all.
///
/// Ninety distinct days is past the category ceiling `chart_kinds` applies, and
/// that ceiling was applied to every non-binnable column — so this file, opened
/// from the front door, came back as a sentence about the column it had left
/// out. It is a fixture rather than a literal because ninety lines of CSV in a
/// constant is ninety lines nobody reads; January, February and March of a
/// non-leap year are 31 + 28 + 31 days, which is the ninety.
///
/// The first day is written twice so the counts are not uniformly one.
fn ninety_days_csv() -> String {
    let mut out = String::from("day\n2026-01-01\n");
    for (month, days) in [(1, 31), (2, 28), (3, 31)] {
        for day in 1..=days {
            out.push_str(&format!("2026-{month:02}-{day:02}\n"));
        }
    }
    out
}

/// One column of **instants**, ninety of them an hour apart — a `TIMESTAMP` in
/// DuckDB rather than a `DATE`, which is the type this route used to have no
/// answer for at all.
///
/// Ninety readings put the column past the ceiling `chart_kinds` applies to a
/// category, and the times of day put it past what a `DATE` can spell: three
/// days and change is counted by the hour, and there is no cast that turns an
/// instant into an hour-shaped calendar value.
fn ninety_hours_csv() -> String {
    let mut out = String::from("observed\n");
    for hour in 0..90 {
        let day = 1 + hour / 24;
        out.push_str(&format!("2026-01-{day:02} {:02}:00:00\n", hour % 24));
    }
    out
}

/// **A column of instants opens as a picture, through the profile DuckDB
/// actually returns.**
///
/// The unit tests around `Dashboard::of` hand it a `ColumnProfile` they wrote,
/// so what they cannot show is that a CSV of instants comes back typed
/// `TIMESTAMP` and carrying the `min`/`max` the step is chosen from. This opens
/// the file the way the front door does and reads the answer off the dashboard.
///
/// The claim is the card's: **no omission**. Before this, the column arrived in
/// `Dashboard::omitted` reading *"no chart in this build fits it"* — a true
/// sentence about a file whose one column a reader can see is a series.
#[test]
fn a_column_of_instants_opens_as_a_tile_and_not_as_an_omission() {
    let dir = TempDir::new("instants");
    let path = dir.write("instants.csv", &ninety_hours_csv());

    let opened = data_file::open(&path.to_string_lossy()).expect("the file opens");
    assert!(
        opened.dashboard.omitted().is_empty(),
        "a column of instants was left out: {:?}",
        opened.dashboard.omitted()
    );
    let tile = opened
        .dashboard
        .sole_tile()
        .expect("one column is one tile");
    assert_eq!(tile.column(), "observed", "the tile is of the column");
    assert_eq!(tile.kind(), chart_kinds::COUNTS_OVER_TIME);
    assert_eq!(
        tile.resampled(),
        Some(Step::Hour),
        "three days of hourly readings are counted by the hour"
    );
    assert_eq!(tile.drawn_column(), "observed by hour");
    // And the picture is a picture: the composition carries the plot, over the
    // one table every tile reads.
    let spec = opened.dashboard.to_spec();
    assert!(
        spec.contains("strftime(CAST(\"observed\" AS TIMESTAMP)"),
        "{spec}"
    );
    assert!(!opened.composed.plots.is_empty(), "nothing was composed");
}

/// **A file a user opened arrives on screen as a picture**, and each kind a
/// single column can fill is drawn through its own module.
///
/// The gap this closes was measured rather than imagined. Emptying the
/// `Authored` record's `fields` in `MeridianApp::open_data_file`, or its
/// `block`, left the whole `brightfield-shell` suite green while the chart pane
/// went blank for every file opened from the front door — the first stops at
/// `ChartKind::bind` inside `ChartModule::ui`, the second at
/// `ChartDoc::draw_module`'s comparison, and both end with no raster, no legend
/// band and no `empty_state` to explain it.
///
/// So the assertion is `raster_rect` on a settled window — the observable
/// `ChartDoc::present_raster` writes, and the one a GPU-free machine has (see
/// `tests/chart_module.rs`'s header for why a rect rather than pixels).
///
/// **Written over the kinds a lone column can fill**, which is the set a
/// per-column dashboard can choose from and therefore the only set this route
/// can reach. A kind needing two columns — `count-grid` today, a scatter later —
/// is reachable by no fixture here and the partition below says so rather than
/// letting the absence read as an oversight.
#[test]
fn every_kind_a_column_can_fill_draws_its_picture_from_the_open_a_file_route() {
    let dir = TempDir::new("open-draws-a-picture");
    let mut through_a_module: Vec<ChartKindId> = Vec::new();
    let ninety_days = ninety_days_csv();

    for (name, contents) in [
        // One tile each: the document IS that kind's picture, so the pane hosts
        // it through the kind's module.
        ("one-measure.csv", ONE_MEASURE_CSV),
        ("names.csv", ONE_CATEGORY_CSV),
        ("ninety-days.csv", ninety_days.as_str()),
        // Several tiles: one picture no single kind built, presented directly.
        ("readings.csv", READINGS_CSV),
        ("crossed.csv", CROSSED_CSV),
    ] {
        let path = dir.write(name, contents);
        let mut win = Window::open();
        win.settle();
        let ctx = win.ctx.clone();
        win.app.open_data_file(&ctx, &path.to_string_lossy());
        win.settle();

        let raster = win.app.chart_doc().raster_rect.unwrap_or_else(|| {
            panic!(
                "{name}: opened and nothing reached the pane — what the user \
                 gets is a blank chart with no sentence on it"
            )
        });
        assert!(
            raster.width() > 0.0 && raster.height() > 0.0,
            "{name}: the raster was reserved at {raster:?}, which has no room \
             for a picture in it"
        );
        if let Some(authored) = win.app.chart_doc().authored() {
            assert!(
                chart_kinds::find(authored.kind).is_some(),
                "{name}: opened as {}, which is not in this build's registry, \
                 so the pane has nothing to draw the picture with",
                authored.kind
            );
            through_a_module.push(authored.kind);
        }
    }

    for kind in dashboard::single_column_kinds() {
        assert!(
            through_a_module.contains(&kind.id),
            "{}: one column fills this kind, so a one-column file opens as its \
             picture and is drawn through its module — but no fixture here \
             reaches it. The fixtures above reached {through_a_module:?}",
            kind.id
        );
    }

    // The partition, stated: a kind whose required slots one column cannot fill
    // is drawn by nothing on this route, because a per-column dashboard can
    // never choose it. `count-grid` is that kind today.
    let tileable: Vec<ChartKindId> = dashboard::single_column_kinds()
        .iter()
        .map(|k| k.id)
        .collect();
    for kind in chart_kinds::registry().kinds() {
        if tileable.contains(&kind.id) {
            continue;
        }
        assert!(
            kind.slots.iter().filter(|s| s.required).count() != 1,
            "{}: one column fills this kind's required slot, so it belongs in \
             the tileable set and a fixture above should reach it",
            kind.id
        );
        assert!(
            !through_a_module.contains(&kind.id),
            "{}: this route drew a kind no single column can fill",
            kind.id
        );
    }
}

// ---------------------------------------------------------------------------
// A brush on any tile reaches every other tile
// ---------------------------------------------------------------------------

/// Two measures, so both tiles are histograms and both narrow — the shape that
/// lets a cross-filter be read as a row count rather than as pixels.
const TWO_MEASURES_CSV: &str = "temp,power\n\
                                1,4\n2,12\n3,7\n4,16\n5,9\n6,21\n\
                                7,13\n8,18\n9,24\n10,11\n11,27\n12,19\n\
                                13,33\n14,22\n15,38\n16,26\n17,41\n18,29\n";

/// **A brush on one tile filters every other tile, and not itself — and no
/// tile's denominator moves.**
///
/// Read off the engine rather than off the picture: after the drag, the row set
/// each layer's query materialises is what a re-render would draw, so the
/// arithmetic is the assertion. Each histogram tile is two layers, and all three
/// facts are visible in their four row counts:
///
/// - the other tile's **subset** narrows to the rows the drag kept;
/// - the brushed tile's subset does not, because `select: crossfilter` drops a
///   consumer's own clause — which is what stops a brush from erasing the very
///   bars the hand is on;
/// - and **neither ghost moves**, because a ghost that re-queried under the
///   filter would take the denominator off the page while still drawing a
///   plausible chart.
///
/// The predicate is handed to the document here, so this reaches everything
/// downstream of a selection and nothing upstream of one. Whether a hand on the
/// tile can *produce* that predicate is
/// `a_pointer_sweep_on_one_tile_filters_the_others` below.
#[test]
fn a_brush_on_one_tile_filters_the_others_and_not_itself() {
    let dir = TempDir::new("crossfilter");
    let path = dir.write("pairs.csv", TWO_MEASURES_CSV);

    let data_file::OpenedFile {
        mut live,
        composed,
        dashboard,
        ..
    } = data_file::open(&path.to_string_lossy()).expect("two measures open");
    assert_eq!(dashboard.tiles().len(), 2, "{dashboard:?}");
    assert_eq!(composed.plots.len(), 2);

    // Two layers per tile, in emission order: tile 0's ghost and subset, then
    // tile 1's.
    let (ghost_0, subset_0, ghost_1, subset_1) = (0, 1, 2, 3);
    let rows = |live: &mut brightfield_shell::pipeline::LiveDashboard, i: usize| -> u64 {
        live.coordinator()
            .session()
            .step_rows_count(i)
            .expect("the step counts")
    };
    for layer in [ghost_0, subset_0, ghost_1, subset_1] {
        assert_eq!(
            rows(&mut live, layer),
            18,
            "at rest every layer holds the whole file"
        );
    }

    // Drag an x-range on the FIRST tile — `temp` between 1 and 9 inclusive is
    // nine of the eighteen rows.
    let brushed = ComponentPath(composed.plots[0].path.clone());
    live.apply(Interaction::Select {
        name: brightfield_shell::dashboard::SELECTION.to_string(),
        contributor: brushed,
        predicate: SqlPredicate::Interval {
            column: "temp".to_string(),
            lo: ScalarValue::Float(1.0),
            hi: ScalarValue::Float(9.0),
            meta: None,
        },
    })
    .expect("the brush re-composites");

    assert_eq!(
        rows(&mut live, subset_1),
        9,
        "the OTHER tile has to narrow to the rows the drag kept — a tile that \
         does not is a dashboard of unconnected charts"
    );
    assert_eq!(
        rows(&mut live, subset_0),
        18,
        "…and the brushed tile keeps its own rows, because a crossfilter \
         consumer drops its own clause"
    );
    for ghost in [ghost_0, ghost_1] {
        assert_eq!(
            rows(&mut live, ghost),
            18,
            "a ghost layer must never narrow, or the subset has nothing to be a \
             fraction of"
        );
    }
}

/// How many data rows [`TWO_MEASURES_CSV`] holds — counted off the fixture so
/// the assertions below carry no second copy of it.
fn fixture_total() -> u64 {
    TWO_MEASURES_CSV
        .lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .count() as u64
}

/// How many of the fixture's rows `column` keeps between `lo` and `hi`
/// inclusive, read straight out of the CSV text.
///
/// The independent oracle the sweep test compares the engine against: the bounds
/// come from the gesture, the count comes from the file, and DuckDB is in
/// neither. A count taken from a second query would only show the engine
/// agreeing with itself.
fn rows_kept(column: &str, lo: f64, hi: f64) -> u64 {
    let mut lines = TWO_MEASURES_CSV.lines();
    let header: Vec<&str> = lines
        .next()
        .expect("the fixture has a header")
        .split(',')
        .collect();
    let at = header.iter().position(|h| *h == column).unwrap_or_else(|| {
        panic!(
            "the committed clause names {column}, which is no column of the fixture ({header:?})"
        )
    });
    lines
        .filter(|l| !l.trim().is_empty())
        .filter(|l| {
            let cell = l.split(',').nth(at).expect("the row has that column");
            let v: f64 = cell.parse().expect("a numeric cell");
            v >= lo && v <= hi
        })
        .count() as u64
}

/// Every `Interval` clause in a predicate tree, as `(column, lo, hi)`.
///
/// Walked rather than matched at the root because how many contributors a
/// selection's clause wraps, and in what, is `compile_selection`'s business
/// rather than this test's.
fn intervals(predicate: &SqlPredicate) -> Vec<(String, f64, f64)> {
    match predicate {
        SqlPredicate::Interval { column, lo, hi, .. } => match (lo, hi) {
            (ScalarValue::Float(lo), ScalarValue::Float(hi)) => vec![(column.clone(), *lo, *hi)],
            _ => Vec::new(),
        },
        SqlPredicate::And(parts) | SqlPredicate::Or(parts) => {
            parts.iter().flat_map(intervals).collect()
        }
        _ => Vec::new(),
    }
}

/// The one interval clause the window's selections are holding, or a panic
/// naming what they held instead.
fn held_interval(win: &Window) -> (String, f64, f64) {
    let held = win
        .app
        .chart_doc()
        .live_dashboard()
        .expect("an opened data file has a live session")
        .selection_clauses();
    let found: Vec<(String, f64, f64)> = held.iter().flat_map(|(_, p)| intervals(p)).collect();
    match found.as_slice() {
        [one] => one.clone(),
        other => panic!(
            "a released sweep was expected to commit exactly one interval clause; \
             the document holds {other:?} (selections: {:?})",
            win.app.chart_doc().selection_sql()
        ),
    }
}

/// The rows the live session now returns for the step at `index`.
///
/// Read off the engine rather than off the document's record of what it asked
/// for, for the reason `ChartDoc::live_coordinator` is public: a gate that
/// asserted this from a field the code under test writes would be asserting
/// against itself.
fn step_rows(win: &mut Window, index: usize) -> u64 {
    win.app
        .chart_doc_mut()
        .live_coordinator()
        .expect("an opened data file has a live session")
        .session()
        .step_rows_count(index)
        .expect("the step counts")
}

/// **A pointer sweep on one tile — down, moved, up — filters every other
/// tile.**
///
/// The sibling above hands `Interaction::Select` to the document with a
/// predicate the test wrote. That proves every tile *consumes* a selection and
/// cannot prove one of them can *make* one. This drives the path a hand takes
/// instead: a CSV opened through `MeridianApp::open_data_file`, pointer events
/// through `ctx.run_ui`, and the chart pane's `drive_gestures` →
/// `resolve_gesture` → `interval_predicate` to a committed clause.
///
/// That path was dead on a binned tile until the interval binding learned to
/// read the column out of a bin transform, and no test in this file reddened
/// while it was: `x: {bin: temp}` named no column the binding could see, so it
/// bailed before dispatching anything and a reader got a brush rectangle that
/// painted and resolved to nothing.
///
/// **The engine is not asked to confirm its own arithmetic.** The bounds come
/// from the gesture; the row count they should keep is counted out of the CSV
/// text by `rows_kept`; the number compared against it is what the other tile's
/// query materialised.
#[test]
fn a_pointer_sweep_on_one_tile_filters_the_others() {
    let dir = TempDir::new("sweep-crossfilter");
    let path = dir.write("pairs.csv", TWO_MEASURES_CSV);
    let total = fixture_total();

    let mut win = Window::open();
    win.settle();
    let ctx = win.ctx.clone();
    win.app.open_data_file(&ctx, &path.to_string_lossy());
    win.settle();

    assert_eq!(
        win.app.chart_doc().composed.plots.len(),
        2,
        "two measures have to open as two tiles, or the sweep below is on some \
         other picture"
    );

    // Two layers per tile, in emission order: tile 0's ghost and subset, then
    // tile 1's.
    let (ghost_0, subset_0, ghost_1, subset_1) = (0, 1, 2, 3);
    for layer in [ghost_0, subset_0, ghost_1, subset_1] {
        assert_eq!(
            step_rows(&mut win, layer),
            total,
            "at rest every layer holds the whole file"
        );
    }
    assert!(
        win.app.chart_doc().selection_sql().is_none(),
        "a tile nobody has swept is already holding {:?}",
        win.app.chart_doc().selection_sql()
    );

    // The sweep, across the middle of the first tile's data area. Middling
    // fractions on purpose: far enough inside the axis that a bound cannot be
    // right by clamping to an end of the domain.
    win.sweep(0, 0.2, 0.7);

    let (column, lo, hi) = held_interval(&win);
    assert!(
        lo < hi,
        "the sweep committed the degenerate interval [{lo}, {hi}] over {column}"
    );
    let kept = rows_kept(&column, lo, hi);
    assert!(
        kept > 0 && kept < total,
        "the sweep committed {column} in [{lo}, {hi}], which keeps {kept} of the \
         fixture's {total} rows — bounds keeping all of them or none of them \
         cannot tell a working cross-filter from a broken one"
    );

    assert_eq!(
        step_rows(&mut win, subset_1),
        kept,
        "the OTHER tile has to narrow to the rows the sweep kept: {column} in \
         [{lo}, {hi}] is {kept} of the fixture's rows"
    );
    assert_eq!(
        step_rows(&mut win, subset_0),
        total,
        "…and the swept tile keeps its own rows, because a crossfilter consumer \
         drops its own clause"
    );
    for ghost in [ghost_0, ghost_1] {
        assert_eq!(
            step_rows(&mut win, ghost),
            total,
            "a ghost layer must never narrow, or the subset has nothing to be a \
             fraction of"
        );
    }

    // A press and release on one pixel is the other half of the same branch:
    // the crossfilter convention retracts this plot's contribution.
    win.click(0, 0.5);
    assert!(
        win.app.chart_doc().selection_sql().is_none(),
        "a click on an interval binding did not retract the contribution: {:?}",
        win.app.chart_doc().selection_sql()
    );
    assert_eq!(
        step_rows(&mut win, subset_1),
        total,
        "the retracted sweep left the other tile narrowed"
    );
}

/// **A brush reaches a tile of another kind too**, in the form that kind
/// consumes a selection in.
///
/// A ranked-bars module deliberately does *not* filter its own mark — see
/// `ranked_bars`'s header for what that would cost — so its row count is the
/// wrong place to look. The predicate lands inside its conditional `SUM`
/// instead, and the executed SQL is where that is observable.
#[test]
fn a_brush_reaches_a_tile_that_consumes_by_highlighting() {
    let dir = TempDir::new("crossfilter-mixed");
    let path = dir.write("readings.csv", READINGS_CSV);

    let data_file::OpenedFile {
        mut live,
        composed,
        dashboard,
        ..
    } = data_file::open(&path.to_string_lossy()).expect("a mixed table opens");
    let ranked = dashboard
        .tiles()
        .iter()
        .position(|t| t.kind() == brightfield_shell::ranked_bars::KIND_ID)
        .expect("the categorical column is a ranking");
    let measure = dashboard
        .tiles()
        .iter()
        .position(|t| t.kind() == chart_kinds::BINNED_HISTOGRAM)
        .expect("the numeric column is a histogram");

    live.coordinator().session_mut().clear_executed_sql();
    live.apply(Interaction::Select {
        name: brightfield_shell::dashboard::SELECTION.to_string(),
        contributor: ComponentPath(composed.plots[measure].path.clone()),
        predicate: SqlPredicate::Interval {
            column: "reading".to_string(),
            lo: ScalarValue::Float(10.0),
            hi: ScalarValue::Float(30.0),
            meta: None,
        },
    })
    .expect("the brush re-composites");

    let ranked_column = dashboard.tiles()[ranked].column();
    let reached: Vec<String> = live
        .coordinator()
        .session()
        .executed_sql()
        .into_iter()
        .filter(|sql| sql.contains(ranked_column) && sql.contains("reading"))
        .collect();
    assert!(
        !reached.is_empty(),
        "the brush on the measure never reached the ranking's query, so the \
         two tiles are not cross-filtered. Executed: {:?}",
        live.coordinator().session().executed_sql()
    );
}

// ---------------------------------------------------------------------------
// The generated dashboard is a spec, not internal state
// ---------------------------------------------------------------------------

/// **The dashboard nobody authored is a spec file the reader can open, read and
/// edit** — in the same pane, through the same field, as a dashboard composed
/// from a spec someone wrote.
///
/// Three things are asserted, and the first is the one that makes the other two
/// worth anything: the bytes on disk are the bytes that composed the picture. A
/// writer that re-serialised the spec would produce a plausible document that is
/// evidence of nothing.
#[test]
fn the_generated_dashboard_is_a_spec_the_reader_can_open_and_edit() {
    let dir = TempDir::new("spec-is-visible");
    let path = dir.write("readings.csv", READINGS_CSV);

    let mut win = Window::open();
    win.settle();
    let ctx = win.ctx.clone();
    win.app.open_data_file(&ctx, &path.to_string_lossy());
    win.settle();

    let spec_path = win.app.chart_doc().spec_path.clone().expect(
        "the document has to carry the spec it was composed from, or the \
                 editor pane has nothing to open and the dashboard is opaque",
    );
    let on_disk = std::fs::read_to_string(&spec_path).expect("the spec file reads");

    // 1. It is the source that ran.
    let data_file::OpenedFile { dashboard, .. } =
        data_file::open(&path.to_string_lossy()).expect("re-opens");
    assert_eq!(
        on_disk,
        dashboard.to_spec(),
        "the file the reader opens is not the source the picture was composed \
         from"
    );

    // 2. It composes to the picture on screen — same tiles, same plots.
    let recomposed = brightfield_shell::pipeline::compose_spec_str(&on_disk, None)
        .expect("the written spec composes on its own");
    assert_eq!(
        recomposed.plots.len(),
        win.app.chart_doc().composed.plots.len(),
        "the spec the reader can edit draws a different dashboard from the one \
         they are looking at"
    );

    // 3. The pane the shell hosts opens it, shows it, and can write it back.
    let mut pane = EditorPane::new();
    pane.open_file(&spec_path);
    assert_eq!(pane.buffer(), Some(on_disk.as_str()));
    assert!(
        !pane.can_save(),
        "nothing has been typed yet, so there is nothing to write"
    );
    if let Some(buffer) = pane.buffer_mut() {
        *buffer = buffer.replace("height: 300", "height: 220");
    }
    pane.note_buffer_edited();
    assert!(pane.can_save(), "an edited buffer has something to write");
    assert!(
        matches!(pane.save_now(), SaveReport::Written),
        "the reader's edit did not reach the file"
    );
    let edited = std::fs::read_to_string(&spec_path).expect("the edited spec reads");
    assert!(edited.contains("height: 220"), "{edited}");
    assert!(
        brightfield_shell::pipeline::compose_spec_str(&edited, None).is_ok(),
        "the edited spec no longer composes:\n{edited}"
    );
}

/// The spec carries **why each tile is the tile it is**, and what was left out —
/// so the rule is checkable by a reader holding the artefact.
#[test]
fn the_generated_spec_states_the_rule_it_was_generated_by() {
    let dir = TempDir::new("spec-states-the-rule");
    let path = dir.write(
        "mixed.csv",
        "region,reading,version\n\
         north,12,1\n\
         north,18,1\n\
         south,31,1\n\
         south,44,1\n\
         east,7,1\n\
         east,25,1\n\
         west,52,1\n\
         west,63,1\n",
    );

    let data_file::OpenedFile { spec_file, .. } =
        data_file::open(&path.to_string_lossy()).expect("the file opens");
    let source =
        std::fs::read_to_string(spec_file.expect("a spec file")).expect("the spec file reads");

    assert!(
        source.contains(
            "# reading: no trusted label, and DuckDB stored it as BIGINT → binned-histogram"
        ),
        "{source}"
    );
    assert!(source.contains("region: no trusted label"), "{source}");
    assert!(
        source.contains("version: one distinct value"),
        "a column left out has to say so in the spec the reader opens:\n{source}"
    );
}

/// A table no chart kind fits is refused **by name and by reason**, rather than
/// composing an empty window.
///
/// One column of identifiers: too many distinct values to read as an axis, and
/// nothing numeric to bin. The composition path returns `Err` when no mark
/// renders, so the alternative to a sentence is not a blank chart, it is an
/// unexplained one. Reopening this shape as a table with no picture is residual
/// scope.
///
/// **The sentence names the column and its reason**, which is a better answer
/// than the list of what the build's charts take that it replaces: a reader
/// looking at a file whose contents they can see is owed *why this column*, not
/// a catalogue of shapes.
#[test]
fn a_table_with_nothing_to_draw_says_what_it_is_missing() {
    let dir = TempDir::new("nothing-to-draw");
    let mut csv = String::from("id\n");
    for i in 0..200 {
        csv.push_str(&format!("row-{i}\n"));
    }
    let path = dir.write("identifiers.csv", &csv);

    let refusal = data_file::open(&path.to_string_lossy())
        .err()
        .expect("200 distinct identifiers fill no chart kind's slot");
    assert!(refusal.contains("identifiers.csv"), "{refusal}");
    assert!(
        refusal.contains("id:"),
        "the refusal has to name the column it could not draw: {refusal}"
    );
    assert!(
        refusal.contains("no chart in this build fits it"),
        "…and why: {refusal}"
    );
}

// ---------------------------------------------------------------------------
// AC1, the other half — the file that opens is the file that was CHOSEN
// ---------------------------------------------------------------------------

/// Assert the invariant this section exists for, against a real decoy on a real
/// filesystem: **either the chosen file's rows come back, or the open is
/// refused by name.** Silently opening the other file is the one outcome barred.
///
/// Written as a constraint rather than as "it returns `Err`" on purpose. The
/// refusal is this build's answer, but a later one that escaped the name
/// faithfully instead would be a *better* answer, and a test asserting the
/// mechanism would redden on the improvement while staying green on the defect.
/// What must never pass is `Ok` over the decoy.
fn assert_opens_the_chosen_file_or_refuses(chosen: &Path, chosen_rows: u64, decoy_rows: u64) {
    let shown = chosen.display().to_string();
    match data_file::open(&chosen.to_string_lossy()) {
        Err(refusal) => {
            let name = chosen
                .file_name()
                .map(|n| n.to_string_lossy().escape_debug().to_string())
                .unwrap_or_default();
            assert!(
                refusal.contains(&name),
                "a refusal has to name the file the user picked. Wanted {name:?} \
                 in: {refusal}"
            );
            assert!(
                refusal.len() > name.len() + 8,
                "…and has to carry a reason as well as a name: {refusal}"
            );
        }
        Ok(data_file::OpenedFile { mut live, .. }) => {
            let rows = live
                .coordinator()
                .session()
                .step_rows_count(0)
                .expect("the step counts");
            assert_ne!(
                rows, decoy_rows,
                "{shown} opened, and the table behind it holds the DECOY's \
                 {decoy_rows} rows — the window is titled from the picked name, \
                 so this is one file's name over another file's data"
            );
            assert_eq!(
                rows, chosen_rows,
                "{shown} opened over neither file's row count"
            );
        }
    }
}

/// A file name holding a bracket sits beside the file a **glob** of that name
/// matches, and the app does not quietly read the neighbour.
///
/// `sales[1].csv` is not a contrived name — it is what a browser writes for a
/// second download of `sales.csv`. DuckDB resolves a reader path as a glob, so
/// `read_csv('…/sales[1].csv')` binds over `sales1.csv`: measured on this
/// build's DuckDB, 3 rows where the chosen file has 8. Nothing is red when it
/// happens, which is why this gate is here rather than a comment.
#[test]
fn a_bracket_in_the_name_does_not_open_the_file_a_glob_would_match() {
    let dir = TempDir::new("glob-decoy");
    // The decoy is what `sales[1].csv` matches as a pattern: 3 rows.
    dir.write("sales1.csv", "region,reading\nnorth,12\nsouth,31\neast,7\n");
    // …and the file actually picked, 8 rows.
    let chosen = dir.write("sales[1].csv", READINGS_CSV);

    assert_opens_the_chosen_file_or_refuses(&chosen, 8, 3);
}

/// The same, one layer up: the **folder** carries the bracket. A reader path is
/// globbed whole, so a directory component matches exactly as a file name does,
/// and a guard that only looked at `file_name()` would pass this.
#[test]
fn a_bracket_in_a_parent_folder_is_caught_too() {
    let dir = TempDir::new("glob-decoy-dir");
    std::fs::create_dir_all(dir.path().join("q1")).expect("the decoy folder");
    std::fs::write(
        dir.path().join("q1").join("sales.csv"),
        "region,reading\nnorth,12\nsouth,31\neast,7\n",
    )
    .expect("the decoy writes");
    std::fs::create_dir_all(dir.path().join("q[1]")).expect("the chosen folder");
    let chosen = dir.path().join("q[1]").join("sales.csv");
    std::fs::write(&chosen, READINGS_CSV).expect("the fixture writes");

    assert_opens_the_chosen_file_or_refuses(&chosen, 8, 3);
}

/// A file name holding a **line break** sits beside the file that name folds
/// to, and the app does not quietly read the neighbour.
///
/// A line break is legal in a POSIX file name and YAML folds one inside a
/// quoted scalar to a space, so `sales<LF>2026.csv` written into the
/// synthesised spec parses back as `sales 2026.csv` — a different file, which
/// on this fixture has 3 rows rather than 8. DuckDB reads the real name
/// perfectly well; the loss is entirely in the round trip through the spec.
#[test]
fn a_line_break_in_the_name_does_not_open_the_file_it_folds_to() {
    let dir = TempDir::new("fold-decoy");
    // The decoy is what the line break folds to — a space: 3 rows.
    dir.write(
        "sales 2026.csv",
        "region,reading\nnorth,12\nsouth,31\neast,7\n",
    );
    let chosen = dir.path().join("sales\n2026.csv");
    let Ok(()) = std::fs::write(&chosen, READINGS_CSV) else {
        // A filesystem that refuses the name has nothing to gate here.
        return;
    };

    assert_opens_the_chosen_file_or_refuses(&chosen, 8, 3);
}

/// A refused name reaches the **window** as a banner and leaves the door up,
/// which is what makes the refusal an answer rather than a dropped click.
///
/// `data_file::open` returning a good sentence is not the same as a user seeing
/// one: `open_data_file` is the seam the door's control reaches through, and
/// the swallow would happen there.
#[test]
fn a_pattern_name_refused_at_the_window_opens_nothing_and_says_so() {
    let dir = TempDir::new("glob-window");
    dir.write("sales1.csv", "region,reading\nnorth,12\nsouth,31\neast,7\n");
    let chosen = dir.write("sales[1].csv", READINGS_CSV);

    let mut win = Window::open();
    win.settle();
    let ctx = win.ctx.clone();
    win.app.open_data_file(&ctx, &chosen.to_string_lossy());
    win.settle();

    assert!(
        !win.app.chart_doc().is_live(),
        "no session may be built over a name the reader would resolve to a \
         different file"
    );
    assert_eq!(
        win.app.notifications().len(),
        1,
        "…and it is refused out loud, not ignored"
    );
    assert!(
        win.app.front_door_is_live(),
        "the door is still standing — a refusal is never a blank frame"
    );
}

/// **The dialect gate.** Ask this build's DuckDB which characters it resolves
/// as a pattern, and require `accept` to refuse every one of them.
///
/// The refusal list in `data_file.rs` is a constant, and a constant about
/// somebody else's parser goes stale silently — a DuckDB bump that taught the
/// reader a new metacharacter would reopen exactly the defect this section
/// gates, with every test above still green. So the list is not trusted here:
/// each candidate character gets a real file of its own in a directory of
/// plausible siblings, `glob()` is asked what that path matches, and the
/// **danger condition is measured** — a non-empty match set that is not the
/// file itself. When nothing matches, DuckDB falls back to the literal path,
/// which is why most punctuation is safe and is asserted safe rather than
/// assumed.
///
/// This gate is one-directional by design: it requires the refusal list to
/// COVER what DuckDB globs, not to equal it. Refusing a character DuckDB
/// happens to resolve literally today is the deliberate margin documented on
/// `PATTERN_CHARACTERS`.
#[test]
fn every_character_this_duckdb_reads_as_a_pattern_is_refused() {
    let dir = TempDir::new("dialect");
    // Siblings a pattern could plausibly land on instead of the file itself.
    for sibling in [
        "s1.csv", "sa.csv", "sb.csv", "sX.csv", "sq.csv", "sxyz.csv", "s.csv",
    ] {
        dir.write(sibling, "region,reading\nnorth,12\n");
    }

    let conn = duckdb::Connection::open_in_memory().expect("an in-memory DuckDB");
    let mut globbed_by_duckdb: Vec<char> = Vec::new();

    // Every printable ASCII character, which is the alphabet a file name is
    // realistically drawn from. `/` is excluded because no POSIX file name may
    // hold one; anything else the filesystem refuses is skipped below rather
    // than predicted here.
    let candidates: Vec<char> = (0x20u8..0x7f)
        .map(char::from)
        .filter(|c| *c != '/')
        .collect();

    for candidate in candidates {
        let name = format!("s{candidate}.csv");
        let chosen = dir.path().join(&name);
        // Windows and some filesystems refuse a few of these outright; a name
        // that cannot exist cannot be opened, so it is not this gate's business.
        if std::fs::write(&chosen, READINGS_CSV).is_err() {
            continue;
        }
        let literal = chosen.display().to_string();
        let sql = format!("SELECT file FROM glob('{}')", literal.replace('\'', "''"));
        let mut statement = conn.prepare(&sql).expect("glob() prepares");
        let matched: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("glob() runs")
            .map(|row| row.expect("a matched path"))
            .collect();
        let resolves_elsewhere = !matched.is_empty() && matched != vec![literal.clone()];
        if resolves_elsewhere {
            globbed_by_duckdb.push(candidate);
            assert!(
                data_file::accept(&literal).is_err(),
                "DuckDB resolves a path containing `{candidate}` to {matched:?} \
                 rather than to the file itself, so opening it would read a \
                 different file — `accept` has to refuse it and does not"
            );
        }
        let _ = std::fs::remove_file(&chosen);
    }

    // The sweep has to have found something, or a `glob()` that silently
    // stopped working would make this test vacuous — the shape a structural
    // gate fails in.
    assert!(
        globbed_by_duckdb.contains(&'*') && globbed_by_duckdb.contains(&'?'),
        "the sweep did not observe DuckDB globbing `*` or `?`, so it proved \
         nothing about anything. Observed: {globbed_by_duckdb:?}"
    );
}

// ---------------------------------------------------------------------------
// AC2 — the front door offers it
// ---------------------------------------------------------------------------

/// The front door draws an open-a-file control, inside the window, and clicking
/// it asks for the dialog.
///
/// Clicked where the last frame actually laid it out rather than at a
/// coordinate typed here, for the reason `front_door.rs` records: a hand-typed
/// point lands today and goes on being green while clicking empty background
/// the first time a padding moves. What is asserted is the **request**, not a
/// dialog: the picker is the one thing here a headless test may not run.
#[test]
fn the_front_door_offers_opening_a_file_and_the_control_is_wired() {
    let mut win = Window::open();
    win.settle();
    assert!(
        win.app.front_door_is_live(),
        "a launch with nothing named shows the door"
    );

    let target = win
        .app
        .front_door_open_file_rect()
        .expect("the door draws an open-a-data-file control");
    assert!(
        win.screen.contains_rect(target),
        "the control drew at {target:?}, outside the window — nothing could \
         click it"
    );
    assert!(!win.app.pick_requested(), "nothing has been clicked yet");

    win.run(vec![click_at(target.center())]);
    assert!(
        win.app.pick_requested(),
        "clicking the control has to ask for the file dialog"
    );
}

/// …and the control belongs to the door: once something is open it is gone,
/// exactly as the gallery cards are.
#[test]
fn the_open_a_file_control_goes_with_the_door() {
    let dir = TempDir::new("door-morph");
    let path = dir.write("readings.csv", READINGS_CSV);

    let mut win = Window::open();
    win.settle();
    assert!(win.app.front_door_open_file_rect().is_some());

    let ctx = win.ctx.clone();
    win.app.open_data_file(&ctx, &path.to_string_lossy());
    win.settle();

    assert!(
        !win.app.front_door_is_live(),
        "an opened file is content, and content outcompetes the door"
    );
    assert!(
        win.app.front_door_open_file_rect().is_none(),
        "a test asking where the control was after the door has gone must \
         hear 'nowhere'"
    );
}

/// Opening a file through the window puts the file on the charts view, live —
/// so a brush over the histogram re-queries rather than filtering a frozen
/// batch, and the Data pane has rows to read.
#[test]
fn opening_a_file_lands_on_the_charts_view_with_a_live_session() {
    let dir = TempDir::new("lands-live");
    let path = dir.write("readings.csv", READINGS_CSV);

    let mut win = Window::open();
    win.settle();
    let ctx = win.ctx.clone();
    win.app.open_data_file(&ctx, &path.to_string_lossy());
    win.settle();

    assert_eq!(win.app.active(), ViewKind::Charts);
    assert!(
        win.app.chart_doc().is_live(),
        "a file opened from the door has to arm its own gestures"
    );
    assert!(
        win.app.title().contains("readings.csv"),
        "the window names the file the user picked, not the path they never \
         typed: {}",
        win.app.title()
    );
    assert!(
        win.app.notifications().is_empty(),
        "an ordinary open raises no banner"
    );
}

// ---------------------------------------------------------------------------
// AC3 — a file the engine cannot read fails by name
// ---------------------------------------------------------------------------

/// A file DuckDB will not read fails with the path and the engine's own reason,
/// and the window stays exactly as it was.
///
/// Asserted through the window rather than through `data_file::open` alone,
/// which is the difference between "the function returns a good message" and
/// "the user sees one": a banner is what stands between this and a blank frame.
#[test]
fn a_file_the_engine_cannot_read_names_itself_and_leaves_the_window_up() {
    let dir = TempDir::new("unreadable");
    // A .parquet that is not a Parquet: the extension passes, the reader does
    // not. This is the ordinary shape of the failure — a download that landed
    // as an HTML error page, a truncated copy.
    let path = dir.write("broken.parquet", "this is not a parquet file at all\n");

    let refusal = data_file::open(&path.to_string_lossy())
        .err()
        .expect("a file that is not a Parquet does not open");
    assert!(
        refusal.contains("broken.parquet"),
        "the failure has to name the file: {refusal}"
    );
    assert!(
        refusal.len() > "broken.parquet: ".len() + 8,
        "the failure has to carry a reason as well as a name: {refusal}"
    );

    let mut win = Window::open();
    win.settle();
    let ctx = win.ctx.clone();
    win.app.open_data_file(&ctx, &path.to_string_lossy());
    win.settle();

    assert_eq!(
        win.app.notifications().len(),
        1,
        "the reason has to reach a surface the user is looking at"
    );
    assert!(
        win.app.front_door_is_live(),
        "a failed open changes nothing — the door is still standing, which is \
         the whole of 'never a blank frame'"
    );
    assert!(
        win.app.front_door_open_file_rect().is_some(),
        "…and the control that failed is still there to try again with"
    );
}

/// A second failure replaces the first's banner rather than stacking a history
/// of what did not open, and a success takes it down.
#[test]
fn a_failed_open_leaves_one_banner_and_a_success_clears_it() {
    let dir = TempDir::new("banner-life");
    let broken = dir.write("broken.parquet", "not a parquet\n");
    let good = dir.write("readings.csv", READINGS_CSV);

    let mut win = Window::open();
    win.settle();
    let ctx = win.ctx.clone();

    win.app.open_data_file(&ctx, &broken.to_string_lossy());
    assert_eq!(win.app.notifications().len(), 1);
    win.app.open_data_file(&ctx, "/no/such/file.csv");
    assert_eq!(
        win.app.notifications().len(),
        1,
        "the last attempt is the only one that matters"
    );
    win.app.open_data_file(&ctx, &good.to_string_lossy());
    assert!(
        win.app.notifications().is_empty(),
        "a success takes its own failure banner down"
    );
}

/// A path naming nothing fails before the engine is built, and says so in the
/// words of the thing the user picked.
#[test]
fn a_path_naming_nothing_fails_by_name() {
    let refusal = data_file::open("/no/such/directory/readings.csv")
        .err()
        .expect("a missing file does not open");
    assert!(refusal.contains("readings.csv"), "{refusal}");
}

// ---------------------------------------------------------------------------
// AC4 — a URL is refused
// ---------------------------------------------------------------------------

/// A URL is refused, through the same entry point the door's control reaches,
/// with a message saying it is a URL — and nothing is opened.
///
/// This is the cheapest way for the box to stop being about this machine.
/// DuckDB binds a view over an `https://` Parquet through `httpfs` eagerly and
/// without complaint, so a path box that merely *passed the string on* would
/// fetch, succeed, and leave nothing red anywhere.
#[test]
fn a_url_is_refused_at_the_window_and_opens_nothing() {
    let mut win = Window::open();
    win.settle();
    let ctx = win.ctx.clone();

    win.app
        .open_data_file(&ctx, "https://openlake.meridian.online/edgar_gleif.parquet");
    win.settle();

    assert!(
        win.app.front_door_is_live(),
        "a URL opens nothing at all — the door is still standing"
    );
    assert_eq!(
        win.app.notifications().len(),
        1,
        "…and it is refused out loud, not ignored"
    );
    assert!(
        !win.app.chart_doc().is_live(),
        "no session was built over a remote table"
    );
}

/// The refusal is by scheme and happens before any engine exists, so a URL that
/// would have resolved is turned away exactly as one that would not.
#[test]
fn every_url_scheme_is_refused_before_the_engine() {
    for url in [
        "https://openlake.meridian.online/edgar_gleif.parquet",
        "http://127.0.0.1:1/never.csv",
        "s3://bucket/key.parquet",
        "ducklake:https://example.com/catalog.ducklake",
    ] {
        let refusal = data_file::open(url)
            .err()
            .unwrap_or_else(|| panic!("{url} is not a local file and must not open"));
        assert!(refusal.contains("URL"), "{url}: {refusal}");
    }
}
