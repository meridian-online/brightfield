//! Gate: Every YAML file under `vendor/mosaic-specs/yaml/` must parse
//! without `ParseError`. Warnings are expected — the corpus exercises the
//! full breadth of the Mosaic vocabulary, and brightfield has `Unimplemented`
//! status on every vocabulary entry in v1 — but no file may fail outright.
//!
//! This is a gate test. Failure = parser regression.

use std::path::PathBuf;

use brightfield_spec::{parse_spec_path, ParseError};

#[test]
fn dfspec_ac12_corpus_totality() {
    let corpus = corpus_dir();
    let entries = read_yaml_files(&corpus);
    assert!(
        !entries.is_empty(),
        "no YAML files found under {corpus:?} — did the vendor step fail?"
    );

    let mut failed: Vec<(PathBuf, ParseError)> = Vec::new();
    for path in &entries {
        match parse_spec_path(path) {
            Ok(_) => {}
            Err(e) => failed.push((path.clone(), e)),
        }
    }

    if !failed.is_empty() {
        let mut msg = String::from("corpus totality failed:\n");
        for (p, e) in &failed {
            msg.push_str(&format!("  {:?}: {}\n", p.file_name().unwrap_or_default(), e));
        }
        panic!("{msg}");
    }

    eprintln!("corpus totality: {} files parsed", entries.len());
}

fn corpus_dir() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest)
        .join("vendor")
        .join("mosaic-specs")
        .join("yaml")
}

fn read_yaml_files(dir: &PathBuf) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            let p = ent.path();
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "yaml" || ext == "yml" {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}
