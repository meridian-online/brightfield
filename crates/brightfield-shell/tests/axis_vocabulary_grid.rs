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
//! lines, the tick marks and the mark are in both. A switch resolved correctly
//! and dropped before the draw would pass a check that read the resolver, and a
//! rule at the wrong value or short of the data area would pass one that only
//! counted them. Each rule is therefore matched against where the tick it
//! belongs to sits, and against the data area's edges.
//!
//! The axes are given different tick counts — three x ticks and eleven y ticks
//! on a 0 to 100 domain — so a switch wired to the other axis draws the wrong
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
fn points(composed: &Composed) -> Vec<(f64, f64)> {
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

/// The rules a spec paints: the run of points its scene holds beyond the scene
/// of the same plot with `grid: false`, read two points to a rule, and where in
/// the stream that run sits.
struct Painted {
    /// The whole point stream of the spec's scene.
    stream: Vec<(f64, f64)>,
    /// Index in `stream` of the first point of the first rule.
    start: usize,
    rules: Vec<Rule>,
}

impl Painted {
    /// Index in `stream` one past the last point of the last rule.
    fn end(&self) -> usize {
        self.start + self.rules.len() * 2
    }

    fn horizontal(&self) -> Vec<Rule> {
        self.rules
            .iter()
            .copied()
            .filter(|r| r[0].1 == r[1].1)
            .collect()
    }

    fn vertical(&self) -> Vec<Rule> {
        self.rules
            .iter()
            .copied()
            .filter(|r| r[0].0 == r[1].0)
            .collect()
    }
}

fn painted(attrs: &str) -> Painted {
    let bare = points(&compose("grid: false"));
    let stream = points(&compose(attrs));
    let added = stream
        .len()
        .checked_sub(bare.len())
        .expect("a spec never paints fewer points than the same plot with no gridlines");
    let start = stream.iter().zip(&bare).take_while(|(a, b)| a == b).count();
    assert_eq!(
        stream[start + added..],
        bare[start..],
        "`{attrs}`: what the gridline switches add is one run of points, and the rest of the \
         scene is unchanged"
    );
    assert_eq!(added % 2, 0, "a rule is two points");
    let rules = stream[start..start + added]
        .chunks_exact(2)
        .map(|pair| [pair[0], pair[1]])
        .collect();
    Painted {
        stream,
        start,
        rules,
    }
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

/// Where the axis puts each tick, in the plot's own coordinates.
fn tick_positions(composed: &Composed, channel: Channel, target: usize) -> Vec<f64> {
    compute_ticks(scale(composed, channel), target)
        .iter()
        .map(|tick| tick.position)
        .collect()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

/// Whether `rule` runs the whole data area horizontally at `y`.
fn is_horizontal_rule_at(rule: &Rule, y: f64, area: (f64, f64, f64, f64)) -> bool {
    let (left, right, _, _) = area;
    near(rule[0].0, left) && near(rule[1].0, right) && near(rule[0].1, y) && near(rule[1].1, y)
}

/// Whether `rule` runs the whole data area vertically at `x`.
fn is_vertical_rule_at(rule: &Rule, x: f64, area: (f64, f64, f64, f64)) -> bool {
    let (_, _, top, bottom) = area;
    near(rule[0].0, x) && near(rule[1].0, x) && near(rule[0].1, top) && near(rule[1].1, bottom)
}

/// Assert that `rules` are exactly the horizontal rules at each y tick of
/// `composed`, in tick order and across the data area.
fn assert_a_rule_at_each_y_tick(rules: &[Rule], composed: &Composed, what: &str) {
    let ticks = tick_positions(composed, Channel::Y, 10);
    assert_eq!(ticks.len(), Y_TICKS, "fixture check: the y ticks");
    assert_eq!(
        rules.len(),
        Y_TICKS,
        "{what}: one horizontal rule at each of the {Y_TICKS} y ticks; got {rules:?}"
    );
    let area = data_area(composed);
    for (rule, tick) in rules.iter().zip(&ticks) {
        assert!(
            is_horizontal_rule_at(rule, *tick, area),
            "{what}: the rule {rule:?} should run the data area at the y tick {tick} \
             (data area {area:?})"
        );
    }
}

/// Assert that `rules` are exactly the vertical rules at each x tick of
/// `composed`, in tick order and across the data area.
fn assert_a_rule_at_each_x_tick(rules: &[Rule], composed: &Composed, what: &str) {
    let ticks = tick_positions(composed, Channel::X, 2);
    assert_eq!(ticks.len(), X_TICKS, "fixture check: the x ticks");
    assert_eq!(
        rules.len(),
        X_TICKS,
        "{what}: one vertical rule at each of the {X_TICKS} x ticks; got {rules:?}"
    );
    let area = data_area(composed);
    for (rule, tick) in rules.iter().zip(&ticks) {
        assert!(
            is_vertical_rule_at(rule, *tick, area),
            "{what}: the rule {rule:?} should run the data area at the x tick {tick} \
             (data area {area:?})"
        );
    }
}

// ---------------------------------------------------------------------------
// AC1 and AC2 — one axis
// ---------------------------------------------------------------------------

/// **`yGrid: true` draws one horizontal rule at each y tick across the data
/// area, and `yGrid: false` draws none.** The x axis names no key in either
/// arm, so its vertical rules are what an unset key draws.
#[test]
fn y_grid_draws_a_horizontal_rule_at_each_y_tick_or_none() {
    let composed = compose("yGrid: true");
    let on = painted("yGrid: true");
    assert_a_rule_at_each_y_tick(&on.horizontal(), &composed, "`yGrid: true`");
    assert_a_rule_at_each_x_tick(&on.vertical(), &composed, "`yGrid: true` leaves x at its default");

    let off = painted("yGrid: false");
    assert!(
        off.horizontal().is_empty(),
        "`yGrid: false` draws no horizontal rule; got {:?}",
        off.horizontal()
    );
    assert_a_rule_at_each_x_tick(
        &off.vertical(),
        &compose("yGrid: false"),
        "`yGrid: false` leaves x at its default",
    );
}

/// **`xGrid: true` draws one vertical rule at each x tick, and `xGrid: false`
/// draws none.** The y axis names no key in either arm.
#[test]
fn x_grid_draws_a_vertical_rule_at_each_x_tick_or_none() {
    let composed = compose("xGrid: true");
    let on = painted("xGrid: true");
    assert_a_rule_at_each_x_tick(&on.vertical(), &composed, "`xGrid: true`");
    assert_a_rule_at_each_y_tick(&on.horizontal(), &composed, "`xGrid: true` leaves y at its default");

    let off = painted("xGrid: false");
    assert!(
        off.vertical().is_empty(),
        "`xGrid: false` draws no vertical rule; got {:?}",
        off.vertical()
    );
    assert_a_rule_at_each_y_tick(
        &off.horizontal(),
        &compose("xGrid: false"),
        "`xGrid: false` leaves y at its default",
    );
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
    let composed = compose("grid: true");

    let both = painted("grid: true");
    assert_a_rule_at_each_x_tick(&both.vertical(), &composed, "`grid: true`");
    assert_a_rule_at_each_y_tick(&both.horizontal(), &composed, "`grid: true`");

    let y_only = painted("grid: true\nxGrid: false");
    assert!(
        y_only.vertical().is_empty(),
        "`grid: true` with `xGrid: false` draws no vertical rule; got {:?}",
        y_only.vertical()
    );
    assert_a_rule_at_each_y_tick(
        &y_only.horizontal(),
        &composed,
        "`grid: true` with `xGrid: false`",
    );

    let x_only = painted("grid: true\nyGrid: false");
    assert!(
        x_only.horizontal().is_empty(),
        "`grid: true` with `yGrid: false` draws no horizontal rule; got {:?}",
        x_only.horizontal()
    );
    assert_a_rule_at_each_x_tick(
        &x_only.vertical(),
        &composed,
        "`grid: true` with `yGrid: false`",
    );

    let x_back_on = painted("grid: false\nxGrid: true");
    assert!(
        x_back_on.horizontal().is_empty(),
        "`grid: false` with `xGrid: true` draws no horizontal rule; got {:?}",
        x_back_on.horizontal()
    );
    assert_a_rule_at_each_x_tick(
        &x_back_on.vertical(),
        &composed,
        "`grid: false` with `xGrid: true`",
    );

    let neither = painted("grid: false");
    assert!(
        neither.rules.is_empty(),
        "`grid: false` draws no rule; got {:?}",
        neither.rules
    );
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
/// labels.** A rule's ends are on the data area's edges; the tick labels are
/// read off the same scene and none of them is inside that area; and the rules
/// are drawn before the dot, so the dot is on top of them.
#[test]
fn the_rules_are_under_the_marks_and_inside_the_data_area_clear_of_the_labels() {
    let composed = compose("grid: true");
    let both = painted("grid: true");
    assert_eq!(both.rules.len(), X_TICKS + Y_TICKS, "fixture check: the rules");

    let (left, right, top, bottom) = data_area(&composed);
    for rule in &both.rules {
        for (x, y) in rule {
            assert!(
                *x >= left - 1e-3 && *x <= right + 1e-3 && *y >= top - 1e-3 && *y <= bottom + 1e-3,
                "the rule {rule:?} leaves the data area {left}..{right} by {top}..{bottom}"
            );
        }
    }

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
    let unset = compose("");
    let on = painted("");
    assert_a_rule_at_each_x_tick(&on.vertical(), &unset, "an unset plot");
    assert_a_rule_at_each_y_tick(&on.horizontal(), &unset, "an unset plot");

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

/// A `plotDefaults` switch reaches a plot that writes none of its own, and a
/// switch the plot writes wins over it.
#[test]
fn a_plot_defaults_switch_reaches_the_plot_and_the_plots_own_wins() {
    let inherited = painted("plotDefaults:\n  yGrid: false");
    assert!(
        inherited.horizontal().is_empty(),
        "the plot inherits `yGrid: false`; got {:?}",
        inherited.horizontal()
    );
    assert_a_rule_at_each_x_tick(
        &inherited.vertical(),
        &compose("plotDefaults:\n  yGrid: false"),
        "an inherited `yGrid: false` leaves x at its default",
    );

    let overridden = painted("plotDefaults:\n  yGrid: false\nyGrid: true");
    assert_a_rule_at_each_y_tick(
        &overridden.horizontal(),
        &compose("plotDefaults:\n  yGrid: false\nyGrid: true"),
        "the plot's own `yGrid: true` over an inherited `false`",
    );
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
