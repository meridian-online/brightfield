//! **A committed baseline, in both themes, of a hexbin coloured by its count and
//! of a heatmap, each with its legend at the plot's right: the transform's word
//! over the ramp.** The structural half is `legend_transform_name.rs`, which holds
//! the name over the painted text; this file is the photograph of it, and each
//! test states the structural fact before it takes the picture, so a reader of a
//! red image can tell whether the name or only the ink moved
//! (`tests/dashboard_baseline.rs` says why the order matters).
//!
//! The specs carry their data inline, so the run needs no file beside it.
//!
//! Regenerate with: `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell
//! --test legend_transform_baseline`.

use std::path::PathBuf;

use brightfield_shell::capture::capture_png;
use brightfield_shell::design::Mode;
use brightfield_shell::legend::{band_width, legend_name};
use brightfield_shell::pipeline::compose_spec_str;
use brightfield_shell::window::Boot;

/// Sixty rows in seven clusters, so the bins hold different counts and the
/// ramp has a range to show.
fn page(block: &str) -> String {
    let rows: String = (0..60)
        .map(|i| {
            let cluster = (i % 7) as f64;
            let x = cluster * 1.5 + ((i * 13) % 10) as f64 / 40.0;
            let y = (cluster * 2.0) % 9.0 + ((i * 7) % 10) as f64 / 40.0;
            format!("    - {{ x: {x}, y: {y} }}\n")
        })
        .collect();
    format!(
        "data:\n  t:\n{rows}plot:\n  - mark: {block}\n    data: {{ from: t }}\n    x: x\n    y: y\n  - legend: color\ncolorScheme: blues\nwidth: 420\nheight: 300\n"
    )
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!("{name}.capture.png"))
}

/// The chart as a booted page, photographed: the name the legend carries first.
fn baseline(name: &str, mode: Mode, block: &str, word: &str) {
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let source = page(block);
    let composed = compose_spec_str(&source, None)
        .unwrap_or_else(|e| panic!("{name}: the spec must compose: {e}\n{source}"));
    assert_eq!(
        legend_name(&composed.plots[0]),
        Some(word),
        "{name}: the legend's name"
    );
    assert!(band_width(&composed) > 0.0, "{name}: no band for the legend");

    let out = scratch(name);
    let (w, h) = capture_png(Boot::charts(composed), mode, 1.0, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();
    egui_kittest::image_snapshot(&image, name);
}

/// **A hexbin coloured by its count — light.** `count` over the ramp.
#[test]
fn the_hexbin_legend_light_baseline() {
    baseline(
        "legend_hexbin_count_light",
        Mode::Light,
        "hexbin\n    fill: { count: }",
        "count",
    );
}

/// **The same chart in dark.**
#[test]
fn the_hexbin_legend_dark_baseline() {
    baseline(
        "legend_hexbin_count_dark",
        Mode::Dark,
        "hexbin\n    fill: { count: }",
        "count",
    );
}

/// **A heatmap — light.** `density` over the ramp.
#[test]
fn the_heatmap_legend_light_baseline() {
    baseline(
        "legend_heatmap_density_light",
        Mode::Light,
        "heatmap\n    bins: 20",
        "density",
    );
}

/// **The same chart in dark.**
#[test]
fn the_heatmap_legend_dark_baseline() {
    baseline(
        "legend_heatmap_density_dark",
        Mode::Dark,
        "heatmap\n    bins: 20",
        "density",
    );
}
