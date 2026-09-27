//! **A spec that sets `xTicks` or `yTicks` draws the ticks it asked for.**
//!
//! A plot's tick count was stored in its attribute bag and read by nothing, so
//! an axis drew its own count whatever the spec said. The count is a target,
//! not a promise: the axis picks the step of 1, 2 or 5 times a power of ten
//! nearest it. On a 0 to 100 domain that puts `2` at 0, 50 and 100, and `10`
//! at the multiples of ten.
//!
//! Assertions read what was PAINTED — the tick labels' glyph runs on
//! `Composed::scene`, which is what a viewer is shown — rather than what the
//! pipeline reports it drew from. A count resolved correctly and dropped
//! before the draw would pass a check that read the resolver, and a label
//! placed at the wrong value would pass one that read only how many there are.
//! Each painted label is therefore matched on two things at once: its glyph
//! count, which tells `"5"` from `"50"` from `"100"`, and its position, which
//! is checked against where the SCALE places the value it should be.
//!
//! An asked-for arm is worth pairing with the same spec asking for nothing.
//! A tick set that changed could be a fixture whose default is the number
//! asked for, and the unset arm is what shows the difference is the spec's.

use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_render::text::{measure_width, LABEL_SIZE};
use brightfield_shell::pipeline::{Composed, LiveDashboard};

// ---------------------------------------------------------------------------
// The fixture
// ---------------------------------------------------------------------------

/// Two columns that each run from 0 to 100, so both axes infer the same
/// `[0, 100]` domain. `ATTRS` marks where the plot attributes go, so the arms
/// differ by those lines and nothing else.
const TEMPLATE: &str = r"
data:
  pts:
    - { a: 0,   b: 0 }
    - { a: 25,  b: 60 }
    - { a: 70,  b: 30 }
    - { a: 100, b: 100 }
plot:
  - mark: dot
    data: { from: pts }
    x: a
    y: b
width: 600
height: 300
ATTRS
";

fn compose(attrs: &str) -> Composed {
    let spec = TEMPLATE.replace("ATTRS", attrs);
    LiveDashboard::load_str(&spec, None)
        .expect("the spec loads")
        .present()
        .expect("the spec composes")
}

fn scale(composed: &Composed, channel: Channel) -> &Scale {
    composed.plots[0]
        .scales
        .get(channel)
        .expect("the plot drew this channel")
}

/// The number of glyphs a whole-number label paints: one per character.
fn glyphs(value: f64) -> usize {
    format!("{}", value as i64).len()
}

// ---------------------------------------------------------------------------
// Reading the painted tick labels
// ---------------------------------------------------------------------------

/// The plot's horizontal, label-sized glyph runs, as `(x0, baseline_y, glyphs)`
/// in the dashboard's coordinates. The axis titles are a different size and the
/// y title is rotated, so neither is in this list.
fn label_runs(composed: &Composed) -> Vec<(f64, f64, usize)> {
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
                run.glyphs.len(),
            )
        })
        .collect()
}

/// A tick label is placed `LABEL_SIZE / 3` below its tick on the y axis and
/// `TICK_LENGTH + LABEL_SIZE` below the axis line on the x axis (both private
/// to `axis.rs`). A y label at the very bottom edge is therefore at most
/// `LABEL_SIZE / 3` below it, and an x label at least `+ 10.0` below it, so the
/// bottom edge plus ten separates the two rows without naming either constant
/// — the same split `log_binned_histogram.rs` reads its x row by.
const ROW_SPLIT: f64 = 10.0;

/// The x axis's painted tick labels, left to right: `(x0, glyphs)`.
fn painted_x_labels(composed: &Composed) -> Vec<(f64, usize)> {
    let plot = &composed.plots[0];
    let x_row = plot.rect.y + plot.layout.plot_y_end() + ROW_SPLIT;
    let mut runs: Vec<(f64, usize)> = label_runs(composed)
        .into_iter()
        .filter(|&(_, y, _)| y > x_row)
        .map(|(x, _, n)| (x, n))
        .collect();
    runs.sort_by(|a, b| a.0.total_cmp(&b.0));
    runs
}

/// The y axis's painted tick labels, top to bottom: `(baseline_y, glyphs)`.
/// They sit left of the axis line, above the x row.
fn painted_y_labels(composed: &Composed) -> Vec<(f64, usize)> {
    let plot = &composed.plots[0];
    let x_row = plot.rect.y + plot.layout.plot_y_end() + ROW_SPLIT;
    let axis_x = plot.rect.x + plot.layout.plot_x_start();
    let mut runs: Vec<(f64, usize)> = label_runs(composed)
        .into_iter()
        .filter(|&(x, y, _)| y <= x_row && x < axis_x)
        .map(|(_, y, n)| (y, n))
        .collect();
    runs.sort_by(|a, b| a.0.total_cmp(&b.0));
    runs
}

/// What the x axis should paint for `values`, left to right, by where the
/// SCALE places each — not by where `compute_ticks` says it did.
fn expected_x_labels(composed: &Composed, values: &[f64]) -> Vec<(f64, usize)> {
    let plot = &composed.plots[0];
    let scale = scale(composed, Channel::X);
    let mut out: Vec<(f64, usize)> = values
        .iter()
        .map(|&v| {
            let label = format!("{}", v as i64);
            let width = measure_width(&label, LABEL_SIZE);
            (plot.rect.x + scale.map_f64(v) - width / 2.0, glyphs(v))
        })
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// What the y axis should paint for `values`, top to bottom: each label's
/// baseline is `LABEL_SIZE / 3` below its tick.
fn expected_y_labels(composed: &Composed, values: &[f64]) -> Vec<(f64, usize)> {
    let plot = &composed.plots[0];
    let scale = scale(composed, Channel::Y);
    let mut out: Vec<(f64, usize)> = values
        .iter()
        .map(|&v| {
            (
                plot.rect.y + scale.map_f64(v) + f64::from(LABEL_SIZE) / 3.0,
                glyphs(v),
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// Assert `painted` is `expected`, label for label, within a pixel: the same
/// number of labels, each with the glyph count and position of the value it
/// stands for.
fn assert_labels(axis: &str, painted: &[(f64, usize)], expected: &[(f64, usize)], values: &[f64]) {
    assert_eq!(
        painted.len(),
        expected.len(),
        "the {axis} axis should paint one label per tick at {values:?}; painted {painted:?}, \
         expected {expected:?}"
    );
    for ((p_pos, p_glyphs), (e_pos, e_glyphs)) in painted.iter().zip(expected) {
        assert_eq!(
            p_glyphs, e_glyphs,
            "a painted {axis} label has {p_glyphs} glyphs where the tick at {values:?} wanted \
             {e_glyphs}; painted {painted:?}, expected {expected:?}"
        );
        assert!(
            (p_pos - e_pos).abs() < 1.0,
            "a painted {axis} label sits at {p_pos}, not at {e_pos} where the scale places its \
             value; painted {painted:?}, expected {expected:?}"
        );
    }
}

/// The multiples of `step` from 0 to 100, inclusive.
fn multiples(step: usize) -> Vec<f64> {
    (0..=100).step_by(step).map(|v| v as f64).collect()
}

/// Fixture check: the axis under test is a linear one over `[0, 100]`. A
/// domain that drifted (padded, niced, log) would move every expected value,
/// and each assertion below would then be a claim about the fixture.
fn assert_domain_is_0_to_100(composed: &Composed, channel: Channel) {
    match scale(composed, channel) {
        Scale::Linear {
            domain_min,
            domain_max,
            ..
        } => assert!(
            domain_min.abs() < 1e-9 && (domain_max - 100.0).abs() < 1e-9,
            "fixture check: {channel:?} should span [0, 100], spans [{domain_min}, {domain_max}]"
        ),
        other => panic!("fixture check: {channel:?} should be linear, is {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// AC1 — xTicks on the x axis
// ---------------------------------------------------------------------------

/// `xTicks: 2` on a 0 to 100 axis draws ticks at 0, 50 and 100.
#[test]
fn xticks_2_draws_ticks_at_0_50_and_100() {
    let composed = compose("xTicks: 2");
    assert_domain_is_0_to_100(&composed, Channel::X);
    let values = [0.0, 50.0, 100.0];
    assert_labels(
        "x",
        &painted_x_labels(&composed),
        &expected_x_labels(&composed, &values),
        &values,
    );
}

/// `xTicks: 10` draws eleven ticks, at the multiples of ten.
#[test]
fn xticks_10_draws_eleven_ticks_at_the_multiples_of_10() {
    let composed = compose("xTicks: 10");
    assert_domain_is_0_to_100(&composed, Channel::X);
    let values = multiples(10);
    assert_eq!(values.len(), 11, "fixture check: eleven multiples of ten");
    assert_labels(
        "x",
        &painted_x_labels(&composed),
        &expected_x_labels(&composed, &values),
        &values,
    );
}

// ---------------------------------------------------------------------------
// AC2 — the same on the y axis
// ---------------------------------------------------------------------------

/// `yTicks: 2` on a 0 to 100 axis draws ticks at 0, 50 and 100.
#[test]
fn yticks_2_draws_ticks_at_0_50_and_100() {
    let composed = compose("yTicks: 2");
    assert_domain_is_0_to_100(&composed, Channel::Y);
    let values = [0.0, 50.0, 100.0];
    assert_labels(
        "y",
        &painted_y_labels(&composed),
        &expected_y_labels(&composed, &values),
        &values,
    );
}

/// `yTicks: 10` draws eleven ticks, at the multiples of ten.
#[test]
fn yticks_10_draws_eleven_ticks_at_the_multiples_of_10() {
    let composed = compose("yTicks: 10");
    assert_domain_is_0_to_100(&composed, Channel::Y);
    let values = multiples(10);
    assert_labels(
        "y",
        &painted_y_labels(&composed),
        &expected_y_labels(&composed, &values),
        &values,
    );
}

/// **Each key reaches its own axis and neither reaches across.** `xTicks: 10`
/// leaves the y axis at its default, and `yTicks: 10` leaves the x axis at its
/// own. A resolver that read one key for both axes would satisfy each test
/// above and fail this one.
#[test]
fn each_key_reaches_only_its_own_axis() {
    let default = compose("");
    let x_only = compose("xTicks: 10");
    assert_eq!(
        painted_y_labels(&x_only),
        painted_y_labels(&default),
        "xTicks must not move the y axis's ticks"
    );
    let y_only = compose("yTicks: 10");
    assert_eq!(
        painted_x_labels(&y_only),
        painted_x_labels(&default),
        "yTicks must not move the x axis's ticks"
    );
}

// ---------------------------------------------------------------------------
// AC3 — a spec that sets neither key draws what it drew before
// ---------------------------------------------------------------------------

/// **The default arm.** A spec that sets neither key draws the six ticks a
/// target of five gives on 0 to 100, at 0, 20, 40, 60, 80 and 100 — the ticks
/// this build drew before either key was read — on both axes. Naming the
/// values, rather than only comparing against another composition, is what
/// keeps this from being satisfied by two draws that moved together.
#[test]
fn a_spec_that_sets_neither_key_draws_the_default_ticks() {
    let composed = compose("");
    assert_domain_is_0_to_100(&composed, Channel::X);
    assert_domain_is_0_to_100(&composed, Channel::Y);
    let values = multiples(20);
    assert_labels(
        "x",
        &painted_x_labels(&composed),
        &expected_x_labels(&composed, &values),
        &values,
    );
    assert_labels(
        "y",
        &painted_y_labels(&composed),
        &expected_y_labels(&composed, &values),
        &values,
    );
}

/// A target of five is the default, so asking for five draws what asking for
/// nothing does. This ties the default arm above to the request's own scale:
/// the two are one number, not two that happen to agree on this domain.
#[test]
fn asking_for_five_draws_what_asking_for_nothing_draws() {
    let default = compose("");
    let five = compose("xTicks: 5\nyTicks: 5");
    assert_eq!(painted_x_labels(&five), painted_x_labels(&default));
    assert_eq!(painted_y_labels(&five), painted_y_labels(&default));
}

// ---------------------------------------------------------------------------
// AC4 — a count that is not a whole number above zero is named and ignored
// ---------------------------------------------------------------------------

/// **A bad count draws the default ticks and says why.** `xTicks: -3` is the
/// spec's own example; zero, a fraction and a word are the other shapes of the
/// same mistake, and a list is what Mosaic writes for a form this build does
/// not read yet. Each is named in the load diagnostics the warning banner
/// draws from — by its key — and leaves that axis on the default.
#[test]
fn a_count_that_is_not_a_whole_number_above_zero_is_named_and_ignored() {
    let default = compose("");
    for (key, value) in [
        ("xTicks", "-3"),
        ("xTicks", "0"),
        ("xTicks", "2.5"),
        ("xTicks", "many"),
        ("xTicks", "[0, 50, 100]"),
        ("yTicks", "-3"),
    ] {
        let composed = compose(&format!("{key}: {value}"));
        let named = composed
            .diagnostics
            .advisory()
            .into_iter()
            .any(|d| d.wire_name == key && d.message.contains(key));
        assert!(
            named,
            "`{key}: {value}` should be named in the warning banner by its key; the load said \
             {:?}",
            composed.diagnostics.lines()
        );
        assert_eq!(
            painted_x_labels(&composed),
            painted_x_labels(&default),
            "`{key}: {value}` should leave the x axis at its default ticks"
        );
        assert_eq!(
            painted_y_labels(&composed),
            painted_y_labels(&default),
            "`{key}: {value}` should leave the y axis at its default ticks"
        );
    }
}

/// **A good count says nothing.** The warning is for the malformed value, so
/// a spec that asks for a count it can have carries no diagnostic about the
/// key — a banner that spoke on a valid request would be one nobody reads.
#[test]
fn a_valid_count_is_not_named() {
    let composed = compose("xTicks: 2\nyTicks: 10");
    let named: Vec<String> = composed
        .diagnostics
        .lines()
        .into_iter()
        .filter(|l| l.contains("Ticks"))
        .collect();
    assert!(
        named.is_empty(),
        "a valid tick count should not be named in the banner: {named:?}"
    );
}
