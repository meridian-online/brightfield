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
//! V1 ships the *machinery* — preflight [`SupportReport`], curated and
//! observed corpora, deviation registry, layer-1 gate — behind stable trait
//! seams. Layers 2–4 return [`LayerOutcome::Pending`] until their downstream
//! infra (SQL emitter, renderer, event pump) lands in a later card. The
//! scaffolding is deliberate: an honest `Pending` today becomes a failing
//! test the moment its concrete impl goes in, which is what upgrades this
//! from "spec says it works" to "tests prove it works".
//!
//! See `README.md` for the user-facing API walkthrough.

pub mod corpus;
pub mod deviations;
pub mod expectations;
pub mod identity;
pub mod layer;
pub mod report;
pub mod support;

pub use crate::corpus::{curated_entries, observed_entries, Corpus, CorpusEntry, OBSERVED_CORPUS};
pub use crate::deviations::{load_deviations, Deviation, DeviationRegistry, RegistryError};
pub use crate::expectations::{
    ExpectationError, Layer1Expectation, LayerExpectations, LayerNExpectation,
};
pub use crate::identity::{ComponentIdentity, Surface};
pub use crate::layer::{
    AstRoundTripCheck, ConformanceLayer, EncodingEquivalenceCheck, InteractionEquivalenceCheck,
    LayerCheck, LayerOutcome, SqlEquivalenceCheck,
};
pub use crate::report::{run_conformance, ConformanceReport, LayerRecord, ReportSummary};
pub use crate::support::{preflight, SupportEntry, SupportReport};
