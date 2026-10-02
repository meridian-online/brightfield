//! **A plot's `colorScheme` that brightfield cannot draw is named by the
//! parser, with the key and the value as written; one it draws, and one the
//! parser cannot yet read, are not.**
//!
//! `read_colour_scheme` is the one judge. A name in `DRAWN_COLOUR_SCHEMES` is
//! drawn; `null` and a lifted `$param` are deferrals; everything else is a
//! value the plot draws as if the key were absent, so it is named. The same
//! judge runs for a plot's own attribute and for `plotDefaults`, which is
//! merged into each plot after the per-plot checks and would otherwise drop the
//! value in silence.

use std::path::PathBuf;

use brightfield_spec::layout::{
    collect_plot_nodes, read_colour_scheme, ColourSchemeReading, DRAWN_COLOUR_SCHEMES,
};
use brightfield_spec::{parse_spec, parse_spec_path, Format, ParseWarning, SpecValue};

/// A one-plot spec with the given params and plot attributes.
fn warnings(params: &str, attrs: &str) -> Vec<ParseWarning> {
    let source = format!(
        "{params}data:\n  t:\n    - {{ x: 1, y: 2 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n{attrs}"
    );
    parse_spec(&source, Format::Yaml)
        .unwrap_or_else(|e| panic!("the spec parses: {e}\n{source}"))
        .warnings
}

/// The `(key, value)` of each colour-key warning in `warnings`.
fn colour_warnings(warnings: &[ParseWarning]) -> Vec<(String, String)> {
    warnings
        .iter()
        .filter_map(|w| match w {
            ParseWarning::UnreadColourKey { attribute, value } => {
                Some((attribute.clone(), value.clone()))
            }
            _ => None,
        })
        .collect()
}

fn pair(key: &str, value: &str) -> (String, String) {
    (key.to_string(), value.to_string())
}

/// **AC1.** A scheme brightfield does not draw is named, key and value as
/// written: a Mosaic scheme it has no ramp for, a wrong-case spelling of one it
/// has, a number, and a list.
#[test]
fn a_scheme_brightfield_does_not_draw_is_named_with_its_key_and_value() {
    for (written, shown) in [
        ("magma", "magma"),
        ("ylgnbu", "ylgnbu"),
        ("Viridis", "Viridis"),
        ("''", ""),
        ("3", "3"),
        ("[blues]", "<non-string>"),
    ] {
        let found = colour_warnings(&warnings("", &format!("colorScheme: {written}\n")));
        assert_eq!(
            found,
            vec![pair("colorScheme", shown)],
            "`colorScheme: {written}`"
        );
    }
}

/// The warning's sentence names the key and the value too, so a banner that
/// shows only the text still says which key and what it held.
#[test]
fn the_warning_sentence_names_the_key_and_the_value() {
    let found = warnings("", "colorScheme: magma\n");
    let text = found
        .iter()
        .find(|w| matches!(w, ParseWarning::UnreadColourKey { .. }))
        .unwrap_or_else(|| panic!("no colour-key warning in {found:?}"))
        .to_string();
    assert!(text.contains("`colorScheme`"), "the key: {text}");
    assert!(text.contains("`magma`"), "the value: {text}");
}

/// **AC3, parser side.** Each name the renderer draws raises nothing.
#[test]
fn a_scheme_brightfield_draws_raises_no_warning() {
    for name in DRAWN_COLOUR_SCHEMES {
        let found = colour_warnings(&warnings("", &format!("colorScheme: {name}\n")));
        assert!(found.is_empty(), "`{name}` is drawn: {found:?}");
        assert_eq!(
            read_colour_scheme(&SpecValue::String(name.to_string())),
            ColourSchemeReading::Drawn,
            "{name}"
        );
    }
    assert_eq!(
        DRAWN_COLOUR_SCHEMES,
        ["viridis", "blues", "turbo", "meridian"],
        "the four names this build draws"
    );
}

/// **AC2.** A `$param`, and `null`, are deferrals: no warning, whether or not
/// the param holds a name the renderer draws.
#[test]
fn a_param_and_null_raise_no_warning() {
    for (params, attrs, what) in [
        (
            "params:\n  scheme: blues\n",
            "colorScheme: $scheme\n",
            "a param holding a drawn name",
        ),
        (
            "params:\n  scheme: magma\n",
            "colorScheme: $scheme\n",
            "a param holding another name",
        ),
        ("", "colorScheme: $scheme\n", "a param nobody declared"),
        ("", "colorScheme: null\n", "null"),
        ("", "colorScheme: ~\n", "a bare tilde"),
    ] {
        let found = colour_warnings(&warnings(params, attrs));
        assert!(found.is_empty(), "{what}: {found:?}");
    }
}

/// A `plotDefaults` scheme is named once, where it is declared, and not once
/// per plot that inherits it; a drawn one and `null` there say nothing.
#[test]
fn a_plot_defaults_scheme_is_judged_once_where_it_is_declared() {
    let two_plots = |defaults: &str| {
        let source = format!(
            "data:\n  t:\n    - {{ x: 1, y: 2 }}\nplotDefaults:\n{defaults}vconcat:\n  \
             - plot:\n      - mark: dot\n        data: {{ from: t }}\n        x: x\n        y: y\n  \
             - plot:\n      - mark: dot\n        data: {{ from: t }}\n        x: x\n        y: y\n"
        );
        colour_warnings(
            &parse_spec(&source, Format::Yaml)
                .unwrap_or_else(|e| panic!("the spec parses: {e}\n{source}"))
                .warnings,
        )
    };
    assert_eq!(
        two_plots("  colorScheme: magma\n"),
        vec![pair("colorScheme", "magma")],
        "named once for two plots"
    );
    // A `$param` cannot be written in `plotDefaults`: the parser refuses it, so
    // the judge does not see one there.
    for held in ["blues", "null"] {
        let found = two_plots(&format!("  colorScheme: {held}\n"));
        assert!(found.is_empty(), "`{held}` in plotDefaults: {found:?}");
    }
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

/// **AC4.** Of the vendored Mosaic examples, the ones that name a scheme outside
/// the four brightfield draws raise the warning, one per such value; the ones
/// that give theirs as a `$param`, and the ones that name a drawn scheme, raise
/// none.
///
/// The walk reads each plot's own `colorScheme` and predicts the warnings from
/// the values, then holds the parse to that prediction. It also names the six
/// files and the one value each held when this was written, so a vendor bump
/// that dropped one, or a walk that read no plot, fails here and does not pass
/// over nothing.
#[test]
fn the_vendored_examples_that_name_a_scheme_outside_the_four_are_warned_and_a_param_is_not() {
    let mut warned_files: Vec<(String, String)> = Vec::new();
    let mut param_files: Vec<String> = Vec::new();

    for path in corpus() {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let Ok(parsed) = parse_spec_path(&path) else {
            continue; // corpus_totality is the gate for a file that does not parse
        };

        let mut expected: Vec<String> = Vec::new();
        let mut gives_a_param = false;
        for (at, plot) in collect_plot_nodes(&parsed.spec) {
            match plot.attributes.get("colorScheme") {
                None | Some(SpecValue::Null) => {}
                Some(SpecValue::Param(_)) => gives_a_param = true,
                Some(SpecValue::String(s)) if DRAWN_COLOUR_SCHEMES.contains(&s.as_str()) => {}
                Some(SpecValue::String(s)) => expected.push(s.clone()),
                Some(other) => panic!("{name}::{at} colorScheme is {other:?}"),
            }
        }
        expected.sort();
        expected.dedup();

        let mut found: Vec<String> = colour_warnings(&parsed.warnings)
            .into_iter()
            .map(|(key, value)| {
                assert_eq!(key, "colorScheme", "{name}: the only colour key judged");
                value
            })
            .collect();
        found.sort();
        found.dedup();
        assert_eq!(
            found, expected,
            "{name}: the parse warns of each scheme outside the four, and of no other"
        );

        for value in found {
            warned_files.push((name.clone(), value));
        }
        if gives_a_param && expected.is_empty() {
            param_files.push(name);
        }
    }

    let warned: Vec<(&str, &str)> = warned_files
        .iter()
        .map(|(file, value)| (file.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        warned,
        [
            ("flights-density.yaml", "ylgnbu"),
            ("flights-hexbin.yaml", "ylgnbu"),
            ("nyc-taxi-rides.yaml", "oranges"),
            ("observable-latency.yaml", "observable10"),
            ("population-arrows.yaml", "BuRd"),
            ("wnba-shots.yaml", "YlOrRd"),
        ],
        "the six vendored examples that name a scheme outside the four, each warned"
    );
    assert_eq!(
        param_files,
        ["line-density.yaml", "protein-design.yaml"],
        "the vendored examples that give their scheme as a $param, none warned"
    );
}
