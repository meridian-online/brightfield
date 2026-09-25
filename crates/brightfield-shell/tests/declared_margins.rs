//! **A spec's declared margins reach the layout its plot is drawn in.**
//!
//! `marginTop`, `marginRight`, `marginBottom`, `marginLeft` and the `margin`
//! shorthand were parsed and then dropped: the pipeline built every plot's
//! margins from `Margins::default()` grown by its titles, so a plot declaring
//! `marginLeft: 0` drew its data area 40 px in, exactly where a plot declaring
//! nothing drew it.
//!
//! Driven through [`compose_spec_str`], the real composition, and read off
//! [`PlotHandle::layout`] — the layout the plot's scales, axes and gesture
//! hit-testing all live in. The plots suppress their axis titles
//! (`xLabel: null`, `yLabel: null`) wherever the question is where the
//! **declared** value lands, because a derived title grows the margin by a
//! band and this file would otherwise be asserting the band's size at the same
//! time; the one test that wants the growth asks for it by name.

use brightfield_render::title::TITLE_BAND;
use brightfield_shell::pipeline::{compose_spec_str, Composed};

/// Observable Plot's defaults, which `Margins::default` is and which a plot
/// that declares nothing must still get.
const DEFAULT_TOP: f64 = 20.0;
const DEFAULT_RIGHT: f64 = 20.0;
const DEFAULT_BOTTOM: f64 = 30.0;
const DEFAULT_LEFT: f64 = 40.0;

/// One dot plot's YAML list item, with `attrs` as the plot's own attributes —
/// siblings of `plot:`, at the key's indent.
fn plot(attrs: &[&str]) -> String {
    let mut out = String::from(
        "  - plot:\n    - mark: dot\n      data: { from: t }\n      x: x\n      y: y\n    \
         width: 400\n    height: 300\n",
    );
    for attr in attrs {
        out.push_str("    ");
        out.push_str(attr);
        out.push('\n');
    }
    out
}

/// A spec of `plots` side by side over three inline rows.
fn spec(plots: &[String]) -> String {
    let mut out = String::from(
        "data:\n  t:\n    - { x: 1, y: 2 }\n    - { x: 2, y: 4 }\n    - { x: 3, y: 3 }\nhconcat:\n",
    );
    for p in plots {
        out.push_str(p);
    }
    out
}

/// The composition of `plots`, insisting every one of them was placed.
fn compose(plots: &[String]) -> Composed {
    let source = spec(plots);
    let composed = compose_spec_str(&source, None)
        .unwrap_or_else(|e| panic!("the spec composes: {e}\n{source}"));
    assert_eq!(
        composed.plots.len(),
        plots.len(),
        "every plot the spec declares is placed, or a test below is reading the wrong one"
    );
    composed
}

/// One plot's four margins, as `(top, right, bottom, left)`.
fn margins_of(composed: &Composed, index: usize) -> (f64, f64, f64, f64) {
    let m = composed.plots[index].layout.margins();
    (m.top, m.right, m.bottom, m.left)
}

/// The two title-suppressing attributes, so a margin is read without a title
/// band on top of it.
const NO_TITLES: [&str; 2] = ["xLabel: null", "yLabel: null"];

fn with_no_titles(extra: &[&str]) -> String {
    let mut attrs: Vec<&str> = NO_TITLES.to_vec();
    attrs.extend_from_slice(extra);
    plot(&attrs)
}

/// **The claim.** A spec declaring `marginLeft: 0` draws its data area at the
/// left edge of the plot, not at the default 40 px; the other three sides,
/// which it did not declare, keep their defaults.
///
/// The control is the same plot declaring nothing: it must land at the default,
/// or the first assertion is measuring a harness that draws at 0 whatever the
/// spec says.
#[test]
fn a_declared_margin_left_of_zero_starts_the_data_area_at_the_edge() {
    let declared = compose(&[with_no_titles(&["marginLeft: 0"])]);
    let control = compose(&[with_no_titles(&[])]);

    assert!(
        (declared.plots[0].layout.plot_x_start() - 0.0).abs() < f64::EPSILON,
        "marginLeft: 0 puts the data area at x = 0, and it starts at {}",
        declared.plots[0].layout.plot_x_start()
    );
    assert_eq!(
        margins_of(&declared, 0),
        (DEFAULT_TOP, DEFAULT_RIGHT, DEFAULT_BOTTOM, 0.0),
        "only the declared side moves"
    );
    assert!(
        (control.plots[0].layout.plot_x_start() - DEFAULT_LEFT).abs() < f64::EPSILON,
        "the plot declaring nothing starts at the default {DEFAULT_LEFT}, and it starts at {}",
        control.plots[0].layout.plot_x_start()
    );
}

/// Each key lands on its own side. Four different values, so a pair of sides
/// swapped on the way through cannot pass.
#[test]
fn each_declared_side_lands_on_its_own_edge() {
    let composed = compose(&[with_no_titles(&[
        "marginTop: 3",
        "marginRight: 5",
        "marginBottom: 7",
        "marginLeft: 9",
    ])]);
    assert_eq!(margins_of(&composed, 0), (3.0, 5.0, 7.0, 9.0));

    // The data area is what is left inside them.
    let l = composed.plots[0].layout;
    assert!((l.plot_x_start() - 9.0).abs() < f64::EPSILON);
    assert!((l.plot_x_end() - (l.width - 5.0)).abs() < f64::EPSILON);
    assert!((l.plot_y_start() - 3.0).abs() < f64::EPSILON);
    assert!((l.plot_y_end() - (l.height - 7.0)).abs() < f64::EPSILON);
}

/// The `margin` shorthand sets all four sides, and a side key beats it.
#[test]
fn the_margin_shorthand_sets_every_side_and_a_side_key_beats_it() {
    let composed = compose(&[with_no_titles(&["margin: 0", "marginBottom: 12"])]);
    assert_eq!(margins_of(&composed, 0), (0.0, 0.0, 12.0, 0.0));
}

/// **A declared margin is a floor, not a replacement.** An axis title still
/// needs its room, so each present title grows the declared side by one band:
/// the y title the left, the x title the bottom, the plot title the top. The
/// right margin has no title and does not grow.
#[test]
fn titles_grow_the_declared_margins_and_do_not_replace_them() {
    let composed = compose(&[plot(&[
        "xLabel: Depth",
        "yLabel: Reading",
        "title: A plot",
        "marginTop: 4",
        "marginRight: 6",
        "marginBottom: 8",
        "marginLeft: 10",
    ])]);
    assert_eq!(
        margins_of(&composed, 0),
        (4.0 + TITLE_BAND, 6.0, 8.0 + TITLE_BAND, 10.0 + TITLE_BAND)
    );
}

/// The reading is per plot: each plot takes the margins **it** declared, and a
/// plot that declares none keeps the defaults beside one that does.
#[test]
fn each_plot_takes_its_own_declared_margins() {
    let composed = compose(&[
        with_no_titles(&["marginLeft: 0"]),
        with_no_titles(&["marginLeft: 60"]),
        with_no_titles(&[]),
    ]);
    assert_eq!(margins_of(&composed, 0).3, 0.0);
    assert_eq!(margins_of(&composed, 1).3, 60.0);
    assert_eq!(margins_of(&composed, 2).3, DEFAULT_LEFT);
}
