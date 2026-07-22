//! Pixel baselines for the code surface — the one part of the editor a
//! GPU-free test cannot see.
//!
//! Four baselines, all NEW in this increment: the editable surface and the
//! read-only treatment, each in both modes. The sample buffer exercises
//! every token kind the highlighter distinguishes (keys, strings, numbers,
//! booleans, comments — whole-line and trailing — anchors, flow
//! punctuation), plus enough lines that the gutter shows two-digit
//! numbering, so an ink regression in any of them marks pixels.
//!
//! Same tier and workflow as `snapshot.rs`: egui_kittest on real wgpu,
//! thresholds from `kittest.toml`, regenerate with
//! `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell --test
//! editor_snapshot`.

use brightfield_shell::design::{self, Mode};
use brightfield_shell::editor::{code_surface, Access};
use egui_kittest::{Harness, SnapshotOptions};

/// Every token kind, two-digit line numbers, real indentation, and a
/// deliberately preserved double space — the things a baseline should
/// notice changing. A raw string (not `\`-continuations, which eat the
/// leading whitespace) so the indentation is real.
const SAMPLE: &str = r#"# dashboard spec — sample for the pixel tier
title:  'Two spaces, kept'
size: [640, 320]
base: &shared
  opacity: 0.85
  visible: true
plots:
  - mark: dot
    from: *shared
    label: "μ per day"  # trailing note
  - mark: line
    stroke: steelblue
    width: 2
"#;

fn harness(access: Access, mode: Mode) -> Harness<'static, String> {
    Harness::builder()
        .with_size(egui::vec2(520.0, 340.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .build_ui_state(
            move |ui, text| {
                design::apply(ui.ctx(), mode);
                egui::containers::CentralPanel::default().show(ui, |ui| {
                    code_surface(ui, "editor-baseline", text, access, mode);
                });
            },
            SAMPLE.to_string(),
        )
}

/// The repo-default perceptual gate — see `kittest.toml`'s policy comment.
fn options() -> SnapshotOptions {
    SnapshotOptions::default()
}

#[test]
fn editor_light_snapshot() {
    let mut harness = harness(Access::Editable, Mode::Light);
    harness.run();
    harness.snapshot_options("editor_light", &options());
}

#[test]
fn editor_dark_snapshot() {
    let mut harness = harness(Access::Editable, Mode::Dark);
    harness.run();
    harness.snapshot_options("editor_dark", &options());
}

#[test]
fn editor_readonly_light_snapshot() {
    let mut harness = harness(Access::ReadOnly, Mode::Light);
    harness.run();
    harness.snapshot_options("editor_readonly_light", &options());
}

#[test]
fn editor_readonly_dark_snapshot() {
    let mut harness = harness(Access::ReadOnly, Mode::Dark);
    harness.run();
    harness.snapshot_options("editor_readonly_dark", &options());
}
