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
//! - **ChartState** (`chart_state`) — one plot's mutable chart state, generic
//!   over the host texture handle. The gpui shell wraps it in `gpui::Entity`
//!   (`GpuiChartState`); the state itself names no host type. Owns scene,
//!   interaction, navigation, transition, dimensions, and VelloRenderer ref.
//! - **ReactiveHandle** (`reactive`) — the host-side reactive-cell seam the
//!   crossfilter coordinator addresses plot state through; the gpui
//!   realisation (`Entity<GpuiChartState>` + `App`) lives in `gpui_canvas`.
//! - **CanvasHost / ChartSurface / OverlayPainter** (`canvas_host`) — the
//!   framework-free render/host boundary. The chart's paint logic
//!   (`chart_element`) is expressed against these traits and names no gpui type.
//! - **chart_element** — the framework-free chart paint logic (present + overlay
//!   + cursor + input routing), driven each frame through a `ChartSurface`.
//! - **GpuiCanvasHost / GpuiChartSurface** (`gpui_canvas`) — the gpui host. The
//!   `Element` shell (layout/prepaint/paint + mouse listeners) and the present
//!   path (scene → RenderImage). The sole place gpui element/paint types live.
//! - **ChartView** — GPUI `Render` component. Owns `Entity<ChartState>`.
//!   Public API for consumers.
//! - **VelloRenderer** — wgpu-backed Vello renderer. Arc-shared, dedicated
//!   device (not shared with GPUI).
//! - **ChartLayout** — coordinate mapping pipeline for mouse events.

pub mod brush;
// The framework-free render/host seam now lives in `brightfield-render` (so a
// gpui-free egui host can implement it). Re-exported here so this crate's
// `crate::canvas_host::…` paths and downstream `brightfield_ui::CanvasHost`
// keep resolving unchanged.
pub use brightfield_render::canvas_host;
pub mod chart_element;
pub mod chart_layout;
pub mod chart_state;
pub mod chart_view;
pub mod crossfilter;
pub mod gpui_canvas;
pub mod interaction;
pub mod legend_element;
pub mod legend_scene;
pub mod menu;
pub mod menu_element;
pub mod reactive;
pub mod slider;
pub mod slider_element;
mod theme_bridge;
// Moved to `brightfield-render` alongside the seam it serves; re-exported so
// `crate::vello_renderer::VelloRenderer` and `brightfield_ui::VelloRenderer`
// keep resolving.
pub use brightfield_render::vello_renderer;
pub mod workspace;
pub mod workspace_actions;

pub use brush::{
    brush_rect_to_predicate, brush_rect_to_structured, point_to_structured, scalar_value_from,
    BrushKind, ChannelColumns,
};
pub use canvas_host::{CanvasHost, ChartSurface, OverlayPainter};
pub use chart_layout::ChartLayout;
pub use chart_state::ChartState;
pub use chart_view::{ChartView, PlacedChart, PlacedMenu, PlacedSlider};
pub use crossfilter::{
    clause_meta_for_scale, structured_brush_predicate, CrossfilterCoordinator, LegendSelectBinding,
    LivePlot, MarkInput,
};
pub use gpui_canvas::{
    GpuiCanvasHost, GpuiChartState, GpuiChartSurface, GpuiCrossfilter, GpuiStateHandle, GpuiSurface,
};
pub use interaction::InteractionState;
pub use legend_element::{swatch_hit_category, LegendElement, PlacedLegend};
pub use legend_scene::build_legend_scene;
pub use menu::{
    checkbox_checked, checkbox_toggle_index, commit_menu_release, option_label, DerivedOptions,
    MenuBinding, MenuState, MenuStyle,
};
pub use menu_element::MenuWidget;
pub use reactive::ReactiveHandle;
pub use slider::{commit_slider_release, ParamDispatcher, SliderBinding, SliderState};
pub use slider_element::{SliderElement, SliderWidget};
pub use vello_renderer::VelloRenderer;
pub use workspace::{
    framed_window_size, resolve_title, PresentationMode, CONTENT_PADDING, HEADER_HEIGHT,
};
pub use workspace_actions::{workspace_key_bindings, TogglePresentation, WORKSPACE_KEY_CONTEXT};
