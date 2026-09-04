//! **The point map's equal-aspect frame survives a window resize** — one unit
//! of longitude stays the same length on screen as one unit of latitude after
//! the window is resized, with no reset in between, whether or not the reader
//! had already panned or zoomed the map.
//!
//! `DotRenderer::augment_scales` (`crates/brightfield-render/src/mark.rs`)
//! fits the map's x/y domains to the pane's pixel range so `aspectRatio: 1`
//! holds. That fit is recomputed from the raw column domain each time the
//! plot composes. A resize with **no** navigation in force was already
//! correct before this card, since no other step touches the domain it
//! produces.
//!
//! The break is a **navigated** plot: `infer_multi_mark_scales`
//! (`crates/brightfield-render/src/scene.rs`) applies the analyst's view
//! extent — a pan or a zoom — by overwriting the domain with the fixed values
//! the gesture settled on, at whatever pixel range the plot happens to have
//! at that moment. A resize afterwards changes the pixel range and leaves the
//! domain exactly where the gesture put it, so the two axes' px-per-unit
//! silently drift apart — the map stretches, and nothing re-squares it until
//! the reader presses "Reset view", which throws the navigation away instead
//! of fixing the frame it drew.
//!
//! Driven through the real shell: [`MeridianApp::headless`] over
//! `california_housing_sample.csv`, [`ChartDoc::zoom_view`] for the gesture
//! (a document method — a legitimate gesture entry point, the same standing
//! `tests/navigation_extent.rs`'s module doc gives real key events), and a
//! **new screen size on the next frame** for the resize itself, never a
//! direct call to `ChartDoc::reflow_to`.

use brightfield_render::axis::compute_ticks;
use brightfield_render::channel::Channel;
use brightfield_render::scale::{Scale, ScaleSet};
use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp};

// ---------------------------------------------------------------------------
// Fixture and frame driving
// ---------------------------------------------------------------------------

/// California Housing, sampled — nine numeric columns, two of them a
/// longitude/latitude pair the generator opens as the point-map hero. Shared
/// with `tests/canvas_pane_group.rs`, which documents the shape at length.
fn fixture() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv")
}

/// A live window over the fixture, opened as a data file — the same route a
/// reader takes, so the composed dashboard is a hero beside a column of
/// tiles above a rows pane, exactly as `tests/canvas_pane_group.rs` reads it.
fn open() -> (MeridianApp, egui::Context) {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    (
        MeridianApp::headless(boot, Mode::Light),
        egui::Context::default(),
    )
}

/// Run `frames` frames of `app` at `size`, egui's screen rect — the resize
/// itself: a new screen size on the next frame, the shape
/// `tests/pane_reflow.rs`'s `resizing_the_window_relays_out_the_chart_without_a_restart`
/// drives its own resize through.
fn settle_at(app: &mut MeridianApp, ctx: &egui::Context, size: egui::Vec2, frames: usize) {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        ..Default::default()
    };
    for _ in 0..frames {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
}

const WIDE: egui::Vec2 = egui::vec2(1440.0, 900.0);
const NARROW: egui::Vec2 = egui::vec2(900.0, 620.0);

/// Frames enough for a resizable panel's reported size to be read back on the
/// frame after, the settle count `tests/canvas_pane_group.rs` and
/// `tests/pane_reflow.rs` both use.
const SETTLE: usize = 3;

// ---------------------------------------------------------------------------
// Reading a drawn scale and its grid
// ---------------------------------------------------------------------------

/// A continuous channel's drawn `(domain_min, domain_max, range_start,
/// range_end)`, insisting the channel resolved to [`Scale::Linear`] — the map
/// draws `dot` marks over two quantitative columns, so anything else is a
/// fixture bug rather than a case this test means to cover.
fn linear(scales: &ScaleSet, channel: Channel) -> (f64, f64, f64, f64) {
    match scales.get(channel) {
        Some(Scale::Linear {
            domain_min,
            domain_max,
            range_start,
            range_end,
        }) => (*domain_min, *domain_max, *range_start, *range_end),
        other => panic!("plot 0's {channel:?} channel is not a linear scale: {other:?}"),
    }
}

/// Pixels per one data unit, from a drawn linear scale's own domain/range.
fn px_per_unit((domain_min, domain_max, range_start, range_end): (f64, f64, f64, f64)) -> f64 {
    (range_end - range_start).abs() / (domain_max - domain_min)
}

/// Pixels per one data unit, read off the GRID rather than off the scale's
/// raw fields — [`compute_ticks`] is what `render_x_grid`/`render_y_grid`
/// actually draw from (`crates/brightfield-render/src/grid.rs`), and it picks
/// its own "nice" step independent of the domain's own bounds. Two ticks are
/// enough to measure the spacing; a plot fit to a single point is not this
/// fixture's shape.
fn grid_px_per_unit(scale: &Scale) -> f64 {
    let ticks = compute_ticks(scale, 5);
    assert!(
        ticks.len() >= 2,
        "need at least two ticks to measure the grid's spacing, got {}",
        ticks.len()
    );
    let dv = (ticks[1].value - ticks[0].value).abs();
    let dp = (ticks[1].position - ticks[0].position).abs();
    dp / dv
}

/// One point of tolerance on a px-per-unit comparison: for the SAME one-unit
/// step, the two axes' drawn lengths must not differ by a point.
fn assert_square(px_x: f64, px_y: f64, where_: &str) {
    assert!(
        (px_x - px_y).abs() < 1.0,
        "{where_}: x reads {px_x} px/unit, y reads {px_y} px/unit — {} px apart, \
         not within a point",
        (px_x - px_y).abs()
    );
}

// ---------------------------------------------------------------------------
// AC1 + AC2 — a resize after a zoom, no reset
// ---------------------------------------------------------------------------

#[test]
fn a_navigated_map_keeps_equal_aspect_through_a_resize() {
    let (mut app, ctx) = open();
    settle_at(&mut app, &ctx, WIDE, SETTLE);

    // The gesture: zoom the map in, so a view extent is on record for plot 0
    // — the hero, which `Dashboard::hero_index` picks to lead the plot order
    // with the coordinate pair. This is what makes the bug reproduce: an
    // unnavigated resize was already aspect-correct before this card.
    assert!(
        app.chart_doc_mut().zoom_view(2.0),
        "the zoom gesture settled onto the map"
    );
    settle_at(&mut app, &ctx, WIDE, 2);

    let zoomed = &app.chart_doc().composed.plots[0].scales;
    let zx = linear(zoomed, Channel::X);
    let zy = linear(zoomed, Channel::Y);
    assert_square(
        px_per_unit(zx),
        px_per_unit(zy),
        "right after the zoom, before any resize",
    );

    // The resize — through the shell, a new screen size on the next frame,
    // not a direct call to `ChartDoc::reflow_to`.
    settle_at(&mut app, &ctx, NARROW, SETTLE);

    let plot0 = &app.chart_doc().composed.plots[0];
    let x = linear(&plot0.scales, Channel::X);
    let y = linear(&plot0.scales, Channel::Y);

    // AC1 — the two scales agree on px-per-unit after the resize.
    assert_square(
        px_per_unit(x),
        px_per_unit(y),
        "after the resize that followed the zoom (AC1)",
    );

    // The navigated domain held still rather than being discarded — the
    // governing claim this card must not regress. It widened (per
    // `aspect_fit_domains`), so it is asserted as containment rather than as
    // equality: the analyst's chosen centre and extent are still on-plot.
    let (zx0, zx1, ..) = zx;
    let (zy0, zy1, ..) = zy;
    let (x0, x1, ..) = x;
    let (y0, y1, ..) = y;
    assert!(
        x0 <= zx0 + 1e-6 && x1 >= zx1 - 1e-6,
        "the resize narrowed the navigated x domain: had {zx0}..{zx1}, now {x0}..{x1}"
    );
    assert!(
        y0 <= zy0 + 1e-6 && y1 >= zy1 - 1e-6,
        "the resize narrowed the navigated y domain: had {zy0}..{zy1}, now {y0}..{y1}"
    );

    // AC2 — the grid drawn behind the map agrees with it: `compute_ticks`
    // over the SAME drawn scales gives square cells too, not just the raw
    // domain/range fields.
    let x_scale = plot0.scales.get(Channel::X).expect("x scale drawn");
    let y_scale = plot0.scales.get(Channel::Y).expect("y scale drawn");
    assert_square(
        grid_px_per_unit(x_scale),
        grid_px_per_unit(y_scale),
        "the grid's own tick spacing after the resize (AC2)",
    );

    // And the OTHER dimension too — a fix that only reacted to a narrower
    // window and not a wider one would still leave a real resize broken.
    settle_at(&mut app, &ctx, WIDE, SETTLE);
    let plot0 = &app.chart_doc().composed.plots[0];
    let x = linear(&plot0.scales, Channel::X);
    let y = linear(&plot0.scales, Channel::Y);
    assert_square(
        px_per_unit(x),
        px_per_unit(y),
        "after resizing back up (AC1, the other direction)",
    );
}

// ---------------------------------------------------------------------------
// AC3 — reset is not the route to correctness
// ---------------------------------------------------------------------------

#[test]
fn reset_after_an_unnavigated_resize_changes_nothing() {
    let (mut app, ctx) = open();
    settle_at(&mut app, &ctx, WIDE, SETTLE);

    // No gesture — the resize alone has to be enough.
    settle_at(&mut app, &ctx, NARROW, SETTLE);
    let plot0 = &app.chart_doc().composed.plots[0];
    let before = linear(&plot0.scales, Channel::X);
    let before_y = linear(&plot0.scales, Channel::Y);
    assert_square(
        px_per_unit(before),
        px_per_unit(before_y),
        "the resize alone, with nothing navigated",
    );

    // Reset, with no navigation on record: `ChartDoc::reset_navigation`
    // returns `false` and skips the re-compose when `view_extents()` is
    // empty. That leaves the scales the resize already produced exactly
    // where they were, not merely aspect-correct but numerically the same.
    let applied = app.chart_doc_mut().reset_navigation();
    assert!(
        !applied,
        "reset had nothing to clear, so it must not have re-composed"
    );

    let plot0 = &app.chart_doc().composed.plots[0];
    let after = linear(&plot0.scales, Channel::X);
    let after_y = linear(&plot0.scales, Channel::Y);
    assert_eq!(
        before, after,
        "reset changed the x scale the resize already left correct"
    );
    assert_eq!(
        before_y, after_y,
        "reset changed the y scale the resize already left correct"
    );
}
