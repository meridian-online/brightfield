//! Headless chart rendering: Mosaic spec + Arrow data -> Vello scene.
//!
//! This crate sits downstream of `brightfield-spec` (parsing) and consumes
//! Arrow `RecordBatch` data from the execution engine. It produces a
//! `vello::Scene` ready for composition — no UI-framework dependency.
//!
//! **Dependency chain:** `brightfield-spec` -> `brightfield-render`.
//! No UI framework and no `brightfield-engine` in the dependency list.

pub mod asset_scene;
pub mod axis;
pub mod canvas_host;
pub mod channel;
pub mod contour;
pub mod frame_ink;
pub mod grid;
pub mod ink;
pub mod inset;
pub mod kde;
pub mod layout;
pub mod legend;
pub mod mark;
pub mod nearest;
pub mod sample_notice;
pub mod scale;
pub mod scene;
pub mod selection;
pub mod text;
pub mod title;
pub mod tooltip;
pub mod transition;
pub mod vello_renderer;

// Re-exports for downstream consumers.
pub use canvas_host::{CanvasHost, ChartSurface, OverlayPainter};
pub use channel::ChannelMap;
pub use frame_ink::FrameInk;
pub use layout::ChartLayout;
pub use mark::{HighlightState, MarkRenderer};
pub use nearest::{find_nearest, NearestHit, NearestMode};
pub use scale::{infer_scales_multi, Scale, ScaleSet, ViewExtent};
pub use scene::{build_chart_scene, build_multi_mark_scene};
pub use selection::{render_committed_selection, CommittedSelection, Selected};
pub use title::{grow_margins, resolve_titles, ResolvedTitles};
pub use tooltip::TooltipContent;
pub use transition::{Transition, TransitionState};
pub use vello_renderer::VelloRenderer;
