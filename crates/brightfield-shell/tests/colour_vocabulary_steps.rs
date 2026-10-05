//! **A plot's `colorScale: quantize` with `colorN` paints a dot's number column
//! in steps: the domain cut into as many steps of equal width as the file asks
//! for, each step one flat colour of the scheme.**
//!
//! These run the whole composition — spec text, DuckDB, per-plot scales, the
//! scene — and read what the plot paints, which is the only place a colour is a
//! fact rather than a promise. A colour is attributed to a step by how many
//! points wear it: the fixtures give each step a different number of points, so
//! the five colours are told apart by their counts, and each keyed fixture is
//! paired with the same fixture without the key so a fixture the key could not
//! move would not pass.
//!
//! The specs carry their data inline, so the run needs no file beside it.

use brightfield_render::channel::Channel;
use brightfield_render::scale::{Scale, SequentialScheme};
use brightfield_shell::legend::LegendSpec;
use brightfield_shell::pipeline::{compose_spec_str, Composed};
use kurbo::{Affine, Circle};
use peniko::{Color, Fill};
use vello::Scene;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];

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

/// `value` repeated `times`, for each pair: how many points wear a step's colour.
fn counted(pairs: &[(f64, usize)]) -> Vec<f64> {
    pairs
        .iter()
        .flat_map(|(value, times)| std::iter::repeat_n(*value, *times))
        .collect()
}

/// A one-plot spec of dots filled by the string column `g`.
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

/// How many points wear each of `colours`, in order.
fn counts(composed: &Composed, colours: &[[f32; 4]]) -> Vec<usize> {
    colours.iter().map(|c| points_in(composed, *c)).collect()
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
/// stops that bracket it. Written out here, so the expected colours are not read
/// from the code under test.
fn ramp_at(stops: &[[f32; 4]], t: f64) -> [f32; 4] {
    let at = t * (stops.len() - 1) as f64;
    let i = (at.floor() as usize).min(stops.len() - 2);
    let frac = (at - i as f64) as f32;
    let (a, b) = (stops[i], stops[i + 1]);
    if frac >= 1.0 {
        return b;
    }
    [
        a[0] + (b[0] - a[0]) * frac,
        a[1] + (b[1] - a[1]) * frac,
        a[2] + (b[2] - a[2]) * frac,
        a[3] + (b[3] - a[3]) * frac,
    ]
}

/// The colours of `steps` steps of the default scheme: the first its lowest
/// colour, the last its highest, and the ones between at even spacing.
fn viridis_steps(steps: usize) -> Vec<[f32; 4]> {
    let stops = SequentialScheme::Viridis.stops();
    (0..steps)
        .map(|i| ramp_at(&stops, i as f64 / (steps - 1) as f64))
        .collect()
}

/// The fill's stepped scale, as the first plot drew it.
fn steps_of(composed: &Composed) -> (Vec<f64>, Vec<[f32; 4]>) {
    let scale = composed.plots[0]
        .scales
        .get(Channel::Fill)
        .expect("the plot has a fill scale");
    match scale {
        Scale::Quantized { colours, .. } => (
            scale.step_edges().expect("a stepped scale has edges"),
            colours.clone(),
        ),
        other => panic!("the fill is not stepped: {other:?}"),
    }
}

/// Whether the fill is still a continuous ramp.
fn is_ramp(composed: &Composed) -> bool {
    matches!(
        composed.plots[0].scales.get(Channel::Fill),
        Some(Scale::Sequential { .. })
    )
}

fn assert_edges(found: &[f64], expected: &[f64], what: &str) {
    assert_eq!(found.len(), expected.len(), "{what}: {found:?}");
    for (f, e) in found.iter().zip(expected) {
        assert!(
            (f - e).abs() < 1e-6,
            "{what}: {found:?} against {expected:?}"
        );
    }
}

/// The 0-to-100 fixture: points on both sides of every edge, a different number
/// in each step so the five colours are told apart by count, and one at the top
/// of the domain.
///
/// Step one holds 0 and 19.99, step two 20 and 39.99, step three 40 and 59.99,
/// step four 60 and 79.99, step five 80 and 100.
fn hundred() -> Vec<f64> {
    counted(&[
        (0.0, 1),
        (19.99, 1),
        (20.0, 1),
        (39.99, 2),
        (40.0, 2),
        (59.99, 2),
        (60.0, 2),
        (79.99, 3),
        (80.0, 3),
        (100.0, 3),
    ])
}

/// How many points the [`hundred`] fixture puts in each of its five steps.
const HUNDRED_COUNTS: [usize; 5] = [2, 3, 4, 5, 6];

/// **AC1.** With `colorScale: quantize` and `colorN: 5` on a number column running
/// 0 to 100, each point wears one of five colours: points below 20 share the
/// first, points from 20 up to but not including 40 the second, and so on, with
/// points from 80 to 100 in the fifth. The first step wears the scheme's lowest
/// colour, the fifth its highest, and the three between are taken at even spacing.
#[test]
fn five_steps_cut_nought_to_a_hundred_into_twenties_and_each_point_wears_its_step() {
    let steps = viridis_steps(5);
    let plain = compose(&spec(&hundred(), ""));
    assert!(is_ramp(&plain), "fixture check: no key, a continuous ramp");
    assert_eq!(
        counts(&plain, &steps[1..4]),
        [0, 0, 0],
        "fixture check: the ramp wears none of the three inner step colours, so the steps below \
         are the key's doing"
    );

    let stepped = compose(&spec(&hundred(), "colorScale: quantize\ncolorN: 5\n"));
    let (edges, colours) = steps_of(&stepped);
    assert_edges(&edges, &[0.0, 20.0, 40.0, 60.0, 80.0, 100.0], "the edges");
    assert_eq!(
        colours, steps,
        "the scheme's lowest, three between, highest"
    );
    assert_eq!(
        counts(&stepped, &steps),
        HUNDRED_COUNTS,
        "each step's points wear its colour: 20 and 40 and 60 and 80 are in the step above, and \
         100 is in the fifth"
    );
    assert_eq!(
        HUNDRED_COUNTS.iter().sum::<usize>(),
        hundred().len(),
        "and every point wears one of the five"
    );
}

/// **AC3.** With `colorScale: quantize` and no `colorN`, the same column draws
/// five steps.
#[test]
fn quantize_with_no_count_draws_five_steps() {
    let steps = viridis_steps(5);
    let stepped = compose(&spec(&hundred(), "colorScale: quantize\n"));
    let (edges, colours) = steps_of(&stepped);
    assert_edges(&edges, &[0.0, 20.0, 40.0, 60.0, 80.0, 100.0], "the edges");
    assert_eq!(colours, steps);
    assert_eq!(counts(&stepped, &steps), HUNDRED_COUNTS);
}

/// A count other than five cuts that many steps: three, and eight.
#[test]
fn the_count_is_the_count_asked_for() {
    let values = counted(&[(5.0, 1), (50.0, 2), (95.0, 3)]);
    let three = compose(&spec(&values, "colorScale: quantize\ncolorN: 3\n"));
    let steps = viridis_steps(3);
    assert_edges(
        &steps_of(&three).0,
        &[0.0, 95.0 / 3.0, 95.0 * 2.0 / 3.0, 95.0],
        "three steps over the rows' own 0 to 95",
    );
    assert_eq!(counts(&three, &steps), [1, 2, 3]);
    let eight = compose(&spec(&values, "colorScale: quantize\ncolorN: 8\n"));
    assert_eq!(steps_of(&eight).1, viridis_steps(8));
    assert_eq!(steps_of(&eight).0.len(), 9);
}

/// **AC4.** With `colorN: 5` on a column running 0 to 500001, the plot draws five
/// steps of equal width. Mosaic's renderer rounds the thresholds to tidy values
/// and draws six there; this draws what was asked.
#[test]
fn five_steps_over_nought_to_half_a_million_are_five_equal_steps() {
    let values = counted(&[
        (50_000.0, 1),
        (150_000.0, 2),
        (250_000.0, 3),
        (350_000.0, 4),
        (450_000.0, 5),
        (500_001.0, 1),
    ]);
    let steps = viridis_steps(5);
    let stepped = compose(&spec(&values, "colorScale: quantize\ncolorN: 5\n"));
    let (edges, colours) = steps_of(&stepped);
    assert_eq!(colours.len(), 5, "five steps, not the six tidy ticks give");
    let width = 500_001.0 / 5.0;
    let expected: Vec<f64> = (0..=5).map(|k| width * k as f64).collect();
    assert_edges(&edges, &expected, "equal widths over 0 to 500001");
    assert_eq!(
        counts(&stepped, &steps),
        [1, 2, 3, 4, 6],
        "and each step holds the points that fall in it"
    );
}

/// **AC5.** With `colorDomain: [0, 50]` as well, the five steps are each 10 wide,
/// and a point past the end wears the last step's colour.
#[test]
fn a_fixed_domain_is_cut_before_the_steps_are() {
    let values = counted(&[
        (5.0, 1),
        (15.0, 2),
        (25.0, 3),
        (35.0, 4),
        (45.0, 5),
        (80.0, 6),
    ]);
    let steps = viridis_steps(5);
    let keys = "colorScale: quantize\ncolorN: 5\n";
    let without = compose(&spec(&values, keys));
    assert_edges(
        &steps_of(&without).0,
        &[0.0, 16.0, 32.0, 48.0, 64.0, 80.0],
        "fixture check: the rows' own domain, 0 to 80",
    );
    let fixed = compose(&spec(&values, &format!("colorDomain: [0, 50]\n{keys}")));
    assert_edges(
        &steps_of(&fixed).0,
        &[0.0, 10.0, 20.0, 30.0, 40.0, 50.0],
        "each step 10 wide",
    );
    assert_eq!(
        counts(&fixed, &steps),
        [1, 2, 3, 4, 11],
        "the points at 45 and at 80 are both in the fifth"
    );
}

/// **AC6.** With `colorReverse: true` as well, the first step wears the colour
/// the fifth wore, and the edges read as they did.
#[test]
fn reverse_gives_the_first_step_the_colour_the_fifth_wore() {
    let steps = viridis_steps(5);
    let keys = "colorScale: quantize\ncolorN: 5\n";
    let forward = compose(&spec(&hundred(), keys));
    let reversed = compose(&spec(&hundred(), &format!("colorReverse: true\n{keys}")));
    assert_eq!(
        counts(&forward, &steps),
        HUNDRED_COUNTS,
        "fixture check: forward, the first step wears the scheme's lowest"
    );
    let turned: Vec<[f32; 4]> = steps.iter().rev().copied().collect();
    assert_eq!(steps_of(&reversed).1, turned);
    assert_eq!(
        counts(&reversed, &turned),
        HUNDRED_COUNTS,
        "reversed, the points of the first step wear the colour the fifth wore"
    );
    assert_eq!(
        steps_of(&reversed).0,
        steps_of(&forward).0,
        "and the edges read as they did"
    );
}

/// **AC7.** With a `colorRange` of four colours as well, the plot draws four
/// steps, one in each colour, whatever `colorN` says.
#[test]
fn a_range_of_four_colours_draws_four_steps_whatever_the_count_says() {
    let values = counted(&[(10.0, 1), (30.0, 2), (60.0, 3), (90.0, 4)]);
    let range = "colorRange: ['#ff0000', '#00ff00', '#0000ff', '#ffff00']\n";
    for count in ["", "colorN: 9\n", "colorN: 2\n"] {
        let attrs = format!("colorScale: quantize\n{count}{range}");
        let stepped = compose(&spec(&values, &attrs));
        let (edges, colours) = steps_of(&stepped);
        assert_eq!(colours, [RED, GREEN, BLUE, YELLOW], "{count:?}");
        assert_edges(&edges, &[0.0, 22.5, 45.0, 67.5, 90.0], "four equal steps");
        assert_eq!(
            counts(&stepped, &[RED, GREEN, BLUE, YELLOW]),
            [1, 2, 3, 4],
            "{count:?}: one colour for each step"
        );
    }
}

/// **AC8.** A `colorN` that is not a whole number above zero draws five steps,
/// and the page's warning names the key and the value.
#[test]
fn a_count_that_is_no_count_draws_five_steps_and_the_page_names_it() {
    for (written, shown) in [
        ("0", "0"),
        ("-2", "-2"),
        ("2.5", "2.5"),
        ("five", "five"),
        ("true", "true"),
        ("[3]", "<non-string>"),
        ("1000", "1000"),
    ] {
        let attrs = format!("colorScale: quantize\ncolorN: {written}\n");
        let composed = compose(&spec(&hundred(), &attrs));
        assert_eq!(steps_of(&composed).1.len(), 5, "colorN: {written}");
        let told = about(&composed, "colorN");
        assert_eq!(told.len(), 1, "colorN: {written} is named once: {told:?}");
        assert!(
            told[0].contains("`colorN`") && told[0].contains(&format!("`{shown}`")),
            "the warning names the key and the value as written: {}",
            told[0]
        );
    }
}

/// **AC8, the other half.** A `$param` raises no warning: it draws as the param's
/// value would when the param holds a count, and as a file without the key when it
/// does not.
#[test]
fn a_count_through_a_param_raises_no_warning() {
    let with_param = |param: &str| {
        format!(
            "params:\n  n: {param}\n{}",
            spec(&hundred(), "colorScale: quantize\ncolorN: $n\n")
        )
    };
    let seven = compose(&with_param("7"));
    assert_eq!(
        steps_of(&seven).1.len(),
        7,
        "a param holding a count draws that many steps"
    );
    assert!(about(&seven, "colorN").is_empty());
    let junk = compose(&with_param("banana"));
    assert_eq!(
        steps_of(&junk).1.len(),
        5,
        "a param holding no count draws five"
    );
    assert!(
        about(&junk, "colorN").is_empty(),
        "and the page says nothing: {:?}",
        about(&junk, "colorN")
    );
    let by_scale = compose(&format!(
        "params:\n  s: quantize\n{}",
        spec(&hundred(), "colorScale: $s\ncolorN: 5\n")
    ));
    assert_eq!(
        steps_of(&by_scale).1.len(),
        5,
        "the scale through a param steps too"
    );
}

/// `quantize` is a scale the plot draws, so the page does not warn of it.
#[test]
fn the_page_does_not_warn_of_quantize() {
    let composed = compose(&spec(&hundred(), "colorScale: quantize\ncolorN: 5\n"));
    assert!(
        about(&composed, "colorScale").is_empty(),
        "{:?}",
        about(&composed, "colorScale")
    );
}

/// **AC9.** A plot whose fill is a string column draws as it does today under
/// `colorScale: quantize`, and a plot that sets no `colorScale` draws as it does
/// today whatever `colorN` says.
#[test]
fn a_string_fill_and_a_plot_with_no_scale_draw_as_they_did() {
    let groups = ["coast", "coast", "inland", "valley", "valley", "valley"];
    let plain = compose(&spec_by_string(&groups, ""));
    let keyed = compose(&spec_by_string(
        &groups,
        "colorScale: quantize\ncolorN: 3\n",
    ));
    assert!(
        matches!(
            keyed.plots[0].scales.get(Channel::Fill),
            Some(Scale::Colour { .. })
        ),
        "a string fill stays categorical"
    );
    assert_eq!(painted(&keyed), painted(&plain), "and draws as it did");
    assert_eq!(
        LegendSpec::from_scales(&keyed.plots[0].scales),
        LegendSpec::from_scales(&plain.plots[0].scales),
        "with the legend it had"
    );

    let plain = compose(&spec(&hundred(), ""));
    let counted_only = compose(&spec(&hundred(), "colorN: 3\n"));
    assert!(is_ramp(&counted_only), "a count alone steps nothing");
    assert_eq!(painted(&counted_only), painted(&plain));
    let linear = compose(&spec(&hundred(), "colorScale: linear\ncolorN: 3\n"));
    assert!(is_ramp(&linear));
    assert_eq!(painted(&linear), painted(&plain));
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
    let with = mark_specs("colorScale: quantize\ncolorN: 4\n");
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

/// The legend reads the scale the marks were painted with: its colours are the
/// steps' and its boundaries are the edges that cut the points.
#[test]
fn the_legend_is_the_scale_the_marks_were_painted_with() {
    let composed = compose(&spec(&hundred(), "colorScale: quantize\ncolorN: 5\n"));
    let (edges, colours) = steps_of(&composed);
    let legend = LegendSpec::from_scales(&composed.plots[0].scales).expect("a stepped legend");
    assert_eq!(
        legend,
        LegendSpec::Steps { colours, edges },
        "colours and edges, verbatim"
    );
}
