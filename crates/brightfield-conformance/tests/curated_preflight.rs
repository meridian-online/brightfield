//! Curated corpus preflight gate (ac-16).
//!
//! For every curated entry:
//!   (a) `parse_spec` succeeds without `ParseError`
//!   (b) `preflight(&spec).blocking()` is either empty OR every blocking
//!       entry is accounted for by a deviation registry entry whose
//!       `affected_specs` list names this spec's filename.
//!
//! The accounting discipline is name-level (affected_specs contains the
//! filename) rather than identity-level — ac-16 specifies the spec filename
//! is the accounting handle, which matches how the deviation registry keys
//! against curated files.

use std::fs;
use std::path::PathBuf;

use brightfield_conformance::{curated_entries, load_deviations, preflight};
use brightfield_spec::{parse_spec, Format};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

#[test]
fn dfconf_curated_preflight_accounted() {
    let registry_path = repo_root().join("deviations.yaml");
    let registry = load_deviations(&registry_path).expect("load registry");

    let entries = curated_entries().expect("load curated");
    assert!(
        entries.len() >= 10,
        "curated corpus must contain ≥10 entries; found {}",
        entries.len()
    );

    for entry in &entries {
        let filename = entry
            .source_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        // (a) parse_spec succeeds.
        let source = fs::read_to_string(&entry.source_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", entry.name));
        let parsed = parse_spec(&source, Format::Yaml)
            .unwrap_or_else(|e| panic!("parse {}: {e}", entry.name));
        let spec = parsed.spec;

        // (b) preflight blocking entries are accounted for.
        let report = preflight(&spec);
        let blocking = report.blocking();
        if blocking.is_empty() {
            continue;
        }
        // Some deviation's affected_specs must name this file.
        let accounted = registry
            .iter()
            .any(|d| d.affected_specs.contains(&filename));
        assert!(
            accounted,
            "curated spec {filename} has {} blocking preflight entries but no deviation lists it in affected_specs",
            blocking.len()
        );
    }
}
