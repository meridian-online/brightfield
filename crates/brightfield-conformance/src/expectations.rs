//! Per-spec per-layer expectations (ac-07, ac-12).
//!
//! Each curated-corpus entry has a sibling `<name>.expected.yaml`
//! declaring per-layer expectations. The parsed form is `LayerExpectations`
//! with two Rust enums: `Layer1Expectation` (no `Pending` — layer-1
//! machinery ships in v1) and `LayerNExpectation` (`Pass | Pending |
//! Suppressed`). Encoding "layer 1 cannot be pending" in the type system
//! means a curated `.expected.yaml` declaring `layer_1: pending` is caught
//! at load time as `ExpectationError::InvalidLayer1Pending`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Errors raised when parsing a `<name>.expected.yaml` fixture.
#[derive(Debug, thiserror::Error)]
pub enum ExpectationError {
    /// OS I/O error.
    #[error("expectations I/O error at {path:?}: {source}")]
    Io {
        /// File path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// YAML syntax error.
    #[error("expectations YAML syntax error at {path:?}: {error}")]
    YamlSyntax {
        /// File path.
        path: PathBuf,
        /// Error detail.
        error: String,
    },
    /// A required field was missing.
    #[error("expectations at {path:?} missing field {field}")]
    MissingField {
        /// File path.
        path: PathBuf,
        /// The missing field name.
        field: &'static str,
    },
    /// `layer_1: pending` — layer 1 machinery ships in v1 so `Pending` is
    /// not a valid state for layer 1.
    #[error("expectations at {path:?} declare `layer_1: pending` — layer 1 does not admit pending")]
    InvalidLayer1Pending {
        /// File path.
        path: PathBuf,
    },
    /// A deviation id referenced in a `suppressed(..)` expectation has a
    /// shape that isn't `DEV-NNNN`. Existence in the registry is checked
    /// later by the integrity gate (ac-14), not here.
    #[error("expectations at {path:?} reference deviation id {id:?} which is not in DEV-NNNN form")]
    UnknownDeviationId {
        /// File path.
        path: PathBuf,
        /// Malformed id.
        id: String,
    },
}

/// Layer-1 expectation — no `Pending` because layer 1 machinery ships in v1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layer1Expectation {
    /// Spec is expected to pass layer 1.
    Pass,
    /// Spec's layer-1 divergence is suppressed by the named deviation.
    Suppressed(String),
}

/// Layer-N (2..=4) expectation — includes `Pending` for layers whose
/// infra is not yet available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerNExpectation {
    /// Spec is expected to pass this layer.
    Pass,
    /// Layer is pending downstream infra; no judgement made.
    Pending,
    /// Spec's divergence at this layer is suppressed by the named deviation.
    Suppressed(String),
}

/// Per-layer expectations for one corpus entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerExpectations {
    /// Layer 1 — AST round-trip.
    pub layer_1: Layer1Expectation,
    /// Layer 2 — SQL equivalence.
    pub layer_2: LayerNExpectation,
    /// Layer 3 — encoding equivalence.
    pub layer_3: LayerNExpectation,
    /// Layer 4 — interaction equivalence.
    pub layer_4: LayerNExpectation,
}

impl LayerExpectations {
    /// Default used for observed-corpus entries (no sibling expected file):
    /// `layer_1: Pass, layer_{2,3,4}: Pending`.
    #[must_use]
    pub fn observed_default() -> Self {
        Self {
            layer_1: Layer1Expectation::Pass,
            layer_2: LayerNExpectation::Pending,
            layer_3: LayerNExpectation::Pending,
            layer_4: LayerNExpectation::Pending,
        }
    }
}

/// Raw YAML shape: `{ layer_1: "pass" | "suppressed: DEV-NNNN", layer_2: ... }`.
/// We parse to strings and then cook to the typed enums so error messages
/// name the offending field precisely.
#[derive(Debug, Serialize, Deserialize)]
struct ExpectationsWire {
    layer_1: String,
    layer_2: String,
    layer_3: String,
    layer_4: String,
}

/// Load a `<name>.expected.yaml` file and parse into typed `LayerExpectations`.
pub fn load_expectations(path: &Path) -> Result<LayerExpectations, ExpectationError> {
    let source = fs::read_to_string(path).map_err(|source| ExpectationError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let wire: ExpectationsWire =
        serde_yaml::from_str(&source).map_err(|e| ExpectationError::YamlSyntax {
            path: path.to_path_buf(),
            error: e.to_string(),
        })?;
    let layer_1 = parse_layer1(&wire.layer_1, path)?;
    let layer_2 = parse_layern(&wire.layer_2, path)?;
    let layer_3 = parse_layern(&wire.layer_3, path)?;
    let layer_4 = parse_layern(&wire.layer_4, path)?;
    Ok(LayerExpectations {
        layer_1,
        layer_2,
        layer_3,
        layer_4,
    })
}

fn parse_layer1(raw: &str, path: &Path) -> Result<Layer1Expectation, ExpectationError> {
    let trimmed = raw.trim();
    if trimmed == "pass" {
        return Ok(Layer1Expectation::Pass);
    }
    if trimmed == "pending" {
        return Err(ExpectationError::InvalidLayer1Pending {
            path: path.to_path_buf(),
        });
    }
    if let Some(id) = trimmed.strip_prefix("suppressed:") {
        let id = id.trim().to_string();
        check_dev_id(&id, path)?;
        return Ok(Layer1Expectation::Suppressed(id));
    }
    Err(ExpectationError::MissingField {
        path: path.to_path_buf(),
        field: "layer_1",
    })
}

fn parse_layern(raw: &str, path: &Path) -> Result<LayerNExpectation, ExpectationError> {
    let trimmed = raw.trim();
    if trimmed == "pass" {
        return Ok(LayerNExpectation::Pass);
    }
    if trimmed == "pending" {
        return Ok(LayerNExpectation::Pending);
    }
    if let Some(id) = trimmed.strip_prefix("suppressed:") {
        let id = id.trim().to_string();
        check_dev_id(&id, path)?;
        return Ok(LayerNExpectation::Suppressed(id));
    }
    Err(ExpectationError::MissingField {
        path: path.to_path_buf(),
        field: "layer_N",
    })
}

fn check_dev_id(id: &str, path: &Path) -> Result<(), ExpectationError> {
    // DEV-NNNN shape: "DEV-" then 1+ ASCII digits.
    if let Some(rest) = id.strip_prefix("DEV-") {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            return Ok(());
        }
    }
    Err(ExpectationError::UnknownDeviationId {
        path: path.to_path_buf(),
        id: id.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tmp(name: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dfconf-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn dfconf_expectations_all_pass() {
        let p = write_tmp(
            "pass.expected.yaml",
            "layer_1: pass\nlayer_2: pass\nlayer_3: pass\nlayer_4: pass\n",
        );
        let e = load_expectations(&p).unwrap();
        assert_eq!(e.layer_1, Layer1Expectation::Pass);
        assert_eq!(e.layer_2, LayerNExpectation::Pass);
    }

    #[test]
    fn dfconf_expectations_pending_mix() {
        let p = write_tmp(
            "mix.expected.yaml",
            "layer_1: pass\nlayer_2: pending\nlayer_3: pending\nlayer_4: pending\n",
        );
        let e = load_expectations(&p).unwrap();
        assert_eq!(e.layer_2, LayerNExpectation::Pending);
    }

    #[test]
    fn dfconf_expectations_suppressed() {
        let p = write_tmp(
            "supp.expected.yaml",
            "layer_1: pass\nlayer_2: \"suppressed: DEV-0001\"\nlayer_3: pending\nlayer_4: pending\n",
        );
        let e = load_expectations(&p).unwrap();
        assert_eq!(
            e.layer_2,
            LayerNExpectation::Suppressed("DEV-0001".to_string())
        );
    }

    #[test]
    fn dfconf_expectations_rejects_layer1_pending() {
        let p = write_tmp(
            "bad1.expected.yaml",
            "layer_1: pending\nlayer_2: pass\nlayer_3: pass\nlayer_4: pass\n",
        );
        let err = load_expectations(&p).unwrap_err();
        assert!(matches!(err, ExpectationError::InvalidLayer1Pending { .. }));
    }

    #[test]
    fn dfconf_expectations_rejects_bad_dev_id() {
        let p = write_tmp(
            "baddev.expected.yaml",
            "layer_1: pass\nlayer_2: \"suppressed: NOT-AN-ID\"\nlayer_3: pass\nlayer_4: pass\n",
        );
        let err = load_expectations(&p).unwrap_err();
        assert!(matches!(err, ExpectationError::UnknownDeviationId { .. }));
    }
}
