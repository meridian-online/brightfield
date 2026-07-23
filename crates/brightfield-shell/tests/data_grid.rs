//! The Data pane's contract, exercised against a LIVE engine session.
//!
//! Four questions, each asked at the seam it belongs to:
//!
//! 1. **The visible window is the only thing materialised client-side.** The
//!    windowed read over a million-row source returns exactly the window; the
//!    pane's cache is bounded by the viewport plus scroll margin, never by the
//!    result set.
//! 2. **Grid and chart cannot disagree** — the agreement the engine seam
//!    guarantees, re-exercised here **from the grid side**: the same pushed
//!    predicate, read back through the grid's own windowed fetch path, resolves
//!    exactly the rows the chart's query resolves. Not re-implemented — the
//!    single compile path makes it true by construction; this test exercises
//!    that construction through `fetch_page`, the code the pane scrolls with.
//! 3. **Meridian cell chrome, asserted numerically** — the dense row rung via
//!    `Node::rect()` (row pitch, not eyeballed), right-aligned numerics via
//!    two cells' shared right edge, the tabular-numeral declaration pinned to
//!    the token layer at table scope (in the module's unit tests).
//! 4. **Run-state honesty** — a never-run/failed state presents NO rows; a
//!    stale state presents rows only under the stale banner; the pill speaks
//!    the five-state workbench vocabulary and no second one.

use brightfield_engine::coordinator::{Coordinator, Interaction};
use brightfield_engine::{RecordBatch, SqlPredicate};
use brightfield_shell::app::ChartDoc;
use brightfield_shell::data_grid::{fetch_page, DataGridItem, PAGE_PAD};
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::LiveDashboard;
use brightfield_spec::analysis::{analyse_spec, ComponentPath};
use brightfield_spec::{parse_spec, Format};
use brightfield_workbench::subject::RunState;
use brightfield_workbench::{Item, ItemCtx, PaneKey, ViewKind};
use egui_kittest::kittest::Queryable;

// ---------------------------------------------------------------------------
// Engine-side fixtures.
// ---------------------------------------------------------------------------

fn coordinator_from(yaml: &str) -> Coordinator {
    let parsed = parse_spec(yaml, Format::Yaml).expect("parse");
    let analysis = analyse_spec(&parsed.spec).expect("analyse");
    Coordinator::load(parsed.spec, analysis, None).expect("load")
}

/// A brushable two-plot dashboard over five inline rows — the same shape the
/// engine's own agreement tests use, so this file re-exercises the identical
/// property from the consumer side.
const BRUSH_DASHBOARD: &str = r#"
params:
  brush:
    select: intersect
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
    - { x: 4, y: 40 }
    - { x: 5, y: 50 }
vconcat:
  - plot:
    - mark: dot
      data: { from: t, filterBy: $brush }
      x: x
      y: y
  - plot:
    - mark: line
      data: { from: t, filterBy: $brush }
      x: x
      y: y
"#;

/// A million-row source — far more rows than any grid should ever hold.
const MILLION_ROWS: &str = r#"
data:
  t: { query: "SELECT i AS x, (i % 100) AS y FROM range(1000000) t(i)" }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
"#;

fn rows(batches: &[RecordBatch]) -> usize {
    batches.iter().map(RecordBatch::num_rows).sum()
}

// ---------------------------------------------------------------------------
// 1. The visible window never materialises the result set client-side.
// ---------------------------------------------------------------------------

#[test]
fn a_window_of_a_million_row_table_returns_only_the_window() {
    let coord = coordinator_from(MILLION_ROWS);
    let session = coord.session();

    let total = session.step_rows_count(0).expect("count");
    assert_eq!(total, 1_000_000, "the scroll range is the real cardinality");

    // The read the grid scrolls with: a mid-table window, sized like a
    // viewport. Only the window comes back.
    let page = fetch_page(session, 0, 500_000..500_100).expect("window fetch");
    assert_eq!(page.rows.len(), 100, "exactly the window, nothing more");
    assert_eq!(page.window, 500_000..500_100);
    assert_eq!(
        page.columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["x", "y"]
    );
    assert!(
        page.columns.iter().all(|c| c.numeric),
        "both columns are numeric — right-aligned, tabular-numeral scope"
    );
    // The window's first cell is the row at the offset — the LIMIT/OFFSET
    // went into the SQL, so DuckDB did the skipping, not this side.
    assert_eq!(page.rows[0][0].text, "500000");

    // A window at the tail clamps rather than reading past the end.
    let tail = fetch_page(session, 0, 999_990..1_000_000).expect("tail fetch");
    assert_eq!(tail.rows.len(), 10);
    assert_eq!(tail.rows[9][0].text, "999999");
}

#[test]
fn the_windowed_reads_bypass_the_mark_caches() {
    // The windowed reads ride the raw query path: scrolling must never evict
    // a chart's cached result. Executes and cache size are observable on the
    // session, so hold them across a scroll's worth of window reads.
    let mut coord = coordinator_from(BRUSH_DASHBOARD);
    let _ = coord.chart_rows(0).expect("chart read");
    let cached = coord.session().sql_cache_len();
    let executes = coord.session().duckdb_execute_count();

    let session = coord.session();
    for start in [0_u64, 1, 2, 3] {
        let _ = fetch_page(session, 0, start..start + 2).expect("window");
        let _ = session.step_rows_count(0).expect("count");
    }
    assert_eq!(
        coord.session().sql_cache_len(),
        cached,
        "window reads left the SQL->batches LRU untouched"
    );
    assert_eq!(
        coord.session().duckdb_execute_count(),
        executes,
        "window reads did not count against the mark-execute path"
    );
}

// ---------------------------------------------------------------------------
// 2. Grid/chart agreement, re-exercised from the grid side.
// ---------------------------------------------------------------------------

#[test]
fn the_grids_windowed_read_agrees_with_the_chart_under_a_brush() {
    // Push a predicate through the coordinator — the exact interaction seam a
    // real brush uses — then read the SAME step back three ways: the chart's
    // query, the engine's full grid read, and the grid's own windowed fetch
    // path in viewport-sized pages. All three must resolve the identical row
    // set, because all three WHEREs come from one compile path.
    let mut coord = coordinator_from(BRUSH_DASHBOARD);
    let contributor = ComponentPath("root/vconcat[99]".to_string());
    coord.apply(Interaction::Select {
        name: "brush".to_string(),
        contributor,
        predicate: SqlPredicate::Expr("x >= 3".to_string()),
    });

    for mark in 0..=1 {
        let chart = coord.chart_rows(mark).expect("chart");
        let full = coord.grid_rows(mark).expect("grid full read");
        assert_eq!(rows(&chart), 3, "the predicate went into DuckDB");
        assert_eq!(rows(&full), rows(&chart), "the landed agreement holds");

        // The grid side: count + paged windows over the same state.
        let session = coord.session();
        let total = session.step_rows_count(mark).expect("count");
        assert_eq!(total as usize, rows(&chart), "the scroll range agrees");

        let mut seen: Vec<String> = Vec::new();
        let mut start = 0;
        while start < total {
            let page = fetch_page(session, mark, start..(start + 2).min(total)).expect("page");
            assert!(page.rows.len() <= 2, "no page exceeds its window");
            for row in &page.rows {
                seen.push(row[0].text.clone());
            }
            start += 2;
        }
        seen.sort_unstable();
        assert_eq!(
            seen,
            ["3", "4", "5"],
            "mark {mark}: the union of the grid's windows is exactly the \
             chart's filtered row set — two queries over one materialisation \
             cannot disagree"
        );
    }
}

#[test]
fn clearing_the_brush_restores_the_grids_full_range() {
    let mut coord = coordinator_from(BRUSH_DASHBOARD);
    let contributor = ComponentPath("root/vconcat[99]".to_string());
    coord.apply(Interaction::Select {
        name: "brush".to_string(),
        contributor: contributor.clone(),
        predicate: SqlPredicate::Expr("x > 4".to_string()),
    });
    assert_eq!(coord.session().step_rows_count(0).expect("count"), 1);
    coord.apply(Interaction::ClearSelect {
        name: "brush".to_string(),
        contributor,
    });
    assert_eq!(
        coord.session().step_rows_count(0).expect("count"),
        5,
        "retraction re-queries: the grid's range follows the interaction state"
    );
}

// ---------------------------------------------------------------------------
// The pane over a live document, driven through the real Item contract.
// ---------------------------------------------------------------------------

/// A live document over `yaml`, headless (no GPU anywhere near this pane).
fn live_doc(yaml: &str) -> ChartDoc {
    let mut live = LiveDashboard::load_str(yaml, None).expect("load live");
    let composed = live.present().expect("present");
    let mut doc = ChartDoc::headless(composed);
    doc.attach_live(live);
    doc
}

/// Drive the pane's `ui` once inside a plain egui context sized like a pane.
fn run_pane(doc: &mut ChartDoc, item: &mut DataGridItem, ctx: &egui::Context) {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(640.0, 400.0),
        )),
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            let mut requests = Vec::new();
            let mut cx = ItemCtx::new(
                Mode::Light,
                PaneKey::new(ViewKind::Charts, item.item_id()),
                egui_tiles::TileId::from_u64(1),
                true,
                &mut requests,
            );
            item.ui(doc, ui, &mut cx);
        });
    });
}

#[test]
fn the_pane_holds_a_window_never_the_table() {
    // The pane itself, over the million-row source: after frames of drawing,
    // what it holds client-side is bounded by the viewport plus the scroll
    // margin — a function of the window, never of the million rows behind it.
    let mut doc = live_doc(MILLION_ROWS);
    let mut item = DataGridItem::new();
    let ctx = egui::Context::default();
    for _ in 0..4 {
        run_pane(&mut doc, &mut item, &ctx);
    }
    let held = item.held_rows();
    assert!(held > 0, "the pane fetched its first window");
    let viewport_rows = (400.0_f32 / meridian_design::spacing::ROW_DENSE).ceil() as usize;
    let bound = viewport_rows + 2 * (PAGE_PAD as usize) + 2;
    assert!(
        held <= bound,
        "the pane holds {held} rows — more than the viewport-plus-margin \
         bound of {bound}; something materialised beyond the visible window"
    );
}

#[test]
fn a_brush_generation_drops_the_panes_cache_and_refetches() {
    let mut doc = live_doc(BRUSH_DASHBOARD);
    let mut item = DataGridItem::new();
    let ctx = egui::Context::default();
    for _ in 0..3 {
        run_pane(&mut doc, &mut item, &ctx);
    }
    assert_eq!(item.held_rows(), 5, "all five rows fit one window");

    // The same interaction path a real brush takes, through the document.
    assert!(doc.apply_interaction(Interaction::Select {
        name: "brush".to_string(),
        contributor: ComponentPath("root/vconcat[99]".to_string()),
        predicate: SqlPredicate::Expr("x >= 4".to_string()),
    }));
    for _ in 0..3 {
        run_pane(&mut doc, &mut item, &ctx);
    }
    assert_eq!(
        item.held_rows(),
        2,
        "the generation key dropped the stale window and the grid re-read \
         the new materialisation state — never a stale frame presented"
    );
}

// ---------------------------------------------------------------------------
// 3. The cell chrome, numerically.
// ---------------------------------------------------------------------------

/// A harness drawing the real pane over a live five-row document.
fn grid_harness(doc: ChartDoc) -> egui_kittest::Harness<'static, (ChartDoc, DataGridItem)> {
    egui_kittest::Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_ui_state(
            move |ui, state: &mut (ChartDoc, DataGridItem)| {
                brightfield_shell::design::apply(ui.ctx(), Mode::Light);
                let (doc, item) = state;
                let mut requests = Vec::new();
                let mut cx = ItemCtx::new(
                    Mode::Light,
                    PaneKey::new(ViewKind::Charts, item.item_id()),
                    egui_tiles::TileId::from_u64(1),
                    true,
                    &mut requests,
                );
                item.ui(doc, ui, &mut cx);
            },
            (doc, DataGridItem::new()),
        )
}

#[test]
fn the_row_pitch_is_the_dense_rung_measured_by_node_rect() {
    let mut harness = grid_harness(live_doc(BRUSH_DASHBOARD));
    harness.run();

    // Two vertically adjacent numeric cells in the x column. `Node::rect()`
    // is the accessibility tree's own geometry — the pitch between the two
    // rows IS the row height, measured, not eyeballed.
    let r1 = harness.get_by_label("1").rect();
    let r2 = harness.get_by_label("2").rect();
    let pitch = r2.top() - r1.top();
    assert!(
        (pitch - meridian_design::spacing::ROW_DENSE).abs() < 0.5,
        "row pitch {pitch} is not the dense rung {}",
        meridian_design::spacing::ROW_DENSE
    );
}

#[test]
fn numeric_cells_right_align_on_a_shared_edge() {
    let mut harness = grid_harness(live_doc(
        r#"
data:
  t:
    - { x: 7, y: 1000 }
    - { x: 8, y: 5 }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
"#,
    ));
    harness.run();

    // Two numeric cells of very different widths in one column: right
    // alignment means their RIGHT edges coincide while their lefts do not —
    // magnitude alignment, asserted on the node geometry.
    let wide = harness.get_by_label("1000").rect();
    let narrow = harness.get_by_label("5").rect();
    assert!(
        (wide.right() - narrow.right()).abs() < 0.5,
        "numeric cells do not share a right edge: {} vs {}",
        wide.right(),
        narrow.right()
    );
    assert!(
        (wide.left() - narrow.left()).abs() > 1.0,
        "cells of different widths sharing a left edge would mean the column \
         is left-aligned"
    );
}

#[test]
fn the_header_row_exists_and_speaks_the_schema() {
    let mut harness = grid_harness(live_doc(BRUSH_DASHBOARD));
    harness.run();
    // `egui_table` lays its header out in both split-scroll regions (the
    // sticky-column region is zero-width here but still populates the
    // accessibility tree), so the header label can appear more than once —
    // the assertion is presence, not cardinality.
    assert!(harness.query_all_by_label("x").next().is_some());
    assert!(harness.query_all_by_label("y").next().is_some());
}

// ---------------------------------------------------------------------------
// 4. Run-state honesty at the pane.
// ---------------------------------------------------------------------------

/// The same live document, with its preview annotated as `state` — the
/// ingestion path `Composed::with_run_state` models (recorded by a run,
/// never computed shell-side).
fn annotated_doc(state: RunState) -> ChartDoc {
    let mut live = LiveDashboard::load_str(BRUSH_DASHBOARD, None).expect("load live");
    let composed = live.present().expect("present").with_run_state(state);
    let mut doc = ChartDoc::headless(composed);
    doc.attach_live(live);
    doc
}

#[test]
fn a_never_run_state_presents_no_rows() {
    let mut harness = grid_harness(annotated_doc(RunState::NeverRun));
    harness.run();
    // The pill speaks the vocabulary's own words...
    let _ = harness.get_by_label(RunState::NeverRun.label());
    // ...and no materialised row is presented as though it existed.
    assert!(
        harness.query_by_label("1").is_none(),
        "a never-run step presented a materialised row as current"
    );
}

#[test]
fn a_failed_state_presents_no_rows() {
    let mut harness = grid_harness(annotated_doc(RunState::Failed));
    harness.run();
    let _ = harness.get_by_label(RunState::Failed.label());
    assert!(
        harness.query_by_label("1").is_none(),
        "a failed step presented rows its run never produced"
    );
}

#[test]
fn a_stale_state_presents_rows_only_under_the_stale_banner() {
    let mut harness = grid_harness(annotated_doc(RunState::StaleUpstream));
    harness.run();
    // The banner is there, in the vocabulary's words...
    let _ = harness.get_by_label(RunState::StaleUpstream.label());
    // ...and the rows render beneath it: stale data is shown, labelled.
    let _ = harness.get_by_label("1");
}

#[test]
fn a_fresh_state_presents_rows_under_the_fresh_pill() {
    let mut harness = grid_harness(annotated_doc(RunState::Fresh));
    harness.run();
    let _ = harness.get_by_label(RunState::Fresh.label());
    let _ = harness.get_by_label("1");
}

// ---------------------------------------------------------------------------
// Snapshots — the grid's own goldens (light + dark).
// ---------------------------------------------------------------------------

/// The pane at rest over the five-row fixture: header band, dense rows,
/// zebra striping, right-aligned numerics. Rendered through kittest's wgpu
/// backend like the sheet tier — this pane is pure egui chrome (the Vello
/// canvas is nowhere near it), so the kittest renderer sees all of it.
fn grid_snapshot(mode: Mode, name: &str) {
    let mut live = LiveDashboard::load_str(BRUSH_DASHBOARD, None).expect("load live");
    let composed = live.present().expect("present");
    let mut doc = ChartDoc::headless(composed);
    doc.attach_live(live);

    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(420.0, 220.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .build_ui_state(
            move |ui, state: &mut (ChartDoc, DataGridItem)| {
                brightfield_shell::design::apply(ui.ctx(), mode);
                egui::CentralPanel::default().show(ui, |ui| {
                    let (doc, item) = state;
                    let mut requests = Vec::new();
                    let mut cx = ItemCtx::new(
                        mode,
                        PaneKey::new(ViewKind::Charts, item.item_id()),
                        egui_tiles::TileId::from_u64(1),
                        true,
                        &mut requests,
                    );
                    item.ui(doc, ui, &mut cx);
                });
            },
            (doc, DataGridItem::new()),
        );
    harness.run();
    harness.snapshot(name);
}

#[test]
fn data_grid_light_snapshot() {
    grid_snapshot(Mode::Light, "data_grid_light");
}

#[test]
fn data_grid_dark_snapshot() {
    grid_snapshot(Mode::Dark, "data_grid_dark");
}
