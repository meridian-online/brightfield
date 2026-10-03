//! **A plot's `colorReverse` that is neither `true` nor `false` is named by the
//! parser with the key and the value as written; `true`, `false`, `null` and a
//! lifted `$param` are not.**
//!
//! `colour_reverse_switch` is the judge, and the same one runs for a plot's own
//! attribute and for `plotDefaults`. The resolver the renderer's shell reads the
//! key through asks it too, so a value the parser accepts is a value the plot
//! draws.

use brightfield_spec::layout::{collect_plot_nodes, colour_reverse_switch, resolve_colour_reverse};
use brightfield_spec::{parse_spec, Format, ParseWarning, SpecValue};

fn parsed(params: &str, attrs: &str) -> brightfield_spec::ParseOutput {
    let source = format!(
        "{params}data:\n  t:\n    - {{ x: 1, y: 2 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n{attrs}"
    );
    parse_spec(&source, Format::Yaml).unwrap_or_else(|e| panic!("the spec parses: {e}\n{source}"))
}

/// The `(key, value)` of each `colorReverse` warning.
fn warned(warnings: &[ParseWarning]) -> Vec<(String, String)> {
    warnings
        .iter()
        .filter_map(|w| match w {
            ParseWarning::UnreadColourKey { attribute, value } if attribute == "colorReverse" => {
                Some((attribute.clone(), value.clone()))
            }
            _ => None,
        })
        .collect()
}

fn pair(value: &str) -> (String, String) {
    ("colorReverse".to_string(), value.to_string())
}

/// A value that is no switch is named, key and value as written: a number, a
/// word, a quoted `true`, and a list.
#[test]
fn a_colour_reverse_that_is_no_switch_is_named_with_its_key_and_value() {
    for (attrs, value) in [
        ("colorReverse: 1\n", "1"),
        ("colorReverse: 0.5\n", "0.5"),
        ("colorReverse: reverse\n", "reverse"),
        ("colorReverse: \"true\"\n", "true"),
        ("colorReverse: [a, b]\n", "<non-string>"),
    ] {
        let found = warned(&parsed("", attrs).warnings);
        assert_eq!(found, [pair(value)], "{attrs}");
    }
}

/// A switch, `null` and a lifted `$param` raise nothing.
#[test]
fn a_switch_null_and_a_param_raise_no_warning() {
    for (params, attrs, what) in [
        ("", "colorReverse: true\n", "true"),
        ("", "colorReverse: false\n", "false"),
        ("", "colorReverse: null\n", "null"),
        ("", "", "no key"),
        (
            "params:\n  r: 3\n",
            "colorReverse: $r\n",
            "a param holding a number",
        ),
        ("", "colorReverse: $r\n", "a param nobody declared"),
    ] {
        let found = warned(&parsed(params, attrs).warnings);
        assert!(found.is_empty(), "{what}: {found:?}");
    }
    assert_eq!(colour_reverse_switch(&SpecValue::Bool(true)), Some(true));
    assert_eq!(colour_reverse_switch(&SpecValue::Bool(false)), Some(false));
    for value in [
        SpecValue::Integer(1),
        SpecValue::String("true".to_string()),
        SpecValue::Null,
    ] {
        assert_eq!(colour_reverse_switch(&value), None, "{value:?}");
    }
}

/// A `plotDefaults` value is judged once where it is declared, however many
/// plots inherit it.
#[test]
fn a_plot_defaults_colour_reverse_is_judged_once_where_it_is_declared() {
    let two_plots = |defaults: &str| {
        let source = format!(
            "{defaults}data:\n  t:\n    - {{ x: 1, y: 2 }}\nhconcat:\n  \
             - plot:\n      - mark: dot\n        data: {{ from: t }}\n        x: x\n        y: y\n  \
             - plot:\n      - mark: dot\n        data: {{ from: t }}\n        x: x\n        y: y\n"
        );
        parse_spec(&source, Format::Yaml)
            .unwrap_or_else(|e| panic!("the spec parses: {e}\n{source}"))
            .warnings
    };
    assert_eq!(
        warned(&two_plots("plotDefaults:\n  colorReverse: 3\n")),
        [pair("3")],
        "named once, not per plot"
    );
    let found = warned(&two_plots("plotDefaults:\n  colorReverse: true\n"));
    assert!(found.is_empty(), "{found:?}");
}

/// The resolver the shell reads the key through: a literal, and a `$param` as
/// its value param holds it now; a plot that says neither does not reverse.
#[test]
fn the_resolver_reads_a_literal_and_a_param_as_it_stands() {
    let resolved = |params: &str, attrs: &str| {
        let out = parsed(params, attrs);
        let plot = collect_plot_nodes(&out.spec)
            .into_iter()
            .next()
            .expect("one plot")
            .1
            .clone();
        resolve_colour_reverse(&plot, &out.spec.params)
    };
    assert!(!resolved("", ""), "no key");
    assert!(resolved("", "colorReverse: true\n"));
    assert!(!resolved("", "colorReverse: false\n"));
    assert!(!resolved("", "colorReverse: 1\n"), "a number is no switch");
    assert!(
        !resolved("", "colorReverse: \"true\"\n"),
        "a string is no switch"
    );
    assert!(
        resolved("params:\n  r: true\n", "colorReverse: $r\n"),
        "a param is read as its value param holds it"
    );
    assert!(!resolved("params:\n  r: false\n", "colorReverse: $r\n"));
    assert!(
        !resolved("params:\n  r: 1\n", "colorReverse: $r\n"),
        "a param holding no switch draws as the key absent"
    );
    assert!(
        !resolved("", "colorReverse: $nobody\n"),
        "a param nobody declared is no value"
    );
}
