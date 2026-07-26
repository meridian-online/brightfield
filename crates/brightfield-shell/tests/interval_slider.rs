//! Gate: the interval slider RENDERS, DRAGS, and drags well.
//!
//! Every assertion here is made through the surface a person touches — the
//! controls rail inside a real window, driven by real pointer events at a rect
//! a real layout pass produced. A green test one layer down would prove a
//! function works; what has to be true is that the product works.
//!
//! GPU-free throughout: `MeridianApp::headless_with_layout` builds the whole
//! window over a live session without a device, so the rail lays out and
//! responds exactly as it does on screen and every re-query really hits
//! DuckDB.
//!
//! Four things are held here, and each one names a way the feature can be
//! broken while still looking finished:
//!
//! - **It reaches the rail at all.** A node carrying both `input:` and
//!   `select:` parses as an interactor, so the input-only collector cannot see
//!   it; a fix confined to the input side renders nothing.
//! - **Its contributor matches no plot.** Get this wrong and crossfilter
//!   self-exclusion exempts a plot from its own slider — half a dashboard
//!   quietly stops filtering, with nothing on screen saying so. A slider has
//!   no picture of its own to spare.
//! - **The drag is coalesced, and the handle is the pointer's.** Positions
//!   swept past while a query ran must never be executed, and the rendered
//!   handle must never be the last finished query's value.
//! - **A drag is not an edit.** It must leave the spec file's bytes, the
//!   editor buffer and every dirty flag alone.

use std::fs;
use std::path::{Path, PathBuf};

use brightfield_engine::RecordBatch;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::design::Mode;
use brightfield_shell::editor::EditorPane;
use brightfield_shell::pipeline::{compose_spec, live_spec, IntervalControl, LiveDashboard};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::{Dirty, Item};

/// The shipped example, which is the fixture on purpose: the thing a person is
/// going to sit down in front of is the thing these tests drive.
fn example_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/interval-slider.yaml")
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

/// The window rect every frame here is run at — big enough that the rail is
/// laid out rather than collapsed.
fn screen() -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0))
}

/// Run one frame with `events`.
fn frame(app: &mut MeridianApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput {
        screen_rect: Some(screen()),
        events,
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| app.draw(ui));
}

/// The rows the chart's first mark is currently drawn from — the honest read
/// of "did the picture change", taken off the live session rather than off any
/// field the code under test writes.
fn doc_rows(doc: &mut ChartDoc) -> usize {
    doc.live_coordinator()
        .expect("a live document")
        .chart_rows(0)
        .expect("the fixture has a mark")
        .iter()
        .map(RecordBatch::num_rows)
        .sum()
}

/// The total occupancy of the density curve the chart is drawing — the sum of
/// its per-bin counts.
///
/// This is what "the picture changed" means for the shipped example. Counting
/// result ROWS would not: a density mark returns one row per bin whatever the
/// filter admits, so a chart frozen solid and a chart tracking the pointer
/// return the same number of rows. The curve's mass is the picture.
fn curve_mass(app: &mut MeridianApp) -> f64 {
    use arrow::array::Float64Array;
    use arrow::compute::cast;
    use arrow::datatypes::DataType;
    let batches = app
        .chart_doc_mut()
        .live_coordinator()
        .expect("a live document")
        .chart_rows(0)
        .expect("the fixture has a mark");
    let mut total = 0.0;
    for batch in &batches {
        let idx = batch
            .schema()
            .index_of("__bf_count")
            .expect("a density mark returns its per-bin count");
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

/// The materialisation generation — one per applied interaction, so counting
/// it counts the queries a drag actually caused.
fn generation(doc: &mut ChartDoc) -> u64 {
    doc.live_coordinator()
        .expect("a live document")
        .generation()
}

/// The example's single interval control, read off the composition.
fn control(app: &MeridianApp) -> IntervalControl {
    let intervals = &app.chart_doc().composed.intervals;
    assert_eq!(
        intervals.len(),
        1,
        "the example declares exactly one interval slider; the collector found \
         {} — a node carrying BOTH `input: slider` and `select:` parses as an \
         interactor, so an input-only collector sees nothing here",
        intervals.len()
    );
    intervals[0].clone()
}

// ---------------------------------------------------------------------------
// It renders
// ---------------------------------------------------------------------------

/// The rail draws a slider for it, inside the rail, with area.
///
/// This is the whole of "an interval slider cannot be rendered today": the
/// widget has to appear on the surface a person looks at, at a rect they can
/// aim at.
#[test]
fn the_controls_rail_draws_a_slider_for_a_spec_declared_interval() {
    let mut app = live_window(&example_path());
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());

    let rects = app.chart_doc().interval_slider_rects.clone();
    assert_eq!(rects.len(), 1, "the rail drew no interval slider");
    let (key, rect) = &rects[0];
    assert_eq!(
        key,
        control(&app).key(),
        "the rect is filed under the widget"
    );
    assert!(
        rect.width() > 20.0 && rect.height() > 4.0,
        "the slider was drawn with no area to grab: {rect:?}"
    );
    assert!(
        screen().contains_rect(*rect),
        "the slider was drawn outside the window: {rect:?}"
    );
}

/// The spec renders whole: no banner, blocking or advisory.
///
/// It is why the example declares `intervalX` rather than the upstream
/// `interval`, which this build registers as unimplemented — a demo that opens
/// under a "cannot render" notice is a demo nobody trusts.
#[test]
fn the_example_loads_without_a_single_warning_banner() {
    let app = live_window(&example_path());
    let diagnostics = app.load_diagnostics();
    assert!(
        diagnostics.is_empty(),
        "the example must render whole; it says: {:#?}",
        diagnostics.diagnostics
    );
    assert_eq!(app.notifications().len(), 0, "and raise no banner");
}

// ---------------------------------------------------------------------------
// Its contributor identity
// ---------------------------------------------------------------------------

/// **The invisible failure.** The slider's contributor path must match no
/// plot's identity, or crossfilter self-exclusion drops its clause for that
/// plot and the plot goes on showing everything under a control that says
/// otherwise.
///
/// Asserted against the identity the ENGINE compares — `plot_node_path` of
/// each mark's own path — not against a second copy of the derivation.
///
/// **This test alone does not hold the law, and it is worth saying why here
/// rather than letting the next reader assume it does.** The per-spec
/// inequalities below pass under the realistic wrong implementation — giving
/// the contributor `plot_node_path(path)`, which is what a brush does —
/// because in this fixture the slider sits at `root/vconcat[1]` and the plot
/// at `root/vconcat[0]`, so a wrong derivation still collides with nothing.
/// Layout accident, not an invariant.
///
/// So the first assertion is the structural one: the contributor ends at an
/// `input[slider]` leaf, and `plot_node_path` can never produce such a path.
/// That holds for every spec, not just this one. The behavioural sibling
/// `a_plot_that_declares_the_slider_is_still_filtered_by_it` is what actually
/// reddens when the derivation is wrong.
#[test]
fn an_interval_sliders_contributor_matches_no_plot_in_the_spec() {
    use brightfield_spec::analysis::plot_node_path;

    let parsed = brightfield_spec::parse_spec_path(example_path()).expect("the fixture parses");
    let composed = compose_spec(example_path().to_str().expect("utf-8 path")).expect("composes");
    let control = composed
        .intervals
        .first()
        .expect("the example declares an interval slider");

    // The spec-independent half: the leaf is what makes the identity safe, so
    // assert the leaf. A derivation that dropped it would fail here on any
    // fixture, where the inequalities below only fail on a fixture unlucky
    // enough to place the slider and the plot at the same path.
    assert!(
        control.contributor.0.ends_with("/input[slider]"),
        "the contributor lost its input[slider] leaf ({}) — without it the \
         path is one a plot can legitimately own, and crossfilter would \
         exempt that plot from its own slider",
        control.contributor.0
    );

    let marks = brightfield_sql::emit::collect_marks_with_paths(&parsed.spec);
    assert!(!marks.is_empty(), "the fixture has marks to check against");
    for (_, mark_path) in &marks {
        assert_ne!(
            plot_node_path(mark_path),
            control.contributor.0,
            "the slider's contributor is the identity of the plot holding \
             {mark_path} — crossfilter would exempt that plot from its own \
             slider, and nothing on screen would say so"
        );
    }
    for plot in &composed.plots {
        assert_ne!(
            plot.path, control.contributor.0,
            "the slider's contributor collides with a placed plot's path"
        );
    }
}

/// The same law stated behaviourally, over the shape that makes it bite: a
/// plot that declares the slider as its own child. The plot MUST still be
/// filtered by it.
#[test]
fn a_plot_that_declares_the_slider_is_still_filtered_by_it() {
    // The slider is an item of the plot, so its synthetic path sits under a
    // `/plot[i]` segment — exactly the case where a contributor derived by
    // stripping to the plot node would self-exclude.
    const SPEC: &str = r#"
params:
  window:
    select: crossfilter
data:
  t: { query: "SELECT i AS k, (i % 7) AS v FROM range(100) t(i)" }
plot:
  - mark: dot
    data: { from: t, filterBy: $window }
    x: k
    y: v
  - select: intervalX
    input: slider
    as: $window
    column: k
    min: 0
    max: 99
    value: 99
"#;
    let mut live = LiveDashboard::load_str(SPEC, None).expect("loads live");
    let composed = live.present().expect("first paint");
    let control = composed
        .intervals
        .first()
        .expect("the in-plot slider is collected")
        .clone();
    let mut doc = ChartDoc::headless(composed);
    doc.attach_live(live);
    assert_eq!(doc_rows(&mut doc), 100, "unfiltered to start");

    doc.note_interval_drag(&control, 9.0);
    assert_eq!(doc.pump_interval_drags(std::slice::from_ref(&control)), 1);
    assert_eq!(
        doc_rows(&mut doc),
        10,
        "the plot declaring the slider exempted itself from it — half a \
         dashboard would go on showing everything with nothing saying so"
    );
}

// ---------------------------------------------------------------------------
// It drags, and the drag behaves
// ---------------------------------------------------------------------------

/// A real pointer drag on the rail's slider moves the handle AND the chart.
///
/// Driven through `egui` events at the recorded rect, so what is under test is
/// the widget a person grabs, not a function beneath it.
#[test]
fn a_pointer_drag_on_the_rail_moves_the_handle_and_the_chart() {
    let mut app = live_window(&example_path());
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());

    let control = control(&app);
    let rect = app.chart_doc().interval_slider_rects[0].1;
    let start = app
        .chart_doc_mut()
        .interval_drags
        .shown(control.key(), control.value);
    let mass_before = curve_mass(&mut app);

    // Grab the handle where it sits and sweep left, one frame per step, the
    // way a hand does.
    let y = rect.center().y;
    let grab = egui::pos2(rect.lerp_inside(egui::vec2(0.5, 0.5)).x, y);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(grab)]);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerButton {
            pos: grab,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        }],
    );
    for step in 1..=6 {
        let x = rect.left() + rect.width() * (0.5 - 0.06 * step as f32);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(x, y))],
        );
    }
    let released = egui::pos2(rect.left() + rect.width() * 0.14, y);
    frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::PointerMoved(released),
            egui::Event::PointerButton {
                pos: released,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ],
    );
    // Frames after the release: the value the drag ended on is still owed a
    // query, and must land without another pointer event.
    for _ in 0..4 {
        frame(&mut app, &ctx, Vec::new());
    }

    let ended = app
        .chart_doc_mut()
        .interval_drags
        .shown(control.key(), control.value);
    assert!(
        ended < start - 1.0,
        "the handle did not follow the pointer: {start} → {ended}"
    );
    let mass_after = curve_mass(&mut app);
    assert!(
        mass_after < mass_before,
        "the chart did not follow the handle: the curve still carries \
         {mass_after} of {mass_before}"
    );
    assert!(
        !app.chart_doc().interval_drags.any_pending(),
        "the value the drag ended on was never dispatched — a drag that ends \
         between queries would leave the chart at a position the hand had \
         already left"
    );
    let dispatched = app.chart_doc().interval_drags.dispatched().to_vec();
    let last = *dispatched.last().expect("something was dispatched");
    assert!(
        (last - ended).abs() < f64::EPSILON,
        "the last value executed ({last}) is not where the handle rests \
         ({ended})"
    );
}

/// Coalescing, at the seam the rail drives: values a drag swept past while a
/// query was outstanding are never executed, so the engine advances once, not
/// once per frame.
#[test]
fn values_swept_past_between_queries_never_reach_duckdb() {
    let mut app = live_window(&example_path());
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    let control = control(&app);
    // Settle the boot push so the count below is about the drag alone.
    for _ in 0..3 {
        frame(&mut app, &ctx, Vec::new());
    }
    let doc = app.chart_doc_mut();
    let before = generation(doc);

    // A sweep, all inside one frame's worth of pointer motion.
    for v in [150.0, 120.0, 90.0, 60.0, 30.0] {
        doc.note_interval_drag(&control, v);
    }
    assert_eq!(doc.pump_interval_drags(std::slice::from_ref(&control)), 1);
    let after = generation(doc);
    assert_eq!(
        after - before,
        1,
        "the sweep executed {} queries; only the newest value should have run",
        after - before
    );
    assert_eq!(
        doc.interval_drags.dispatched().last().copied(),
        Some(30.0),
        "and the value that ran is the newest, not the oldest"
    );
}

/// The rendered handle is the pointer's, not the last completed query's — so a
/// re-query that FAILS cannot snap it back under the hand holding it.
///
/// Driven by making the query fail for real: the interval names a column that
/// does not exist, which is what a bad predicate looks like from the session's
/// side.
#[test]
fn a_failed_requery_leaves_the_handle_where_the_pointer_put_it() {
    const SPEC: &str = r#"
params:
  window:
    select: crossfilter
data:
  t: { query: "SELECT i AS k FROM range(50) t(i)" }
vconcat:
  - plot:
      - mark: dot
        data: { from: t, filterBy: $window }
        x: k
        y: k
  - select: intervalX
    input: slider
    as: $window
    column: no_such_column
    min: 0
    max: 49
    value: 49
"#;
    let mut live = LiveDashboard::load_str(SPEC, None).expect("loads live");
    let composed = live.present().expect("first paint");
    let control = composed.intervals.first().expect("collected").clone();
    let mut doc = ChartDoc::headless(composed);
    doc.attach_live(live);

    doc.note_interval_drag(&control, 12.0);
    doc.pump_interval_drags(std::slice::from_ref(&control));
    assert!(
        (doc.interval_drags.shown(control.key(), control.value) - 12.0).abs() < f64::EPSILON,
        "a failed re-query dragged the handle back to the composition's value"
    );
    // And the slider is not wedged: the next drag still dispatches.
    doc.note_interval_drag(&control, 8.0);
    assert_eq!(doc.pump_interval_drags(std::slice::from_ref(&control)), 1);
}

// ---------------------------------------------------------------------------
// A drag is not an edit
// ---------------------------------------------------------------------------

/// Dragging the slider leaves the spec file's bytes, the editor's buffer and
/// every dirty flag exactly as they were. A selection is live state; nothing
/// about it is a change to the document on disk.
#[test]
fn a_drag_changes_no_byte_of_the_spec_and_no_dirty_flag() {
    let dir = std::env::temp_dir().join(format!("bf-interval-drag-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join("interval-slider.yaml");
    fs::copy(example_path(), &path).expect("copy the fixture");
    let before = fs::read(&path).expect("read the fixture");

    let mut app = live_window(&path);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());

    // The editor pane over the SAME file, seeded exactly as the shell seeds it
    // when the pane opens beside the chart. Its dirty state is a function of
    // its buffer and that file, and the drag must move neither.
    let mut editor = EditorPane::new();
    editor.open_file(&path);
    let buffer_before = editor.buffer().expect("the editor seeded").to_string();
    assert!(!editor.can_save(), "the seeded buffer starts clean");
    assert_eq!(editor.subject(app.chart_doc()).dirty, Dirty::Clean);

    let control = control(&app);
    for v in [150.0, 100.0, 50.0] {
        app.chart_doc_mut().note_interval_drag(&control, v);
        app.chart_doc_mut()
            .pump_interval_drags(std::slice::from_ref(&control));
    }
    for _ in 0..3 {
        frame(&mut app, &ctx, Vec::new());
    }
    assert!(
        app.chart_doc().interval_drags.dispatched().len() >= 3,
        "the drag never ran, so this proves nothing"
    );

    assert_eq!(
        fs::read(&path).expect("read back"),
        before,
        "a slider drag rewrote the spec file"
    );
    assert_eq!(
        editor.buffer().expect("still open"),
        buffer_before,
        "a slider drag rewrote the editor buffer"
    );
    assert!(
        !editor.can_save(),
        "a slider drag left the document looking edited"
    );
    assert_eq!(
        editor.subject(app.chart_doc()).dirty,
        Dirty::Clean,
        "a slider drag raised the editor's dirty dot"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// It rides the pre-aggregated path
// ---------------------------------------------------------------------------

/// **The reason a sustained drag is affordable at all.** Because the slider
/// pushes a structured interval into a SELECTION, the pre-aggregation layer
/// derives a cube on the first push and serves every later one from it — so a
/// drag step re-queries a few thousand cube rows instead of scanning a million
/// base rows.
///
/// Proven by what DuckDB was actually asked to run, not by timing: after the
/// cube is built, no executed statement names the base table.
#[test]
fn a_drag_step_is_served_from_the_pre_aggregate_not_the_base_table() {
    let mut app = live_window(&example_path());
    let ctx = egui::Context::default();
    // Two frames: the rail lays out and pushes the declared start, which is
    // what builds the cube before a hand is on the slider.
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());

    let control = control(&app);
    let doc = app.chart_doc_mut();
    assert!(
        generation(doc) > 0,
        "the rail never pushed the declared start, so nothing built a cube"
    );
    doc.live_coordinator()
        .expect("live")
        .session_mut()
        .clear_executed_sql();

    // One drag step, after the cube exists. Timed and printed, never asserted
    // on — the gate is the statement log below, because a timing threshold is
    // a claim about the machine and this one has to hold on all of them. The
    // number is here so a person about to sit down in front of the window
    // knows what a step costs, and so a regression that keeps the cube but
    // loses the speed still leaves a trace.
    //
    // WHAT THIS PRINT IS AND IS NOT. It is one sample, in whatever profile the
    // suite was built with — a debug run prints several times the release
    // figure, so a single number quoted next to it in either profile reads as
    // a contradiction. It is not a paired measurement: the cube-off arm is not
    // run here, so no ratio stated in this file could be reproduced from this
    // file. The honest paired form — both arms, same footing, with spread —
    // belongs in the bench harness, which already has a pre-aggregation
    // on/off scenario. Quote that, not this.
    //
    // Note also that the timed region spans query, execute and a full scene
    // recomposite. The quantity is "what a drag step costs the window", which
    // is the user-facing one, not "what the query costs".
    let started = std::time::Instant::now();
    doc.note_interval_drag(&control, 120.0);
    assert_eq!(doc.pump_interval_drags(std::slice::from_ref(&control)), 1);
    eprintln!(
        "INTERVAL_DRAG_STEP_MS {:.3}",
        started.elapsed().as_secs_f64() * 1e3
    );

    let session = doc.live_coordinator().expect("live").session();
    let stats = session.preagg_stats().clone();
    assert!(
        stats.cube_hits > 0,
        "the drag step was NOT served from a cube — it scanned the base \
         table, which is what makes a sustained drag feel like a series of \
         stalls. Stats: {stats:?}"
    );
    assert_eq!(stats.serve_failures, 0, "a registered serve fell back");
    let log = session.executed_sql();
    assert!(!log.is_empty(), "the drag ran no SQL at all");
    assert!(
        log.iter()
            .all(|sql| !sql.contains("\"traces\"") || sql.starts_with("CREATE TEMP TABLE")),
        "a drag step read the base table: {log:#?}"
    );
}
