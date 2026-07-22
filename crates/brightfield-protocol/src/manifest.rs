//! Arcform-manifest ingest — the `arc` crate's loader, not a copy of it.
//!
//! This module used to carry brightfield's own serde model of the Protocol
//! manifest (an "interim contract": a second schema, deliberately lenient,
//! kept in sync by discipline). That model is deleted. The schema, the loader
//! and the validators now come from the `arc` crate itself (`arc::spec`),
//! linked as a rev-pinned git dependency — so a spec brightfield accepts and a
//! spec `arc run` accepts are the same thing **by construction**, and a spec
//! arc would refuse gets refused here too, with arc's own diagnostic.
//!
//! [`parse_manifest_str`] is exactly the parse + validation gate `arc run`
//! loads with ([`Manifest::from_yaml_str`]): names unique, exactly one of
//! `sql:`/`command:`/`op:` per step, `op:` steps resolved against the operator
//! catalog with a `with:` block that deserialises, well-formed preconditions
//! and retry policies. The consequence for callers: manifests that the old
//! interim parser tolerated (an armless step, an unknown operator) are now
//! **errors**, matching what `arc` itself would say.
//!
//! Writing is arc's business too. Editing an existing spec goes through
//! `arc::spec::{SpecEdit, apply_edits, edit_spec}` — format-preserving splices
//! into the original bytes, re-gated by the same loader — so brightfield never
//! serialises a whole spec it did not create (asserted by
//! `tests/roundtrip.rs` over a hand-authored hostile corpus).
//!
//! The one piece of brightfield-side logic left here is
//! [`is_protocol_manifest`]: the routing sniff that decides whether a YAML
//! belongs to the existing Mosaic-spec pipeline or the protocol arm. That is a
//! router, not a schema — it never validates, and Mosaic inputs reach the spec
//! parser byte-untouched.

pub use arc::spec::{Manifest, Step};

/// Derived accessors on [`Step`] that brightfield's graph derivation uses.
///
/// These are *readers*, not schema: the schema is arc's. They live here so the
/// graph builder can keep asking the questions it asks without brightfield
/// owning any manifest shape of its own.
pub trait StepExt {
    /// Split `op` into `(name, version)`; `op` without `@` yields version
    /// `"?"`. `None` when the step has no `op`.
    #[must_use]
    fn op_parts(&self) -> Option<(String, String)>;

    /// The string value of a `with:` key, if present.
    #[must_use]
    fn with_str(&self, key: &str) -> Option<&str>;
}

impl StepExt for Step {
    fn op_parts(&self) -> Option<(String, String)> {
        let raw = self.op.as_deref()?;
        Some(match raw.split_once('@') {
            Some((name, version)) => (name.to_string(), version.to_string()),
            None => (raw.to_string(), "?".to_string()),
        })
    }

    fn with_str(&self, key: &str) -> Option<&str> {
        self.with.as_ref()?.get(key)?.as_str()
    }
}

/// Parse **and validate** a manifest from YAML text — the same gate `arc run`
/// loads with.
///
/// # Errors
///
/// arc's own error: a parse error when the text is not YAML in the shape of a
/// manifest, or a validation error when the shape is right but the content is
/// not (duplicate step names, an armless step, an unknown operator, a `with:`
/// block that does not deserialise, …).
pub fn parse_manifest_str(text: &str) -> Result<Manifest, arc::spec::Error> {
    Manifest::from_yaml_str(text)
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
";

    #[test]
    fn pds_manifest_parses_through_arcs_gate() {
        let m = parse_manifest_str(MINI).expect("parses");
        assert_eq!(m.name, "mini");
        assert_eq!(m.steps.len(), 3);
        assert_eq!(m.steps[1].sql.as_deref(), Some("models/t.sql"));
        assert_eq!(m.steps[1].depends_on, vec!["build/a.csv".to_string()]);
        assert_eq!(m.steps[1].produces, vec!["t".to_string()]);
    }

    #[test]
    fn pds_manifest_op_version_split() {
        let m = parse_manifest_str(MINI).unwrap();
        assert_eq!(
            m.steps[0].op_parts(),
            Some(("http_fetch".to_string(), "1".to_string()))
        );
        // op without `@` degrades to version "?" on the accessor; whether a
        // whole manifest carrying it loads is the catalog's call, not ours.
        let step: Step = serde_yaml::from_str("{ name: x, op: http_fetch }").unwrap();
        assert_eq!(
            step.op_parts(),
            Some(("http_fetch".to_string(), "?".to_string()))
        );
        assert_eq!(m.steps[0].with_str("out"), Some("build/a.csv"));
        assert_eq!(m.steps[0].with_str("absent"), None);
    }

    #[test]
    fn pds_gate_refuses_what_arc_refuses() {
        // An armless step (no sql/command/op) — tolerated by the deleted
        // interim parser, refused by arc's validator, therefore refused here.
        let err = parse_manifest_str("name: m\nsteps:\n  - name: mystery\n").unwrap_err();
        assert!(
            err.to_string().contains("must have one of"),
            "arc's own diagnostic surfaces: {err}"
        );
        // An operator the catalog does not know is refused at load, not at run.
        let err =
            parse_manifest_str("name: m\nsteps:\n  - name: x\n    op: not_a_real_operator@1\n")
                .unwrap_err();
        assert!(
            err.to_string().contains("unknown operator"),
            "catalog resolution is part of the gate: {err}"
        );
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
