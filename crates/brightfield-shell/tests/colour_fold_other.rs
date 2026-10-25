//! **A column with more categories than the Harbour palette has hues never
//! draws two named categories in one colour: the first seven keep their own
//! hues, and the rest share the eighth under one "Other".**
//!
//! The design system's colour guideline assigns the Harbour hues in fixed order
//! and never cycles them, since the order is what keeps neighbouring hues apart
//! for colour-blind readers. Before this, the ninth category started the
//! palette again and drew in the first category's blue.
//!
//! These compose a whole plot and read the answer off the raster the window
//! and the PNG export draw, and off the legend the page is handed.

use brightfield_render::channel::Channel;
use brightfield_render::ink::ChartInk;
use brightfield_render::scale::OTHER_CATEGORY;
use brightfield_render::VelloRenderer;
use brightfield_shell::legend::LegendSpec;
use brightfield_shell::pipeline::{compose_spec_str, Composed};

/// Nine bars, one per category of `s`, each filled through the colour scale.
fn nine_bars(attrs: &str) -> Composed {
    let rows: String = (0..9)
        .map(|s| format!("    - {{ s: s{s}, v: 10 }}\n"))
        .collect();
    let source = format!(
        "data:\n  t:\n{rows}plot:\n  - mark: barY\n    data: {{ from: t }}\n    x: s\n    y: v\n    fill: s\nwidth: 640\nheight: 300\n{attrs}"
    );
    compose_spec_str(&source, None)
        .unwrap_or_else(|e| panic!("the spec must compose: {e}\n{source}"))
}

/// The colour at the centre of each category's bar, in category order, read
/// off the raster.
fn bar_colours(c: &Composed) -> Vec<[u8; 4]> {
    let renderer = VelloRenderer::new();
    let px = renderer
        .lock()
        .expect("renderer poisoned")
        .render_to_pixels(&c.scene, c.width, c.height);
    let img = image::RgbaImage::from_raw(c.width, c.height, px).expect("pixel buffer size");
    let plot = c.plots.first().expect("one plot");
    let x_scale = plot.scales.get(Channel::X).expect("an x scale");
    let y = plot.rect.y + (plot.layout.plot_y_start() + plot.layout.plot_y_end()) / 2.0;
    (0..9)
        .map(|s| {
            let x = plot.rect.x
                + x_scale
                    .map_category(&format!("s{s}"))
                    .unwrap_or_else(|| panic!("no band for s{s}"));
            img.get_pixel(x.round() as u32, y.round() as u32).0
        })
        .collect()
}

fn to_bytes(c: [f32; 4]) -> [u8; 4] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Within one 8-bit step on every channel, which is rounding and nothing else.
fn same(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 1)
}

#[test]
fn nine_bars_draw_seven_hues_and_an_other_in_the_eighth() {
    let composed = nine_bars("");
    let harbour: Vec<[u8; 4]> = ChartInk::LIGHT
        .categorical
        .iter()
        .copied()
        .map(to_bytes)
        .collect();
    let drawn = bar_colours(&composed);
    for (s, colour) in drawn.iter().enumerate() {
        let slot = s.min(7);
        assert!(
            same(*colour, harbour[slot]),
            "bar s{s} drew {colour:?}, not Harbour slot {} {:?}",
            slot + 1,
            harbour[slot]
        );
    }
    assert!(
        !same(drawn[8], harbour[0]),
        "the ninth bar must not wear the first category's colour"
    );
}

#[test]
fn the_legend_names_seven_categories_and_one_other() {
    let composed = nine_bars("");
    let Some(LegendSpec::Categorical { entries }) =
        LegendSpec::from_scales(&composed.plots[0].scales)
    else {
        panic!("no categorical legend");
    };
    let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(
        labels,
        ["s0", "s1", "s2", "s3", "s4", "s5", "s6", OTHER_CATEGORY]
    );
    let colours: Vec<[f32; 4]> = entries.iter().map(|e| e.colour).collect();
    assert_eq!(colours, ChartInk::LIGHT.categorical.to_vec());
    // Each row wears what its marks wear.
    let scale = composed.plots[0].scales.get(Channel::Fill).expect("fill");
    for s in 7..9 {
        assert_eq!(
            scale.map_colour(&format!("s{s}")),
            Some(entries[7].colour),
            "s{s}"
        );
    }
}

/// Eight categories fit the palette and nothing folds.
#[test]
fn eight_bars_keep_eight_names() {
    let rows: String = (0..8)
        .map(|s| format!("    - {{ s: s{s}, v: 10 }}\n"))
        .collect();
    let source = format!(
        "data:\n  t:\n{rows}plot:\n  - mark: barY\n    data: {{ from: t }}\n    x: s\n    y: v\n    fill: s\nwidth: 640\nheight: 300\n"
    );
    let composed = compose_spec_str(&source, None).expect("composes");
    let Some(LegendSpec::Categorical { entries }) =
        LegendSpec::from_scales(&composed.plots[0].scales)
    else {
        panic!("no categorical legend");
    };
    assert_eq!(entries.len(), 8);
    assert!(entries.iter().all(|e| e.label != OTHER_CATEGORY));
}

/// A plot that writes its own colours chose them: a short range starts again
/// from its first colour, as Mosaic draws it, and the legend lists every
/// category.
#[test]
fn a_written_colour_range_cycles_and_lists_every_category() {
    let composed = nine_bars("colorRange: [\"#ff0000\", \"#00ff00\", \"#0000ff\"]\n");
    let drawn = bar_colours(&composed);
    let written = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];
    for (s, colour) in drawn.iter().enumerate() {
        assert!(same(*colour, written[s % 3]), "bar s{s} drew {colour:?}");
    }
    let Some(LegendSpec::Categorical { entries }) =
        LegendSpec::from_scales(&composed.plots[0].scales)
    else {
        panic!("no categorical legend");
    };
    assert_eq!(entries.len(), 9);
}
