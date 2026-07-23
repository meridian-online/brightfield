//! Framework-free chart interaction logic — brush/click/widget commit paths,
//! per-plot chart state, and the live cross-filter coordinator.
//!
//! This crate holds the interaction half of the chart pipeline as plain data
//! and state machines: no UI framework is named anywhere in it. A windowed
//! host addresses plot state through the [`ReactiveHandle`] seam and drives
//! the commit helpers from its own gesture handlers; a headless test drives
//! the same paths directly.
//!
//! **Dependency chain:** `brightfield-render` -> this crate. The render/host
//! seam (`CanvasHost` / `ChartSurface` / `OverlayPainter`) and the Vello
//! renderer live in `brightfield-render` and are re-exported here so
//! `brightfield_ui::CanvasHost` keeps resolving.
//!
//! ## Architecture
//!
//! - **ChartState** (`chart_state`) — one plot's mutable chart state, generic
//!   over the host texture handle; the state itself names no host type. Owns
//!   scene, interaction, navigation, transition, dimensions, and the
//!   VelloRenderer ref.
//! - **ReactiveHandle** (`reactive`) — the host-side reactive-cell seam the
//!   crossfilter coordinator addresses plot state through. Each host realises
//!   it over its own cell type.
//! - **brush** — rect/point → predicate producers (structured and string
//!   forms), the [`SelectionDispatcher`](brush::SelectionDispatcher) trait,
//!   and the release/click commit path ([`brush::commit_brush_release_multi`],
//!   [`brush::commit_click_multi`]).
//! - **interaction** — the brush-region grammar: grab classification,
//!   move/resize transforms, and the release re-dispatch resolver.
//! - **menu / slider** — the framework-free widget models (`input: menu`,
//!   `input: slider`) and their commit-on-release helpers.
//! - **crossfilter** — the live cross-filter coordinator: keeps the engine
//!   session and per-plot render metadata alive so a committed gesture
//!   re-executes subscriber marks and swaps their scenes in place.
//! - **workspace** — the shell-geometry model (presentation mode, the framed
//!   window formula, title resolution).
//! - **ChartLayout** (`chart_layout`) — coordinate mapping pipeline for
//!   pointer events.

pub mod brush;
// The framework-free render/host seam lives in `brightfield-render` (so any
// host can implement it without this crate). Re-exported so this crate's
// `crate::canvas_host::…` paths and downstream `brightfield_ui::CanvasHost`
// keep resolving unchanged.
pub use brightfield_render::canvas_host;
pub mod chart_layout;
pub mod chart_state;
pub mod crossfilter;
pub mod interaction;
pub mod menu;
pub mod reactive;
pub mod slider;
// Moved to `brightfield-render` alongside the seam it serves; re-exported so
// `crate::vello_renderer::VelloRenderer` and `brightfield_ui::VelloRenderer`
// keep resolving.
pub use brightfield_render::vello_renderer;
pub mod workspace;

pub use brush::{
    brush_rect_to_predicate, brush_rect_to_structured, commit_brush_clear, commit_brush_release,
    commit_brush_release_multi, commit_click_multi, point_to_structured, scalar_value_from,
    BrushBinding, BrushKind, ChannelColumns, ZERO_AREA_EPSILON,
};
pub use canvas_host::{CanvasHost, ChartSurface, OverlayPainter};
pub use chart_layout::ChartLayout;
pub use chart_state::ChartState;
pub use crossfilter::{
    clause_meta_for_scale, structured_brush_predicate, CrossfilterCoordinator, LegendSelectBinding,
    LivePlot, MarkInput,
};
pub use interaction::InteractionState;
pub use menu::{
    checkbox_checked, checkbox_toggle_index, commit_menu_release, option_label, DerivedOptions,
    MenuBinding, MenuState, MenuStyle,
};
pub use reactive::ReactiveHandle;
pub use slider::{commit_slider_release, ParamDispatcher, SliderBinding, SliderState};
pub use vello_renderer::VelloRenderer;
pub use workspace::{
    framed_window_size, resolve_title, PresentationMode, CONTENT_PADDING, HEADER_HEIGHT,
};
