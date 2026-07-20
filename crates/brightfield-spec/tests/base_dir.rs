//! Tests for ParseOutput.base_dir.

use brightfield_spec::parse::{parse_spec, parse_spec_path, Format};

#[test]
fn dfspec_parse_spec_path_populates_base_dir() {
    // Use a known vendored corpus spec
    let corpus_dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/vendor/mosaic-specs/yaml/"
    );
    let specs: Vec<_> = std::fs::read_dir(corpus_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "yaml")
                .unwrap_or(false)
        })
        .collect();

    assert!(!specs.is_empty(), "Expected at least one corpus spec");

    let spec_path = specs[0].path();
    let output = parse_spec_path(&spec_path).unwrap();

    assert!(output.base_dir.is_some(), "base_dir should be Some when parsed from path");
    assert_eq!(
        output.base_dir.unwrap(),
        spec_path.parent().unwrap(),
        "base_dir should equal the parent directory of the spec file"
    );
}

#[test]
fn dfspec_parse_spec_string_base_dir_is_none() {
    let yaml = "meta:\n  title: test\n";
    let output = parse_spec(yaml, Format::Yaml).unwrap();
    assert!(output.base_dir.is_none(), "base_dir should be None when parsed from string");
}
