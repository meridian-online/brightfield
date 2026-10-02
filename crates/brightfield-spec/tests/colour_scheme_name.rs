//! **A plot's `colorScheme` is read as the name the spec gives it: the string
//! it wrote, or the string the `$param` it names holds when the spec is read.**
//!
//! The resolver judges nothing about the name. Whether a name is one a renderer
//! draws is the renderer's to say, so a name nothing draws still comes back.
//! What comes back as no name is a plot with no `colorScheme`, a value that is
//! no string, and a `$param` that holds no string or is not declared.

use brightfield_spec::layout::{collect_plot_nodes, resolve_colour_scheme_name};
use brightfield_spec::{parse_spec, Format, Spec};

/// A one-plot spec with the given plot attributes and params.
fn spec(params: &str, attrs: &str) -> Spec {
    let source = format!(
        "{params}data:\n  t:\n    - {{ x: 1, y: 2 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n{attrs}"
    );
    parse_spec(&source, Format::Yaml)
        .unwrap_or_else(|e| panic!("the spec parses: {e}\n{source}"))
        .spec
}

/// The name the spec's one plot gives its `colorScheme`, owned.
fn name_of(spec: &Spec) -> Option<String> {
    let plots = collect_plot_nodes(spec);
    assert_eq!(plots.len(), 1, "one plot");
    resolve_colour_scheme_name(plots[0].1, &spec.params).map(str::to_string)
}

/// A string is the name, whichever name it is.
#[test]
fn a_string_is_the_name_it_spells() {
    for name in ["blues", "viridis", "turbo", "meridian", "ylgnbu"] {
        let s = spec("", &format!("colorScheme: {name}\n"));
        assert_eq!(name_of(&s).as_deref(), Some(name), "{name}");
    }
}

/// A plot that writes no `colorScheme`, and one that writes something that is
/// no name, give none.
#[test]
fn no_scheme_and_a_value_that_is_no_string_give_no_name() {
    assert_eq!(name_of(&spec("", "")), None, "no key");
    assert_eq!(name_of(&spec("", "colorScheme: 3\n")), None, "a number");
    assert_eq!(name_of(&spec("", "colorScheme: true\n")), None, "a bool");
    assert_eq!(name_of(&spec("", "colorScheme: [blues]\n")), None, "a list");
}

/// A `$param` is the name its value param holds now, and the answer follows the
/// param when it is written: the spec a redraw reads is the one with the new
/// value in it.
#[test]
fn a_param_is_the_name_it_holds_when_the_spec_is_read() {
    let mut s = spec("params:\n  scheme: blues\n", "colorScheme: $scheme\n");
    assert_eq!(name_of(&s).as_deref(), Some("blues"), "the param's value");

    s.params.insert(
        "scheme".to_string(),
        brightfield_spec::ParamNode::Value(brightfield_spec::SpecValue::String("turbo".into())),
    );
    assert_eq!(
        name_of(&s).as_deref(),
        Some("turbo"),
        "the param, rewritten"
    );
}

/// A `$param` that holds no string, a selection, and one nobody declared name
/// nothing.
#[test]
fn a_param_that_holds_no_string_names_nothing() {
    let number = spec("params:\n  scheme: 4\n", "colorScheme: $scheme\n");
    assert_eq!(name_of(&number), None, "a number-valued param");

    let selection = spec(
        "params:\n  scheme: { select: crossfilter }\n",
        "colorScheme: $scheme\n",
    );
    assert_eq!(name_of(&selection), None, "a selection");

    let undeclared = spec("", "colorScheme: $scheme\n");
    assert_eq!(name_of(&undeclared), None, "a param nobody declared");
}
