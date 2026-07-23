//! brightfield-shell — the egui/eframe host for the Vello mosaic canvas.
//!
//! The second stage of the gpui → egui/Vello migration: a framework-clean shell that
//! renders the real chart through the framework-free render seam on eframe's
//! shared wgpu device, with the Metal↔wgpu readback deleted. Its reason
//! to exist first is the loop — every later UI change is verifiable because the
//! real window can be captured headlessly:
//!
//! - [`window::MeridianApp`] — the one window, and the single frame logic all
//!   tiers share.
//! - [`canvas`] — [`canvas::EguiCanvasHost`] / [`canvas::EguiChartFrame`], the
//!   egui realisation of the `CanvasHost`/`ChartSurface`/`OverlayPainter` seam.
//! - [`design`] — the Meridian Design System → egui `Visuals`/`Style`/fonts.
//! - [`pipeline`] — spec → composited Vello scene (framework-free).
//! - [`capture`] — headless egui_wgpu → PNG (the `brightfield-shot` binary).
//! - [`app`] / [`protocol`] — the two views, each declared as `Item`s on the
//!   `brightfield-workbench` shell contract.
//! - [`chart_item`] — the chart pane: one `Item`, parameterised by mark kind,
//!   binding brush/click gestures to the engine's coordinator seam.
//! - [`legend`] — the chart legend as a native egui margin panel, outside the
//!   plot rect, derived from the scales each chart was drawn against.
//! - [`data_grid`] — the Data pane: the chart's peer, a DuckDB-backed grid
//!   reading the SAME step materialisation through the engine's windowed
//!   rows seam — and the one Meridian table chrome the Steps sheet renders
//!   through as well.
//! - [`editor`] — the YAML spec editor: the highlighted code surface, its
//!   read-only treatment, and the `EditorPane` item over the
//!   `brightfield-model` save intelligence.
//! - [`overlays`] — the picker delegates: the domain halves of the command
//!   palette, help sheet, jump lists and argument prompt, over the
//!   framework-free corpora in `brightfield-keys` / `brightfield-model`.
//!   The chrome they render through is `meridian-egui`'s one `Picker` inside
//!   its one `ModalLayer`.
//! - [`startup`] — where the layout file lives, and the one order it can be
//!   read in.
//! - [`starts`] — the starting points that ship inside the binary, which are
//!   what a launch with nothing on the command line offers.

pub mod app;
pub mod canvas;
pub mod capture;
pub mod chart_item;
pub mod data_grid;
pub mod editor;
pub mod legend;
pub mod overlays;
pub mod pipeline;
pub mod protocol;
pub mod starts;
pub mod startup;
pub mod window;

/// The Meridian Design System → egui bridge.
///
/// It lives in the `meridian-egui` crate now — the design repo's egui emitter,
/// beside the tokens it translates (ADR 0003/0011), rather than in this
/// consumer. Re-exported under this crate's `design` path because
/// `brightfield_shell::design::…` is the spelling this crate, its snapshot tier
/// and the headless shot binary all already use, and renaming them would bury
/// the change worth reading.
pub mod design {
    pub use meridian_egui::theme::{
        apply, install_fonts, meridian_style, meridian_visuals, to_color32,
    };
    pub use meridian_egui::Mode;
}
