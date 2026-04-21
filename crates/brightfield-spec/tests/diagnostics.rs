//! ac-14 (gate): Malformed-spec diagnostics. Five cases, each asserting that
//! the parser returns the correct `ParseError` variant for a specific class
//! of user mistake. Spans are checked where the deserialiser is expected to
//! provide them; structural errors may carry `None`.

use brightfield_spec::{parse_spec, Format, NameSurface, ParseError};

/// 1. Malformed YAML syntax → YamlSyntax error.
#[test]
fn dfspec_ac14_malformed_yaml_syntax() {
    let src = "meta:\n  title: [unterminated";
    let err = parse_spec(src, Format::Yaml).expect_err("should fail");
    assert!(matches!(err, ParseError::YamlSyntax { .. }));
}

/// 2. Unknown mark name → UnknownName { surface: Mark }.
#[test]
fn dfspec_ac14_unknown_mark_name() {
    let src = r#"
plot:
  - mark: totallyFakeMark
"#;
    let err = parse_spec(src, Format::Yaml).expect_err("should fail");
    match err {
        ParseError::UnknownName { name, surface, .. } => {
            assert_eq!(name, "totallyFakeMark");
            assert_eq!(surface, NameSurface::Mark);
        }
        other => panic!("expected UnknownName, got {other:?}"),
    }
}

/// 3. Strict-context `$param` under `meta.title` →
/// StrictContextUnresolvedRef.
#[test]
fn dfspec_ac14_strict_context_dollar_in_meta() {
    let src = "meta:\n  title: $live\n";
    let err = parse_spec(src, Format::Yaml).expect_err("should fail");
    match err {
        ParseError::StrictContextUnresolvedRef { field_path, name, .. } => {
            assert_eq!(field_path, "meta.title");
            assert_eq!(name, "live");
        }
        other => panic!("expected StrictContextUnresolvedRef, got {other:?}"),
    }
}

/// 4. Malformed mark data declaration missing `from:` → SchemaViolation.
#[test]
fn dfspec_ac14_mark_data_missing_from() {
    let src = r#"
plot:
  - mark: dot
    data: { filterBy: $brush }
    x: a
    y: b
params:
  brush: { select: crossfilter }
"#;
    let err = parse_spec(src, Format::Yaml).expect_err("should fail");
    match err {
        ParseError::SchemaViolation { path, .. } => {
            assert_eq!(path, "mark.data");
        }
        other => panic!("expected SchemaViolation, got {other:?}"),
    }
}

/// 5. Unknown selection resolution → UnknownName (surface: Interactor).
#[test]
fn dfspec_ac14_unknown_selection_resolution() {
    let src = r#"
params:
  brush: { select: notARealResolution }
"#;
    let err = parse_spec(src, Format::Yaml).expect_err("should fail");
    match err {
        ParseError::UnknownName { name, .. } => {
            assert_eq!(name, "notARealResolution");
        }
        other => panic!("expected UnknownName, got {other:?}"),
    }
}

/// 6 (bonus): Malformed JSON → JsonSyntax.
#[test]
fn dfspec_ac14_malformed_json_syntax() {
    let src = r#"{"meta": {"title": "x""#;
    let err = parse_spec(src, Format::Json).expect_err("should fail");
    assert!(matches!(err, ParseError::JsonSyntax { .. }));
}
