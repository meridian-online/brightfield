//! **A committed baseline, in both themes, of the hero map with a number column
//! on colour: the legend at its right when the chart holds the legend item, and
//! no legend when it does not.** The structural half is `legend_declared.rs`,
//! which holds the rule over rects; this file is the photograph of it, and each
//! test states the structural fact before it takes the picture, so a reader of
//! a red image can tell whether the legend or only the ink moved
//! (`tests/dashboard_baseline.rs` says why the order matters).
//!
//! The chart is the one the shelf makes: the generated dashboard for the point
//! map's table, its hero map coloured by the `reading` column through
//! `shelf_edit::put_colour`, which writes the legend item beside the scheme. The
//! chart without the item is that spec with the item taken out, which is what a
//! file that says no legend holds.
//!
//! Regenerate with: `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell
//! --test legend_baseline`.

use std::path::{Path, PathBuf};

use brightfield_engine::ProfileOutcome;
use brightfield_shell::capture::capture_png;
use brightfield_shell::data_file;
use brightfield_shell::design::Mode;
use brightfield_shell::legend::{band_width, LegendSpec};
use brightfield_shell::pipeline::LiveDashboard;
use brightfield_shell::shelf_edit::put_colour;
use brightfield_shell::window::Boot;
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::ast::Component;
use brightfield_spec::edit::plot_at_path_mut;

/// Device pixels per logical point — `tests/dashboard_baseline.rs`'s scale.
const SCALE: f32 = 1.0;

/// The number column the hero is coloured by.
const COLUMN: &str = "reading";

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/point_map_baseline.csv")
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!("{name}.capture.png"))
}

/// The hero map coloured by [`COLUMN`], as a booted chart page: with the legend
/// item `put_colour` writes, or with it taken out.
fn coloured_hero(with_item: bool) -> Boot {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let mut file =
        data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let table = file
        .live
        .coordinator()
        .session()
        .profile_sources()
        .into_iter()
        .find(|p| p.name == data_file::SOURCE)
        .map(|p| match p.outcome {
            ProfileOutcome::Profiled { columns, .. } => columns,
            other => panic!("the table did not profile: {other:?}"),
        })
        .expect("the opened file has a source to profile");
    let hero = ComponentPath(file.composed.plots[0].path.clone());
    let mut spec = file.live.spec().clone();
    put_colour(&mut spec, &hero, COLUMN, &table).expect("the table has the column");
    if !with_item {
        let taken = plot_at_path_mut(&mut spec, &hero.0)
            .expect("the hero")
            .items
            .pop();
        assert!(
            matches!(taken, Some(Component::Legend(_))),
            "the last item of the coloured hero is {taken:?}, not the legend"
        );
    }

    let base = file.live.base_dir().map(Path::to_path_buf);
    let mut live = LiveDashboard::load(spec, base.as_deref()).expect("the coloured spec loads");
    let composed = live.present().expect("the coloured page presents");

    // The fact each picture is of, before the picture.
    let scales = &composed.plots[0].scales;
    assert!(
        matches!(
            LegendSpec::from_scales(scales),
            Some(LegendSpec::Sequential { .. })
        ),
        "{COLUMN} on colour put no sequential scale on the hero"
    );
    if with_item {
        assert!(
            matches!(
                LegendSpec::of_plot(&composed.plots[0]),
                Some(LegendSpec::Sequential { .. })
            ),
            "the hero holding the item draws no legend"
        );
        assert!(band_width(&composed) > 0.0, "no band for the legend");
    } else {
        assert_eq!(
            LegendSpec::of_plot(&composed.plots[0]),
            None,
            "the hero without the item draws a legend"
        );
        assert_eq!(band_width(&composed), 0.0, "a band for no legend");
    }

    let mut boot = Boot::charts(composed);
    boot.live = Some(live);
    boot
}

fn baseline(name: &str, mode: Mode, with_item: bool) {
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = coloured_hero(with_item);
    let out = scratch(name);
    let (w, h) = capture_png(boot, mode, SCALE, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");

    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();
    egui_kittest::image_snapshot(&image, name);
}

/// **The hero coloured by a number column, holding the legend item — light.**
#[test]
fn the_coloured_hero_with_the_legend_item_light_baseline() {
    baseline("legend_hero_with_item_light", Mode::Light, true);
}

/// **The same chart in dark.**
#[test]
fn the_coloured_hero_with_the_legend_item_dark_baseline() {
    baseline("legend_hero_with_item_dark", Mode::Dark, true);
}

/// **The hero coloured by a number column, the legend item taken out — light.**
/// The points wear the colour and no legend is drawn.
#[test]
fn the_coloured_hero_without_the_legend_item_light_baseline() {
    baseline("legend_hero_without_item_light", Mode::Light, false);
}

/// **The same chart in dark.**
#[test]
fn the_coloured_hero_without_the_legend_item_dark_baseline() {
    baseline("legend_hero_without_item_dark", Mode::Dark, false);
}
