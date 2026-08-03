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
    // Pinned, because it is QUOTED. The filter pass's module doc and
    // `Session::declined_navigation` both describe this exact gesture by its
    // numbers, and a figure in prose that nothing enforces is how a wrong one
    // shipped in the first place. The zoom is a function of the data's domain
    // and the step alone — no screen size in it — so this is stable: fifteen
    // rows in, the frame settles at weight 56.3–90.7 and height 161.1–185.9,
    // and ten rows satisfy both bounds.
    assert_eq!(
        (scatter_before, scatter_after),
        (15, 10),
        "the survivor count moved; the two doc comments that quote it have to \
         move with it"
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

/// **The refusal and the scope notice hold the rail at the same time.**
///
/// They are two entries rather than two vocabularies, and the reason is that
/// they differ in lifetime: the refusal is about the gesture just made and the
/// next one replaces it, the scope notice is about the extent in force and
/// stands until a reset. Given one id, whichever was written last would silence
/// the other — and both are true here, so the reader would lose one of them
/// with nothing to say it had gone.
///
/// Real keystrokes throughout: `=` to hold the plot at an extent its fit cannot
/// follow, then `x` to change which axes move, which is a refusal-shaped
/// statement about a gesture that has nothing to do with the first.
#[test]
fn a_refusal_and_a_scope_notice_can_hold_the_rail_at_once() {
    use egui::Key;
    let mut app = live_window(&example("regression.yaml"));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());

    frame(&mut app, &ctx, vec![press(Key::Equals)]);
    frame(&mut app, &ctx, vec![press(Key::X)]);

    let rail = chart_rail(app.chart_doc());
    let refusal = rail
        .iter()
        .find(|(id, _)| *id == "chart-navigation")
        .unwrap_or_else(|| panic!("the axis-lock change went unsaid: {rail:?}"));
    let scope = rail
        .iter()
        .find(|(id, _)| *id == "chart-navigation-scope")
        .unwrap_or_else(|| panic!("the unscoped fit went unsaid: {rail:?}"));
    assert!(
        refusal.1 != scope.1,
        "the two entries are carrying the same sentence, which means one of them \
         is not saying its own thing: {rail:?}"
    );
    assert!(
        scope.1.contains("regressionY"),
        "the scope notice lost its subject: {rail:?}"
    );
}

// ---------------------------------------------------------------------------
// The same fact, in the data ink
// ---------------------------------------------------------------------------

/// The mark colour a `dot` and a `regressionY` both take when nothing binds a
/// colour channel — Harbour slot 1, read from the token layer so a palette bump
/// moves the expectation with it.
fn mark_ink() -> [i32; 3] {
    let c = meridian_design::viz::MARK_DEFAULT_LIGHT;
    [
        (c.r * 255.0).round() as i32,
        (c.g * 255.0).round() as i32,
        (c.b * 255.0).round() as i32,
    ]
}

/// How many separate horizontal RUNS of mark ink an exported chart holds.
///
/// A column counts as inked when any pixel in it is within [`MARK_INK_TOL`] of
/// the mark colour on every channel — the stroke's core and the first ring of
/// its anti-aliasing, which is what "the mark landed here" means at 2 px wide.
/// The count is of maximal runs of adjacent inked columns: a solid line drawn
/// across the plot is one run whatever its slope, a dashed one is many.
///
/// Nothing else in the picture is admitted. The confidence band is the same hue
/// at 0.20 alpha and composites to roughly `#c9e4f0`, nowhere near; gridlines
/// are grey, axis type is near-black, the surface is near-white.
///
/// This is deliberately a measure of the PICTURE. A test that asked the scene
/// how many stroke ops it encoded would be satisfied by a dash pattern that
/// never reached a pixel, and by one drawn outside the plot clip.
const MARK_INK_TOL: i32 = 20;

fn mark_ink_runs(png: &std::path::Path) -> usize {
    let img = image::open(png).expect("open png").to_rgba8();
    let (w, h) = img.dimensions();
    let want = mark_ink();
    let mut runs = 0usize;
    let mut inside = false;
    for x in 0..w {
        let inked = (0..h).any(|y| {
            let p = img.get_pixel(x, y).0;
            (0..3).all(|c| (i32::from(p[c]) - want[c]).abs() <= MARK_INK_TOL)
        });
        if inked && !inside {
            runs += 1;
        }
        inside = inked;
    }
    runs
}

/// Navigate `dash` to an interval inside the data and hand back the composite.
///
/// Any narrowing extent makes the fit decline — it is a scalar aggregate with
/// no grouping key beneath which a bound could go — so the numbers here are
/// chosen only to keep points on both sides of them, not tuned to provoke
/// anything.
fn zoom_to(dash: &mut LiveDashboard, plot: &str) -> brightfield_shell::pipeline::Composed {
    use brightfield_engine::coordinator::Interaction;
    use brightfield_engine::{AxisExtent, NavigationExtent};
    use brightfield_spec::analysis::ComponentPath;
    dash.apply(Interaction::Navigate {
        plot: ComponentPath(plot.to_string()),
        extent: NavigationExtent {
            x: Some(AxisExtent::new("weight", 60.0, 88.0)),
            y: None,
        },
    })
    .expect("the navigation re-composites")
}

/// **A fit that outlived its frame says so in the picture, not only in the
/// panel.**
///
/// `examples/regression.yaml` holds a `dot` scatter and a `regressionY` over one
/// pair of columns. Narrow the frame and the scatter follows; the fit cannot —
/// its plan is a scalar aggregate with nothing beneath it to bind — so it goes
/// on drawing a line computed from every row, clipped at the frame edge, which
/// is exactly what a fit over the visible points looks like. The pane says
/// otherwise in a sentence. A screenshot carries the picture and drops the
/// sentence, and the picture is what people keep.
///
/// So this is asserted on an EXPORTED PNG, counting runs of mark ink across the
/// plot. Before the zoom the fit is one unbroken run; after it, many. Nothing
/// here reads the renderer's intent — a dash requested but clipped away, or
/// drawn in a colour that never lands, scores as solid.
///
/// The premise is asserted first, so this cannot pass by the decline having
/// quietly gone away.
#[test]
fn the_unrescoped_fit_is_dashed_in_the_exported_picture() {
    use brightfield_shell::capture::capture_vello_only;

    let dir = std::env::temp_dir().join(format!("bf-unrescoped-ink-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");

    let (mut dash, first) =
        live_spec(example("regression.yaml").to_str().unwrap()).expect("the fixture loads live");
    let plot = first.plots[0].path.clone();
    let before_png = dir.join("full-extent.png");
    capture_vello_only(first, 1.0, &before_png).expect("export at full extent");

    let zoomed = zoom_to(&mut dash, &plot);
    // The premise: the fit really did decline, and it is the fit that did.
    let declined = dash.declined_navigation(&plot);
    assert_eq!(
        declined.len(),
        1,
        "expected exactly the fit to decline, got {declined:?}"
    );
    assert_eq!(declined[0].kind.to_string(), "regressionY");

    let after_png = dir.join("navigated.png");
    capture_vello_only(zoomed, 1.0, &after_png).expect("export at the extent");

    let solid = mark_ink_runs(&before_png);
    let dashed = mark_ink_runs(&after_png);
    assert!(
        solid <= 3,
        "the full-extent fit drew {solid} separate runs of ink — it is supposed to be one \
         continuous line, so this measure is not measuring what it thinks it is"
    );
    assert!(
        dashed >= 20,
        "the navigated export drew {dashed} runs of mark ink against the full-extent \
         picture's {solid}. A fit that still summarises rows off screen has to LOOK \
         different from one that does not — a reader holding this screenshot has no \
         sentence to fall back on"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// How many image COLUMNS carry mark-hued ink at a strength between the
/// confidence band's own fill and the fit line's full opacity.
///
/// The band is filled at one fixed alpha. Stroking its boundary in the same ink
/// lays a second coat over the first, so the edge lands strictly darker than
/// the fill and strictly lighter than the fit — a level the picture holds
/// nowhere else, which is what makes it countable.
///
/// Three things make this a measure of the BAND and not of everything nearby.
///
/// The page tone is read from the image (its modal colour — the plot surface is
/// most of the frame) rather than named, so a token bump moves the reference
/// with the picture instead of silently emptying the count.
///
/// A pixel qualifies only if the alpha implied by its three channels AGREES
/// across them. That is what "on the mark's hue" means, and it is what keeps
/// grey gridlines and near-black axis type out: they are not this hue at any
/// opacity.
///
/// And a pixel within `CORE_GUARD` rows of full-strength mark ink is thrown
/// out. Everything drawn opaque — the fitted line, every scatter dot — has an
/// antialiasing skirt that passes through this alpha window on its way from
/// opaque to nothing, and that skirt is not the band. That exclusion is what
/// makes the measure mean anything: without it the full-extent picture scores
/// 342 columns rather than 17, which is the skirt, not a band edge it does not
/// have.
fn band_edge_ink_columns(png: &std::path::Path) -> usize {
    /// Rows either side of a full-strength pixel that its antialiasing can
    /// reach at 2 px of stroke width. Measured rather than guessed: over the
    /// two fixtures below the full-extent count is 17 at 2, at 3 and at 5, so
    /// the answer does not hang off this number.
    const CORE_GUARD: u32 = 3;
    /// Alpha window the band's doubled edge falls in: above the fill's own
    /// level once antialiasing is allowed for, well below opaque.
    const EDGE_LO: f64 = 0.28;
    const EDGE_HI: f64 = 0.60;
    /// Alpha above which a pixel is full-strength mark ink rather than band.
    const CORE: f64 = 0.75;
    /// How far the three channels' implied alphas may disagree and still count
    /// as one colour laid over the page at one opacity.
    const HUE_SPREAD: f64 = 0.06;

    let img = image::open(png).expect("open png").to_rgba8();
    let (w, h) = img.dimensions();

    let mut hist: std::collections::HashMap<[u8; 3], usize> = std::collections::HashMap::new();
    for px in img.pixels() {
        *hist.entry([px.0[0], px.0[1], px.0[2]]).or_insert(0) += 1;
    }
    let page = hist
        .into_iter()
        .max_by_key(|&(_, n)| n)
        .map(|(c, _)| c)
        .expect("a non-empty picture");
    let page = [f64::from(page[0]), f64::from(page[1]), f64::from(page[2])];
    let mark = mark_ink().map(f64::from);

    let alpha_at = |x: u32, y: u32| -> Option<f64> {
        let px = img.get_pixel(x, y).0;
        let per_channel: Vec<f64> = (0..3)
            .map(|c| (f64::from(px[c]) - page[c]) / (mark[c] - page[c]))
            .collect();
        let lo = per_channel.iter().copied().fold(f64::MAX, f64::min);
        let hi = per_channel.iter().copied().fold(f64::MIN, f64::max);
        (hi - lo <= HUE_SPREAD).then(|| per_channel.iter().sum::<f64>() / 3.0)
    };

    (0..w)
        .filter(|&x| {
            (0..h).any(|y| {
                alpha_at(x, y).is_some_and(|a| {
                    (EDGE_LO..=EDGE_HI).contains(&a)
                        && !(y.saturating_sub(CORE_GUARD)..=(y + CORE_GUARD).min(h - 1))
                            .any(|yy| alpha_at(x, yy).is_some_and(|v| v > CORE))
                })
            })
        })
        .count()
}

/// **The band's half of the caveat reaches the picture too.**
///
/// The fit was already dashed when it outlived its frame. Its confidence band
/// was not: it went on filling the same interval at the same strength, so the
/// larger half of the mark — the interval claim itself, computed from exactly
/// the rows the frame excludes — still read as a statement about what is on
/// screen. Half the mark said "this summarises data outside the frame" and half
/// did not.
///
/// Asserted on an EXPORTED PNG for the same reason the fit's own dash is: the
/// panel's sentence does not travel with a screenshot, and the picture is what
/// people keep. Nothing here reads the renderer's intent — an edge requested
/// but clipped away, or drawn at an alpha that never separates from the fill,
/// scores as untreated.
///
/// **What this does NOT hold is the rhythm.** Column counts say the band's
/// boundary carries ink it did not carry before; they say nothing about that
/// ink being broken on the fit's own 6-on/4-off period, because the band's two
/// edges dash independently and partly fill each other's gaps. The renderer's
/// `the_bands_caveat_is_the_fits_own_dash_and_not_a_second_vocabulary` holds the
/// rhythm, against the encoded scene where the two edges can be told apart.
/// Name the test that fails.
#[test]
fn the_unrescoped_fits_band_is_edged_in_the_exported_picture() {
    use brightfield_shell::capture::capture_vello_only;

    let dir = std::env::temp_dir().join(format!("bf-unrescoped-band-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");

    let (mut dash, first) =
        live_spec(example("regression.yaml").to_str().unwrap()).expect("the fixture loads live");
    let plot = first.plots[0].path.clone();
    let before_png = dir.join("full-extent.png");
    capture_vello_only(first, 1.0, &before_png).expect("export at full extent");

    let zoomed = zoom_to(&mut dash, &plot);
    // The premise, asserted first: the fit really did decline, so this cannot
    // pass by the decline having quietly gone away.
    let declined = dash.declined_navigation(&plot);
    assert_eq!(
        declined.len(),
        1,
        "expected exactly the fit to decline, got {declined:?}"
    );
    assert_eq!(declined[0].kind.to_string(), "regressionY");

    let after_png = dir.join("navigated.png");
    capture_vello_only(zoomed, 1.0, &after_png).expect("export at the extent");

    let untreated = band_edge_ink_columns(&before_png);
    let treated = band_edge_ink_columns(&after_png);

    assert!(
        untreated <= 40,
        "the full-extent picture already carries {untreated} columns of edge-strength \
         band ink, and its band has no edge — so this measure is picking up something \
         else and the comparison below would prove nothing"
    );
    assert!(
        treated >= 200,
        "the navigated export carries {treated} columns of edge-strength band ink \
         against the full-extent picture's {untreated}. The band is the interval claim \
         and it is the larger half of this mark: if it draws the same either way, the \
         reader holding this screenshot is told the fit outlived its frame and shown a \
         confidence interval that says otherwise"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **The ink and the rail sentence are independent statements of the same
/// fact.** Neither is derived from the other, and losing one must not take the
/// other with it.
///
/// The proof is in which artefact each is found in. `capture_vello_only`
/// rasterises the composed Vello scene and never constructs an egui context, so
/// every word the shell draws — the status rail included — is absent from that
/// PNG **by construction**; ink found there cannot have come from the sentence.
/// The document half runs the real window over real keystrokes and reads
/// `nav_scope_notice`, which is computed from `Session::declined_navigation`
/// and never rasterises a thing; a sentence found there cannot have come from
/// the ink.
///
/// The failure this guards is a later tidy-up that makes one the source of the
/// other — most plausibly by having the pane ask the composed scene whether it
/// drew a dash. That would read as a simplification and would mean a headless
/// export silently stops carrying the caveat, or the pane goes quiet whenever
/// the mark that declined happens to have no treatment of its own.
#[test]
fn the_ink_and_the_rail_sentence_do_not_depend_on_each_other() {
    use brightfield_shell::capture::capture_vello_only;
    use egui::Key;

    let dir = std::env::temp_dir().join(format!("bf-ink-and-sentence-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");

    // The sentence, on a document that never rasterises.
    let mut app = live_window(&example("regression.yaml"));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    assert_eq!(
        app.chart_doc().nav_scope_notice(),
        None,
        "an unnavigated chart claims nothing"
    );
    frame(&mut app, &ctx, vec![press(Key::Equals)]);
    let sentence = app
        .chart_doc()
        .nav_scope_notice()
        .expect("the pane still owes the reader the sentence");
    assert!(
        sentence.contains("regressionY"),
        "the sentence lost its subject: {sentence}"
    );

    // The ink, in an artefact that holds no chrome at all.
    let (mut dash, first) =
        live_spec(example("regression.yaml").to_str().unwrap()).expect("the fixture loads live");
    let plot = first.plots[0].path.clone();
    drop(first);
    let zoomed = zoom_to(&mut dash, &plot);
    let png = dir.join("chart-only.png");
    capture_vello_only(zoomed, 1.0, &png).expect("export the navigated chart");
    assert!(
        mark_ink_runs(&png) >= 20,
        "the chart-only export carries no dash, so everything the reader is told about \
         this fit lives in chrome a screenshot drops"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **A settled gesture that draws nothing puts the query store back where the
/// picture is.** A pan far enough off the data returns no rows, the
/// re-composite fails, and the caller keeps the picture it had. The session's
/// extent has to go back to the one that picture was drawn at, or every later
/// re-query is emitted at a range the reader never saw a picture of, and the
/// store disagrees with the rows on screen.
///
/// **It restores, it does not clear**, and the difference is the whole of the
/// mechanism. So this zooms SUCCESSFULLY first and only then walks off the
/// data: from full extent the value being restored is the default and the
/// engine drops the key for it, which makes a rollback that put the previous
/// extent back and a rollback that simply forgot the plot produce identical
/// stores. Held here against the zoomed extent by value, so the two are told
/// apart.
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

    // A zoom that WORKS, first — the state the rollback has to restore. One
    // real keystroke, so the extent under test is one a person can produce.
    frame(&mut app, &ctx, vec![press(egui::Key::Equals)]);
    let zoomed_rows = mark_rows(app.chart_doc_mut(), 0);
    assert!(
        zoomed_rows > 0 && zoomed_rows < full,
        "the zoom has to leave a NARROWER picture that still draws, or there is \
         no restorable state to lose: {zoomed_rows} vs {full}"
    );
    let zoomed = app
        .chart_doc()
        .live_dashboard()
        .expect("a live document")
        .query_extents()
        .get(&path)
        .cloned()
        .expect("the settled zoom is in the query store");

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
    assert_eq!(
        live.query_extents().get(&path),
        Some(&zoomed),
        "the query store did not go back to the extent the picture on screen was \
         drawn at. Clearing the plot instead of restoring it looks the same from \
         full extent and is not the same thing: it silently throws the zoom away. \
         Store: {:?}",
        live.query_extents()
    );
    assert!(
        live.view_extents().contains_key(&path),
        "the axes were rolled back too — the frame on screen IS the moved one"
    );
    assert!(doc.navigated(), "so there is still a frame to reset");

    // The rows the session serves are the ones the picture was drawn from —
    // the zoomed set, not the full one.
    assert_eq!(
        mark_rows(app.chart_doc_mut(), 0),
        zoomed_rows,
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

// ---------------------------------------------------------------------------
// 8. A sampled plot the reader has zoomed
// ---------------------------------------------------------------------------

/// A sampled plot whose two marks sit on differently-NAMED x columns, far apart.
///
/// The name is what makes this fixture bite. A navigation extent names the
/// column of the plot's first mark, and `Session::navigation_pass` applies an
/// axis to a mark only when that mark's own column carries the same name — so
/// `far` is left alone by the gesture, goes on drawing and measuring its whole
/// column, and its unsampled extent sits an order of magnitude outside the
/// interval the reader asked for.
///
/// `CAST(... AS DOUBLE)` before the arithmetic: a DuckDB decimal literal would
/// make these columns DECIMAL, which contributes no drawn domain at all.
const SAMPLED_TWO_COLUMN_SPEC: &str = "data:
  near:
    query: |
      SELECT CAST(i % 101 AS DOUBLE) * 10.0 AS a,
             CAST(i * 104729 % 1013 AS DOUBLE) / 10.0 AS b
      FROM range(8192) AS t(i)
  far:
    query: |
      SELECT CAST(i % 101 AS DOUBLE) * 10.0 + 5000.0 AS c,
             CAST(i * 104729 % 1013 AS DOUBLE) / 10.0 AS d
      FROM range(8192) AS t(i)
plot:
  - mark: dot
    data: { from: near }
    x: a
    y: b
  - mark: dot
    data: { from: far }
    x: c
    y: d
width: 400
height: 300
";

/// The `(min, max)` of a plot's linear positional scale.
fn nav_domain(
    composed: &brightfield_shell::pipeline::Composed,
    channel: brightfield_render::channel::Channel,
) -> (f64, f64) {
    match composed.plots[0].scales.get(channel) {
        Some(brightfield_render::scale::Scale::Linear {
            domain_min,
            domain_max,
            ..
        }) => (*domain_min, *domain_max),
        other => panic!("expected a linear {channel:?} scale, got {other:?}"),
    }
}

/// Load `SAMPLED_TWO_COLUMN_SPEC` live at `rate`, zoom x to `[255, 745]`, and
/// hand back the composite the zoom produced.
fn zoomed_two_column_plot(
    rate: Option<brightfield_sql::ir::SampleRate>,
) -> brightfield_shell::pipeline::Composed {
    use brightfield_engine::coordinator::Interaction;
    use brightfield_engine::{AxisExtent, NavigationExtent};
    use brightfield_spec::analysis::ComponentPath;

    let mut live = LiveDashboard::load_str(SAMPLED_TWO_COLUMN_SPEC, None).expect("loads live");
    live.set_sample(rate);
    let first = live.present().expect("first paint");
    let path = first.plots[0].path.clone();

    live.apply(Interaction::Navigate {
        plot: ComponentPath(path),
        extent: NavigationExtent {
            x: Some(AxisExtent::new("a", 255.0, 745.0)),
            y: None,
        },
    })
    .expect("the navigation re-composites")
}

/// **A zoom holds on a sampled plot, on the axis the reader zoomed.**
///
/// The unsampled-domain restoration runs AFTER the view extent has landed on
/// the scale, so it is the last writer on a navigated axis and the reader's
/// interval is whatever it leaves there. Widening that interval out to the
/// unsampled extent moves the frame off where the reader put it: the second
/// mark here is on a column the gesture does not name, so it is never scoped,
/// and its measured extent reaches `6000` on an axis the reader asked to stop
/// at `745`.
///
/// The `y` half is asserted in the same breath and is the reason this is not
/// simply "do not restore a sampled plot that has been navigated": y carries no
/// extent, so it is restored exactly as on an unnavigated plot, and the two
/// axes have to come out of one pass disagreeing about whether a gesture
/// happened.
#[test]
fn a_zoomed_sampled_plot_keeps_the_readers_interval_on_the_navigated_axis() {
    use brightfield_render::channel::Channel;

    let rate = brightfield_sql::ir::SampleRate::from_modulus(32).expect("power of two");
    let complete = zoomed_two_column_plot(None);
    let sampled = zoomed_two_column_plot(Some(rate));

    // Fixture check: the unscoped sibling really is measurable far outside the
    // reader's interval, so widening the navigated axis would be visible.
    let unzoomed_x = {
        let mut live = LiveDashboard::load_str(SAMPLED_TWO_COLUMN_SPEC, None).expect("loads live");
        live.set_sample(Some(rate));
        nav_domain(&live.present().expect("first paint"), Channel::X)
    };
    assert!(
        unzoomed_x.1 > 5000.0,
        "fixture check: at rest the plot's x domain reaches {}, and the second mark is \
         supposed to carry it past 5000",
        unzoomed_x.1
    );

    assert_eq!(
        nav_domain(&sampled, Channel::X),
        (255.0, 745.0),
        "the reader zoomed a sampled plot to [255, 745] and the axes came back describing \
         something else. The restoration runs after the extent lands on the scale, so it \
         must leave a navigated axis alone — replacing it crops the frame to the rows that \
         survived inside it, and widening it drags the frame out to a sibling the gesture \
         never scoped."
    );
    assert_eq!(
        nav_domain(&sampled, Channel::X),
        nav_domain(&complete, Channel::X),
        "the same zoom on the same spec must land on the same x domain sampled as complete"
    );

    assert_eq!(
        nav_domain(&sampled, Channel::Y),
        nav_domain(&complete, Channel::Y),
        "y carries no extent, so it is restored: a sampled plot's y domain must still be \
         the complete plot's, or zooming x has quietly narrowed the other axis"
    );

    let fact = sampled.plots[0]
        .sample
        .expect("the zoomed plot must still carry its sampling fact");
    assert!(
        fact.drawn * 4 < fact.of,
        "fixture check: a 1-in-32 sample must drop most of the rows ({} of {})",
        fact.drawn,
        fact.of
    );
}
