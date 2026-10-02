//! **A plot's `colorScheme` reaches the ramp its marks are painted along, and
//! the legend that ramp's stops give.**
//!
//! The shell drew every mark through the default renderers, so a dot filled by
//! a number column drew viridis under `colorScheme: blues`. These tests run the
//! whole composition — spec text, DuckDB, per-plot scales, the scene — and read
//! three things off it: the stops of the plot's fill scale, the stops of the
//! legend derived from that scale, and the colour words the scene paints, which
//! are the only place a point's colour is a fact rather than a promise.
//!
//! The specs carry their data inline, so the run needs no file beside it.

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_render::channel::Channel;
use brightfield_render::scale::{Scale, SequentialScheme};
use brightfield_shell::legend::LegendSpec;
use brightfield_shell::pipeline::{compose_spec_str, Composed, LiveDashboard};
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::SpecValue;
use brightfield_sql::ir::ScalarValue;
use kurbo::{Affine, Circle};
use peniko::{Color, Fill};
use vello::Scene;

/// The column the dots are filled by. The values are positive, so the ramp is
/// anchored at zero and ends at the maximum, 4.9.
const VALUES: [f64; 5] = [0.825, 2.291, 1.5, 4.9, 3.2];

/// The three schemes that are not the default, and the default.
const NAMED: [(&str, SequentialScheme); 4] = [
    ("viridis", SequentialScheme::Viridis),
    ("blues", SequentialScheme::Blues),
    ("turbo", SequentialScheme::Turbo),
    ("meridian", SequentialScheme::Meridian),
];

fn rows() -> String {
    VALUES
        .iter()
        .enumerate()
        .map(|(i, v)| format!("    - {{ x: {i}, y: {}, v: {v}, g: g{} }}\n", i * 3, i % 2))
        .collect()
}

/// A one-plot spec: `layers` are the plot's marks, `attrs` its attributes.
fn spec(layers: &str, attrs: &str) -> String {
    format!(
        "data:\n  t:\n{}plot:\n{layers}width: 420\nheight: 300\n{attrs}",
        rows()
    )
}

fn dot(fill: &str) -> String {
    format!("  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n    fill: {fill}\n")
}

fn compose(source: &str) -> Composed {
    compose_spec_str(source, None)
        .unwrap_or_else(|e| panic!("the spec must compose: {e}\n{source}"))
}

/// The stops of the plot's fill scale, when it is a ramp.
fn fill_stops(composed: &Composed, plot: usize) -> Option<Vec<[f32; 4]>> {
    match composed.plots[plot].scales.get(Channel::Fill) {
        Some(Scale::Sequential { stops, .. }) => Some(stops.clone()),
        _ => None,
    }
}

/// The stops of the legend the plot's scales derive.
fn legend_stops(composed: &Composed, plot: usize) -> Option<Vec<[f32; 4]>> {
    match LegendSpec::from_scales(&composed.plots[plot].scales) {
        Some(LegendSpec::Sequential { stops, .. }) => Some(stops),
        _ => None,
    }
}

/// A colour as the scene encodes it, by drawing one circle in it.
fn packed(colour: Color) -> u32 {
    let mut scene = Scene::new();
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        colour,
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

/// The colour word a value takes along `scheme`'s ramp over `[0, max]`.
fn along(scheme: SequentialScheme, max: f64, value: f64) -> u32 {
    let ramp = Scale::Sequential {
        domain_min: 0.0,
        domain_max: max,
        stops: scheme.stops(),
    };
    packed(Color::new(ramp.map_continuous(value)))
}

/// **AC1.** A dot filled by a number column under `colorScheme: blues`,
/// `turbo` or `meridian` paints each point along that scheme's ramp, and the
/// legend's ramp is the same stops. With `viridis` and with no `colorScheme`
/// the plot draws what it draws without one.
#[test]
fn a_dot_filled_by_a_number_paints_and_draws_its_legend_along_the_scheme_it_names() {
    let none = compose(&spec(&dot("v"), ""));
    assert_eq!(
        fill_stops(&none, 0),
        Some(SequentialScheme::Viridis.stops()),
        "no colorScheme is viridis"
    );

    for (name, scheme) in NAMED {
        let composed = compose(&spec(&dot("v"), &format!("colorScheme: {name}\n")));
        assert_eq!(
            fill_stops(&composed, 0),
            Some(scheme.stops()),
            "{name}: the fill ramp is the scheme's stops"
        );
        assert_eq!(
            legend_stops(&composed, 0),
            Some(scheme.stops()),
            "{name}: the legend's ramp is the scheme's stops"
        );
        let words = painted(&composed);
        for v in VALUES {
            assert!(
                words.contains(&along(scheme, 4.9, v)),
                "{name}: no point painted the colour {v} takes along the {name} ramp"
            );
        }
        if scheme == SequentialScheme::Viridis {
            assert_eq!(
                words,
                painted(&none),
                "viridis draws what no colorScheme draws"
            );
        } else {
            assert_ne!(words, painted(&none), "{name} drew viridis");
        }
    }
}

/// The two-tile spec the redraw is read on: a scatter whose x-range is swept,
/// and a dot plot filled by a number column that `filterBy` the sweep. `attrs`
/// is the second plot's attributes.
fn swept_spec(attrs: &str) -> String {
    format!(
        "params:\n  brush: {{ select: crossfilter }}\ndata:\n  t:\n{}hconcat:\n  \
         - plot:\n      - mark: dot\n        data: {{ from: t }}\n        x: x\n        y: y\n      \
         - select: intervalX\n        as: $brush\n    width: 300\n    height: 260\n  \
         - plot:\n      - mark: dot\n        data: {{ from: t, filterBy: $brush }}\n        \
         x: x\n        y: y\n        fill: v\n{attrs}    width: 300\n    height: 260\n",
        rows()
    )
}

/// **AC2.** After a range is swept on the other tile, the plot is redrawn in
/// the scheme it names — its points and its legend — and not in viridis.
#[test]
fn a_plot_redrawn_after_a_range_is_swept_elsewhere_keeps_its_scheme() {
    let source = swept_spec("    colorScheme: blues\n");
    let mut live = LiveDashboard::load_str(&source, None)
        .unwrap_or_else(|e| panic!("the spec must load live: {e}\n{source}"));
    let resting = live.present().expect("the resting composition");
    assert_eq!(resting.plots.len(), 2, "two tiles");
    assert_eq!(
        fill_stops(&resting, 1),
        Some(SequentialScheme::Blues.stops()),
        "at rest, the plot draws blues"
    );
    let swept_on = ComponentPath(resting.plots[0].path.clone());

    let swept = live
        .apply(Interaction::Select {
            name: "brush".to_string(),
            contributor: swept_on,
            predicate: SqlPredicate::Interval {
                column: "x".to_string(),
                lo: ScalarValue::Float(1.0),
                hi: ScalarValue::Float(3.0),
                meta: None,
            },
        })
        .expect("the sweep recomposes");

    assert_eq!(
        fill_stops(&swept, 1),
        Some(SequentialScheme::Blues.stops()),
        "after the sweep, the fill ramp is blues"
    );
    assert_eq!(
        legend_stops(&swept, 1),
        Some(SequentialScheme::Blues.stops()),
        "after the sweep, the legend's ramp is blues"
    );
    let Some(Scale::Sequential { domain_max, .. }) = swept.plots[1].scales.get(Channel::Fill)
    else {
        panic!("the swept plot has no fill ramp");
    };
    let words = painted(&swept);
    // x in {1, 2, 3} survive the sweep: v is 2.291, 1.5 and 4.9.
    for v in [2.291, 1.5, 4.9] {
        assert!(
            words.contains(&along(SequentialScheme::Blues, *domain_max, v)),
            "after the sweep, no point painted the colour {v} takes along the blues ramp"
        );
    }
}

/// The count grid's tile: a `cell` mark crossing two string columns, counted.
/// The four crossings hold 3, 1, 2 and 4 rows, so the counts differ and the
/// ramp has somewhere to go. `attrs` is the plot's attributes.
fn count_grid_spec(attrs: &str) -> String {
    let crossings = [
        ("mon", "a", 3),
        ("mon", "b", 1),
        ("tue", "a", 2),
        ("tue", "b", 4),
    ];
    let rows: String = crossings
        .iter()
        .flat_map(|(day, hour, n)| (0..*n).map(move |_| (day, hour)))
        .map(|(day, hour)| format!("    - {{ day: {day}, hour: {hour} }}\n"))
        .collect();
    format!(
        "data:\n  t:\n{rows}plot:\n  - mark: cell\n    data: {{ from: t }}\n    x: hour\n    y: day\n    \
         fill: {{ count: }}\nwidth: 420\nheight: 300\n{attrs}"
    )
}

/// **AC3.** A `cell` mark with a counted fill — the count grid's mark — on a
/// plot that names `colorScheme: blues` paints along blues: the ramp, the
/// legend and each cell's colour.
#[test]
fn a_cell_with_a_counted_fill_paints_along_the_scheme_its_plot_names() {
    let none = compose(&count_grid_spec(""));
    assert_eq!(
        fill_stops(&none, 0),
        Some(SequentialScheme::Viridis.stops()),
        "no colorScheme is viridis"
    );
    let blues = compose(&count_grid_spec("colorScheme: blues\n"));
    assert_eq!(
        fill_stops(&blues, 0),
        Some(SequentialScheme::Blues.stops()),
        "the counted cells' ramp is blues"
    );
    assert_eq!(
        legend_stops(&blues, 0),
        Some(SequentialScheme::Blues.stops()),
        "the counted cells' legend is blues"
    );
    let Some(Scale::Sequential { domain_max, .. }) = blues.plots[0].scales.get(Channel::Fill)
    else {
        panic!("the count grid has no fill ramp");
    };
    let words = painted(&blues);
    for count in [1.0, 2.0, 3.0, 4.0] {
        assert!(
            words.contains(&along(SequentialScheme::Blues, *domain_max, count)),
            "no cell painted the colour a count of {count} takes along the blues ramp"
        );
    }
    assert_ne!(painted(&blues), painted(&none), "the cells drew viridis");
}

/// **AC4.** A `colorScheme` given as a `$param` draws as the param's value
/// would when the param holds one of the four names, and as a plot with no
/// `colorScheme` when it does not — and a redraw after the param is written
/// draws in the scheme the param then holds.
#[test]
fn a_param_scheme_draws_as_its_value_would_and_as_none_when_it_names_no_scheme() {
    let none = compose(&spec(&dot("v"), ""));
    for (name, _) in NAMED {
        let by_literal = compose(&spec(&dot("v"), &format!("colorScheme: {name}\n")));
        let by_param = compose(&format!(
            "params:\n  scheme: {name}\n{}",
            spec(&dot("v"), "colorScheme: $scheme\n")
        ));
        assert_eq!(
            painted(&by_param),
            painted(&by_literal),
            "a param holding {name} draws as {name} written in"
        );
    }
    for held in ["ylgnbu", "Blues", "\"\"", "4"] {
        let by_param = compose(&format!(
            "params:\n  scheme: {held}\n{}",
            spec(&dot("v"), "colorScheme: $scheme\n")
        ));
        assert_eq!(
            painted(&by_param),
            painted(&none),
            "a param holding {held}, which is no scheme brightfield draws, draws as no colorScheme"
        );
    }

    let source = format!(
        "params:\n  scheme: viridis\n{}",
        spec(&dot("v"), "colorScheme: $scheme\n")
    );
    let mut live = LiveDashboard::load_str(&source, None).expect("the spec loads live");
    let resting = live.present().expect("the resting composition");
    assert_eq!(
        fill_stops(&resting, 0),
        Some(SequentialScheme::Viridis.stops()),
        "the param holds viridis at rest"
    );
    let written = live
        .apply(Interaction::SetParam {
            name: "scheme".to_string(),
            value: SpecValue::String("turbo".to_string()),
        })
        .expect("writing the param recomposes");
    assert_eq!(
        fill_stops(&written, 0),
        Some(SequentialScheme::Turbo.stops()),
        "after the param is written, the plot draws turbo"
    );
}

/// **AC5.** A plot whose fill is a string column, and one whose fill is a
/// colour literal, draws as it does without a `colorScheme` under each of the
/// four names.
#[test]
fn a_string_fill_and_a_colour_literal_draw_the_same_under_each_scheme() {
    for (label, fill) in [
        ("a string column", "g"),
        ("a colour literal", "\"#aaaaaa\""),
    ] {
        let none = compose(&spec(&dot(fill), ""));
        for (name, _) in NAMED {
            let named = compose(&spec(&dot(fill), &format!("colorScheme: {name}\n")));
            assert_eq!(
                painted(&named),
                painted(&none),
                "{label} under {name} draws differently from the plot with no colorScheme"
            );
        }
    }
}
