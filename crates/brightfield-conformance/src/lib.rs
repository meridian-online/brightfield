//! Mosaic web → brightfield portability contract.
//!
//! This crate defines and enforces the portability contract between Mosaic
//! web specs and brightfield's rendering. The contract is **layered**, with
//! each layer an independently-diagnostic pass/fail gate:
//!
//! 1. **AST round-trip** — `parse → serialise → parse` is idempotent.
//! 2. **SQL equivalence** — same query text (semantically), same result set.
//! 3. **Visual-encoding equivalence** — mark + scale + channel structure.
//! 4. **Interaction equivalence** — scripted event → same selection state.
//!
//! Layers 1 and 2 are live gates. Layers 3 and 4 return
//! [`LayerOutcome::Pending`] for want of an **oracle** — nothing yet diffs a
//! rendered brightfield scene, or a scripted interaction's selection state,
//! against Mosaic web's. Both the renderer and the scriptable
//! Interaction/Coordinator seam shipped long ago; the missing piece is the
//! comparison, not the capability. Where the registry names a deviation for a
//! (spec, layer) pair, the check still runs: coverage is necessary and *not*
//! sufficient. A covered pair that genuinely fails reports
//! [`LayerOutcome::Suppressed`] against the named record; one that has quietly
//! started passing reports a failure, so a stale deviation cannot hide an
//! improvement.
//!
//! What a spec load must SAY is [`LoadDiagnostics`]: the blocking preflight
//! entries plus every warning the parse and the analysis produced, in one
//! value a caller can put in front of a person.
//!
//! See `README.md` for the user-facing API walkthrough.

pub mod corpus;
pub mod deviations;
pub mod diagnostics;
pub mod expectations;
pub mod identity;
pub mod layer;
pub mod report;
pub mod support;

pub use crate::corpus::{curated_entries, observed_entries, Corpus, CorpusEntry, OBSERVED_CORPUS};
pub use crate::deviations::{load_deviations, Deviation, DeviationRegistry, RegistryError};
pub use crate::diagnostics::{Diagnostic, DiagnosticSeverity, LoadDiagnostics};
pub use crate::expectations::{
    ExpectationError, Layer1Expectation, LayerExpectations, LayerNExpectation,
};
pub use crate::identity::{ComponentIdentity, Surface};
pub use crate::layer::{
    AstRoundTripCheck, ConformanceLayer, EncodingEquivalenceCheck, InteractionEquivalenceCheck,
    LayerCheck, LayerOutcome, SqlEquivalenceCheck,
};
pub use crate::report::{
    run_conformance, suppressing_deviation, ConformanceReport, LayerCells, LayerRecord,
    ReportSummary,
};
pub use crate::support::{preflight, SupportEntry, SupportReport};
