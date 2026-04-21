//! Registry-integrity gate (ac-14).
//!
//! Bidirectional check:
//!   (a) every `suppressed(DEV-ID)` reference in a curated `<name>.expected.yaml`
//!       resolves to a record in `deviations.yaml`
//!   (b) every `deviations.yaml` entry's `affected_specs` list only names
//!       filenames present under `vendor/curated/yaml/`

use std::collections::HashSet;
use std::path::PathBuf;

use brightfield_conformance::{
    curated_entries, load_deviations, Layer1Expectation, LayerNExpectation,
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

#[test]
fn dfconf_registry_integrity() {
    let registry_path = repo_root().join("deviations.yaml");
    let registry = load_deviations(&registry_path).expect("load registry");

    let entries = curated_entries().expect("load curated");

    // Curated files that exist under vendor/curated/yaml/ by name.
    let curated_files: HashSet<String> = entries
        .iter()
        .map(|e| {
            e.source_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
        })
        .collect();

    // (a) Every suppressed(DEV-ID) in curated expectations hits the registry.
    for entry in &entries {
        let mut ids: Vec<&str> = Vec::new();
        if let Layer1Expectation::Suppressed(id) = &entry.expectations.layer_1 {
            ids.push(id);
        }
        for layern in [
            &entry.expectations.layer_2,
            &entry.expectations.layer_3,
            &entry.expectations.layer_4,
        ] {
            if let LayerNExpectation::Suppressed(id) = layern {
                ids.push(id);
            }
        }
        for id in ids {
            assert!(
                registry.get(id).is_some(),
                "curated entry {} references unknown deviation id {id}",
                entry.name
            );
        }
    }

    // (b) Every registry affected_spec is a real curated file.
    for dev in registry.iter() {
        for spec in &dev.affected_specs {
            assert!(
                curated_files.contains(spec),
                "deviation {} lists unknown affected spec {spec}",
                dev.id
            );
        }
    }
}
