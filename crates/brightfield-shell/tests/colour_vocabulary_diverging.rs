//! **A plot's `colorScale: diverging` paints a number fill in two arms about a
//! pivot, and the legend shows both arms from pole to pole.**
//!
//! These run the whole composition — spec text, DuckDB, per-plot scales, the
//! scene — and read three things off it: the plot's fill scale, the legend
//! derived from it, and the colour words the scene paints, which are the only
//! place a point's colour is a fact rather than a promise. The oracle for a
//! colour is the design system's own arms and midpoint, and the arm formula
//! written out here, not the renderer's scale.
//!
//! The specs carry their data inline, so the run needs no file beside it.

use brightfield_render::channel::Channel;
use brightfield_render::scale::{Scale, SequentialScheme};
use brightfield_shell::legend::{diverging_strip_colours, LegendSpec};
use brightfield_shell::pipeline::{compose_spec_str, Composed};
use kurbo::{Affine, Circle};
use meridian_design::colour::Rgba;
use meridian_design::viz::{DIVERGING_BLUE_ARM, DIVERGING_MID_LIGHT, DIVERGING_RED_ARM};
use peniko::{Color, Fill};
use vello::Scene;

/// A one-plot spec of dots filled by `v`; `attrs` is the plot's attributes.
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

fn rgba(c: Rgba) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
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

/// The plot's fill scale as `(domain_min, domain_max, pivot)`, when diverging.
fn diverging_of(composed: &Composed) -> Option<(f64, f64, f64)> {
    match composed.plots[0].scales.get(Channel::Fill) {
        Some(Scale::Diverging {
            domain_min,
            domain_max,
            pivot,
            ..
        }) => Some((*domain_min, *domain_max, *pivot)),
        _ => None,
    }
}

/// **AC1.** With `colorScale: diverging` and `colorPivot: 2`, a point at 2 is
/// the midpoint colour, points below 2 are in the blue arm and points above are
/// in the red arm, each nearer its arm's pole the further its value is from 2;
/// and the legend's ramp runs from the blue pole through the midpoint to the red
/// pole.
#[test]
fn a_number_fill_paints_two_arms_about_the_pivot_and_the_legend_shows_both() {
    // Below the pivot the rows reach 2, above it 7, so the domain reaches 7
    // either side and the blue pole is not drawn by a point.
    let values = [0.0, 1.0, 2.0, 3.0, 9.0];
    let composed = compose(&spec(&values, "colorScale: diverging\ncolorPivot: 2\n"));
    let words = painted(&composed);
    let stops = design_stops();

    for v in values {
        let want = packed(about_pivot(&stops, 2.0, 7.0, v));
        assert!(
            words.contains(&want),
            "no point painted the colour {v} takes about the pivot 2"
        );
    }
    assert!(
        words.contains(&packed(rgba(DIVERGING_MID_LIGHT))),
        "the point at the pivot is the midpoint colour"
    );
    assert!(
        words.contains(&packed(rgba(DIVERGING_RED_ARM[4]))),
        "the point at the far end of the red arm is the red pole"
    );
    // The arms: blue below the pivot, red above it.
    for (v, blue) in [(0.0, true), (1.0, true), (3.0, false), (9.0, false)] {
        let [r, _, b, _] = about_pivot(&stops, 2.0, 7.0, v);
        assert_eq!(
            b > r,
            blue,
            "{v} is in the {} arm",
            if blue { "blue" } else { "red" }
        );
    }
    // Nearer the pole the further from the pivot: 0 is further than 1.
    assert_ne!(
        packed(about_pivot(&stops, 2.0, 7.0, 0.0)),
        packed(about_pivot(&stops, 2.0, 7.0, 1.0)),
        "two distances, two colours"
    );

    let Some(LegendSpec::Diverging {
        min,
        max,
        pivot,
        stops: legend_stops,
    }) = LegendSpec::from_scales(&composed.plots[0].scales)
    else {
        panic!("a diverging fill derives a diverging legend");
    };
    assert_eq!(
        (min, max, pivot),
        (-5.0, 9.0, 2.0),
        "the legend's ends and pivot"
    );
    let strips = diverging_strip_colours(&legend_stops);
    assert_eq!(
        strips[0],
        rgba(DIVERGING_BLUE_ARM[0]),
        "the bar starts at the blue pole"
    );
    assert_eq!(
        strips[strips.len() - 1],
        rgba(DIVERGING_RED_ARM[4]),
        "the bar ends at the red pole"
    );
    assert_eq!(
        strips[strips.len() / 2],
        rgba(DIVERGING_MID_LIGHT),
        "the middle of the bar, where the pivot is labelled, is the midpoint colour"
    );
}

/// **AC2.** The domain is even about the pivot: the two ends are the same
/// distance from it, the greater of the two distances the rows reach.
#[test]
fn the_domain_is_even_about_the_pivot_and_reaches_as_far_as_the_rows_do() {
    // (values, written pivot, expected ends). The rows reach 2 below and 7 above
    // the pivot 2, then 5 below and 1 above the pivot 0.
    for (values, attrs, ends) in [
        (
            [0.0, 1.0, 2.0, 3.0, 9.0],
            "colorScale: diverging\ncolorPivot: 2\n",
            (-5.0, 9.0),
        ),
        (
            [-5.0, -1.0, 0.0, 0.5, 1.0],
            "colorScale: diverging\ncolorPivot: 0\n",
            (-5.0, 5.0),
        ),
    ] {
        let composed = compose(&spec(&values, attrs));
        let Some((lo, hi, pivot)) = diverging_of(&composed) else {
            panic!("{attrs}: the fill scale is not diverging");
        };
        assert_eq!((lo, hi), ends, "{attrs}: the ends");
        assert_eq!(
            pivot - lo,
            hi - pivot,
            "{attrs}: the ends are as far from the pivot"
        );
    }
}

/// **AC3.** With no `colorPivot` the pivot is 0 when the rows hold a negative
/// value and a positive one, and the rows' median otherwise.
#[test]
fn the_pivot_is_zero_across_the_origin_and_the_median_otherwise() {
    for (values, pivot) in [
        // A negative and a positive: zero, not the median 1.
        (vec![-3.0, 1.0, 1.0, 10.0], 0.0),
        // Positive only: the median.
        (vec![1.0, 2.0, 9.0], 2.0),
        // An even count: the mean of the middle two.
        (vec![5.0, 6.0, 7.0, 8.0], 6.5),
        // Zero is the smallest, not a negative: the median, not zero.
        (vec![0.0, 4.0, 8.0], 4.0),
        // Negative only: the median.
        (vec![-5.0, -3.0, -1.0], -3.0),
    ] {
        let composed = compose(&spec(&values, "colorScale: diverging\n"));
        let Some((lo, hi, got)) = diverging_of(&composed) else {
            panic!("{values:?}: the fill scale is not diverging");
        };
        assert_eq!(got, pivot, "{values:?}: the pivot");
        assert_eq!(
            got - lo,
            hi - got,
            "{values:?}: the ends are as far from the pivot"
        );
    }
}

/// **AC4.** With no `colorScheme` the arms are the design system's blue and
/// red about its midpoint; `rdbu` is ColorBrewer's, red low and blue high;
/// `viridis` and `blues` are those ramps about the pivot, the middle colour at
/// the pivot.
#[test]
fn the_arms_are_the_designs_by_default_and_the_named_scheme_otherwise() {
    // Symmetric about 0, so the domain's ends are the extreme values drawn.
    let values = [-3.0, -1.5, 0.0, 1.5, 3.0];
    let ends_and_middle = |attrs: &str| -> (Composed, [u32; 3]) {
        let composed = compose(&spec(&values, attrs));
        let words = painted(&composed);
        let Some(Scale::Diverging { stops, .. }) = composed.plots[0].scales.get(Channel::Fill)
        else {
            panic!("{attrs}: the fill scale is not diverging");
        };
        let at = |v: f64| {
            let word = packed(about_pivot(stops, 0.0, 3.0, v));
            assert!(
                words.contains(&word),
                "{attrs}: no point painted {v}'s colour"
            );
            word
        };
        let ends = [at(-3.0), at(0.0), at(3.0)];
        (composed, ends)
    };

    let (_, got) = ends_and_middle("colorScale: diverging\n");
    assert_eq!(
        got,
        [
            packed(rgba(DIVERGING_BLUE_ARM[0])),
            packed(rgba(DIVERGING_MID_LIGHT)),
            packed(rgba(DIVERGING_RED_ARM[4])),
        ],
        "no colorScheme: blue pole, midpoint, red pole"
    );

    let (_, got) = ends_and_middle("colorScale: diverging\ncolorScheme: rdbu\n");
    let hex = |h: u32| {
        let c = |shift: u32| ((h >> shift) & 0xff) as f32 / 255.0;
        [c(16), c(8), c(0), 1.0]
    };
    assert_eq!(
        got,
        [packed(hex(0x67001f)), packed(hex(0xf7f7f7)), packed(hex(0x053061))],
        "rdbu: ColorBrewer's dark red at the low end, white at the pivot, dark blue at the high end"
    );

    for scheme in [SequentialScheme::Viridis, SequentialScheme::Blues] {
        let stops = scheme.stops();
        let (_, got) = ends_and_middle(&format!(
            "colorScale: diverging\ncolorScheme: {}\n",
            scheme.wire_name()
        ));
        assert_eq!(
            got,
            [
                packed(stops[0]),
                packed(stops[stops.len() / 2]),
                packed(stops[stops.len() - 1]),
            ],
            "{}: its low end, its middle colour at the pivot, its high end",
            scheme.wire_name()
        );
    }
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

/// **AC5.** A `colorScale` brightfield does not draw, `quantile` say, draws the
/// linear ramp and the page's warning names the key and the value; `linear`, a
/// `$param`, `null` and `diverging` raise none.
#[test]
fn a_scale_brightfield_does_not_draw_draws_the_linear_ramp_and_the_page_names_it() {
    let values = [0.825, 2.291, 1.5, 4.9, 3.2];
    let none = compose(&spec(&values, ""));
    let quantile = compose(&spec(&values, "colorScale: quantile\n"));

    assert_eq!(
        painted(&quantile),
        painted(&none),
        "quantile draws the linear ramp"
    );
    let told = about(&quantile, "colorScale");
    assert_eq!(told.len(), 1, "one line about the key: {told:?}");
    assert!(
        told[0].contains("`colorScale`") && told[0].contains("`quantile`"),
        "the warning names the key and the value: {}",
        told[0]
    );

    let with_param = |params: &str, attrs: &str| {
        let source = format!("{params}{}", spec(&values, attrs));
        compose(&source)
    };
    for (what, composed) in [
        ("none", &none),
        ("linear", &compose(&spec(&values, "colorScale: linear\n"))),
        ("null", &compose(&spec(&values, "colorScale: null\n"))),
        (
            "a param",
            &with_param("params:\n  s: quantile\n", "colorScale: $s\n"),
        ),
        (
            "diverging",
            &compose(&spec(&values, "colorScale: diverging\n")),
        ),
    ] {
        assert!(
            about(composed, "colorScale").is_empty(),
            "{what}: {:?}",
            about(composed, "colorScale")
        );
    }
    assert_eq!(
        painted(&compose(&spec(&values, "colorScale: linear\n"))),
        painted(&none),
        "linear draws what no colorScale draws"
    );
}

/// A `$param` holding `diverging`, or a pivot, is read as the param stands, and
/// raises no warning; a pivot that is no number is named.
#[test]
fn a_param_is_read_for_the_scale_and_the_pivot_and_a_pivot_that_is_no_number_is_named() {
    let values = [0.0, 1.0, 2.0, 3.0, 9.0];
    let source = format!(
        "params:\n  s: diverging\n  p: 2\n{}",
        spec(&values, "colorScale: $s\ncolorPivot: $p\n")
    );
    let composed = compose(&source);
    assert_eq!(
        diverging_of(&composed),
        Some((-5.0, 9.0, 2.0)),
        "the params hold diverging and a pivot of 2"
    );
    assert!(about(&composed, "colorScale").is_empty() && about(&composed, "colorPivot").is_empty());

    // No number to read, so the pivot is chosen from the rows: their median, 4,
    // and the domain reaches 5 either side of it.
    let values = [0.0, 1.0, 4.0, 5.0, 9.0];
    let word = compose(&spec(
        &values,
        "colorScale: diverging\ncolorPivot: middle\n",
    ));
    let told = about(&word, "colorPivot");
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(
        told[0].contains("`colorPivot`") && told[0].contains("`middle`"),
        "{}",
        told[0]
    );
    assert_eq!(
        diverging_of(&word),
        Some((-1.0, 9.0, 4.0)),
        "a pivot that is no number is the plot asking for the pivot to be chosen"
    );
}

/// The cell, heatmap and raster specs the unchanged-draw test reads.
fn mark_specs() -> Vec<(&'static str, String)> {
    let rows: String = (0..6)
        .flat_map(|day| (0..4).map(move |hour| (day, hour)))
        .map(|(day, hour)| format!("    - {{ day: {day}, hour: {hour}, x: {hour}, y: {day} }}\n"))
        .collect();
    let plot = |layer: &str, attrs: &str| {
        format!("data:\n  t:\n{rows}plot:\n{layer}width: 420\nheight: 300\n{attrs}")
    };
    let layers = [
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
    ];
    layers
        .into_iter()
        .flat_map(|(name, layer)| {
            [
                (name, plot(layer, "")),
                (name, plot(layer, "colorScale: diverging\n")),
            ]
        })
        .collect()
}

/// **AC6.** A fill that is a string column, and a cell, a heatmap and a raster,
/// draw under `colorScale: diverging` what they draw without it.
#[test]
fn a_string_fill_a_cell_a_heatmap_and_a_raster_draw_as_they_do_without_it() {
    let rows: String = ["a", "b", "c", "a", "b"]
        .iter()
        .enumerate()
        .map(|(i, g)| format!("    - {{ x: {i}, y: {}, g: {g} }}\n", i * 3))
        .collect();
    let by_string = |attrs: &str| {
        compose(&format!(
            "data:\n  t:\n{rows}plot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n    fill: g\nwidth: 420\nheight: 300\n{attrs}"
        ))
    };
    let plain = by_string("");
    let diverging = by_string("colorScale: diverging\ncolorPivot: 1\n");
    assert!(
        matches!(
            diverging.plots[0].scales.get(Channel::Fill),
            Some(Scale::Colour { .. })
        ),
        "a string fill keeps its categorical scale"
    );
    assert_eq!(
        painted(&diverging),
        painted(&plain),
        "a string fill draws as it does"
    );
    assert_eq!(
        LegendSpec::from_scales(&diverging.plots[0].scales),
        LegendSpec::from_scales(&plain.plots[0].scales),
        "and its legend"
    );

    let specs = mark_specs();
    for pair in specs.chunks(2) {
        let (name, without) = &pair[0];
        let (_, with) = &pair[1];
        let (without, with) = (compose(without), compose(with));
        assert!(
            !painted(&without).is_empty(),
            "{name}: the fixture draws something"
        );
        assert_eq!(
            painted(&with),
            painted(&without),
            "{name}: draws as it does without colorScale: diverging"
        );
        assert!(
            diverging_of(&with).is_none(),
            "{name}: its fill is not a diverging scale"
        );
    }
}
