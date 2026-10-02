//! **An axis attribute brightfield does not read is named in the warning
//! banner, with the plot that carries it, and the plot still draws.**
//!
//! A plot's attributes are an open bag, so `xTickRotate: 45` was carried and
//! dropped without a word: the analyst saw unrotated ticks and could not tell
//! a typing mistake from a gap in brightfield.
//!
//! Each arm composes a spec through the same load the window runs and reads
//! `Composed::diagnostics`, which is what the banner draws, one line per
//! diagnostic. "Draws" is read off the painted scene: a spec that sets the
//! unread key paints the same points as the spec without it, so the key is
//! neither drawn through nor allowed to stop the draw.

use brightfield_shell::pipeline::{Composed, LiveDashboard};

/// Three points on a 0 to 100 domain. `ATTRS` marks where the plot attributes
/// go; the template's own attributes are all ones brightfield reads or that are
/// no axis attribute, so it says nothing on its own.
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
name: rotated
width: 600
height: 300
xTicks: 2
ATTRS
";

/// Two plots stacked, so a line naming the wrong one is told apart. `TOP` and
/// `BOTTOM` mark each plot's attributes, `DEFAULTS` the `plotDefaults:` block.
const STACKED: &str = r"
data:
  pts:
    - { a: 0,   b: 0 }
    - { a: 100, b: 100 }
plotDefaults:
  height: 150
DEFAULTS
vconcat:
  - plot:
      - { mark: dot, data: { from: pts }, x: a, y: b }
    width: 600
TOP
  - plot:
      - { mark: dot, data: { from: pts }, x: a, y: b }
    width: 600
BOTTOM
";

fn compose_str(spec: &str) -> Composed {
    LiveDashboard::load_str(spec, None)
        .expect("the spec loads")
        .present()
        .expect("the spec composes")
}

fn compose(attrs: &str) -> Composed {
    compose_str(&TEMPLATE.replace("ATTRS", attrs))
}

fn stacked(defaults: &str, top: &str, bottom: &str) -> Composed {
    compose_str(
        &STACKED
            .replace("DEFAULTS", defaults)
            .replace("TOP", top)
            .replace("BOTTOM", bottom),
    )
}

/// What the load said, one line per diagnostic: what the warning banner draws.
fn said(composed: &Composed) -> Vec<String> {
    composed.diagnostics.lines()
}

/// Every coordinate pair the scene's path stream holds, in draw order.
/// `Encoding::path_data` is a flat run of `f32` bits, two words per point.
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

/// **`xTickRotate: 45` draws, and the banner names `xTickRotate` and the plot
/// that carries it, by path and by its `name:`.**
#[test]
fn a_tick_rotation_draws_and_the_banner_names_it_and_its_plot() {
    let unset = compose("");
    assert!(
        said(&unset).is_empty(),
        "the template says nothing; got {:?}",
        said(&unset)
    );
    assert!(!points(&unset).is_empty(), "the template paints");

    let rotated = compose("xTickRotate: 45");
    assert_eq!(
        points(&rotated),
        points(&unset),
        "the plot draws as it does without the key"
    );
    let lines = said(&rotated);
    assert_eq!(lines.len(), 1, "one line for one unread key; got {lines:?}");
    assert!(
        lines[0].contains("`xTickRotate`") && lines[0].contains("root (`rotated`)"),
        "the line names the key and the plot, by path and name; got {:?}",
        lines[0]
    );
}

/// **A facet axis's attribute draws, and the banner names it and its plot: the
/// schema declares `fxLabel` and `fyTickFormat`, and no resolver reads either.
/// A `null` is named as an `x` or `y` name set to `null` is, and a name that
/// starts as a facet axis's does and is not in the schema says nothing.**
#[test]
fn a_facet_axis_attribute_draws_and_the_banner_names_it_and_its_plot() {
    let unset = compose("");
    for attrs in ["fxLabel: Region", "fyTickFormat: '%b'", "fxLabel: null"] {
        let key = attrs.split(':').next().expect("a key");
        let asked = compose(attrs);
        assert_eq!(
            points(&asked),
            points(&unset),
            "`{attrs}`: the plot draws as it does without the key"
        );
        let lines = said(&asked);
        assert_eq!(lines.len(), 1, "one line for `{attrs}`; got {lines:?}");
        assert!(
            lines[0].contains(&format!("`{key}`")) && lines[0].contains("root (`rotated`)"),
            "the line names the key and the plot; got {:?}",
            lines[0]
        );
    }
    let lines = said(&compose("yAxis: null"));
    assert!(
        lines.len() == 1 && lines[0].contains("`yAxis`"),
        "an `x` or `y` name set to `null` is named today; got {lines:?}"
    );
    let lines = said(&compose("fxFlavour: 1\nfacetLabel: x"));
    assert!(
        lines.is_empty(),
        "a name the schema does not declare says nothing; got {lines:?}"
    );
}

/// **The line names the plot that sets the key and not its neighbour.**
#[test]
fn the_banner_names_the_plot_that_carries_it_not_its_neighbour() {
    for (top, bottom, carrier, other) in [
        (
            "    xTickRotate: 45",
            "",
            "root/vconcat[0]",
            "root/vconcat[1]",
        ),
        (
            "",
            "    xTickRotate: 45",
            "root/vconcat[1]",
            "root/vconcat[0]",
        ),
    ] {
        let lines = said(&stacked("", top, bottom));
        assert!(
            lines.len() == 1 && lines[0].contains(carrier) && !lines[0].contains(other),
            "the line names {carrier} and not {other}; got {lines:?}"
        );
    }
}

/// **A key under `plotDefaults:` is named once, there, and not again at each
/// plot that inherits it; a plot that sets it too is named for its own.**
#[test]
fn a_plot_defaults_key_is_named_once_where_it_is_written() {
    let lines = said(&stacked("  yAxis: right", "", ""));
    assert!(
        lines.len() == 1 && lines[0].contains("`plotDefaults`") && lines[0].contains("`yAxis`"),
        "one line, naming `plotDefaults` and the key; got {lines:?}"
    );
    assert!(
        !lines[0].contains("vconcat["),
        "no plot is named for an inherited key: {lines:?}"
    );

    let lines = said(&stacked("  yAxis: right", "", "    yAxis: left"));
    assert_eq!(
        lines.len(),
        2,
        "the default and the plot's own; got {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("`plotDefaults`")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("root/vconcat[1]")),
        "{lines:?}"
    );
}

/// **An axis attribute brightfield reads says nothing.**
#[test]
fn an_axis_attribute_brightfield_reads_says_nothing() {
    for asked in [
        "xLabel: Width",
        "yLabel: null",
        "xGrid: false",
        "grid: true",
        "yTicks: 4",
        "xTickFormat: '.1f'",
        "xReverse: true",
        "yZero: true",
        "xNice: false",
        "xDomain: Fixed",
        "yScale: linear",
        "xInset: 4",
        "yInsetTop: 4",
    ] {
        let lines = said(&compose(asked));
        assert!(
            lines.is_empty(),
            "`{asked}` is read and draws in silence; got {lines:?}"
        );
    }
}
