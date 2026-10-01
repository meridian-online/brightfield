//! **A spec that sets `xZero`, `xNice`, `yZero` or `yNice` draws the axis ends
//! it asked for.**
//!
//! No code read the four keys, so a plot drew each axis from its data's lowest
//! value to its highest whatever the spec said, and an analyst who asked for an
//! axis that starts at zero, or ends on round numbers, got neither.
//!
//! Assertions read two things. The domain of the scale the plot was composed
//! against says what the axis ends are; the path stream of the painted scene
//! says the marks were drawn against that same domain, because a domain widened
//! after the marks are placed would pass a check that only read the scale and
//! leave the picture where it was. The dot at a known value is found in the
//! stream and compared with where the expected domain puts it.
//!
//! An asked-for arm is paired with the other axis left alone and with the same
//! spec asking for nothing, since a fixture whose default is the setting asked
//! for would pass without the key being read.

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_render::axis::compute_ticks;
use brightfield_render::channel::Channel;
use brightfield_render::scale::{Scale, ViewExtent};
use brightfield_shell::pipeline::{Composed, LiveDashboard};
use brightfield_spec::analysis::ComponentPath;
use brightfield_sql::ir::ScalarValue;

// ---------------------------------------------------------------------------
// The fixtures
// ---------------------------------------------------------------------------

/// Three dots whose values stop short of zero: x from 45 to 95 and y from 40 to
/// 90. `ATTRS` marks where the plot attributes go, so the arms differ by those
/// lines.
const ABOVE_ZERO: &str = r"
data:
  pts:
    - { a: 45, b: 40 }
    - { a: 70, b: 65 }
    - { a: 95, b: 90 }
plot:
  - mark: dot
    data: { from: pts }
    x: a
    y: b
width: 600
height: 300
ATTRS
";

/// Three dots whose values stop short of the round numbers either side: x from
/// 4 to 96 and y from 3 to 97, so a nice axis runs 0 to 100 on both.
const OFF_ROUND: &str = r"
data:
  pts:
    - { a: 4,  b: 3 }
    - { a: 50, b: 60 }
    - { a: 96, b: 97 }
plot:
  - mark: dot
    data: { from: pts }
    x: a
    y: b
width: 600
height: 300
ATTRS
";

/// Four bars, the loads from 40 to 90, on a value axis that is y.
const BARS: &str = r"
data:
  pts:
    - { site: Alder,  load: 40 }
    - { site: Birch,  load: 55 }
    - { site: Cedar,  load: 70 }
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

/// A scatter to brush on, and a second scatter of the same table filtered by
/// that brush, the plot the arms put their attributes on.
const BRUSHED: &str = r"
params:
  brush: { select: crossfilter }
data:
  pts:
    - { a: 4,  b: 3 }
    - { a: 20, b: 22 }
    - { a: 50, b: 60 }
    - { a: 80, b: 71 }
    - { a: 96, b: 97 }
hconcat:
  - plot:
      - mark: dot
        data: { from: pts }
        x: a
        y: b
      - select: intervalX
        as: $brush
    width: 300
    height: 240
  - plot:
      - mark: dot
        data: { from: pts, filterBy: $brush }
        x: a
        y: b
    width: 300
    height: 240
ATTRS
";

fn compose_from(template: &str, attrs: &str) -> Composed {
    LiveDashboard::load_str(&template.replace("ATTRS", attrs), None)
        .expect("the spec loads")
        .present()
        .expect("the spec composes")
}

/// What the load said, one line per diagnostic: what the warning banner draws.
fn said(composed: &Composed) -> Vec<String> {
    composed.diagnostics.lines()
}

// ---------------------------------------------------------------------------
// Reading a drawn scale, and where a dot is painted
// ---------------------------------------------------------------------------

/// The drawn `(min, max)` of the first plot's `channel`, insisting it is a
/// linear scale.
fn domain(composed: &Composed, plot: usize, channel: Channel) -> (f64, f64) {
    match composed.plots[plot]
        .scales
        .get(channel)
        .expect("the plot drew this channel")
    {
        Scale::Linear {
            domain_min,
            domain_max,
            ..
        } => (*domain_min, *domain_max),
        other => panic!("fixture check: expected a linear scale, got {other:?}"),
    }
}

fn first_domain(composed: &Composed, channel: Channel) -> (f64, f64) {
    domain(composed, 0, channel)
}

/// Every coordinate pair the scene's path stream holds, in draw order, in the
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

/// Where `value` lands on an axis drawn over `(lo, hi)`, on the pixel range of
/// the first plot's `channel` scale.
fn pixel_at(composed: &Composed, channel: Channel, (lo, hi): (f64, f64), value: f64) -> f64 {
    let scale = composed.plots[0]
        .scales
        .get(channel)
        .expect("the plot drew this channel");
    scale.range_start() + (value - lo) / (hi - lo) * (scale.range_end() - scale.range_start())
}

/// The y of the centre of the dot painted near `(x, y)`: the mean y of the
/// path points within twelve pixels of it. A dot's outline sits around its
/// centre, and the gridline and axis points that pass through the plot sit at
/// its edges, so they are not near.
fn painted_dot_y(composed: &Composed, near: (f64, f64)) -> f64 {
    let hits: Vec<f64> = scene_points(composed)
        .into_iter()
        .filter(|p| (p.0 - near.0).hypot(p.1 - near.1) < 12.0)
        .map(|p| p.1)
        .collect();
    assert!(
        !hits.is_empty(),
        "no dot is painted near ({}, {})",
        near.0,
        near.1
    );
    hits.iter().sum::<f64>() / hits.len() as f64
}

// ---------------------------------------------------------------------------
// AC1 — yZero
// ---------------------------------------------------------------------------

/// **With `yZero: true`, a plot of dots whose y values run from 40 to 90 draws
/// a y axis that starts at 0. The same plot without the key draws one that
/// starts above 0.** The x axis, which the key does not name, is left where it
/// was, and the top of the y axis stays at the data's top.
#[test]
fn y_zero_starts_a_dot_plots_y_axis_at_zero() {
    let asked = compose_from(ABOVE_ZERO, "yZero: true");
    let unset = compose_from(ABOVE_ZERO, "");

    let (y_lo, y_hi) = first_domain(&asked, Channel::Y);
    assert_eq!(y_lo, 0.0, "`yZero: true` starts the y axis at 0");
    let (unset_lo, unset_hi) = first_domain(&unset, Channel::Y);
    assert!(
        unset_lo > 0.0,
        "fixture check: with no key the y axis starts above 0, at {unset_lo}"
    );
    assert_eq!(
        y_hi, unset_hi,
        "zero moves the start of the axis, not its end"
    );
    assert_eq!(
        first_domain(&asked, Channel::X),
        first_domain(&unset, Channel::X),
        "`yZero` names the y axis; the x axis draws as it did"
    );

    // Painted: the dot at (70, 65) sits where an axis from 0 puts it, and not
    // where the axis it had put it.
    let x_px = pixel_at(
        &asked,
        Channel::X,
        first_domain(&asked, Channel::X),
        70.0,
    );
    let expected = pixel_at(&asked, Channel::Y, (0.0, y_hi), 65.0);
    let moved = pixel_at(&asked, Channel::Y, (unset_lo, unset_hi), 65.0);
    assert!(
        (expected - moved).abs() > 5.0,
        "fixture check: zero moves the dot by more than the tolerance below"
    );
    let painted = painted_dot_y(&asked, (x_px, expected));
    assert!(
        (painted - expected).abs() < 1.5,
        "the dot at y = 65 is painted at {painted}; an axis from 0 puts it at {expected}"
    );

    // `false` is no request.
    assert_eq!(
        first_domain(&compose_from(ABOVE_ZERO, "yZero: false"), Channel::Y),
        (unset_lo, unset_hi),
        "`yZero: false` draws what an unset plot draws"
    );
}

// ---------------------------------------------------------------------------
// AC2 — yNice
// ---------------------------------------------------------------------------

/// **With `yNice: true`, a plot of dots whose y values run from 3 to 97 draws a
/// y axis from 0 to 100.** The same plot without the key draws the data's own
/// ends, and the dots are painted against the rounded ends.
#[test]
fn y_nice_ends_a_dot_plots_y_axis_on_round_numbers() {
    let asked = compose_from(OFF_ROUND, "yNice: true");
    let unset = compose_from(OFF_ROUND, "");

    assert_eq!(
        first_domain(&asked, Channel::Y),
        (0.0, 100.0),
        "`yNice: true` runs the y axis from 0 to 100"
    );
    assert_eq!(
        first_domain(&unset, Channel::Y),
        (3.0, 97.0),
        "fixture check: with no key the y axis runs from the data's low to its high"
    );
    assert_eq!(
        first_domain(&asked, Channel::X),
        first_domain(&unset, Channel::X),
        "`yNice` names the y axis; the x axis draws as it did"
    );

    let x_px = pixel_at(
        &asked,
        Channel::X,
        first_domain(&asked, Channel::X),
        50.0,
    );
    let expected = pixel_at(&asked, Channel::Y, (0.0, 100.0), 60.0);
    let moved = pixel_at(&asked, Channel::Y, (3.0, 97.0), 60.0);
    assert!(
        (expected - moved).abs() > 1.0,
        "fixture check: rounding moves the dot by more than the tolerance below"
    );
    let painted = painted_dot_y(&asked, (x_px, expected));
    assert!(
        (painted - expected).abs() < 0.75,
        "the dot at y = 60 is painted at {painted}; an axis from 0 to 100 puts it at {expected}"
    );

    assert_eq!(
        first_domain(&compose_from(OFF_ROUND, "yNice: false"), Channel::Y),
        (3.0, 97.0),
        "`yNice: false` draws what an unset plot draws"
    );
}

/// **A rounded end is a tick.** The ends are multiples of the step the axis
/// draws its ticks at for the plot's target count, so the axis has a tick at
/// each of its ends and the top one is a number an analyst can say aloud. The
/// same data rounds to different ends under different counts, which is how a
/// rounding that ignored the count would show: 12 to 47 under the default five
/// ticks steps by 5 and runs 10 to 50, and under `yTicks: 10` steps by 2 and runs
/// 12 to 48.
#[test]
fn a_rounded_end_is_a_tick_at_the_plots_own_tick_count() {
    const TEMPLATE: &str = r"
data:
  pts:
    - { a: 1, b: 12 }
    - { a: 2, b: 30 }
    - { a: 3, b: 47 }
plot:
  - mark: dot
    data: { from: pts }
    x: a
    y: b
width: 600
height: 300
ATTRS
";
    for (attrs, target, expected) in [
        ("yNice: true", 5, (10.0, 50.0)),
        ("yNice: true\nyTicks: 10", 10, (12.0, 48.0)),
    ] {
        let composed = compose_from(TEMPLATE, attrs);
        let ends = first_domain(&composed, Channel::Y);
        assert_eq!(ends, expected, "`{attrs}`: the rounded ends");
        let scale = composed.plots[0]
            .scales
            .get(Channel::Y)
            .expect("the plot drew y");
        let ticks = compute_ticks(scale, target);
        let values: Vec<f64> = ticks.iter().map(|t| t.value).collect();
        let first = *values.first().expect("the axis has ticks");
        let last = *values.last().expect("the axis has ticks");
        assert!(
            (first - ends.0).abs() < 1e-9 && (last - ends.1).abs() < 1e-9,
            "`{attrs}`: the axis ends at {ends:?} and its ticks run {values:?}"
        );
    }
}

/// Zero first, then round ends, as Observable Plot orders them: a domain carried
/// to zero is rounded from zero. 40 to 90 with both runs 0 to 100.
#[test]
fn zero_is_applied_before_round_ends() {
    assert_eq!(
        first_domain(&compose_from(ABOVE_ZERO, "yZero: true\nyNice: true"), Channel::Y),
        (0.0, 100.0)
    );
    assert_eq!(
        first_domain(&compose_from(ABOVE_ZERO, "yNice: true"), Channel::Y),
        (40.0, 90.0),
        "rounding alone leaves 40 to 90, which are already round"
    );
}

// ---------------------------------------------------------------------------
// AC3 — the x axis
// ---------------------------------------------------------------------------

/// **The same two hold on the x axis with `xZero` and `xNice`.** Each is paired
/// with the y axis left alone.
#[test]
fn x_zero_and_x_nice_hold_on_the_x_axis() {
    let unset_zero = compose_from(ABOVE_ZERO, "");
    let zeroed = compose_from(ABOVE_ZERO, "xZero: true");
    let (x_lo, x_hi) = first_domain(&zeroed, Channel::X);
    let (unset_lo, unset_hi) = first_domain(&unset_zero, Channel::X);
    assert_eq!(x_lo, 0.0, "`xZero: true` starts the x axis at 0");
    assert!(
        unset_lo > 0.0,
        "fixture check: with no key the x axis starts above 0, at {unset_lo}"
    );
    assert_eq!(x_hi, unset_hi, "zero moves the start of the axis only");
    assert_eq!(
        first_domain(&zeroed, Channel::Y),
        first_domain(&unset_zero, Channel::Y),
        "`xZero` names the x axis; the y axis draws as it did"
    );
    assert_eq!(
        first_domain(&compose_from(ABOVE_ZERO, "xZero: false"), Channel::X),
        (unset_lo, unset_hi)
    );

    let unset_nice = compose_from(OFF_ROUND, "");
    let rounded = compose_from(OFF_ROUND, "xNice: true");
    assert_eq!(
        first_domain(&rounded, Channel::X),
        (0.0, 100.0),
        "`xNice: true` runs the x axis from 0 to 100"
    );
    assert_eq!(first_domain(&unset_nice, Channel::X), (4.0, 96.0));
    assert_eq!(
        first_domain(&rounded, Channel::Y),
        first_domain(&unset_nice, Channel::Y),
        "`xNice` names the x axis; the y axis draws as it did"
    );
    assert_eq!(
        first_domain(&compose_from(OFF_ROUND, "xNice: false"), Channel::X),
        (4.0, 96.0)
    );

    // Painted: the dot at (50, 60) sits where an x axis from 0 to 100 puts it.
    let expected_x = pixel_at(&rounded, Channel::X, (0.0, 100.0), 50.0);
    let y_px = pixel_at(
        &rounded,
        Channel::Y,
        first_domain(&rounded, Channel::Y),
        60.0,
    );
    let hits: Vec<f64> = scene_points(&rounded)
        .into_iter()
        .filter(|p| (p.0 - expected_x).hypot(p.1 - y_px) < 12.0)
        .map(|p| p.0)
        .collect();
    assert!(!hits.is_empty(), "no dot is painted near the expected place");
    let painted = hits.iter().sum::<f64>() / hits.len() as f64;
    assert!(
        (painted - expected_x).abs() < 0.75,
        "the dot at x = 50 is painted at {painted}; an axis from 0 to 100 puts it at {expected_x}"
    );
}

// ---------------------------------------------------------------------------
// AC4 — a bar's own zero
// ---------------------------------------------------------------------------

/// **A bar's value axis starts at 0 with `yZero` set to `true`, set to `false`
/// or absent.** A request can carry an axis to zero and cannot cut a bar's off
/// it, so `yZero: false` leaves the bar drawn from zero as it was.
#[test]
fn a_bars_value_axis_starts_at_zero_whatever_y_zero_says() {
    for attrs in ["", "yZero: true", "yZero: false"] {
        let composed = compose_from(BARS, attrs);
        let (lo, hi) = first_domain(&composed, Channel::Y);
        assert_eq!(lo, 0.0, "`{attrs}`: a bar's value axis starts at 0");
        assert_eq!(hi, 90.0, "`{attrs}`: and ends at the tallest bar");
    }
    // Round ends still apply to a bar's value axis, from the zero it carries.
    assert_eq!(
        first_domain(&compose_from(BARS, "yNice: true"), Channel::Y),
        (0.0, 100.0)
    );
}

// ---------------------------------------------------------------------------
// AC5 — a spec that asks for nothing
// ---------------------------------------------------------------------------

/// **A spec that sets none of the four draws the axis ends it drew before the
/// keys were read.** The scene is the same scene, byte for byte, as the one
/// that writes `false` at each key, and the banner has nothing to say about it.
/// The committed dashboard baselines are held by the workspace's own suites,
/// which run this build against them.
#[test]
fn a_spec_that_sets_none_of_the_four_draws_what_it_drew() {
    for template in [ABOVE_ZERO, OFF_ROUND, BARS] {
        let unset = compose_from(template, "");
        let off = compose_from(template, "xZero: false\nyZero: false\nxNice: false\nyNice: false");
        assert_eq!(
            scene_points(&unset),
            scene_points(&off),
            "`false` at each key asks for what an unset plot draws"
        );
        assert!(
            said(&unset).is_empty(),
            "an unset plot has nothing to say; got {:?}",
            said(&unset)
        );
        assert!(
            said(&off).is_empty(),
            "a literal switch draws in silence; got {:?}",
            said(&off)
        );
    }
    // And a request is not a no-op: the same spec asked for something draws a
    // different scene, so the equality above is not two scenes that ignore the keys.
    assert_ne!(
        scene_points(&compose_from(OFF_ROUND, "")),
        scene_points(&compose_from(OFF_ROUND, "yNice: true")),
    );
}

// ---------------------------------------------------------------------------
// A fixed domain, and a reader who navigates
// ---------------------------------------------------------------------------

/// Drive one interval selection into `live` and re-composite.
fn brush(live: &mut LiveDashboard, from: &str, lo: f64, hi: f64) -> Composed {
    live.apply(Interaction::Select {
        name: "brush".to_string(),
        contributor: ComponentPath(from.to_string()),
        predicate: SqlPredicate::Interval {
            column: "a".to_string(),
            lo: ScalarValue::Float(lo),
            hi: ScalarValue::Float(hi),
            meta: None,
        },
    })
    .expect("the brush re-composites")
}

/// **A nice end meets a fixed domain as Mosaic's renderer meets it: the domain
/// `Fixed` holds still is the one with its ends already carried.** Mosaic reads
/// a fixed domain back from the rendered scale, after Observable Plot has applied
/// zero and round ends, and Plot applies them again to the explicit domain it is
/// handed, which moves nothing. So a filter that narrows the data leaves a fixed
/// axis at the rounded ends of the first composition, and the same plot
/// without `Fixed` rounds the filtered data afresh.
#[test]
fn a_fixed_domain_holds_the_ends_carried_on_its_first_composition() {
    // The attributes go at the plot's own indent, after the last plot's size,
    // so they belong to the filtered plot, the second.
    let pinned_source = BRUSHED.replace("ATTRS", "    yDomain: Fixed\n    yNice: true\n");
    let unpinned_source = BRUSHED.replace("ATTRS", "    yNice: true\n");

    for (label, source, holds) in [
        ("pinned", pinned_source, true),
        ("unpinned", unpinned_source, false),
    ] {
        let mut live = LiveDashboard::load_str(&source, None).expect("the spec loads live");
        let first = live.present().expect("first composite");
        assert_eq!(
            domain(&first, 1, Channel::Y),
            (0.0, 100.0),
            "{label}: the first composition rounds the data's 3 to 97 to 0 to 100"
        );
        let path = first.plots[0].path.clone();
        // A brush over the middle of x keeps the dots at y = 22, 60 and 71 and
        // so a data extent of 22 to 71, which rounds to 20 to 80 on its own.
        let narrowed = brush(&mut live, &path, 15.0, 85.0);
        let after = domain(&narrowed, 1, Channel::Y);
        if holds {
            assert_eq!(
                after,
                (0.0, 100.0),
                "{label}: a fixed axis keeps the ends the first composition carried"
            );
        } else {
            assert_eq!(
                after,
                (20.0, 80.0),
                "{label}: an unpinned axis rounds the filtered data afresh"
            );
        }
    }
}

/// **A reader who navigates an axis leaves it where they put it.** Rounding a
/// zoomed frame outward would undo the zoom, so an axis with a view extent is
/// not carried to zero or to round ends, and the axis the reader did not touch
/// still is.
#[test]
fn a_navigated_axis_is_not_carried_back_to_zero_or_round_ends() {
    let source = OFF_ROUND.replace("ATTRS", "yZero: true\nyNice: true\nxNice: true");
    let mut live = LiveDashboard::load_str(&source, None).expect("the spec loads live");
    let before = live.present().expect("first composite");
    assert_eq!(first_domain(&before, Channel::Y), (0.0, 100.0));

    let zoom = (12.3, 47.9);
    let path = before.plots[0].path.clone();
    live.set_view_extent(
        &path,
        ViewExtent {
            x: None,
            y: Some(zoom),
        },
    );
    let after = live.present().expect("re-composite at the navigated extent");
    assert_eq!(
        first_domain(&after, Channel::Y),
        zoom,
        "the reader zoomed y; zero and round ends put it back"
    );
    assert_eq!(
        first_domain(&after, Channel::X),
        (0.0, 100.0),
        "x was not navigated, so it still takes its round ends"
    );
}

// ---------------------------------------------------------------------------
// plotDefaults, and a value that is no switch
// ---------------------------------------------------------------------------

/// A `plotDefaults` switch reaches a plot that does not write its own, and a
/// switch the plot writes wins over it.
#[test]
fn a_plot_defaults_switch_reaches_the_plot_and_the_plots_own_wins() {
    assert_eq!(
        first_domain(
            &compose_from(OFF_ROUND, "plotDefaults:\n  yNice: true"),
            Channel::Y
        ),
        (0.0, 100.0)
    );
    assert_eq!(
        first_domain(
            &compose_from(OFF_ROUND, "plotDefaults:\n  yNice: true\nyNice: false"),
            Channel::Y
        ),
        (3.0, 97.0)
    );
}

/// **A value that is no `true` or `false` is named in the warning banner with
/// its key, and the plot draws the axis ends it draws without the key.** A
/// literal switch says nothing.
#[test]
fn a_value_that_is_no_switch_is_named_and_the_plot_draws_its_default() {
    let unset = scene_points(&compose_from(OFF_ROUND, ""));
    for asked in ["yNice: 'yes'", "xZero: 1", "yNice: 5", "xNice: null"] {
        let composed = compose_from(OFF_ROUND, asked);
        assert_eq!(
            scene_points(&composed),
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
    for asked in ["yZero: true", "yNice: false", "xZero: false", "xNice: true"] {
        let lines = said(&compose_from(OFF_ROUND, asked));
        assert!(
            lines.is_empty(),
            "`{asked}` is a switch and draws in silence; got {lines:?}"
        );
    }
}
