//! **A spec that sets `xReverse` or `yReverse` draws that axis from its high end
//! to its low end, and a brush on it reads the values the axis shows.**
//!
//! No code read the two keys, so a plot drew each axis from its lowest value to
//! its highest whatever the spec said, and an analyst who flipped an axis in the
//! chart file saw the data drawn the other way up from what the file says.
//!
//! The expectation is never read off the code under test. The same spec composed
//! without the key says where each dot, tick mark and bar belongs on the default
//! axis, and the reversed one belongs at that position mirrored about the middle
//! of the axis's pixel range. The painted path stream of the reversed scene is
//! then searched for it, and searched for the default position too, so a scene
//! that drew the unflipped picture under a flipped scale fails here and does not
//! pass a check that only read the scale.
//!
//! The brush is a real pointer drag through the whole window, read back as the
//! clause the session holds. A scale that flipped while a brush went on reading
//! the pixels the default way would select the opposite rows, which the paired
//! unreversed drag shows to be the rows the same pointer picks without the key.

use brightfield_engine::SqlPredicate;
use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::{Composed, LiveDashboard};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::analysis::ComponentPath;
use brightfield_sql::ir::ScalarValue;

// ---------------------------------------------------------------------------
// The fixtures
// ---------------------------------------------------------------------------

/// The five dots every drawn arm reads, `(a, b)`. The x values run 12 to 93 and
/// the y values 6 to 100, so neither domain is symmetric about a round number
/// and the default tick marks are not their own mirror image: a plot that
/// ignored the key would draw tick marks in the same places a reversed one
/// does, if they were. No dot sits on its axis's midpoint, so each one moves
/// when the axis does, and the `inset` holds each extreme off the frame, away
/// from the axis line and the gridline ends that would otherwise be painted
/// within a dot's own neighbourhood.
const DOTS: [(f64, f64); 5] = [
    (12.0, 6.0),
    (45.0, 100.0),
    (93.0, 40.0),
    (30.0, 65.0),
    (70.0, 80.0),
];

/// `ATTRS` marks where the plot attributes go, so the arms differ by those
/// lines.
const DOT_PLOT: &str = r"
data:
  pts:
    - { a: 12, b: 6 }
    - { a: 45, b: 100 }
    - { a: 93, b: 40 }
    - { a: 30, b: 65 }
    - { a: 70, b: 80 }
plot:
  - mark: dot
    data: { from: pts }
    x: a
    y: b
width: 600
height: 300
inset: 24
ATTRS
";

/// Four bars on a band x axis, the loads from 40 to 90.
const BARS: &str = r"
data:
  pts:
    - { site: Alder,   load: 40 }
    - { site: Birch,   load: 55 }
    - { site: Cedar,   load: 70 }
    - { site: Dogwood, load: 90 }
plot:
  - mark: barY
    data: { from: pts }
    x: site
    y: load
width: 600
height: 300
ATTRS
";

/// Eleven rows from 0 to 100 and a brush on x over them. `AXIS` is the brush
/// kind (`intervalX` or `intervalY`) and `ATTRS` the plot attributes.
const BRUSHED: &str = r"
params:
  brush: { select: crossfilter }
data:
  pts:
    - { a: 0,   b: 0 }
    - { a: 10,  b: 10 }
    - { a: 20,  b: 20 }
    - { a: 30,  b: 30 }
    - { a: 40,  b: 40 }
    - { a: 50,  b: 50 }
    - { a: 60,  b: 60 }
    - { a: 70,  b: 70 }
    - { a: 80,  b: 80 }
    - { a: 90,  b: 90 }
    - { a: 100, b: 100 }
plot:
  - mark: dot
    data: { from: pts }
    x: a
    y: b
  - select: AXIS
    as: $brush
width: 600
height: 360
ATTRS
";

fn spec(template: &str, attrs: &str) -> String {
    template.replace("ATTRS", attrs)
}

fn compose_from(template: &str, attrs: &str) -> Composed {
    LiveDashboard::load_str(&spec(template, attrs), None)
        .expect("the spec loads")
        .present()
        .expect("the spec composes")
}

// ---------------------------------------------------------------------------
// Reading the picture
// ---------------------------------------------------------------------------

/// The coordinate pairs the scene's path stream holds, in draw order, in the
/// plot's own coordinates (`Encoding::path_data` is a flat run of `f32` bits,
/// two words per point).
fn scene_points(composed: &Composed) -> Vec<(f64, f64)> {
    composed
        .scene
        .encoding()
        .path_data
        .chunks_exact(2)
        .map(|pair| {
            (
                f64::from(f32::from_bits(pair[0])),
                f64::from(f32::from_bits(pair[1])),
            )
        })
        .collect()
}

fn scale(composed: &Composed, channel: Channel) -> &Scale {
    composed.plots[0]
        .scales
        .get(channel)
        .expect("the plot drew this channel")
}

/// Where `value` lands on the first plot's `channel` axis as it is drawn.
fn pixel(composed: &Composed, channel: Channel, value: f64) -> f64 {
    scale(composed, channel).map_f64(value)
}

/// `px` reflected about the middle of the first plot's `channel` pixel range:
/// where a value drawn at `px` on the default axis is drawn once the axis runs
/// the other way. Read from the DEFAULT composition's scale, never the reversed
/// one, so the expectation does not borrow from what is being checked.
fn mirrored(default: &Composed, channel: Channel, px: f64) -> f64 {
    let scale = scale(default, channel);
    scale.range_start() + scale.range_end() - px
}

/// The mean of the path points within twelve pixels of `near`: a dot's outline
/// sits around its centre, and the gridline and axis points the plot paints sit
/// at the frame, further than that from any dot here. `None` when nothing is
/// painted that near.
fn painted_dot(composed: &Composed, near: (f64, f64)) -> Option<(f64, f64)> {
    let hits: Vec<(f64, f64)> = scene_points(composed)
        .into_iter()
        .filter(|p| (p.0 - near.0).hypot(p.1 - near.1) < 12.0)
        .collect();
    if hits.is_empty() {
        return None;
    }
    let n = hits.len() as f64;
    Some((
        hits.iter().map(|p| p.0).sum::<f64>() / n,
        hits.iter().map(|p| p.1).sum::<f64>() / n,
    ))
}

/// Where each of [`DOTS`] is painted when the plot is composed from `attrs`, by
/// searching the path stream around `anchor(a, b)`. A dot with nothing painted
/// near its anchor is `None`.
fn painted_dots(
    composed: &Composed,
    anchor: impl Fn(f64, f64) -> (f64, f64),
) -> Vec<Option<(f64, f64)>> {
    DOTS.iter()
        .map(|&(a, b)| painted_dot(composed, anchor(a, b)))
        .collect()
}

/// The path points the plot draws inside its tile: the scene opens with two
/// rectangles the size of the tile (the background and the clip), and their
/// corners are no tick mark and no rule.
fn drawn_points(composed: &Composed) -> Vec<(f64, f64)> {
    let (width, height) = (f64::from(composed.width), f64::from(composed.height));
    scene_points(composed)
        .into_iter()
        .filter(|p| p.0 > 0.0 && p.1 > 0.0 && p.0 < width && p.1 < height)
        .collect()
}

/// The x of each x-axis tick mark: the drawn paths' lowest points. A tick mark
/// runs down from the axis line, and nothing else this plot draws reaches
/// further down (the gridlines stop at the axis line), so the points at the
/// greatest y are the tick marks' ends.
fn bottom_tick_xs(composed: &Composed) -> Vec<f64> {
    let points = drawn_points(composed);
    let lowest = points.iter().map(|p| p.1).fold(f64::MIN, f64::max);
    sorted_unique(points.iter().filter(|p| p.1 >= lowest - 1e-3).map(|p| p.0))
}

/// The y of each horizontal rule at the left edge of the frame: the drawn
/// paths' leftmost points, by the same reasoning turned a quarter. A y
/// gridline and a y tick mark both stand at a tick's position, and the dots,
/// held off the frame by the plot's `inset`, are further right.
fn left_tick_ys(composed: &Composed) -> Vec<f64> {
    let points = drawn_points(composed);
    let leftmost = points.iter().map(|p| p.0).fold(f64::MAX, f64::min);
    sorted_unique(
        points
            .iter()
            .filter(|p| p.0 <= leftmost + 1e-3)
            .map(|p| p.1),
    )
}

fn sorted_unique(values: impl Iterator<Item = f64>) -> Vec<f64> {
    let mut all: Vec<f64> = values.collect();
    all.sort_by(|a, b| a.partial_cmp(b).expect("finite coordinates"));
    all.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    all
}

/// Whether two sorted coordinate lists hold the same marks to within a
/// hundredth of a pixel.
fn same_marks(left: &[f64], right: &[f64]) -> bool {
    left.len() == right.len() && left.iter().zip(right).all(|(a, b)| (a - b).abs() < 0.05)
}

/// `values` reflected about the middle of `default`'s `channel` range, in
/// ascending order.
fn mirrored_all(default: &Composed, channel: Channel, values: &[f64]) -> Vec<f64> {
    sorted_unique(values.iter().map(|&v| mirrored(default, channel, v)))
}

// ---------------------------------------------------------------------------
// AC1 — yReverse
// ---------------------------------------------------------------------------

/// **With `yReverse: true` the y axis draws its lowest value at the top, and
/// the point with the largest y value is the lowest on the screen.** Each dot
/// is painted where the default axis's mirror puts it and nowhere near where the
/// default axis put it; the y axis's tick marks are mirrored with them; the x
/// axis, which the key does not name, is drawn as it was.
#[test]
fn y_reverse_draws_the_largest_y_value_lowest_on_the_screen() {
    let asked = compose_from(DOT_PLOT, "yReverse: true");
    let default = compose_from(DOT_PLOT, "");

    // The default axis, as the fixture check: the largest y value is the
    // highest dot on the screen, which is what the key turns over.
    let default_dots = painted_dots(&default, |a, b| {
        (
            pixel(&default, Channel::X, a),
            pixel(&default, Channel::Y, b),
        )
    });
    let default_dots: Vec<(f64, f64)> = default_dots
        .into_iter()
        .map(|d| d.expect("fixture check: every dot is painted on the default axis"))
        .collect();
    let highest = default_dots.iter().map(|d| d.1).fold(f64::MAX, f64::min);
    assert!(
        (default_dots[1].1 - highest).abs() < 1e-6,
        "fixture check: with no key the dot with the largest y value is the highest on the screen"
    );

    // Painted: each dot sits at its mirrored y, and nothing is left behind at
    // the default one.
    let reversed = painted_dots(&asked, |a, b| {
        (
            pixel(&default, Channel::X, a),
            mirrored(&default, Channel::Y, pixel(&default, Channel::Y, b)),
        )
    });
    for ((&(a, b), found), default_at) in DOTS.iter().zip(&reversed).zip(&default_dots) {
        let expected = (default_at.0, mirrored(&default, Channel::Y, default_at.1));
        assert!(
            (expected.1 - default_at.1).abs() > 30.0,
            "fixture check: the dot at ({a}, {b}) moves by more than the search radius"
        );
        let found = found.unwrap_or_else(|| {
            panic!("no dot is painted where a reversed y axis puts ({a}, {b}): {expected:?}")
        });
        assert!(
            (found.1 - expected.1).abs() < 1.5 && (found.0 - expected.0).abs() < 1.5,
            "the dot at ({a}, {b}) is painted at {found:?}; a reversed y axis puts it at {expected:?}"
        );
        assert!(
            painted_dot(&asked, *default_at).is_none(),
            "a dot is still painted where the default axis put ({a}, {b}): {default_at:?}"
        );
    }

    // The largest y value is the lowest dot on the screen.
    let lowest = reversed
        .iter()
        .map(|d| d.expect("every dot is painted").1)
        .fold(f64::MIN, f64::max);
    assert!(
        (reversed[1].expect("painted").1 - lowest).abs() < 1.5,
        "the point with the largest y value is the lowest on the screen"
    );

    // The axis itself: the drawn scale runs from the bottom edge to the top
    // with its domain as it was, and the tick marks are the default's, mirrored.
    assert!(
        scale(&asked, Channel::Y).range_start() < scale(&asked, Channel::Y).range_end(),
        "the y scale runs from the top of the data area to the bottom"
    );
    assert_eq!(
        (
            scale(&asked, Channel::Y).domain_min(),
            scale(&asked, Channel::Y).domain_max()
        ),
        (
            scale(&default, Channel::Y).domain_min(),
            scale(&default, Channel::Y).domain_max()
        ),
        "reversing an axis moves where its values are drawn, not which values it covers"
    );
    let default_ticks = left_tick_ys(&default);
    assert!(
        default_ticks.len() >= 3
            && !same_marks(
                &default_ticks,
                &mirrored_all(&default, Channel::Y, &default_ticks)
            ),
        "fixture check: the default y tick marks are not their own mirror image, so a \
         plot that ignored the key would not pass the comparison below ({default_ticks:?})"
    );
    assert!(
        same_marks(
            &left_tick_ys(&asked),
            &mirrored_all(&default, Channel::Y, &default_ticks)
        ),
        "the y tick marks are the default's, mirrored: {:?} against {:?}",
        left_tick_ys(&asked),
        mirrored_all(&default, Channel::Y, &default_ticks)
    );

    // The key names y: the x axis is drawn as it was.
    assert!(
        same_marks(&bottom_tick_xs(&asked), &bottom_tick_xs(&default)),
        "`yReverse` names the y axis; the x tick marks are where they were"
    );
}

// ---------------------------------------------------------------------------
// AC2 — xReverse
// ---------------------------------------------------------------------------

/// **With `xReverse: true` the x axis draws its lowest value at the right.**
/// Each dot is painted at the default axis's mirror and not at its default
/// position, the x tick marks are mirrored with them, and the y axis, which the
/// key does not name, is drawn as it was.
#[test]
fn x_reverse_draws_the_lowest_x_value_at_the_right() {
    let asked = compose_from(DOT_PLOT, "xReverse: true");
    let default = compose_from(DOT_PLOT, "");

    let default_dots: Vec<(f64, f64)> = painted_dots(&default, |a, b| {
        (
            pixel(&default, Channel::X, a),
            pixel(&default, Channel::Y, b),
        )
    })
    .into_iter()
    .map(|d| d.expect("fixture check: every dot is painted on the default axis"))
    .collect();
    let leftmost = default_dots.iter().map(|d| d.0).fold(f64::MAX, f64::min);
    assert!(
        (default_dots[0].0 - leftmost).abs() < 1e-6,
        "fixture check: with no key the dot with the lowest x value is the leftmost"
    );

    let reversed = painted_dots(&asked, |a, b| {
        (
            mirrored(&default, Channel::X, pixel(&default, Channel::X, a)),
            pixel(&default, Channel::Y, b),
        )
    });
    for ((&(a, b), found), default_at) in DOTS.iter().zip(&reversed).zip(&default_dots) {
        let expected = (mirrored(&default, Channel::X, default_at.0), default_at.1);
        assert!(
            (expected.0 - default_at.0).abs() > 30.0,
            "fixture check: the dot at ({a}, {b}) moves by more than the search radius"
        );
        let found = found.unwrap_or_else(|| {
            panic!("no dot is painted where a reversed x axis puts ({a}, {b}): {expected:?}")
        });
        assert!(
            (found.0 - expected.0).abs() < 1.5 && (found.1 - expected.1).abs() < 1.5,
            "the dot at ({a}, {b}) is painted at {found:?}; a reversed x axis puts it at {expected:?}"
        );
        assert!(
            painted_dot(&asked, *default_at).is_none(),
            "a dot is still painted where the default axis put ({a}, {b}): {default_at:?}"
        );
    }

    // The lowest x value is the rightmost dot on the screen.
    let rightmost = reversed
        .iter()
        .map(|d| d.expect("every dot is painted").0)
        .fold(f64::MIN, f64::max);
    assert!(
        (reversed[0].expect("painted").0 - rightmost).abs() < 1.5,
        "the point with the lowest x value is the rightmost on the screen"
    );

    assert!(
        scale(&asked, Channel::X).range_start() > scale(&asked, Channel::X).range_end(),
        "the x scale runs from the right of the data area to the left"
    );
    let default_ticks = bottom_tick_xs(&default);
    assert!(
        default_ticks.len() >= 3
            && !same_marks(
                &default_ticks,
                &mirrored_all(&default, Channel::X, &default_ticks)
            ),
        "fixture check: the default x tick marks are not their own mirror image, so a \
         plot that ignored the key would not pass the comparison below ({default_ticks:?})"
    );
    assert!(
        same_marks(
            &bottom_tick_xs(&asked),
            &mirrored_all(&default, Channel::X, &default_ticks)
        ),
        "the x tick marks are the default's, mirrored: {:?} against {:?}",
        bottom_tick_xs(&asked),
        mirrored_all(&default, Channel::X, &default_ticks)
    );

    assert!(
        same_marks(&left_tick_ys(&asked), &left_tick_ys(&default)),
        "`xReverse` names the x axis; the y tick marks are where they were"
    );
}

/// **A band axis reverses too: the first category takes the far end, and each
/// bar is painted at its category's mirrored slot, its corners at the height it
/// had.** A bar is a rectangle whose two band edges come out of the band scale's
/// range as a signed pair, and the y band already runs that way by default, so
/// this holds the x band to the same shape.
#[test]
fn x_reverse_runs_a_band_axis_from_the_last_category_to_the_first() {
    let asked = compose_from(BARS, "xReverse: true");
    let default = compose_from(BARS, "");
    let sites = ["Alder", "Birch", "Cedar", "Dogwood"];
    let loads = [40.0, 55.0, 70.0, 90.0];

    let centre = |composed: &Composed, site: &str| {
        scale(composed, Channel::X)
            .map_category(site)
            .expect("the band scale places every site")
    };
    assert!(
        centre(&default, "Alder") < centre(&default, "Dogwood"),
        "fixture check: with no key the first category is leftmost"
    );
    assert!(
        centre(&asked, "Alder") > centre(&asked, "Dogwood"),
        "with the key the first category is rightmost"
    );

    let half = scale(&default, Channel::X)
        .band_width()
        .expect("a band scale has a width")
        .abs()
        / 2.0;
    let points = scene_points(&asked);
    let default_points = scene_points(&default);
    let near = |points: &[(f64, f64)], at: (f64, f64)| {
        points
            .iter()
            .any(|p| (p.0 - at.0).abs() < 0.6 && (p.1 - at.1).abs() < 0.6)
    };
    for (site, load) in sites.iter().zip(loads) {
        let tip = pixel(&default, Channel::Y, load);
        let default_centre = centre(&default, site);
        let reversed_centre = mirrored(&default, Channel::X, default_centre);
        for edge in [-half, half] {
            assert!(
                near(&default_points, (default_centre + edge, tip)),
                "fixture check: {site}'s bar has a corner at its default slot"
            );
            assert!(
                near(&points, (reversed_centre + edge, tip)),
                "{site}'s bar is painted at the mirrored slot: no corner at ({}, {tip})",
                reversed_centre + edge
            );
            assert!(
                !near(&points, (default_centre + edge, tip)),
                "{site}'s bar is not still painted at its default slot: a corner is at ({}, {tip})",
                default_centre + edge
            );
        }
    }
}

// ---------------------------------------------------------------------------
// AC4 — a spec that sets neither key
// ---------------------------------------------------------------------------

/// **A spec that sets neither key draws what it drew before the keys were
/// read, and `false` asks for the same.** The path stream is identical, point
/// for point, and the scales run as the layout puts them: x from the left edge
/// to the right and y from the bottom edge to the top.
#[test]
fn a_spec_that_sets_neither_key_draws_what_it_drew() {
    let default = compose_from(DOT_PLOT, "");
    let spelled_out = compose_from(DOT_PLOT, "xReverse: false\nyReverse: false");
    assert_eq!(
        scene_points(&default),
        scene_points(&spelled_out),
        "`false` on each key draws the plot an unset one draws"
    );
    assert!(
        scale(&default, Channel::X).range_start() < scale(&default, Channel::X).range_end(),
        "the default x axis runs from the left edge to the right"
    );
    assert!(
        scale(&default, Channel::Y).range_start() > scale(&default, Channel::Y).range_end(),
        "the default y axis runs from the bottom edge to the top"
    );

    // And the keys do move the picture, so the equality above compares two
    // pictures that could have differed.
    let flipped = compose_from(DOT_PLOT, "xReverse: true\nyReverse: true");
    assert_ne!(
        scene_points(&default),
        scene_points(&flipped),
        "fixture check: reversing both axes changes the picture"
    );

    // A value that is no switch reverses nothing and is named in the load's
    // diagnostics.
    let typo = compose_from(DOT_PLOT, "yReverse: 'yes'");
    assert_eq!(
        scene_points(&default),
        scene_points(&typo),
        "a key that is no switch is ignored"
    );
    assert!(
        typo.diagnostics
            .lines()
            .iter()
            .any(|line| line.contains("yReverse")),
        "the dropped key is named: {:?}",
        typo.diagnostics.lines()
    );
}

// ---------------------------------------------------------------------------
// AC3 — a brush on a reversed axis
// ---------------------------------------------------------------------------

const SCREEN: egui::Vec2 = egui::vec2(1280.0, 820.0);

fn frame(app: &mut MeridianApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
        events,
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| app.draw(ui));
}

/// The whole window over `BRUSHED` with `axis` as its brush and `attrs` on the
/// plot, live, with three settled frames drawn so the plot has reflowed into
/// its pane.
fn window(axis: &str, attrs: &str, ctx: &egui::Context) -> MeridianApp {
    let source = spec(&BRUSHED.replace("AXIS", axis), attrs);
    let mut live = LiveDashboard::load_str(&source, None).expect("the spec loads live");
    let composed = live.present().expect("the spec composes");
    let mut boot = Boot::charts(composed);
    boot.live = Some(live);
    let mut app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
    for _ in 0..3 {
        frame(&mut app, ctx, Vec::new());
    }
    app
}

/// A window position on the first plot, at plot-local pixel `local`.
fn window_pos(app: &MeridianApp, local: (f64, f64)) -> egui::Pos2 {
    let raster = app
        .chart_doc()
        .raster_rect
        .expect("a settled frame presented the raster");
    let rect = app.chart_doc().composed.plots[0].rect;
    egui::pos2(
        raster.min.x + (rect.x + local.0) as f32,
        raster.min.y + (rect.y + local.1) as f32,
    )
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// A real drag, pressed at `from` and released at `to` (plot-local pixels).
fn drag(app: &mut MeridianApp, ctx: &egui::Context, from: (f64, f64), to: (f64, f64)) {
    let start = window_pos(app, from);
    frame(
        app,
        ctx,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    let end = window_pos(app, to);
    frame(app, ctx, vec![egui::Event::PointerMoved(end)]);
    frame(app, ctx, vec![button(end, false)]);
    frame(app, ctx, Vec::new());
}

/// The rows (their values, which are the same in both columns) the brush the
/// session holds selects: the fixture's values inside the clause's bounds.
fn selected_rows(app: &MeridianApp, column: &str) -> Vec<i64> {
    let plot = &app.chart_doc().composed.plots[0];
    let held = app
        .chart_doc()
        .held_clause("brush", &ComponentPath(plot.path.clone()))
        .expect("the drag committed a clause");
    let SqlPredicate::Interval {
        column: held_column,
        lo: ScalarValue::Float(lo),
        hi: ScalarValue::Float(hi),
        ..
    } = held
    else {
        panic!("an interval brush holds an interval over floats, got {held:?}");
    };
    assert!(
        held_column.contains(column),
        "the clause is over `{column}`, got {held_column}"
    );
    assert!(lo <= hi, "the clause's bounds are ordered: {lo} .. {hi}");
    (0..=10)
        .map(|i| i64::from(i) * 10)
        .filter(|&v| lo <= v as f64 && v as f64 <= hi)
        .collect()
}

/// The first plot's `channel` pixel range as `(low, high)` — the data area's
/// edges, whichever way the scale runs between them.
fn data_area(app: &MeridianApp, channel: Channel) -> (f64, f64) {
    let scale = app.chart_doc().composed.plots[0]
        .scales
        .get(channel)
        .expect("the plot drew this channel");
    let (a, b) = (scale.range_start(), scale.range_end());
    (a.min(b), a.max(b))
}

/// **On a reversed x axis, a brush over the left third of the data area selects
/// the rows in the highest third of the x range.** The same drag with no key
/// selects the lowest third, so the rows are the axis's doing and not the
/// fixture's. The drag runs from 1% to 33% of the data area's width, a hair
/// inside both edges, so the edge row is not on a bound.
#[test]
fn a_brush_over_the_left_third_of_a_reversed_x_axis_selects_the_highest_third() {
    let ctx = egui::Context::default();
    let sweep = |attrs: &str| {
        let mut app = window("intervalX", attrs, &ctx);
        let (left, right) = data_area(&app, Channel::X);
        let width = right - left;
        let y = app.chart_doc().composed.plots[0].rect.height / 2.0;
        drag(
            &mut app,
            &ctx,
            (left + 0.01 * width, y),
            (left + 0.33 * width, y),
        );
        let still_reversed = {
            let scale = app.chart_doc().composed.plots[0]
                .scales
                .get(Channel::X)
                .expect("the x scale");
            scale.range_start() > scale.range_end()
        };
        (selected_rows(&app, "a"), still_reversed)
    };

    let (default_rows, default_reversed) = sweep("");
    assert!(
        !default_reversed,
        "fixture check: with no key the x axis runs left to right"
    );
    assert_eq!(
        default_rows,
        vec![10, 20, 30],
        "fixture check: with no key the left third of the data area is the lowest third of x"
    );

    let (rows, still_reversed) = sweep("xReverse: true");
    assert_eq!(
        rows,
        vec![70, 80, 90],
        "on a reversed x axis the left third of the data area is the highest third of x"
    );
    assert!(
        still_reversed,
        "the axis is still reversed once the brush has committed and the plot has been drawn again"
    );
}

/// **A brush on a reversed y axis reads the flipped values too.** The top
/// third of the data area is the lowest third of y, the rows the bottom third
/// holds with no key.
#[test]
fn a_brush_over_the_top_third_of_a_reversed_y_axis_selects_the_lowest_third() {
    let ctx = egui::Context::default();
    let sweep = |attrs: &str| {
        let mut app = window("intervalY", attrs, &ctx);
        let (top, bottom) = data_area(&app, Channel::Y);
        let height = bottom - top;
        let x = app.chart_doc().composed.plots[0].rect.width / 2.0;
        drag(
            &mut app,
            &ctx,
            (x, top + 0.01 * height),
            (x, top + 0.33 * height),
        );
        selected_rows(&app, "b")
    };

    assert_eq!(
        sweep(""),
        vec![70, 80, 90],
        "fixture check: with no key the top third of the data area is the highest third of y"
    );
    assert_eq!(
        sweep("yReverse: true"),
        vec![10, 20, 30],
        "on a reversed y axis the top third of the data area is the lowest third of y"
    );
}
