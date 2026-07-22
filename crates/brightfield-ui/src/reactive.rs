//! ReactiveHandle — the host-side reactive-state seam.
//!
//! The cross-filter coordinator's whole job is "mutate a plot's
//! [`ChartState`], then have the host repaint it". Which *cell* owns that
//! state, and how a repaint is requested, is the only part of the chain that
//! belongs to a UI framework: the gpui shell wraps each `ChartState` in an
//! `Entity` and notifies through an `App`; an egui shell owns plain cells and
//! requests a repaint from its frame context; a headless test can hold the
//! state directly and ignore repaints entirely. This trait is that seam —
//! the coordinator (and any other chart logic) is written against it and
//! names no host type, so a new shell drops in by implementing ONE trait
//! without touching the logic again.
//!
//! # Shape of the contract
//!
//! - **`Surface`** — the host texture handle its [`ChartState`] caches (the
//!   [`crate::canvas_host::CanvasHost::Surface`] of the host's present path).
//!   It rides here so a handle determines the full state type
//!   `ChartState<Surface>` and the two host seams cannot be mixed.
//! - **`Cx`** — whatever the host must thread through an update to service
//!   reactivity (gpui: `App`). A host with globally reachable cells uses
//!   `()`. It is an associated type, not a generic parameter, so the
//!   coordinator's methods stay callable with exactly one context type per
//!   handle — the compiler rejects a caller handing the wrong shell's
//!   context to a handle.
//! - **`update` takes `FnOnce(&mut ChartState<_>) -> bool`** — the closure
//!   returns whether the state changed in a way the host should repaint.
//!   Putting the repaint *decision* in the closure keeps it with the logic
//!   (only the logic knows whether anything changed); the repaint
//!   *mechanism* stays with the host (gpui: `notify`; egui: request-repaint;
//!   tests: nothing). No `read` counterpart is offered: the coordinator's
//!   data half lives in its own fields, and paint-time reads are the host
//!   shell's business, through its own cell type.
//! - **`Clone`** — the coordinator clones a plot's handle out of its plot
//!   table before servicing a rebuild loop, exactly as the gpui shell
//!   cloned `Entity` handles. Handles are expected to be cheap references
//!   to shared cells, not owners.
//!
//! The gpui implementation (`Entity<ChartState<…>>` + `App`) lives in
//! [`crate::gpui_canvas`] with the rest of that host's glue.

use crate::chart_state::ChartState;

/// A cloneable host-side handle to one plot's reactively owned
/// [`ChartState`] cell. See the module docs for the design of the seam.
pub trait ReactiveHandle: Clone {
    /// The host texture handle the addressed state caches its base raster
    /// as (gpui: `Arc<RenderImage>`; an egui host: its texture id).
    type Surface;

    /// The host context threaded through every update (gpui: `App`; a host
    /// whose cells are reachable without one: `()`).
    type Cx;

    /// Run `f` over the addressed chart state. `f` returns `true` to request
    /// a host repaint of the surfaces reading this state; the host decides
    /// what "repaint" means (gpui: `Context::notify`).
    fn update(&self, cx: &mut Self::Cx, f: impl FnOnce(&mut ChartState<Self::Surface>) -> bool);
}
