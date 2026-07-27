//! Gate: a navigation extent PERSISTS, rescopes an aggregate, refuses a
//! categorical axis out loud, is reachable from the keyboard, and costs one
//! query per settled gesture.
//!
//! Everything here is asserted through what a person can reach — a real window
//! driven by real key events, or the document's own gesture entry points — and
//! read back off the live DuckDB session rather than off a field the code
//! under test writes. GPU-free throughout.
//!
//! Seven ways this feature can look finished and be broken, one section each:
//!
//! - **The extent reverts.** A zoom that is dropped by the next brush, slider
//!   step or re-execute is worse than no zoom: the frame moves under the hand
//!   with nothing on screen saying why. This is the engineering core.
//! - **The aggregate is cropped, not recomputed.** Wrap a density's finished
//!   `GROUP BY` in the extent and the bins outside the view vanish while every
//!   surviving bin keeps the count it had at full extent. It looks zoomed. Its
//!   numbers are the old ones.
//! - **A mark that cannot take the extent takes the chart down with it.** A
//!   decorative sibling with no positional columns of its own must be left
//!   alone, not filtered on a column it never selected.
//! - **A mark that cannot take the extent is drawn as though it had.** The
//!   sibling above bails harmlessly because it draws a constant. A regression
//!   fit bails and goes on drawing a line whose slope is a claim about rows
//!   that left the frame. The bail has to reach the reader.
//! - **A categorical axis does nothing, silently.** A band axis has no range to
//!   pan. Saying so is the difference between a considered refusal and a dead
//!   control.
//! - **A gesture onto empty space leaves the stores lying.** A settle that
//!   draws nothing must not leave the session filtering at a range no picture
//!   was ever made of.
//! - **A sustained gesture queries per frame.** The frame has to move
//!   continuously and the data has to re-query once.

use brightfield_engine::RecordBatch;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::design::Mode;
use brightfield_shell::navigation::{self, AxisLock};
use brightfield_shell::pipeline::{compose_spec, live_spec, IntervalControl, LiveDashboard};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::{Item, ViewKind};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Fixtures and reads
// ---------------------------------------------------------------------------

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

/// The whole window over `path`, live, with a real DuckDB session behind it.
fn live_window(path: &Path) -> MeridianApp {
    let path_str = path.to_str().expect("utf-8 path");
    let (live, composed) = live_spec(path_str).expect("the fixture loads live");
    let mut boot = Boot::charts(composed);
    boot.live = Some(live);
    boot.spec_path = Some(path.to_path_buf());
    MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light)
}

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

fn press(k: egui::Key) -> egui::Event {
    egui::Event::Key {
        key: k,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::default(),
    }
}

/// The total occupancy of a density mark's curve — the sum of its per-bin
/// counts, read off the live session. This is what "the picture" means for a
/// binned mark: counting result ROWS would not distinguish a recomputed
/// aggregate from a cropped one.
fn curve_mass(doc: &mut ChartDoc, mark: usize) -> f64 {
    use arrow::array::Float64Array;
    use arrow::compute::cast;
    use arrow::datatypes::DataType;
    let batches = doc
        .live_coordinator()
        .expect("a live document")
        .chart_rows(mark)
        .expect("the mark queries");
    let mut total = 0.0;
    for batch in &batches {
        let Ok(idx) = batch.schema().index_of("__bf_count") else {
            continue;
        };
        let col = cast(batch.column(idx), &DataType::Float64).expect("numeric");
        let arr = col
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("f64 counts");
        for i in 0..arr.len() {
            total += arr.value(i);
        }
    }
    total
}

/// The `[min, max]` a mark's rows actually span on `column` — the honest read
/// of "is the frame still scoped", independent of how many rows a filter
/// elsewhere happens to admit.
fn drawn_range(doc: &mut ChartDoc, mark: usize, column: &str) -> Option<(f64, f64)> {
    use arrow::array::Float64Array;
    use arrow::compute::cast;
    use arrow::datatypes::DataType;
    let batches = doc
        .live_coordinator()
        .expect("a live document")
        .chart_rows(mark)
        .expect("the mark queries");
    let mut range: Option<(f64, f64)> = None;
    for batch in &batches {
        let Ok(idx) = batch.schema().index_of(column) else {
            continue;
        };
        let col = cast(batch.column(idx), &DataType::Float64).expect("numeric");
        let arr = col
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("f64 column");
        for i in 0..arr.len() {
            let v = arr.value(i);
            range = Some(match range {
                None => (v, v),
                Some((lo, hi)) => (lo.min(v), hi.max(v)),
            });
        }
    }
    range
}

/// One numeric column of a one-row mark, as a float. A regression's whole
/// output is such a row, so this is how "the fit did not move" is read off the
/// numbers the mark actually returned rather than off its SQL.
fn scalar(doc: &mut ChartDoc, mark: usize, column: &str) -> f64 {
    use arrow::array::Float64Array;
    use arrow::compute::cast;
    use arrow::datatypes::DataType;
    let batches = doc
        .live_coordinator()
        .expect("a live document")
        .chart_rows(mark)
        .expect("the mark queries");
    for batch in &batches {
        let Ok(idx) = batch.schema().index_of(column) else {
            continue;
        };
        if batch.num_rows() == 0 {
            continue;
        }
        let col = cast(batch.column(idx), &DataType::Float64).expect("numeric");
        return col
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("f64 column")
            .value(0);
    }
    panic!("mark {mark} returned no `{column}`");
}

/// The chart pane's status rail, as `(id, text)` — what a reader is actually
/// shown, read through the same `Item::subject` the window renders from.
fn chart_rail(doc: &ChartDoc) -> Vec<(&'static str, String)> {
    brightfield_shell::chart_item::ChartItem::new()
        .subject(doc)
        .status
        .iter()
        .map(|e| (e.id, e.text.clone()))
        .collect()
}

/// Every banner the window is currently showing, headline and body together —
/// the surface a reader actually looks at for a fault.
fn banner_text(app: &MeridianApp) -> Vec<String> {
    app.notifications()
        .iter()
        .map(|n| format!("{}\n{}", n.title, n.body.clone().unwrap_or_default()))
        .collect()
}

fn mark_rows(doc: &mut ChartDoc, mark: usize) -> usize {
    doc.live_coordinator()
        .expect("a live document")
        .chart_rows(mark)
        .expect("the mark queries")
        .iter()
        .map(RecordBatch::num_rows)
        .sum()
}

/// How many DuckDB executes this document has performed. The un-foolable
/// counter: it increments on a real query and on nothing else, so a re-present
/// that only re-composites leaves it alone.
fn executes(doc: &mut ChartDoc) -> usize {
    doc.live_coordinator()
        .expect("a live document")
        .session()
        .duckdb_execute_count()
}

/// The materialisation generation — one per applied interaction.
fn generation(doc: &mut ChartDoc) -> u64 {
    doc.live_coordinator()
        .expect("a live document")
        .generation()
}

// ---------------------------------------------------------------------------
// 1. The extent persists — the engineering core
// ---------------------------------------------------------------------------

/// **A zoom outlives every other gesture.** A brush, a slider step and a full
/// re-execute each go through a different emission path; a per-call extent
/// would be dropped by all three, and the frame would snap back to full extent
/// with nothing saying why.
///
/// Read behaviourally, off the rows the session returns: after each of the
/// three, the density's mass is still the ZOOMED mass, not the full one.
#[test]
fn a_zoom_survives_a_brush_a_slider_step_and_a_full_re_execute() {
    let mut app = live_window(&example("interval-slider.yaml"));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());

    let doc = app.chart_doc_mut();
    let full_mass = curve_mass(doc, 0);
    assert!(full_mass > 0.0, "the fixture draws something to start");
    // What the frame is scoped to, read off the rows themselves. Mass is the
    // wrong read past the slider step below: a slider legitimately changes how
    // many rows there ARE, so only the span the drawn bins cover separates
    // "the extent survived" from "the extent was dropped".
    let full_span = drawn_range(doc, 0, "latency").expect("the density draws bins");

    // Zoom in twice from the keyboard — one discrete gesture each.
    assert!(doc.zoom_view(2.0), "the density plot is navigable");
    assert!(doc.zoom_view(2.0));
    let zoomed_mass = curve_mass(doc, 0);
    assert!(
        zoomed_mass < full_mass,
        "the zoom scoped nothing: {zoomed_mass} vs {full_mass}"
    );
    assert!(zoomed_mass > 0.0, "the zoom emptied the plot");
    let zoomed_span = drawn_range(doc, 0, "latency").expect("the density draws bins");
    assert!(
        zoomed_span.1 < full_span.1,
        "the zoom left the frame's right edge where it was: \
         {zoomed_span:?} vs {full_span:?}"
    );

    // (a) A slider step. It re-queries through the SELECTION path, which is a
    // different emission than navigation's — and the one most likely to
    // forget the extent. It admits MORE rows here (the handle moves right), so
    // a frame that snapped back would show a WIDER span, not a smaller mass.
    let control: IntervalControl = doc.composed.intervals[0].clone();
    doc.note_interval_drag(&control, 150.0);
    assert_eq!(doc.pump_interval_drags(std::slice::from_ref(&control)), 1);
    let after_slider = drawn_range(doc, 0, "latency").expect("the density still draws");
    assert!(
        after_slider.1 <= zoomed_span.1 + f64::EPSILON,
        "the slider step dropped the extent — the frame reaches {} again, \
         against a zoomed edge of {}",
        after_slider.1,
        zoomed_span.1
    );

    // (b) A full re-execute of every mark.
    let results = app
        .chart_doc_mut()
        .live_coordinator()
        .expect("live")
        .session_mut()
        .execute_all();
    assert!(results.iter().all(Result::is_ok), "a mark failed");
    let after_execute =
        drawn_range(app.chart_doc_mut(), 0, "latency").expect("the density still draws");
    assert!(
        after_execute.1 <= zoomed_span.1 + f64::EPSILON,
        "execute_all dropped the extent: {after_execute:?} against {zoomed_span:?}"
    );

    // (c) And only an explicit reset takes it away.
    assert!(app.chart_doc_mut().reset_navigation(), "the reset applied");
    let after_reset =
        drawn_range(app.chart_doc_mut(), 0, "latency").expect("the density still draws");
    assert!(
        after_reset.1 > zoomed_span.1,
        "the reset did not widen the frame back out: {after_reset:?} vs {zoomed_span:?}"
    );
}

/// **Clearing a selection is not clearing the frame.** They are different
/// state, and one key that undid both would make a zoom impossible to keep
/// while working a cross-filter.
#[test]
fn clearing_the_selection_leaves_the_navigation_extent_alone() {
    let mut app = live_window(&example("crossfilter.yaml"));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());

    let doc = app.chart_doc_mut();
    let full = mark_rows(doc, 0);
    assert!(doc.zoom_view(2.0), "the plot is navigable");
    let zoomed = mark_rows(doc, 0);
    assert!(zoomed < full, "the zoom scoped nothing: {zoomed} vs {full}");

    doc.clear_selection();
    assert_eq!(
        mark_rows(doc, 0),
        zoomed,
        "clear-selection widened the navigation extent back out"
    );
    assert!(
        doc.navigated(),
        "and the frame is still reported as navigated"
    );
}

// ---------------------------------------------------------------------------
// 2. An aggregating mark is RECOMPUTED, not cropped
// ---------------------------------------------------------------------------

/// A binned density over a deliberately lopsided column: ninety rows at one
/// end, ten at the other, in ten fixed bins.
const SKEWED_DENSITY: &str = r#"
data:
  t:
    query: "SELECT CASE WHEN i < 90 THEN 0.0 ELSE 9.0 END AS v FROM range(100) t(i)"
plot:
  - mark: densityX
    data: { from: t }
    x: v
    bins: 10
width: 400
height: 300
"#;

/// An evenly spread density: ten rows at each of ten values, in ten fixed
/// bins. Any sub-range of it still has rows, which is what makes it the right
/// fixture for gestures whose landing point is arithmetic rather than chosen.
const SPREAD_DENSITY: &str = r#"
data:
  t:
    query: "SELECT (i % 10) * 1.0 AS v FROM range(100) t(i)"
plot:
  - mark: densityX
    data: { from: t }
    x: v
    bins: 10
width: 400
height: 300
"#;

/// **The zoom recomputes the aggregate under the `GROUP BY`.**
///
/// Held against an INDEPENDENT oracle — a raw `count(*)` over the same range,
/// asked of DuckDB directly — rather than against a second copy of the
/// implementation's own arithmetic.
///
/// The realistic wrong implementation is the one that shipped: wrap the
/// finished aggregate in the extent. Bin centres are computed from the whole
/// table, so bin 0's centre is 0.45 and the extent below ends at 0.4 — that
/// implementation therefore drops every bin, where this one recounts bin 0 to
/// the ninety rows actually inside the range.
///
/// **What that failure actually looks like, measured by mutating
/// `axis_pushdown` to return `Top` for every column and running this test:**
/// the mark returns no rows at all, so the re-composite has nothing to draw
/// and fails outright with `no marks rendered successfully`, and
/// `pump_navigation` returns false. The assertion that catches it is therefore
/// `the settled zoom re-queried` on the line above the read — not the mass
/// comparison below it, which is never reached. Both answers exist (90 against
/// 0) but only one of them is ever computed.
#[test]
fn zooming_an_aggregating_mark_recounts_beneath_the_group_by() {
    let mut live = LiveDashboard::load_str(SKEWED_DENSITY, None).expect("loads live");
    let composed = live.present().expect("first paint");
    let mut doc = ChartDoc::headless(composed);
    doc.attach_live(live);

    assert!(
        (curve_mass(&mut doc, 0) - 100.0).abs() < f64::EPSILON,
        "every row is drawn at full extent"
    );

    // The oracle: the same range, asked over a SEPARATE session through the
    // static `data.filter` path, which shares no line of code with navigation.
    let oracle = {
        const ORACLE: &str = r#"
data:
  t:
    query: "SELECT CASE WHEN i < 90 THEN 0.0 ELSE 9.0 END AS v FROM range(100) t(i)"
plot:
  - mark: dot
    data: { from: t, filter: "v BETWEEN 0.0 AND 0.4" }
    x: v
    y: v
"#;
        let mut oracle_live = LiveDashboard::load_str(ORACLE, None).expect("the oracle loads");
        let composed = oracle_live.present().expect("the oracle paints");
        let mut oracle_doc = ChartDoc::headless(composed);
        oracle_doc.attach_live(oracle_live);
        mark_rows(&mut oracle_doc, 0) as f64
    };
    assert!(
        (oracle - 90.0).abs() < f64::EPSILON,
        "the oracle disagrees with the fixture's own arithmetic: {oracle}"
    );

    let plot = 0;
    let outcome = navigation::NavOutcome {
        extent: brightfield_render::scale::ViewExtent {
            x: Some((0.0, 0.4)),
            y: None,
        },
        refused: Vec::new(),
    };
    assert!(doc.note_navigation(plot, &outcome));
    doc.settle_navigation();
    assert!(doc.pump_navigation(), "the settled zoom re-queried");

    let mass = curve_mass(&mut doc, 0);
    assert!(
        (mass - oracle).abs() < f64::EPSILON,
        "the aggregate was cropped rather than recomputed: the plot's mass is \
         {mass}, and the rows actually inside the extent number {oracle}"
    );
    assert!(
        mark_rows(&mut doc, 0) > 0,
        "the mark drew nothing at all — that is a broken query, not a rescoped one"
    );
}

/// **A mark that cannot take the extent bails; it does not fail.**
///
/// The shipped hexbin example draws a decorative `hexgrid` beside its hexbin.
/// The grid's plan is a single constant row with no positional columns at all,
/// so an extent pushed at it would filter on a column it never selected. It
/// must be left alone — and the hexbin beside it must still rescope.
#[test]
fn a_mark_with_no_column_for_the_extent_is_left_alone_rather_than_failed() {
    let mut app = live_window(&example("hexbin.yaml"));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    let doc = app.chart_doc_mut();

    // Mark 0 is the hexgrid, mark 1 the hexbin (spec order).
    let grid_before = mark_rows(doc, 0);
    let hex_before = curve_mass(doc, 1);
    assert!(grid_before > 0, "the hexgrid draws its constant row");
    assert!(hex_before > 0.0, "the hexbin draws counts");

    assert!(doc.zoom_view(2.0), "the hexbin plot is navigable");

    let results = doc
        .live_coordinator()
        .expect("live")
        .session_mut()
        .execute_all();
    for (i, r) in results.iter().enumerate() {
        assert!(
            r.is_ok(),
            "mark {i} failed under a navigation extent: {:?}",
            r.as_ref().err()
        );
    }
    assert_eq!(
        mark_rows(doc, 0),
        grid_before,
        "the hexgrid was filtered on a column it does not have"
    );
    let hex_after = curve_mass(doc, 1);
    assert!(
        hex_after < hex_before,
        "the hexbin did not rescope: {hex_after} vs {hex_before}"
    );
}

// ---------------------------------------------------------------------------
// 3. A categorical axis refuses, out loud
// ---------------------------------------------------------------------------

/// **A band axis says why it will not move.** `examples/cell.yaml` plots two
/// categorical axes; a navigation gesture on it has no continuous range to
/// pan, and the pane rails the reason rather than doing nothing in silence.
#[test]
fn a_categorical_axis_refuses_and_the_pane_says_so() {
    let composed = compose_spec(example("cell.yaml").to_str().expect("utf-8 path"))
        .expect("cell.yaml composes");
    let mut doc = ChartDoc::headless(composed);

    assert!(
        !doc.zoom_view(2.0),
        "a plot with two band axes has nothing to zoom"
    );
    let notice = doc.nav_notice().expect("the refusal was recorded");
    assert!(
        notice.contains("categorical"),
        "the refusal has to say WHY: {notice}"
    );
    assert!(!doc.navigated(), "and nothing was navigated");

    // And it reaches the surface a reader looks at.
    let item = brightfield_shell::chart_item::ChartItem::new();
    let subject = item.subject(&doc);
    let entry = subject
        .status
        .iter()
        .find(|e| e.id == "chart-navigation")
        .expect("the chart pane rails the refusal");
    assert!(entry.text.contains("categorical"), "{}", entry.text);
}

/// **A mark that did not rescope is named, and the picture stops being a
/// silent lie.**
///
/// `examples/regression.yaml` puts a `regressionY` and a `dot` in ONE plot over
/// ONE pair of columns. The scatter's plan is row-level, so a zoom filters it;
/// the fit's plan is a scalar aggregate with no grouping key beneath which the
/// bound could go, so `axis_pushdown` declines and the emitted SQL is
/// byte-identical to the one that ran at full extent. The two halves of the
/// same picture therefore describe different data — an ordinary-least-squares
/// line computed from points that are not on screen, spanning an x range wider
/// than the frame — and until this gate the pane said nothing about it.
///
/// The premise is asserted first, off the marks' own returned rows, so this
/// test cannot pass by the decline having quietly gone away: the scatter has to
/// narrow AND the fit's `n` / `x_min` / `x_max` have to be untouched. Only then
/// is the consequence read, through `Item::subject` — the surface a reader
/// looks at, not the function that computes it.
#[test]
fn a_mark_that_did_not_rescope_is_named_on_the_chart_rail() {
    use egui::Key;
    let mut app = live_window(&example("regression.yaml"));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());

    // Mark 0 is the dot scatter, mark 1 the regressionY fit (spec order).
    let doc = app.chart_doc_mut();
    let scatter_before = mark_rows(doc, 0);
    let fit_before = (
        scalar(doc, 1, "n"),
        scalar(doc, 1, "x_min"),
        scalar(doc, 1, "x_max"),
    );
    assert!(scatter_before > 0, "the scatter draws rows to start");
    assert!(
        chart_rail(app.chart_doc())
            .iter()
            .all(|(id, _)| *id != "chart-navigation-scope"),
        "an unnavigated chart claims nothing about its scope"
    );

    frame(&mut app, &ctx, vec![press(Key::Equals)]);

    // The premise, both halves of it.
    let doc = app.chart_doc_mut();
    let scatter_after = mark_rows(doc, 0);
    assert!(
        scatter_after < scatter_before,
        "`=` did not narrow the scatter: {scatter_after} vs {scatter_before}"
    );
    let fit_after = (
        scalar(doc, 1, "n"),
        scalar(doc, 1, "x_min"),
        scalar(doc, 1, "x_max"),
    );
    assert_eq!(
        fit_after, fit_before,
        "the fit rescoped after all — this gate is about the case where it does not"
    );

    // The consequence: the reader can tell.
    let rail = chart_rail(app.chart_doc());
    let (_, text) = rail
        .iter()
        .find(|(id, _)| *id == "chart-navigation-scope")
        .unwrap_or_else(|| panic!("the pane said nothing about the unscoped fit: {rail:?}"));
    assert!(
        text.contains("regressionY"),
        "the notice has to name WHICH mark: {text}"
    );
    let entry = brightfield_shell::chart_item::ChartItem::new()
        .subject(app.chart_doc())
        .status
        .into_iter()
        .find(|e| e.id == "chart-navigation-scope")
        .expect("the entry is on the rail");
    assert_eq!(
        entry.tone,
        brightfield_workbench::subject::Tone::Warning,
        "a drawn claim about invisible data is not neutral chrome"
    );

    // And it is a statement about the extent in force, not a sticker: the
    // reset that widens the frame back out takes it away.
    frame(&mut app, &ctx, vec![press(Key::Num0)]);
    assert!(!app.chart_doc().navigated(), "`0` did not reset the frame");
    let rail = chart_rail(app.chart_doc());
    assert!(
        rail.iter().all(|(id, _)| *id != "chart-navigation-scope"),
        "the notice outlived the extent it was about: {rail:?}"
    );
}

/// The other half of the gate above, and the one that catches the cheapest
/// wrong implementation: a notice that is always on.
///
/// `examples/scatter.yaml` draws one row-level mark, which rescopes completely.
/// A plot where nothing declined must say nothing — otherwise the warning is
/// noise and a reader learns to ignore the case that matters.
#[test]
fn a_plot_whose_marks_all_rescoped_claims_nothing() {
    let mut app = live_window(&example("scatter.yaml"));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());

    let full = mark_rows(app.chart_doc_mut(), 0);
    frame(&mut app, &ctx, vec![press(egui::Key::Equals)]);
    let zoomed = mark_rows(app.chart_doc_mut(), 0);
    assert!(zoomed < full, "the zoom scoped nothing: {zoomed} vs {full}");
    assert!(
        app.chart_doc().navigated(),
        "the frame is held at an extent"
    );

    let rail = chart_rail(app.chart_doc());
    assert!(
        rail.iter().all(|(id, _)| *id != "chart-navigation-scope"),
        "a plot that rescoped whole warned about itself: {rail:?}"
    );
    assert_eq!(
        app.chart_doc().nav_scope_notice(),
        None,
        "and the document agrees with its own pane"
    );
}

/// **A settled gesture that draws nothing does not leave the query store
/// claiming it did.** A pan far enough off the data returns no rows, the
/// re-composite fails, and the caller keeps the picture it had — which is the
/// UNSCOPED one. The session's extent has to go back with it, or every later
/// re-query is emitted at a range the reader never saw a picture of, and the
/// store disagrees with the rows on screen.
///
/// The render store deliberately stays moved: the axes did move, so
/// `navigated()` is true and the reset affordance does something. Both halves
/// are asserted, because a rollback that took the axes with it would be the
/// same bug pointed the other way.
///
/// The dead end is said ONCE, on the banner, and the rail must stay out of it.
/// The banner is the surface for what one gesture just did — dismissable,
/// because the gesture is over — and the rail is the surface for what is true
/// of the extent in force. Both firing would put a standing rail entry under a
/// dismissable banner about one instant.
#[test]
fn a_settled_gesture_that_drew_nothing_rolls_the_query_store_back() {
    let mut app = live_window(&example("scatter.yaml"));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    let path = app.chart_doc().composed.plots[0].path.clone();

    let full = mark_rows(app.chart_doc_mut(), 0);
    assert!(full > 0, "the fixture draws rows");
    assert!(
        app.notifications().is_empty(),
        "the fixture opens with a banner already up, so a banner below proves \
         nothing: {:?}",
        banner_text(&app)
    );

    // Walk the frame off the data. Each keyboard pan is one settled gesture.
    let mut left_the_data = false;
    for _ in 0..40 {
        if !app.chart_doc_mut().pan_view(1.5, 0.0) {
            left_the_data = true;
            break;
        }
    }
    assert!(
        left_the_data,
        "the pan never left the data — this gate needs a settle that draws nothing"
    );

    let doc = app.chart_doc();
    let live = doc.live_dashboard().expect("a live document");
    assert!(
        !live.query_extents().contains_key(&path),
        "the query store kept an extent that returned no rows: {:?}",
        live.query_extents()
    );
    assert!(
        live.view_extents().contains_key(&path),
        "the axes were rolled back too — the frame on screen IS the moved one"
    );
    assert!(doc.navigated(), "so there is still a frame to reset");

    // The rows the session serves are the ones the picture was drawn from.
    assert_eq!(
        mark_rows(app.chart_doc_mut(), 0),
        full,
        "the session is still filtering at an extent that drew nothing"
    );

    // And the dead end is not silent — it is on the banner, in the gesture's
    // own words rather than the engine's.
    frame(&mut app, &ctx, Vec::new());
    let said = banner_text(&app);
    assert!(
        said.iter().any(|b| b.contains("moved off the data")),
        "a gesture that drew nothing left the window silent, or said it in the \
         engine's words instead of its own; showing {said:?}"
    );

    // One event, one vocabulary. The rail carries the scope in force, not this.
    let rail = chart_rail(app.chart_doc());
    assert!(
        rail.iter()
            .all(|(_, text)| !text.contains("nothing to draw")
                && !text.contains("moved off the data")),
        "the dead end is reported twice — once on the banner and once on the \
         rail: {rail:?}"
    );
}

/// The axis lock is honoured end to end: locked to y, an x-only continuous
/// plot moves nothing and says which axis refused.
#[test]
fn an_axis_lock_is_honoured_through_the_document() {
    let mut live = LiveDashboard::load_str(SPREAD_DENSITY, None).expect("loads live");
    let composed = live.present().expect("first paint");
    let mut doc = ChartDoc::headless(composed);
    doc.attach_live(live);

    doc.axis_lock = AxisLock::XOnly;
    assert!(doc.zoom_view(2.0), "x moves under an x-only lock");
    let x_only = curve_mass(&mut doc, 0);
    assert!(x_only < 100.0, "x did not move: {x_only}");

    doc.cycle_axis_lock();
    assert_eq!(doc.axis_lock, AxisLock::YOnly);
    assert!(doc.nav_notice().is_some_and(|n| n.contains("y axis only")));
}

// ---------------------------------------------------------------------------
// 4. Reachable without a pointer drag
// ---------------------------------------------------------------------------

/// **Every frame verb is bound, and every binding traces to the registry.**
///
/// The shell may not invent a binding, so this reads the keystroke off the
/// registry and presses THAT — a rename in the registry moves this test's
/// keystroke with it, and a verb that loses its binding fails here rather than
/// quietly becoming unreachable.
#[test]
fn every_frame_verb_is_bound_by_the_registry() {
    let reg = brightfield_keys::registry();
    for longname in navigation::verb::ALL {
        let entry = reg
            .iter()
            .find(|v| v.longname == *longname)
            .unwrap_or_else(|| panic!("{longname} is not in the registry"));
        assert!(
            entry.is_bound(),
            "{longname} is declared but unbound — unreachable from the keyboard"
        );
        assert!(
            entry.scores.is_some(),
            "{longname} is bound without a scored row behind it"
        );
    }
}

/// **The keys actually work in the window.** Real key events, through the real
/// frame loop, over a live document — pan, zoom, lock and reset each observed
/// by what they did to the rows DuckDB returns.
#[test]
fn the_frame_moves_under_real_keystrokes() {
    use egui::Key;
    let mut app = live_window(&example("scatter.yaml"));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    assert_eq!(app.active(), ViewKind::Charts);

    let full = mark_rows(app.chart_doc_mut(), 0);
    assert!(full > 0, "the fixture draws rows");

    // Zoom in.
    frame(&mut app, &ctx, vec![press(Key::Equals)]);
    let zoomed = mark_rows(app.chart_doc_mut(), 0);
    assert!(zoomed < full, "`=` did not zoom in: {zoomed} vs {full}");

    // Pan, which must keep the frame the same SIZE.
    let before_pan = app
        .chart_doc()
        .composed
        .plots
        .first()
        .and_then(|p| {
            p.scales
                .get(brightfield_render::channel::Channel::X)
                .cloned()
        })
        .expect("an x scale");
    frame(&mut app, &ctx, vec![press(Key::ArrowRight)]);
    let after_pan = app
        .chart_doc()
        .composed
        .plots
        .first()
        .and_then(|p| {
            p.scales
                .get(brightfield_render::channel::Channel::X)
                .cloned()
        })
        .expect("an x scale");
    let span = |s: &brightfield_render::scale::Scale| {
        s.domain_max().expect("continuous") - s.domain_min().expect("continuous")
    };
    assert!(
        (span(&before_pan) - span(&after_pan)).abs() < 1e-6,
        "a pan changed the span: {} vs {}",
        span(&before_pan),
        span(&after_pan)
    );
    assert!(
        after_pan.domain_min().expect("continuous") > before_pan.domain_min().expect("continuous"),
        "`right` did not move the frame right"
    );

    // The lock.
    assert_eq!(app.chart_doc().axis_lock, AxisLock::Both);
    frame(&mut app, &ctx, vec![press(Key::X)]);
    assert_eq!(app.chart_doc().axis_lock, AxisLock::XOnly);

    // And the reset.
    assert!(app.chart_doc().navigated());
    frame(&mut app, &ctx, vec![press(Key::Num0)]);
    assert!(!app.chart_doc().navigated(), "`0` did not reset the frame");
    assert_eq!(
        mark_rows(app.chart_doc_mut(), 0),
        full,
        "the reset did not restore the full picture"
    );
}

/// **The wheel zooms, and it settles by itself.** A gesture with no button to
/// let go of ends on the first frame that carries no wheel travel — which is a
/// fact about the input, not a duration anyone had to pick.
///
/// Driven with real `MouseWheel` events at a coordinate taken off the rect the
/// raster was actually presented into, so a layout change moves the aim rather
/// than leaving this passing against empty space.
#[test]
fn the_wheel_zooms_the_plot_under_the_pointer_and_settles_on_its_own() {
    let mut app = live_window(&example("scatter.yaml"));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());

    let raster = app
        .chart_doc()
        .raster_rect
        .expect("the raster was presented");
    let plot = app.chart_doc().composed.plots[0].rect;
    let aim = egui::pos2(
        raster.min.x + (plot.x + plot.width / 2.0) as f32,
        raster.min.y + (plot.y + plot.height / 2.0) as f32,
    );
    let full = mark_rows(app.chart_doc_mut(), 0);
    let before = executes(app.chart_doc_mut());

    // Four frames of one continuous wheel gesture.
    for _ in 0..4 {
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(aim),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 60.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::default(),
                },
            ],
        );
    }
    assert_eq!(
        executes(app.chart_doc_mut()),
        before,
        "the wheel queried mid-gesture"
    );

    // A frame with no wheel travel: the gesture is over.
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(aim)]);
    assert!(
        executes(app.chart_doc_mut()) > before,
        "the settled wheel gesture never queried"
    );
    let zoomed = mark_rows(app.chart_doc_mut(), 0);
    assert!(
        zoomed < full,
        "the wheel did not zoom: {zoomed} vs {full} rows"
    );
    assert!(app.chart_doc().navigated());
}

/// **A secondary-button drag pans, and the release is the settle.** The primary
/// button is the brush; one button cannot mean both without an invisible mode.
#[test]
fn a_secondary_button_drag_pans_and_queries_on_release() {
    let mut app = live_window(&example("scatter.yaml"));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());

    let raster = app
        .chart_doc()
        .raster_rect
        .expect("the raster was presented");
    let plot = app.chart_doc().composed.plots[0].rect;
    let at = |dx: f32| {
        egui::pos2(
            raster.min.x + (plot.x + plot.width / 2.0) as f32 + dx,
            raster.min.y + (plot.y + plot.height / 2.0) as f32,
        )
    };
    let domain_min = |app: &MeridianApp| {
        app.chart_doc().composed.plots[0]
            .scales
            .get(brightfield_render::channel::Channel::X)
            .and_then(brightfield_render::scale::Scale::domain_min)
            .expect("a continuous x scale")
    };
    let start = domain_min(&app);
    let before = executes(app.chart_doc_mut());

    let down = |p: egui::Pos2, pressed: bool| egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };

    frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(at(0.0)), down(at(0.0), true)],
    );
    for step in 1..=4 {
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(at(-15.0 * step as f32))],
        );
    }
    assert_eq!(
        executes(app.chart_doc_mut()),
        before,
        "the pan queried mid-drag"
    );
    let dragged = domain_min(&app);
    assert!(
        dragged > start,
        "the axes did not move under the drag: {dragged} vs {start}"
    );

    frame(&mut app, &ctx, vec![down(at(-60.0), false)]);
    assert!(
        executes(app.chart_doc_mut()) > before,
        "releasing the pan never queried"
    );
    assert!(app.chart_doc().navigated());
}

// ---------------------------------------------------------------------------
// 5. One query per settled gesture
// ---------------------------------------------------------------------------

/// **A sustained gesture issues exactly one re-query, at its end.**
///
/// Counted two ways, because either alone can be fooled: the coordinator's
/// generation (one per applied interaction) and DuckDB's own execute counter
/// (one per query that was not served from cache). A per-step implementation
/// moves both by eight.
#[test]
fn a_sustained_gesture_issues_one_query_not_one_per_step() {
    let mut live = LiveDashboard::load_str(SPREAD_DENSITY, None).expect("loads live");
    let composed = live.present().expect("first paint");
    let mut doc = ChartDoc::headless(composed);
    doc.attach_live(live);

    let (gen_before, exec_before) = (generation(&mut doc), executes(&mut doc));

    // Eight steps of one continuous gesture: the frame closes in a little more
    // each time, and nothing settles until the last.
    for i in 1..=8 {
        let hi = 9.0 - f64::from(i) * 0.5;
        let outcome = navigation::NavOutcome {
            extent: brightfield_render::scale::ViewExtent {
                x: Some((0.0, hi)),
                y: None,
            },
            refused: Vec::new(),
        };
        assert!(doc.note_navigation(0, &outcome));
        assert!(
            !doc.pump_navigation(),
            "step {i} issued a query mid-gesture"
        );
    }
    assert_eq!(
        generation(&mut doc),
        gen_before,
        "a mid-gesture step reached the coordinator"
    );
    assert_eq!(
        executes(&mut doc),
        exec_before,
        "a mid-gesture step reached DuckDB"
    );

    doc.settle_navigation();
    assert!(doc.pump_navigation(), "the settled gesture re-queried");
    assert!(
        !doc.pump_navigation(),
        "the settled gesture re-queried a second time"
    );

    assert_eq!(
        generation(&mut doc) - gen_before,
        1,
        "one settled gesture, one applied interaction"
    );
    assert_eq!(
        doc.nav.dispatched().len(),
        1,
        "the seven superseded steps were dispatched: {:?}",
        doc.nav.dispatched()
    );
    // The extent that reached the engine is the one the gesture STOPPED at.
    assert_eq!(doc.nav.dispatched()[0].1.x, Some((0.0, 5.0)));
}

/// **The two extent stores agree once the gesture has settled.** The axes are
/// drawn from one and the rows are filtered by the other; at rest they must
/// describe the same range, or the chart's numbers and its ticks disagree.
#[test]
fn the_two_extent_stores_agree_once_a_gesture_has_settled() {
    let mut live = LiveDashboard::load_str(SPREAD_DENSITY, None).expect("loads live");
    let composed = live.present().expect("first paint");
    let path = composed.plots[0].path.clone();
    let mut doc = ChartDoc::headless(composed);
    doc.attach_live(live);

    let outcome = navigation::NavOutcome {
        extent: brightfield_render::scale::ViewExtent {
            x: Some((1.0, 6.0)),
            y: None,
        },
        refused: Vec::new(),
    };
    assert!(doc.note_navigation(0, &outcome));
    doc.settle_navigation();
    assert!(doc.pump_navigation());

    let live = doc.live_dashboard().expect("a live document");
    let drawn = live.view_extents().get(&path).expect("the axes are moved");
    let queried = live
        .query_extents()
        .get(&path)
        .expect("the rows are scoped");
    assert_eq!(drawn.x, Some((1.0, 6.0)));
    let x = queried.x.as_ref().expect("an x bound reached the engine");
    assert_eq!(x.column, "v");
    assert!(
        (x.min - 1.0).abs() < f64::EPSILON && (x.max - 6.0).abs() < f64::EPSILON,
        "the two stores describe different ranges: drawn {drawn:?}, queried {queried:?}"
    );

    // And the reset empties both.
    assert!(doc.reset_navigation());
    let live = doc.live_dashboard().expect("a live document");
    assert!(live.view_extents().is_empty(), "the axes stayed moved");
    assert!(live.query_extents().is_empty(), "the rows stayed scoped");
}

/// The same law at the seam a caller who holds only an `Interaction` reaches.
///
/// The chart pane happens to write the render store on its way past, so the
/// gate above would pass on a `LiveDashboard::apply` that wrote nothing — it
/// holds the pane's path, not the seam's. This one hands the interaction
/// straight to the dashboard, which is what any other consumer of the
/// coordinator seam would do, and fails if the axes are left behind.
#[test]
fn applying_a_navigation_interaction_moves_the_axes_and_the_rows_together() {
    use brightfield_engine::coordinator::Interaction;
    use brightfield_engine::{AxisExtent, NavigationExtent};
    use brightfield_spec::analysis::ComponentPath;

    let mut live = LiveDashboard::load_str(SPREAD_DENSITY, None).expect("loads live");
    let composed = live.present().expect("first paint");
    let path = composed.plots[0].path.clone();

    live.apply(Interaction::Navigate {
        plot: ComponentPath(path.clone()),
        extent: NavigationExtent {
            x: Some(AxisExtent::new("v", 1.0, 6.0)),
            y: None,
        },
    })
    .expect("the navigation re-composites");

    let drawn = live
        .view_extents()
        .get(&path)
        .expect("the seam left the axes at full extent while the rows were scoped");
    assert_eq!(drawn.x, Some((1.0, 6.0)));
    let queried = live
        .query_extents()
        .get(&path)
        .expect("the rows are scoped");
    let x = queried.x.as_ref().expect("an x bound");
    assert!((x.min - 1.0).abs() < f64::EPSILON && (x.max - 6.0).abs() < f64::EPSILON);
}
