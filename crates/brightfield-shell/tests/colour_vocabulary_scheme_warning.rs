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

use brightfield_engine::coordinator::Interaction;
use brightfield_render::channel::Channel;
use brightfield_render::scale::{Scale, SequentialScheme};
use brightfield_shell::pipeline::{compose_spec_str, Composed, LiveDashboard};
use brightfield_spec::layout::DRAWN_COLOUR_SCHEMES;
use brightfield_spec::SpecValue;

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

// ---------------------------------------------------------------------------
// A `colorScheme` given through a `$param`
// ---------------------------------------------------------------------------

/// The params block that declares `scheme` holding `value`, as YAML writes it.
fn scheme_param(value: &str) -> String {
    format!("params:\n  scheme: {value}\n")
}

/// **Held undrawn.** A `colorScheme` that is a `$param` holding a name outside the drawn
/// five draws the default ramp, and the page's warning is the line the same
/// value written in the file raises — the key and the value, word for word. The
/// names are the two the vendored specs give (`pubugn`, `observable10`), a
/// wrong-case drawn name, and a value that is no name.
#[test]
fn a_param_holding_an_undrawn_name_draws_viridis_and_the_page_names_it_as_the_file_does() {
    let none = compose(&spec("", ""));
    for held in ["magma", "pubugn", "observable10", "Viridis", "5"] {
        let written = compose(&spec("", &format!("colorScheme: {held}\n")));
        let through = compose(&spec(&scheme_param(held), "colorScheme: $scheme\n"));

        assert_eq!(
            fill_stops(&through),
            Some(SequentialScheme::Viridis.stops()),
            "{held} through a param draws viridis"
        );
        assert_eq!(
            painted(&through),
            painted(&none),
            "{held} through a param paints the default's scene"
        );

        let told = about_the_scheme(&through);
        assert_eq!(told.len(), 1, "{held}: one line about the key: {told:?}");
        assert!(
            told[0].contains("`colorScheme`") && told[0].contains(&format!("`{held}`")),
            "{held}: the warning names the key and the value: {}",
            told[0]
        );
        assert_eq!(
            told,
            about_the_scheme(&written),
            "{held}: a param holding it is named as the file writing it is"
        );
    }

    // A list is no name either, and is shown as the file shows it.
    let list = compose(&spec(
        "params:\n  scheme: [a, b]\n",
        "colorScheme: $scheme\n",
    ));
    assert_eq!(
        about_the_scheme(&list),
        about_the_scheme(&compose(&spec("", "colorScheme: [a, b]\n"))),
        "a list held by a param is named as a list written is"
    );
    assert_eq!(about_the_scheme(&list).len(), 1);
}

/// **Held drawn, and written.** A param holding a name brightfield draws raises nothing and the plot
/// draws that ramp, for each name; and writing the param to a drawn name clears
/// the warning and moves the plot to that ramp, while writing it back to an
/// undrawn name names the new value again.
#[test]
fn writing_the_param_to_a_drawn_name_clears_the_warning_and_draws_that_ramp() {
    for name in DRAWN_COLOUR_SCHEMES {
        let held = compose(&spec(&scheme_param(name), "colorScheme: $scheme\n"));
        assert!(
            about_the_scheme(&held).is_empty(),
            "{name} is drawn and a param holding it must not be warned of: {:?}",
            about_the_scheme(&held)
        );
    }
    for (name, scheme) in NAMED {
        let held = compose(&spec(&scheme_param(name), "colorScheme: $scheme\n"));
        assert_eq!(
            fill_stops(&held),
            Some(scheme.stops()),
            "a param holding {name} draws its ramp"
        );
    }

    let write = |live: &mut LiveDashboard, value: &str| -> Composed {
        live.apply(Interaction::SetParam {
            name: "scheme".to_string(),
            value: SpecValue::String(value.to_string()),
        })
        .expect("writing the param recomposes")
    };
    let mut live = LiveDashboard::load_str(
        &spec(&scheme_param("magma"), "colorScheme: $scheme\n"),
        None,
    )
    .expect("the spec loads live");

    let resting = live.present().expect("the resting composition");
    assert_eq!(
        fill_stops(&resting),
        Some(SequentialScheme::Viridis.stops()),
        "magma at rest draws the default"
    );
    assert_eq!(
        about_the_scheme(&resting).len(),
        1,
        "magma at rest is named: {:?}",
        about_the_scheme(&resting)
    );

    let drawn = write(&mut live, "blues");
    assert!(
        about_the_scheme(&drawn).is_empty(),
        "blues written clears the warning: {:?}",
        about_the_scheme(&drawn)
    );
    assert_eq!(
        fill_stops(&drawn),
        Some(SequentialScheme::Blues.stops()),
        "blues written draws blues"
    );

    let again = write(&mut live, "plasma");
    let told = about_the_scheme(&again);
    assert_eq!(told.len(), 1, "plasma written is named: {told:?}");
    assert!(
        told[0].contains("`plasma`") && !told[0].contains("`magma`"),
        "the warning names the value the param holds now: {}",
        told[0]
    );
    assert_eq!(
        fill_stops(&again),
        Some(SequentialScheme::Viridis.stops()),
        "plasma draws the default"
    );
}

/// **No value to judge.** A `colorScheme` param the file never declares, one that holds a
/// selection, and one that holds `null` draw as a file without the key and
/// raise no warning.
#[test]
fn a_param_nobody_declared_or_holding_a_selection_or_null_raises_no_warning() {
    let none = compose(&spec("", ""));
    for (what, params, value) in [
        ("an undeclared param", "", "$nobody"),
        (
            "a selection",
            "params:\n  s:\n    select: intersect\n",
            "$s",
        ),
        ("a param that holds null", "params:\n  s: null\n", "$s"),
    ] {
        let through = compose(&spec(params, &format!("colorScheme: {value}\n")));
        assert!(
            about_the_scheme(&through).is_empty(),
            "{what}: {:?}",
            about_the_scheme(&through)
        );
        assert_eq!(
            fill_stops(&through),
            fill_stops(&none),
            "{what} draws the default ramp"
        );
        assert_eq!(
            painted(&through),
            painted(&none),
            "{what} paints the default's scene"
        );
    }
}
