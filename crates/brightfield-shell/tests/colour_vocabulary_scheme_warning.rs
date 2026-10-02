//! **A plot whose `colorScheme` brightfield cannot draw draws the default
//! ramp, and the page's warning names the key and the value; one it draws
//! draws its own ramp and raises no warning.**
//!
//! These run the whole composition — spec text, DuckDB, per-plot scales, the
//! scene — and read two things off it: the ramp the plot's fill scale and the
//! painted scene carry, and the diagnostics the page is handed. Together they
//! show the warning is raised exactly when the drawing falls back, and not on
//! some other list: the parser judges against the names the renderer draws.
//!
//! The specs carry their data inline, so the run needs no file beside it.

use brightfield_render::channel::Channel;
use brightfield_render::scale::{Scale, SequentialScheme};
use brightfield_shell::pipeline::{compose_spec_str, Composed};

/// The column the dots are filled by.
const VALUES: [f64; 5] = [0.825, 2.291, 1.5, 4.9, 3.2];

/// The schemes brightfield draws, by the name a spec gives them.
const NAMED: [(&str, SequentialScheme); 4] = [
    ("viridis", SequentialScheme::Viridis),
    ("blues", SequentialScheme::Blues),
    ("turbo", SequentialScheme::Turbo),
    ("meridian", SequentialScheme::Meridian),
];

/// A one-plot spec filled by a number column; `params` precedes the data and
/// `attrs` is the plot's attributes.
fn spec(params: &str, attrs: &str) -> String {
    let rows: String = VALUES
        .iter()
        .enumerate()
        .map(|(i, v)| format!("    - {{ x: {i}, y: {}, v: {v} }}\n", i * 3))
        .collect();
    format!(
        "{params}data:\n  t:\n{rows}plot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n    fill: v\nwidth: 420\nheight: 300\n{attrs}"
    )
}

fn compose(source: &str) -> Composed {
    compose_spec_str(source, None)
        .unwrap_or_else(|e| panic!("the spec must compose: {e}\n{source}"))
}

/// The stops of the plot's fill scale, when it is a ramp.
fn fill_stops(composed: &Composed) -> Option<Vec<[f32; 4]>> {
    match composed.plots[0].scales.get(Channel::Fill) {
        Some(Scale::Sequential { stops, .. }) => Some(stops.clone()),
        _ => None,
    }
}

/// The colour words the whole composed scene paints.
fn painted(composed: &Composed) -> Vec<u32> {
    composed.scene.encoding().draw_data.to_vec()
}

/// What the page is told about the key, as it words it.
fn about_the_scheme(composed: &Composed) -> Vec<String> {
    composed
        .diagnostics
        .diagnostics
        .iter()
        .filter(|d| d.wire_name == "colorScheme")
        .map(ToString::to_string)
        .collect()
}

/// **AC1.** A `colorScheme` brightfield does not draw, `magma` say, draws the
/// default ramp — the stops and the painted scene are what no `colorScheme`
/// draws — and the page's warning names the key and the value.
#[test]
fn a_scheme_brightfield_does_not_draw_draws_viridis_and_the_page_names_it() {
    let none = compose(&spec("", ""));
    let magma = compose(&spec("", "colorScheme: magma\n"));

    assert_eq!(
        fill_stops(&magma),
        Some(SequentialScheme::Viridis.stops()),
        "magma draws viridis"
    );
    assert_eq!(
        painted(&magma),
        painted(&none),
        "the scene is the default's"
    );

    let told = about_the_scheme(&magma);
    assert_eq!(told.len(), 1, "one line about the key: {told:?}");
    assert!(
        told[0].contains("`colorScheme`") && told[0].contains("`magma`"),
        "the warning names the key and the value: {}",
        told[0]
    );
    assert!(
        about_the_scheme(&none).is_empty(),
        "no key, no warning: {:?}",
        about_the_scheme(&none)
    );
}

/// **AC2.** A `colorScheme` given as a `$param`, and one given as `null`, raise
/// no warning; the param's plot draws the scheme the param holds.
#[test]
fn a_param_and_null_raise_no_warning_and_the_param_draws_the_scheme_it_holds() {
    let param = compose(&spec(
        "params:\n  scheme: blues\n",
        "colorScheme: $scheme\n",
    ));
    assert!(
        about_the_scheme(&param).is_empty(),
        "a param: {:?}",
        about_the_scheme(&param)
    );
    assert_eq!(
        fill_stops(&param),
        Some(SequentialScheme::Blues.stops()),
        "the param's plot draws blues"
    );

    let null = compose(&spec("", "colorScheme: null\n"));
    assert!(
        about_the_scheme(&null).is_empty(),
        "null: {:?}",
        about_the_scheme(&null)
    );
}

/// **AC3.** The names the warning accepts and the names the renderer draws are
/// one list: each of `viridis`, `blues`, `turbo` and `meridian` raises no
/// warning and draws its own ramp. A name the warning would accept that the
/// renderer did not draw would show here as the default ramp under a name that
/// is not the default's.
#[test]
fn each_scheme_brightfield_draws_raises_no_warning_and_draws_its_own_ramp() {
    let default_stops = SequentialScheme::Viridis.stops();
    for (name, scheme) in NAMED {
        let composed = compose(&spec("", &format!("colorScheme: {name}\n")));
        assert!(
            about_the_scheme(&composed).is_empty(),
            "{name} is drawn and must not be warned of: {:?}",
            about_the_scheme(&composed)
        );
        assert_eq!(
            fill_stops(&composed),
            Some(scheme.stops()),
            "{name} draws its own ramp"
        );
        if scheme != SequentialScheme::Viridis {
            assert_ne!(
                fill_stops(&composed),
                Some(default_stops.clone()),
                "{name} drew the default ramp"
            );
        }
    }
}
