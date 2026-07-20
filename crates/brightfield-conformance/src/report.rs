//! Conformance runner.
//!
//! `run_conformance` is the library entry point both `cargo test` and the
//! `conformance` binary dispatch through. It enumerates the requested
//! corpus, parses each spec, and runs each requested `LayerCheck` against
//! it — surfacing per-(spec, layer) [`LayerOutcome`]s plus a roll-up
//! [`ReportSummary`].
//!
//! Unknown layers (outside 1..=4) are filtered silently.

use std::fs;

use brightfield_spec::{parse_spec, Format};

use crate::corpus::{curated_entries, observed_entries, Corpus, CorpusEntry};
use crate::deviations::DeviationRegistry;
use crate::layer::{
    default_layer_checks, ConformanceLayer, LayerCheck, LayerOutcome,
};

/// One `(spec, layer)` record in a [`ConformanceReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerRecord {
    /// The spec's filename stem (e.g. `crossfilter`).
    pub spec_name: String,
    /// The layer this outcome relates to.
    pub layer: ConformanceLayer,
    /// The verdict the `LayerCheck` produced.
    pub outcome: LayerOutcome,
}

/// Per-outcome roll-up counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReportSummary {
    /// Count of `LayerOutcome::Pass`.
    pub passed: usize,
    /// Count of `LayerOutcome::Fail`.
    pub failed: usize,
    /// Count of `LayerOutcome::Suppressed`.
    pub suppressed: usize,
    /// Count of `LayerOutcome::Pending`.
    pub pending: usize,
}

impl ReportSummary {
    /// Increment the counter for `outcome`.
    fn bump(&mut self, outcome: &LayerOutcome) {
        match outcome {
            LayerOutcome::Pass => self.passed += 1,
            LayerOutcome::Fail { .. } => self.failed += 1,
            LayerOutcome::Suppressed { .. } => self.suppressed += 1,
            LayerOutcome::Pending { .. } => self.pending += 1,
        }
    }
}

/// The result of a conformance run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceReport {
    /// Which corpus was exercised.
    pub corpus: Corpus,
    /// All per-(spec, layer) records, in document order × layer order.
    pub records: Vec<LayerRecord>,
    /// Roll-up counts.
    pub summary: ReportSummary,
}

/// Drive the `LayerCheck` registry across `corpus` at `layers` using
/// `registry` for suppression lookups. Unknown layers are filtered silently.
#[must_use]
pub fn run_conformance(
    corpus: Corpus,
    layers: &[ConformanceLayer],
    registry: &DeviationRegistry,
) -> ConformanceReport {
    let entries = match corpus {
        Corpus::Curated => match curated_entries() {
            Ok(e) => e,
            Err(e) => {
                return ConformanceReport {
                    corpus,
                    records: vec![LayerRecord {
                        spec_name: "<curated-corpus-load>".to_string(),
                        layer: ConformanceLayer::AstRoundTrip,
                        outcome: LayerOutcome::Fail {
                            details: format!("curated corpus load failed: {e}"),
                        },
                    }],
                    summary: ReportSummary {
                        failed: 1,
                        ..Default::default()
                    },
                };
            }
        },
        Corpus::Observed => observed_entries(),
    };
    let checks = default_layer_checks();
    let mut records = Vec::new();
    let mut summary = ReportSummary::default();
    for entry in &entries {
        let spec_source = match fs::read_to_string(&entry.source_path) {
            Ok(s) => s,
            Err(e) => {
                for layer in layers {
                    let outcome = LayerOutcome::Fail {
                        details: format!("read {:?}: {e}", entry.source_path),
                    };
                    summary.bump(&outcome);
                    records.push(LayerRecord {
                        spec_name: entry.name.clone(),
                        layer: *layer,
                        outcome,
                    });
                }
                continue;
            }
        };
        let spec = match parse_spec(&spec_source, Format::Yaml) {
            Ok(o) => o.spec,
            Err(e) => {
                for layer in layers {
                    let outcome = LayerOutcome::Fail {
                        details: format!("parse {:?}: {e}", entry.source_path),
                    };
                    summary.bump(&outcome);
                    records.push(LayerRecord {
                        spec_name: entry.name.clone(),
                        layer: *layer,
                        outcome,
                    });
                }
                continue;
            }
        };
        for layer in layers {
            let Some(check) = checks.iter().find(|c| c.layer() == *layer) else {
                continue;
            };
            let outcome = run_check(check.as_ref(), &spec, entry, registry);
            summary.bump(&outcome);
            records.push(LayerRecord {
                spec_name: entry.name.clone(),
                layer: *layer,
                outcome,
            });
        }
    }
    ConformanceReport {
        corpus,
        records,
        summary,
    }
}

fn run_check(
    check: &dyn LayerCheck,
    spec: &brightfield_spec::Spec,
    fixture: &CorpusEntry,
    registry: &DeviationRegistry,
) -> LayerOutcome {
    check.run(spec, fixture, registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dfconf_report_summary_bumps_each_outcome() {
        let mut s = ReportSummary::default();
        s.bump(&LayerOutcome::Pass);
        s.bump(&LayerOutcome::Fail {
            details: "x".to_string(),
        });
        s.bump(&LayerOutcome::Suppressed {
            deviation_id: "DEV-0001".to_string(),
        });
        s.bump(&LayerOutcome::Pending {
            reason: "pending",
        });
        assert_eq!(s.passed, 1);
        assert_eq!(s.failed, 1);
        assert_eq!(s.suppressed, 1);
        assert_eq!(s.pending, 1);
    }

    #[test]
    fn dfconf_run_conformance_layer_1_curated_all_pass() {
        let reg = DeviationRegistry::default();
        let report = run_conformance(
            Corpus::Curated,
            &[ConformanceLayer::AstRoundTrip],
            &reg,
        );
        assert_eq!(report.summary.failed, 0);
        assert_eq!(report.summary.pending, 0);
        assert_eq!(report.summary.suppressed, 0);
        assert!(report.summary.passed >= 10);
        for rec in &report.records {
            assert_eq!(rec.outcome, LayerOutcome::Pass, "{} failed", rec.spec_name);
        }
    }

    #[test]
    fn dfconf_run_conformance_mixed_layers() {
        let reg = DeviationRegistry::default();
        let report = run_conformance(
            Corpus::Observed,
            &[ConformanceLayer::AstRoundTrip, ConformanceLayer::SqlEquivalence],
            &reg,
        );
        assert!(report.summary.failed == 0);
        // Every spec × 2 layers → one Pending for layer-2 per spec.
        assert!(report.summary.pending > 0);
        for rec in &report.records {
            if rec.layer == ConformanceLayer::SqlEquivalence {
                // Observed corpus entries have no .layer2.expected.sql fixture,
                // so the active SqlEquivalenceCheck returns "no expected SQL fixture"
                assert_eq!(
                    rec.outcome,
                    LayerOutcome::Pending {
                        reason: "no expected SQL fixture"
                    }
                );
            }
        }
    }
}
