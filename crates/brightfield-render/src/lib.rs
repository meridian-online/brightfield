//! Headless chart rendering: Mosaic spec + Arrow data -> Vello scene.
//!
//! This crate sits downstream of `brightfield-spec` (parsing) and consumes
//! Arrow `RecordBatch` data from the execution engine. It produces a
//! `vello::Scene` ready for composition — no GPUI dependency.
//!
//! **Dependency chain:** `brightfield-spec` -> `brightfield-render`.
//! Neither `gpui` nor `brightfield-engine` is a dependency.

pub mod asset_scene;
pub mod axis;
pub mod canvas_host;
pub mod channel;
pub mod contour;
pub mod grid;
pub mod ink;
pub mod inset;
pub mod kde;
pub mod layout;
pub mod legend;
pub mod mark;
pub mod nearest;
pub mod scale;
pub mod scene;
pub mod text;
pub mod title;
pub mod tooltip;
pub mod transition;
pub mod vello_renderer;

// Re-exports for downstream consumers.
pub use canvas_host::{CanvasHost, ChartSurface, OverlayPainter};
pub use channel::ChannelMap;
pub use vello_renderer::VelloRenderer;
pub use layout::ChartLayout;
pub use mark::{HighlightState, MarkRenderer};
pub use nearest::{find_nearest, NearestHit, NearestMode};
pub use scale::{infer_scales_multi, Scale, ScaleSet, ViewExtent};
pub use scene::{build_chart_scene, build_multi_mark_scene};
pub use title::{grow_margins, resolve_titles, ResolvedTitles};
pub use tooltip::TooltipContent;
pub use transition::{Transition, TransitionState};
