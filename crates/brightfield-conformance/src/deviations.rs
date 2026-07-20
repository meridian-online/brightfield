//! Deviation registry.
//!
//! The registry lives at repo-root `deviations.yaml` and is the single
//! source of truth for deliberate divergences from Mosaic-web rendering.
//! The loader is a pure file-level validator: YAML syntax, id uniqueness,
//! id format, layer-range, field completeness. Cross-artefact integrity
//! (affected-spec names referencing real curated files) is owned by
//! AC-14's registry-integrity gate, not by this loader.

use std::fs;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::layer::ConformanceLayer;

/// One deviation record. All fields are `pub` so consumers can read without
/// accessors; the registry integrity gate walks the struct directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deviation {
    /// Stable identifier, shape `DEV-NNNN`.
    pub id: String,
    /// Surface name (e.g. "legend", "tooltip").
    pub surface: String,
    /// One-line description of Mosaic web's behaviour.
    pub mosaic_behaviour: String,
    /// One-line description of brightfield's chosen native behaviour.
    pub brightfield_behaviour: String,
    /// Why the deviation exists.
    pub rationale: String,
    /// Filenames in the curated corpus this deviation applies to.
    #[serde(default)]
    pub affected_specs: Vec<String>,
    /// Layers at which the divergence is accepted.
    #[serde(default)]
    pub conformance_layers_suppressed: Vec<ConformanceLayer>,
}

/// The loaded registry. Keys are `id`; insertion order from source is
/// preserved for deterministic output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviationRegistry {
    /// All entries keyed by id.
    pub entries: IndexMap<String, Deviation>,
}

impl DeviationRegistry {
    /// Look up a deviation by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Deviation> {
        self.entries.get(id)
    }

    /// Iterate deviations in source order.
    pub fn iter(&self) -> impl Iterator<Item = &Deviation> {
        self.entries.values()
    }

    /// Count of deviations in the registry.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` iff the registry has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Errors raised by the registry loader.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// OS I/O error.
    #[error("registry I/O error at {path:?}: {source}")]
    Io {
        /// File path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// YAML syntax error.
    #[error("registry YAML syntax error: {error}")]
    YamlSyntax {
        /// Error detail.
        error: String,
    },
    /// Two entries share the same `id`.
    #[error("registry has duplicate id {id:?}")]
    DuplicateId {
        /// The duplicated id.
        id: String,
    },
    /// An entry references a conformance layer outside `1..=4`.
    #[error("registry entry {id:?} references invalid layer {layer}; expected 1..=4")]
    InvalidLayer {
        /// The offending entry.
        id: String,
        /// Bad layer number.
        layer: u8,
    },
    /// An entry's `id` does not match `DEV-NNNN`.
    #[error("registry entry has invalid id format {id:?}; expected DEV-NNNN")]
    InvalidIdFormat {
        /// The malformed id.
        id: String,
    },
    /// An entry is missing a required field.
    #[error("registry entry {id:?} missing field {field}")]
    MissingField {
        /// The offending entry.
        id: String,
        /// Missing field name.
        field: &'static str,
    },
}

/// Raw wire schema — kept separate from `Deviation` so YAML-level errors
/// can surface before the typed `ConformanceLayer` conversion runs.
#[derive(Debug, Deserialize)]
struct DeviationWire {
    id: String,
    surface: Option<String>,
    mosaic_behaviour: Option<String>,
    brightfield_behaviour: Option<String>,
    rationale: Option<String>,
    #[serde(default)]
    affected_specs: Vec<String>,
    #[serde(default)]
    conformance_layers_suppressed: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct RegistryWire {
    #[serde(default)]
    deviations: Vec<DeviationWire>,
}

/// Parse a `deviations.yaml` file. Enforces id-uniqueness, `DEV-NNNN`
/// format, layer-range, and field completeness. Does NOT check that
/// `affected_specs` names point at real curated files — that check is
/// AC-14's.
pub fn load_deviations(path: &Path) -> Result<DeviationRegistry, RegistryError> {
    let source = fs::read_to_string(path).map_err(|source| RegistryError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let wire: RegistryWire = serde_yaml::from_str(&source).map_err(|e| RegistryError::YamlSyntax {
        error: e.to_string(),
    })?;
    let mut entries: IndexMap<String, Deviation> = IndexMap::new();
    for w in wire.deviations {
        check_id_format(&w.id)?;
        let surface = required(w.surface, &w.id, "surface")?;
        let mosaic_behaviour = required(w.mosaic_behaviour, &w.id, "mosaic_behaviour")?;
        let brightfield_behaviour = required(w.brightfield_behaviour, &w.id, "brightfield_behaviour")?;
        let rationale = required(w.rationale, &w.id, "rationale")?;
        let mut layers: Vec<ConformanceLayer> = Vec::new();
        for n in w.conformance_layers_suppressed {
            let layer =
                ConformanceLayer::try_from(n).map_err(|_| RegistryError::InvalidLayer {
                    id: w.id.clone(),
                    layer: n,
                })?;
            layers.push(layer);
        }
        let dev = Deviation {
            id: w.id.clone(),
            surface,
            mosaic_behaviour,
            brightfield_behaviour,
            rationale,
            affected_specs: w.affected_specs,
            conformance_layers_suppressed: layers,
        };
        if entries.insert(w.id.clone(), dev).is_some() {
            return Err(RegistryError::DuplicateId { id: w.id });
        }
    }
    Ok(DeviationRegistry { entries })
}

fn check_id_format(id: &str) -> Result<(), RegistryError> {
    if let Some(rest) = id.strip_prefix("DEV-") {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            return Ok(());
        }
    }
    Err(RegistryError::InvalidIdFormat {
        id: id.to_string(),
    })
}

fn required<T>(value: Option<T>, id: &str, field: &'static str) -> Result<T, RegistryError> {
    value.ok_or_else(|| RegistryError::MissingField {
        id: id.to_string(),
        field,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tmp(name: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dfconf-reg-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn dfconf_deviation_struct_shape() {
        let d = Deviation {
            id: "DEV-0001".to_string(),
            surface: "legend".to_string(),
            mosaic_behaviour: "a".to_string(),
            brightfield_behaviour: "b".to_string(),
            rationale: "c".to_string(),
            affected_specs: vec!["crossfilter".to_string()],
            conformance_layers_suppressed: vec![ConformanceLayer::EncodingEquivalence],
        };
        assert_eq!(d.id, "DEV-0001");
        assert_eq!(d.affected_specs, vec!["crossfilter"]);
    }

    #[test]
    fn dfconf_load_empty_registry() {
        let p = write_tmp("empty.yaml", "deviations: []\n");
        let r = load_deviations(&p).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn dfconf_load_rejects_duplicate_id() {
        let p = write_tmp(
            "dupe.yaml",
            r#"deviations:
  - id: DEV-0001
    surface: legend
    mosaic_behaviour: a
    brightfield_behaviour: b
    rationale: c
  - id: DEV-0001
    surface: tooltip
    mosaic_behaviour: a
    brightfield_behaviour: b
    rationale: c
"#,
        );
        let err = load_deviations(&p).unwrap_err();
        assert!(matches!(err, RegistryError::DuplicateId { .. }));
    }

    #[test]
    fn dfconf_load_rejects_invalid_layer() {
        let p = write_tmp(
            "badlayer.yaml",
            r#"deviations:
  - id: DEV-0001
    surface: legend
    mosaic_behaviour: a
    brightfield_behaviour: b
    rationale: c
    conformance_layers_suppressed: [5]
"#,
        );
        let err = load_deviations(&p).unwrap_err();
        assert!(matches!(err, RegistryError::InvalidLayer { layer: 5, .. }));
    }

    #[test]
    fn dfconf_load_rejects_bad_id_format() {
        let p = write_tmp(
            "badid.yaml",
            r#"deviations:
  - id: NOT-AN-ID
    surface: legend
    mosaic_behaviour: a
    brightfield_behaviour: b
    rationale: c
"#,
        );
        let err = load_deviations(&p).unwrap_err();
        assert!(matches!(err, RegistryError::InvalidIdFormat { .. }));
    }

    #[test]
    fn dfconf_load_rejects_missing_field() {
        let p = write_tmp(
            "missing.yaml",
            r#"deviations:
  - id: DEV-0001
    surface: legend
    mosaic_behaviour: a
    brightfield_behaviour: b
"#,
        );
        let err = load_deviations(&p).unwrap_err();
        assert!(matches!(
            err,
            RegistryError::MissingField { field: "rationale", .. }
        ));
    }

    #[test]
    fn dfconf_registry_error_display_names_id() {
        let e = RegistryError::DuplicateId {
            id: "DEV-0005".to_string(),
        };
        assert!(e.to_string().contains("DEV-0005"));
        let e = RegistryError::InvalidLayer {
            id: "DEV-0006".to_string(),
            layer: 9,
        };
        assert!(e.to_string().contains("DEV-0006"));
        let e = RegistryError::InvalidIdFormat {
            id: "WRONG".to_string(),
        };
        assert!(e.to_string().contains("WRONG"));
        let e = RegistryError::MissingField {
            id: "DEV-0007".to_string(),
            field: "rationale",
        };
        assert!(e.to_string().contains("DEV-0007"));
    }
}
