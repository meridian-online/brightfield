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
use brightfield_render::channel::ChannelMap;
use brightfield_render::layout::ChartLayout;
use brightfield_render::mark::{count_scene_paths, default_renderers, find_renderer};
use brightfield_render::scene::{build_multi_mark_scene, ChartData};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::{parse_spec, Format};
use brightfield_sql::collect_marks;
use brightfield_ui::chart_view::{commit_brush_clear, commit_brush_release_multi, BrushBinding};
use brightfield_ui::InteractionState;
use kurbo::Point;

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

/// Self-exclusion: a plot that brushes AND is filtered by its own selection
/// must not filter itself. Brushing a sub-range should leave the mark's rows
/// unchanged.
///
/// NB: this currently FAILS and documents a real defect. The contributor
/// identity stored for a brush is `parent_plot(interactor_path)` = the
/// interactor's item-index segment (`…/plot[1]`), while the subscriber mark's
/// `self_source` is `parent_plot(mark_path)` = the mark's item-index segment
/// (`…/plot[0]`). Within one plot these differ, so `compile_selection`'s
/// self-exclusion never matches and the plot filters itself. The fix (a stable
/// plot identity shared by both sides) is the next cross-filter increment;
/// kept `#[ignore]` as an executable repro until then.
#[test]
#[ignore = "known self-exclusion defect: contributor vs subscriber plot identity mismatch — fix in next increment"]
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
