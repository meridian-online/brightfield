//! **An axis key a plot gives through a `$param` draws as the value the param
//! holds, and a value the key refuses is named as the same value written in the
//! file is.**
//!
//! `xTicks`, `xTickFormat`, `grid`, `xZero`, `xNice` and `xReverse`, and their y
//! forms, were read from the plot's own attributes. A `$param` on any of them
//! drew brightfield's default, and the banner said nothing, because the parser
//! reads `$name` as a deferral and no later step judged what the param held.
//!
//! Every comparison is between two compositions of the same spec, one with the
//! value written in the key and one with the key a `$param` that holds it. A
//! drawing is compared as the whole scene encoding — paths, draw data,
//! transforms, styles and glyph runs — so a tick, a rule, a label or a mark in
//! the wrong place fails it, and a banner is compared line for line. Each arm
//! meant to move the plot is paired with the same plot with no key, because a
//! fixture whose default is the value asked for would pass without the param
//! being read.

use brightfield_engine::coordinator::Interaction;
use brightfield_shell::pipeline::{Composed, LiveDashboard};
use brightfield_spec::SpecValue;

// ---------------------------------------------------------------------------
// The fixture
// ---------------------------------------------------------------------------

/// Ten rows whose x runs 43 to 97 and whose y runs 40 to 85, so neither axis
/// holds zero and neither ends on a round number: `Zero` and `Nice` each have
/// somewhere to move it. `day` is a date column for a date format, `name` a
/// column of names. `PARAMS` marks where the `params:` block goes, `X` and `Y`
/// the columns the dots take, `ATTRS` the plot attributes.
const TEMPLATE: &str = r#"
PARAMS
data:
  d:
    query: "SELECT i * 6 + 43 AS a, i * 5 + 40 AS b, DATE '2024-03-01' + CAST(i AS INTEGER) * 40 AS day, 'g' || CAST(i AS VARCHAR) AS name, -120.0 + i AS lon, 30.0 + i AS lat FROM range(10) t(i)"
plot:
  - mark: dot
    data: { from: d }
    x: X
    y: Y
width: 600
height: 300
ATTRS
"#;

fn source(params: &str, x: &str, y: &str, attrs: &str) -> String {
    TEMPLATE
        .replace("PARAMS", params)
        .replace("    x: X", &format!("    x: {x}"))
        .replace("    y: Y", &format!("    y: {y}"))
        .replace("ATTRS", attrs)
}

fn compose_source(source: &str) -> Composed {
    LiveDashboard::load_str(source, None)
        .expect("the spec loads")
        .present()
        .expect("the spec composes")
}

fn compose(params: &str, x: &str, y: &str, attrs: &str) -> Composed {
    compose_source(&source(params, x, y, attrs))
}

/// Everything the scene draws, as one comparable string: the coordinates of every
/// path, the draw data that carries each fill and stroke, the transforms and
/// styles, and the glyphs with their places. `Encoding` has no `Debug` of its
/// own, so the streams are named here.
fn drawn(composed: &Composed) -> String {
    let encoding = composed.scene.encoding();
    format!(
        "{:?}\n{:?}\n{}/{}\n{:?}\n{:?}\n{:?}\n{}",
        encoding.path_data,
        encoding.draw_data,
        encoding.path_tags.len(),
        encoding.draw_tags.len(),
        encoding.transforms,
        encoding.styles,
        encoding.resources.glyphs,
        encoding.resources.glyph_runs.len(),
    )
}

/// What the load said about a plot attribute, one line per diagnostic: the
/// banner's `plot ...` lines.
///
/// The banner also says `param ... has no subscribers` of a param that only a
/// plot attribute reads, and says it as readily of `colorReverse`, `colorScheme`
/// and `xDomain`, which already read through params, as of these six keys. That
/// line is about how the subscriber graph counts a reader, not about what a key
/// holds, and it is kept out of the comparison so that a param on one of these
/// keys and the same value written in the file are compared on what the plot
/// says of the key.
fn said(composed: &Composed) -> Vec<String> {
    composed
        .diagnostics
        .lines()
        .into_iter()
        .filter(|line| line.starts_with("plot `"))
        .collect()
}

// ---------------------------------------------------------------------------
// The cases
// ---------------------------------------------------------------------------

/// One key (or pair of keys) written in the file and the same given through
/// params, on the columns the dots take.
struct Case {
    label: &'static str,
    x: &'static str,
    y: &'static str,
    /// The attributes with the value written in.
    literal: &'static str,
    /// The `params:` block the through form declares.
    params: &'static str,
    /// The attributes with each value a `$param`.
    through: &'static str,
}

const CASES: [Case; 18] = [
    Case {
        label: "xTicks",
        x: "a",
        y: "b",
        literal: "xTicks: 2\n",
        params: "params:\n  n: 2\n",
        through: "xTicks: $n\n",
    },
    Case {
        label: "yTicks",
        x: "a",
        y: "b",
        literal: "yTicks: 9\n",
        params: "params:\n  n: 9\n",
        through: "yTicks: $n\n",
    },
    Case {
        label: "xTickFormat on a number axis",
        x: "a",
        y: "b",
        literal: "xTickFormat: \".1f\"\n",
        params: "params:\n  f: \".1f\"\n",
        through: "xTickFormat: $f\n",
    },
    Case {
        label: "yTickFormat on a number axis",
        x: "a",
        y: "b",
        literal: "yTickFormat: \"+.2s\"\n",
        params: "params:\n  f: \"+.2s\"\n",
        through: "yTickFormat: $f\n",
    },
    Case {
        label: "xTickFormat on a date axis",
        x: "day",
        y: "b",
        literal: "xTickFormat: \"%b %Y\"\n",
        params: "params:\n  f: \"%b %Y\"\n",
        through: "xTickFormat: $f\n",
    },
    Case {
        label: "bare grid",
        x: "a",
        y: "b",
        literal: "grid: false\n",
        params: "params:\n  g: false\n",
        through: "grid: $g\n",
    },
    Case {
        label: "xGrid",
        x: "a",
        y: "b",
        literal: "xGrid: false\n",
        params: "params:\n  g: false\n",
        through: "xGrid: $g\n",
    },
    Case {
        label: "yGrid",
        x: "a",
        y: "b",
        literal: "yGrid: false\n",
        params: "params:\n  g: false\n",
        through: "yGrid: $g\n",
    },
    Case {
        label: "the axis grid key outranks a bare one given through params",
        x: "a",
        y: "b",
        literal: "grid: false\nxGrid: true\n",
        params: "params:\n  g: false\n  h: true\n",
        through: "grid: $g\nxGrid: $h\n",
    },
    Case {
        label: "xZero",
        x: "a",
        y: "b",
        literal: "xZero: true\n",
        params: "params:\n  z: true\n",
        through: "xZero: $z\n",
    },
    Case {
        label: "yZero",
        x: "a",
        y: "b",
        literal: "yZero: true\n",
        params: "params:\n  z: true\n",
        through: "yZero: $z\n",
    },
    Case {
        label: "xNice",
        x: "a",
        y: "b",
        literal: "xNice: true\n",
        params: "params:\n  z: true\n",
        through: "xNice: $z\n",
    },
    Case {
        label: "yNice",
        x: "a",
        y: "b",
        literal: "yNice: true\n",
        params: "params:\n  z: true\n",
        through: "yNice: $z\n",
    },
    Case {
        label: "xReverse",
        x: "a",
        y: "b",
        literal: "xReverse: true\n",
        params: "params:\n  r: true\n",
        through: "xReverse: $r\n",
    },
    Case {
        label: "yReverse",
        x: "a",
        y: "b",
        literal: "yReverse: true\n",
        params: "params:\n  r: true\n",
        through: "yReverse: $r\n",
    },
    Case {
        label: "an axis end and a count together, each through its own param",
        x: "a",
        y: "b",
        literal: "xZero: true\nyTicks: 4\n",
        params: "params:\n  z: true\n  n: 4\n",
        through: "xZero: $z\nyTicks: $n\n",
    },
    Case {
        label: "x and y reversed through one param",
        x: "a",
        y: "b",
        literal: "xReverse: true\nyReverse: true\n",
        params: "params:\n  r: true\n",
        through: "xReverse: $r\nyReverse: $r\n",
    },
    Case {
        label: "a count given through params and a format written in",
        x: "a",
        y: "b",
        literal: "xTicks: 3\nxTickFormat: \",.1f\"\n",
        params: "params:\n  n: 3\n",
        through: "xTicks: $n\nxTickFormat: \",.1f\"\n",
    },
];

/// **AC1.** A key given through a param that holds a literal draws as that
/// literal would, for each of the six groups and each axis, and the literal moves
/// the plot from what the plot with no key draws.
#[test]
fn a_param_holding_a_literal_draws_as_the_literal_does() {
    for case in &CASES {
        let literal = compose("", case.x, case.y, case.literal);
        let through = compose(case.params, case.x, case.y, case.through);
        let none = compose("", case.x, case.y, "");
        assert_ne!(
            drawn(&literal),
            drawn(&none),
            "{}: the literal must move the plot, or the arm below proves nothing",
            case.label
        );
        assert_eq!(
            drawn(&through),
            drawn(&literal),
            "{}: given through a param it draws as the literal does",
            case.label
        );
        assert_eq!(
            said(&through),
            said(&literal),
            "{}: the banner reads as the literal's does",
            case.label
        );
    }
}

/// A key and a value its judge refuses, and the plot's columns.
const REFUSED: [(&str, &str, &str, &str); 12] = [
    ("xTicks", "0", "a", "b"),
    ("yTicks", "100000", "a", "b"),
    ("xTickFormat", "\"~~\"", "a", "b"),
    ("yTickFormat", "\"%K\"", "a", "b"),
    ("xTickFormat", "\"%Y-%K\"", "day", "b"),
    ("grid", "yes", "a", "b"),
    ("xGrid", "1", "a", "b"),
    ("yGrid", "\"false\"", "a", "b"),
    ("xZero", "yes", "a", "b"),
    ("yNice", "5", "a", "b"),
    ("xReverse", "1", "a", "b"),
    ("yReverse", "\"true\"", "a", "b"),
];

/// **AC1.** A literal that would raise the page's warning raises it through the
/// param too: the same line, naming the key and the value, and the plot draws as
/// the literal draws.
#[test]
fn a_param_holding_a_bad_literal_raises_the_page_warning_the_literal_raises() {
    for (key, value, x, y) in REFUSED {
        let literal = compose("", x, y, &format!("{key}: {value}\n"));
        let through = compose(
            &format!("params:\n  p: {value}\n"),
            x,
            y,
            &format!("{key}: $p\n"),
        );
        let lines = said(&literal);
        assert!(
            lines.iter().any(|line| line.contains(&format!("`{key}`"))),
            "{key}: {value} written in the file is named; got {lines:?}"
        );
        assert_eq!(
            said(&through),
            lines,
            "{key} through a param holding {value} is named as {value} written in is"
        );
        assert_eq!(
            drawn(&through),
            drawn(&literal),
            "{key} through a param holding {value} draws as {value} written in does"
        );
    }
}

/// **AC1.** A tick format of the other kind and a gridline switch on a map
/// projection are judged where the draw knows the axis, and they read the param's
/// value: a date format on a number axis is named with the text the param holds,
/// and a gridline switch on a projected plot is named as changing nothing there.
#[test]
fn the_warnings_that_wait_for_the_scales_read_a_param_too() {
    for (label, literal, params, through) in [
        (
            "a date format on a number axis",
            "xTickFormat: \"%b\"\n",
            "params:\n  f: \"%b\"\n",
            "xTickFormat: $f\n",
        ),
        (
            "a number format on a date axis",
            "yTickFormat: \",d\"\n",
            "params:\n  f: \",d\"\n",
            "yTickFormat: $f\n",
        ),
    ] {
        let (x, y) = if label.contains("date axis") {
            ("a", "day")
        } else {
            ("a", "b")
        };
        let by_literal = compose("", x, y, literal);
        let by_param = compose(params, x, y, through);
        assert!(
            !said(&by_literal).is_empty(),
            "{label} written in the file is named"
        );
        assert_eq!(said(&by_param), said(&by_literal), "{label}");
    }

    for key in ["xGrid", "yGrid", "grid"] {
        let literal = format!("projectionType: equirectangular\n{key}: true\n");
        let through = format!("projectionType: equirectangular\n{key}: $g\n");
        let by_literal = compose("", "lon", "lat", &literal);
        let by_param = compose("params:\n  g: true\n", "lon", "lat", &through);
        let lines = said(&by_literal);
        assert_eq!(lines.len(), 1, "{key} on a projected plot: {lines:?}");
        assert!(lines[0].contains(&format!("`{key}`")), "{lines:?}");
        assert_eq!(
            said(&by_param),
            lines,
            "{key} given through a param on a projected plot is named as it is written in"
        );
        let off = compose("params:\n  g: false\n", "lon", "lat", &through);
        assert!(
            said(&off).is_empty(),
            "{key} through a param that holds false asks for nothing; got {:?}",
            said(&off)
        );
    }
}

/// **AC2.** A param the file never declares, or that holds a selection, draws as
/// a file without the key and raises no warning.
#[test]
fn a_param_nobody_declared_or_that_holds_a_selection_draws_as_the_key_absent() {
    for case in &CASES {
        // The key each case sets, taken from its through form.
        let key = case
            .through
            .lines()
            .next()
            .and_then(|line| line.split(':').next())
            .expect("a key");
        let none = compose("", case.x, case.y, "");
        for (what, params, value) in [
            ("an undeclared param", "", "$nobody"),
            (
                "a selection",
                "params:\n  s:\n    select: intersect\n",
                "$s",
            ),
        ] {
            let through = compose(params, case.x, case.y, &format!("{key}: {value}\n"));
            assert_eq!(
                drawn(&through),
                drawn(&none),
                "{}: {key} through {what} draws as the key absent",
                case.label
            );
            let lines = said(&through);
            assert!(
                lines.iter().all(|line| !line.contains(&format!("`{key}`"))),
                "{}: {key} through {what} raises no warning; got {lines:?}",
                case.label
            );
        }
    }
}

/// **AC3.** After the param is written a new value in the app, the plot redraws
/// with it, and the banner follows the value: a count that was refused and is
/// now a target draws at it and says nothing, and a switch written to something
/// that is no switch says so.
#[test]
fn after_the_param_is_written_a_new_value_the_plot_redraws_with_it() {
    let write = |live: &mut LiveDashboard, name: &str, value: SpecValue| -> Composed {
        live.apply(Interaction::SetParam {
            name: name.to_string(),
            value,
        })
        .expect("writing the param recomposes")
    };

    for (label, key, x, y, start, then, wrote) in [
        (
            "xTicks",
            "xTicks",
            "a",
            "b",
            "2",
            "8",
            SpecValue::Integer(8),
        ),
        (
            "xTickFormat",
            "xTickFormat",
            "a",
            "b",
            "\".1f\"",
            "\"+.2s\"",
            SpecValue::String("+.2s".to_string()),
        ),
        (
            "grid",
            "grid",
            "a",
            "b",
            "false",
            "true",
            SpecValue::Bool(true),
        ),
        (
            "yZero",
            "yZero",
            "a",
            "b",
            "true",
            "false",
            SpecValue::Bool(false),
        ),
        (
            "xNice",
            "xNice",
            "a",
            "b",
            "false",
            "true",
            SpecValue::Bool(true),
        ),
        (
            "yReverse",
            "yReverse",
            "a",
            "b",
            "false",
            "true",
            SpecValue::Bool(true),
        ),
    ] {
        let params = format!("params:\n  p: {start}\n");
        let mut live =
            LiveDashboard::load_str(&source(&params, x, y, &format!("{key}: $p\n")), None)
                .expect("the spec loads live");
        let resting = live.present().expect("the resting composition");
        let at_start = compose("", x, y, &format!("{key}: {start}\n"));
        assert_eq!(
            drawn(&resting),
            drawn(&at_start),
            "{label}: the param holds {start} at rest"
        );

        let written = write(&mut live, "p", wrote);
        let at_then = compose("", x, y, &format!("{key}: {then}\n"));
        assert_eq!(
            drawn(&written),
            drawn(&at_then),
            "{label}: after the param is written {then} the plot draws {then}"
        );
        assert_ne!(
            drawn(&written),
            drawn(&resting),
            "{label}: the write must move the plot, or the arm above proves nothing"
        );
    }

    // The banner follows the value the param holds now.
    let mut live =
        LiveDashboard::load_str(&source("params:\n  p: 0\n", "a", "b", "xTicks: $p\n"), None)
            .expect("the spec loads live");
    let refused = live.present().expect("the resting composition");
    assert!(
        said(&refused).iter().any(|line| line.contains("`xTicks`")),
        "a count of 0 is named at rest; got {:?}",
        said(&refused)
    );
    let allowed = write(&mut live, "p", SpecValue::Integer(4));
    assert!(
        said(&allowed).iter().all(|line| !line.contains("`xTicks`")),
        "a count of 4 raises nothing; got {:?}",
        said(&allowed)
    );
    assert_eq!(
        drawn(&allowed),
        drawn(&compose("", "a", "b", "xTicks: 4\n")),
        "the count written 4 draws at 4"
    );
    let refused_again = write(&mut live, "p", SpecValue::Integer(-2));
    assert!(
        said(&refused_again)
            .iter()
            .any(|line| line.contains("`xTicks`")),
        "a count written -2 is named again; got {:?}",
        said(&refused_again)
    );
}

/// **AC4.** A plot that sets none of the six through a param draws as it does
/// without a `params:` block at all: a block that no axis key names changes
/// nothing, and the literal form of each key still draws as it did.
#[test]
fn a_plot_that_gives_none_of_the_keys_through_a_param_draws_as_it_did() {
    let bare = compose("", "a", "b", "");
    let with_params = compose("params:\n  n: 7\n  z: true\n", "a", "b", "");
    assert_eq!(
        drawn(&with_params),
        drawn(&bare),
        "a params block no axis key names does not move the plot"
    );
    assert!(
        said(&bare).is_empty(),
        "a plot with no keys says nothing; got {:?}",
        said(&bare)
    );
}
