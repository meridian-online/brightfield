// INTERIM CONTRACT
//! Interim arcform-manifest parser.
//!
//! Serde model of arcform's Protocol manifest *as it exists today*
//! (name/engine/params/defaults/steps; steps carry `op: name@version` +
//! `with:`, or `sql:` model paths + `depends_on`). Deliberately thin and
//! deletable: this module swaps for the arcform-emitted Protocol+Run JSON
//! when that contract lands upstream — nothing outside this crate should
//! depend on its serde shapes.
//!
//! Unknown keys are **ignored, never errors** (serde's default), so the
//! parser survives manifest evolution (`produces:`, `retry:`, `timeout_sec:`
//! and whatever comes next all pass through silently).

use serde::Deserialize;

/// A Protocol manifest: the ordered step list plus engine/params metadata.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    /// Protocol name — the namespace for every derived asset id.
    pub name: String,
    /// Execution engine (`duckdb`); informational here.
    #[serde(default)]
    pub engine: Option<String>,
    /// Engine version constraint; informational here.
    #[serde(default)]
    pub engine_version: Option<String>,
    /// Engine database path; informational here.
    #[serde(default)]
    pub db: Option<String>,
    /// Declared run parameters, preserved raw for later cards' seam labels.
    #[serde(default)]
    pub params: serde_yaml::Mapping,
    /// Step defaults (retry policy etc.), preserved raw.
    #[serde(default)]
    pub defaults: serde_yaml::Mapping,
    /// The ordered steps — the graph derivation input.
    #[serde(default)]
    pub steps: Vec<Step>,
}

/// One manifest step: a typed operator (`op:` + `with:`), a SQL model
/// (`sql:` path), an opaque command, or none of those (parses as opaque —
/// never an error).
#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    /// Step name — unique within the manifest; the seam identity.
    pub name: String,
    /// `"http_fetch@1"` — split into (name, version) by [`Step::op_parts`].
    #[serde(default)]
    pub op: Option<String>,
    /// Operator arguments, preserved raw for asset derivation + seam labels.
    #[serde(default)]
    pub with: Option<serde_yaml::Mapping>,
    /// SQL model path relative to the manifest's parent directory.
    #[serde(default)]
    pub sql: Option<String>,
    /// Opaque-by-design shell steps.
    #[serde(default)]
    pub command: Option<String>,
    /// File/table refs this step reads (paths or bare relation idents).
    #[serde(default)]
    pub depends_on: Vec<String>,
}

impl Step {
    /// Split `op` into `(name, version)`; `op` without `@` yields version
    /// `"?"`. `None` when the step has no `op`.
    #[must_use]
    pub fn op_parts(&self) -> Option<(String, String)> {
        let raw = self.op.as_deref()?;
        Some(match raw.split_once('@') {
            Some((name, version)) => (name.to_string(), version.to_string()),
            None => (raw.to_string(), "?".to_string()),
        })
    }

    /// The string value of a `with:` key, if present.
    #[must_use]
    pub fn with_str(&self, key: &str) -> Option<&str> {
        self.with.as_ref()?.get(key)?.as_str()
    }
}

/// Parse a manifest from YAML text.
///
/// # Errors
///
/// Returns the serde error when the text is not a mapping with the required
/// `name` (unknown keys never error — see the module docs).
pub fn parse_manifest_str(text: &str) -> Result<Manifest, serde_yaml::Error> {
    serde_yaml::from_str(text)
}

/// Mosaic-spec keys that decide a YAML belongs to the existing spec pipeline,
/// never the protocol arm — top-level `data:`/`meta:`/`config:` plus every
/// component-root discriminator the Mosaic parser recognises (`walk_component`:
/// plot/vconcat/hconcat/hspace/vspace/legend/mark/select/input). A Mosaic spec
/// carrying a stray `steps:` key must still route to the spec pipeline, so any
/// of these present vetoes the sniff. (`params:` is deliberately absent — a
/// Protocol manifest legitimately carries it too.)
const MOSAIC_KEYS: &[&str] = &[
    "plot",
    "data",
    "vconcat",
    "hconcat",
    "hspace",
    "vspace",
    "legend",
    "mark",
    "select",
    "input",
    "meta",
    "config",
    "plotDefaults",
];

/// The protocol sniff (the guard): a YAML document whose top level is a
/// mapping with a `steps:` **sequence** and NONE of the Mosaic keys is a
/// Protocol manifest. Anything else — including every Mosaic spec, even one
/// with a stray `steps:` key — is not, so Mosaic inputs reach the existing
/// spec parser byte-untouched.
#[must_use]
pub fn is_protocol_manifest(text: &str) -> bool {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(text) else {
        return false;
    };
    let Some(map) = value.as_mapping() else {
        return false;
    };
    let steps_is_sequence = map.get("steps").is_some_and(serde_yaml::Value::is_sequence);
    let has_mosaic_keys = MOSAIC_KEYS.iter().any(|k| map.contains_key(*k));
    steps_is_sequence && !has_mosaic_keys
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINI: &str = r"
name: mini
engine: duckdb
steps:
  - name: fetch
    op: http_fetch@1
    with:
      url: https://example.com/a.csv
      out: build/a.csv
  - name: transform
    sql: models/t.sql
    depends_on: [build/a.csv]
    produces: [t]
    retry:
      max_attempts: 1
  - name: legacy
    command: ./scripts/legacy.sh
  - name: mystery
";

    #[test]
    fn pds_manifest_parses_steps_and_ignores_unknown_keys() {
        // `produces:` and `retry:` are unknown to the interim contract — they
        // must be ignored, never errors.
        let m = parse_manifest_str(MINI).expect("parses");
        assert_eq!(m.name, "mini");
        assert_eq!(m.steps.len(), 4);
        assert_eq!(m.steps[1].sql.as_deref(), Some("models/t.sql"));
        assert_eq!(m.steps[1].depends_on, vec!["build/a.csv".to_string()]);
    }

    #[test]
    fn pds_manifest_op_version_split() {
        let m = parse_manifest_str(MINI).unwrap();
        assert_eq!(
            m.steps[0].op_parts(),
            Some(("http_fetch".to_string(), "1".to_string()))
        );
        // op without `@` degrades to version "?".
        let step: Step = serde_yaml::from_str("{ name: x, op: http_fetch }").unwrap();
        assert_eq!(
            step.op_parts(),
            Some(("http_fetch".to_string(), "?".to_string()))
        );
        // A step with none of op/sql/command still parses (opaque step).
        let opaque = &m.steps[3];
        assert!(opaque.op.is_none() && opaque.sql.is_none() && opaque.command.is_none());
    }

    #[test]
    fn pds_sniff_accepts_protocol_rejects_mosaic() {
        assert!(is_protocol_manifest(MINI));
        // A Mosaic spec has data/plot at top level — never sniffed as protocol.
        let mosaic = "data:\n  points:\n    - { x: 1 }\nplot:\n  - mark: dot\n";
        assert!(!is_protocol_manifest(mosaic));
        // steps alongside a plot key stays Mosaic (conservative sniff).
        let hybrid = "steps:\n  - name: a\nplot:\n  - mark: dot\n";
        assert!(!is_protocol_manifest(hybrid));
        // steps must be a sequence.
        assert!(!is_protocol_manifest("steps: 3\n"));
        // Non-mapping and non-YAML inputs are not protocols.
        assert!(!is_protocol_manifest("- 1\n- 2\n"));
        assert!(!is_protocol_manifest(": : :"));
    }

    #[test]
    fn pds_sniff_rejects_mosaic_component_roots_with_stray_steps() {
        // A Mosaic spec whose root is a component discriminator (no top-level
        // plot/data) plus a stray `steps:` sequence must still route to the
        // spec pipeline, not the protocol arm.
        for root in [
            "vconcat", "hconcat", "hspace", "vspace", "legend", "mark", "select", "input",
        ] {
            let spec = format!("{root}:\n  - a\nsteps:\n  - name: x\n");
            assert!(
                !is_protocol_manifest(&spec),
                "a Mosaic `{root}` root with a stray steps: must not sniff as protocol"
            );
        }
        // meta/config roots likewise veto.
        assert!(!is_protocol_manifest(
            "meta:\n  title: t\nsteps:\n  - name: x\n"
        ));
    }
}
