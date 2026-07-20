//! The shell's frame logic, shared by all three capture tiers.
//!
//! [`draw_shell`] is the single source of the UI — the live eframe window
//! (`main.rs`), the headless `brightfield-shot` binary, and the `egui_kittest`
//! snapshot tests all call it, so what an agent sees in a PNG is exactly what
//! ships. It presents the composited Vello dashboard as a zero-copy egui texture
//! in a real `CentralPanel`, wraps it in native egui chrome (header + a side
//! panel with a legend and a slider — the widgets the render trait does *not*
//! cover), and drives the overlay/hit-test seam from egui pointer input.

use brightfield_render::canvas_host::{CanvasHost, ChartSurface, Color, PixelSize, SurfaceCursor};
use meridian_design::chrome::{INK_DARK, INK_LIGHT};

use egui::containers::{CentralPanel, Panel};

use crate::canvas::{surface_input, EguiCanvasHost, EguiChartFrame};
use crate::design::{self, Mode};
use crate::pipeline::Composed;

/// The shell's whole state: the composited dashboard, the egui canvas host, the
/// presented texture id, and the small bit of native-chrome state.
pub struct ShellState {
    composed: Composed,
    host: EguiCanvasHost,
    texture: Option<egui::TextureId>,
    /// The device-pixel resolution the texture was last presented at (re-present
    /// on a HiDPI-scale change so the raster stays crisp).
    presented_size: Option<PixelSize>,
    mode: Mode,
    /// A native egui slider (a shell widget, not part of the chart trait).
    demo_value: f32,
    /// Whether to draw the hover crosshair overlay — the worked example that keeps
/// the overlay seam exercised end to end.
    overlay_on: bool,
    fonts_installed: bool,
}

impl ShellState {
    /// Build the shell around a composited dashboard and its egui host.
    pub fn new(composed: Composed, host: EguiCanvasHost, mode: Mode) -> Self {
        Self {
            composed,
            host,
            texture: None,
            presented_size: None,
            mode,
            demo_value: 0.5,
            overlay_on: true,
            fonts_installed: false,
        }
    }

    /// The composited dashboard's logical size.
    pub fn size(&self) -> (u32, u32) {
        (self.composed.width, self.composed.height)
    }

    /// The shell's natural window size in logical points: the dashboard plus the
    /// header band and the right side panel. Used to size the headless capture
    /// so the whole UI fits without clipping.
    pub fn window_size(&self) -> (f32, f32) {
        // chart + CentralPanel margins + the 180px side panel + header band.
        (self.composed.width as f32 + 214.0, self.composed.height as f32 + 60.0)
    }

    /// The window/spec title.
    pub fn title(&self) -> &str {
        self.composed.title.as_deref().unwrap_or("Brightfield")
    }

    fn page_base(&self) -> Color {
        let page = match self.mode {
            Mode::Light => INK_LIGHT.page,
            Mode::Dark => INK_DARK.page,
        };
        Color::from_token(page)
    }

    /// Rasterise the Vello dashboard onto a shared-device texture at the current
    /// HiDPI scale and register it for zero-copy egui sampling — only when the
    /// device resolution actually changed.
    ///
    /// The composited scene is in logical coordinates, so it is scaled by `ppp`
    /// onto the device-resolution texture (the same scale-the-scene step the
    /// app's dump path uses) — otherwise the logical-sized scene would fill only
    /// the top-left corner of the larger texture.
    fn ensure_presented(&mut self, ppp: f32) {
        let dev = PixelSize {
            width: ((self.composed.width as f32) * ppp).round().max(1.0) as u32,
            height: ((self.composed.height as f32) * ppp).round().max(1.0) as u32,
        };
        if self.presented_size == Some(dev) && self.texture.is_some() {
            return;
        }
        let mut scaled = vello::Scene::new();
        scaled.append(
            &self.composed.scene,
            Some(kurbo::Affine::scale(f64::from(ppp))),
        );
        let base = self.page_base();
        let id = self.host.present_scene(&scaled, dev, base);
        self.texture = Some(id);
        self.presented_size = Some(dev);
    }
}

/// Draw one shell frame into the root `ui` (egui 0.35's Ui-rooted model — the
/// same `ui` eframe hands `App::ui` and `Context::run_ui` yields). Idempotent
/// and tier-agnostic.
pub fn draw_shell(ui: &mut egui::Ui, state: &mut ShellState) {
    let ctx = ui.ctx().clone();
    if !state.fonts_installed {
        design::apply(&ctx, state.mode);
        state.fonts_installed = true;
    }

    state.ensure_presented(ctx.pixels_per_point());
    let texture = state.texture.expect("texture presented above");

    Panel::top("bf-shell-header").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading(state.title().to_string());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mode = match state.mode {
                    Mode::Light => "light",
                    Mode::Dark => "dark",
                };
                ui.label(
                    egui::RichText::new(format!("egui · Vello · wgpu 29  —  {mode}")).monospace(),
                );
            });
        });
    });

    Panel::right("bf-shell-side")
        .resizable(false)
        .exact_size(180.0)
        .show(ui, |ui| {
            ui.add_space(6.0);
            // Legend fix (review): the hardcoded "Series A/B/C" swatch block used
            // to live here — it duplicated the chart's own in-scene legend and,
            // being fixed at three series, mislabelled a single-series bar chart.
            // Cut. Accurate, one-per-chart legends belong in the plot margin,
            // derived from each chart's real series — a follow-up that needs the
            // compose pipeline to surface series metadata (it is currently baked
            // into the Vello scene by `build_multi_mark_scene`).
            ui.label(egui::RichText::new("Controls").strong());
            ui.label("param");
            ui.add(egui::Slider::new(&mut state.demo_value, 0.0..=1.0));
            ui.checkbox(&mut state.overlay_on, "hover overlay");
            ui.add_space(6.0);
            let (w, h) = state.size();
            ui.label(egui::RichText::new(format!("{w}×{h} logical")).weak().monospace());
        });

    CentralPanel::default().show(ui, |ui| {
        let mut frame = EguiChartFrame::new(ui, texture);
        let (w, h) = state.size();
        frame.present(PixelSize { width: w, height: h });

        // Drive the overlay/hit-test seam from egui pointer input: a crosshair
        // marker at the pointer while it's over the chart, with a grab cursor.
        if let Some(rect) = frame.reserved() {
            let input = surface_input(&ctx, rect);
            if state.overlay_on && input.hovered {
                if let Some(p) = input.pointer_pos {
                    let focus = match state.mode {
                        Mode::Light => INK_LIGHT.focus,
                        Mode::Dark => INK_DARK.focus,
                    };
                    let ink = Color::from_token_alpha(focus, 0.9);
                    let (wid, hgt) = state.size();
                    let painter = frame.overlay();
                    painter.line(
                        kurbo::Point::new(p.x, 0.0),
                        kurbo::Point::new(p.x, f64::from(hgt)),
                        ink,
                        1.0,
                    );
                    painter.line(
                        kurbo::Point::new(0.0, p.y),
                        kurbo::Point::new(f64::from(wid), p.y),
                        ink,
                        1.0,
                    );
                    painter.fill_circle(p, 3.0, ink);
                }
                frame.set_cursor(Some(SurfaceCursor::Grab));
            }
        }
    });
}
