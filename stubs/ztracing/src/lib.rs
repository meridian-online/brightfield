//! No-op stand-in for zed's `ztracing` crate (GPL-3.0-or-later), substituted via
//! `[patch."https://github.com/zed-industries/zed"]` in the workspace manifest so
//! that MIT-licensed Brightfield binaries statically link no GPL code.
//!
//! Written clean-room against the API surface our dependency graph actually
//! consumes (currently: `sum_tree` uses `#[instrument(skip_all)]` on a handful of
//! methods — profiling instrumentation that is inert outside zed's tracy builds).
//! If a future zed pin bump grows the consumed surface, the compiler will point
//! here; add the missing no-op item rather than widening the patch.

pub use ztracing_stub_macro::instrument;
