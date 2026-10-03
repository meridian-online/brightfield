//! **A colour legend under the plot it is for, in a `vconcat`, is laid out as a
//! band under that plot: [`BELOW_LEGEND_HEIGHT`] high, as wide as the plot, with
//! the plot taking the height that is left. A standalone legend placed any other
//! way keeps the rect it always had.**
//!
//! The arrangement is decided in one place (`plot_above_legend`), and these
//! tests read it through the layout's public answers: [`compute_layout`] for
//! where things are, [`below_legends`] for which legends are under a plot, and
//! [`placed_plots`] for the plot's height. The shell draws into the band and
//! its tests are `crates/brightfield-shell/tests/legend_below.rs`.

use brightfield_spec::layout::{
    below_legends, compute_layout, placed_legends, placed_plots, LayoutNode, Rect,
    BELOW_LEGEND_HEIGHT, DEFAULT_LEGEND_HEIGHT, DEFAULT_LEGEND_WIDTH,
};
use brightfield_spec::{parse_spec, Format, Spec};

fn spec(source: &str) -> Spec {
    parse_spec(source, Format::Yaml)
        .unwrap_or_else(|e| panic!("the spec parses: {e}\n{source}"))
        .spec
}

/// A plot named `scatter`, 300 by 200, coloured by `grp`, as a concat item written
/// at `indent` spaces.
fn plot(indent: usize) -> String {
    let pad = " ".repeat(indent);
    format!(
        "{pad}- plot:\n{pad}    - {{ mark: dot, data: {{ from: t }}, x: x, y: y, fill: grp }}\n{pad}  name: scatter\n{pad}  width: 300\n{pad}  height: 200\n"
    )
}

const DATA: &str = "data:\n  t:\n    - { x: 1, y: 2, grp: a }\n";

fn vconcat_of(items: &str) -> Spec {
    spec(&format!("{DATA}vconcat:\n{items}"))
}

const VIEWPORT: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 500.0,
    height: 400.0,
};

/// **The band's place.** A `vconcat` of the named plot and a colour legend `for`
/// it puts the legend in a band [`BELOW_LEGEND_HEIGHT`] high, directly under the
/// plot and as wide as it, and says which plot it is under.
#[test]
fn a_legend_under_its_plot_is_a_band_as_wide_as_the_plot_and_44_high() {
    let spec = vconcat_of(&format!("{}  - legend: color\n    for: scatter\n", plot(2)));
    let below = below_legends(&spec, Rect::zero());
    assert_eq!(below.len(), 1, "one legend is under its plot: {below:?}");
    let band = &below[0];
    assert_eq!(band.plot_path, "root/vconcat[0]");
    assert_eq!(band.legend_path, "root/vconcat[1]");
    assert_eq!(
        band.rect,
        Rect::new(0.0, 200.0, 300.0, 44.0),
        "directly under the 300 by 200 plot, as wide as it"
    );
    assert_eq!(BELOW_LEGEND_HEIGHT, 44.0, "the band's height is the design's");
    // And it is the rect the layout reserved, not a second answer.
    let placed = placed_legends(&spec, Rect::zero());
    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].rect, band.rect);
}

/// **The plot takes the remainder.** Offered a height, the plot above the band
/// is laid out in what is left after it, and the band ends where the offer does.
#[test]
fn the_plot_takes_the_height_the_band_leaves() {
    let spec = vconcat_of(&format!("{}  - legend: color\n    for: scatter\n", plot(2)));
    let plots = placed_plots(&spec, VIEWPORT);
    assert_eq!(plots.len(), 1);
    assert_eq!(
        plots[0].rect.height,
        VIEWPORT.height - BELOW_LEGEND_HEIGHT,
        "the plot takes the 400 it was offered less the band"
    );
    let below = below_legends(&spec, VIEWPORT);
    assert_eq!(below[0].rect.y, plots[0].rect.height, "under the plot");
    assert_eq!(
        below[0].rect.y + below[0].rect.height,
        VIEWPORT.height,
        "the band ends where the offer does"
    );
    assert_eq!(
        below[0].rect.width, plots[0].rect.width,
        "as wide as the plot, which fills the offered width"
    );
}

/// **Placed any other way, it is not under a plot.** In an `hconcat`; naming a
/// plot that is not its sibling; before the plot it names; for a channel that is
/// not colour; with a `for:` that is a `$param`; and with no `for:` — each keeps
/// the standalone legend's own rect and is not in [`below_legends`].
#[test]
fn a_legend_placed_any_other_way_keeps_the_rect_it_always_had() {
    let cases: [(&str, String); 6] = [
        (
            "in an hconcat",
            format!("{DATA}hconcat:\n{}  - legend: color\n    for: scatter\n", plot(2)),
        ),
        (
            "naming a plot that is not its sibling",
            format!(
                "{DATA}vconcat:\n  - hconcat:\n{}  - legend: color\n    for: scatter\n",
                plot(6)
            ),
        ),
        (
            "before the plot it names",
            format!("{DATA}vconcat:\n  - legend: color\n    for: scatter\n{}", plot(2)),
        ),
        (
            "for another channel",
            format!("{DATA}vconcat:\n{}  - legend: opacity\n    for: scatter\n", plot(2)),
        ),
        (
            "with a $param for:",
            format!(
                "params:\n  which: scatter\n{DATA}vconcat:\n{}  - legend: color\n    for: $which\n",
                plot(2)
            ),
        ),
        (
            "with no for:",
            format!("{DATA}vconcat:\n{}  - legend: color\n", plot(2)),
        ),
    ];
    for (what, source) in cases {
        let spec = spec(&source);
        assert_eq!(
            below_legends(&spec, Rect::zero()),
            vec![],
            "a legend {what} was placed under a plot"
        );
        let placed = placed_legends(&spec, Rect::zero());
        assert_eq!(placed.len(), 1, "a legend {what}: one standalone legend");
        assert_eq!(
            (placed[0].rect.width, placed[0].rect.height),
            (DEFAULT_LEGEND_WIDTH, DEFAULT_LEGEND_HEIGHT),
            "a legend {what} was given a band's rect"
        );
    }
}

/// **A column inside a column measures the band too.** The outer `vconcat` shares
/// its height between its items by their intrinsic heights, and an inner `vconcat`
/// holding a legend under its plot measures the plot and the band, not the
/// plot and the default legend: with the band, the inner column's share is
/// 244/344 of the offer.
#[test]
fn a_column_holding_a_band_is_measured_with_the_band() {
    let spec = spec(&format!(
        "{DATA}vconcat:\n  - vconcat:\n{}      - legend: color\n        for: scatter\n  - plot:\n      - {{ mark: dot, data: {{ from: t }}, x: x, y: y }}\n    width: 300\n    height: 100\n",
        plot(6)
    ));
    let offer = Rect::new(0.0, 0.0, 300.0, 544.0);
    let tree = compute_layout(&spec, offer).expect("a layout");
    let LayoutNode::VConcat { children, .. } = tree else {
        panic!("the root is a vconcat");
    };
    let inner = children[0].rect().height;
    let expected = 544.0 * (200.0 + BELOW_LEGEND_HEIGHT) / (200.0 + BELOW_LEGEND_HEIGHT + 100.0);
    assert!(
        (inner - expected).abs() < 1e-6,
        "the inner column took {inner} of 544, not the {expected} its plot and band weigh"
    );
}
