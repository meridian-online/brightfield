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
pub mod interaction;
pub mod vello_renderer;

pub use brush::{brush_rect_to_predicate, BrushKind, ChannelColumns};
pub use chart_element::ChartElement;
pub use chart_layout::ChartLayout;
pub use chart_state::ChartState;
pub use chart_view::ChartView;
pub use interaction::InteractionState;
pub use vello_renderer::VelloRenderer;
