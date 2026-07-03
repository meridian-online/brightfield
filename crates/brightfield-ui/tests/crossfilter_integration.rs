//! Cross-filter end-to-end (headless) — card 0006.
//!
//! Proves the full producer→engine→results→scene chain that makes brushing a
//! range in one plot of a multi-view dashboard filter another plot:
//!
//!   analysis.brushable_bindings  →  BrushBinding (real spec-derived)
//!     →  commit_brush_release_multi(..., &mut Session)   (Session IS the
//!         SelectionDispatcher; no RecordingDispatcher double)
//!       →  Session::propagate_selection re-executes the subscriber mark
//!         →  build_multi_mark_scene over the filtered batches yields a scene.
//!
//! This is the seam that, until now, only existed against a recording test
//! double: every `commit_brush_*` / `propagate_selection` test drove a fake
//! dispatcher. Here a *live* `Session` (real DuckDB, real `emit_query`
//! selection threading) is the dispatcher, and a real `BrushBinding` comes
//! from `analyse_spec`, not a hand-built struct.
//!
//! Coordinates are constructed directly in DATA space — the pixel→data
//! inversion that the live GPUI window needs is a separate increment, so it is
//! deliberately bypassed here (the brush rect is authored in column units).

use brightfield_engine::Engine;
use brightfield_render::channel::{Channel, ChannelMap};
use brightfield_render::layout::ChartLayout;
use brightfield_render::mark::{count_scene_paths, default_renderers, find_renderer};
use brightfield_render::scale::infer_scales;
use brightfield_render::scene::{build_multi_mark_scene, ChartData};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::{parse_spec, Format};
use brightfield_sql::collect_marks;
use brightfield_ui::brush::{brush_rect_to_predicate, point_to_predicate, BrushKind};
use brightfield_ui::chart_view::{
    commit_brush_clear, commit_brush_release_multi, commit_click_multi, BrushBinding,
};
use brightfield_ui::InteractionState;
use kurbo::{Point, Rect};

/// A two-plot dashboard over one inline table. Plot A (left) carries an
/// `intervalX` brush writing `$brush`; plot B (right) is filtered BY `$brush`.
/// Brushing an x-range on A must reduce the rows plot B renders.
const SPEC: &str = r#"
params:
  brush: { select: crossfilter }
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
    - { x: 4, y: 40 }
    - { x: 5, y: 50 }
    - { x: 6, y: 60 }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
    - select: intervalX
      as: $brush
    width: 360
    height: 300
  - plot:
    - mark: dot
      data: { from: t, filterBy: $brush }
      x: x
      y: y
    width: 360
    height: 300
"#;

/// A single plot that both BRUSHES (`intervalX as: $brush`) and is FILTERED BY
/// its own selection (`filterBy: $brush`). Under crossfilter resolution a plot
/// must NOT filter itself, so brushing here should leave the mark's rows
/// unchanged (self-exclusion). This is the canonical mutual-crossfilter shape
/// reduced to one plot.
const SELF_SPEC: &str = r#"
params:
  brush: { select: crossfilter }
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
    - { x: 4, y: 40 }
    - { x: 5, y: 50 }
    - { x: 6, y: 60 }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
    y: y
  - select: intervalX
    as: $brush
"#;

/// Plot A carries an `intervalY` brush; plot B is filtered by it. Brushing a
/// y-range on A must reduce plot B's rows on the y column. (scenario 1, Y)
const SPEC_Y: &str = r#"
params:
  brush: { select: crossfilter }
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
    - { x: 4, y: 40 }
    - { x: 5, y: 50 }
    - { x: 6, y: 60 }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
    - select: intervalY
      as: $brush
    width: 360
    height: 300
  - plot:
    - mark: dot
      data: { from: t, filterBy: $brush }
      x: x
      y: y
    width: 360
    height: 300
"#;

/// Plot A carries an `intervalXY` brush; plot B is filtered by it. Brushing a
/// 2D region must restrict plot B on BOTH axes. (scenario 1, XY)
const SPEC_XY: &str = r#"
params:
  brush: { select: crossfilter }
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
    - { x: 4, y: 40 }
    - { x: 5, y: 50 }
    - { x: 6, y: 60 }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
    - select: intervalXY
      as: $brush
    width: 360
    height: 300
  - plot:
    - mark: dot
      data: { from: t, filterBy: $brush }
      x: x
      y: y
    width: 360
    height: 300
"#;

/// Sum the rows across a batch list.
fn total_rows(batches: &[brightfield_engine::RecordBatch]) -> usize {
    batches.iter().map(|b| b.num_rows()).sum()
}

#[test]
fn crossfilter_brush_in_plot_a_filters_plot_b() {
    let parsed = parse_spec(SPEC, Format::Yaml).expect("spec parses");
    let spec = parsed.spec;
    let analysis = analyse_spec(&spec).expect("spec analyses");

    // --- The brush SOURCE: a real analysis-derived binding, not hand-built. ---
    // Plot A's single intervalX interactor is the only brushable binding, and
    // its x channel is resolved from plot A's dot mark (column `x`).
    assert_eq!(
        analysis.brushable_bindings.len(),
        1,
        "exactly one brushable binding (plot A's intervalX)"
    );
    let spec_binding = &analysis.brushable_bindings[0];
    assert_eq!(spec_binding.selection, "brush");
    // The contributor identity lives under the left plot (hconcat item 0).
    // NB: the `plot[i]` index is the brushing interactor's item-index within
    // its plot, not the plot's concat position — a known path-scheme quirk
    // (see the multiview memo). For A→B (distinct plots) self-exclusion does
    // not fire, so plot B is still filtered; the row-reduction below proves it.
    assert!(
        spec_binding.parent_plot.0.starts_with("root/hconcat[0]"),
        "contributor is the left plot (A); got {}",
        spec_binding.parent_plot.0
    );
    assert_eq!(
        spec_binding.channels.x.as_deref(),
        Some("x"),
        "x channel resolved from plot A's dot mark"
    );
    let binding: BrushBinding = spec_binding.into();
    let bindings = [binding];

    // --- The render metadata for plot B's subscriber mark (mark index 1), ---
    // captured before `spec` is moved into the session, so we can rebuild its
    // scene from the filtered batches.
    let marks = collect_marks(&spec);
    assert_eq!(marks.len(), 2, "two dot marks, one per plot");
    let plot_b_mark = marks[1];
    let plot_b_channels = ChannelMap::from_mark(plot_b_mark);
    let plot_b_kind = plot_b_mark.kind;

    // --- Live engine + session: the dispatcher is a real DuckDB-backed Session. ---
    let engine = Engine::new();
    let mut session = engine
        .load_spec(spec, analysis, None)
        .expect("spec loads")
        .session;

    // Baseline: unfiltered plot B renders every row.
    let baseline = session.execute_all();
    let baseline_b = baseline[1].as_ref().expect("plot B executes");
    let baseline_rows = total_rows(baseline_b);
    assert_eq!(baseline_rows, 6, "all 6 rows before any brush");

    // --- Brush an x-range [2.5, 4.5] in DATA coordinates on plot A. ---
    // intervalX uses only the x extent; y is immaterial. The rect is authored
    // in column units (pixel→data inversion is a later increment).
    let mut interaction = InteractionState::start_brush(Point::new(2.5, 0.0));
    interaction.update_brush(Point::new(4.5, 100.0));

    let (next_state, aggregated) =
        commit_brush_release_multi(&interaction, &bindings, &mut session);
    assert!(
        matches!(next_state, InteractionState::Idle),
        "brush release returns to Idle"
    );

    // One binding → one selection's results; plot B (the only subscriber) re-ran.
    assert_eq!(aggregated.len(), 1, "one binding dispatched");
    let (selection_name, results) = &aggregated[0];
    assert_eq!(selection_name, "brush");
    assert_eq!(
        results.len(),
        1,
        "exactly one subscriber mark re-executes (plot B; plot A self-excluded)"
    );
    let (mark_index, result) = &results[0];
    assert_eq!(*mark_index, 1, "the subscriber is plot B's mark (index 1)");
    let filtered_b = result.as_ref().expect("plot B re-executes ok under brush");
    let filtered_rows = total_rows(filtered_b);

    // The brush x∈[2.5,4.5] keeps integer x ∈ {3,4}: 2 of 6 rows.
    assert!(
        filtered_rows < baseline_rows,
        "brush reduces plot B rows: {filtered_rows} !< {baseline_rows}"
    );
    assert_eq!(filtered_rows, 2, "x∈[2.5,4.5] keeps rows x=3,4");

    // --- The result→scene bridge: filtered batches rebuild a real scene. ---
    // The filtered result is small (2 rows) so DuckDB returns a single batch;
    // larger results would concatenate (the app's run_pipeline does this).
    assert_eq!(filtered_b.len(), 1, "small filtered result fits one batch");
    let registry = default_renderers();
    let renderer = find_renderer(&registry, plot_b_kind).expect("renderer for dot");
    let chart_data = ChartData {
        batch: &filtered_b[0],
        channel_map: &plot_b_channels,
        renderer,
        layout: ChartLayout::new(360.0, 300.0),
        view_extent: None,
        highlight: None,
    };
    let (scene, _scales) = build_multi_mark_scene(&[&chart_data]);
    assert!(
        count_scene_paths(&scene) > 0,
        "rebuilt plot B scene draws the filtered marks"
    );

    // --- Clear round-trip: retract the brush, plot B returns to full data. ---
    // A click (Idle interaction) routes through commit_brush_clear → the
    // Session's clear_selection, re-executing the subscriber unfiltered.
    let (_cleared_state, clear_results) =
        commit_brush_clear(&InteractionState::Idle, &bindings[0], &mut session);
    assert_eq!(
        clear_results.len(),
        1,
        "clear re-executes the one subscriber mark"
    );
    let (clear_idx, clear_res) = &clear_results[0];
    assert_eq!(*clear_idx, 1);
    let cleared_rows = total_rows(clear_res.as_ref().expect("plot B re-executes on clear"));
    assert_eq!(
        cleared_rows, baseline_rows,
        "clearing the brush restores all {baseline_rows} rows"
    );
}

/// scenario 1 (Y): an analysis-derived `intervalY` binding threaded through a
/// live Session reduces the downstream plot on the y column — the y-channel
/// resolution path into an actual row reduction, which was proven nowhere
/// end-to-end (only intervalX was).
#[test]
fn crossfilter_brush_interval_y_filters_downstream() {
    let parsed = parse_spec(SPEC_Y, Format::Yaml).expect("spec parses");
    let spec = parsed.spec;
    let analysis = analyse_spec(&spec).expect("spec analyses");

    let spec_binding = &analysis.brushable_bindings[0];
    assert_eq!(
        spec_binding.channels.y.as_deref(),
        Some("y"),
        "y channel resolved from plot A's dot mark"
    );
    let bindings = [BrushBinding::from(spec_binding)];

    let engine = Engine::new();
    let mut session = engine
        .load_spec(spec, analysis, None)
        .expect("spec loads")
        .session;

    let baseline_rows = total_rows(session.execute_all()[1].as_ref().expect("plot B executes"));
    assert_eq!(baseline_rows, 6, "all 6 rows before any brush");

    // Brush a y-range [25, 45] in DATA coords; x is immaterial for intervalY.
    let mut interaction = InteractionState::start_brush(Point::new(0.0, 25.0));
    interaction.update_brush(Point::new(100.0, 45.0));
    let (_state, aggregated) = commit_brush_release_multi(&interaction, &bindings, &mut session);

    let (_name, results) = &aggregated[0];
    let (mark_index, result) = &results[0];
    assert_eq!(*mark_index, 1, "the subscriber is plot B's mark");
    let filtered_rows = total_rows(result.as_ref().expect("plot B re-executes under y-brush"));
    // y∈[25,45] keeps y=30,40 → rows x=3,4: 2 of 6.
    assert!(
        filtered_rows < baseline_rows,
        "intervalY brush reduces plot B rows: {filtered_rows} !< {baseline_rows}"
    );
    assert_eq!(filtered_rows, 2, "y∈[25,45] keeps y=30,40");
}

/// scenario 1 (XY): an analysis-derived `intervalXY` binding restricts the
/// downstream plot on BOTH axes. The ranges are chosen so the 2D result is
/// strictly smaller than either axis alone would give — proving both bounds
/// bind, not just one.
#[test]
fn crossfilter_brush_interval_xy_filters_downstream() {
    let parsed = parse_spec(SPEC_XY, Format::Yaml).expect("spec parses");
    let spec = parsed.spec;
    let analysis = analyse_spec(&spec).expect("spec analyses");

    let spec_binding = &analysis.brushable_bindings[0];
    assert_eq!(spec_binding.channels.x.as_deref(), Some("x"));
    assert_eq!(spec_binding.channels.y.as_deref(), Some("y"));
    let bindings = [BrushBinding::from(spec_binding)];

    let engine = Engine::new();
    let mut session = engine
        .load_spec(spec, analysis, None)
        .expect("spec loads")
        .session;

    let baseline_rows = total_rows(session.execute_all()[1].as_ref().expect("plot B executes"));
    assert_eq!(baseline_rows, 6);

    // Brush x∈[2.5,5.5] (would keep x=3,4,5 → 3 rows on its own) AND
    // y∈[15,45] (would keep y=20,30,40 → x=2,3,4 → 3 rows on its own).
    // The intersection is x∈{3,4} = 2 rows: strictly fewer than either axis,
    // so a dropped bound on either axis would show 3, not 2.
    let mut interaction = InteractionState::start_brush(Point::new(2.5, 15.0));
    interaction.update_brush(Point::new(5.5, 45.0));
    let (_state, aggregated) = commit_brush_release_multi(&interaction, &bindings, &mut session);

    let (_name, results) = &aggregated[0];
    let (mark_index, result) = &results[0];
    assert_eq!(*mark_index, 1);
    let filtered_rows = total_rows(result.as_ref().expect("plot B re-executes under xy-brush"));
    assert_eq!(
        filtered_rows, 2,
        "intervalXY intersects both axes: x∈{{3,4}} (fewer than either axis's 3)"
    );
}

/// Self-exclusion: a plot that brushes AND is filtered by its own selection
/// must not filter itself. Brushing a sub-range leaves the mark's rows
/// unchanged.
///
/// This was the executable repro for the self-exclusion defect (contributor
/// identity used the interactor's item-index path while the subscriber used the
/// mark's, so within one plot they never matched). Fixed by `plot_node_path` —
/// both sides now resolve to the stable plot-node identity.
#[test]
fn crossfilter_plot_does_not_filter_itself() {
    let parsed = parse_spec(SELF_SPEC, Format::Yaml).expect("spec parses");
    let spec = parsed.spec;
    let analysis = analyse_spec(&spec).expect("spec analyses");
    let binding: BrushBinding = (&analysis.brushable_bindings[0]).into();
    let bindings = [binding];

    let engine = Engine::new();
    let mut session = engine
        .load_spec(spec, analysis, None)
        .expect("spec loads")
        .session;

    let baseline_rows = total_rows(session.execute_all()[0].as_ref().expect("executes"));
    assert_eq!(baseline_rows, 6);

    let mut interaction = InteractionState::start_brush(Point::new(2.5, 0.0));
    interaction.update_brush(Point::new(4.5, 100.0));
    let (_state, aggregated) = commit_brush_release_multi(&interaction, &bindings, &mut session);

    let (_name, results) = &aggregated[0];
    let (_idx, result) = &results[0];
    let rows = total_rows(result.as_ref().expect("self re-executes"));
    assert_eq!(
        rows, baseline_rows,
        "a plot must not filter itself under crossfilter (self-exclusion)"
    );
}

/// Plot A carries a `toggleX` point selection; plot B is filtered by it. A
/// click on data value x=3 must restrict plot B to that single value.
const SPEC_POINT: &str = r#"
params:
  pt: { select: crossfilter }
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
    - { x: 4, y: 40 }
    - { x: 5, y: 50 }
    - { x: 6, y: 60 }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
    - select: toggleX
      as: $pt
    width: 360
    height: 300
  - plot:
    - mark: dot
      data: { from: t, filterBy: $pt }
      x: x
      y: y
    width: 360
    height: 300
"#;

/// scenario 5 (feature) — the point-selection DATA path, end-to-end at the
/// engine level: analysis yields a PointX binding, `point_to_predicate` turns a
/// clicked value into `x = <value>`, and a live Session restricts the downstream
/// plot to that value. NB this drives `point_to_predicate` + `propagate_selection`
/// directly; the live click gesture that RESOLVES a pixel to that data value
/// (and thus calls `point_to_predicate` in production) is the deferred,
/// window-gated increment — so this proves the adapter + engine, not the pixel
/// gesture. The interval tests above, by contrast, go through the production
/// `commit_brush_release_multi` drag path.
#[test]
fn crossfilter_point_toggle_x_filters_downstream() {
    let parsed = parse_spec(SPEC_POINT, Format::Yaml).expect("spec parses");
    let spec = parsed.spec;
    let analysis = analyse_spec(&spec).expect("spec analyses");

    let ui_binding = BrushBinding::from(&analysis.brushable_bindings[0]);
    assert_eq!(ui_binding.kind, BrushKind::PointX, "toggleX → PointX");
    assert_eq!(ui_binding.channels.x.as_deref(), Some("x"));

    let engine = Engine::new();
    let mut session = engine
        .load_spec(spec, analysis, None)
        .expect("spec loads")
        .session;
    let baseline_rows = total_rows(session.execute_all()[1].as_ref().expect("plot B executes"));
    assert_eq!(baseline_rows, 6);

    // A click on data value x=3 → point predicate `x = 3`.
    let pred = point_to_predicate(3.0, ui_binding.kind, &ui_binding.channels);
    let results = session.propagate_selection(
        &ui_binding.selection_name,
        ui_binding.contributor.clone(),
        pred,
    );
    let (mark_index, result) = &results[0];
    assert_eq!(*mark_index, 1, "plot B (index 1) is the subscriber");
    let rows = total_rows(result.as_ref().expect("plot B re-executes under point selection"));
    assert_eq!(rows, 1, "x = 3 selects exactly one row");
}

/// Plot A drives TWO selections — a `toggleX` point (`$pt`) and an `intervalY`
/// (`$iv`). Plot B is filtered by `$pt`, plot C by `$iv`. Each selection must
/// reach only its own subscriber with only its own predicate.
const SPEC_TWO_SELECTIONS: &str = r#"
params:
  pt: { select: crossfilter }
  iv: { select: crossfilter }
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
    - { x: 4, y: 40 }
    - { x: 5, y: 50 }
    - { x: 6, y: 60 }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
    - select: toggleX
      as: $pt
    - select: intervalY
      as: $iv
    width: 360
    height: 300
  - plot:
    - mark: dot
      data: { from: t, filterBy: $pt }
      x: x
      y: y
    width: 360
    height: 300
  - plot:
    - mark: dot
      data: { from: t, filterBy: $iv }
      x: x
      y: y
    width: 360
    height: 300
"#;

/// scenario 5 (the decisive independence assertion): one plot drives a point
/// selection AND an interval selection into two distinct channels. A subscriber
/// to the point selection sees ONLY the point predicate; a subscriber to the
/// interval sees ONLY the interval predicate. Values are chosen so any
/// cross-contamination would collapse a subscriber to zero rows.
#[test]
fn crossfilter_two_selections_stay_independent() {
    let parsed = parse_spec(SPEC_TWO_SELECTIONS, Format::Yaml).expect("spec parses");
    let spec = parsed.spec;
    let analysis = analyse_spec(&spec).expect("spec analyses");

    // Two bindings on plot A: a PointX ($pt) and an IntervalY ($iv).
    use brightfield_spec::analysis::BrushKind as Spec;
    let pt_b = analysis
        .brushable_bindings
        .iter()
        .find(|b| b.kind == Spec::PointX)
        .map(BrushBinding::from)
        .expect("toggleX binding");
    let iv_b = analysis
        .brushable_bindings
        .iter()
        .find(|b| b.kind == Spec::IntervalY)
        .map(BrushBinding::from)
        .expect("intervalY binding");
    assert_eq!(pt_b.selection_name, "pt");
    assert_eq!(iv_b.selection_name, "iv");

    let engine = Engine::new();
    let mut session = engine
        .load_spec(spec, analysis, None)
        .expect("spec loads")
        .session;

    // Values chosen so cross-contamination collapses a subscriber to 0 rows:
    // $pt = x=5 (its row has y=50, OUTSIDE the interval) and $iv = y∈[25,45]
    // (rows y=30,40 → x=3,4, none of which is x=5). So x=5 AND y∈[25,45] = 0.
    let pt_pred = point_to_predicate(5.0, pt_b.kind, &pt_b.channels);
    let iv_pred = brush_rect_to_predicate(Rect::new(0.0, 25.0, 100.0, 45.0), iv_b.kind, &iv_b.channels);

    // Populate BOTH selections first, so each subscriber is later measured with
    // both predicates live — otherwise a leak from the not-yet-active selection
    // could never show up in a row count.
    let pt_first =
        session.propagate_selection(&pt_b.selection_name, pt_b.contributor.clone(), pt_pred.clone());
    let iv_results =
        session.propagate_selection(&iv_b.selection_name, iv_b.contributor.clone(), iv_pred);

    // Routing independence: each selection re-executes ONLY its own subscriber.
    assert_eq!(pt_first.len(), 1, "only plot B subscribes to $pt");
    assert_eq!(pt_first[0].0, 1, "$pt subscriber is plot B (mark 1)");
    assert_eq!(iv_results.len(), 1, "only plot C subscribes to $iv");
    let (iv_idx, iv_res) = &iv_results[0];
    assert_eq!(*iv_idx, 2, "$iv subscriber is plot C (mark 2)");

    // Predicate independence, $pt→C direction: C is measured with BOTH active,
    // so a $pt leak into C would give x=5 AND y∈[25,45] = 0 ≠ 2.
    assert_eq!(
        total_rows(iv_res.as_ref().expect("C re-executes")),
        2,
        "plot C sees only $iv (y∈[25,45]) → 2 rows; would be 0 if $pt leaked in"
    );

    // Predicate independence, $iv→B direction: re-propagate $pt so plot B is
    // re-measured while $iv is ALSO live. A $iv leak into B would give
    // x=5 AND y∈[25,45] = 0 ≠ 1.
    let pt_again =
        session.propagate_selection(&pt_b.selection_name, pt_b.contributor.clone(), pt_pred);
    let (pt_idx, pt_res) = &pt_again[0];
    assert_eq!(*pt_idx, 1);
    assert_eq!(
        total_rows(pt_res.as_ref().expect("B re-executes")),
        1,
        "plot B sees only $pt (x=5) → 1 row; would be 0 if $iv leaked in"
    );
}

/// scenario 5 (feature) — the LIVE point-CLICK gesture, the pixel→data increment
/// the data-path test above explicitly deferred. A click PIXEL landing on the
/// x=3 datum is resolved by `commit_click_multi` (via `find_nearest` + the plot's
/// scales) to that datum's EXACT stored value, dispatched as `x = 3`; plot B
/// restricts to it. A second click far from every datum finds no hit and CLEARS
/// the selection, restoring plot B to full. This is the production gesture path
/// (pixel in → nearest datum → point_to_predicate → propagate_selection).
#[test]
fn crossfilter_point_click_resolves_and_selects_nearest_datum() {
    let parsed = parse_spec(SPEC_POINT, Format::Yaml).expect("spec parses");
    let spec = parsed.spec;
    let analysis = analyse_spec(&spec).expect("spec analyses");

    let binding = BrushBinding::from(&analysis.brushable_bindings[0]);
    assert_eq!(binding.kind, BrushKind::PointX, "toggleX → PointX");

    // Plot A's mark metadata (index 0), captured before `spec` moves into the
    // session — the nearest-datum search needs the batch + channel columns.
    let marks = collect_marks(&spec);
    let plot_a_channels = ChannelMap::from_mark(marks[0]);

    let engine = Engine::new();
    let mut session = engine
        .load_spec(spec, analysis, None)
        .expect("spec loads")
        .session;

    let baseline = session.execute_all();
    let batch_a = baseline[0].as_ref().expect("plot A executes")[0].clone();
    let baseline_b = total_rows(baseline[1].as_ref().expect("plot B executes"));
    assert_eq!(baseline_b, 6);
    drop(baseline);

    // Scales for plot A over the spec's 360×300 plot. The click pixel is the x=3
    // datum mapped THROUGH these scales, so it lands exactly on that point.
    let layout = ChartLayout::new(360.0, 300.0);
    let scales = infer_scales(&batch_a, &plot_a_channels, layout.x_range(), layout.y_range());
    let x3_px = scales.get(Channel::X).expect("x scale").map_f64(3.0);
    let y3_px = scales.get(Channel::Y).expect("y scale").map_f64(30.0);

    let bindings = [binding];
    let marks_meta = [(&batch_a, &plot_a_channels)];

    // Click on the x=3 datum → `x = 3` → plot B restricts to that one row.
    let (_next, aggregated) = commit_click_multi(
        Point::new(x3_px, y3_px),
        &marks_meta,
        &scales,
        &bindings,
        &mut session,
    );
    assert_eq!(aggregated.len(), 1, "one binding dispatched");
    let (_sel, results) = &aggregated[0];
    let (mark_index, result) = &results[0];
    assert_eq!(*mark_index, 1, "plot B (index 1) is the subscriber");
    assert_eq!(
        total_rows(result.as_ref().expect("plot B re-executes under point select")),
        1,
        "click resolves to x=3 and selects exactly that datum"
    );

    // Click far to the right of every datum (X-distance ≫ hit radius) → a miss →
    // the selection clears, restoring plot B to all 6 rows.
    let x6_px = scales.get(Channel::X).expect("x scale").map_f64(6.0);
    let (_n2, agg2) = commit_click_multi(
        Point::new(x6_px + 200.0, y3_px),
        &marks_meta,
        &scales,
        &bindings,
        &mut session,
    );
    let (_s2, results2) = &agg2[0];
    assert_eq!(
        total_rows(results2[0].1.as_ref().expect("plot B re-executes on clear")),
        baseline_b,
        "a click on empty space clears the point selection → plot B full again"
    );
}

/// A click on a plot whose binding is an INTERVAL (not a point) still CLEARS
/// through the shared click path — the click-outside-brush retract must survive
/// the point-selection addition. Brush to filter plot B, then click to clear.
#[test]
fn crossfilter_interval_click_clears_via_shared_path() {
    let parsed = parse_spec(SPEC, Format::Yaml).expect("spec parses");
    let spec = parsed.spec;
    let analysis = analyse_spec(&spec).expect("spec analyses");
    let binding = BrushBinding::from(&analysis.brushable_bindings[0]);
    assert_eq!(binding.kind, BrushKind::IntervalX);

    let marks = collect_marks(&spec);
    let plot_a_channels = ChannelMap::from_mark(marks[0]);

    let engine = Engine::new();
    let mut session = engine
        .load_spec(spec, analysis, None)
        .expect("spec loads")
        .session;

    let baseline = session.execute_all();
    let batch_a = baseline[0].as_ref().expect("plot A executes")[0].clone();
    let baseline_b = total_rows(baseline[1].as_ref().expect("plot B executes"));
    drop(baseline);

    // First brush x∈[2.5,4.5] (data space) to filter plot B down.
    let bindings = [binding];
    let mut brush = InteractionState::start_brush(Point::new(2.5, 0.0));
    brush.update_brush(Point::new(4.5, 100.0));
    let (_n, agg) = commit_brush_release_multi(&brush, &bindings, &mut session);
    let filtered = total_rows(agg[0].1[0].1.as_ref().expect("B re-executes under brush"));
    assert!(filtered < baseline_b, "brush reduces plot B");

    // Now a click (any pixel) on the interval plot → clears → plot B full again.
    let layout = ChartLayout::new(360.0, 300.0);
    let scales = infer_scales(&batch_a, &plot_a_channels, layout.x_range(), layout.y_range());
    let marks_meta = [(&batch_a, &plot_a_channels)];
    let (_n2, agg2) = commit_click_multi(
        Point::new(180.0, 150.0),
        &marks_meta,
        &scales,
        &bindings,
        &mut session,
    );
    assert_eq!(
        total_rows(agg2[0].1[0].1.as_ref().expect("B re-executes on clear")),
        baseline_b,
        "clicking the interval plot clears its brush → plot B full again"
    );
}

/// A `toggleX` on a CATEGORICAL (string) axis is out of numeric scope: point
/// selection forms a `col = value` numeric predicate that can't equate a string.
/// The gesture SKIPS such a binding — a clean no-op — rather than mis-clearing
/// (find_nearest can't read a string column) or dispatching a broken literal.
/// Categorical point selection is a documented follow-up.
const SPEC_POINT_CATEGORICAL: &str = r#"
params:
  pick: { select: crossfilter }
data:
  t:
    - { cat: A, n: 3 }
    - { cat: B, n: 7 }
    - { cat: C, n: 5 }
hconcat:
  - plot:
    - mark: barY
      data: { from: t }
      x: cat
      y: n
    - select: toggleX
      as: $pick
    width: 360
    height: 300
  - plot:
    - mark: barY
      data: { from: t, filterBy: $pick }
      x: cat
      y: n
    width: 360
    height: 300
"#;

#[test]
fn crossfilter_categorical_point_click_is_noop() {
    let parsed = parse_spec(SPEC_POINT_CATEGORICAL, Format::Yaml).expect("spec parses");
    let spec = parsed.spec;
    let analysis = analyse_spec(&spec).expect("spec analyses");
    let binding = BrushBinding::from(&analysis.brushable_bindings[0]);
    assert_eq!(binding.kind, BrushKind::PointX, "toggleX → PointX");
    assert_eq!(binding.channels.x.as_deref(), Some("cat"), "x is the string column");

    let marks = collect_marks(&spec);
    let plot_a_channels = ChannelMap::from_mark(marks[0]);

    let engine = Engine::new();
    let mut session = engine
        .load_spec(spec, analysis, None)
        .expect("spec loads")
        .session;
    let baseline = session.execute_all();
    let batch_a = baseline[0].as_ref().expect("plot A executes")[0].clone();
    let baseline_b = total_rows(baseline[1].as_ref().expect("plot B executes"));
    assert_eq!(baseline_b, 3);
    drop(baseline);

    let layout = ChartLayout::new(360.0, 300.0);
    let scales = infer_scales(&batch_a, &plot_a_channels, layout.x_range(), layout.y_range());
    let bindings = [binding];
    let marks_meta = [(&batch_a, &plot_a_channels)];

    // A click anywhere on the categorical toggle plot dispatches nothing.
    let (_next, aggregated) = commit_click_multi(
        Point::new(100.0, 100.0),
        &marks_meta,
        &scales,
        &bindings,
        &mut session,
    );
    assert!(
        aggregated.is_empty(),
        "a categorical point binding is skipped — neither selects nor clears"
    );
    assert_eq!(
        total_rows(session.execute_all()[1].as_ref().expect("plot B")),
        baseline_b,
        "plot B is unchanged — the categorical click was a no-op"
    );
}
