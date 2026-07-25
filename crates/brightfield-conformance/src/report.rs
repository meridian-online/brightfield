//! Conformance runner.
//!
//! `run_conformance` is the library entry point both `cargo test` and the
//! `conformance` binary dispatch through. It enumerates the requested
//! corpus, parses each spec, and settles each requested layer against it —
//! surfacing per-(spec, layer) [`LayerOutcome`]s, a per-layer breakdown, and
//! a roll-up [`ReportSummary`].
//!
//! Two things settle a cell, in this order:
//!
//! 1. **The deviation registry.** If some entry in `deviations.yaml` names
//!    this spec's filename in `affected_specs` AND this layer in
//!    `conformance_layers_suppressed`, the cell is
//!    [`LayerOutcome::Suppressed`] and the check does not run. Coverage is
//!    the whole rule: a pair the registry does not cover can never come back
//!    Suppressed, so a suppression is always traceable to a written-down,
//!    reviewed deviation record rather than to a check's private judgement.
//!    Every check used to bind the registry as `_registry`, which made
//!    `Suppressed` unreachable and the registry decorative.
//! 2. **The declared expectation.** Each curated entry ships a
//!    `<name>.expected.yaml`. A cell whose settled outcome differs from what
//!    that file declares is turned into a [`LayerOutcome::Fail`] naming both.
//!    That is what makes the expectation an ASSERTION: a layer regressing
//!    Pass → Pending, or a suppressed layer quietly starting to pass while
//!    its deviation record still claims otherwise, both redden the run.
//!
//! Unknown layers (outside 1..=4) are filtered silently.

use std::fs;

use brightfield_spec::{parse_spec, Format};

use crate::corpus::{curated_entries, observed_entries, Corpus, CorpusEntry};
use crate::deviations::{Deviation, DeviationRegistry};
use crate::expectations::{Layer1Expectation, LayerNExpectation};
use crate::layer::{default_layer_checks, ConformanceLayer, LayerCheck, LayerOutcome};

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

    /// Cells counted, whatever they came back as.
    #[must_use]
    pub fn cells(&self) -> usize {
        self.passed + self.failed + self.suppressed + self.pending
    }
}

/// One layer's slice of a run: how many cells it had, and how they landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerCells {
    /// The layer.
    pub layer: ConformanceLayer,
    /// Its counts.
    pub summary: ReportSummary,
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

impl ConformanceReport {
    /// Cell counts broken down per layer, in layer order 1..=4, skipping
    /// layers this run did not exercise.
    ///
    /// The roll-up alone cannot answer the only question worth asking of a
    /// layered contract — *which* layer is carrying the greens — so a run
    /// that reports 20/40 without saying where is not a report.
    #[must_use]
    pub fn per_layer(&self) -> Vec<LayerCells> {
        let mut out = Vec::new();
        for layer in ConformanceLayer::all() {
            let mut summary = ReportSummary::default();
            for rec in self.records.iter().filter(|r| r.layer == layer) {
                summary.bump(&rec.outcome);
            }
            if summary.cells() > 0 {
                out.push(LayerCells { layer, summary });
            }
        }
        out
    }

    /// `true` iff no cell failed. What a caller gates on.
    #[must_use]
    pub fn is_green(&self) -> bool {
        self.summary.failed == 0
    }
}

/// The deviation record, if any, that accepts a divergence at
/// (`spec_file`, `layer`).
///
/// Coverage is name-level on `affected_specs` — the spec's FILENAME, which is
/// the same accounting handle the registry-integrity gate and the curated
/// preflight gate both key on. First match in registry order wins; the loader
/// already rejects duplicate ids, so "first" is stable.
#[must_use]
pub fn suppressing_deviation<'a>(
    registry: &'a DeviationRegistry,
    spec_file: &str,
    layer: ConformanceLayer,
) -> Option<&'a Deviation> {
    registry.iter().find(|d| {
        d.conformance_layers_suppressed.contains(&layer)
            && d.affected_specs.iter().any(|s| s == spec_file)
    })
}

/// Turn a settled outcome into the outcome the run REPORTS, by holding it
/// against what the corpus entry declared it would be.
///
/// A match passes the outcome through untouched. A mismatch becomes a
/// `Fail` naming both sides, because a declared expectation that cannot fail
/// a run is a comment.
fn against_expectation(
    outcome: LayerOutcome,
    layer: ConformanceLayer,
    entry: &CorpusEntry,
) -> LayerOutcome {
    let (matches, declared) = match layer {
        ConformanceLayer::AstRoundTrip => match &entry.expectations.layer_1 {
            Layer1Expectation::Pass => (outcome == LayerOutcome::Pass, "pass".to_string()),
            Layer1Expectation::Suppressed(id) => (
                matches!(&outcome, LayerOutcome::Suppressed { deviation_id } if deviation_id == id),
                format!("suppressed: {id}"),
            ),
        },
        other => {
            let declared = match other {
                ConformanceLayer::SqlEquivalence => &entry.expectations.layer_2,
                ConformanceLayer::EncodingEquivalence => &entry.expectations.layer_3,
                _ => &entry.expectations.layer_4,
            };
            match declared {
                LayerNExpectation::Pass => (outcome == LayerOutcome::Pass, "pass".to_string()),
                LayerNExpectation::Pending => (
                    matches!(outcome, LayerOutcome::Pending { .. }),
                    "pending".to_string(),
                ),
                LayerNExpectation::Suppressed(id) => (
                    matches!(&outcome, LayerOutcome::Suppressed { deviation_id } if deviation_id == id),
                    format!("suppressed: {id}"),
                ),
            }
        }
    };
    if matches {
        return outcome;
    }
    LayerOutcome::Fail {
        details: format!(
            "expectation mismatch at {}: {} declares `{declared}`, run observed {}",
            layer.display_name(),
            entry.name,
            observed_label(&outcome),
        ),
    }
}

/// A short label for what a run observed, for the mismatch message.
fn observed_label(outcome: &LayerOutcome) -> String {
    match outcome {
        LayerOutcome::Pass => "pass".to_string(),
        LayerOutcome::Fail { details } => format!("fail ({details})"),
        LayerOutcome::Suppressed { deviation_id } => format!("suppressed: {deviation_id}"),
        LayerOutcome::Pending { reason } => format!("pending ({reason})"),
    }
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
        let spec_file = entry
            .source_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        for layer in layers {
            let Some(check) = checks.iter().find(|c| c.layer() == *layer) else {
                continue;
            };
            // Registry coverage settles the cell before any check runs — see
            // the module docs. A pair the registry does not name can never
            // come back Suppressed.
            let settled = match suppressing_deviation(registry, &spec_file, *layer) {
                Some(dev) => LayerOutcome::Suppressed {
                    deviation_id: dev.id.clone(),
                },
                None => run_check(check.as_ref(), &spec, entry, registry),
            };
            let outcome = against_expectation(settled, *layer, entry);
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
    use crate::deviations::load_deviations;
    use crate::expectations::LayerExpectations;

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
        s.bump(&LayerOutcome::Pending { reason: "pending" });
        assert_eq!(s.passed, 1);
        assert_eq!(s.failed, 1);
        assert_eq!(s.suppressed, 1);
        assert_eq!(s.pending, 1);
    }

    #[test]
    fn dfconf_run_conformance_layer_1_curated_all_pass() {
        let reg = DeviationRegistry::default();
        let report = run_conformance(Corpus::Curated, &[ConformanceLayer::AstRoundTrip], &reg);
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
            &[
                ConformanceLayer::AstRoundTrip,
                ConformanceLayer::SqlEquivalence,
            ],
            &reg,
        );
        assert!(report.summary.failed == 0);
        // Every spec × 2 layers → one Pending for layer-2 per spec.
        assert!(report.summary.pending > 0);
        for rec in &report.records {
            if rec.layer == ConformanceLayer::SqlEquivalence {
                // Observed corpus entries have no `.layer2.expected.sql`
                // sibling, and some declare no data sources at all; either way
                // layer 2 makes no claim about them.
                assert!(
                    matches!(rec.outcome, LayerOutcome::Pending { .. }),
                    "{} layer 2: {:?}",
                    rec.spec_name,
                    rec.outcome
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Suppression is the registry's decision, and only the registry's.
    // -----------------------------------------------------------------------

    fn registry_with(affected: &[&str], layers: &[ConformanceLayer]) -> DeviationRegistry {
        let mut entries = indexmap::IndexMap::new();
        entries.insert(
            "DEV-0009".to_string(),
            Deviation {
                id: "DEV-0009".to_string(),
                surface: "test".to_string(),
                mosaic_behaviour: "a".to_string(),
                brightfield_behaviour: "b".to_string(),
                rationale: "c".to_string(),
                affected_specs: affected.iter().map(|s| (*s).to_string()).collect(),
                conformance_layers_suppressed: layers.to_vec(),
            },
        );
        DeviationRegistry { entries }
    }

    #[test]
    fn dfconf_suppression_needs_both_the_spec_and_the_layer() {
        let reg = registry_with(&["line.yaml"], &[ConformanceLayer::EncodingEquivalence]);
        assert_eq!(
            suppressing_deviation(&reg, "line.yaml", ConformanceLayer::EncodingEquivalence)
                .map(|d| d.id.as_str()),
            Some("DEV-0009"),
            "named spec at a named layer is covered"
        );
        assert!(
            suppressing_deviation(&reg, "line.yaml", ConformanceLayer::SqlEquivalence).is_none(),
            "the same spec at an unnamed layer is NOT covered"
        );
        assert!(
            suppressing_deviation(&reg, "table.yaml", ConformanceLayer::EncodingEquivalence)
                .is_none(),
            "an unnamed spec at the same layer is NOT covered"
        );
        assert!(
            suppressing_deviation(
                &DeviationRegistry::default(),
                "line.yaml",
                ConformanceLayer::EncodingEquivalence
            )
            .is_none(),
            "an empty registry covers nothing"
        );
    }

    /// The whole point of the fix: with a registry that covers a pair, that
    /// pair reports `Suppressed` — and every pair it does not cover reports
    /// something else. Before this, every check bound the registry as
    /// `_registry` and `Suppressed` was unreachable in the shipped runner.
    #[test]
    fn dfconf_suppressed_is_produced_for_covered_pairs_and_no_others() {
        // The real registry covers all ten curated specs at layers 3 and 4.
        let reg = load_deviations(
            &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../deviations.yaml"),
        )
        .expect("load registry");
        let report = run_conformance(Corpus::Curated, &ConformanceLayer::all(), &reg);
        assert_eq!(report.summary.failed, 0, "records: {:?}", report.records);

        for rec in &report.records {
            let covered = matches!(
                rec.layer,
                ConformanceLayer::EncodingEquivalence | ConformanceLayer::InteractionEquivalence
            );
            let suppressed = matches!(rec.outcome, LayerOutcome::Suppressed { .. });
            assert_eq!(
                suppressed,
                covered,
                "{} at {} — suppression must track registry coverage exactly, got {:?}",
                rec.spec_name,
                rec.layer.display_name(),
                rec.outcome
            );
        }
        assert_eq!(
            report.summary.suppressed, 20,
            "ten curated specs × the two layers the registry covers"
        );
    }

    // -----------------------------------------------------------------------
    // The declared expectation is an assertion.
    // -----------------------------------------------------------------------

    /// A curated entry declaring `pass` where the run observes something else
    /// fails the run. Built by pointing an entry at a spec whose layer-1
    /// expectation is `suppressed` while nothing suppresses it.
    #[test]
    fn dfconf_an_outcome_that_differs_from_the_expectation_fails() {
        let entry = CorpusEntry {
            name: "synthetic".to_string(),
            source_path: std::path::PathBuf::from("synthetic.yaml"),
            expectations: LayerExpectations {
                layer_1: Layer1Expectation::Suppressed("DEV-0009".to_string()),
                layer_2: LayerNExpectation::Pending,
                layer_3: LayerNExpectation::Pending,
                layer_4: LayerNExpectation::Pending,
            },
        };
        let outcome =
            against_expectation(LayerOutcome::Pass, ConformanceLayer::AstRoundTrip, &entry);
        let LayerOutcome::Fail { details } = outcome else {
            panic!("a pass where suppression was declared must fail: {outcome:?}");
        };
        assert!(
            details.contains("DEV-0009") && details.contains("pass"),
            "the message names both sides: {details}"
        );
    }

    /// …and a matching outcome passes straight through unchanged.
    #[test]
    fn dfconf_a_matching_expectation_passes_the_outcome_through() {
        let entry = CorpusEntry {
            name: "synthetic".to_string(),
            source_path: std::path::PathBuf::from("synthetic.yaml"),
            expectations: LayerExpectations {
                layer_1: Layer1Expectation::Pass,
                layer_2: LayerNExpectation::Pending,
                layer_3: LayerNExpectation::Suppressed("DEV-0001".to_string()),
                layer_4: LayerNExpectation::Pending,
            },
        };
        assert_eq!(
            against_expectation(LayerOutcome::Pass, ConformanceLayer::AstRoundTrip, &entry),
            LayerOutcome::Pass
        );
        assert_eq!(
            against_expectation(
                LayerOutcome::Suppressed {
                    deviation_id: "DEV-0001".to_string()
                },
                ConformanceLayer::EncodingEquivalence,
                &entry
            ),
            LayerOutcome::Suppressed {
                deviation_id: "DEV-0001".to_string()
            }
        );
    }

    /// A layer regressing Pass → Pending reddens the run. This is the
    /// concrete regression the assertion exists to catch.
    #[test]
    fn dfconf_a_pass_regressing_to_pending_fails() {
        let entry = CorpusEntry {
            name: "synthetic".to_string(),
            source_path: std::path::PathBuf::from("synthetic.yaml"),
            expectations: LayerExpectations {
                layer_1: Layer1Expectation::Pass,
                layer_2: LayerNExpectation::Pass,
                layer_3: LayerNExpectation::Pending,
                layer_4: LayerNExpectation::Pending,
            },
        };
        let outcome = against_expectation(
            LayerOutcome::Pending {
                reason: "no expected SQL fixture",
            },
            ConformanceLayer::SqlEquivalence,
            &entry,
        );
        assert!(
            matches!(outcome, LayerOutcome::Fail { .. }),
            "Pass → Pending must fail: {outcome:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Per-layer cell counts.
    // -----------------------------------------------------------------------

    #[test]
    fn dfconf_per_layer_counts_partition_the_run() {
        let reg = load_deviations(
            &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../deviations.yaml"),
        )
        .expect("load registry");
        let report = run_conformance(Corpus::Curated, &ConformanceLayer::all(), &reg);
        let per_layer = report.per_layer();
        assert_eq!(per_layer.len(), 4, "all four layers were exercised");
        let total: usize = per_layer.iter().map(|c| c.summary.cells()).sum();
        assert_eq!(
            total,
            report.summary.cells(),
            "the per-layer counts must partition the run, not approximate it"
        );
        for cells in &per_layer {
            assert_eq!(
                cells.summary.cells(),
                10,
                "ten curated specs at {}",
                cells.layer.display_name()
            );
        }
    }
}
