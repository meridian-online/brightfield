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
//!
//! Thresholds come from `kittest.toml` at the workspace root — read the policy
//! comment there before reaching for any per-test override.
//!
//! This tier needs a real wgpu adapter, so it is the one part of the suite a
//! GPU-less machine cannot run. There is deliberately no skip switch: every
//! machine that runs the suite today (the local loop and the macOS CI runner)
//! has an adapter, and an env-var opt-out would render "no GPU here" as a
//! passing test — the exact silently-green outcome this tier exists to prevent.
//! If a GPU-less runner is ever added, mark these `#[ignore]` and opt in
//! explicitly on the runners that can render, so a skip reads as a skip.

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
                ui.painter()
                    .rect_filled(rect, 2.0, design::to_color32(token));
                ui.label(name);
            });
        }
        ui.separator();
        ui.add(egui::Slider::new(demo, 0.0..=1.0).text("param"));
        ui.checkbox(checked, "hover overlay");
        let _ = ui.button("Run");
    });
}

/// The repo-default perceptual gate, straight from `kittest.toml`: `dify`
/// per-pixel delta `0.6`, and `failed_pixel_count_threshold = 0` so no pixel may
/// exceed that delta.
///
/// Read the second number honestly: zero pixels of *budget*, not zero
/// difference. `0.6` is itself a perceptual tolerance, so a render that differs
/// from the baseline everywhere by less than it still passes. Measured on this
/// harness, nudging the swatch corner rounding 2.0 → 3.0 does **not** trip the
/// gate — sub-pixel geometry on a 12x12 square stays under the per-pixel delta.
/// What does trip it is a change with real ink behind it: swapping one swatch to
/// the adjacent shade of the same scale marks 536 pixels.
///
/// This used to carry `.threshold(2.5).failed_pixel_count_threshold(400)`, a
/// pre-emptive loosening against cross-machine AA jitter that had never been
/// shown to be necessary. Re-measured at the library floor, both baselines pass
/// with zero differing pixels on the reference machine, so the slack bought
/// nothing here and is gone.
///
/// If a future machine genuinely jitters, override per test and say in the
/// comment which jitter, and where you saw it.
fn options() -> SnapshotOptions {
    SnapshotOptions::default()
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
