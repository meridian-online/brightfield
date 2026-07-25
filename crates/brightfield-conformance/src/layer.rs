//! Conformance layer definitions and the `LayerCheck` trait seam.
//!
//! Each layer is an independently-diagnostic pass/fail gate. V1 ships
//! `AstRoundTripCheck` as the only concrete implementation; layers 2–4
//! return `LayerOutcome::Pending` with a named missing-infrastructure
//! reason. When a later card lands the SQL emitter / renderer / event pump,
//! flipping the corresponding `LayerCheck` from `Pending` to `Pass`/`Fail`
//! is all that's required — no plumbing rework.

use std::convert::TryFrom;
use std::fmt;
use std::path::PathBuf;

use brightfield_spec::{parse_spec, Format, Spec};

use crate::corpus::CorpusEntry;
use crate::deviations::DeviationRegistry;

/// The four equivalence layers. Each has a fixed 1..=4 discriminant so the
/// deviation registry can cite layers by number without coupling to Rust
/// variant ordering.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ConformanceLayer {
    /// AST round-trip — `parse → serialise → parse` is idempotent.
    AstRoundTrip = 1,
    /// SQL equivalence — emitted SQL produces matching result sets.
    SqlEquivalence = 2,
    /// Visual-encoding equivalence — mark+scale+channel structure matches.
    EncodingEquivalence = 3,
    /// Interaction equivalence — scripted events → matching selection state.
    InteractionEquivalence = 4,
}

impl ConformanceLayer {
    /// Human-readable full name.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::AstRoundTrip => "layer 1: AST round-trip",
            Self::SqlEquivalence => "layer 2: SQL equivalence",
            Self::EncodingEquivalence => "layer 3: encoding equivalence",
            Self::InteractionEquivalence => "layer 4: interaction equivalence",
        }
    }

    /// Numeric discriminant (stable cross-language identity).
    #[must_use]
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// All four layers in order.
    #[must_use]
    pub fn all() -> [Self; 4] {
        [
            Self::AstRoundTrip,
            Self::SqlEquivalence,
            Self::EncodingEquivalence,
            Self::InteractionEquivalence,
        ]
    }
}

impl fmt::Display for ConformanceLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// Invalid numeric discriminant supplied to `TryFrom<u8>`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid conformance layer discriminant {value}; expected 1..=4")]
pub struct InvalidLayer {
    /// The offending value.
    pub value: u8,
}

impl TryFrom<u8> for ConformanceLayer {
    type Error = InvalidLayer;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            1 => Ok(Self::AstRoundTrip),
            2 => Ok(Self::SqlEquivalence),
            3 => Ok(Self::EncodingEquivalence),
            4 => Ok(Self::InteractionEquivalence),
            _ => Err(InvalidLayer { value: v }),
        }
    }
}

/// The verdict a `LayerCheck` produces for a single (spec, layer) pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerOutcome {
    /// Layer passed.
    Pass,
    /// Layer failed — `details` is the diagnostic message.
    Fail {
        /// What went wrong.
        details: String,
    },
    /// Layer's expected divergence is accepted by a deviation registry entry.
    Suppressed {
        /// The deviation id that accepts the divergence.
        deviation_id: String,
    },
    /// Layer's machinery is not yet available in this build. The `reason`
    /// is a fixed compile-time literal — one reason per pending layer.
    Pending {
        /// Why the layer is pending.
        reason: &'static str,
    },
}

/// A single layer's check implementation.
pub trait LayerCheck: Send + Sync {
    /// Which layer this impl is responsible for.
    fn layer(&self) -> ConformanceLayer;
    /// Run the check and report what it actually observed. **Do not resolve
    /// suppression here** — an impl that returns `Suppressed` on its own makes
    /// the cell unfalsifiable, which is the failure the caller's
    /// coverage-is-not-sufficient rule exists to prevent. The registry is
    /// threaded through for diagnostics only; the caller decides whether an
    /// observed failure is suppressed.
    fn run(&self, spec: &Spec, fixture: &CorpusEntry, registry: &DeviationRegistry)
        -> LayerOutcome;
}

/// Layer 1: AST round-trip — `parse → serialise → parse` idempotence.
pub struct AstRoundTripCheck;

impl LayerCheck for AstRoundTripCheck {
    fn layer(&self) -> ConformanceLayer {
        ConformanceLayer::AstRoundTrip
    }
    fn run(
        &self,
        spec: &Spec,
        _fixture: &CorpusEntry,
        _registry: &DeviationRegistry,
    ) -> LayerOutcome {
        let serialised = match serde_yaml::to_string(spec) {
            Ok(s) => s,
            Err(e) => {
                return LayerOutcome::Fail {
                    details: format!("serialise error: {e}"),
                }
            }
        };
        let reparsed = match parse_spec(&serialised, Format::Yaml) {
            Ok(o) => o,
            Err(e) => {
                return LayerOutcome::Fail {
                    details: format!("second parse failed: {e}"),
                }
            }
        };
        if *spec == reparsed.spec {
            LayerOutcome::Pass
        } else {
            LayerOutcome::Fail {
                details: "AST inequality after round-trip".to_string(),
            }
        }
    }
}

/// Layer 2: SQL equivalence — data-source DDL slice.
///
/// Calls the `brightfield-sql` emitter to produce DDL for the spec's data
/// sources, canonicalises the output, and string-diffs against the sibling
/// `<name>.layer2.expected.sql` file. Plot-SELECT conformance stays pending.
pub struct SqlEquivalenceCheck;

impl SqlEquivalenceCheck {
    /// Compute the expected-SQL fixture path for a given corpus entry.
    fn expected_sql_path(fixture: &CorpusEntry) -> PathBuf {
        let stem = fixture
            .source_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        fixture
            .source_path
            .with_file_name(format!("{stem}.layer2.expected.sql"))
    }
}

impl LayerCheck for SqlEquivalenceCheck {
    fn layer(&self) -> ConformanceLayer {
        ConformanceLayer::SqlEquivalence
    }
    fn run(
        &self,
        spec: &Spec,
        fixture: &CorpusEntry,
        _registry: &DeviationRegistry,
    ) -> LayerOutcome {
        // A spec with no `data:` block has no data-source DDL to be equivalent
        // ABOUT. Answering Pass there is a green cell earned by having nothing
        // to check — which is exactly how `legends.yaml` came to sit behind a
        // zero-byte expected-SQL fixture and still count towards the curated
        // pass total. Say what is true instead.
        if spec.data.is_empty() {
            return LayerOutcome::Pending {
                reason: "spec declares no data sources",
            };
        }
        // Check for expected SQL fixture
        let expected_path = Self::expected_sql_path(fixture);
        if !expected_path.exists() {
            return LayerOutcome::Pending {
                reason: "no expected SQL fixture",
            };
        }

        // Read expected SQL
        let expected = match std::fs::read_to_string(&expected_path) {
            Ok(s) => s,
            Err(e) => {
                return LayerOutcome::Fail {
                    details: format!(
                        "failed to read expected SQL at {}: {e}",
                        expected_path.display()
                    ),
                };
            }
        };

        // Emit sources — pass None as base_dir so paths stay relative
        // in canonical form. Conformance compares SQL shape, not resolved paths.
        let output = match brightfield_sql::emit::emit_sources(spec, None) {
            Ok(o) => o,
            Err(e) => {
                return LayerOutcome::Fail {
                    details: format!("emission error: {e}"),
                };
            }
        };

        // Canonicalise and compare
        let actual = brightfield_sql::render::canonicalise_ddl(&output.statements);

        if actual == expected {
            LayerOutcome::Pass
        } else {
            // Produce a readable diff
            let mut diff = String::from("DDL mismatch:\n--- expected\n+++ actual\n");
            for (i, (exp_line, act_line)) in expected.lines().zip(actual.lines()).enumerate() {
                if exp_line != act_line {
                    diff.push_str(&format!(
                        "line {}: expected '{}', got '{}'\n",
                        i + 1,
                        exp_line,
                        act_line
                    ));
                }
            }
            let exp_count = expected.lines().count();
            let act_count = actual.lines().count();
            if exp_count != act_count {
                diff.push_str(&format!(
                    "line count: expected {exp_count}, actual {act_count}\n"
                ));
            }
            LayerOutcome::Fail { details: diff }
        }
    }
}

/// Layer 3: visual-encoding equivalence.
///
/// Pending for want of an **oracle**, not for want of a renderer. The
/// renderer shipped months ago and draws every curated spec; what does not
/// exist is the thing that would compare a rendered brightfield scene's
/// mark/scale/channel structure against Mosaic web's for the same spec. The
/// reason string used to say "renderer not yet available", which was false in
/// the product's own repo and contradicted in prose by the deviation registry
/// on the very cells it was reported for.
pub struct EncodingEquivalenceCheck;

impl LayerCheck for EncodingEquivalenceCheck {
    fn layer(&self) -> ConformanceLayer {
        ConformanceLayer::EncodingEquivalence
    }
    fn run(
        &self,
        _spec: &Spec,
        _fixture: &CorpusEntry,
        _registry: &DeviationRegistry,
    ) -> LayerOutcome {
        LayerOutcome::Pending {
            reason:
                "no encoding-equivalence oracle: nothing diffs a rendered scene against Mosaic web",
        }
    }
}

/// Layer 4: interaction equivalence.
///
/// Pending for want of an oracle, not for want of an event pump. The shipped
/// Interaction/Coordinator seam IS a scriptable event pump — an interaction
/// resolves to a pushed predicate and a re-query, and headless tests drive it
/// today. What is missing is Mosaic web's selection state for the same
/// scripted events to compare against.
pub struct InteractionEquivalenceCheck;

impl LayerCheck for InteractionEquivalenceCheck {
    fn layer(&self) -> ConformanceLayer {
        ConformanceLayer::InteractionEquivalence
    }
    fn run(
        &self,
        _spec: &Spec,
        _fixture: &CorpusEntry,
        _registry: &DeviationRegistry,
    ) -> LayerOutcome {
        LayerOutcome::Pending {
            reason: "no interaction-equivalence oracle: no Mosaic-web selection state to compare against",
        }
    }
}

/// Build the V1 registry of concrete `LayerCheck` impls. One per layer,
/// stable order 1..=4.
#[must_use]
pub fn default_layer_checks() -> Vec<Box<dyn LayerCheck>> {
    vec![
        Box::new(AstRoundTripCheck),
        Box::new(SqlEquivalenceCheck),
        Box::new(EncodingEquivalenceCheck),
        Box::new(InteractionEquivalenceCheck),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::CorpusEntry;
    use crate::expectations::{Layer1Expectation, LayerExpectations, LayerNExpectation};
    use brightfield_spec::parse_spec;
    use std::path::PathBuf;

    fn test_fixture() -> CorpusEntry {
        CorpusEntry {
            name: "synthetic".to_string(),
            source_path: PathBuf::from("synthetic.yaml"),
            expectations: LayerExpectations {
                layer_1: Layer1Expectation::Pass,
                layer_2: LayerNExpectation::Pending,
                layer_3: LayerNExpectation::Pending,
                layer_4: LayerNExpectation::Pending,
            },
        }
    }

    #[test]
    fn dfconf_layer_discriminants_match_spec() {
        assert_eq!(ConformanceLayer::AstRoundTrip as u8, 1);
        assert_eq!(ConformanceLayer::SqlEquivalence as u8, 2);
        assert_eq!(ConformanceLayer::EncodingEquivalence as u8, 3);
        assert_eq!(ConformanceLayer::InteractionEquivalence as u8, 4);
    }

    #[test]
    fn dfconf_layer_try_from_u8_round_trip() {
        for l in ConformanceLayer::all() {
            assert_eq!(ConformanceLayer::try_from(l.as_u8()).unwrap(), l);
        }
        assert!(ConformanceLayer::try_from(0).is_err());
        assert!(ConformanceLayer::try_from(5).is_err());
    }

    #[test]
    fn dfconf_layer_display_format() {
        assert_eq!(
            ConformanceLayer::AstRoundTrip.to_string(),
            "layer 1: AST round-trip"
        );
        assert_eq!(
            ConformanceLayer::SqlEquivalence.to_string(),
            "layer 2: SQL equivalence"
        );
    }

    #[test]
    fn dfconf_ast_round_trip_passes_on_valid_spec() {
        let src = "meta:\n  title: t\nplot:\n  - mark: line\n    data: { from: d }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let reg = DeviationRegistry::default();
        let outcome = AstRoundTripCheck.run(&spec, &test_fixture(), &reg);
        assert_eq!(outcome, LayerOutcome::Pass);
    }

    #[test]
    fn dfconf_layer_2_missing_fixture_is_pending() {
        // Synthetic fixture with data sources but no .layer2.expected.sql
        // sibling → Pending on the fixture, not on the data.
        let src = "data:\n  d: { file: d.parquet }\nplot:\n  - mark: line\n    data: { from: d }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let reg = DeviationRegistry::default();
        let outcome = SqlEquivalenceCheck.run(&spec, &test_fixture(), &reg);
        assert_eq!(
            outcome,
            LayerOutcome::Pending {
                reason: "no expected SQL fixture"
            }
        );
    }

    /// A spec with no `data:` block has no data-source DDL to be equivalent
    /// about, and must not bank a green cell for having nothing to check.
    /// This is what the zero-byte `legends.layer2.expected.sql` was doing.
    #[test]
    fn dfconf_layer_2_dataless_spec_is_pending_not_pass() {
        let src = "plot:\n  - mark: line\n    data: { from: d }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        assert!(spec.data.is_empty(), "fixture declares no data sources");
        let reg = DeviationRegistry::default();
        let outcome = SqlEquivalenceCheck.run(&spec, &test_fixture(), &reg);
        assert_eq!(
            outcome,
            LayerOutcome::Pending {
                reason: "spec declares no data sources"
            },
            "a dataless spec must not pass layer 2 by vacuum"
        );
    }

    #[test]
    fn dfconf_layer_3_returns_pending() {
        let src = "plot:\n  - mark: line\n    data: { from: d }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let reg = DeviationRegistry::default();
        let outcome = EncodingEquivalenceCheck.run(&spec, &test_fixture(), &reg);
        let LayerOutcome::Pending { reason } = outcome else {
            panic!("layer 3 has no oracle yet: {outcome:?}");
        };
        assert!(
            reason.contains("oracle"),
            "the pending reason must name the missing ORACLE — the renderer \
             shipped, and saying otherwise is the stale string this replaced: {reason}"
        );
        assert!(
            !reason.contains("renderer not yet"),
            "the renderer is available; {reason} is false"
        );
    }

    #[test]
    fn dfconf_layer_4_returns_pending() {
        let src = "plot:\n  - mark: line\n    data: { from: d }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let reg = DeviationRegistry::default();
        let outcome = InteractionEquivalenceCheck.run(&spec, &test_fixture(), &reg);
        let LayerOutcome::Pending { reason } = outcome else {
            panic!("layer 4 has no oracle yet: {outcome:?}");
        };
        assert!(
            reason.contains("oracle"),
            "the pending reason must name the missing ORACLE — the shipped \
             Interaction/Coordinator seam IS a scriptable event pump: {reason}"
        );
        assert!(
            !reason.contains("event pump not yet"),
            "the event pump exists and is driven by headless tests; {reason} is false"
        );
    }

    #[test]
    fn dfconf_layer_check_dyn_dispatch() {
        let src = "plot:\n  - mark: line\n    data: { from: d }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let reg = DeviationRegistry::default();
        let checks = default_layer_checks();
        assert_eq!(checks.len(), 4);
        for check in &checks {
            let _ = check.run(&spec, &test_fixture(), &reg);
        }
    }
}
