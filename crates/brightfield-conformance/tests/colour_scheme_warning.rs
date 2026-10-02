//! **A `colorScheme` brightfield cannot draw reaches the load diagnostics the
//! page shows, as an advisory that names the key and the value; one it draws
//! does not.**
//!
//! The parser raises `ParseWarning::UnreadColourKey`; `LoadDiagnostics` carries
//! it to the page worded, with the key as the diagnostic's wire name so a
//! reader can search their own file for it. These read the diagnostic and not
//! the parse warning, because the diagnostic is what a person is shown.

use brightfield_conformance::{DiagnosticSeverity, LoadDiagnostics};
use brightfield_spec::{analysis::analyse_spec, parse_spec, Format};

/// The diagnostics a one-plot spec with these attributes loads with.
fn diagnose(params: &str, attrs: &str) -> LoadDiagnostics {
    let source = format!(
        "{params}data:\n  t:\n    - {{ x: 1, y: 2 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n{attrs}"
    );
    let out = parse_spec(&source, Format::Yaml)
        .unwrap_or_else(|e| panic!("the spec parses: {e}\n{source}"));
    let analysis = analyse_spec(&out.spec).expect("analyses");
    LoadDiagnostics::collect(
        Some("test.yaml".to_string()),
        &out.spec,
        &out.warnings,
        &analysis.warnings,
    )
}

/// The lines about `colorScheme`, as the page words them.
fn about_the_scheme(d: &LoadDiagnostics) -> Vec<String> {
    d.diagnostics
        .iter()
        .filter(|d| d.wire_name == "colorScheme")
        .map(ToString::to_string)
        .collect()
}

/// **AC1.** A scheme brightfield does not draw is an advisory on the plot,
/// headed by the key, and its sentence holds the value as written.
#[test]
fn a_scheme_brightfield_does_not_draw_is_an_advisory_naming_the_key_and_the_value() {
    let d = diagnose("", "colorScheme: magma\n");
    let named: Vec<_> = d
        .diagnostics
        .iter()
        .filter(|d| d.wire_name == "colorScheme")
        .collect();
    assert_eq!(named.len(), 1, "one line about the key: {:?}", d.lines());
    let line = named[0];
    assert_eq!(
        line.severity,
        DiagnosticSeverity::Advisory,
        "it still draws"
    );
    assert_eq!(line.surface, "plot");
    assert!(
        line.message.contains("`colorScheme`") && line.message.contains("`magma`"),
        "the sentence names the key and the value: {}",
        line.message
    );
}

/// **AC2 and AC3.** Each name the renderer draws, a `$param`, and `null` leave
/// the page with nothing to say about the key.
#[test]
fn a_drawn_scheme_a_param_and_null_say_nothing_to_the_page() {
    let drawn = [
        ("", "colorScheme: viridis\n", "viridis"),
        ("", "colorScheme: blues\n", "blues"),
        ("", "colorScheme: turbo\n", "turbo"),
        ("", "colorScheme: meridian\n", "meridian"),
        (
            "params:\n  scheme: blues\n",
            "colorScheme: $scheme\n",
            "a param",
        ),
        (
            "params:\n  scheme: magma\n",
            "colorScheme: $scheme\n",
            "a param holding another name",
        ),
        ("", "colorScheme: null\n", "null"),
        ("", "", "no colorScheme"),
    ];
    for (params, attrs, what) in drawn {
        let d = diagnose(params, attrs);
        assert!(
            about_the_scheme(&d).is_empty(),
            "{what}: {:?}",
            about_the_scheme(&d)
        );
    }
}
