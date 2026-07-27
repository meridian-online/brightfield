//! Gate: a navigation extent PERSISTS, rescopes an aggregate, refuses a
//! categorical axis out loud, is reachable from the keyboard, and costs one
//! query per settled gesture.
//!
//! Everything here is asserted through what a person can reach — a real window
//! driven by real key events, or the document's own gesture entry points — and
//! read back off the live DuckDB session rather than off a field the code
//! under test writes. GPU-free throughout.
//!
//! Five ways this feature can look finished and be broken, one section each:
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
//! - **A categorical axis does nothing, silently.** A band axis has no range to
//!   pan. Saying so is the difference between a considered refusal and a dead
//!   control.
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
    assert!(doc.navigated(), "and the frame is still reported as navigated");
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
/// implementation therefore drops every bin and draws an empty plot, where
/// this one recounts bin 0 to the ninety rows actually inside the range. The
/// two answers are 90 and 0.
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
    let composed = compose_spec(
        example("cell.yaml")
            .to_str()
            .expect("utf-8 path"),
    )
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
    assert!(doc
        .nav_notice()
        .is_some_and(|n| n.contains("y axis only")));
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
        .and_then(|p| p.scales.get(brightfield_render::channel::Channel::X).cloned())
        .expect("an x scale");
    frame(&mut app, &ctx, vec![press(Key::ArrowRight)]);
    let after_pan = app
        .chart_doc()
        .composed
        .plots
        .first()
        .and_then(|p| p.scales.get(brightfield_render::channel::Channel::X).cloned())
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
        after_pan.domain_min().expect("continuous")
            > before_pan.domain_min().expect("continuous"),
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
    let queried = live.query_extents().get(&path).expect("the rows are scoped");
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
