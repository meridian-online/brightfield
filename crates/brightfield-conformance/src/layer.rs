//! Conformance layer definitions and the `LayerCheck` trait seam (ac-05, ac-06).
//!
//! Each layer is an independently-diagnostic pass/fail gate. V1 ships
//! `AstRoundTripCheck` as the only concrete implementation; layers 2–4
//! return `LayerOutcome::Pending` with a named missing-infrastructure
//! reason. When a later card lands the SQL emitter / renderer / event pump,
//! flipping the corresponding `LayerCheck` from `Pending` to `Pass`/`Fail`
//! is all that's required — no plumbing rework.

use std::convert::TryFrom;
use std::fmt;

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
    /// Run the check. The registry is threaded through so concrete impls can
    /// resolve suppression themselves (e.g. SQL-equivalence may note that
    /// the expected divergence is registered and emit `Suppressed`).
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
            Err(e) => return LayerOutcome::Fail {
                details: format!("serialise error: {e}"),
            },
        };
        let reparsed = match parse_spec(&serialised, Format::Yaml) {
            Ok(o) => o,
            Err(e) => return LayerOutcome::Fail {
                details: format!("second parse failed: {e}"),
            },
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

/// Layer 2: SQL equivalence. V1 stub — returns `Pending` until the SQL
/// emitter lands.
pub struct SqlEquivalenceCheck;

impl LayerCheck for SqlEquivalenceCheck {
    fn layer(&self) -> ConformanceLayer {
        ConformanceLayer::SqlEquivalence
    }
    fn run(
        &self,
        _spec: &Spec,
        _fixture: &CorpusEntry,
        _registry: &DeviationRegistry,
    ) -> LayerOutcome {
        LayerOutcome::Pending {
            reason: "SQL emitter not yet available",
        }
    }
}

/// Layer 3: visual-encoding equivalence. V1 stub — returns `Pending` until
/// the renderer lands.
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
            reason: "renderer not yet available",
        }
    }
}

/// Layer 4: interaction equivalence. V1 stub — returns `Pending` until the
/// event pump lands.
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
            reason: "event pump not yet available",
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
    fn dfconf_layer_2_returns_pending() {
        let src = "plot:\n  - mark: line\n    data: { from: d }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let reg = DeviationRegistry::default();
        let outcome = SqlEquivalenceCheck.run(&spec, &test_fixture(), &reg);
        assert_eq!(
            outcome,
            LayerOutcome::Pending {
                reason: "SQL emitter not yet available"
            }
        );
    }

    #[test]
    fn dfconf_layer_3_returns_pending() {
        let src = "plot:\n  - mark: line\n    data: { from: d }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let reg = DeviationRegistry::default();
        let outcome = EncodingEquivalenceCheck.run(&spec, &test_fixture(), &reg);
        assert_eq!(
            outcome,
            LayerOutcome::Pending {
                reason: "renderer not yet available"
            }
        );
    }

    #[test]
    fn dfconf_layer_4_returns_pending() {
        let src = "plot:\n  - mark: line\n    data: { from: d }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let reg = DeviationRegistry::default();
        let outcome = InteractionEquivalenceCheck.run(&spec, &test_fixture(), &reg);
        assert_eq!(
            outcome,
            LayerOutcome::Pending {
                reason: "event pump not yet available"
            }
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
