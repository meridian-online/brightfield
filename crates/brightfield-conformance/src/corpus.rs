//! Corpus discovery (ac-07, ac-08).
//!
//! Two corpora are addressed:
//!
//! - **Curated** — 10–12 hand-picked specs under
//!   `crates/brightfield-conformance/vendor/curated/yaml/`, each with a
//!   sibling `<name>.expected.yaml`. The curated set gates all four
//!   conformance layers.
//! - **Observed** — card 0001's full vendored corpus at
//!   `crates/brightfield-spec/vendor/mosaic-specs/yaml/`, referenced via
//!   path const. Observed is layer-1 only.
//!
//! Resolution primitive for the observed path is `env!("CARGO_MANIFEST_DIR")`
//! at compile time — stable across workspace-root invocation, crate-dir
//! invocation, and IDE runners.

use std::fs;
use std::path::{Path, PathBuf};

use crate::expectations::{load_expectations, ExpectationError, LayerExpectations};

/// Which corpus a conformance run is targeting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Corpus {
    /// Curated hand-picked set with per-layer expectations.
    Curated,
    /// Full canon (card 0001's vendored corpus) — layer-1 only.
    Observed,
}

impl Corpus {
    /// Human-readable name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Curated => "curated",
            Self::Observed => "observed",
        }
    }
}

/// Crate-relative path to the observed corpus (card 0001's vendored set).
/// Resolved via `CARGO_MANIFEST_DIR` — stable across invocation contexts.
pub const OBSERVED_CORPUS: &str = "../brightfield-spec/vendor/mosaic-specs/yaml";

/// Crate-relative path to the curated corpus.
pub const CURATED_CORPUS: &str = "vendor/curated/yaml";

/// A single corpus entry in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusEntry {
    /// Filename stem (`crossfilter`, not `crossfilter.yaml`).
    pub name: String,
    /// Path to the `.yaml` source.
    pub source_path: PathBuf,
    /// Per-layer expectations — parsed at discovery time for curated
    /// entries; defaulted for observed entries.
    pub expectations: LayerExpectations,
}

/// Resolve a crate-relative path under `CARGO_MANIFEST_DIR`.
#[must_use]
pub fn resolve_crate_relative(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Enumerate observed-corpus specs. Expectations default to
/// `{ layer_1: Pass, layer_{2,3,4}: Pending }`.
#[must_use]
pub fn observed_entries() -> Vec<CorpusEntry> {
    let dir = resolve_crate_relative(OBSERVED_CORPUS);
    list_yaml_curated_like(&dir)
        .into_iter()
        .map(|p| CorpusEntry {
            name: stem(&p),
            source_path: p,
            expectations: LayerExpectations::observed_default(),
        })
        .collect()
}

/// Path iterator over observed-corpus yaml files (no `.expected.yaml`
/// siblings under the observed corpus).
#[must_use]
pub fn observed_specs() -> Vec<PathBuf> {
    list_yaml_curated_like(&resolve_crate_relative(OBSERVED_CORPUS))
}

/// Enumerate curated-corpus entries, parsing each sibling
/// `<name>.expected.yaml` at discovery time. Returns an error on any
/// malformed expectations file.
pub fn curated_entries() -> Result<Vec<CorpusEntry>, ExpectationError> {
    let dir = resolve_crate_relative(CURATED_CORPUS);
    let mut out = Vec::new();
    for path in list_yaml_curated_like(&dir) {
        let expected_path = expectations_path_for(&path);
        let expectations = load_expectations(&expected_path)?;
        out.push(CorpusEntry {
            name: stem(&path),
            source_path: path,
            expectations,
        });
    }
    Ok(out)
}

/// List every `.yaml` under `dir` that is NOT an `<name>.expected.yaml`
/// sibling.
fn list_yaml_curated_like(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for ent in entries.flatten() {
        let p = ent.path();
        if !p.is_file() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !(name.ends_with(".yaml") || name.ends_with(".yml")) {
            continue;
        }
        if name.ends_with(".expected.yaml") || name.ends_with(".expected.yml") {
            continue;
        }
        out.push(p);
    }
    out.sort();
    out
}

/// Filename stem without the `.yaml` suffix (no path, no extension).
fn stem(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Compute the sibling expectations path for a curated spec at `source`.
#[must_use]
pub fn expectations_path_for(source: &Path) -> PathBuf {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    source.with_file_name(format!("{stem}.expected.yaml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dfconf_observed_corpus_points_at_brightfield_spec() {
        let dir = resolve_crate_relative(OBSERVED_CORPUS);
        assert!(dir.is_dir(), "observed corpus dir not found: {dir:?}");
        let yaml_count = observed_specs().len();
        assert!(
            yaml_count >= 1,
            "observed corpus must contain at least one .yaml file; found {yaml_count}"
        );
    }

    #[test]
    fn dfconf_expectations_path_for_adds_expected_suffix() {
        let p = PathBuf::from("/x/y/crossfilter.yaml");
        let e = expectations_path_for(&p);
        assert_eq!(e, PathBuf::from("/x/y/crossfilter.expected.yaml"));
    }
}
