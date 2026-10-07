//! **A spec that sets `xDomain`, `yDomain` or `xyDomain` to two numbers fixes the
//! ends of the axis it names.**
//!
//! `Fixed` was the one value read, so a plot drew each axis from its rows'
//! lowest value to their highest whatever ends the file wrote, and an analyst who
//! wanted every chart in a report on one axis, 0 to 100, could not have it.
//!
//! Assertions read two things, as the zero and round-ends file beside this one
//! does. The domain of the scale the plot was composed against says what the axis
//! ends are, and the path stream of the painted scene says the marks were drawn
//! against that domain: a row past an end puts no mark inside the data area,
//! and a row inside one is where the fixed ends put it. What the page says is
//! read from `Composed::diagnostics`, which is what the warning banner draws.
//!
//! Each arm that sets a key is paired with the same spec without it, so a
//! fixture whose default already drew the ends asked for would pass without the
//! key being read.

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_shell::pipeline::{Composed, LiveDashboard};
use brightfield_spec::analysis::ComponentPath;
use brightfield_sql::ir::ScalarValue;

// ---------------------------------------------------------------------------
// The fixtures
// ---------------------------------------------------------------------------

/// Four dots, three of them inside 0 to 100 on x and the fourth, at x = 150,
/// past it. x runs 4 to 150 and y 3 to 97 as the rows stand, so a plot with no
/// domain key draws neither axis from 0 to 100, and a nice or a zero request has
/// something to move on each. `PARAMS` marks where `params:` go and `ATTRS`
/// where the plot attributes go, so the arms differ by those lines.
const DOTS: &str = r"
PARAMS
data:
  pts:
    - { a: 4,   b: 3 }
    - { a: 50,  b: 60 }
    - { a: 96,  b: 97 }
    - { a: 150, b: 60 }
plot:
  - mark: dot
    data: { from: pts }
    x: a
    y: b
name: probe
width: 600
height: 300
ATTRS
";

/// The same plot without its fourth row: the rows that fall inside 0 to 100.
const DOTS_WITHIN: &str = r"
data:
  pts:
    - { a: 4,   b: 3 }
    - { a: 50,  b: 60 }
    - { a: 96,  b: 97 }
plot:
  - mark: dot
    data: { from: pts }
    x: a
    y: b
name: probe
width: 600
height: 300
ATTRS
";

/// Eleven rows with one column of each kind a positional axis is typed from:
/// `X` and `Y` mark which columns the dots take.
const TYPED: &str = r#"
data:
  d:
    query: "SELECT i + 1 AS n, TIMESTAMP '2024-03-01 14:00:00' + to_minutes(CAST(i AS INTEGER)) AS at, 'g' || CAST(i AS VARCHAR) AS name FROM range(11) t(i)"
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

/// Two plots stacked, so a line naming the wrong one is told apart. `DEFAULTS`
/// marks the `plotDefaults:` block and `TOP` and `BOTTOM` each plot's attributes.
const STACKED: &str = r"
data:
  pts:
    - { a: 4,  b: 3 }
    - { a: 96, b: 97 }
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

fn dots(attrs: &str) -> Composed {
    compose_str(&DOTS.replace("PARAMS", "").replace("ATTRS", attrs))
}

fn dots_with_params(params: &str, attrs: &str) -> Composed {
    compose_str(&DOTS.replace("PARAMS", params).replace("ATTRS", attrs))
}

fn typed(x: &str, y: &str, attrs: &str) -> Composed {
    compose_str(
        &TYPED
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

// ---------------------------------------------------------------------------
// Reading a drawn scale, and where a dot is painted
// ---------------------------------------------------------------------------

/// The drawn `(min, max)` of plot `plot`'s `channel`, insisting it is a linear
/// scale.
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

fn x_domain(composed: &Composed) -> (f64, f64) {
    domain(composed, 0, Channel::X)
}

fn y_domain(composed: &Composed) -> (f64, f64) {
    domain(composed, 0, Channel::Y)
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

/// The pixel span of the first plot's `channel`, low pixel first.
fn pixel_span(composed: &Composed, channel: Channel) -> (f64, f64) {
    let scale = composed.plots[0]
        .scales
        .get(channel)
        .expect("the plot drew this channel");
    let (a, b) = (scale.range_start(), scale.range_end());
    (a.min(b), a.max(b))
}

/// The path points that lie strictly inside the first plot's data area: not on
/// its edge, where the axis lines and the ends of each gridline sit, and not
/// outside it.
fn inside_the_data_area(composed: &Composed) -> Vec<(u32, u32)> {
    let (x0, x1) = pixel_span(composed, Channel::X);
    let (y0, y1) = pixel_span(composed, Channel::Y);
    points(composed)
        .into_iter()
        .filter(|&(x, y)| x > x0 + 0.5 && x < x1 - 0.5 && y > y0 + 0.5 && y < y1 - 0.5)
        // The f32 bits stand for the f64, which compares exactly.
        .map(|(x, y)| ((x as f32).to_bits(), (y as f32).to_bits()))
        .collect()
}

/// Where `value` lands on the first plot's x axis drawn over `(lo, hi)`.
fn x_pixel_at((lo, hi): (f64, f64), composed: &Composed, value: f64) -> f64 {
    let scale = composed.plots[0]
        .scales
        .get(Channel::X)
        .expect("the plot drew x");
    scale.range_start() + (value - lo) / (hi - lo) * (scale.range_end() - scale.range_start())
}

/// The x of the centre of the dot painted near `(x, y)`: the mean x of the path
/// points within twelve pixels of it.
fn painted_dot_x(composed: &Composed, near: (f64, f64)) -> f64 {
    let hits: Vec<f64> = points(composed)
        .into_iter()
        .filter(|p| (p.0 - near.0).hypot(p.1 - near.1) < 12.0)
        .map(|p| p.0)
        .collect();
    assert!(
        !hits.is_empty(),
        "no dot is painted near ({}, {})",
        near.0,
        near.1
    );
    hits.iter().sum::<f64>() / hits.len() as f64
}

/// The y pixel of the row at `b = value`, on the first plot's y axis, which the
/// arms that call it leave at the data's own.
fn y_pixel_of(composed: &Composed, value: f64) -> f64 {
    let (lo, hi) = y_domain(composed);
    let scale = composed.plots[0]
        .scales
        .get(Channel::Y)
        .expect("the plot drew y");
    scale.range_start() + (value - lo) / (hi - lo) * (scale.range_end() - scale.range_start())
}

// ---------------------------------------------------------------------------
// AC1 — xDomain
// ---------------------------------------------------------------------------

/// **With `xDomain: [0, 100]`, a plot whose rows run from 4 to 150 draws an x
/// axis from 0 to 100, a dot at 50 sits where that axis puts it, and the row at
/// 150 draws no mark inside the data area.** The same plot without the key draws
/// the rows' own 4 to 150.
#[test]
fn x_domain_of_two_numbers_fixes_the_x_axis_and_a_row_past_its_end_draws_nothing_inside() {
    let unset = dots("");
    assert_eq!(
        x_domain(&unset),
        (4.0, 150.0),
        "fixture check: without the key the axis runs over the rows"
    );

    let asked = dots("xDomain: [0, 100]");
    assert_eq!(
        x_domain(&asked),
        (0.0, 100.0),
        "the axis runs from 0 to 100 whatever the rows span"
    );
    assert_eq!(
        y_domain(&asked),
        y_domain(&unset),
        "the key names x, so y keeps the rows' own ends"
    );
    assert!(
        said(&asked).is_empty(),
        "two numbers say nothing: {:?}",
        said(&asked)
    );

    // A dot at 50 is painted where 0 to 100 puts it, and not where 4 to 150 does.
    let near = (
        x_pixel_at((0.0, 100.0), &asked, 50.0),
        y_pixel_of(&asked, 60.0),
    );
    let at = painted_dot_x(&asked, near);
    assert!(
        (at - near.0).abs() < 1.0,
        "the dot at 50 is painted at {at}, and 0 to 100 puts it at {}",
        near.0
    );
    assert!(
        (at - x_pixel_at((4.0, 150.0), &unset, 50.0)).abs() > 20.0,
        "fixture check: the rows' own ends put the dot somewhere else"
    );

    // The row at 150 is past the end: no mark it paints is inside the data area,
    // so the plot paints inside it what the same rows without that row do.
    let within = compose_str(&DOTS_WITHIN.replace("ATTRS", "xDomain: [0, 100]"));
    assert_eq!(
        y_domain(&within),
        y_domain(&asked),
        "fixture check: the rows kept share the plot's y ends"
    );
    assert_eq!(
        inside_the_data_area(&asked),
        inside_the_data_area(&within),
        "a row past an end paints a mark inside the data area"
    );
}

// ---------------------------------------------------------------------------
// AC2 — yDomain and xyDomain
// ---------------------------------------------------------------------------

/// **`yDomain: [a, b]` does the same for y, and `xyDomain: [a, b]` sets both
/// axes to those ends.** A key for one axis leaves the other where the rows put
/// it, and an axis's own key wins over `xyDomain` on that axis.
#[test]
fn y_domain_fixes_y_and_xy_domain_fixes_both_axes() {
    let unset = dots("");
    let y_only = dots("yDomain: [0, 200]");
    assert_eq!(y_domain(&y_only), (0.0, 200.0));
    assert_eq!(
        x_domain(&y_only),
        x_domain(&unset),
        "the key names y, so x keeps the rows' own ends"
    );
    assert_ne!(y_domain(&unset), (0.0, 200.0), "fixture check");

    let both = dots("xyDomain: [0, 100]");
    assert_eq!(x_domain(&both), (0.0, 100.0));
    assert_eq!(y_domain(&both), (0.0, 100.0));
    assert!(said(&both).is_empty(), "{:?}", said(&both));

    let own_wins = dots("xyDomain: [0, 100]\nyDomain: [10, 50]");
    assert_eq!(x_domain(&own_wins), (0.0, 100.0));
    assert_eq!(
        y_domain(&own_wins),
        (10.0, 50.0),
        "an axis's own key is the one that fixes it"
    );
    assert!(said(&own_wins).is_empty(), "{:?}", said(&own_wins));
}

// ---------------------------------------------------------------------------
// AC3 — what draws as it did
// ---------------------------------------------------------------------------

/// **`xDomain: Fixed` draws as it does today, a plot with no domain key draws as
/// it does today, and `xyDomain: Fixed` is still the key this build does not
/// read.** The committed baselines are the snapshot targets; this holds the same
/// at the level of the scene.
#[test]
fn fixed_and_no_key_draw_as_they_did_and_xy_domain_fixed_is_still_named() {
    let unset = dots("");
    let fixed = dots("xDomain: Fixed\nyDomain: Fixed");
    assert_eq!(x_domain(&fixed), (4.0, 150.0));
    assert_eq!(y_domain(&fixed), (3.0, 97.0));
    assert_eq!(
        points(&fixed),
        points(&unset),
        "Fixed changes what is painted"
    );
    assert!(said(&fixed).is_empty(), "{:?}", said(&fixed));
    assert!(said(&unset).is_empty(), "{:?}", said(&unset));

    let xy = dots("xyDomain: Fixed");
    assert_eq!(points(&xy), points(&unset), "xyDomain: Fixed is read");
    let lines = said(&xy);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("`xyDomain`") && lines[0].contains("root (`probe`)"),
        "the page names the key this build does not read; got {:?}",
        lines[0]
    );
}

/// **A pair of ends is read again after a brush, and a filter that narrows the
/// rows leaves it where the file wrote it.** The plot without the key draws the
/// narrowed rows' own ends.
#[test]
fn a_pair_of_ends_holds_through_a_brush() {
    let pinned = BRUSHED.replace("ATTRS", "    yDomain: [0, 120]\n");
    let unpinned = BRUSHED.replace("ATTRS", "");
    for (label, source, holds) in [("pinned", pinned, true), ("unpinned", unpinned, false)] {
        let mut live = LiveDashboard::load_str(&source, None).expect("the spec loads live");
        let first = live.present().expect("first composite");
        let path = first.plots[0].path.clone();
        let narrowed = live
            .apply(Interaction::Select {
                name: "brush".to_string(),
                contributor: ComponentPath(path),
                predicate: SqlPredicate::Interval {
                    column: "a".to_string(),
                    lo: ScalarValue::Float(15.0),
                    hi: ScalarValue::Float(85.0),
                    meta: None,
                },
            })
            .expect("the brush re-composites");
        let after = domain(&narrowed, 1, Channel::Y);
        if holds {
            assert_eq!(
                domain(&first, 1, Channel::Y),
                (0.0, 120.0),
                "{label}: the first composition draws the ends the file wrote"
            );
            assert_eq!(after, (0.0, 120.0), "{label}: the brush moved the ends");
        } else {
            assert_eq!(
                after,
                (22.0, 71.0),
                "{label}: an unpinned axis draws the narrowed rows' own ends"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// AC4 — a value that is not two numbers
// ---------------------------------------------------------------------------

/// **A domain that is not two numbers low first draws as if the key were
/// absent, and the page's warning names the key and the value.** A pair high
/// first is refused rather than read as a reversal, since `xReverse` is the key
/// for that.
#[test]
fn a_domain_that_is_not_two_numbers_low_first_draws_without_the_key_and_is_named() {
    let unset = dots("");
    for key in ["xDomain", "yDomain", "xyDomain"] {
        for value in [
            "[100, 0]",
            "[0, \"high\"]",
            "[0]",
            "[5, 5]",
            "[0, 50, 100]",
            "wide",
            "50",
        ] {
            let asked = dots(&format!("{key}: {value}"));
            assert_eq!(
                points(&asked),
                points(&unset),
                "`{key}: {value}` changes what is painted"
            );
            let lines = said(&asked);
            assert_eq!(
                lines.len(),
                1,
                "`{key}: {value}` is named once, however many axes it landed on: {lines:?}"
            );
            let value_as_written = if value == "wide" { "\"wide\"" } else { value };
            assert!(
                lines[0].contains(&format!("`{key}: {value_as_written}`"))
                    && lines[0].contains("root (`probe`)")
                    && lines[0].contains("two numbers"),
                "the line names the key, the value and the plot; got {:?}",
                lines[0]
            );
        }
    }
}

/// **A `$param` holding two numbers draws them. A `$param` holding a bad literal
/// gets that literal's warning. A `$param` the file does not declare, or one that
/// holds a selection, draws as if the key were absent and raises no warning.**
///
/// The page also says a param read by a plot attribute alone "has no
/// subscribers", as it does for `colorDomain: $param`: the subscriber graph
/// counts marks and legends and not plot attributes. That line is not this
/// key's, so the arms read the lines that name a domain.
#[test]
fn a_param_is_read_for_the_value_it_holds() {
    let unset = dots("");

    let good = dots_with_params("params:\n  ends: [0, 100]", "xDomain: $ends");
    assert_eq!(x_domain(&good), (0.0, 100.0), "a param holds the ends");
    assert!(
        said(&good).iter().all(|line| !line.contains("Domain")),
        "a param that holds two numbers raises no warning about the key; got {:?}",
        said(&good)
    );

    let bad = dots_with_params("params:\n  ends: [100, 0]", "xDomain: $ends");
    assert_eq!(
        points(&bad),
        points(&unset),
        "a param with bad ends draws them"
    );
    let lines: Vec<String> = said(&bad)
        .into_iter()
        .filter(|line| line.contains("Domain"))
        .collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("`xDomain: [100, 0]`"),
        "the param's value is named as a literal's is; got {:?}",
        lines[0]
    );

    for (label, params, attrs) in [
        ("undeclared", "", "xDomain: $nowhere"),
        (
            "a selection",
            "params:\n  picked: { select: crossfilter }",
            "xDomain: $picked",
        ),
    ] {
        let asked = dots_with_params(params, attrs);
        assert_eq!(points(&asked), points(&unset), "{label}: the key is read");
        assert!(
            said(&asked).iter().all(|line| !line.contains("Domain")),
            "{label}: a param with no value to read raises no warning about it; got {:?}",
            said(&asked)
        );
    }
}

// ---------------------------------------------------------------------------
// AC5 — two numbers on an axis that does not take them
// ---------------------------------------------------------------------------

/// **Two numbers on a date axis, or an axis of names, are named by the page's
/// warning as having no effect on that axis, and the plot draws as it does
/// without the key.** The axis the key does not land on is unaffected.
#[test]
fn two_numbers_on_a_date_or_a_name_axis_are_named_as_changing_nothing() {
    for (x, y, attrs, key, axis) in [
        ("at", "n", "xDomain: [0, 100]", "xDomain", "date"),
        ("n", "at", "yDomain: [0, 100]", "yDomain", "date"),
        ("name", "n", "xDomain: [0, 100]", "xDomain", "category"),
        ("n", "name", "yDomain: [0, 100]", "yDomain", "category"),
    ] {
        let unset = typed(x, y, "");
        assert!(said(&unset).is_empty(), "fixture check: {:?}", said(&unset));
        let asked = typed(x, y, attrs);
        assert_eq!(
            points(&asked),
            points(&unset),
            "`{attrs}` on {axis} changes what is painted"
        );
        let lines = said(&asked);
        assert_eq!(lines.len(), 1, "`{attrs}` on {axis}: {lines:?}");
        assert!(
            lines[0].contains(&format!("`{key}`"))
                && lines[0].contains("root (`probe`)")
                && lines[0].contains(&format!("changes nothing on a {axis} axis")),
            "the line names the key, the plot and the axis; got {:?}",
            lines[0]
        );
    }

    // `xyDomain` lands on both axes: the date one is named and the linear one is
    // fixed.
    let both = typed("at", "n", "xyDomain: [0, 100]");
    assert_eq!(domain(&both, 0, Channel::Y), (0.0, 100.0));
    let lines = said(&both);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("`xyDomain`") && lines[0].contains("a date axis"),
        "{:?}",
        lines[0]
    );
}

// ---------------------------------------------------------------------------
// AC6 — nice and zero beside fixed ends
// ---------------------------------------------------------------------------

/// **`xNice` or `xZero` beside an `xDomain` of two numbers leaves the ends as
/// written, and the page's warning names the key as having no effect on an axis
/// with fixed ends.** The same keys beside no domain move the ends, and beside a
/// domain that was refused they still do.
#[test]
fn nice_and_zero_beside_written_ends_leave_the_ends_and_are_named() {
    // Without a domain the keys act on the rows' own ends: x runs 4 to 150, so
    // nice rounds it to 0 to 160 and zero starts it at 0; y runs 3 to 97.
    assert_eq!(
        x_domain(&dots("xNice: true")),
        (0.0, 160.0),
        "fixture check"
    );
    assert_eq!(
        x_domain(&dots("xZero: true")),
        (0.0, 150.0),
        "fixture check"
    );
    assert_eq!(
        y_domain(&dots("yNice: true")),
        (0.0, 100.0),
        "fixture check"
    );
    assert_eq!(y_domain(&dots("yZero: true")), (0.0, 97.0), "fixture check");

    for (attrs, key, x, y) in [
        ("xDomain: [3, 97]\nxNice: true", "xNice", (3.0, 97.0), None),
        (
            "xDomain: [10, 90]\nxZero: true",
            "xZero",
            (10.0, 90.0),
            None,
        ),
        (
            "yDomain: [3, 97]\nyNice: true",
            "yNice",
            (4.0, 150.0),
            Some((3.0, 97.0)),
        ),
        (
            "yDomain: [10, 90]\nyZero: true",
            "yZero",
            (4.0, 150.0),
            Some((10.0, 90.0)),
        ),
        (
            "xyDomain: [3, 97]\nxNice: true",
            "xNice",
            (3.0, 97.0),
            Some((3.0, 97.0)),
        ),
    ] {
        let asked = dots(attrs);
        assert_eq!(x_domain(&asked), x, "`{attrs}`: the x ends moved");
        if let Some(y) = y {
            assert_eq!(y_domain(&asked), y, "`{attrs}`: the y ends moved");
        }
        let lines = said(&asked);
        assert_eq!(lines.len(), 1, "`{attrs}`: {lines:?}");
        assert!(
            lines[0].contains(&format!("`{key}`"))
                && lines[0].contains("root (`probe`)")
                && lines[0].contains("an axis with fixed ends"),
            "the line names the key and the plot; got {:?}",
            lines[0]
        );
    }

    // A key on the axis the pair did not name is not named, and acts.
    let other = dots("xDomain: [3, 97]\nyNice: true");
    assert_eq!(x_domain(&other), (3.0, 97.0));
    assert_eq!(y_domain(&other), (0.0, 100.0), "yNice acts on y");
    assert!(said(&other).is_empty(), "{:?}", said(&other));

    // A refused domain fixes no ends, so the key beside it still acts and is not
    // named as having no effect.
    let refused = dots("xDomain: [97, 3]\nxNice: true");
    assert_eq!(
        x_domain(&refused),
        (0.0, 160.0),
        "xNice acts when the pair is refused"
    );
    let lines = said(&refused);
    assert_eq!(lines.len(), 1, "only the refused pair is named: {lines:?}");
    assert!(lines[0].contains("`xDomain: [97, 3]`"), "{:?}", lines[0]);
}

// ---------------------------------------------------------------------------
// plotDefaults
// ---------------------------------------------------------------------------

/// **A domain under `plotDefaults:` reaches each plot, and the plot's own wins.
/// A bad one is named at each plot that inherits it.**
#[test]
fn a_plot_defaults_domain_reaches_each_plot_and_a_bad_one_is_named_at_each() {
    let reached = stacked("  xDomain: [0, 100]", "    xDomain: [0, 50]", "");
    assert_eq!(
        domain(&reached, 0, Channel::X),
        (0.0, 50.0),
        "the plot's own pair wins"
    );
    assert_eq!(
        domain(&reached, 1, Channel::X),
        (0.0, 100.0),
        "the plot with none inherits the default's"
    );
    assert!(said(&reached).is_empty(), "{:?}", said(&reached));

    let bad = stacked("  xDomain: [100, 0]", "", "");
    let lines = said(&bad);
    let named: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains("`xDomain: [100, 0]`"))
        .collect();
    assert_eq!(
        named.len(),
        2,
        "each plot that inherits the bad pair is named: {lines:?}"
    );
    assert_ne!(named[0], named[1], "the two lines name different plots");
}
