//! **A committed baseline, in both themes, of the hero map with a number column
//! on colour and its legend below it: the ramp under the plot in a band 44 high,
//! the column's name at its left and the domain's ends under it, nothing at the
//! plot's right.** The structural half is `legend_below.rs`, which holds the rule
//! over rects and shapes; this file is the photograph of it, and the test states
//! the structural fact before it takes the picture, so a reader of a red image
//! can tell whether the legend or only the ink moved (`tests/dashboard_baseline.rs`
//! says why the order matters).
//!
//! The chart is the one `legend_baseline.rs` photographs: the generated dashboard
//! for the point map's table, its hero map coloured by the `reading` column
//! through `shelf_edit::put_colour`. The legend `put_colour` writes in the hero is
//! taken out and written beside it instead, in the hero's own `vconcat`, `for` the
//! hero by name.
//!
//! Regenerate with: `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell
//! --test legend_below_baseline`.

use std::path::{Path, PathBuf};

use brightfield_engine::ProfileOutcome;
use brightfield_shell::capture::capture_png;
use brightfield_shell::data_file;
use brightfield_shell::design::Mode;
use brightfield_shell::legend::{band_width, below_blocks, blocks, LegendSpec};
use brightfield_shell::pipeline::LiveDashboard;
use brightfield_shell::shelf_edit::put_colour;
use brightfield_shell::window::Boot;
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::ast::{Component, SpecValue, ValueOrParamRef};
use brightfield_spec::edit::plot_at_path_mut;
use brightfield_spec::Spec;

/// Device pixels per logical point — `tests/dashboard_baseline.rs`'s scale.
const SCALE: f32 = 1.0;

/// The number column the hero is coloured by.
const COLUMN: &str = "reading";

/// The name the hero is given, which the legend beside it names by `for:`.
const HERO: &str = "hero";

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/point_map_baseline.csv")
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!("{name}.capture.png"))
}

/// The items of the concat at `path` — `root`, then `hconcat[i]` / `vconcat[i]`
/// for each step down.
fn concat_items_mut<'a>(spec: &'a mut Spec, path: &str) -> &'a mut Vec<Component> {
    let mut component = spec.root.as_mut().expect("the spec has a root");
    for step in path.split('/').skip(1) {
        let index: usize = step
            .split_once('[')
            .and_then(|(_, i)| i.strip_suffix(']'))
            .and_then(|i| i.parse().ok())
            .unwrap_or_else(|| panic!("{step:?} is not a path step"));
        component = match component {
            Component::HConcat(c) | Component::VConcat(c) => &mut c.items[index],
            other => panic!("{path}: {other:?} is not a concat"),
        };
    }
    match component {
        Component::VConcat(c) => &mut c.items,
        other => panic!("{path} is {other:?}, not a vconcat"),
    }
}

/// The hero map coloured by [`COLUMN`] with its legend below it, as a booted chart
/// page.
fn hero_with_legend_below() -> Boot {
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
    let hero_path = file.composed.plots[0].path.clone();
    let hero = ComponentPath(hero_path.clone());
    let mut spec = file.live.spec().clone();
    put_colour(&mut spec, &hero, COLUMN, &table).expect("the table has the column");

    // The legend `put_colour` wrote inside the hero comes out and is written
    // beside it, `for` the hero by name.
    let plot = plot_at_path_mut(&mut spec, &hero_path).expect("the hero");
    plot.attributes
        .insert("name".to_string(), SpecValue::String(HERO.to_string()));
    let Some(Component::Legend(mut legend)) = plot.items.pop() else {
        panic!("the last item of the coloured hero is not the legend");
    };
    legend.options.insert(
        "for".to_string(),
        ValueOrParamRef::Value(SpecValue::String(HERO.to_string())),
    );
    let (column, at) = hero_path.rsplit_once('/').expect("the hero is in a concat");
    let at: usize = at
        .split_once('[')
        .and_then(|(_, i)| i.strip_suffix(']'))
        .and_then(|i| i.parse().ok())
        .expect("the hero's place in its concat");
    concat_items_mut(&mut spec, column).insert(at + 1, Component::Legend(legend));

    let base = file.live.base_dir().map(Path::to_path_buf);
    let mut live = LiveDashboard::load(spec, base.as_deref()).expect("the spec loads");
    let composed = live.present().expect("the page presents");

    // The fact the picture is of, before the picture.
    assert!(
        matches!(
            below_blocks(&composed)[..],
            [(0, LegendSpec::Sequential { .. }, _)]
        ),
        "the hero does not draw a ramp under it: {:?}",
        below_blocks(&composed)
    );
    assert!(
        blocks(&composed).is_empty(),
        "the hero draws a legend at its right as well"
    );
    assert_eq!(
        band_width(&composed),
        0.0,
        "a band is reserved at the right"
    );

    let mut boot = Boot::charts(composed);
    boot.live = Some(live);
    boot
}

fn baseline(name: &str, mode: Mode) {
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = hero_with_legend_below();
    let out = scratch(name);
    let (w, h) = capture_png(boot, mode, SCALE, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");

    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();
    egui_kittest::image_snapshot(&image, name);
}

/// **The hero coloured by a number column, its legend below — light.**
#[test]
fn the_coloured_hero_with_its_legend_below_light_baseline() {
    baseline("legend_hero_below_light", Mode::Light);
}

/// **The same chart in dark.**
#[test]
fn the_coloured_hero_with_its_legend_below_dark_baseline() {
    baseline("legend_hero_below_dark", Mode::Dark);
}
