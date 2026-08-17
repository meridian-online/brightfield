//! **No light paint reaches a dark chart.**
//!
//! The failure this exists for has one shape, and `paints()` below enumerates
//! nineteen of the twenty scalar paints plus two of the eight Harbour slots.
//! `legend_bar_border` is the scalar it omits, so this sweep is not the guard
//! for that field. The shape: a module
//! that resolves its ink from a `const` bound to a `*_LIGHT` token draws that
//! ink whatever mode the window is in, and everything around it goes dark while
//! it does not. On the chart surface that is a white slab exactly the size of
//! the plot; on a gridline or a tick it is a hairline nobody can see.
//!
//! So this does not check that the modules were *given* a [`ChartInk`] — that
//! is the thing a test can pass while the code is broken, because a module can
//! take a palette and ignore it. It builds real scenes through the real scene
//! builder, reads the colours vello actually encoded into `draw_data`, and
//! holds two claims over every paint:
//!
//! 1. the DARK value is in the dark scene, and
//! 2. the LIGHT value is **not**.
//!
//! The second is the one that reddens. Revert any single module to its retired
//! `*_LIGHT` const and its light colour reappears in the buffer, naming itself.
//!
//! `draw_data` is vello's packed premultiplied RGBA8, one `u32` per encoded
//! brush — the probe `legend.rs` and `mark.rs` already use for the same reason:
//! it reads what was drawn rather than what the code was handed.

use std::collections::HashSet;
use std::sync::Arc;

use arrow::array::{Float64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use peniko::Color;

use brightfield_render::channel::{Channel, ChannelMap};
use brightfield_render::ink::ChartInk;
use brightfield_render::layout::ChartLayout;
use brightfield_render::mark::DotRenderer;
use brightfield_render::scale::{Scale, ScaleSet};
use brightfield_render::scene::{
    build_multi_mark_scene_with_domains, render_checkbox, render_menu, render_radio, render_slider,
    ChartData, UnsampledDomains,
};
use brightfield_render::selection::{render_committed_selection, CommittedSelection, Selected};
use brightfield_render::title::ResolvedTitles;
use vello::Scene;

/// vello's own packing for an encoded brush: premultiplied RGBA8 as one `u32`.
fn pack(c: Color) -> u32 {
    c.premultiply().to_rgba8().to_u32()
}

/// Every brush colour vello encoded into `scene`.
fn colours(scene: &Scene) -> HashSet<u32> {
    scene.encoding().draw_data.iter().copied().collect()
}

/// Four rows over two categories, plus one row whose category is NULL so the
/// NULL ink is exercised rather than assumed.
fn batch_with_nulls() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("region", DataType::Utf8, true),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0])),
            Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0, 40.0])),
            Arc::new(StringArray::from(vec![
                Some("north"),
                Some("south"),
                Some("north"),
                None,
            ])),
        ],
    )
    .expect("fixture batch")
}

/// The same shape with no colour channel at all, so the single-mark default
/// ink is the colour every dot takes.
fn batch_without_fill() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
            Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
        ],
    )
    .expect("fixture batch")
}

fn titles() -> ResolvedTitles {
    ResolvedTitles {
        x: Some("day".to_string()),
        y: Some("kWh".to_string()),
        plot: Some("Readings".to_string()),
    }
}

/// A full plot on `ink`'s canvas: background, grid, axes and their titles, the
/// plot title, the marks (categorical fill, one NULL row) and the inline colour
/// legend.
fn plot_scene(ink: ChartInk) -> Scene {
    let batch = batch_with_nulls();
    let mut channels = ChannelMap::new();
    channels.insert(Channel::X, "x".to_string());
    channels.insert(Channel::Y, "y".to_string());
    channels.insert(Channel::Fill, "region".to_string());
    let dot = DotRenderer;
    let data = ChartData {
        batch: &batch,
        channel_map: &channels,
        renderer: &dot,
        layout: ChartLayout::new(640.0, 480.0),
        view_extent: None,
        highlight: None,
        sample: None,
        beyond_frame: false,
    };
    build_multi_mark_scene_with_domains(
        &[&data],
        true,
        &titles(),
        &UnsampledDomains::default(),
        ink,
    )
    .0
}

/// A plot with no colour channel, so every dot takes the default mark ink.
fn undyed_plot_scene(ink: ChartInk) -> Scene {
    let batch = batch_without_fill();
    let mut channels = ChannelMap::new();
    channels.insert(Channel::X, "x".to_string());
    channels.insert(Channel::Y, "y".to_string());
    let dot = DotRenderer;
    let data = ChartData {
        batch: &batch,
        channel_map: &channels,
        renderer: &dot,
        layout: ChartLayout::new(640.0, 480.0),
        view_extent: None,
        highlight: None,
        sample: None,
        beyond_frame: false,
    };
    build_multi_mark_scene_with_domains(
        &[&data],
        false,
        &ResolvedTitles::default(),
        &UnsampledDomains::default(),
        ink,
    )
    .0
}

/// A committed x-interval band on `ink`'s canvas.
fn selection_scene(ink: ChartInk) -> Scene {
    let mut scales = ScaleSet::in_ink(ink);
    scales.insert(
        Channel::X,
        Scale::Linear {
            domain_min: 0.0,
            domain_max: 10.0,
            range_start: 40.0,
            range_end: 340.0,
        },
    );
    let mut scene = Scene::new();
    render_committed_selection(
        &mut scene,
        &ChartLayout::new(360.0, 300.0),
        &scales,
        &CommittedSelection {
            x: Some(Selected::Interval(2.0, 4.0)),
            y: None,
        },
    );
    scene
}

/// Every resting widget the headless dump previews, on `ink`'s canvas.
fn widget_scene(ink: ChartInk) -> Scene {
    let mut scene = Scene::new();
    render_slider(&mut scene, 0.0, 0.0, 200.0, 32.0, 0.5, ink);
    render_menu(&mut scene, 0.0, 40.0, 200.0, 28.0, "region", ink);
    render_radio(
        &mut scene,
        0.0,
        80.0,
        200.0,
        &["a".to_string(), "b".to_string()],
        Some(0),
        ink,
    );
    render_checkbox(&mut scene, 0.0, 140.0, 28.0, true, "on", ink);
    scene
}

/// Every colour any of the scenes above laid down, for one mode.
fn every_colour(ink: ChartInk) -> HashSet<u32> {
    let mut all = HashSet::new();
    for scene in [
        plot_scene(ink),
        undyed_plot_scene(ink),
        selection_scene(ink),
        widget_scene(ink),
    ] {
        all.extend(colours(&scene));
    }
    all
}

/// The paints under test, as `(name, light value, dark value)`.
///
/// The categorical slots are the first two only, and on purpose: slots 3 and 8
/// of the Harbour order are byte-identical in both published scales (`#31aa8c`,
/// `#47944c`), so asserting their absence from a dark scene would assert
/// something false about the design system rather than something true about
/// this renderer. The fixture carries two categories so the two slots it uses
/// are two that move.
fn paints() -> Vec<(&'static str, Color, Color)> {
    let l = ChartInk::LIGHT;
    let d = ChartInk::DARK;
    let mut v = vec![
        ("background", l.background, d.background),
        ("grid", l.grid, d.grid),
        ("tick", l.tick, d.tick),
        ("axis", l.axis, d.axis),
        ("label", l.label, d.label),
        ("title", l.title, d.title),
        ("legend_panel", l.legend_panel, d.legend_panel),
        ("legend_border", l.legend_border, d.legend_border),
        ("mark_default", l.mark_default, d.mark_default),
        ("null", l.null, d.null),
        ("selection_wash", l.selection_wash, d.selection_wash),
        ("selection_bound", l.selection_bound, d.selection_bound),
        ("slider_track", l.slider_track, d.slider_track),
        ("slider_thumb", l.slider_thumb, d.slider_thumb),
        ("widget_fill", l.widget_fill, d.widget_fill),
        ("widget_border", l.widget_border, d.widget_border),
        ("widget_label", l.widget_label, d.widget_label),
        (
            "widget_affordance",
            l.widget_affordance,
            d.widget_affordance,
        ),
        ("widget_active", l.widget_active, d.widget_active),
    ];
    for slot in 0..2 {
        v.push((
            if slot == 0 {
                "categorical[0]"
            } else {
                "categorical[1]"
            },
            Color::new(l.categorical[slot]),
            Color::new(d.categorical[slot]),
        ));
    }
    v
}

/// **The light scene draws every light paint.**
///
/// The control for the test below: without it, "no light paint in the dark
/// scene" would also pass on a fixture that never exercises the paint at all,
/// which is the shape of a guard that cannot fail.
#[test]
fn the_light_canvas_draws_every_light_paint() {
    let drawn = every_colour(ChartInk::LIGHT);
    for (name, light, _dark) in paints() {
        assert!(
            drawn.contains(&pack(light)),
            "{name}'s light value {light:?} is nowhere in the light scenes, so \
             the fixtures below never exercise it and its absence from the dark \
             scene would prove nothing"
        );
    }
}

/// **No light paint reaches the dark canvas, and every dark paint does.**
///
/// Revert one module to its retired `*_LIGHT` const and the first assertion
/// names it.
#[test]
fn no_light_paint_reaches_the_dark_canvas() {
    let drawn = every_colour(ChartInk::DARK);
    for (name, light, dark) in paints() {
        assert!(
            !drawn.contains(&pack(light)),
            "{name} drew its LIGHT value {light:?} on the dark canvas — that \
             module is still resolving its ink from a light token, and the \
             reader sees it against everything else that went dark"
        );
        assert!(
            drawn.contains(&pack(dark)),
            "{name}'s dark value {dark:?} is nowhere in the dark scenes"
        );
    }
}

/// **The plot's own background is the dark surface**, held separately from the
/// sweep above because it is the pixel the card is about: the chart area is the
/// largest single region on the screen and the one the analyst came to read.
#[test]
fn the_dark_plot_fills_its_area_with_the_dark_surface() {
    let drawn = colours(&plot_scene(ChartInk::DARK));
    assert!(
        drawn.contains(&pack(ChartInk::DARK.background)),
        "the dark plot never fills with the dark chart surface"
    );
    assert!(
        !drawn.contains(&pack(ChartInk::LIGHT.background)),
        "the dark plot fills with the LIGHT chart surface — the white slab"
    );
}
