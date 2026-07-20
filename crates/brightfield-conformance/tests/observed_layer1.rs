//! Layer-1 corpus totality gate.
//!
//! Exercises the entire observed corpus through the conformance runner's
//! `LayerCheck` plumbing. When layers 2–4 activate, the same dispatch path
//! scales — the runner is the single invariant, not the test.

use brightfield_conformance::{
    observed_entries, run_conformance, ConformanceLayer, Corpus, DeviationRegistry, LayerOutcome,
};

#[test]
fn dfconf_observed_layer_1_all_pass() {
    let registry = DeviationRegistry::default();
    let report = run_conformance(
        Corpus::Observed,
        &[ConformanceLayer::AstRoundTrip],
        &registry,
    );
    let expected = observed_entries().len();
    assert!(expected >= 1, "observed corpus must contain ≥1 spec");
    assert_eq!(
        report.summary.passed, expected,
        "every observed spec must pass layer 1; summary={:?}",
        report.summary
    );
    assert_eq!(
        report.summary.failed, 0,
        "no observed spec may fail layer 1; summary={:?}",
        report.summary
    );
    assert_eq!(
        report.summary.pending, 0,
        "layer 1 has no pending state; summary={:?}",
        report.summary
    );
    assert_eq!(
        report.summary.suppressed, 0,
        "observed defaults to no suppression; summary={:?}",
        report.summary
    );
    for rec in &report.records {
        assert_eq!(
            rec.outcome,
            LayerOutcome::Pass,
            "spec {} failed layer 1: {:?}",
            rec.spec_name,
            rec.outcome
        );
    }
}
