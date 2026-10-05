//! **A plot's `colorN` is the count of steps a `quantize` scale draws, a whole
//! number from one up; the parser names a value that is no count with the key and
//! the value as written, and the resolvers read a literal and a `$param` as it
//! stands.**
//!
//! `read_colour_steps` is the one judge, and the same one runs for a plot's own
//! attribute and for `plotDefaults`. The shell draws a plot through
//! `resolve_colour_steps`, which asks it too, so a count the parser accepts is a
//! count the plot draws and a value it names is drawn as the default five.

use brightfield_spec::layout::{
    collect_plot_nodes, read_colour_steps, resolve_colour_scale_quantize, resolve_colour_steps,
    ColourStepsReading, DEFAULT_COLOUR_STEPS, MAX_COLOUR_STEPS,
};
use brightfield_spec::{parse_spec, Format, ParseOutput, ParseWarning, SpecValue};

fn parsed(params: &str, attrs: &str) -> ParseOutput {
    let source = format!(
        "{params}data:\n  t:\n    - {{ x: 1, y: 2 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n{attrs}"
    );
    parse_spec(&source, Format::Yaml).unwrap_or_else(|e| panic!("the spec parses: {e}\n{source}"))
}

/// The `(key, value)` of each warning of a colour key `colorN` names.
fn warned(warnings: &[ParseWarning]) -> Vec<(String, String)> {
    warnings
        .iter()
        .filter_map(|w| match w {
            ParseWarning::UnreadColourKey { attribute, value } if attribute == "colorN" => {
                Some((attribute.clone(), value.clone()))
            }
            _ => None,
        })
        .collect()
}

fn pair(value: &str) -> (String, String) {
    ("colorN".to_string(), value.to_string())
}

/// A count is a whole number from one up to the most a plot can draw, written as
/// an integer or as a float with no fraction; `null` and a `$param` are
/// deferrals. None of them is named.
#[test]
fn a_count_null_and_a_param_raise_no_warning() {
    for attrs in [
        "colorN: 1\n",
        "colorN: 5\n",
        "colorN: 5.0\n",
        "colorN: 12\n",
        &format!("colorN: {MAX_COLOUR_STEPS}\n"),
        "colorN: null\n",
        "colorN: $n\n",
    ] {
        let found = warned(&parsed("", attrs).warnings);
        assert!(found.is_empty(), "{attrs}: {found:?}");
    }
    // A param that holds a value that is no count is the one case where the
    // warning and the drawing differ: nothing is raised, the plot draws five.
    let found = warned(&parsed("params:\n  n: banana\n", "colorN: $n\n").warnings);
    assert!(found.is_empty(), "a param holding junk: {found:?}");
}

/// A value that is no count is named, key and value as written: zero, a negative,
/// a fraction, a count past the most a plot can draw, a word, a boolean, a list.
#[test]
fn a_value_that_is_no_count_is_named_with_its_key_and_value() {
    let past_the_most = (MAX_COLOUR_STEPS + 1).to_string();
    for (attrs, value) in [
        ("colorN: 0\n".to_string(), "0".to_string()),
        ("colorN: -3\n".to_string(), "-3".to_string()),
        ("colorN: 2.5\n".to_string(), "2.5".to_string()),
        ("colorN: 0.0\n".to_string(), "0".to_string()),
        (format!("colorN: {past_the_most}\n"), past_the_most.clone()),
        ("colorN: five\n".to_string(), "five".to_string()),
        ("colorN: true\n".to_string(), "true".to_string()),
        ("colorN: [3]\n".to_string(), "<non-string>".to_string()),
    ] {
        let found = warned(&parsed("", &attrs).warnings);
        assert_eq!(found, [pair(&value)], "{attrs}");
    }
}

/// The warning does not wait for `colorScale: quantize`: a count no scale could
/// read is named whether the plot steps or not, as a pivot is named on a plot
/// that does not diverge.
#[test]
fn a_bad_count_is_named_without_a_quantize_scale() {
    let found = warned(&parsed("", "colorN: 0\n").warnings);
    assert_eq!(found, [pair("0")]);
    let found = warned(&parsed("", "colorScale: quantize\ncolorN: 0\n").warnings);
    assert_eq!(found, [pair("0")], "and once, with the scale");
}

/// `quantize` is a scale the plot draws, so naming it raises no warning on
/// `colorScale`.
#[test]
fn quantize_raises_no_scale_warning() {
    let out = parsed("", "colorScale: quantize\ncolorN: 5\n");
    assert!(
        out.warnings
            .iter()
            .all(|w| !matches!(w, ParseWarning::UnreadColourKey { .. })),
        "{:?}",
        out.warnings
    );
}

/// A `plotDefaults` count is named once, where it is declared, and not once per
/// plot that inherits it.
#[test]
fn a_plot_defaults_count_is_judged_once_where_it_is_declared() {
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
    let found = warned(&two_plots("plotDefaults:\n  colorN: 0\n"));
    assert_eq!(found, [pair("0")], "named once, not per plot");
    let found = warned(&two_plots(
        "plotDefaults:\n  colorScale: quantize\n  colorN: 7\n",
    ));
    assert!(found.is_empty(), "{found:?}");
}

/// The judge, directly: what it accepts is what the resolver returns.
#[test]
fn the_judge_reads_a_whole_number_in_range_and_nothing_else() {
    assert_eq!(
        read_colour_steps(&SpecValue::Integer(5)),
        ColourStepsReading::Steps(5)
    );
    assert_eq!(
        read_colour_steps(&SpecValue::Float(5.0)),
        ColourStepsReading::Steps(5)
    );
    assert_eq!(
        read_colour_steps(&SpecValue::Integer(MAX_COLOUR_STEPS as i64)),
        ColourStepsReading::Steps(MAX_COLOUR_STEPS)
    );
    assert_eq!(
        read_colour_steps(&SpecValue::Integer(MAX_COLOUR_STEPS as i64 + 1)),
        ColourStepsReading::Unknown
    );
    assert_eq!(
        read_colour_steps(&SpecValue::Integer(0)),
        ColourStepsReading::Unknown
    );
    assert_eq!(
        read_colour_steps(&SpecValue::Float(f64::NAN)),
        ColourStepsReading::Unknown
    );
    assert_eq!(
        read_colour_steps(&SpecValue::Float(f64::INFINITY)),
        ColourStepsReading::Unknown
    );
    assert_eq!(
        read_colour_steps(&SpecValue::Null),
        ColourStepsReading::Deferred
    );
}

/// The resolvers the shell reads the keys through: a literal, and a `$param` as
/// its value param holds it now; a plot that says neither does not step and has
/// no count, which draws the default five under `quantize`.
#[test]
fn the_resolvers_read_a_literal_and_a_param_as_it_stands() {
    let check = |params: &str, attrs: &str| {
        let spec = parsed(params, attrs);
        let plot = collect_plot_nodes(&spec.spec)
            .into_iter()
            .next()
            .expect("one plot")
            .1
            .clone();
        (
            resolve_colour_scale_quantize(&plot, &spec.spec.params),
            resolve_colour_steps(&plot, &spec.spec.params),
        )
    };
    assert_eq!(DEFAULT_COLOUR_STEPS, 5);
    assert_eq!(check("", ""), (false, None));
    assert_eq!(check("", "colorScale: linear\n"), (false, None));
    assert_eq!(check("", "colorScale: quantile\n"), (false, None));
    assert_eq!(check("", "colorScale: Quantize\n"), (false, None));
    assert_eq!(check("", "colorScale: quantize\n"), (true, None));
    assert_eq!(
        check("", "colorScale: quantize\ncolorN: 8\n"),
        (true, Some(8))
    );
    assert_eq!(check("", "colorN: 8\n"), (false, Some(8)));
    assert_eq!(
        check("", "colorScale: quantize\ncolorN: 6.0\n"),
        (true, Some(6))
    );
    assert_eq!(
        check("", "colorScale: quantize\ncolorN: 0\n"),
        (true, None),
        "a value that is no count draws the default"
    );
    assert_eq!(
        check("", "colorScale: quantize\ncolorN: 2.5\n"),
        (true, None)
    );
    assert_eq!(
        check(
            "params:\n  s: quantize\n  n: 7\n",
            "colorScale: $s\ncolorN: $n\n"
        ),
        (true, Some(7)),
        "a param is read as its value param holds it"
    );
    assert_eq!(
        check(
            "params:\n  s: quantile\n  n: x\n",
            "colorScale: $s\ncolorN: $n\n"
        ),
        (false, None),
        "a param holding an undrawn value draws as the key absent"
    );
    assert_eq!(
        check("", "colorScale: $nobody\ncolorN: $nobody\n"),
        (false, None),
        "a param nobody declared is no value"
    );
}
