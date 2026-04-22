//! GPUI application shell — Vello chart rendering + interaction in a native window.
//!
//! This crate wraps `brightfield-render`'s headless chart scene into a GPUI
//! element with texture handoff (CPU readback for v1) and interaction state
//! management (brush/hover overlay).
//!
//! **Dependency chain:** `brightfield-render` -> `brightfield-ui`.
//! GPUI is a dependency of this crate only, not of `brightfield-render`.

pub mod chart_element;
pub mod interaction;

pub use chart_element::ChartElement;
pub use interaction::InteractionState;
