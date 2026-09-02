//! **A committed baseline, in both themes, for a table whose columns are a
//! coordinate pair** — the AC4 half of `point-map-kind`. The structural half
//! mirrors `tests/dashboard_baseline.rs`, which explains at length why the
//! choice table runs ahead of the photograph: an image diff reddens on a font
//! bump exactly as loudly as on a moved tile choice, and a reviewer holding
//! one red baseline cannot tell which of those happened.
//!
//! Regenerate with: `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell
//! --test point_map_baseline`.

use std::path::PathBuf;

use brightfield_shell::capture::capture_png;
use brightfield_shell::dashboard::{self, ChosenBy, Dashboard};
use brightfield_shell::design::Mode;
use brightfield_shell::window::Boot;
use brightfield_shell::{chart_kinds, data_file};

/// Device pixels per logical point — `tests/dashboard_baseline.rs`'s scale.
const SCALE: f32 = 1.0;

/// The committed table: a coordinate pair named so the column-name tier of
/// `dashboard::coordinate_pair` finds it (a `cargo test` binary carries no
/// FineType bundle, so the label tier never fires here — the same reason
/// `tests/dashboard_baseline.rs` documents for its own fixture), plus one
/// ordinary measure so the baseline also proves a coordinate pair does not
/// swallow every numeric column in the file.
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/point_map_baseline.csv")
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!("{name}.capture.png"))
}

/// **The tile choice, as a table** — `tests/dashboard_baseline.rs`'s device,
/// over a fixture where the choice is a pair rather than four singles.
fn assert_choices(dash: &Dashboard) {
    let tiles = dash.tiles();
    assert_eq!(
        tiles.len(),
        2,
        "the fixture has three columns and a coordinate pair among them, so \
         this dashboard should hold one point-map tile and one histogram — it \
         holds {}: {:?}",
        tiles.len(),
        tiles
            .iter()
            .map(dashboard::Tile::column)
            .collect::<Vec<_>>()
    );

    let map = &tiles[0];
    assert_eq!(
        map.kind(),
        chart_kinds::POINT_MAP,
        "the first tile is not the point map — the generator's tile choices \
         have moved"
    );
    assert_eq!(map.column(), "longitude", "the map's primary column moved");
    assert_eq!(
        map.paired_column(),
        Some("latitude"),
        "the map's paired column moved"
    );
    match map.chosen_by() {
        ChosenBy::CoordinatePair { latitude, rule } => {
            assert_eq!(latitude, "latitude");
            assert_eq!(
                *rule, "name",
                "a test binary carries no FineType bundle, so the pair should \
                 have been found by column name, not by label"
            );
        }
        other => panic!("the map tile's chosen_by moved: {other:?}"),
    }

    let reading = &tiles[1];
    assert_eq!(reading.column(), "reading");
    assert_eq!(
        reading.kind(),
        chart_kinds::BINNED_HISTOGRAM,
        "the third column's tile choice moved"
    );

    assert!(
        dash.omitted().is_empty(),
        "a column was left out of this dashboard: {:?}",
        dash.omitted()
    );
}

/// **The structural claim**, on its own — so a reader of a red pixel test
/// finds this line first and knows whether the choice or only the ink moved.
#[test]
fn the_point_map_and_a_histogram_are_the_two_tiles_this_table_earns() {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let opened = data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    assert_choices(&opened.dashboard);
}

/// **The generated dashboard for a coordinate pair, as pixels — light.**
#[test]
fn the_point_map_dashboard_light_baseline() {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");

    let opened = data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    assert_choices(&opened.dashboard);
    assert_eq!(
        opened.composed.plots.len(),
        2,
        "the walk chose 2 tiles and the composition placed {} plots, so the \
         image below is not a picture of those choices",
        opened.composed.plots.len()
    );
    drop(opened);

    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let out = scratch("point_map_light");
    let (w, h) = capture_png(boot, Mode::Light, SCALE, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture point_map_light: {e}"));
    assert!(w > 0 && h > 0, "point_map_light: empty capture");

    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();
    egui_kittest::image_snapshot(&image, "point_map_light");
}

/// **The same dashboard in dark.**
#[test]
fn the_point_map_dashboard_dark_baseline() {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");

    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let out = scratch("point_map_dark");
    let (w, h) = capture_png(boot, Mode::Dark, SCALE, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture point_map_dark: {e}"));
    assert!(w > 0 && h > 0, "point_map_dark: empty capture");

    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();
    egui_kittest::image_snapshot(&image, "point_map_dark");
}
