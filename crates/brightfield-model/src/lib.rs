//! Framework-free application models for the Brightfield desktop app.
//!
//! Every module here is a decision the app makes expressed as plain data and
//! arithmetic — no window, no GPU, no framework types — so each one compiles
//! and tests headlessly on any host. The windowed shell (the egui app,
//! which replaced the retired gpui one at the shell cutover) is a thin
//! translation shim over these models: it *executes* their decisions and
//! never re-makes them.
//!
//! The semantic-layer rule these modules carried as file headers is now the
//! crate boundary itself: **no UI-framework import may enter this crate.**
//! Its manifest names no UI-framework dependency, and keeping it that way
//! is what lets a new shell adopt every model below without touching one.
//!
//! - [`boot`] — the one decision separating the headless PNG dump from the
//!   windowed workspace.
//! - [`arg_collector`] — the palette argument-prompt state machine for the
//!   editing verbs that take arguments.
//! - [`dock_state_file`] — dock-layout persistence: file location, load
//!   usability, canvas stripping, save policy.
//! - [`log_model`] — the append-only reload/save feedback log (and the
//!   notification [`log_model::Severity`] vocabulary it shares with the
//!   reload-feedback router below).
//! - [`menu_resolve`] — resolving `input: menu` specs into launch-fixed
//!   widget placements.
//! - [`profile_model`] — source-profile presentation strings for the
//!   sidebar.
//! - [`reload_feedback`] — the hot-reload outcome → notification decision:
//!   what surfaces, how sticky it is, and what clears it.
//! - [`shell_model`] — panel identities, default dock geometry, the
//!   initial-window-size formula, and the presentation-mode visibility
//!   mapping.
//! - [`spec_save`] — the spec editor's save intelligence: the atomic
//!   temp+rename write, the two-writer conflict guard, and the pristine-
//!   buffer reseed/commit gates. Bytes in, bytes out — no spec type, no
//!   serialisation, no canonicalisation.

pub mod arg_collector;
pub mod boot;
pub mod dock_state_file;
pub mod log_model;
pub mod menu_resolve;
pub mod panel_capture;
pub mod profile_model;
pub mod reload_feedback;
pub mod shell_model;
pub mod spec_save;
