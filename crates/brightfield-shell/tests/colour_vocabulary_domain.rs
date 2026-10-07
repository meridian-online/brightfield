//! **A plot's `colorDomain` and `colorRange` set a dot's ramp: the two ends of a
//! number ramp, the order of a string column's categories, and the colours both
//! draw in.**
//!
//! These run the whole composition — spec text, DuckDB, per-plot scales, the
//! scene — and read what the plot paints and what its legend lists, which are
//! the only places a colour is a fact rather than a promise. A colour is
//! attributed to a value by how many points wear it: the fixtures give each
//! value or category a different number of points, so the same set of colours
//! painted with and without a key is still told apart by which colour has which
//! count, and each fixed arm is paired with the same fixture without the key so a
//! fixture the key could not move would not pass.
//!
//! The specs carry their data inline, except the sampled plot, whose rows come
//! from DuckDB's own `range`.

use std::path::PathBuf;

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_render::channel::Channel;
use brightfield_render::scale::{Scale, SequentialScheme};
use brightfield_shell::legend::{LegendEntry, LegendSpec};
use brightfield_shell::pipeline::{
    compose_spec_sampled, compose_spec_str, Composed, LiveDashboard,
};
use brightfield_spec::analysis::ComponentPath;
use brightfield_sql::ir::{SampleRate, ScalarValue};
use kurbo::{Affine, Circle};
use meridian_design::colour::Rgba;
use meridian_design::viz::{DIVERGING_BLUE_ARM, DIVERGING_MID_LIGHT, DIVERGING_RED_ARM};
use peniko::{Color, Fill};
use vello::Scene;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// A one-plot spec of dots filled by `v`, one dot for each entry of `values`;
/// `attrs` is the plot's attributes.
fn spec(values: &[f64], attrs: &str) -> String {
    let rows: String = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!("    - {{ x: {i}, y: {}, v: {v} }}\n", i * 3))
        .collect();
    format!(
        "data:\n  t:\n{rows}plot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n    fill: v\nwidth: 420\nheight: 300\n{attrs}"
    )
}

/// `value` repeated `times`, for each pair: how many points wear a value's colour.
fn counted(pairs: &[(f64, usize)]) -> Vec<f64> {
    pairs
        .iter()
        .flat_map(|(value, times)| std::iter::repeat_n(*value, *times))
        .collect()
}

/// A one-plot spec of dots filled by the string column `g`, one dot for each
/// entry of `groups`.
fn spec_by_string(groups: &[&str], attrs: &str) -> String {
    let rows: String = groups
        .iter()
        .enumerate()
        .map(|(i, g)| format!("    - {{ x: {i}, y: {}, g: {g} }}\n", i * 3))
        .collect();
    format!(
        "data:\n  t:\n{rows}plot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n    fill: g\nwidth: 420\nheight: 300\n{attrs}"
    )
}

fn compose(source: &str) -> Composed {
    compose_spec_str(source, None)
        .unwrap_or_else(|e| panic!("the spec must compose: {e}\n{source}"))
}

/// A colour as the scene encodes it, by drawing one circle in it.
fn packed(colour: [f32; 4]) -> u32 {
    let mut scene = Scene::new();
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::new(colour),
        None,
        &Circle::new((0.0, 0.0), 1.0),
    );
    let words: Vec<u32> = scene.encoding().draw_data.to_vec();
    assert_eq!(words.len(), 1, "one solid fill encodes one colour word");
    words[0]
}

/// The colour words the whole composed scene paints.
fn painted(composed: &Composed) -> Vec<u32> {
    composed.scene.encoding().draw_data.to_vec()
}

/// How many times the scene paints `colour`.
fn points_in(composed: &Composed, colour: [f32; 4]) -> usize {
    let word = packed(colour);
    painted(composed).iter().filter(|w| **w == word).count()
}

fn rgba(c: Rgba) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

/// What the page is told about `key`, as it words it.
fn about(composed: &Composed, key: &str) -> Vec<String> {
    composed
        .diagnostics
        .diagnostics
        .iter()
        .filter(|d| d.wire_name == key)
        .map(ToString::to_string)
        .collect()
}

/// The colour a ramp of `stops` has at `t`, `0` to `1`: a lerp between the two
/// stops that bracket it.
fn ramp_at(stops: &[[f32; 4]], t: f64) -> [f32; 4] {
    let at = t * (stops.len() - 1) as f64;
    let i = (at.floor() as usize).min(stops.len() - 2);
    let frac = (at - i as f64) as f32;
    let (a, b) = (stops[i], stops[i + 1]);
    [
        a[0] + (b[0] - a[0]) * frac,
        a[1] + (b[1] - a[1]) * frac,
        a[2] + (b[2] - a[2]) * frac,
        a[3] + (b[3] - a[3]) * frac,
    ]
}

/// The design system's blue-red ramp with the light midpoint, pole to pole.
fn design_stops() -> Vec<[f32; 4]> {
    DIVERGING_BLUE_ARM
        .iter()
        .copied()
        .map(rgba)
        .chain(std::iter::once(rgba(DIVERGING_MID_LIGHT)))
        .chain(DIVERGING_RED_ARM.iter().copied().map(rgba))
        .collect()
}

/// The plot's fill categories and palette, when the fill is categorical.
fn fill_categories(composed: &Composed) -> (Vec<String>, Vec<[f32; 4]>) {
    match composed.plots[0].scales.get(Channel::Fill) {
        Some(Scale::Colour {
            categories,
            palette,
        }) => (categories.clone(), palette.clone()),
        other => panic!("the fill is not categorical: {other:?}"),
    }
}

/// The ends the plot's legend reads for a number fill.
fn legend_ends(composed: &Composed, plot: usize) -> (f64, f64) {
    match LegendSpec::from_scales(&composed.plots[plot].scales) {
        Some(LegendSpec::Sequential { min, max, .. } | LegendSpec::Diverging { min, max, .. }) => {
            (min, max)
        }
        other => panic!("a number fill derives a ramp legend: {other:?}"),
    }
}

/// **AC1.** On a linear scale with `colorDomain: [0, 10]` a plot whose dots are
/// filled by a number column draws a legend whose ends read 0 and 10, paints a
/// point at 5 in the ramp's middle colour, and paints a point above 10 in the
/// colour of 10.
#[test]
fn a_fixed_domain_sets_a_ramps_ends_and_a_point_past_an_end_wears_its_colour() {
    // One point at 0, two at 5, three at 10, four at 15: told apart by count.
    let values = counted(&[(0.0, 1), (5.0, 2), (10.0, 3), (15.0, 4)]);
    let stops = SequentialScheme::Viridis.stops();
    let (low, middle, high) = (stops[0], stops[4], stops[stops.len() - 1]);

    let plain = compose(&spec(&values, ""));
    assert_eq!(
        (
            points_in(&plain, low),
            points_in(&plain, middle),
            points_in(&plain, high)
        ),
        (1, 0, 4),
        "fixture check: without the key the ramp runs 0 to 15, so only the four points at 15 wear \
         the high end and none at 5 wears the middle"
    );
    assert_eq!(
        legend_ends(&plain, 0),
        (0.0, 15.0),
        "fixture check: the rows' own ends"
    );

    let fixed = compose(&spec(&values, "colorDomain: [0, 10]\n"));
    assert_eq!(
        legend_ends(&fixed, 0),
        (0.0, 10.0),
        "the legend's ends read 0 and 10"
    );
    assert_eq!(
        points_in(&fixed, low),
        1,
        "the point at 0 wears the low end"
    );
    assert_eq!(
        points_in(&fixed, middle),
        2,
        "the two points at 5 wear the ramp's middle colour"
    );
    assert_eq!(
        points_in(&fixed, high),
        7,
        "the three points at 10 and the four above it wear the colour of 10"
    );
}

/// The brush-and-filtered-dots dashboard AC2 sweeps: the first plot is brushed
/// on `temp`, the second draws the rows the brush leaves, filled by `v`.
fn swept_spec(attrs: &str) -> String {
    format!(
        "params:
  brush: {{ select: crossfilter }}
data:
  readings:
    - {{ temp: 1, v: 0 }}
    - {{ temp: 2, v: 2 }}
    - {{ temp: 3, v: 4 }}
    - {{ temp: 4, v: 6 }}
    - {{ temp: 5, v: 8 }}
    - {{ temp: 6, v: 10 }}
    - {{ temp: 7, v: 10 }}
hconcat:
  - plot:
      - mark: dot
        data: {{ from: readings }}
        x: temp
        y: v
      - select: intervalX
        as: $brush
    width: 320
    height: 240
  - plot:
      - mark: dot
        data: {{ from: readings, filterBy: $brush }}
        x: temp
        y: v
        fill: v
    width: 320
    height: 240
    {attrs}
"
    )
}

/// **AC2.** After a range is swept on another tile, the legend's ends still
/// read 0 and 10, and what the sweep leaves is painted along the same ramp.
#[test]
fn the_legends_ends_hold_when_a_range_is_swept_on_another_tile() {
    let mut live = LiveDashboard::load_str(&swept_spec("colorDomain: [0, 10]"), None)
        .expect("the spec loads live");
    let before = live.present().expect("first composite");
    let stops = SequentialScheme::Viridis.stops();
    let high = stops[stops.len() - 1];
    assert_eq!(
        (legend_ends(&before, 1), points_in(&before, high)),
        ((0.0, 10.0), 2),
        "fixture check: at rest the ends read 0 and 10 and the two rows at 10 wear the high end"
    );

    // Sweep temp 0 to 3.5: the rows at v 0, 2 and 4 are left and the two at 10
    // are not.
    let swept = live
        .apply(Interaction::Select {
            name: "brush".to_string(),
            contributor: ComponentPath(before.plots[0].path.clone()),
            predicate: SqlPredicate::Interval {
                column: "temp".to_string(),
                lo: ScalarValue::Float(0.0),
                hi: ScalarValue::Float(3.5),
                meta: None,
            },
        })
        .expect("the sweep re-composites");
    assert_eq!(
        points_in(&swept, high),
        0,
        "fixture check: the sweep took the rows that wore the high end, so the rows left top out at 4"
    );
    assert_eq!(
        legend_ends(&swept, 1),
        (0.0, 10.0),
        "the legend's ends still read 0 and 10"
    );
    let at_four = ramp_at(&stops, 0.4);
    assert_eq!(
        points_in(&swept, at_four),
        1,
        "the row at 4 is painted at 4 tenths of the fixed ramp and not at the top of the rows left"
    );
}

/// **AC3.** On a diverging scale with `colorDomain: [0, 10]` and `colorPivot: 2`
/// a point at 0 wears the low pole, a point at 2 the midpoint colour and a point
/// at 10 the high pole; each arm runs over its own span.
#[test]
fn a_diverging_domain_is_drawn_as_written_about_the_pivot() {
    let stops = design_stops();
    let (low_pole, mid, high_pole) = (
        rgba(DIVERGING_BLUE_ARM[0]),
        rgba(DIVERGING_MID_LIGHT),
        rgba(DIVERGING_RED_ARM[4]),
    );
    let attrs = "colorScale: diverging\ncolorPivot: 2\n";
    // One point at 0, two at 2, three at 10, four at 1: told apart by count.
    let values = counted(&[(0.0, 1), (2.0, 2), (10.0, 3), (1.0, 4)]);

    let plain = compose(&spec(&values, attrs));
    assert_eq!(
        points_in(&plain, low_pole),
        0,
        "fixture check: without the key the domain is even about the pivot, so it reaches 8 either \
         side, 0 is short of the low pole and nothing wears it"
    );
    assert_eq!(
        legend_ends(&plain, 0),
        (-6.0, 10.0),
        "fixture check: even about 2, reaching 8 either side"
    );

    let fixed = compose(&spec(&values, &format!("colorDomain: [0, 10]\n{attrs}")));
    assert_eq!(
        (
            points_in(&fixed, low_pole),
            points_in(&fixed, mid),
            points_in(&fixed, high_pole)
        ),
        (1, 2, 3),
        "the point at 0 wears the low pole, the two at the pivot the midpoint colour and the three \
         at 10 the high pole"
    );
    assert_eq!(
        legend_ends(&fixed, 0),
        (0.0, 10.0),
        "the legend's ends are as written and not widened about the pivot"
    );
    // The arm below the pivot runs 0 to 2 over the lower half of the ramp, so 1
    // is a quarter of the way along it.
    assert_eq!(
        points_in(&fixed, ramp_at(&stops, 0.25)),
        4,
        "the four points at 1 are halfway up the lower arm, which spans 0 to 2"
    );
}

/// **AC4.** With `colorRange` naming two or more colours the ramp runs through
/// them in order; on a diverging scale with an odd count the middle colour sits at
/// the pivot; and with both a `colorScheme` and a `colorRange` the range draws.
#[test]
fn a_range_runs_the_ramp_through_its_colours_and_wins_over_a_scheme() {
    let values = counted(&[(0.0, 1), (5.0, 2), (10.0, 3)]);

    // Two colours: a two-stop ramp, so 5 is the midpoint of the pair.
    let two = compose(&spec(&values, "colorRange: ['#ff0000', '#0000ff']\n"));
    assert_eq!(
        (
            points_in(&two, RED),
            points_in(&two, [0.5, 0.0, 0.5, 1.0]),
            points_in(&two, BLUE)
        ),
        (1, 2, 3),
        "0 wears the first colour, 10 the last and 5 the colour between them"
    );
    // Three colours: the middle one is at the middle of the domain.
    let three_range = "colorRange: ['#ff0000', '#00ff00', '#0000ff']\n";
    let three = compose(&spec(&values, three_range));
    assert_eq!(
        (
            points_in(&three, RED),
            points_in(&three, GREEN),
            points_in(&three, BLUE)
        ),
        (1, 2, 3),
        "three colours run in order, the second at the middle of the ramp"
    );
    assert_eq!(
        points_in(&compose(&spec(&values, "")), RED),
        0,
        "fixture check: without the key no point wears red"
    );

    // A scheme and a range: the range's colours draw.
    let both = compose(&spec(
        &values,
        &format!("colorScheme: blues\n{three_range}"),
    ));
    assert_eq!(
        painted(&both),
        painted(&three),
        "with a colorScheme and a colorRange the range's colours draw"
    );
    let LegendSpec::Sequential {
        stops: legend_stops,
        ..
    } = LegendSpec::from_scales(&both.plots[0].scales).expect("a number fill has a legend")
    else {
        panic!("a linear number fill derives a sequential legend");
    };
    assert_eq!(
        legend_stops,
        [RED, GREEN, BLUE],
        "and the legend's ramp is the range's"
    );

    // Diverging, odd counts: the middle colour is at the pivot whatever the ends.
    let rows = counted(&[(0.0, 1), (2.0, 2), (10.0, 3)]);
    let five_range = "colorRange: ['#ff0000', '#ff8000', '#00ff00', '#0080ff', '#0000ff']\n";
    for (range, middle, count) in [(three_range, GREEN, 3), (five_range, GREEN, 5)] {
        let drawn = compose(&spec(
            &rows,
            &format!("colorScale: diverging\ncolorPivot: 2\n{range}"),
        ));
        assert_eq!(
            points_in(&drawn, middle),
            2,
            "{count} colours: the two points at the pivot wear the middle one"
        );
        assert_eq!(
            points_in(&drawn, BLUE),
            3,
            "{count} colours: the three at the top of the domain wear the last"
        );
        assert_eq!(
            points_in(&drawn, RED),
            0,
            "{count} colours: the domain is even about the pivot, so the point at 0 is short of the first"
        );
    }
}

/// **AC5.** On a string column `colorDomain` as a list of categories fixes their
/// order in the legend, and `colorRange` gives them its colours in that order.
#[test]
fn a_string_domain_fixes_the_order_and_a_range_gives_the_colours_in_that_order() {
    // One point in a, two in b, three in c.
    let groups = ["a", "b", "b", "c", "c", "c"];
    let plain = compose(&spec_by_string(&groups, ""));
    let (plain_order, palette) = fill_categories(&plain);
    assert_eq!(
        plain_order,
        ["a", "b", "c"],
        "fixture check: the scale's own order"
    );

    // The domain alone: the legend lists c, a, b, and the palette's first slot is c's.
    let ordered = compose(&spec_by_string(&groups, "colorDomain: [c, a, b]\n"));
    let (order, ordered_palette) = fill_categories(&ordered);
    assert_eq!(order, ["c", "a", "b"], "the scale holds the file's order");
    assert_eq!(
        ordered_palette, palette,
        "the palette is the same colours, taken in the new order"
    );
    assert_eq!(
        (
            points_in(&ordered, palette[0]),
            points_in(&ordered, palette[1]),
            points_in(&ordered, palette[2])
        ),
        (3, 1, 2),
        "c, now first, wears the first colour, a the second and b the third"
    );

    // The domain with a range: each category wears the colour at its place.
    let ranged = compose(&spec_by_string(
        &groups,
        "colorDomain: [c, a, b]\ncolorRange: ['#ff0000', '#00ff00', '#0000ff']\n",
    ));
    let Some(LegendSpec::Categorical { entries }) =
        LegendSpec::from_scales(&ranged.plots[0].scales)
    else {
        panic!("a string fill derives a categorical legend");
    };
    let listed: Vec<(&str, [f32; 4])> = entries
        .iter()
        .map(|LegendEntry { label, colour }| (label.as_str(), *colour))
        .collect();
    assert_eq!(
        listed,
        [("c", RED), ("a", GREEN), ("b", BLUE)],
        "the legend lists the categories in the file's order, each beside the colour at its place"
    );
    assert_eq!(
        (
            points_in(&ranged, RED),
            points_in(&ranged, GREEN),
            points_in(&ranged, BLUE)
        ),
        (3, 1, 2),
        "the three points in c are red, the one in a green and the two in b blue"
    );
}

/// **AC6.** A `colorDomain` or a `colorRange` given as a `$param` that names a
/// literal array draws as the array would.
#[test]
fn a_param_that_names_a_literal_array_draws_as_the_array_would() {
    let values = counted(&[(0.0, 1), (5.0, 2), (10.0, 3), (15.0, 4)]);
    let literal = compose(&spec(
        &values,
        "colorDomain: [0, 10]\ncolorRange: ['#ff0000', '#0000ff']\n",
    ));
    let none = compose(&spec(&values, ""));
    assert_ne!(
        painted(&literal),
        painted(&none),
        "fixture check: the literals move this plot"
    );
    let from_params = compose(&format!(
        "params:\n  domain: [0, 10]\n  colors: ['#ff0000', '#0000ff']\n{}",
        spec(&values, "colorDomain: $domain\ncolorRange: $colors\n")
    ));
    assert_eq!(
        painted(&from_params),
        painted(&literal),
        "the params draw as the arrays they hold"
    );
    assert_eq!(
        legend_ends(&from_params, 0),
        (0.0, 10.0),
        "and the legend's ends are the param's"
    );

    let groups = ["a", "b", "b", "c", "c", "c"];
    let literal = compose(&spec_by_string(
        &groups,
        "colorDomain: [c, a, b]\ncolorRange: ['#ff0000', '#00ff00', '#0000ff']\n",
    ));
    let from_params = compose(&format!(
        "params:\n  order: [c, a, b]\n  colors: ['#ff0000', '#00ff00', '#0000ff']\n{}",
        spec_by_string(&groups, "colorDomain: $order\ncolorRange: $colors\n")
    ));
    assert_ne!(
        painted(&literal),
        painted(&compose(&spec_by_string(&groups, ""))),
        "fixture check: the literals move this plot"
    );
    assert_eq!(
        painted(&from_params),
        painted(&literal),
        "the params draw as the arrays they hold on a string column"
    );
}

/// **AC7.** `colorDomain: Fixed`, and every other value that is no domain or no
/// colour list, draws as a file without the key draws, and the page says nothing
/// of it.
#[test]
fn fixed_and_values_that_are_no_domain_draw_as_a_file_without_the_key() {
    let values = counted(&[(0.0, 1), (5.0, 2), (10.0, 3)]);
    let bare = painted(&compose(&spec(&values, "")));
    for attrs in [
        "colorDomain: Fixed\n",
        "colorDomain: [10, 0]\n",
        "colorDomain: [4]\n",
        "colorDomain: [0, 5, 10]\n",
        "colorDomain: [a, 3]\n",
        "colorRange: viridis\n",
        "colorRange: [red]\n",
        "colorRange: ['#ff0000', 'not-a-colour']\n",
        // One entry that is no colour makes the key no range: dropping it would
        // leave a two-colour ramp the file did not write.
        "colorRange: ['#ff0000', 'not-a-colour', '#0000ff']\n",
        "colorRange: []\n",
        "colorDomain: Fixed\ncolorRange: [red]\n",
    ] {
        let composed = compose(&spec(&values, attrs));
        assert_eq!(
            painted(&composed),
            bare,
            "{attrs:?} draws as the key absent"
        );
        for key in ["colorDomain", "colorRange"] {
            assert!(
                about(&composed, key).is_empty(),
                "{attrs:?}: the page says nothing of {key}: {:?}",
                about(&composed, key)
            );
        }
    }
    let groups = ["a", "b", "b", "c", "c", "c"];
    let bare = painted(&compose(&spec_by_string(&groups, "")));
    for attrs in ["colorDomain: Fixed\n", "colorDomain: [0, 10]\n"] {
        assert_eq!(
            painted(&compose(&spec_by_string(&groups, attrs))),
            bare,
            "a string fill under {attrs:?} draws as the key absent: two numbers are no categories"
        );
    }
    // A string domain on a number fill is likewise no pair of ends.
    assert_eq!(
        painted(&compose(&spec(&values, "colorDomain: [a, b]\n"))),
        painted(&compose(&spec(&values, ""))),
        "a number fill under a list of categories draws as the key absent"
    );
}

/// A row-level dot scatter filled by a category, over `rows` rows and `classes`
/// classes, whose rows come from DuckDB so the fixture is two counts.
fn categorical_fill(rows: u64, classes: u64, attrs: &str) -> String {
    format!(
        "data:
  points:
    query: |
      SELECT
        i                                                          AS n,
        ((i * 40503 + 12345) % 100019) / 1000.0                    AS depth,
        'class-' || lpad(({classes} - 1 - (i % {classes}))::VARCHAR, 3, '0') AS band
      FROM range({rows}) AS t(i)
plot:
  - mark: dot
    data: {{ from: points }}
    x: n
    y: depth
    fill: band
width: 640
height: 480
{attrs}"
    )
}

/// **A sampled plot.** The scene puts a sampled plot's categories back, sorted,
/// after the marks build their scales; the file's order is applied after that,
/// so the sampled plot lists the categories as the file does and not as the
/// restoration sorted them.
#[test]
fn the_files_order_holds_on_a_sampled_plot() {
    const CLASSES: u64 = 200;
    let file_order: Vec<String> = (0..CLASSES)
        .rev()
        .map(|i| format!("class-{i:03}"))
        .collect();
    let attrs = format!("colorDomain: [{}]\n", file_order.join(", "));
    let dir = std::env::temp_dir().join(format!("bf-colour-domain-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let write = |name: &str, source: &str| -> String {
        let path: PathBuf = dir.join(name);
        std::fs::write(&path, source).expect("write spec");
        path.to_str().expect("utf-8 path").to_string()
    };
    let plain_path = write("plain.yaml", &categorical_fill(6_400, CLASSES, ""));
    let ordered_path = write("ordered.yaml", &categorical_fill(6_400, CLASSES, &attrs));

    let (plain_order, _) = fill_categories(
        &compose_spec_sampled(&plain_path, None).expect("compose the plain plot, unflagged"),
    );
    let mut ascending = file_order.clone();
    ascending.reverse();
    assert_eq!(
        plain_order, ascending,
        "fixture check: without the key the scale lists the classes ascending, so the file's list \
         is not the order the restoration would give"
    );

    let complete = compose_spec_sampled(&ordered_path, None).expect("compose unflagged");
    assert!(
        complete.plots[0].sample.is_none(),
        "fixture check: the complete side is below the ceiling, so it is what the sample is a sample of"
    );
    assert_eq!(
        fill_categories(&complete).0,
        file_order,
        "the complete plot lists the file's order"
    );

    let rate = SampleRate::from_modulus(64).expect("power of two");
    let sampled = compose_spec_sampled(&ordered_path, Some(rate)).expect("compose sampled");
    let fact = sampled.plots[0]
        .sample
        .expect("fixture check: the forced rate applied");
    assert!(
        fact.drawn < CLASSES,
        "fixture check: the sample drew {} rows over {CLASSES} classes, so the drawn rows are short of one",
        fact.drawn
    );
    assert_eq!(
        fill_categories(&sampled).0,
        file_order,
        "the sampled plot lists the file's order, and not the restoration's ascending one"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The cell, heatmap and raster specs the unchanged-draw test reads.
fn mark_specs(attrs: &str) -> Vec<(&'static str, String)> {
    let rows: String = (0..6)
        .flat_map(|day| (0..4).map(move |hour| (day, hour)))
        .map(|(day, hour)| format!("    - {{ day: {day}, hour: {hour}, x: {hour}, y: {day} }}\n"))
        .collect();
    let plot =
        |layer: &str| format!("data:\n  t:\n{rows}plot:\n{layer}width: 420\nheight: 300\n{attrs}");
    [
        (
            "cell",
            "  - mark: cell\n    data: { from: t }\n    x: hour\n    y: day\n    fill: { count: }\n",
        ),
        (
            "heatmap",
            "  - mark: heatmap\n    data: { from: t }\n    x: x\n    y: y\n",
        ),
        (
            "raster",
            "  - mark: raster\n    data: { from: t }\n    x: x\n    y: y\n",
        ),
    ]
    .into_iter()
    .map(|(name, layer)| (name, plot(layer)))
    .collect()
}

/// **Which marks.** A cell, a heatmap and a raster keep the ramp they draw today
/// under these keys: the dots are the marks that take them.
#[test]
fn a_cell_a_heatmap_and_a_raster_keep_their_ramp() {
    let without = mark_specs("");
    let with = mark_specs("colorDomain: [0, 3]\ncolorRange: ['#ff0000', '#0000ff']\n");
    for ((name, plain), (_, keyed)) in without.iter().zip(&with) {
        let (plain, keyed) = (compose(plain), compose(keyed));
        assert!(
            !painted(&plain).is_empty(),
            "{name}: the fixture draws something"
        );
        assert_eq!(
            painted(&keyed),
            painted(&plain),
            "{name}: draws as it does without the keys"
        );
        assert_eq!(
            LegendSpec::from_scales(&keyed.plots[0].scales),
            LegendSpec::from_scales(&plain.plots[0].scales),
            "{name}: and so does its legend"
        );
    }
}

/// `colorReverse` turns what the file wrote: a reversed range runs the ramp from
/// its last colour, and the domain's ends read as they were written.
#[test]
fn colour_reverse_turns_what_the_file_wrote() {
    let values = counted(&[(0.0, 1), (5.0, 2), (10.0, 3), (15.0, 4)]);
    let attrs = "colorDomain: [0, 10]\ncolorRange: ['#ff0000', '#00ff00', '#0000ff']\n";
    let forward = compose(&spec(&values, attrs));
    let reversed = compose(&spec(&values, &format!("colorReverse: true\n{attrs}")));
    assert_eq!(
        (
            points_in(&forward, RED),
            points_in(&forward, GREEN),
            points_in(&forward, BLUE)
        ),
        (1, 2, 7),
        "fixture check: forward, 0 is red, 5 green, and 10 and above blue"
    );
    assert_eq!(
        (
            points_in(&reversed, RED),
            points_in(&reversed, GREEN),
            points_in(&reversed, BLUE)
        ),
        (7, 2, 1),
        "reversed, 10 and above are red, 5 stays green and 0 is blue"
    );
    assert_eq!(
        legend_ends(&reversed, 0),
        (0.0, 10.0),
        "the ends read as the file wrote them"
    );
}
