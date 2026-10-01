//! **A spec that sets `grid`, `xGrid` or `yGrid` draws the gridlines it asked
//! for.**
//!
//! No code read any of the three keys, so a plot drew a rule at each tick of
//! both axes whatever the spec said, and an analyst who turned them off in the
//! chart file saw them anyway, with no word that the setting was dropped.
//!
//! Assertions read the rules that were PAINTED. A spec is composed once with
//! `grid: false`, which draws none, and once as asked; the points the second
//! scene's path stream holds beyond the first are the rules, because the axis
//! lines, the tick marks and the mark are in both. That run is compared with
//! the rules the spec should paint — one at each tick of an axis that draws,
//! from one edge of the data area to the other — stroked through the same
//! encoder, so the comparison does not assume how many points the encoder
//! spends on a line. A switch resolved correctly and dropped before the draw
//! would pass a check that read the resolver, and a rule at the wrong value or
//! short of the data area would pass one that only counted them.
//!
//! The axes are given different tick counts — three x ticks and eleven y ticks
//! on a 0 to 100 domain — so a switch wired to the other axis paints the wrong
//! number of rules. An asked-for arm is paired with the other axis left alone
//! and with the same spec asking for nothing, since a fixture whose default is
//! the setting asked for would pass without the key being read.

use brightfield_render::axis::compute_ticks;
use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_render::text::LABEL_SIZE;
use brightfield_shell::pipeline::{Composed, LiveDashboard};

// ---------------------------------------------------------------------------
// The fixture
// ---------------------------------------------------------------------------

/// Three points on a 0 to 100 domain on both axes. `ATTRS` marks where the plot
/// attributes go, so the arms differ by those lines. The tick counts are fixed
/// here: two on x gives ticks at 0, 50 and 100, and ten on y gives the tens.
const TEMPLATE: &str = r"
data:
  pts:
    - { a: 0,   b: 0 }
    - { a: 30,  b: 70 }
    - { a: 100, b: 100 }
plot:
  - mark: dot
    data: { from: pts }
    x: a
    y: b
width: 600
height: 300
xTicks: 2
yTicks: 10
ATTRS
";

const X_TICKS: usize = 3;
const Y_TICKS: usize = 11;

fn compose(attrs: &str) -> Composed {
    LiveDashboard::load_str(&TEMPLATE.replace("ATTRS", attrs), None)
        .expect("the spec loads")
        .present()
        .expect("the spec composes")
}

/// What the load said, one line per diagnostic: what the warning banner draws.
fn said(composed: &Composed) -> Vec<String> {
    composed.diagnostics.lines()
}

// ---------------------------------------------------------------------------
// Reading the painted rules
// ---------------------------------------------------------------------------

/// A rule's two endpoints, in the plot's own coordinates.
type Rule = [(f64, f64); 2];

/// Every coordinate pair the scene's path stream holds, in draw order.
///
/// `Encoding::path_data` is a flat run of `f32` bits, two words per point, and
/// each plot's scene is appended to the dashboard's with its placement in the
/// transform stream, so these are the plot's own coordinates.
fn scene_points(scene: &vello::Scene) -> Vec<(f64, f64)> {
    scene
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

fn points(composed: &Composed) -> Vec<(f64, f64)> {
    scene_points(&composed.scene)
}

/// The rules a spec paints: the run of points its scene holds beyond the scene
/// of the same plot with `grid: false`, and where in the stream that run sits.
struct Painted {
    /// The whole point stream of the spec's scene.
    stream: Vec<(f64, f64)>,
    /// Index in `stream` of the first point of the run.
    start: usize,
    /// The points the spec's scene holds beyond the bare one.
    run: Vec<(f64, f64)>,
}

impl Painted {
    /// Index in `stream` one past the last point of the run.
    fn end(&self) -> usize {
        self.start + self.run.len()
    }
}

fn painted(attrs: &str) -> Painted {
    let bare = points(&compose("grid: false"));
    let stream = points(&compose(attrs));
    let added = stream
        .len()
        .checked_sub(bare.len())
        .expect("a spec paints at least the points of the same plot with no gridlines");
    // The earliest start after which the rest of the scene is what it is with no
    // gridlines. The common prefix alone can overshoot, when a rule's first point
    // is also the next point of the scene without it.
    let common = stream.iter().zip(&bare).take_while(|(a, b)| a == b).count();
    let start = (0..=common)
        .find(|&at| stream[at + added..] == bare[at..])
        .unwrap_or_else(|| {
            panic!(
                "`{attrs}`: what the gridline switches add is not one run of points with the \
                 rest of the scene unchanged"
            )
        });
    let run = stream[start..start + added].to_vec();
    Painted { stream, start, run }
}

/// The first plot's data area, in the plot's own coordinates:
/// `(left, right, top, bottom)`.
fn data_area(composed: &Composed) -> (f64, f64, f64, f64) {
    let layout = &composed.plots[0].layout;
    (
        layout.plot_x_start(),
        layout.plot_x_end(),
        layout.plot_y_start(),
        layout.plot_y_end(),
    )
}

fn scale(composed: &Composed, channel: Channel) -> &Scale {
    composed.plots[0]
        .scales
        .get(channel)
        .expect("the plot drew this channel")
}

/// The vertical rules the plot should paint: one at each x tick, from the top
/// of the data area to its bottom.
fn x_rules(composed: &Composed) -> Vec<Rule> {
    let (_, _, top, bottom) = data_area(composed);
    let ticks = compute_ticks(scale(composed, Channel::X), 2);
    assert_eq!(ticks.len(), X_TICKS, "fixture check: the x ticks");
    ticks
        .iter()
        .map(|tick| [(tick.position, top), (tick.position, bottom)])
        .collect()
}

/// The horizontal rules the plot should paint: one at each y tick, from the
/// left of the data area to its right.
fn y_rules(composed: &Composed) -> Vec<Rule> {
    let (left, right, _, _) = data_area(composed);
    let ticks = compute_ticks(scale(composed, Channel::Y), 10);
    assert_eq!(ticks.len(), Y_TICKS, "fixture check: the y ticks");
    ticks
        .iter()
        .map(|tick| [(left, tick.position), (right, tick.position)])
        .collect()
}

/// The points the encoder holds for `rules`, stroked one after another as the
/// grid strokes them.
fn encoded(rules: &[Rule]) -> Vec<(f64, f64)> {
    let mut scene = vello::Scene::new();
    for rule in rules {
        scene.stroke(
            &kurbo::Stroke::new(0.5),
            kurbo::Affine::IDENTITY,
            peniko::Color::BLACK,
            None,
            &kurbo::Line::new(rule[0], rule[1]),
        );
    }
    scene_points(&scene)
}

/// How many points the encoder spends on one stroked line, measured on a lone
/// one rather than assumed.
fn points_per_rule() -> usize {
    let n = encoded(&[[(0.0, 0.0), (10.0, 0.0)]]).len();
    assert!(n >= 2, "fixture check: a stroked line has points");
    n
}

/// A run of points read a rule at a time, each point rounded to a thousandth of
/// a pixel, the rules sorted: two runs compare as sets of rules, not as the
/// order the x and y sets were drawn in.
fn as_rules(run: &[(f64, f64)]) -> Vec<Vec<(i64, i64)>> {
    let per_rule = points_per_rule();
    assert_eq!(
        run.len() % per_rule,
        0,
        "a run of {} points is not a whole number of rules at {per_rule} points a rule",
        run.len()
    );
    let mut rules: Vec<Vec<(i64, i64)>> = run
        .chunks(per_rule)
        .map(|rule| {
            rule.iter()
                .map(|p| ((p.0 * 1000.0).round() as i64, (p.1 * 1000.0).round() as i64))
                .collect()
        })
        .collect();
    rules.sort();
    rules
}

/// Assert that `attrs` paints exactly the rules of the axes named: one at each
/// tick of an axis that draws, none on one that does not, each running the data
/// area at its tick.
fn assert_paints(attrs: &str, x: bool, y: bool) {
    let composed = compose(attrs);
    let mut expected: Vec<Rule> = Vec::new();
    if x {
        expected.extend(x_rules(&composed));
    }
    if y {
        expected.extend(y_rules(&composed));
    }
    let axes = match (x, y) {
        (true, true) => "both axes",
        (true, false) => "the x axis alone",
        (false, true) => "the y axis alone",
        (false, false) => "neither axis",
    };
    assert_eq!(
        as_rules(&painted(attrs).run),
        as_rules(&encoded(&expected)),
        "`{attrs}` should paint the rules of {axes}: one at each tick of an axis that draws, \
         none on one that does not, each running the data area at its tick"
    );
}

// ---------------------------------------------------------------------------
// AC1 and AC2 — one axis
// ---------------------------------------------------------------------------

/// **`yGrid: true` draws one horizontal rule at each y tick across the data
/// area, and `yGrid: false` draws none.** The x axis names no key in either
/// arm, so its vertical rules are what an unset key draws.
#[test]
fn y_grid_draws_a_horizontal_rule_at_each_y_tick_or_none() {
    assert_paints("yGrid: true", true, true);
    assert_paints("yGrid: false", true, false);
}

/// **`xGrid: true` draws one vertical rule at each x tick, and `xGrid: false`
/// draws none.** The y axis names no key in either arm.
#[test]
fn x_grid_draws_a_vertical_rule_at_each_x_tick_or_none() {
    assert_paints("xGrid: true", true, true);
    assert_paints("xGrid: false", false, true);
}

// ---------------------------------------------------------------------------
// AC3 — the bare key, and the axis key that outranks it
// ---------------------------------------------------------------------------

/// **`grid: true` draws both sets, and with `xGrid: false` the horizontal rules
/// draw and the vertical ones do not: the key that names an axis wins.** The
/// mirror arms turn one axis on under a bare `grid: false`, and `grid: false`
/// alone draws neither.
#[test]
fn the_bare_grid_draws_both_sets_and_the_key_that_names_an_axis_wins() {
    assert_paints("grid: true", true, true);
    assert_paints("grid: true\nxGrid: false", false, true);
    assert_paints("grid: true\nyGrid: false", true, false);
    assert_paints("grid: false\nxGrid: true", true, false);
    assert_paints("grid: false\nyGrid: true", false, true);
    assert_paints("grid: false", false, false);
}

// ---------------------------------------------------------------------------
// AC4 — under the marks, inside the data area, clear of the labels
// ---------------------------------------------------------------------------

/// The horizontal, label-sized glyph runs' anchors as `(x, baseline_y)`, in the
/// dashboard's coordinates. The axis titles are a different size and the y
/// title is rotated, so neither is in this list.
fn label_anchors(composed: &Composed) -> Vec<(f64, f64)> {
    composed
        .scene
        .encoding()
        .resources
        .glyph_runs
        .iter()
        .filter(|run| {
            (run.font_size - LABEL_SIZE).abs() < 0.01 && run.transform.matrix[0].abs() > 0.5
        })
        .map(|run| {
            (
                f64::from(run.transform.translation[0]),
                f64::from(run.transform.translation[1]),
            )
        })
        .collect()
}

/// **The rules sit inside the data area, under the marks, and clear of the axis
/// labels.** The rules' ends are the data area's edges, which
/// [`assert_paints`] holds; the tick labels are read off the same scene and none
/// of them is inside that area; and the rules are drawn before the dot, so the
/// dot is on top of them.
#[test]
fn the_rules_are_under_the_marks_and_inside_the_data_area_clear_of_the_labels() {
    assert_paints("grid: true", true, true);

    let composed = compose("grid: true");
    let both = painted("grid: true");
    assert!(
        both.run.len() >= (X_TICKS + Y_TICKS) * 2,
        "fixture check: the rules are in the scene; the run holds {} points",
        both.run.len()
    );

    let (left, right, top, bottom) = data_area(&composed);
    let plot = &composed.plots[0];
    let (origin_x, origin_y) = (plot.rect.x, plot.rect.y);
    let labels = label_anchors(&composed);
    assert!(
        labels.len() >= X_TICKS + Y_TICKS,
        "fixture check: the tick labels are in the scene; got {labels:?}"
    );
    for (x, y) in labels {
        let inside = x > origin_x + left
            && x < origin_x + right
            && y > origin_y + top
            && y < origin_y + bottom;
        assert!(
            !inside,
            "a tick label at ({x}, {y}) is inside the data area, where a rule would cross it"
        );
    }

    // The dot at (30, 70), placed by the scales the plot was composed against.
    let place = |channel: Channel, value: f64| match scale(&composed, channel) {
        scale @ Scale::Linear {
            domain_min,
            domain_max,
            ..
        } => {
            let t = (value - domain_min) / (domain_max - domain_min);
            scale.range_start() + t * (scale.range_end() - scale.range_start())
        }
        other => panic!("fixture check: expected a linear scale, got {other:?}"),
    };
    let centre = (place(Channel::X, 30.0), place(Channel::Y, 70.0));
    let first_dot_point = both
        .stream
        .iter()
        .position(|p| (p.0 - centre.0).hypot(p.1 - centre.1) < 12.0)
        .expect("the dot at (30, 70) is in the path stream");
    assert!(
        first_dot_point >= both.end(),
        "the rules end at point {} of the stream and the dot's first point is at \
         {first_dot_point}: a rule drawn after the dot would sit on top of it",
        both.end()
    );
}

// ---------------------------------------------------------------------------
// AC5 — a spec that asks for nothing
// ---------------------------------------------------------------------------

/// **A spec that sets none of the three draws the gridlines it drew before the
/// keys were read: a rule at each tick of both axes.** The scene is the same
/// scene, byte for byte, as the one that asks for both explicitly — which is
/// what keeps a spec written before these keys were read unchanged.
#[test]
fn a_spec_that_sets_none_of_the_three_draws_both_sets_as_it_did() {
    assert_paints("", true, true);

    let unset = compose("");
    let stream = points(&unset);
    for asked in ["grid: true", "xGrid: true\nyGrid: true"] {
        assert_eq!(
            points(&compose(asked)),
            stream,
            "`{asked}` asks for what an unset plot draws"
        );
    }
    assert!(
        said(&unset).is_empty(),
        "an unset plot has nothing to say; got {:?}",
        said(&unset)
    );
}

// ---------------------------------------------------------------------------
// plotDefaults, and a value that is no switch
// ---------------------------------------------------------------------------

/// A `plotDefaults` switch reaches a plot that does not write its own, and a
/// switch the plot writes wins over it.
#[test]
fn a_plot_defaults_switch_reaches_the_plot_and_the_plots_own_wins() {
    assert_paints("plotDefaults:\n  yGrid: false", true, false);
    assert_paints("plotDefaults:\n  yGrid: false\nyGrid: true", true, true);
}

/// **A value that is no `true` or `false` is named in the warning banner with
/// its key, and the plot draws the gridlines it draws without the key.** A
/// literal switch says nothing.
#[test]
fn a_value_that_is_no_switch_is_named_and_the_plot_draws_its_default() {
    let unset = points(&compose(""));
    for asked in ["yGrid: 'off'", "grid: 0", "xGrid: null"] {
        let composed = compose(asked);
        assert_eq!(
            points(&composed),
            unset,
            "`{asked}` is read as absent, so the plot draws what an unset plot draws"
        );
        let key = asked.split(':').next().expect("a key");
        let lines = said(&composed);
        assert!(
            lines.len() == 1 && lines[0].contains(key),
            "`{asked}` is named once, by its key `{key}`; got {lines:?}"
        );
    }
    for asked in ["grid: false", "yGrid: true", "xGrid: false"] {
        let lines = said(&compose(asked));
        assert!(
            lines.is_empty(),
            "`{asked}` is a switch and draws in silence; got {lines:?}"
        );
    }
}
