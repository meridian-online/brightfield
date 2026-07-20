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
pub mod design;
pub mod pipeline;
pub mod protocol;
