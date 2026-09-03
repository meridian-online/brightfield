//! **Resting the pointer on a mark, through the shipped window.**
//!
//! The engine's own oracle (`brightfield-engine/tests/nearest_row_oracle.rs`)
//! holds what the read *returns*. Nothing there says a pointer ever reaches
//! it, and that gap is the one this file exists to close: a criterion driven
//! through the read function passes with the gesture missing entirely. So the
//! claims here are driven by **pointer events into `MeridianApp::draw`** — real
//! `PointerMoved` and `PointerButton` events, at coordinates derived from the
//! frame the window laid out — and read back off the frame or off the
//! accessibility tree.
//!
//! # Two harnesses, on purpose
//!
//! The counting and state claims run through a plain `egui::Context`, one
//! frame per `ctx.run_ui` call, because the whole of the pointer-stillness gate
//! is *which frame* a query lands on and a helper that ran several frames per
//! call would make that unassertable.
//!
//! The readout's own contents run through an `egui_kittest` harness instead,
//! because the readout is native egui chrome and its lines are accesskit nodes.
//! No wgpu adapter is asked for: the readout is text in an `Area`, and a
//! headless window draws it exactly as a device-backed one does.

use std::path::PathBuf;

use brightfield_render::channel::Channel;
use brightfield_shell::app::HoverReadout;
use brightfield_shell::data_grid::fetch_page;
use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::arrangement;
use egui_kittest::kittest::Queryable;

// ---------------------------------------------------------------------------
// Fixtures and window plumbing.
// ---------------------------------------------------------------------------

/// The table the first screen opens: nine columns, a coordinate pair among
/// them, so the generator draws a point map as the hero and stacks the rest.
///
/// The same committed sample `canvas_pane_group.rs` uses.
fn housing() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv")
}

/// The window every case here is laid out in — the size the map-pane work was
/// drawn at.
const SCREEN: egui::Rect = egui::Rect {
    min: egui::Pos2::ZERO,
    max: egui::pos2(1440.0, 900.0),
};

/// One row of the fixture: its coordinate pair and every other column by name.
///
/// Read from the file rather than typed here. Two things depend on it. An aim
/// is placed at *exactly* a row's own position, so the nearest mark is at zero
/// distance and the readout has a determinate answer; a coordinate typed here
/// would go on being plausible the day the sample changed, and the hover would
/// then be aimed at empty space — a readout that never appears and a test that
/// fails for the wrong reason. And the brush case partitions these rows by the
/// interval it swept, which needs the values the engine is filtering on.
struct FixtureRow {
    longitude: f64,
    latitude: f64,
    by_name: Vec<(String, f64)>,
}

impl FixtureRow {
    /// This row's value in `column`.
    fn value(&self, column: &str) -> f64 {
        self.by_name
            .iter()
            .find(|(name, _)| name == column)
            .map(|(_, v)| *v)
            .unwrap_or_else(|| panic!("the fixture has no column {column}"))
    }
}

/// Every row of the fixture, in file order.
fn fixture_rows() -> Vec<FixtureRow> {
    let text = std::fs::read_to_string(housing()).expect("the fixture reads");
    let mut lines = text.lines();
    let header: Vec<String> = lines
        .next()
        .expect("a header row")
        .split(',')
        .map(str::to_string)
        .collect();
    let column = |name: &str| {
        header
            .iter()
            .position(|c| c == name)
            .unwrap_or_else(|| panic!("the fixture has no {name} column"))
    };
    let (lon, lat) = (column("longitude"), column("latitude"));
    lines
        .map(|line| {
            let cells: Vec<f64> = line
                .split(',')
                .map(|c| c.parse().expect("a numeric cell"))
                .collect();
            FixtureRow {
                longitude: cells[lon],
                latitude: cells[lat],
                by_name: header.iter().cloned().zip(cells.iter().copied()).collect(),
            }
        })
        .collect()
}

/// The `(longitude, latitude)` of one row of the fixture.
fn a_row_of_the_fixture(index: usize) -> (f64, f64) {
    let rows = fixture_rows();
    let row = &rows[index];
    (row.longitude, row.latitude)
}

/// A booted window over the fixture, with no device behind it.
fn window() -> MeridianApp {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    MeridianApp::headless(boot, Mode::Light)
}

/// A window over a spec file, with no device behind it.
fn spec_window(relative: &str) -> MeridianApp {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    let chosen = path.to_str().expect("utf-8 spec path");
    let boot = Boot::open(chosen, brightfield_protocol::layout::Flow::Vertical, None)
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    MeridianApp::headless(boot, Mode::Light)
}

/// One frame, carrying `events` and nothing else.
///
/// Deliberately **one** `run_ui` per call: the gate under test is a comparison
/// between consecutive frames, so a helper that settled several frames would
/// hide the very thing being asserted.
fn frame(app: &mut MeridianApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let input = egui::RawInput {
        screen_rect: Some(SCREEN),
        events,
        ..Default::default()
    };
    let _ = ctx.run_ui(input, |ui| app.draw(ui));
}

/// A pointer button event.
fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// Three settling frames — egui stores a resizable panel's reported size and
/// reads it back on the frame after, so a rect read before this is not the one
/// the window keeps.
fn settle(app: &mut MeridianApp, ctx: &egui::Context) {
    for _ in 0..3 {
        frame(app, ctx, Vec::new());
    }
}

/// **Where a data point lands on screen**, in window-space logical points.
///
/// Through the plot's own displayed scales and the rect the frame drew it at,
/// so the aim follows the layout instead of being a coordinate somebody typed
/// against one. `plot` indexes the composition, and 0 is the hero — the
/// composition places it first.
fn at_data(app: &MeridianApp, plot: usize, x: f64, y: f64) -> egui::Pos2 {
    let rect = app
        .composed_plot_rects()
        .get(plot)
        .copied()
        .expect("the plot drew");
    let scales = &app.chart_doc().composed.plots[plot].scales;
    let sx = scales.get(Channel::X).expect("an x scale");
    let sy = scales.get(Channel::Y).expect("a y scale");
    #[allow(clippy::cast_possible_truncation)]
    egui::pos2(
        rect.min.x + sx.map_f64(x) as f32,
        rect.min.y + sy.map_f64(y) as f32,
    )
}

/// How many DuckDB executes this window's session has performed.
fn executes(app: &MeridianApp) -> usize {
    app.chart_doc()
        .live_dashboard()
        .expect("a live document")
        .executes()
}

/// How many distinct SQL strings the session's renderer-side cache holds.
fn cached(app: &MeridianApp) -> usize {
    app.chart_doc()
        .live_dashboard()
        .expect("a live document")
        .sql_cache_len()
}

/// **Move the pointer to `at` and let it come to rest.**
///
/// Two frames: the first carries the move, the second carries no events at all
/// and is therefore the frame the pointer has not moved on. It is the second
/// that a read can happen on, and this helper is written as two explicit
/// frames rather than as a settle loop so every caller can count them.
fn rest_at(app: &mut MeridianApp, ctx: &egui::Context, at: egui::Pos2) {
    frame(app, ctx, vec![egui::Event::PointerMoved(at)]);
    frame(app, ctx, Vec::new());
}

// ---------------------------------------------------------------------------
// AC6 — the pointer-stillness gate
// ---------------------------------------------------------------------------

/// **A sweep across a plot issues no nearest-point query.**
///
/// Twelve frames, each carrying a move to a different point along a line
/// through the hero's data, and every one of them over marks a rest would find.
/// The execute count must not move at all: this is the claim that a hover is
/// not a query per frame.
///
/// The rest at the end is what stops the assertion passing vacuously — a hover
/// path that had been deleted outright would also issue nothing during the
/// sweep, and this says the sweep's silence is the gate rather than the
/// absence of the feature.
#[test]
fn a_sweep_across_a_plot_issues_no_query() {
    let mut app = window();
    let ctx = egui::Context::default();
    settle(&mut app, &ctx);

    let (lon, lat) = a_row_of_the_fixture(0);
    let landed = at_data(&app, 0, lon, lat);
    let before = executes(&app);
    for step in 0..12 {
        #[allow(clippy::cast_precision_loss)]
        let at = landed + egui::vec2(step as f32 * 2.0, 0.0);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    }
    assert_eq!(
        executes(&app),
        before,
        "twelve frames of pointer movement issued {} queries — the read is not \
         gated on the pointer having stopped",
        executes(&app) - before
    );

    // And the gate is a gate, not a deletion: the pointer stopping reads.
    frame(&mut app, &ctx, Vec::new());
    assert_eq!(
        executes(&app),
        before + 1,
        "the frame after the sweep stopped issued no query, so the silence \
         above is the hover read being absent rather than being gated"
    );
}

/// **A rest issues exactly one query**, and resting on does not issue another.
///
/// Six further frames after the read, none of them carrying an event: the
/// pointer is still where it was, the answer is already held, and asking again
/// would be a query per frame under a different name.
#[test]
fn a_rest_issues_exactly_one_query_however_long_it_lasts() {
    let mut app = window();
    let ctx = egui::Context::default();
    settle(&mut app, &ctx);

    let (lon, lat) = a_row_of_the_fixture(0);
    let before = executes(&app);
    let at = at_data(&app, 0, lon, lat);
    rest_at(&mut app, &ctx, at);
    assert_eq!(
        executes(&app),
        before + 1,
        "a pointer coming to rest on a mark issued {} queries",
        executes(&app) - before
    );

    for _ in 0..6 {
        frame(&mut app, &ctx, Vec::new());
    }
    assert_eq!(
        executes(&app),
        before + 1,
        "six frames of the pointer sitting still issued {} more queries",
        executes(&app) - before - 1
    );
}

/// A rest with **nothing inside the hit radius** asks once and then stops.
///
/// The sibling above holds the case where an answer comes back and is held.
/// This is the case that has no answer to hold, which is the one a naive
/// implementation re-asks on every frame — and the frames it re-asks on are
/// frames somebody else caused, so nothing about the hover looks wrong.
///
/// The aim is a corner of the hero's data area chosen to be far from the
/// cloud, and the test asserts it found nothing before asserting it stopped
/// asking, so an aim that accidentally landed on a mark cannot pass this.
#[test]
fn a_rest_outside_the_hit_radius_issues_one_query_and_no_more() {
    let mut app = window();
    let ctx = egui::Context::default();
    settle(&mut app, &ctx);

    // The far corner of the hero plot's own rect, inset enough to stay inside
    // the pane. California's coordinate cloud does not reach it.
    let hero = app.composed_plot_rects()[0];
    let corner = egui::pos2(hero.right() - 4.0, hero.top() + 4.0);
    let before = executes(&app);
    rest_at(&mut app, &ctx, corner);
    assert_eq!(
        executes(&app),
        before + 1,
        "the rest issued {} queries",
        executes(&app) - before
    );
    assert!(
        readout_lines(&mut app, &ctx, corner).is_empty(),
        "this aim found a mark, so it is not the empty-space case"
    );

    let after_found_nothing = executes(&app);
    for _ in 0..6 {
        frame(&mut app, &ctx, Vec::new());
    }
    assert_eq!(
        executes(&app),
        after_found_nothing,
        "a rest that found nothing kept asking: {} more queries over six \
         still frames",
        executes(&app) - after_found_nothing
    );
}

/// A sweep with the **button down** is a brush, and a brush reads nothing —
/// including on the frames a hand pauses mid-drag.
#[test]
fn a_paused_drag_reads_nothing() {
    let mut app = window();
    let ctx = egui::Context::default();
    settle(&mut app, &ctx);

    let (lon, lat) = a_row_of_the_fixture(0);
    let landed = at_data(&app, 0, lon, lat);
    let from = landed - egui::vec2(30.0, 30.0);

    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from)]);
    frame(&mut app, &ctx, vec![button(from, true)]);
    let before = executes(&app);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(landed)]);
    // The hand pauses over the mark, button still down.
    for _ in 0..4 {
        frame(&mut app, &ctx, Vec::new());
    }
    assert_eq!(
        executes(&app),
        before,
        "a drag paused over a mark issued {} hover queries",
        executes(&app) - before
    );
    frame(&mut app, &ctx, vec![button(landed, false)]);
}

// ---------------------------------------------------------------------------
// AC4, at the window — the counters around a real hover
// ---------------------------------------------------------------------------

/// The hover's query raises the execute count and leaves the SQL cache alone,
/// **through the shipped pointer path** rather than through the read function.
///
/// The engine's oracle holds the same pair one level down. This one is what
/// says the shell's own call site goes through the uncached read: a hover
/// wired to `execute_step_rows` instead would pass the counting tests above
/// and fail here.
#[test]
fn a_hover_read_raises_the_execute_count_without_touching_the_cache() {
    let mut app = window();
    let ctx = egui::Context::default();
    settle(&mut app, &ctx);

    let (lon, lat) = a_row_of_the_fixture(0);
    let executes_before = executes(&app);
    let cached_before = cached(&app);
    assert!(
        cached_before > 0,
        "the boot composition cached nothing, so an unchanged cache below \
         proves nothing"
    );

    let at = at_data(&app, 0, lon, lat);
    rest_at(&mut app, &ctx, at);

    assert_eq!(
        executes(&app),
        executes_before + 1,
        "the hover is not reaching DuckDB"
    );
    assert_eq!(
        cached(&app),
        cached_before,
        "the hover read went through the caching path — a stream of pointer \
         positions will evict the chart's own results"
    );
}

// ---------------------------------------------------------------------------
// AC5 — a hover is not an interaction
// ---------------------------------------------------------------------------

/// A fingerprint of the Protocol on the canvas: its node ids, its seam ids and
/// its edge count.
fn protocol_fingerprint(app: &MeridianApp) -> String {
    let graph = app.protocol_model().displayed_graph();
    format!(
        "{:?}|{:?}|{}",
        graph.nodes.keys().collect::<Vec<_>>(),
        graph.seams.keys().collect::<Vec<_>>(),
        graph.edges.len()
    )
}

/// **A hover produces no interaction, moves no generation and appends nothing
/// to the Protocol.**
///
/// All three around one rest, and the rest is asserted to have *happened*
/// first — a hover that silently did nothing would satisfy the three
/// assertions below perfectly.
#[test]
fn a_hover_is_not_an_interaction() {
    let mut app = window();
    let ctx = egui::Context::default();
    settle(&mut app, &ctx);

    let generation = |app: &mut MeridianApp| {
        app.chart_doc_mut()
            .live_coordinator()
            .expect("a live document")
            .generation()
    };

    let (lon, lat) = a_row_of_the_fixture(0);
    let at = at_data(&app, 0, lon, lat);
    let before = (
        generation(&mut app),
        app.chart_doc().selection_sql(),
        protocol_fingerprint(&app),
        executes(&app),
    );

    rest_at(&mut app, &ctx, at);

    assert_eq!(
        executes(&app),
        before.3 + 1,
        "no query was issued, so this test is asserting three things about a \
         hover that did not happen"
    );
    assert_eq!(
        generation(&mut app),
        before.0,
        "the hover advanced the coordinator's generation — it went through \
         the interaction seam"
    );
    assert_eq!(
        app.chart_doc().selection_sql(),
        before.1,
        "the hover changed what the selections hold, so it pushed a predicate"
    );
    assert!(
        !before.2.starts_with("[]|[]|"),
        "the Protocol on the canvas has no nodes, no seams and no edges, so \
         comparing its fingerprint across a hover compares two empty lists"
    );
    assert_eq!(
        protocol_fingerprint(&app),
        before.2,
        "the hover changed the Protocol on the canvas"
    );
}

// ---------------------------------------------------------------------------
// AC3 — the row is one the chart is drawing, which means the layer that narrows
// ---------------------------------------------------------------------------

/// Sweep a brush over the hero, from `from` to `to`, and settle it.
fn brush(app: &mut MeridianApp, ctx: &egui::Context, from: egui::Pos2, to: egui::Pos2) {
    frame(app, ctx, vec![egui::Event::PointerMoved(from)]);
    frame(app, ctx, vec![button(from, true)]);
    frame(app, ctx, vec![egui::Event::PointerMoved(to)]);
    frame(app, ctx, vec![button(to, false)]);
    for _ in 0..2 {
        frame(app, ctx, Vec::new());
    }
}

/// **With a brush committed, a hover finds only rows the chart is drawing.**
///
/// This is the assertion the ghost layer would fail. A generated tile is two
/// marks over one table — a ghost that never narrows, drawn first, and the
/// subset that reads the shared selection, drawn over it — and a nearest read
/// wired to the wrong one keeps answering with rows the brush has already taken
/// out of the picture. Nothing about that shows up in a query count or in a
/// readout's shape; only brushing and then asking outside the brush shows it.
///
/// # Why the brush is on a column tile and the hover is on the map
///
/// Crossfilter **self-exclusion** is why. A plot that publishes a clause is not
/// filtered by its own clause — that is what keeps a brushed histogram showing
/// the bars you are selecting between — so brushing the hero and then hovering
/// the hero reads every row whichever layer the read runs against, and would
/// pass with the ghost. The brush therefore goes on a stacked tile, whose
/// clause the hero's subset layer *does* receive.
///
/// # What is derived and what is asserted
///
/// The sweep's interval is read back through the tile's own x scale, exactly as
/// the gesture inverts it, and the fixture's rows are then partitioned by it in
/// this file. The aim is an excluded row whose nearest surviving row is more
/// than [`CLEAR_OF_THE_RADIUS`] points away, so "no row was found" cannot be
/// a surviving neighbour being found instead.
///
/// Three readings, and all three are needed. Before the brush the aim finds a
/// row, so the aim is on a mark. After the brush the same aim finds nothing.
/// And an aim on a *surviving* row still finds one, so what refused the middle
/// reading is the predicate rather than the hover having stopped working.
#[test]
fn a_brush_on_a_tile_leaves_the_hover_reading_only_what_the_map_still_draws() {
    let mut app = window();
    let ctx = egui::Context::default();
    settle(&mut app, &ctx);

    // The first stacked tile the column pane drew whole — a histogram over one
    // of the file's columns, with an `intervalX` brush on it.
    let columns_body = app
        .canvas_panes()
        .pane("columns")
        .expect("the column pane drew")
        .body;
    let rects = app.composed_plot_rects();
    let tile = (1..rects.len())
        .find(|i| columns_body.contains(rects[*i].min) && columns_body.contains(rects[*i].max))
        .expect("a stacked tile is drawn whole inside the column pane");
    let tile_rect = rects[tile];
    let column = app.chart_doc().composed.plots[tile]
        .x_column
        .clone()
        .expect("the tile bins a column");

    // Sweep the right-hand third of the tile, and read the interval back
    // through the same scale the gesture inverts through.
    let from = egui::pos2(
        tile_rect.left() + tile_rect.width() * 0.66,
        tile_rect.center().y,
    );
    let to = egui::pos2(tile_rect.right() - 2.0, tile_rect.center().y);
    let interval = {
        let scale = app.chart_doc().composed.plots[tile]
            .scales
            .get(Channel::X)
            .expect("the tile has an x scale");
        let lo = scale
            .inverse_f64(f64::from(from.x - tile_rect.min.x))
            .expect("a continuous x scale");
        let hi = scale
            .inverse_f64(f64::from(to.x - tile_rect.min.x))
            .expect("a continuous x scale");
        (lo.min(hi), lo.max(hi))
    };

    // Partition the fixture by that interval, in screen coordinates.
    let rows = fixture_rows();
    let kept: Vec<egui::Pos2> = rows
        .iter()
        .filter(|r| r.value(&column) >= interval.0 && r.value(&column) <= interval.1)
        .map(|r| at_data(&app, 0, r.longitude, r.latitude))
        .collect();
    assert!(
        !kept.is_empty() && kept.len() < rows.len(),
        "the sweep kept {} of {} rows — it has to keep some and drop some",
        kept.len(),
        rows.len()
    );
    let hero = app.composed_plot_rects()[0];
    let dropped = rows
        .iter()
        .filter(|r| r.value(&column) < interval.0 || r.value(&column) > interval.1)
        .map(|r| at_data(&app, 0, r.longitude, r.latitude))
        .find(|at| {
            hero.contains(*at)
                && kept
                    .iter()
                    .all(|k| (*k - *at).length() > CLEAR_OF_THE_RADIUS)
        })
        .expect("an excluded row with no surviving row near it on screen");
    let survivor = *kept
        .iter()
        .find(|at| hero.contains(**at))
        .expect("a surviving row inside the hero's rect");

    assert!(
        !readout_lines(&mut app, &ctx, dropped).is_empty(),
        "the aim is not on a mark before any brush, so nothing below means \
         anything"
    );

    brush(&mut app, &ctx, from, to);
    assert!(
        app.chart_doc().selection_sql().is_some(),
        "the sweep on the tile committed no selection, so there is no brush to \
         be outside of"
    );

    assert!(
        readout_lines(&mut app, &ctx, dropped).is_empty(),
        "a hover over a row the brush excluded still named it — the read is \
         running against the ghost layer, which never narrows"
    );
    assert!(
        !readout_lines(&mut app, &ctx, survivor).is_empty(),
        "a hover over a row the brush KEPT found nothing either, so the reading \
         above is the hover being broken rather than the brush narrowing it"
    );
}

/// How far an excluded row has to be from every surviving row, in logical
/// points, for "nothing was found there" to mean the brush and not a
/// neighbour.
///
/// Comfortably more than the pane's hit radius, which is deliberately not
/// imported: a margin that tracked the radius would move with it and stop
/// being a margin.
const CLEAR_OF_THE_RADIUS: f32 = 40.0;

// ---------------------------------------------------------------------------
// AC7 and AC9 — what the readout says, read off the accessibility tree
// ---------------------------------------------------------------------------

/// A kittest harness over a window, settled.
fn harness(app: MeridianApp) -> egui_kittest::Harness<'static, MeridianApp> {
    let mut harness = egui_kittest::Harness::builder()
        .with_size(SCREEN.size())
        .with_pixels_per_point(1.0)
        .build_ui_state(|ui, app: &mut MeridianApp| app.draw(ui), app);
    harness.run();
    harness
}

/// The columns the plot at `plot` encodes, in readout order.
fn encoded_columns(app: &MeridianApp, plot: usize) -> Vec<String> {
    app.chart_doc().composed.plots[plot]
        .hover
        .as_ref()
        .map(|layer| {
            layer
                .channels
                .iter()
                .map(|(_, column)| column.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// **The readout's lines, and proof each one reached the accessibility tree.**
///
/// The lines come off the document and every one of them is then *resolved as
/// a node*, which is the half that matters: the readout is native egui chrome
/// rather than raster ink precisely so a reader with a screen reader gets it,
/// and a panel that was built and never drawn would leave the document's copy
/// intact and the tree empty. `get_by_label` panics naming the label it could
/// not find, so a line that reaches no node fails here by name.
fn lines_on_screen(harness: &egui_kittest::Harness<'_, MeridianApp>) -> Vec<String> {
    let lines = harness
        .state()
        .chart_doc()
        .hover_readout
        .as_ref()
        .map(|r| r.lines.clone())
        .unwrap_or_default();
    for line in &lines {
        harness.get_by_label(line.as_str());
    }
    lines
}

/// The readout's lines after resting at `at`.
///
/// The pointer is moved away and rested first, so that a rest at the same
/// point as the previous one is a fresh read rather than an answer already
/// held — otherwise a second call at the same aim would report the first
/// call's answer and every comparison below would be with itself.
fn readout_lines(app: &mut MeridianApp, ctx: &egui::Context, at: egui::Pos2) -> Vec<String> {
    rest_at(app, ctx, egui::pos2(1.0, 1.0));
    rest_at(app, ctx, at);
    app.chart_doc()
        .hover_readout
        .as_ref()
        .map(|r| r.lines.clone())
        .unwrap_or_default()
}

/// **Hovering the hero point map names the coordinate pair**, each line named
/// by the column it encodes.
///
/// The values are checked by mapping them *back* through the plot's scales and
/// asserting they land on the pixel the pointer was at. That is what makes
/// this a claim about the row the read found rather than about the shape of a
/// string: a readout naming the wrong row, or a readout naming the right
/// columns with values from some other row, fails.
///
/// It is also the "encodes neither colour nor size" case. The generated point
/// map binds x and y and nothing else, so a readout of exactly two lines is
/// the whole of that claim — a blank third line would be three.
#[test]
fn a_hover_on_the_hero_map_names_the_coordinate_pair() {
    let mut h = harness(window());
    let (lon, lat) = a_row_of_the_fixture(0);
    let at = at_data(h.state(), 0, lon, lat);
    let columns = encoded_columns(h.state(), 0);
    assert_eq!(
        columns,
        vec!["longitude".to_string(), "latitude".to_string()],
        "the hero map encodes x and y by these columns and nothing else"
    );

    h.hover_at(at);
    h.step();
    h.step();

    let lines = lines_on_screen(&h);
    assert_eq!(
        lines.len(),
        2,
        "the readout drew {lines:?} — the point map encodes neither colour nor \
         size, so it names exactly the coordinate pair"
    );
    assert!(
        lines[0].starts_with("longitude: ") && lines[1].starts_with("latitude: "),
        "the lines are not named by the columns they encode: {lines:?}"
    );

    let value = |line: &str| -> f64 {
        line.split_once(": ")
            .expect("a `column: value` line")
            .1
            .parse()
            .expect("a numeric value")
    };
    let named = at_data(h.state(), 0, value(&lines[0]), value(&lines[1]));
    assert!(
        (named - at).length() < 1.0,
        "the readout named {lines:?}, which is at {named:?} on screen and the \
         pointer was at {at:?} — this is not the row under the pointer"
    );
}

/// **A plot that encodes colour names it too**, by the column it encodes.
///
/// `examples/dashboard.yaml`'s left plot is a dot mark binding `fill: g`, so
/// its readout is three lines rather than two. Its right plot bands its x
/// axis, which has no pixels-per-unit, so a hover there reads nothing — and
/// that pair in one fixture is why this test uses it.
#[test]
fn a_hover_on_a_plot_that_encodes_colour_names_the_colour_column() {
    let mut h = harness(spec_window("../../examples/dashboard.yaml"));
    let columns = encoded_columns(h.state(), 0);
    assert_eq!(
        columns,
        vec!["x".to_string(), "y".to_string(), "g".to_string()],
        "the scatter binds x, y and fill, and fill is bound to a column"
    );
    assert!(
        h.state().chart_doc().composed.plots[1].hover.is_none(),
        "the bar plot bands its x axis, so there is no pixel distance along it \
         and it offers no hover layer"
    );

    // The first row of the inline data.
    let at = at_data(h.state(), 0, 52.0, 158.0);
    h.hover_at(at);
    h.step();
    h.step();

    let lines = lines_on_screen(&h);
    assert_eq!(
        lines,
        vec![
            "x: 52".to_string(),
            "y: 158".to_string(),
            "g: A".to_string()
        ],
        "the readout named {lines:?}"
    );
}

/// **A plot that encodes size names it too**, by the column it encodes — and
/// names it fourth, after the coordinate pair and the colour.
///
/// The fourth of the readout's four channels, and the one the shipped example
/// corpus does not bind: the generated tiles the first screen draws bind the
/// coordinate pair and stop there, and `examples/dashboard.yaml` adds a
/// colour. So the fixture is a committed test spec rather than an example,
/// and it binds colour *and* size so the ORDER is asserted rather than just
/// the presence — a readout that named size before colour would satisfy a
/// per-channel assertion and read wrongly to a person.
///
/// # A limit stated here rather than discovered later
///
/// `Channel::Size` reaches `ChannelMap` from the spec and is named by the
/// readout, but `DOT_RADIUS` is a constant and the dot renderer draws with it:
/// binding `size:` to a column changes what this readout says and does not
/// change how the picture looks. What is pinned here is therefore the readout
/// half of the channel, not a claim that the marks vary by it.
#[test]
fn a_hover_on_a_plot_that_encodes_size_names_the_size_column() {
    let mut h = harness(spec_window("tests/data/size_and_colour.yaml"));
    let columns = encoded_columns(h.state(), 0);
    assert_eq!(
        columns,
        vec![
            "weight".to_string(),
            "height".to_string(),
            "group".to_string(),
            "span".to_string()
        ],
        "the mark binds x, y, fill and size, each to a column, in that order"
    );

    // The second row of the inline data.
    let at = at_data(h.state(), 0, 30.0, 55.0);
    h.hover_at(at);
    h.step();
    h.step();

    let lines = lines_on_screen(&h);
    assert_eq!(
        lines,
        vec![
            "weight: 30".to_string(),
            "height: 55".to_string(),
            "group: B".to_string(),
            "span: 9".to_string()
        ],
        "the readout named {lines:?}"
    );
}

/// **The *hover overlay* checkbox is gone from a window holding a chart.**
///
/// Asked of the accessibility tree, which is where a checkbox that is still
/// drawn would be. The inspector rail is asserted to have drawn first — with
/// the rail collapsed or the pane never reached, no label in it resolves and
/// an absence assertion passes over a window that drew no rail at all.
#[test]
fn no_hover_overlay_checkbox_is_drawn_on_a_window_holding_a_chart() {
    let h = harness(window());
    assert!(
        h.state()
            .region_rect(arrangement::INSPECTOR_RAIL)
            .is_some_and(|r| r.width() > 0.0),
        "the inspector rail did not draw, so an absence in it means nothing"
    );
    assert!(
        h.state().chart_doc().live_dashboard().is_some(),
        "the window is not holding a live chart"
    );
    assert!(
        h.query_all_by_label_contains("hover overlay")
            .next()
            .is_none(),
        "a control labelled \"hover overlay\" is still drawn"
    );
}

// ---------------------------------------------------------------------------
// AC1, at the window — one row's worth held, and nothing growing behind it
// ---------------------------------------------------------------------------

/// **A run of rests leaves the window holding one row's worth**, and leaves
/// the session holding what it held before the first of them.
///
/// The engine's oracle bounds what one read *returns*. This is the claim one
/// level out and over time: after twelve rests at twelve different marks, what
/// the shell is carrying is still a single row of named values, and the
/// session behind it has not grown by twelve of anything. A hover that filed
/// each answer away — appended to the readout, cached the query, kept the
/// batch — passes every single-rest assertion in this file and fails here.
///
/// Three readings, and each is a different way the same defect shows up.
///
/// The **execute count** moves by exactly twelve. That is what says twelve
/// reads happened, so the two assertions below are about a window that did the
/// work rather than one that quietly stopped hovering.
///
/// The **readout width** after each rest is the number of columns the hovered
/// layer encodes. A readout that accumulated would be two lines after the
/// first rest and twenty-four after the twelfth.
///
/// The **cache length** is unchanged across all twelve. `sql_cache` is the one
/// place in the session a read can leave something behind that outlives it,
/// and each rest is a distinct pixel and therefore a distinct SQL string, so a
/// read on the caching path grows it by one per rest. The single-rest version
/// of this in `a_hover_read_raises_the_execute_count_without_touching_the_cache`
/// cannot tell an unchanged cache from an LRU that happened to be at its cap.
///
/// The width of `HoverReadout` is pinned beside them, because the three above
/// measure *how much* is held, and would not move if a `RecordBatch` rode
/// alongside the lines. Its engine-side twin is
/// `the_reads_result_type_is_exactly_its_two_declared_fields_wide`.
#[test]
fn a_run_of_rests_leaves_the_window_holding_one_rows_worth() {
    const RESTS: usize = 12;

    let mut app = window();
    let ctx = egui::Context::default();
    settle(&mut app, &ctx);

    let width = encoded_columns(&app, 0).len();
    assert_eq!(
        width, 2,
        "the hero map encodes {width} columns, so the per-rest assertion below \
         is not the two-line coordinate readout it is written for"
    );

    // Twelve aims, each exactly on a fixture row so a rest is certain to find
    // one, each inside the hero's own rect, and each at a DISTINCT pixel: two
    // rests at the same position are one read, and the count below would then
    // be reporting the gate rather than the retention.
    let hero = app.composed_plot_rects()[0];
    let mut aims: Vec<egui::Pos2> = Vec::new();
    for row in fixture_rows() {
        let at = at_data(&app, 0, row.longitude, row.latitude);
        if hero.contains(at) && !aims.iter().any(|p| *p == at) {
            aims.push(at);
        }
        if aims.len() == RESTS {
            break;
        }
    }
    assert_eq!(
        aims.len(),
        RESTS,
        "the fixture yielded {} distinct on-screen aims inside the hero",
        aims.len()
    );

    let executes_before = executes(&app);
    let cached_before = cached(&app);
    assert!(
        cached_before > 0,
        "the boot composition cached nothing, so an unchanged cache below \
         would be a comparison of zero with zero"
    );

    for (n, aim) in aims.iter().enumerate() {
        rest_at(&mut app, &ctx, *aim);
        let readout = app
            .chart_doc()
            .hover_readout
            .clone()
            .unwrap_or_else(|| panic!("rest {n} at {aim:?} is on a mark and found none"));
        assert_eq!(
            readout.lines.len(),
            width,
            "rest {n} left the window holding {} lines against the {width} \
             columns the layer encodes: {:?}",
            readout.lines.len(),
            readout.lines
        );
    }

    assert_eq!(
        executes(&app),
        executes_before + RESTS,
        "{RESTS} rests at {RESTS} distinct pixels issued {} queries",
        executes(&app) - executes_before
    );
    assert_eq!(
        cached(&app),
        cached_before,
        "the session's cache grew by {} over {RESTS} rests — a stream of \
         pointer positions is accumulating in it",
        cached(&app) - cached_before
    );

    assert_eq!(
        std::mem::size_of::<HoverReadout>(),
        std::mem::size_of::<egui::Pos2>() + std::mem::size_of::<Vec<String>>(),
        "`HoverReadout` is {} bytes against the {} its declared `at` and \
         `lines` account for — the window is holding something else per hover",
        std::mem::size_of::<HoverReadout>(),
        std::mem::size_of::<egui::Pos2>() + std::mem::size_of::<Vec<String>>(),
    );
}

// ---------------------------------------------------------------------------
// AC8 — the shipped surfaces behave the same whether or not a hover happened
// ---------------------------------------------------------------------------

/// Everything AC8 names, read off one window: what the brush holds, what the
/// cross-filter re-query produced, where the legend band and the inspector
/// rail are, and the first page of the data grid.
#[derive(Debug, PartialEq)]
struct Surfaces {
    selection: Option<String>,
    plots: usize,
    page: (u32, u32),
    legend: Option<egui::Rect>,
    rail: Option<egui::Rect>,
    grid: Vec<Vec<brightfield_shell::data_grid::CellText>>,
}

fn surfaces(app: &mut MeridianApp) -> Surfaces {
    let mark = app.chart_doc().composed.plots[0]
        .hover
        .as_ref()
        .expect("the hero offers a hover layer")
        .mark;
    let selection = app.chart_doc().selection_sql();
    let plots = app.chart_doc().composed.plots.len();
    let page = (
        app.chart_doc().composed.width,
        app.chart_doc().composed.height,
    );
    let legend = app.chart_doc().legend_rect;
    let rail = app.region_rect(arrangement::INSPECTOR_RAIL);
    let session = app
        .chart_doc_mut()
        .live_coordinator()
        .expect("a live document")
        .session();
    let grid = fetch_page(session, mark, 0..8)
        .expect("the grid reads")
        .rows;
    Surfaces {
        selection,
        plots,
        page,
        legend,
        rail,
        grid,
    }
}

/// **The shipped surfaces do not notice a hover.**
///
/// The same scripted brush is run twice — once with a rest over a mark
/// inserted before it, once without — and everything AC8 names is compared.
/// The hover is asserted to have *happened* in the first run, so a comparison
/// of two runs that both did nothing cannot pass this.
#[test]
fn the_shipped_surfaces_behave_the_same_with_and_without_a_hover() {
    let run = |hover: bool| -> (Surfaces, usize) {
        let mut app = window();
        let ctx = egui::Context::default();
        settle(&mut app, &ctx);

        let (lon, lat) = a_row_of_the_fixture(0);
        let landed = at_data(&app, 0, lon, lat);
        // Counted around the rests alone: the brush below re-queries every
        // mark, so a delta taken across the whole run would be dominated by
        // the cross-filter and would not say whether a hover happened.
        let mut hovers = 0;
        if hover {
            let before = executes(&app);
            rest_at(&mut app, &ctx, landed);
            hovers += executes(&app) - before;
        }

        // The same brush either way, swept about the hero's own centre so the
        // press lands inside the plot whatever the fixture's coordinates do.
        let hero = app.composed_plot_rects()[0];
        brush(
            &mut app,
            &ctx,
            hero.center() - egui::vec2(30.0, 30.0),
            hero.center() + egui::vec2(30.0, 30.0),
        );

        // And a second rest AFTER the brush, so a hover that perturbed shared
        // state — retracted a selection, moved a sample rate, selected a tile —
        // has somewhere to show up. A hovering run whose only rest came before
        // the gesture would compare two windows in the same state.
        if hover {
            rest_at(&mut app, &ctx, egui::pos2(1.0, 1.0));
            let before = executes(&app);
            rest_at(&mut app, &ctx, landed);
            hovers += executes(&app) - before;
        }
        // No click after the sweep, deliberately: a click with no sweep on an
        // interval binding is the crossfilter's clear gesture, so one here
        // would retract the brush this comparison is about. The press that
        // began the sweep has already selected the hero's column, so the
        // inspector rail has a subject to draw either way.
        settle(&mut app, &ctx);
        (surfaces(&mut app), hovers)
    };

    let (with, hovers) = run(true);
    let (without, none) = run(false);
    assert!(
        hovers >= 2,
        "the hovering run issued {hovers} hover queries, so it did not rest \
         both before and after the brush"
    );
    assert_eq!(none, 0, "the non-hovering run issued {none} hover queries");
    assert!(
        with.selection.is_some(),
        "the brush committed nothing, so the comparison is between two windows \
         that did nothing"
    );
    assert!(!with.grid.is_empty(), "the grid read no rows");
    assert_eq!(
        with, without,
        "a hover changed what the shipped surfaces do"
    );
}
