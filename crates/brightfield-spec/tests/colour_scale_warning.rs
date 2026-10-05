//! **A plot's `colorScale` that brightfield does not draw, and a `colorPivot`
//! that is no number, are named by the parser with the key and the value as
//! written; `linear`, `diverging`, `quantize`, a number, `null` and a lifted
//! `$param` are not.**
//!
//! `read_colour_scale` and `colour_pivot` are the judges, and the same ones run
//! for a plot's own attribute and for `plotDefaults`. The resolvers the
//! renderer's shell reads the keys through ask them too, so a value the parser
//! accepts is a value the plot draws.

use std::path::PathBuf;

use brightfield_spec::layout::{
    collect_plot_nodes, read_colour_scale, resolve_colour_pivot, resolve_colour_scale_diverging,
    ColourScaleReading, DRAWN_COLOUR_SCALES,
};
use brightfield_spec::{parse_spec, parse_spec_path, Format, ParseWarning, SpecValue};

fn parsed(params: &str, attrs: &str) -> brightfield_spec::ParseOutput {
    let source = format!(
        "{params}data:\n  t:\n    - {{ x: 1, y: 2 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n{attrs}"
    );
    parse_spec(&source, Format::Yaml).unwrap_or_else(|e| panic!("the spec parses: {e}\n{source}"))
}

/// The `(key, value)` of each colour-key warning, restricted to `keys`.
fn warned(warnings: &[ParseWarning], keys: &[&str]) -> Vec<(String, String)> {
    warnings
        .iter()
        .filter_map(|w| match w {
            ParseWarning::UnreadColourKey { attribute, value }
                if keys.contains(&attribute.as_str()) =>
            {
                Some((attribute.clone(), value.clone()))
            }
            _ => None,
        })
        .collect()
}

fn pair(key: &str, value: &str) -> (String, String) {
    (key.to_string(), value.to_string())
}

const KEYS: [&str; 2] = ["colorScale", "colorPivot"];

/// A scale brightfield does not draw is named, key and value as written: a
/// Mosaic scale type it has no ramp for, a wrong-case spelling of one it has,
/// a number, and a list.
#[test]
fn a_scale_brightfield_does_not_draw_is_named_with_its_key_and_value() {
    for (attrs, value) in [
        ("colorScale: quantile\n", "quantile"),
        ("colorScale: symlog\n", "symlog"),
        ("colorScale: diverging-log\n", "diverging-log"),
        ("colorScale: Diverging\n", "Diverging"),
        ("colorScale: 3\n", "3"),
        ("colorScale: [a, b]\n", "<non-string>"),
    ] {
        let found = warned(&parsed("", attrs).warnings, &KEYS);
        assert_eq!(found, [pair("colorScale", value)], "{attrs}");
    }
}

/// A scale it draws, `null` and a lifted `$param` raise nothing.
#[test]
fn a_drawn_scale_null_and_a_param_raise_no_warning() {
    for name in DRAWN_COLOUR_SCALES {
        let found = warned(
            &parsed("", &format!("colorScale: {name}\n")).warnings,
            &KEYS,
        );
        assert!(found.is_empty(), "`{name}` is drawn: {found:?}");
        assert_eq!(
            read_colour_scale(&SpecValue::String(name.to_string())),
            ColourScaleReading::Drawn,
            "{name}"
        );
    }
    assert_eq!(DRAWN_COLOUR_SCALES, ["linear", "diverging", "quantize"]);
    for (params, attrs, what) in [
        (
            "params:\n  s: quantile\n",
            "colorScale: $s\n",
            "a param holding another name",
        ),
        ("", "colorScale: $s\n", "a param nobody declared"),
        ("", "colorScale: null\n", "null"),
    ] {
        let found = warned(&parsed(params, attrs).warnings, &KEYS);
        assert!(found.is_empty(), "{what}: {found:?}");
    }
}

/// A pivot is a number: an integer or a float raises nothing; a word and a list
/// are named; `null` and a `$param` are deferrals.
#[test]
fn a_pivot_that_is_no_number_is_named() {
    for attrs in [
        "colorPivot: 2\n",
        "colorPivot: 2.5\n",
        "colorPivot: -1\n",
        "colorPivot: null\n",
    ] {
        let found = warned(&parsed("", attrs).warnings, &KEYS);
        assert!(found.is_empty(), "{attrs}: {found:?}");
    }
    let found = warned(
        &parsed("params:\n  p: x\n", "colorPivot: $p\n").warnings,
        &KEYS,
    );
    assert!(found.is_empty(), "a param: {found:?}");
    for (attrs, value) in [
        ("colorPivot: middle\n", "middle"),
        ("colorPivot: true\n", "true"),
        ("colorPivot: [1, 2]\n", "<non-string>"),
    ] {
        let found = warned(&parsed("", attrs).warnings, &KEYS);
        assert_eq!(found, [pair("colorPivot", value)], "{attrs}");
    }
}

/// A `plotDefaults` scale is named once, where it is declared, and not once per
/// plot that inherits it; a drawn one says nothing.
#[test]
fn a_plot_defaults_scale_is_judged_once_where_it_is_declared() {
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
    let found = warned(&two_plots("plotDefaults:\n  colorScale: quantile\n"), &KEYS);
    assert_eq!(
        found,
        [pair("colorScale", "quantile")],
        "named once, not per plot"
    );
    let found = warned(
        &two_plots("plotDefaults:\n  colorScale: diverging\n  colorPivot: 3\n"),
        &KEYS,
    );
    assert!(found.is_empty(), "{found:?}");
}

/// The resolvers the shell reads the keys through: a literal, and a `$param` as
/// its value param holds it now; a plot that says neither is not diverging and
/// has no pivot.
#[test]
fn the_resolvers_read_a_literal_and_a_param_as_it_stands() {
    let plot_of = |spec: &brightfield_spec::ParseOutput| {
        collect_plot_nodes(&spec.spec)
            .into_iter()
            .next()
            .expect("one plot")
            .1
            .clone()
    };
    let check = |params: &str, attrs: &str| {
        let spec = parsed(params, attrs);
        let plot = plot_of(&spec);
        (
            resolve_colour_scale_diverging(&plot, &spec.spec.params),
            resolve_colour_pivot(&plot, &spec.spec.params),
        )
    };
    assert_eq!(check("", ""), (false, None));
    assert_eq!(check("", "colorScale: linear\n"), (false, None));
    assert_eq!(check("", "colorScale: quantile\n"), (false, None));
    assert_eq!(
        check("", "colorScale: diverging\ncolorPivot: 2\n"),
        (true, Some(2.0))
    );
    assert_eq!(
        check("", "colorScale: diverging\ncolorPivot: -0.5\n"),
        (true, Some(-0.5))
    );
    assert_eq!(
        check("", "colorScale: diverging\ncolorPivot: middle\n"),
        (true, None)
    );
    assert_eq!(
        check(
            "params:\n  s: diverging\n  p: 7\n",
            "colorScale: $s\ncolorPivot: $p\n"
        ),
        (true, Some(7.0)),
        "a param is read as its value param holds it"
    );
    assert_eq!(
        check(
            "params:\n  s: quantile\n  p: x\n",
            "colorScale: $s\ncolorPivot: $p\n"
        ),
        (false, None),
        "a param holding an undrawn value draws as the key absent"
    );
    assert_eq!(
        check("", "colorScale: $nobody\ncolorPivot: $nobody\n"),
        (false, None),
        "a param nobody declared is no value"
    );
}

fn corpus() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/mosaic-specs/yaml");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {dir:?}: {e}"))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("yaml"))
        .collect();
    files.sort();
    files
}

/// Of the vendored Mosaic examples, the ones that write a `colorScale` outside
/// the two brightfield draws are warned, one per such value, and the ones that
/// write `diverging` or a `$param` are not.
///
/// The walk predicts the warnings from each plot's own values and holds the
/// parse to that prediction; it also names the files and the values the corpus
/// held when this was written, so a vendor bump that dropped one, or a walk that
/// read no plot, fails here and does not pass over nothing.
#[test]
fn the_vendored_examples_that_write_a_scale_outside_the_drawn_names_are_warned() {
    let mut warned_files: Vec<(String, String)> = Vec::new();
    let mut diverging_files: Vec<String> = Vec::new();
    for path in corpus() {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let Ok(spec) = parse_spec_path(&path) else {
            continue; // corpus_totality is the gate for a file that does not parse
        };
        let mut expected: Vec<String> = Vec::new();
        for (_, plot) in collect_plot_nodes(&spec.spec) {
            match plot.attributes.get("colorScale") {
                None | Some(SpecValue::Null) | Some(SpecValue::Param(_)) => {}
                Some(SpecValue::String(s)) if s == "diverging" => {
                    diverging_files.push(name.clone())
                }
                Some(SpecValue::String(s)) if DRAWN_COLOUR_SCALES.contains(&s.as_str()) => {}
                Some(SpecValue::String(s)) => expected.push(s.clone()),
                Some(other) => panic!("{name} colorScale is {other:?}"),
            }
        }
        expected.sort();
        expected.dedup();
        let mut found: Vec<String> = warned(&spec.warnings, &["colorScale"])
            .into_iter()
            .map(|(_, value)| value)
            .collect();
        found.sort();
        found.dedup();
        assert_eq!(
            found, expected,
            "{name}: the parse warns of each scale outside the drawn names, and of no other"
        );
        for value in found {
            warned_files.push((name.clone(), value));
        }
    }
    assert!(
        !warned_files.is_empty() && !diverging_files.is_empty(),
        "the walk found a warned scale and a diverging one: {warned_files:?} {diverging_files:?}"
    );
    assert!(
        diverging_files
            .iter()
            .any(|f| f == "aeromagnetic-survey.yaml"),
        "the vendored survey writes `colorScale: diverging` and is not warned: {diverging_files:?}"
    );
    assert!(
        warned_files
            .iter()
            .any(|(f, v)| f == "flights-density.yaml" && v == "symlog"),
        "the vendored density example writes `colorScale: symlog` and is warned: {warned_files:?}"
    );
}
