//! Headless chart rendering: Mosaic spec + Arrow data -> Vello scene.
//!
//! This crate sits downstream of `brightfield-spec` (parsing) and consumes
//! Arrow `RecordBatch` data from the execution engine. It produces a
//! `vello::Scene` ready for composition — no GPUI dependency.
//!
//! **Dependency chain:** `brightfield-spec` -> `brightfield-render`.
//! Neither `gpui` nor `brightfield-engine` is a dependency.

pub mod axis;
pub mod channel;
pub mod grid;
pub mod layout;
pub mod legend;
pub mod mark;
pub mod scale;
pub mod scene;

// Re-exports for downstream consumers.
pub use channel::ChannelMap;
pub use layout::ChartLayout;
pub use mark::MarkRenderer;
pub use scale::{Scale, ScaleSet, ViewExtent};
pub use scene::build_chart_scene;
