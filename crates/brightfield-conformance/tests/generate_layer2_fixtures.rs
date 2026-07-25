//! Generate `.layer2.expected.sql` fixtures for curated corpus specs.
//!
//! This test both generates and validates — it emits the DDL for each curated
//! spec, canonicalises it, and either writes or verifies against the expected
//! SQL sibling file.

use brightfield_conformance::corpus::curated_entries;
use brightfield_spec::parse::parse_spec_path;
use brightfield_sql::emit::emit_sources;
use brightfield_sql::render::canonicalise_ddl;
use std::fs;
use std::path::PathBuf;

fn expected_sql_path(source_path: &std::path::Path) -> PathBuf {
    let stem = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    source_path.with_file_name(format!("{stem}.layer2.expected.sql"))
}

#[test]
fn dfconf_generate_and_verify_layer2_fixtures() {
    let entries = curated_entries().expect("curated corpus should load");
    assert!(entries.len() >= 10, "Expected at least 10 curated entries");

    for entry in &entries {
        let output = parse_spec_path(&entry.source_path)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {e}", entry.name));

        // A spec with no `data:` block emits no DDL, so its "expected SQL" is
        // an empty file that asserts nothing while still counting as a green
        // layer-2 cell. Write no fixture for it; the layer-2 check reports
        // `pending: spec declares no data sources` instead, which is true.
        if output.spec.data.is_empty() {
            let fixture_path = expected_sql_path(&entry.source_path);
            assert!(
                !fixture_path.exists(),
                "{} declares no data sources — an expected-SQL fixture for it \
                 can only ever be empty, and an empty fixture asserts nothing",
                entry.name
            );
            continue;
        }

        // Pass None as base_dir — conformance canonical form uses relative paths
        let emit_output = emit_sources(&output.spec, None)
            .unwrap_or_else(|e| panic!("Failed to emit sources for {}: {e}", entry.name));

        let canonical = canonicalise_ddl(&emit_output.statements);
        let fixture_path = expected_sql_path(&entry.source_path);

        if std::env::var("GENERATE_FIXTURES").is_ok() {
            // Write mode: generate fixtures
            fs::write(&fixture_path, &canonical)
                .unwrap_or_else(|e| panic!("Failed to write {}: {e}", fixture_path.display()));
            eprintln!("Generated: {}", fixture_path.display());
        } else {
            // Verify mode: check against existing fixture
            if fixture_path.exists() {
                let expected = fs::read_to_string(&fixture_path)
                    .unwrap_or_else(|e| panic!("Failed to read {}: {e}", fixture_path.display()));
                assert_eq!(
                    canonical, expected,
                    "Layer-2 DDL mismatch for {}",
                    entry.name
                );
            }
            // If fixture doesn't exist, skip (will be created in generate mode)
        }
    }
}
