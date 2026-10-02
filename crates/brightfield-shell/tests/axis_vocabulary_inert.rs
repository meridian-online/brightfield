//! **An axis instruction that changes nothing on the axis it meets is named in
//! the warning banner, with the plot that carries it and the kind of axis, and
//! the plot draws as it does without the key.**
//!
//! Three instructions are read and then dropped by the axis they land on:
//!
//! - `xZero`, `xNice`, `yZero`, `yNice` move the ends of a linear axis and no
//!   other: a time, band, log or symlog axis ends where its data and its marks
//!   put it.
//! - `xTicks`, `yTicks` set the count a linear or a time axis steps toward. A
//!   band has a tick per category and a log or symlog axis a tick per decade.
//! - `xReverse`, `yReverse` turn an axis, but not on a plot with a map
//!   projection, whose x and y are the projection's planar units.
//!
//! Which axis a key meets is known only once the data has typed it, so each arm
//! composes a spec through the same load the window runs and reads
//! `Composed::diagnostics`, which is what the banner draws. "Draws as it does
//! without the key" is read off the painted scene: the spec that sets the key
//! paints the same paths as the spec without it.

use brightfield_shell::pipeline::{compose_spec, Composed, LiveDashboard};
use brightfield_shell::starts::{self, Opened};

/// What the load said that this card is about: a line that says the instruction
/// changes nothing where it lands.
const INERT: &str = "changes nothing on";

/// Eleven rows with one column of each kind a positional axis is typed from.
/// `X` and `Y` mark which columns the plot's dots take, `ATTRS` where the plot
/// attributes go.
const TEMPLATE: &str = r#"
data:
  d:
    query: "SELECT i + 1 AS n, TIMESTAMP '2024-03-01 14:00:00' + to_minutes(CAST(i AS INTEGER)) AS at, DATE '2024-03-01' + CAST(i AS INTEGER) AS day, 'g' || CAST(i AS VARCHAR) AS name, -120.0 + i AS lon, 30.0 + i AS lat FROM range(11) t(i)"
plot:
  - mark: dot
    data: { from: d }
    x: X
    y: Y
name: probe
width: 600
height: 300
ATTRS
"#;

/// Two plots stacked, so a line naming the wrong one is told apart. `TOP` and
/// `BOTTOM` mark each plot's attributes, `DEFAULTS` the `plotDefaults:` block.
const STACKED: &str = r#"
data:
  d:
    query: "SELECT i + 1 AS n, 'g' || CAST(i AS VARCHAR) AS name FROM range(5) t(i)"
plotDefaults:
  height: 150
DEFAULTS
vconcat:
  - plot:
      - { mark: dot, data: { from: d }, x: name, y: n }
    width: 600
TOP
  - plot:
      - { mark: dot, data: { from: d }, x: name, y: n }
    width: 600
BOTTOM
"#;

fn compose_str(spec: &str) -> Composed {
    LiveDashboard::load_str(spec, None)
        .expect("the spec loads")
        .present()
        .expect("the spec composes")
}

fn compose(x: &str, y: &str, attrs: &str) -> Composed {
    compose_str(
        &TEMPLATE
            .replace("    x: X", &format!("    x: {x}"))
            .replace("    y: Y", &format!("    y: {y}"))
            .replace("ATTRS", attrs),
    )
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

/// The lines that say an instruction changes nothing where it lands.
fn inert(composed: &Composed) -> Vec<String> {
    said(composed)
        .into_iter()
        .filter(|line| line.contains(INERT))
        .collect()
}

/// Every coordinate pair the scene's path stream holds, in draw order: the dots,
/// the axis lines, the ticks and the gridlines. `Encoding::path_data` is a flat
/// run of `f32` bits, two words per point.
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

/// Compose `x` against `y` under `axes` with and without `key`, and hold what the
/// banner and the scene say: the spec without the key is silent, the spec with it
/// draws as that one does, and one line names the key, the plot and the axis.
fn assert_named(x: &str, y: &str, axes: &str, key: &str, axis: &str) {
    let name = key.split(':').next().expect("a key").trim();
    let unset = compose(x, y, axes);
    assert!(
        said(&unset).is_empty(),
        "x `{x}`, y `{y}`, `{axes}`: the plot without `{name}` says nothing; got {:?}",
        said(&unset)
    );
    assert!(!points(&unset).is_empty(), "the plot paints");

    let asked = compose(x, y, &format!("{axes}{key}\n"));
    assert_eq!(
        points(&asked),
        points(&unset),
        "x `{x}`, y `{y}`, `{axes}`: `{key}` changes what is painted"
    );
    let lines = said(&asked);
    assert_eq!(lines.len(), 1, "`{key}` on {axis}: {lines:?}");
    assert!(
        lines[0].contains(&format!("`{name}`"))
            && lines[0].contains("root (`probe`)")
            && lines[0].contains(&format!("a {axis} axis")),
        "the line names the key, the plot and the axis; got {:?}",
        lines[0]
    );
}

/// Compose `x` against `y` under `axes` with `key` set, and hold that the banner
/// says nothing about it.
fn assert_silent(x: &str, y: &str, axes: &str, key: &str) {
    let lines = said(&compose(x, y, &format!("{axes}{key}\n")));
    assert!(
        lines.is_empty(),
        "x `{x}`, y `{y}`, `{axes}`: `{key}` lands where it acts and says nothing; got {lines:?}"
    );
}

// ---------------------------------------------------------------------------
// AC1 — the ends of an axis that is not linear
// ---------------------------------------------------------------------------

/// **`yNice` and `yZero` on a log, a symlog, a date or a category axis draw as
/// they do without the key, and the banner names the key, the plot and which
/// kind of axis it was. The x keys are the same on the x axis.**
#[test]
fn an_ends_instruction_on_an_axis_that_is_not_linear_is_named_and_the_plot_draws_without_it() {
    for (x, y, axes, key, axis) in [
        ("n", "n", "yScale: log\n", "yNice: true", "log"),
        ("n", "n", "yScale: log\n", "yZero: true", "log"),
        ("n", "n", "yScale: symlog\n", "yZero: true", "symlog"),
        ("n", "n", "xScale: log\n", "xNice: true", "log"),
        ("n", "at", "", "yNice: true", "date"),
        ("n", "at", "", "yZero: true", "date"),
        ("at", "n", "", "xNice: true", "date"),
        ("at", "n", "", "xZero: true", "date"),
        ("n", "day", "", "yZero: true", "date"),
        ("day", "n", "", "xZero: true", "date"),
        ("name", "n", "", "xNice: true", "category"),
        ("n", "name", "", "yZero: true", "category"),
    ] {
        assert_named(x, y, axes, key, axis);
    }
}

/// **On a linear axis the same keys say nothing, and a key set to `false` says
/// nothing on any axis.**
#[test]
fn an_ends_instruction_on_a_linear_axis_or_set_to_false_says_nothing() {
    for key in ["xNice: true", "xZero: true", "yNice: true", "yZero: true"] {
        assert_silent("n", "n", "", key);
    }
    for (x, y, axes, key) in [
        ("n", "n", "yScale: log\n", "yNice: false"),
        ("n", "n", "yScale: symlog\n", "yZero: false"),
        ("at", "n", "", "xNice: false"),
        ("day", "n", "", "xZero: false"),
        ("name", "n", "", "xNice: false"),
    ] {
        assert_silent(x, y, axes, key);
    }
}

// ---------------------------------------------------------------------------
// AC2 — a tick count on an axis that takes none
// ---------------------------------------------------------------------------

/// **`xTicks: 4` on a band, log or symlog axis draws as it does without the
/// key, and the banner names it as the ends are named. `yTicks` is the same on
/// the y axis.**
#[test]
fn a_tick_count_on_an_axis_that_takes_none_is_named_and_the_plot_draws_without_it() {
    for (x, y, axes, key, axis) in [
        ("name", "n", "", "xTicks: 4", "category"),
        ("day", "n", "", "xTicks: 4", "date"),
        ("n", "n", "xScale: log\n", "xTicks: 4", "log"),
        ("n", "n", "xScale: symlog\n", "xTicks: 4", "symlog"),
        ("n", "name", "", "yTicks: 4", "category"),
        ("n", "n", "yScale: log\n", "yTicks: 4", "log"),
    ] {
        assert_named(x, y, axes, key, axis);
    }
}

/// **On a linear or a time axis the same keys say nothing, and a count that is
/// no count is named for that and not also for landing on the wrong axis.**
#[test]
fn a_tick_count_on_an_axis_that_takes_one_says_nothing() {
    for (x, y, key) in [
        ("n", "n", "xTicks: 4"),
        ("n", "n", "yTicks: 4"),
        ("at", "n", "xTicks: 4"),
        ("n", "at", "yTicks: 4"),
    ] {
        assert_silent(x, y, "", key);
    }

    let lines = said(&compose("name", "n", "xTicks: 0\n"));
    assert_eq!(lines.len(), 1, "one line for a count of zero: {lines:?}");
    assert!(
        lines[0].contains("`xTicks`") && !lines[0].contains(INERT),
        "a count of zero is named as no count, not as a count the axis drops: {}",
        lines[0]
    );
}

// ---------------------------------------------------------------------------
// AC3 — a reversal on a plot with a map projection
// ---------------------------------------------------------------------------

/// **`xReverse: true` and `yReverse: true` on a plot with a map projection draw
/// as they do without the key, and the banner names each.**
#[test]
fn a_reversal_on_a_plot_with_a_projection_is_named_and_the_plot_draws_without_it() {
    let projected = "projectionType: equirectangular\n";
    let unset = compose("lon", "lat", projected);
    assert!(
        said(&unset).is_empty(),
        "the projected plot says nothing; got {:?}",
        said(&unset)
    );
    assert!(!points(&unset).is_empty(), "the projected plot paints");

    for key in ["xReverse", "yReverse"] {
        let asked = compose("lon", "lat", &format!("{projected}{key}: true\n"));
        assert_eq!(
            points(&asked),
            points(&unset),
            "`{key}` changes what a projected plot paints"
        );
        let lines = said(&asked);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains(&format!("`{key}`"))
                && lines[0].contains("root (`probe`)")
                && lines[0].contains("a plot with a map projection"),
            "the line names the key and the plot; got {:?}",
            lines[0]
        );
    }

    let both = compose(
        "lon",
        "lat",
        &format!("{projected}xReverse: true\nyReverse: true\n"),
    );
    assert_eq!(said(&both).len(), 2, "{:?}", said(&both));
}

/// **A reversal that lands where it acts says nothing: not on a plot with no
/// projection, and not when set to `false` on one with a projection.**
#[test]
fn a_reversal_that_lands_where_it_acts_says_nothing() {
    assert_silent("n", "n", "", "xReverse: true");
    assert_silent("n", "n", "", "yReverse: true");
    assert_silent(
        "lon",
        "lat",
        "projectionType: equirectangular\n",
        "xReverse: false",
    );
}

// ---------------------------------------------------------------------------
// Which plot, and once
// ---------------------------------------------------------------------------

/// **The line names the plot that sets the key and not its neighbour.**
#[test]
fn the_banner_names_the_plot_that_carries_it_not_its_neighbour() {
    for (top, bottom, carrier, other) in [
        ("    xTicks: 4", "", "root/vconcat[0]", "root/vconcat[1]"),
        ("", "    xTicks: 4", "root/vconcat[1]", "root/vconcat[0]"),
    ] {
        let lines = inert(&stacked("", top, bottom));
        assert!(
            lines.len() == 1 && lines[0].contains(carrier) && !lines[0].contains(other),
            "the line names {carrier} and not {other}; got {lines:?}"
        );
    }
}

/// **A key under `plotDefaults:` reaches each plot that inherits it, and each
/// such plot is named for it.**
#[test]
fn a_plot_defaults_key_is_named_at_each_plot_that_inherits_it() {
    let lines = inert(&stacked("  xTicks: 4", "", ""));
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines.iter().any(|l| l.contains("root/vconcat[0]")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("root/vconcat[1]")),
        "{lines:?}"
    );
}

/// A re-present rebuilds the composition, and the line must be said once, on
/// the first paint and on every repaint after it.
#[test]
fn a_repaint_says_it_once() {
    let spec = TEMPLATE
        .replace("    x: X", "    x: name")
        .replace("    y: Y", "    y: n")
        .replace("ATTRS", "xTicks: 4");
    let mut live = LiveDashboard::load_str(&spec, None).expect("the spec loads");
    let first = live.present().expect("composes");
    let second = live.present().expect("composes again");
    assert_eq!(inert(&first).len(), 1, "{:?}", said(&first));
    assert_eq!(said(&second), said(&first));
}

// ---------------------------------------------------------------------------
// AC5 — what the starts and the examples say
// ---------------------------------------------------------------------------

/// **Each shipped start that opens without a network, and each example, raises
/// none of these warnings: none sets a key where it does nothing.** What a
/// start or an example says about anything else is another test's.
#[test]
fn the_shipped_starts_and_the_examples_raise_no_inert_instruction() {
    let mut composed = 0usize;
    for start in starts::STARTS
        .iter()
        .filter(|s| !s.remote && s.spec.is_some())
    {
        let Ok(Opened::Charts(opened)) = starts::load(start.id) else {
            continue;
        };
        composed += 1;
        let lines = inert(&opened.composed);
        assert!(
            lines.is_empty(),
            "the start `{}` raises {lines:?}",
            start.id
        );
    }
    assert!(
        composed > 0,
        "at least one start composes without a network"
    );

    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut examples = 0usize;
    for entry in std::fs::read_dir(&dir).expect("examples/ exists").flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "yaml") {
            continue;
        }
        let Ok(composed) = compose_spec(path.to_str().expect("utf-8 path")) else {
            continue;
        };
        examples += 1;
        let lines = inert(&composed);
        assert!(
            lines.is_empty(),
            "{} raises {lines:?}",
            path.file_name().unwrap().to_string_lossy()
        );
    }
    assert!(examples >= 15, "only {examples} examples composed");
}

/// **A dashboard generated from a data file raises none of these warnings.**
#[test]
fn a_generated_dashboard_raises_no_inert_instruction() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv");
    let opened = brightfield_shell::data_file::open(path.to_str().expect("utf-8 path"))
        .expect("the file opens");
    let lines = inert(&opened.composed);
    assert!(lines.is_empty(), "{lines:?}");
    assert!(!points(&opened.composed).is_empty(), "the dashboard paints");
}
