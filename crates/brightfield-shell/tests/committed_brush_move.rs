//! **A committed brush rectangle can be dragged to a new place.** A press
//! inside it moves it with the pointer and commits the moved interval on
//! release; a press outside it draws a new one, exactly as before this card.
//!
//! Every gesture below is a real pointer sweep through the whole window —
//! press, move, release — the standing the sibling gesture tiers already
//! give: `tests/committed_selection_ink.rs`'s module doc, and
//! `tests/canvas_pane_group.rs`'s `a_brush_across_the_pane_boundary_commits_what_it_swept`
//! for the two-origin canvas this card's hero/tile split shares.
//!
//! # What "equal to a fresh draw" means in floating point
//!
//! AC1 asks that the moved interval equal the interval a fresh draw of the
//! moved rectangle commits. The two are not produced by the same arithmetic:
//! a moved rectangle's pixel corners are the *committed* selection's data-space
//! bounds mapped forward through the plot's displayed scale
//! (`Scale::map_f64`) and then, on release, inverted back
//! (`Scale::inverse_f64`) — a round trip that is exact algebraically but not
//! bit-exact in IEEE floating point, while a fresh sweep inverts its raw
//! pixels once. The two committed values therefore agree to double precision
//! rather than to the bit, so the comparisons below hold to `1e-6` — the same
//! tolerance `tests/equal_aspect_resize.rs` uses for a domain comparison with
//! the identical shape of concern, and many orders tighter than a pixel.

use brightfield_engine::SqlPredicate;
use brightfield_shell::dashboard::MIN_COLUMN_TILE_HEIGHT;
use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_sql::ir::ScalarValue;
use brightfield_workbench::arrangement;

use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Fixture and frame driving
// ---------------------------------------------------------------------------

/// California Housing, sampled — the same fixture
/// `tests/canvas_pane_group.rs` and `tests/equal_aspect_resize.rs` open: nine
/// numeric columns, a longitude/latitude pair the generator draws as the
/// point-map hero (`intervalXY`), seven others each earning a column tile
/// (`intervalX`).
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv")
}

/// A settled window over the fixture, opened as a data file — three frames,
/// the settle count `tests/canvas_pane_group.rs` and `tests/region_gate.rs`
/// both use for a resizable panel's reported size to be read back.
fn window(screen: egui::Rect) -> (MeridianApp, egui::Context, egui::RawInput) {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut app = MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(screen),
        ..Default::default()
    };
    for _ in 0..3 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    (app, ctx, raw)
}

/// The window the dashboard baseline is photographed in — derived from the
/// composition, exactly as `capture_png` derives it and as
/// `tests/canvas_pane_group.rs`'s own `baseline_screen` does.
fn baseline_screen() -> egui::Rect {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let (w, h) = boot.window_size();
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))
}

fn frame(
    app: &mut MeridianApp,
    ctx: &egui::Context,
    raw: &egui::RawInput,
    events: Vec<egui::Event>,
) {
    let mut input = raw.clone();
    input.events = events;
    let _ = ctx.run_ui(input, |ui| app.draw(ui));
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// A plain sweep: press at `from`, move to `to`, release — the draw path as
/// it stood before this card, used both to seed a committed rectangle and to
/// draw the independent "fresh" comparison rectangle.
fn sweep(
    app: &mut MeridianApp,
    ctx: &egui::Context,
    raw: &egui::RawInput,
    from: egui::Pos2,
    to: egui::Pos2,
) {
    frame(
        app,
        ctx,
        raw,
        vec![egui::Event::PointerMoved(from), button(from, true)],
    );
    frame(app, ctx, raw, vec![egui::Event::PointerMoved(to)]);
    frame(app, ctx, raw, vec![button(to, false)]);
    frame(app, ctx, raw, Vec::new());
    frame(app, ctx, raw, Vec::new());
}

/// Press inside `rect` at its centre, drag by `delta`, release — the move
/// gesture end to end.
fn drag_move(
    app: &mut MeridianApp,
    ctx: &egui::Context,
    raw: &egui::RawInput,
    rect: egui::Rect,
    delta: egui::Vec2,
) {
    let press = rect.center();
    let to = press + delta;
    frame(
        app,
        ctx,
        raw,
        vec![egui::Event::PointerMoved(press), button(press, true)],
    );
    frame(app, ctx, raw, vec![egui::Event::PointerMoved(to)]);
    frame(app, ctx, raw, vec![button(to, false)]);
    frame(app, ctx, raw, Vec::new());
    frame(app, ctx, raw, Vec::new());
}

/// A press and release at the same point — no movement, held one frame the
/// way `tests/canvas_pane_group.rs`'s `a_held_click_on_a_scrolled_tile_clears_the_selection`
/// holds a click, so a phantom travel between press and release would show up
/// here too.
fn click_at(app: &mut MeridianApp, ctx: &egui::Context, raw: &egui::RawInput, at: egui::Pos2) {
    frame(
        app,
        ctx,
        raw,
        vec![egui::Event::PointerMoved(at), button(at, true)],
    );
    frame(app, ctx, raw, Vec::new());
    frame(app, ctx, raw, vec![button(at, false)]);
    frame(app, ctx, raw, Vec::new());
    frame(app, ctx, raw, Vec::new());
}

// ---------------------------------------------------------------------------
// Reading the committed rectangle and the predicate it means
// ---------------------------------------------------------------------------

/// `plot`'s own committed selection, in WINDOW space — the pixel box
/// `PlotHandle::committed_rect` carries, **raster-local**: the same frame
/// `plot.rect` is in, which is not the frame `composed_plot_rects` answers in
/// once a plot is drawn in the canvas's *second* view (a scrolled column
/// tile) — that accessor's own doc names the reason there are two rules for
/// placing a point on it. `committed_rect` is translated by the same offset
/// `composed_plot_rects` already resolved for `plot.rect`, so it inherits
/// the second view's shift instead of re-deriving it.
fn committed_window_rect(app: &MeridianApp, plot: usize) -> egui::Rect {
    let drawn = app.composed_plot_rects()[plot];
    let handle = &app.chart_doc().composed.plots[plot];
    let cr = handle
        .committed_rect
        .unwrap_or_else(|| panic!("plot {plot} holds no committed selection to move"));
    #[allow(clippy::cast_possible_truncation)]
    let (ox, oy) = (
        drawn.left() - handle.rect.x as f32,
        drawn.top() - handle.rect.y as f32,
    );
    #[allow(clippy::cast_possible_truncation)]
    egui::Rect::from_min_size(
        egui::pos2(ox + cr.x as f32, oy + cr.y as f32),
        egui::vec2(cr.width as f32, cr.height as f32),
    )
}

/// The dashboard's one crossfilter selection, structured — the same
/// `Predicate` `LiveDashboard::selection_sql` renders, read here unrendered
/// so the comparisons below are over the numeric bounds rather than over a
/// formatted string. `"sel"` is the name this fixture's generators write:
/// `chart_kinds::point_map` for the hero, `dashboard`'s column-tile builder
/// for a tile.
fn held(app: &MeridianApp) -> Option<SqlPredicate> {
    app.chart_doc()
        .live_dashboard()?
        .selection_clauses()
        .into_iter()
        .find(|(name, _)| name == "sel")
        .map(|(_, p)| p)
}

/// `(column, lo, hi)` for the interval clauses `p` holds, walking `And` the
/// way `chart_item::gather_selected` does. Panics on a shape this file's
/// gestures do not produce (`Or`, `Point`) — a wrong shape here is a fixture
/// bug, not a case to tolerate quietly.
fn intervals(p: &SqlPredicate) -> Vec<(String, f64, f64)> {
    match p {
        SqlPredicate::And(parts) => parts.iter().flat_map(intervals).collect(),
        SqlPredicate::Interval { column, lo, hi, .. } => {
            let as_f64 = |v: &ScalarValue| match v {
                ScalarValue::Float(v) => *v,
                other => panic!("expected a float bound, got {other:?}"),
            };
            vec![(column.clone(), as_f64(lo), as_f64(hi))]
        }
        other => panic!("expected an interval-shaped predicate, got {other:?}"),
    }
}

/// Two predicates hold the same clauses to `1e-6` — see the module doc for
/// why a tolerance and not `assert_eq!`.
fn assert_same_interval(a: &SqlPredicate, b: &SqlPredicate, where_: &str) {
    let mut xa = intervals(a);
    let mut xb = intervals(b);
    xa.sort_by(|p, q| p.0.cmp(&q.0));
    xb.sort_by(|p, q| p.0.cmp(&q.0));
    assert_eq!(
        xa.len(),
        xb.len(),
        "{where_}: different clause shape: {xa:?} vs {xb:?}"
    );
    for ((ca, lo_a, hi_a), (cb, lo_b, hi_b)) in xa.iter().zip(xb.iter()) {
        assert_eq!(ca, cb, "{where_}: different column ({xa:?} vs {xb:?})");
        assert!(
            (lo_a - lo_b).abs() < 1e-6,
            "{where_}: {ca}'s low bound is {lo_a} vs {lo_b}"
        );
        assert!(
            (hi_a - hi_b).abs() < 1e-6,
            "{where_}: {ca}'s high bound is {hi_a} vs {hi_b}"
        );
    }
}

// ---------------------------------------------------------------------------
// AC1 + AC2 — the hero map, at the dashboard baseline's window
// ---------------------------------------------------------------------------

/// A point of the hero's own data area, at `fx`/`fy` of its width/height —
/// resolved against the frame, like `tests/canvas_pane_group.rs`'s
/// `hero_data_point`.
fn hero_point(app: &MeridianApp, fx: f64, fy: f64) -> egui::Pos2 {
    let drawn = app.composed_plot_rects()[0];
    let l = &app.chart_doc().composed.plots[0].layout;
    egui::pos2(
        drawn.left() + (l.plot_x_start() + (l.plot_x_end() - l.plot_x_start()) * fx) as f32,
        drawn.top() + (l.plot_y_start() + (l.plot_y_end() - l.plot_y_start()) * fy) as f32,
    )
}

/// **The whole hero-map claim in one pass**: a press inside the committed
/// rectangle moves it and commits the shifted interval equal to a fresh draw
/// of the moved rectangle (AC1); a press inside with no movement leaves the
/// selection as it was, and a press outside draws a new one (AC2).
#[test]
fn a_press_inside_the_heros_committed_rectangle_moves_it_and_outside_it_draws_a_new_one() {
    let screen = baseline_screen();
    let (mut app, ctx, raw) = window(screen);

    // Draw the rectangle to be moved.
    let a = hero_point(&app, 0.20, 0.20);
    let b = hero_point(&app, 0.55, 0.55);
    sweep(&mut app, &ctx, &raw, a, b);
    held(&app).expect("fixture check: the sweep committed a selection");

    // AC1 — press inside, drag, release.
    let rect = committed_window_rect(&app, 0);
    assert!(
        rect.width() > 8.0 && rect.height() > 8.0,
        "fixture check: the committed rectangle {rect:?} has room to press inside of"
    );
    let delta = egui::vec2(40.0, -30.0);
    drag_move(&mut app, &ctx, &raw, rect, delta);
    let moved = held(&app).expect("the move committed a shifted selection");

    // The independent comparison: a fresh sweep drawn directly at the moved
    // rectangle's corners, in a second window that never saw a move.
    let (mut fresh, fctx, fraw) = window(screen);
    sweep(&mut fresh, &fctx, &fraw, rect.min + delta, rect.max + delta);
    let fresh_predicate = held(&fresh).expect("the fresh sweep committed a selection");
    assert_same_interval(
        &moved,
        &fresh_predicate,
        "AC1: the moved rectangle's predicate vs a fresh draw of it",
    );

    // AC2a — a press inside with no movement leaves the selection as it was.
    let rect_now = committed_window_rect(&app, 0);
    click_at(&mut app, &ctx, &raw, rect_now.center());
    let unmoved = held(&app).expect("fixture check: a click inside left a selection standing");
    assert_same_interval(
        &unmoved,
        &moved,
        "AC2: a press inside with no movement changed the selection",
    );

    // AC2b — a press outside draws a new brush, as today.
    let outside_a = hero_point(&app, 0.05, 0.92);
    let outside_b = hero_point(&app, 0.16, 0.99);
    assert!(
        !rect_now.contains(outside_a) && !rect_now.contains(outside_b),
        "fixture check: the outside sweep {outside_a:?}..{outside_b:?} must not touch the \
         committed rectangle {rect_now:?}"
    );
    sweep(&mut app, &ctx, &raw, outside_a, outside_b);
    let redrawn = held(&app).expect("the outside sweep committed a new selection");
    let (mut fresh2, fctx2, fraw2) = window(screen);
    sweep(&mut fresh2, &fctx2, &fraw2, outside_a, outside_b);
    let fresh_outside = held(&fresh2).expect("the fresh outside sweep committed a selection");
    assert_same_interval(
        &redrawn,
        &fresh_outside,
        "AC2: a press outside the rectangle did not draw a plain new brush there",
    );
}

// ---------------------------------------------------------------------------
// AC3 — a column tile, at scroll 0 and with the column scrolled
// ---------------------------------------------------------------------------

/// The window the scroll-reach gesture tests need — `GESTURE_SCREEN` from
/// `tests/canvas_pane_group.rs`, for the reason recorded there: enough
/// window height taken away that the column outgrows its pane once the
/// ledger rail is reopened.
const GESTURE_SCREEN: egui::Rect = egui::Rect {
    min: egui::Pos2::ZERO,
    max: egui::pos2(1440.0, 780.0),
};

/// One turn of the wheel, in logical points — `tests/canvas_pane_group.rs`'s
/// `WHEEL_NOTCH`.
const WHEEL_NOTCH: f32 = 56.0;

/// Click the collapsed ledger rail's control and settle — the reach the
/// column needs to have anything to scroll, `tests/canvas_pane_group.rs`'s
/// `reopen_the_ledger` unchanged in shape.
fn reopen_the_ledger(app: &mut MeridianApp, ctx: &egui::Context, raw: &egui::RawInput) {
    let at = app
        .rail_collapse_rect(arrangement::LEDGER_RAIL)
        .expect("the collapsed ledger drew the control that reopens it")
        .center();
    frame(app, ctx, raw, vec![egui::Event::PointerMoved(at)]);
    frame(app, ctx, raw, vec![button(at, true), button(at, false)]);
    for _ in 0..3 {
        frame(app, ctx, raw, Vec::new());
    }
}

/// Turn the wheel over the column pane until the scroll stops moving, then
/// settle — `tests/canvas_pane_group.rs`'s `scroll_the_column` unchanged in
/// shape.
fn scroll_the_column(
    app: &mut MeridianApp,
    ctx: &egui::Context,
    raw: &egui::RawInput,
    over: egui::Pos2,
    notches: usize,
) {
    frame(app, ctx, raw, vec![egui::Event::PointerMoved(over)]);
    for _ in 0..notches {
        frame(
            app,
            ctx,
            raw,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -WHEEL_NOTCH),
                modifiers: egui::Modifiers::default(),
                phase: egui::TouchPhase::Move,
            }],
        );
    }
    for _ in 0..6 {
        frame(app, ctx, raw, Vec::new());
    }
}

/// A point across the middle of a stacked tile's data area, at `fx` of its
/// width — `tests/canvas_pane_group.rs`'s `tile_data_point`.
fn tile_point(app: &MeridianApp, tile: usize, fx: f64) -> egui::Pos2 {
    let drawn = app.composed_plot_rects()[tile];
    let l = &app.chart_doc().composed.plots[tile].layout;
    egui::pos2(
        drawn.left() + (l.plot_x_start() + (l.plot_x_end() - l.plot_x_start()) * fx) as f32,
        drawn.top() + ((l.plot_y_start() + l.plot_y_end()) / 2.0) as f32,
    )
}

/// A settled, scrollable window with a tile brushed, moved and re-checked —
/// one run of the whole tile claim, called once unscrolled and once with the
/// column scrolled past a tile's height, so AC3 is the same assertions at
/// both states rather than two different ones.
fn tile_move_case(scrolled: bool) {
    let (mut app, ctx, raw) = window(GESTURE_SCREEN);
    reopen_the_ledger(&mut app, &ctx, &raw);
    let columns = app
        .canvas_panes()
        .pane("columns")
        .expect("the column pane drew")
        .body;

    let tile = if scrolled {
        scroll_the_column(&mut app, &ctx, &raw, columns.center(), 12);
        let scroll = app.canvas_scroll();
        assert!(
            scroll > MIN_COLUMN_TILE_HEIGHT,
            "fixture check: the column scrolled {scroll} points, less than one tile — the \
             scrolled and unscrolled runs would land on the same tile either way"
        );
        app.composed_plot_rects().len() - 1
    } else {
        assert_eq!(
            app.canvas_scroll(),
            0.0,
            "fixture check: this run is meant to be unscrolled"
        );
        1
    };

    // Draw the rectangle to be moved.
    let a = tile_point(&app, tile, 0.20);
    let b = tile_point(&app, tile, 0.55);
    assert!(
        columns.contains(a) && columns.contains(b),
        "fixture check: tile {tile}'s sweep {a:?}..{b:?} is outside the column pane \
         {columns:?} (scrolled={scrolled})"
    );
    sweep(&mut app, &ctx, &raw, a, b);
    held(&app).expect("fixture check: the sweep committed a selection");

    // AC1's equality, on this tile.
    let rect = committed_window_rect(&app, tile);
    assert!(
        rect.width() > 8.0,
        "fixture check: the committed rectangle {rect:?} has room to press inside of \
         (scrolled={scrolled})"
    );
    let delta = egui::vec2(30.0, 0.0);
    let press = rect.center();
    let to = press + delta;
    assert!(
        columns.contains(press) && columns.contains(to),
        "fixture check: the move {press:?} -> {to:?} must stay inside the column pane \
         {columns:?} (scrolled={scrolled})"
    );
    drag_move(&mut app, &ctx, &raw, rect, delta);
    let moved = held(&app).expect("the move committed a shifted selection");

    let (mut fresh, fctx, fraw) = window(GESTURE_SCREEN);
    reopen_the_ledger(&mut fresh, &fctx, &fraw);
    if scrolled {
        let fcolumns = fresh
            .canvas_panes()
            .pane("columns")
            .expect("the column pane drew")
            .body;
        scroll_the_column(&mut fresh, &fctx, &fraw, fcolumns.center(), 12);
    }
    sweep(&mut fresh, &fctx, &fraw, rect.min + delta, rect.max + delta);
    let fresh_predicate = held(&fresh).expect("the fresh sweep committed a selection");
    assert_same_interval(
        &moved,
        &fresh_predicate,
        &format!("AC3 (scrolled={scrolled}): the moved tile rectangle vs a fresh draw of it"),
    );

    // AC2a, on this tile: no movement leaves it as it was.
    let rect_now = committed_window_rect(&app, tile);
    click_at(&mut app, &ctx, &raw, rect_now.center());
    let unmoved = held(&app).expect("fixture check: a click inside left a selection standing");
    assert_same_interval(
        &unmoved,
        &moved,
        &format!("AC3 (scrolled={scrolled}): a press with no movement changed the selection"),
    );
}

/// **AC3 — the move works on a column tile, at scroll 0 and with the column
/// scrolled**, through the latched origin the map-pane rounds pinned. The
/// press-edge hit test reads the same page-local point `plot_at` already
/// uses to find the tile under the pointer, so it inherits that latch rather
/// than adding a second one — this is what proves it still holds once the
/// column has scrolled the tile away from where it started.
#[test]
fn the_move_gesture_works_on_a_column_tile_at_scroll_zero_and_with_the_column_scrolled() {
    tile_move_case(false);
    tile_move_case(true);
}
