//! The Data pane's contract, exercised against a LIVE engine session.
//!
//! Four questions, each asked at the seam it belongs to:
//!
//! 1. **The visible window is the only thing materialised client-side.** The
//!    windowed read over a million-row source returns exactly the window; the
//!    pane's cache is bounded by the viewport plus scroll margin, never by the
//!    result set. And the window is **deterministic**: the engine imposes a
//!    total order on the windowed read, so re-reading a window is
//!    byte-identical and adjacent windows tile — even over a source whose
//!    view body ends in an order-unstable operator (`GROUP BY`, hash joins,
//!    `DISTINCT`), where DuckDB's per-execution insertion order would
//!    otherwise hand each `LIMIT`/`OFFSET` slice a fresh permutation.
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
use brightfield_engine::{RecordBatch, RowsAudience, SqlPredicate};
use brightfield_shell::app::ChartDoc;
use brightfield_shell::data_grid::{fetch_page, DataGridItem, GridPage, PAGE_PAD};
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::{presenting_rows_mark, LiveDashboard};
use brightfield_spec::analysis::{analyse_spec, ComponentPath};
use brightfield_spec::{parse_spec, Format};
use brightfield_workbench::subject::RunState;
use brightfield_workbench::{Item, ItemCtx, PaneKey};
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

/// **The shape every generated tile writes**: two layers over one source, the
/// first with no `filterBy:` (the ghost, the whole table) and the second bound
/// to a `crossfilter` selection (the subset, what a brush leaves) — see
/// `chart_kinds::point_map_tile` and the histogram and scatter tiles beside it.
///
/// [`BRUSH_DASHBOARD`] cannot stand in for it and that is why this exists.
/// Both of its marks carry `filterBy:` and its selection is `intersect`, so it
/// has no ghost to be misread as the presenting layer and no self-exclusion to
/// drop a reader's brush. A test driven only through it is green on a pane
/// reading mark 0 with the chart's own audience, which is exactly the pane
/// that listed 240 of 240 rows under a brush.
const GHOST_AND_SUBSET: &str = r#"
params:
  sel:
    select: crossfilter
data:
  t:
    - { x: 1 }
    - { x: 2 }
    - { x: 3 }
    - { x: 4 }
    - { x: 5 }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: x
  - mark: dot
    data: { from: t, filterBy: $sel }
    x: x
    y: x
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

    let total = session.step_rows_count(0, RowsAudience::Reader).expect("count");
    assert_eq!(total, 1_000_000, "the scroll range is the real cardinality");

    // The read the grid scrolls with: a mid-table window, sized like a
    // viewport. Only the window comes back.
    let page = fetch_page(session, 0, 500_000..500_100, RowsAudience::Reader).expect("window fetch");
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
    let tail = fetch_page(session, 0, 999_990..1_000_000, RowsAudience::Reader).expect("tail fetch");
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
        let _ = fetch_page(session, 0, start..start + 2, RowsAudience::Reader).expect("window");
        let _ = session.step_rows_count(0, RowsAudience::Reader).expect("count");
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
// 1b. The windowed read is deterministic over an order-unstable source.
// ---------------------------------------------------------------------------

/// An ORDER-UNSTABLE source: the rows view's body ends in a `GROUP BY` — a
/// hash aggregate, whose emission order DuckDB does NOT hold stable across
/// executions (insertion-order preservation is per-execution; a hash
/// operator's output order is not). Five thousand groups, enough that two
/// executions of the aggregate essentially never agree — the shape under
/// which an unordered `LIMIT`/`OFFSET` window tears.
const UNSTABLE_GROUPED: &str = r#"
data:
  t: { query: "SELECT (i % 5000) AS x, count(*) AS n, sum(i) AS s FROM range(50000) t(i) GROUP BY x" }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: n
"#;

/// A page's cells as text, row-major — the byte-identity the stability
/// assertions compare.
fn page_texts(page: &GridPage) -> Vec<Vec<String>> {
    page.rows
        .iter()
        .map(|row| row.iter().map(|cell| cell.text.clone()).collect())
        .collect()
}

#[test]
fn re_reading_one_window_of_an_order_unstable_source_is_byte_identical() {
    let coord = coordinator_from(UNSTABLE_GROUPED);
    let session = coord.session();
    assert_eq!(
        session.step_rows_count(0, RowsAudience::Reader).expect("count"),
        5_000
    );

    // The same mid-table window, fetched repeatedly — what a grid does every
    // time a scroll position is revisited. Without the total order the
    // engine imposes on the windowed read, each fetch re-executes the hash
    // aggregate and `OFFSET` slices a fresh permutation: this exact read
    // returned a different row set on most fetches. Byte-identity, every
    // time, is the contract a scrollbar rests on.
    let window = 2_000..2_100;
    let first = page_texts(&fetch_page(session, 0, window.clone(), RowsAudience::Reader).expect("first read"));
    assert_eq!(first.len(), 100, "exactly the window came back");
    for read in 1..=5 {
        let again = page_texts(&fetch_page(session, 0, window.clone(), RowsAudience::Reader).expect("re-read"));
        assert_eq!(
            again, first,
            "re-read {read} of the same window returned a different row set"
        );
    }
}

#[test]
fn adjacent_windows_of_an_order_unstable_source_tile_the_full_read() {
    let coord = coordinator_from(UNSTABLE_GROUPED);
    let session = coord.session();
    let total = session.step_rows_count(0, RowsAudience::Reader).expect("count");

    // The whole step in one ordered read — the reference row set.
    let full = page_texts(&fetch_page(session, 0, 0..total, RowsAudience::Reader).expect("full read"));
    assert_eq!(full.len(), total as usize);

    // The same step as adjacent windows, sized to misalign with any internal
    // batch boundary. Scrolled pages must tile the materialisation: no row
    // lost at a seam, none duplicated across one — which holds only if every
    // window is a slice of ONE total order rather than of per-read
    // permutations.
    let mut tiled: Vec<Vec<String>> = Vec::new();
    let mut start = 0_u64;
    while start < total {
        let end = (start + 97).min(total);
        tiled.extend(page_texts(
            &fetch_page(session, 0, start..end, RowsAudience::Reader).expect("page"),
        ));
        start = end;
    }
    assert_eq!(
        tiled, full,
        "adjacent windows do not reassemble the full ordered read — a seam \
         duplicated or dropped rows"
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
        let full = coord.grid_rows(mark, RowsAudience::Plot).expect("grid full read");
        assert_eq!(rows(&chart), 3, "the predicate went into DuckDB");
        assert_eq!(rows(&full), rows(&chart), "the landed agreement holds");

        // The grid side: count + paged windows over the same state.
        let session = coord.session();
        let total = session.step_rows_count(mark, RowsAudience::Plot).expect("count");
        assert_eq!(total as usize, rows(&chart), "the scroll range agrees");

        let mut seen: Vec<String> = Vec::new();
        let mut start = 0;
        while start < total {
            let page = fetch_page(session, mark, start..(start + 2).min(total), RowsAudience::Plot).expect("page");
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
    assert_eq!(
        coord
            .session()
            .step_rows_count(0, RowsAudience::Reader)
            .expect("count"),
        1
    );
    coord.apply(Interaction::ClearSelect {
        name: "brush".to_string(),
        contributor,
    });
    assert_eq!(
        coord.session()
            .step_rows_count(0, RowsAudience::Reader)
            .expect("count"),
        5,
        "retraction re-queries: the grid's range follows the interaction state"
    );
}

/// **The rows pane's read resolves to the filtered layer, and drops nobody's
/// clause** — the engine-level half of the defect, over a TWO-layer spec.
///
/// Three readings of one brush, and the pane's answer is the third:
///
/// | mark | audience | rows | why |
/// |---|---|---|---|
/// | 0 (ghost) | `Reader` | 5 | it declares no `filterBy:`, so no predicate reaches it at all |
/// | 1 (subset) | `Plot` | 5 | crossfilter drops the clause this plot published — its own |
/// | 1 (subset) | `Reader` | 2 | nothing dropped: the selection's value |
///
/// The first row is what the pane used to read. The second is what reading the
/// subset layer *without* fixing the audience would read, and it is the same
/// wrong number by a different route — which is why both terms are asserted
/// here rather than only the one the fix started from.
///
/// **The contributor is the plot's own node path**, taken off the composition
/// rather than typed. A synthetic path (`root/plot[99]`, which the tests above
/// use) is not this plot, so crossfilter drops nothing for it and mark 1 at
/// `Plot` would answer 2 — the middle row would pass against the unfixed code
/// and the test would pin nothing.
#[test]
fn the_rows_pane_reads_the_filtered_layer_and_drops_no_contributors_clause() {
    let parsed = parse_spec(GHOST_AND_SUBSET, Format::Yaml).expect("parse");
    assert_eq!(
        presenting_rows_mark(&parsed.spec),
        1,
        "the ghost is mark 0 and the layer carrying `filterBy:` is mark 1; a          rule answering 0 here is the rule that read the whole table"
    );

    let mut live = LiveDashboard::load_str(GHOST_AND_SUBSET, None).expect("load live");
    let composed = live.present().expect("present");
    let plot = ComponentPath(composed.plots[0].path.clone());
    live.apply(Interaction::Select {
        name: "sel".to_string(),
        contributor: plot,
        predicate: SqlPredicate::Expr("x >= 4".to_string()),
    })
    .expect("the brush re-composites");

    let mark = presenting_rows_mark(&parsed.spec);
    let session = live.coordinator().session();
    assert_eq!(
        session.step_rows_count(0, RowsAudience::Reader).expect("ghost"),
        5,
        "the ghost layer narrows under nothing, which is what makes reading          mark 0 a grid that never moves"
    );
    assert_eq!(
        session
            .step_rows_count(mark, RowsAudience::Plot)
            .expect("subset as the plot"),
        5,
        "the subset layer asked as its own plot drops the clause that plot          published, so the fixture's crossfilter is live — if this is 2 the          selection is no longer self-excluding and the reading below proves          nothing"
    );
    assert_eq!(
        session
            .step_rows_count(mark, RowsAudience::Reader)
            .expect("subset as a reader"),
        2,
        "`x >= 4` admits {{4, 5}}, and a reader that published no clause has          none to drop"
    );

    // …and the rows themselves, through the code the pane scrolls with.
    let page = fetch_page(session, mark, 0..2, RowsAudience::Reader).expect("page");
    assert_eq!(
        page_texts(&page),
        vec![vec!["4".to_string()], vec!["5".to_string()]],
        "the pane's own read path lists the rows inside the brush"
    );
    let ghost_page = fetch_page(session, 0, 0..2, RowsAudience::Reader).expect("ghost page");
    assert_eq!(
        page_texts(&ghost_page),
        vec![vec!["1".to_string()], vec!["2".to_string()]],
        "…and the same read at mark 0 answers with the head of the whole          table, which is the picture the reader was given before"
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
                PaneKey::new(item.item_id()),
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
                    PaneKey::new(item.item_id()),
                    egui_tiles::TileId::from_u64(1),
                    true,
                    &mut requests,
                );
                item.ui(doc, ui, &mut cx);
            },
            (doc, DataGridItem::new()),
        )
}

/// A table whose **values** are much wider than their headers, and one column
/// beside it that is not — the shape a column sized from its header alone gets
/// wrong.
const WIDE_VALUES: &str = r#"
data:
  t:
    - { n: 1, note: "Alameda County, unincorporated" }
    - { n: 2, note: "short" }
plot:
  - mark: dot
    data: { from: t }
    x: n
    y: n
"#;

/// **A column is as wide as its widest value, not as wide as its header** —
/// so nothing the grid draws is cut off by the column it is drawn in.
///
/// Read off two drawn rects and nothing else: the accessibility tree's rect
/// for the widest cell, and the header cell rect the table reported for the
/// column that cell is in. A column narrower than the value inside it is a
/// value the reader cannot finish, and it is what a width taken from the
/// header — or a width declared as one number for every column — produces
/// here: `note`'s header is four characters and its widest value is thirty.
///
/// The narrow column beside it is the other half. Without it, a rule that made
/// every column as wide as the widest value in the whole table would pass, and
/// that rule is the one that pushes the rest of the table off screen.
#[test]
fn a_columns_width_covers_its_widest_value_not_just_its_header() {
    const WIDEST: &str = "Alameda County, unincorporated";
    let mut harness = grid_harness(live_doc(WIDE_VALUES));
    harness.run();

    let cell = harness.get_by_label(WIDEST).rect();
    let drawn = harness
        .state()
        .0
        .grid_drawn
        .clone()
        .expect("the grid laid a table out");
    assert_eq!(drawn.columns, 2, "the fixture's table has two columns");

    // Which column that cell is in, by the header rect it falls inside —
    // resolved from the frame rather than assumed to be the second one.
    let (col, header, _) = drawn
        .header_cells
        .iter()
        .find(|(_, rect, _)| rect.x_range().contains(cell.min.x + 1.0))
        .unwrap_or_else(|| {
            panic!(
                "the cell drew at {cell:?}, in none of the columns {:?}",
                drawn.header_cells
            )
        });
    let cell_width = cell.width();
    assert!(
        cell_width <= header.width(),
        "the value {WIDEST:?} drew {cell_width} points wide in a column {}          points wide, so the reader sees it cut off — column {col} was sized          from something other than its widest held value",
        header.width()
    );

    let other = drawn
        .header_cells
        .iter()
        .find(|(c, _, _)| c != col)
        .expect("the table has a second column");
    assert!(
        other.1.width() < header.width(),
        "both columns drew {} points wide, so the width is one number for the          whole table rather than each column's own",
        header.width()
    );
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
                        PaneKey::new(item.item_id()),
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
