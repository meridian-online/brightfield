//! GPUI application shell — Vello chart rendering + interaction in a native window.
//!
//! This crate wraps `brightfield-render`'s headless chart scene into a GPUI
//! element with texture handoff (GPU readback) and interaction state
//! management (brush/hover overlay).
//!
//! **Dependency chain:** `brightfield-render` -> `brightfield-ui`.
//! GPUI is a dependency of this crate only, not of `brightfield-render`.
//!
//! ## Architecture
//!
//! - **ChartState** — reactive state wrapped in `gpui::Entity`. Owns scene,
//!   interaction, navigation, transition, dimensions, and VelloRenderer ref.
//! - **ChartElement** — stateless rendering shell. Borrows from ChartState
//!   for one paint cycle. Implements `gpui::Element`.
//! - **ChartView** — GPUI `Render` component. Owns `Entity<ChartState>`.
//!   Public API for consumers.
//! - **VelloRenderer** — wgpu-backed Vello renderer. Arc-shared, dedicated
//!   device (not shared with GPUI).
//! - **ChartLayout** — coordinate mapping pipeline for mouse events.

pub mod brush;
pub mod chart_element;
pub mod chart_layout;
pub mod chart_state;
pub mod chart_view;
pub mod crossfilter;
pub mod interaction;
pub mod legend_element;
pub mod legend_scene;
pub mod slider;
pub mod slider_element;
mod theme_bridge;
pub mod vello_renderer;
pub mod workspace;
pub mod workspace_actions;

pub use brush::{brush_rect_to_predicate, BrushKind, ChannelColumns};
pub use chart_element::ChartElement;
pub use chart_layout::ChartLayout;
pub use chart_state::ChartState;
pub use chart_view::{ChartView, PlacedChart, PlacedSlider};
pub use crossfilter::{CrossfilterCoordinator, LegendSelectBinding, LivePlot, MarkInput};
pub use interaction::InteractionState;
pub use legend_element::{swatch_hit_category, LegendElement, PlacedLegend};
pub use legend_scene::build_legend_scene;
pub use slider::{commit_slider_release, ParamDispatcher, SliderBinding, SliderState};
pub use slider_element::{SliderElement, SliderWidget};
pub use vello_renderer::VelloRenderer;
pub use workspace::{
    framed_window_size, resolve_title, PresentationMode, CONTENT_PADDING, HEADER_HEIGHT,
};
pub use workspace_actions::{workspace_key_bindings, TogglePresentation, WORKSPACE_KEY_CONTEXT};
