//! corpus verification: every vendored Mosaic spec round-trips at the
//! AST level.
//!
//! Contract:
//!   parse(s) → serialise → parse → AST  ==  parse(s) → AST
//!
//! Textual round-trip is not a goal; `ParamRef` canonicalises to `$name`
//! string form on serialisation, and expression whitespace may be lost. AST
//! equality is the invariant.

use std::path::PathBuf;

use brightfield_spec::{parse_spec, Format};

#[test]
fn dfspec_ac11_every_corpus_spec_round_trips() {
    let corpus = corpus_dir();
    let entries = read_yaml_files(&corpus);
    assert!(!entries.is_empty(), "no YAML files found under {corpus:?}");

    let mut failures: Vec<(PathBuf, String)> = Vec::new();
    for path in &entries {
        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                failures.push((path.clone(), format!("read error: {e}")));
                continue;
            }
        };
        let a = match parse_spec(&source, Format::Yaml) {
            Ok(o) => o,
            Err(e) => {
                failures.push((path.clone(), format!("first parse failed: {e}")));
                continue;
            }
        };
        let serialised = match serde_yaml::to_string(&a.spec) {
            Ok(s) => s,
            Err(e) => {
                failures.push((path.clone(), format!("serialise failed: {e}")));
                continue;
            }
        };
        let b = match parse_spec(&serialised, Format::Yaml) {
            Ok(o) => o,
            Err(e) => {
                failures.push((
                    path.clone(),
                    format!("second parse failed: {e}\n---serialised---\n{serialised}"),
                ));
                continue;
            }
        };
        if a.spec != b.spec {
            failures.push((path.clone(), "AST inequality after round-trip".into()));
        }
    }

    if !failures.is_empty() {
        let mut msg = String::from("corpus round-trip failures:\n");
        for (p, detail) in &failures {
            msg.push_str(&format!("  {:?}: {detail}\n", p.file_name().unwrap_or_default()));
        }
        panic!("{msg}");
    }
    eprintln!("corpus round-trip: {} files passed", entries.len());
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
