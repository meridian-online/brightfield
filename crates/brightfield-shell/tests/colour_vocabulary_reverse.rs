//! **A plot's `colorReverse: true` runs a dot's colour the other way: a ramp
//! from its far end, a diverging ramp with its arms on each other's sides, and
//! a category list from its last.**
//!
//! These run the whole composition — spec text, DuckDB, per-plot scales, the
//! scene — and read what the plot paints and what its legend lists, which are
//! the only places a colour is a fact rather than a promise. A colour is
//! attributed to a value by how many points wear it: the fixtures give each
//! value or category a different number of points, so the same set of colours
//! painted on both sides of the reversal is still told apart by which colour
//! has which count.
//!
//! The specs carry their data inline, so the run needs no file beside it, except
//! the sampled plot, whose rows come from DuckDB's own `range`.

use std::path::PathBuf;

use brightfield_render::channel::Channel;
use brightfield_render::scale::{Scale, SequentialScheme};
use brightfield_shell::legend::{ramp_strip_colours, LegendEntry, LegendSpec};
use brightfield_shell::pipeline::{compose_spec_sampled, compose_spec_str, Composed};
use brightfield_sql::ir::SampleRate;
use kurbo::{Affine, Circle};
use meridian_design::colour::Rgba;
use meridian_design::viz::{DIVERGING_BLUE_ARM, DIVERGING_MID_LIGHT, DIVERGING_RED_ARM};
use peniko::{Color, Fill};
use vello::Scene;

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

/// The colour `value` takes on `stops` about `pivot` when the domain reaches
/// `reach` either side of it: the arm below is the lower half of the stops, the
/// arm above the upper half, each over its own distance from the pivot.
fn about_pivot(stops: &[[f32; 4]], pivot: f64, reach: f64, value: f64) -> [f32; 4] {
    let t = 0.5 + 0.5 * ((value - pivot) / reach).clamp(-1.0, 1.0);
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

/// **AC1.** With `colorReverse: true`, a plot whose dots are filled by a number
/// column paints its smallest value in the colour its largest wore, and its
/// largest in the colour its smallest wore; the legend's ramp is flipped and its
/// two ends read the values they read before.
#[test]
fn a_number_fill_paints_its_smallest_value_in_the_colour_its_largest_wore() {
    // One point at 0, two at 5, three at 10: the ramp's ends are told apart by
    // how many points wear them, and 0 anchors the domain at `[0, 10]`.
    let values = counted(&[(0.0, 1), (5.0, 2), (10.0, 3)]);
    let stops = SequentialScheme::Viridis.stops();
    let (low, high) = (stops[0], stops[stops.len() - 1]);

    let plain = compose(&spec(&values, ""));
    assert_eq!(
        (points_in(&plain, low), points_in(&plain, high)),
        (1, 3),
        "fixture check: without the key the one point at 0 wears the low end and the three at 10 the high end"
    );

    let reversed = compose(&spec(&values, "colorReverse: true\n"));
    assert_eq!(
        (points_in(&reversed, low), points_in(&reversed, high)),
        (3, 1),
        "the point at 0 wears the colour 10 wore, and the points at 10 the colour 0 wore"
    );

    let Some(LegendSpec::Sequential {
        min,
        max,
        stops: plain_stops,
    }) = LegendSpec::from_scales(&plain.plots[0].scales)
    else {
        panic!("a number fill derives a sequential legend");
    };
    let Some(LegendSpec::Sequential {
        min: rmin,
        max: rmax,
        stops: reversed_stops,
    }) = LegendSpec::from_scales(&reversed.plots[0].scales)
    else {
        panic!("a reversed number fill derives a sequential legend");
    };
    assert_eq!(
        (rmin, rmax),
        (min, max),
        "the legend's two ends read the same values"
    );
    assert_eq!(
        (min, max),
        (0.0, 10.0),
        "fixture check: the ends are the rows' own"
    );
    let mut flipped = plain_stops.clone();
    flipped.reverse();
    assert_eq!(
        reversed_stops, flipped,
        "the legend's ramp is the plain one end for end"
    );
    let strips = ramp_strip_colours(&reversed_stops);
    assert_eq!(
        (strips[0], strips[strips.len() - 1]),
        (high, low),
        "the bar starts at the colour the high end wore and ends at the low end's"
    );
}

/// **AC2.** On a diverging scale the two arms change sides: the colour the
/// values below the pivot wore, the values above it wear, and the midpoint
/// colour stays at the pivot.
#[test]
fn a_diverging_scale_puts_its_arms_on_each_others_sides_and_the_midpoint_stays() {
    let stops = design_stops();
    let (blue, mid, red) = (
        rgba(DIVERGING_BLUE_ARM[0]),
        rgba(DIVERGING_MID_LIGHT),
        rgba(DIVERGING_RED_ARM[4]),
    );
    let attrs = "colorScale: diverging\ncolorPivot: 0\n";

    // One point at -4, two at the pivot, three at 4: the domain reaches 4 either
    // side, so each pole is painted and told apart by its count.
    let values = counted(&[(-4.0, 1), (0.0, 2), (4.0, 3)]);
    let plain = compose(&spec(&values, attrs));
    assert_eq!(
        (
            points_in(&plain, blue),
            points_in(&plain, mid),
            points_in(&plain, red)
        ),
        (1, 2, 3),
        "fixture check: without the key the point below wears the blue pole, the pivot the midpoint, the three above the red pole"
    );
    let reversed = compose(&spec(&values, &format!("colorReverse: true\n{attrs}")));
    assert_eq!(
        (
            points_in(&reversed, blue),
            points_in(&reversed, mid),
            points_in(&reversed, red)
        ),
        (3, 2, 1),
        "the point below wears the red pole, the three above the blue one, and the pivot keeps the midpoint"
    );

    // An uneven spread, so the colours below and above the pivot are not each
    // other's: 0 and 1 are below 2, and 3 and 9 above. Each value wears what the
    // value as far on the other side of 2 wore.
    let uneven = [0.0, 1.0, 2.0, 3.0, 9.0];
    let plain = compose(&spec(&uneven, "colorScale: diverging\ncolorPivot: 2\n"));
    let reversed = compose(&spec(
        &uneven,
        "colorReverse: true\ncolorScale: diverging\ncolorPivot: 2\n",
    ));
    let words = painted(&reversed);
    for v in uneven {
        let mirrored = about_pivot(&stops, 2.0, 7.0, 4.0 - v);
        assert!(
            words.contains(&packed(mirrored)),
            "the point at {v} wears the colour {} wore",
            4.0 - v
        );
    }
    assert!(
        words.contains(&packed(blue)) && !painted(&plain).contains(&packed(blue)),
        "the point at 9, in the red arm, wears the blue pole now, and without the key no point does"
    );

    let Some(LegendSpec::Diverging {
        min,
        max,
        pivot,
        stops: legend_stops,
    }) = LegendSpec::from_scales(&reversed.plots[0].scales)
    else {
        panic!("a reversed diverging fill derives a diverging legend");
    };
    assert_eq!(
        (min, max, pivot),
        (-5.0, 9.0, 2.0),
        "the legend's ends and pivot are the ones it read before"
    );
    let strips = ramp_strip_colours(&legend_stops);
    assert_eq!(strips[0], red, "the bar starts at the red pole");
    assert_eq!(
        strips[strips.len() - 1],
        blue,
        "the bar ends at the blue pole"
    );
    assert_eq!(
        strips[strips.len() / 2],
        mid,
        "the middle of the bar, where the pivot is labelled, is the midpoint colour"
    );
}

/// **AC3.** On a string column the first category wears the colour the last one
/// wore, and the legend lists the categories in the reversed order.
#[test]
fn a_string_fill_gives_the_first_category_the_colour_the_last_wore() {
    // One point in a, two in b, three in c.
    let groups = ["a", "b", "b", "c", "c", "c"];
    let plain = compose(&spec_by_string(&groups, ""));
    let (plain_categories, palette) = fill_categories(&plain);
    assert_eq!(
        plain_categories,
        ["a", "b", "c"],
        "fixture check: the scale's own order"
    );
    let (p0, p1, p2) = (palette[0], palette[1], palette[2]);
    assert_eq!(
        (
            points_in(&plain, p0),
            points_in(&plain, p1),
            points_in(&plain, p2)
        ),
        (1, 2, 3),
        "fixture check: without the key a wears the first colour, b the second, c the third"
    );

    let reversed = compose(&spec_by_string(&groups, "colorReverse: true\n"));
    assert_eq!(
        (
            points_in(&reversed, p0),
            points_in(&reversed, p1),
            points_in(&reversed, p2)
        ),
        (3, 2, 1),
        "a wears the colour c wore, c the colour a wore, and b keeps its own"
    );

    let Some(LegendSpec::Categorical { entries }) =
        LegendSpec::from_scales(&reversed.plots[0].scales)
    else {
        panic!("a string fill derives a categorical legend");
    };
    let listed: Vec<(&str, [f32; 4])> = entries
        .iter()
        .map(|LegendEntry { label, colour }| (label.as_str(), *colour))
        .collect();
    assert_eq!(
        listed,
        [("c", p0), ("b", p1), ("a", p2)],
        "the legend lists the categories from the last, each beside the colour it wears"
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

/// **AC3, sampled.** On a plot whose rows are sampled, the categories a sample
/// dropped are put back in the scale in their own order, and the reversal is
/// made after that, so the reversed list is the complete plot's list reversed
/// and not the sampled rows' own.
#[test]
fn the_reversal_holds_on_a_sampled_plot() {
    const CLASSES: u64 = 200;
    let dir = std::env::temp_dir().join(format!("bf-colour-reverse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let write = |name: &str, source: &str| -> String {
        let path: PathBuf = dir.join(name);
        std::fs::write(&path, source).expect("write spec");
        path.to_str().expect("utf-8 path").to_string()
    };
    let plain_path = write("plain.yaml", &categorical_fill(6_400, CLASSES, ""));
    let reversed_path = write(
        "reversed.yaml",
        &categorical_fill(6_400, CLASSES, "colorReverse: true\n"),
    );

    let complete = compose_spec_sampled(&plain_path, None).expect("compose unflagged");
    assert!(
        complete.plots[0].sample.is_none(),
        "fixture check: the complete side is below the ceiling, so it is what the sample is a sample of"
    );
    let (complete_order, _) = fill_categories(&complete);
    assert_eq!(
        complete_order.len(),
        CLASSES as usize,
        "fixture check: every class"
    );

    let rate = SampleRate::from_modulus(64).expect("power of two");
    let sampled = compose_spec_sampled(&reversed_path, Some(rate)).expect("compose sampled");
    let fact = sampled.plots[0]
        .sample
        .expect("fixture check: the forced rate applied");
    assert!(
        fact.drawn < CLASSES,
        "fixture check: the sample drew {} rows over {CLASSES} classes, so the drawn rows are short of one",
        fact.drawn
    );

    let mut want = complete_order.clone();
    want.reverse();
    assert_eq!(
        fill_categories(&sampled).0,
        want,
        "the sampled plot's categories are the complete plot's, reversed"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **AC4.** A `colorReverse` that is not `true` or `false`, a number say, draws
/// as a file without the key draws, and the page's warning names the key and the
/// value. A `$param`, `null` and a switch raise none.
#[test]
fn a_colour_reverse_that_is_no_switch_draws_as_absent_and_the_page_names_it() {
    let values = counted(&[(0.0, 1), (5.0, 2), (10.0, 3)]);
    let none = compose(&spec(&values, ""));

    for (attrs, written) in [
        ("colorReverse: 3\n", "3"),
        ("colorReverse: \"true\"\n", "true"),
        ("colorReverse: reverse\n", "reverse"),
        ("colorReverse: [a, b]\n", "<non-string>"),
    ] {
        let composed = compose(&spec(&values, attrs));
        assert_eq!(
            painted(&composed),
            painted(&none),
            "{attrs}: draws as the file without the key"
        );
        let told = about(&composed, "colorReverse");
        assert_eq!(told.len(), 1, "{attrs}: one line about the key: {told:?}");
        assert!(
            told[0].contains("`colorReverse`") && told[0].contains(&format!("`{written}`")),
            "{attrs}: the warning names the key and the value: {}",
            told[0]
        );
    }

    let reversed = compose(&spec(&values, "colorReverse: true\n"));
    for (what, composed) in [
        ("true", &reversed),
        ("false", &compose(&spec(&values, "colorReverse: false\n"))),
        ("null", &compose(&spec(&values, "colorReverse: null\n"))),
        (
            "a param holding a number",
            &compose(&format!(
                "params:\n  r: 1\n{}",
                spec(&values, "colorReverse: $r\n")
            )),
        ),
        (
            "a param nobody declared",
            &compose(&spec(&values, "colorReverse: $nobody\n")),
        ),
    ] {
        assert!(
            about(composed, "colorReverse").is_empty(),
            "{what}: {:?}",
            about(composed, "colorReverse")
        );
    }
}

/// A `$param` that holds `true` reverses the plot as the param stands.
#[test]
fn a_param_holding_a_switch_is_read_as_it_stands() {
    let values = counted(&[(0.0, 1), (5.0, 2), (10.0, 3)]);
    let literal = compose(&spec(&values, "colorReverse: true\n"));
    let none = compose(&spec(&values, ""));
    assert_ne!(
        painted(&literal),
        painted(&none),
        "fixture check: the literal reverses this plot"
    );
    let with_param = |held: &str| {
        compose(&format!(
            "params:\n  r: {held}\n{}",
            spec(&values, "colorReverse: $r\n")
        ))
    };
    assert_eq!(
        painted(&with_param("true")),
        painted(&literal),
        "a param holding true draws as the literal does"
    );
    assert_eq!(
        painted(&with_param("false")),
        painted(&none),
        "a param holding false draws as the file without the key"
    );
    assert_eq!(
        painted(&with_param("yes please")),
        painted(&none),
        "a param holding no switch draws as the file without the key"
    );
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

/// **AC5.** With `colorReverse: false`, and with no `colorReverse`, a plot draws
/// as it does today; and a cell, a heatmap and a raster keep the ramp they draw
/// under `colorReverse: true`.
#[test]
fn false_and_no_key_draw_alike_and_a_cell_a_heatmap_and_a_raster_keep_their_ramp() {
    for source in [
        spec(&counted(&[(0.0, 1), (5.0, 2), (10.0, 3)]), ""),
        spec(
            &counted(&[(-4.0, 1), (0.0, 2), (4.0, 3)]),
            "colorScale: diverging\ncolorPivot: 0\n",
        ),
    ] {
        let off = source.replace("width: 420", "colorReverse: false\nwidth: 420");
        assert_ne!(off, source, "fixture check: the key was added");
        assert_eq!(
            painted(&compose(&off)),
            painted(&compose(&source)),
            "false draws what no key draws"
        );
    }
    let groups = ["a", "b", "b", "c", "c", "c"];
    assert_eq!(
        painted(&compose(&spec_by_string(&groups, "colorReverse: false\n"))),
        painted(&compose(&spec_by_string(&groups, ""))),
        "false on a string fill draws what no key draws"
    );

    let (without, with) = (mark_specs(""), mark_specs("colorReverse: true\n"));
    for ((name, plain), (_, reversed)) in without.iter().zip(&with) {
        let (plain, reversed) = (compose(plain), compose(reversed));
        assert!(
            !painted(&plain).is_empty(),
            "{name}: the fixture draws something"
        );
        assert_eq!(
            painted(&reversed),
            painted(&plain),
            "{name}: draws as it does without colorReverse: true"
        );
        assert_eq!(
            LegendSpec::from_scales(&reversed.plots[0].scales),
            LegendSpec::from_scales(&plain.plots[0].scales),
            "{name}: and so does its legend"
        );
    }
}
