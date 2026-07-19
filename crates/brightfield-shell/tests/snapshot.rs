//! Tier-1 of the real-UI loop: `egui_kittest` snapshot tests.
//!
//! These render the Meridian design bridge through egui's real wgpu backend and
//! perceptually diff against committed baselines (`tests/snapshots/*.png`) using
//! `dify` (egui_kittest's snapshot engine) — NOT byte-exact, so text AA jitter
//! doesn't fail the gate. They pin the design→egui chrome (fonts, visuals, widget
//! ink); the full Vello composite is covered headlessly by `brightfield-shot`
//! (Tier-2), whose zero-copy native texture can't ride kittest's own renderer.
//!
//! Regenerate baselines with: `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p
//! brightfield-shell --test snapshot`.

use brightfield_shell::design::{self, Mode};
use egui_kittest::{Harness, SnapshotOptions};

/// A representative chrome sheet exercising the bridged widgets: heading, body
/// text (Inter), monospace (JetBrains Mono), a slider, a checkbox, and the
/// categorical swatches — everything the shell's native side panel uses.
fn chrome_sheet(ui: &mut egui::Ui, mode: Mode, demo: &mut f32, checked: &mut bool) {
    design::apply(ui.ctx(), mode);
    egui::containers::CentralPanel::default().show(ui, |ui| {
        ui.heading("Meridian chrome");
        ui.separator();
        ui.label("Inter body — situational awareness");
        ui.label(egui::RichText::new("JetBrains Mono  0O 1l  =>").monospace());
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Series").strong());
        for (name, token) in [
            ("A", meridian_design::scales::MARITIME_LIGHT[8]),
            ("B", meridian_design::scales::GREEN_LIGHT[8]),
            ("C", meridian_design::scales::AMBER_LIGHT[8]),
        ] {
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 2.0, design::to_color32(token));
                ui.label(name);
            });
        }
        ui.separator();
        ui.add(egui::Slider::new(demo, 0.0..=1.0).text("param"));
        ui.checkbox(checked, "hover overlay");
        let _ = ui.button("Run");
    });
}

/// A lenient perceptual gate: allow GPU/AA jitter across machines while still
/// catching a real chrome regression (a wrong colour, missing widget, font
/// swap covers far more than this many pixels).
fn options() -> SnapshotOptions {
    SnapshotOptions::default()
        .threshold(2.5)
        .failed_pixel_count_threshold(400)
}

#[test]
fn chrome_light_snapshot() {
    let mut demo = 0.5_f32;
    let mut checked = true;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(280.0, 340.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .build_ui(move |ui| chrome_sheet(ui, Mode::Light, &mut demo, &mut checked));
    harness.run();
    harness.snapshot_options("chrome_light", &options());
}

#[test]
fn chrome_dark_snapshot() {
    let mut demo = 0.5_f32;
    let mut checked = true;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(280.0, 340.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .build_ui(move |ui| chrome_sheet(ui, Mode::Dark, &mut demo, &mut checked));
    harness.run();
    harness.snapshot_options("chrome_dark", &options());
}
