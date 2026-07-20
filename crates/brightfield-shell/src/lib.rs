//! brightfield-shell — the egui/eframe host for the Vello mosaic canvas.
//!
//! The second stage of the gpui → egui/Vello migration: a gpui-free shell that
//! renders the real chart through the framework-free render seam on eframe's
//! shared wgpu device, with the Metal↔wgpu readback deleted. Its reason
//! to exist first is the loop — every later UI change is verifiable because the
//! real window can be captured headlessly:
//!
//! - [`app::draw_shell`] — the single frame logic all tiers share.
//! - [`canvas`] — [`canvas::EguiCanvasHost`] / [`canvas::EguiChartFrame`], the
//!   egui realisation of the `CanvasHost`/`ChartSurface`/`OverlayPainter` seam.
//! - [`design`] — the Meridian Design System → egui `Visuals`/`Style`/fonts.
//! - [`pipeline`] — spec → composited Vello scene (gpui-free).
//! - [`capture`] — headless egui_wgpu → PNG (the `brightfield-shot` binary).

pub mod app;
pub mod canvas;
pub mod capture;
pub mod pipeline;
pub mod protocol;

/// The Meridian Design System → egui bridge.
///
/// It lives in `brightfield-workbench` now: the workbench draws every pixel of
/// chrome from `meridian_design` tokens, so leaving the `Style`/`Visuals`/font
/// bridge downstream of it meant a workbench frame rendered in egui's default
/// type and widget ink. Re-exported here because `brightfield_shell::design`
/// is the path this crate, its snapshot tier and the headless shot binary all
/// already spell, and renaming them would bury the change worth reading.
pub use brightfield_workbench::design;
