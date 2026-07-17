//! Card 0024 (discrete input widgets) — vocabulary honesty + portability
//! gates at the PUBLIC parse/serialise API (diw-ac10 / diw-ac13 / diw-ac15).
//!
//! These are integration tests by mandate, not preference: the card's
//! machine gate keeps `crates/brightfield-spec/src/parse.rs` byte-untouched,
//! so its `#[cfg(test)]` module cannot grow — new parse behaviour is pinned
//! here against the exported surface instead.

use brightfield_spec::ast::{Component, SpecValue, ValueOrParamRef};
use brightfield_spec::{parse_spec, serialise_spec, Format, NameSurface, ParseError, ParseWarning};

/// diw_ac10: `input: radio` and `input: checkbox` are NOT wire names — the
/// no-new-vocab decision made falsifiable. Radio/checkbox exist only as
/// `style:` presentations OF `input: menu`.
#[test]
fn diw_ac10_radio_and_checkbox_are_not_input_wire_names() {
    for name in ["radio", "checkbox"] {
        let yaml = format!(
            r#"
input: {name}
as: $k
options: [a, b]
"#
        );
        let err = parse_spec(&yaml, Format::Yaml).expect_err("must be rejected");
        match err {
            ParseError::UnknownName { name: got, surface, .. } => {
                assert_eq!(got, name);
                assert_eq!(surface, NameSurface::Input);
            }
            other => panic!("expected UnknownName for input: {name}, got {other:?}"),
        }
    }
}

/// diw_ac10: `style:` lands VERBATIM in `Input.options` (the parse.rs
/// catch-all) with no warning — the key rides the preserved options bag,
/// untyped at parse time.
#[test]
fn diw_ac10_style_key_lands_verbatim_in_options_no_warning() {
    let yaml = r#"
input: menu
as: $kind
style: radio
options: [circle, square]
"#;
    let out = parse_spec(yaml, Format::Yaml).expect("parses");
    assert!(
        out.warnings.is_empty(),
        "menu is Implemented and style is an ordinary bag key — no warnings, got {:?}",
        out.warnings
    );
    let root = out.spec.root.as_ref().expect("has root");
    let Component::Input(input) = root else {
        panic!("root is the input node");
    };
    assert_eq!(
        input.options.get("style"),
        Some(&ValueOrParamRef::Value(SpecValue::String("radio".to_string()))),
        "style: radio preserved verbatim in the options bag"
    );
}

/// diw_ac10: a spec using `input: menu` earns no Unimplemented warning
/// (Menu flipped Implemented), while the still-unbuilt kinds — search,
/// table — keep warning honestly.
#[test]
fn diw_ac10_menu_no_longer_warns_search_and_table_still_do() {
    let menu = parse_spec(
        r#"
input: menu
as: $k
options: [a]
"#,
        Format::Yaml,
    )
    .expect("parses");
    assert!(
        !menu
            .warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::Unimplemented { .. })),
        "input: menu is Implemented — no honesty warning, got {:?}",
        menu.warnings
    );

    for name in ["search", "table"] {
        let yaml = format!("input: {name}\nas: $k\n");
        let out = parse_spec(&yaml, Format::Yaml).expect("parses");
        assert!(
            out.warnings.iter().any(|w| matches!(
                w,
                ParseWarning::Unimplemented { name: n, surface: NameSurface::Input, .. } if n == name
            )),
            "input: {name} still warns honestly"
        );
    }
}

/// diw_ac15: a `style:`-carrying spec re-emits the key verbatim through
/// `serialise_spec` and round-trips to an equal AST — portability by
/// construction made falsifiable (every widget this card enables serialises
/// as plain Mosaic `input: menu`).
#[test]
fn diw_ac15_style_spec_serialises_verbatim_and_round_trips() {
    let yaml = r#"
params:
  kind: circle
  flag: false
data:
  t:
    - { x: 1, kind: circle }
vconcat:
  - plot:
      - { mark: dot, data: { from: t }, x: x, y: x }
  - input: menu
    style: radio
    as: $kind
    options: [circle, square]
  - input: menu
    style: checkbox
    as: $flag
"#;
    let first = parse_spec(yaml, Format::Yaml).expect("parses");
    let serialised = serialise_spec(&first.spec).expect("serialises");
    assert!(
        serialised.contains("style: radio"),
        "the radio style key re-emits verbatim:\n{serialised}"
    );
    assert!(
        serialised.contains("style: checkbox"),
        "the checkbox style key re-emits verbatim:\n{serialised}"
    );
    assert!(
        serialised.contains("input: menu") && !serialised.contains("input: radio"),
        "serialisation never invents non-Mosaic input vocabulary:\n{serialised}"
    );
    let second = parse_spec(&serialised, Format::Yaml).expect("re-parses");
    assert_eq!(first.spec, second.spec, "AST equality after round-trip");
}

/// diw_ac13: the shipped example exercises all three presentations and
/// parses warning-free (menu now Implemented; `style:` is an ordinary key).
#[test]
fn diw_ac13_param_menu_example_parses_warning_free() {
    let yaml = include_str!("../../../examples/param-menu.yaml");
    let out = parse_spec(yaml, Format::Yaml).expect("example parses");
    assert!(
        out.warnings.is_empty(),
        "the example must be warning-free, got {:?}",
        out.warnings
    );
    // All three presentations present: a derived menu, a literal radio, a
    // checkbox — the card's coverage claim, pinned.
    let styles: Vec<Option<String>> = collect_inputs(&out.spec.root)
        .iter()
        .map(|i| match i.options.get("style") {
            Some(ValueOrParamRef::Value(SpecValue::String(s))) => Some(s.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(styles.len(), 3, "three input widgets");
    assert!(styles.contains(&None), "a plain (derived) menu");
    assert!(styles.contains(&Some("radio".to_string())), "a literal radio");
    assert!(styles.contains(&Some("checkbox".to_string())), "a checkbox");
}

/// Collect every Input node in the composition tree (test helper).
fn collect_inputs(root: &Option<Component>) -> Vec<&brightfield_spec::ast::Input> {
    fn walk<'a>(c: &'a Component, out: &mut Vec<&'a brightfield_spec::ast::Input>) {
        match c {
            Component::Input(i) => out.push(i),
            Component::Plot(p) => {
                for item in &p.items {
                    walk(item, out);
                }
            }
            Component::HConcat(cc) | Component::VConcat(cc) => {
                for item in &cc.items {
                    walk(item, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Some(c) = root {
        walk(c, &mut out);
    }
    out
}
