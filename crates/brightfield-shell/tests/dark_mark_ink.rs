//! **A dark hexgrid and a dark basemap, measured off the pixels.**
//!
//! `brightfield-render`'s `ink.rs` holds the contrast the two dark strokes were
//! CHOSEN for, and `tests/mode_blind_ink.rs` holds that every mark repaints when
//! the mode changes. Both read colours out of a `ChartInk` or out of a vello
//! encoding. Neither looks at a picture, and this defect was invisible in
//! exactly that gap: `GEO_STROKE_COLOUR` was a perfectly well-formed `Color`
//! that a spec reached, a renderer drew, and a reader could not see.
//!
//! So this file composes two specs through the whole pipeline — parse, lower,
//! execute, compose, rasterise through the same `VelloRenderer` the window and
//! the PNG export use — in `Mode::Dark`, and reads the answer off the raster.
//!
//! # Why the measurement is cropped to the plot frame
//!
//! This is the line that decides whether the file measures the mark or
//! something near it. Over the WHOLE image the brightest pixel of a dataless
//! hexgrid is not its mesh: it is the axis tick label, `ChartInk::DARK.label` at
//! 3.54:1, sitting in the margin. A peak-contrast reading over the full raster
//! would report 3.54:1 for a mesh drawing anything at all — including the
//! literal — because the label is brighter than either. Cropping to
//! `ChartLayout`'s plot rect, which the composition hands back on its
//! `PlotHandle`, leaves the marks and the chart surface. `frame_pixels` refuses
//! a crop under ten thousand pixels, so a rect that stopped being the plot
//! fails both tests here rather than measuring a region nobody chose.
//!
//! # What is measured, and what it is measured against
//!
//! Inside that rect: the SURFACE is the modal pixel, and the mark's ink is the
//! pixel furthest from it by `contrast_ratio` — the crate's one implementation,
//! so the ratio a token was chosen for and the ratio a picture achieves are the
//! same arithmetic. A 0.75px hairline is entirely anti-aliased, so no pixel
//! carries the stroke colour at full strength: the measured peak is 2.90:1
//! where the token is 2.95:1 and 15.41:1 where the token is 15.84:1. That gap
//! is the reason this asserts RATIOS against derived bounds rather than pixel
//! equality with a token.
//!
//! # This needs a GPU, and is not `#[ignore]`d
//!
//! `tests/snapshot.rs`, `tests/sorted_bar_ink.rs` and
//! `brightfield-render/tests/frame_ink.rs` all rasterise through a real wgpu
//! adapter and none of them is ignored or feature-gated. `test.yml` runs the
//! whole suite on `macos-15` precisely because it is the hosted runner with an
//! adapter, so these assertions run in CI. This file follows that decision
//! rather than opting itself out.

use std::collections::HashMap;

use brightfield_render::ink::{contrast_ratio, ChartInk};
use brightfield_render::VelloRenderer;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::{compose_spec_in_mode, Composed};
use peniko::Color;

/// A dataless hexgrid — the standalone mesh, drawn on the plot-corner pixel
/// lattice. No `title:` and no `meta:`, so the only ink inside the frame is the
/// mesh.
const HEXGRID: &str = "plot:\n  - mark: hexgrid\nwidth: 400\nheight: 300\n";

/// A stroke-only basemap: two inline GeoJSON polygons and NO `fill:` channel,
/// which is the branch that takes the outline stroke. A `fill:` would make this
/// a choropleth and never reach the stroke at all — the case the card is about
/// is the one with nothing else on the page.
const GEO: &str = concat!(
    "data:\n  regions:\n",
    "    - { geom: '{\"type\":\"Polygon\",\"coordinates\":",
    "[[[0,0],[12,0],[12,14],[0,14],[0,0]]]}' }\n",
    "    - { geom: '{\"type\":\"Polygon\",\"coordinates\":",
    "[[[12,0],[26,3],[24,27],[12,30],[12,0]]]}' }\n",
    "plot:\n  - mark: geo\n    data: { from: regions }\nwidth: 400\nheight: 300\n"
);

/// How many pixels must be at least half-covered by the mark for the reading to
/// be a drawn feature rather than one stray sample on an anti-aliased edge.
const MIN_INKED: usize = 200;

/// The largest per-channel gap between a raster pixel and a colour, in 8-bit
/// steps.
///
/// Coverage is counted with THIS rather than with `contrast_ratio`, and the
/// difference matters: a contrast ratio is perceptual and its midpoint sits at
/// very different coverages on a light and a dark surface, so "pixels at half
/// the peak RATIO" counts 158 of the same outline in light and 656 in dark. The
/// anti-aliasing that produced them is identical. Distance is linear in
/// coverage, so the two modes count the same feature the same way and the
/// control below is comparing like with like.

/// Compose `source` in `mode`. Written to a file because `compose_spec_in_mode`
/// is the only mode-aware entry point, and inventing a second one to avoid a
/// temp file would put this test on a path the app does not use.
fn compose(source: &str, mode: Mode) -> Composed {
    let dir = std::env::temp_dir().join("bf-dark-mark-ink");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join(format!("f{:x}-{mode:?}.yaml", source.len()));
    std::fs::write(&path, source).expect("write fixture");
    compose_spec_in_mode(path.to_str().expect("utf-8 path"), mode)
        .unwrap_or_else(|e| panic!("the fixture must compose: {e}\n{source}"))
}

/// The composed scene as pixels, through the renderer the window uses.
fn raster(c: &Composed) -> image::RgbaImage {
    let renderer = VelloRenderer::new();
    let px = renderer
        .lock()
        .expect("renderer poisoned")
        .render_to_pixels(&c.scene, c.width, c.height);
    image::RgbaImage::from_raw(c.width, c.height, px).expect("vello pixel buffer size mismatch")
}

/// The pixels inside the first plot's frame — see the module comment for why
/// this crop is the whole point.
fn frame_pixels(img: &image::RgbaImage, c: &Composed) -> Vec<[u8; 4]> {
    let h = c.plots.first().expect("the fixture places one plot");
    let (x0, x1) = (
        h.rect.x + h.layout.plot_x_start(),
        h.rect.x + h.layout.plot_x_end(),
    );
    let (y0, y1) = (
        h.rect.y + h.layout.plot_y_start(),
        h.rect.y + h.layout.plot_y_end(),
    );
    let mut out = Vec::new();
    for y in (y0.ceil() as u32)..(y1.floor() as u32).min(img.height()) {
        for x in (x0.ceil() as u32)..(x1.floor() as u32).min(img.width()) {
            out.push(img.get_pixel(x, y).0);
        }
    }
    assert!(
        out.len() > 10_000,
        "the plot frame cropped to {} px — the crop is wrong, and every reading \
         below is about the wrong region",
        out.len()
    );
    out
}

fn distance(p: [u8; 4], c: Color) -> u32 {
    let want = c.to_rgba8().to_u8_array();
    (0..3)
        .map(|i| u32::from(p[i].abs_diff(want[i])))
        .max()
        .unwrap_or(0)
}

/// What one render says: the surface it laid down, the strongest ink against
/// it, and how much of the frame reaches half that strength.
struct Reading {
    surface: Color,
    peak: Color,
    peak_ratio: f64,
    inked: usize,
}

fn read(source: &str, mode: Mode) -> Reading {
    let c = compose(source, mode);
    let img = raster(&c);
    let px = frame_pixels(&img, &c);

    let mut counts: HashMap<[u8; 4], usize> = HashMap::new();
    for p in &px {
        *counts.entry(*p).or_default() += 1;
    }
    let modal = *counts
        .iter()
        .max_by_key(|(_, n)| **n)
        .expect("a non-empty frame")
        .0;
    let surface = Color::from_rgba8(modal[0], modal[1], modal[2], 255);

    let mut peak = surface;
    let mut peak_ratio = 1.0_f64;
    for p in counts.keys() {
        let c = Color::from_rgba8(p[0], p[1], p[2], 255);
        let r = contrast_ratio(c, surface);
        if r > peak_ratio {
            peak_ratio = r;
            peak = c;
        }
    }
    let peak_distance = distance(peak.to_rgba8().to_u8_array(), surface);
    let inked = px
        .iter()
        .filter(|p| distance(**p, surface) * 2 >= peak_distance)
        .count();

    Reading {
        surface,
        peak,
        peak_ratio,
        inked,
    }
}

/// The surface a mode's render lays down is that mode's chart surface. Every
/// ratio below is measured against it, so a render that photographed the wrong
/// mode would make all of them meaningless rather than wrong.
fn assert_surface(r: &Reading, mode: Mode, what: &str) {
    let want = ChartInk::for_mode(matches!(mode, Mode::Dark)).background;
    assert_eq!(
        r.surface.to_rgba8().to_u32(),
        want.to_rgba8().to_u32(),
        "{what} in {mode:?} filled its plot with {:?}, not the mode's chart \
         surface {want:?}",
        r.surface
    );
}

/// **A dark basemap outline is legible against the dark chart surface.**
///
/// The card's number: before this, `mark: geo` with no `fill:` drew #262626 on
/// #161413 — 1.21:1, painted and invisible. The floor is WCAG AAA because a
/// stroke-only basemap is the whole of what the reader came to see; there is no
/// fill under it and no other ink on the plot.
#[test]
fn the_dark_basemap_outline_clears_wcag_aaa_on_the_page() {
    let dark = read(GEO, Mode::Dark);
    assert_surface(&dark, Mode::Dark, "the basemap");
    assert!(
        dark.peak_ratio >= 7.0,
        "the dark basemap's strongest ink is {:?} at {:.2}:1 against the dark \
         chart surface. The literal this replaced measured 1.21:1 — a stroke \
         that is drawn and cannot be seen",
        dark.peak,
        dark.peak_ratio
    );
    assert!(
        dark.inked >= MIN_INKED,
        "only {} px of the dark plot frame reach half the peak contrast, so the \
         reading above is one stray anti-aliased sample rather than an outline",
        dark.inked
    );

    // The control. Without it the assertions above are also satisfied by a
    // basemap that stopped drawing in light, and by a measurement whose crop or
    // arithmetic is wrong in both modes equally.
    let light = read(GEO, Mode::Light);
    assert_surface(&light, Mode::Light, "the basemap");
    assert!(
        light.peak_ratio >= 7.0 && light.inked >= MIN_INKED,
        "the LIGHT basemap measures {:.2}:1 over {} px — the light values ship \
         today, so a light reading that moved means this measurement changed, \
         not the ink",
        light.peak_ratio,
        light.inked
    );
}

/// **A dark hexgrid mesh is visible without out-shouting the data it sits
/// under.**
///
/// Both bounds are derived from other paints rather than typed. The floor is
/// what the same mesh achieves in light, so the dark mesh cannot be fainter
/// than the one that ships. The ceiling is the default mark ink's contrast,
/// because a decorative lattice that reads louder than data is the same defect
/// the other way round — and it is the one the literal committed: #b8b8b8 on
/// the dark surface is 9.26:1 against the mark ink's 4.75:1.
#[test]
fn the_dark_hexgrid_mesh_is_visible_and_stays_under_the_data_ink() {
    let dark = read(HEXGRID, Mode::Dark);
    let light = read(HEXGRID, Mode::Light);
    assert_surface(&dark, Mode::Dark, "the hexgrid");
    assert_surface(&light, Mode::Light, "the hexgrid");

    assert!(
        light.inked >= MIN_INKED,
        "the LIGHT mesh inks only {} px, so the dark reading has no reference",
        light.inked
    );
    assert!(
        dark.inked >= MIN_INKED,
        "the dark mesh inks only {} px of its plot frame",
        dark.inked
    );

    assert!(
        dark.peak_ratio >= light.peak_ratio,
        "the dark mesh measures {:.2}:1 against the dark surface, fainter than \
         the light mesh's {:.2}:1 against the light one — the mode moved and \
         the mesh got harder to see",
        dark.peak_ratio,
        light.peak_ratio
    );

    let data_ink = contrast_ratio(ChartInk::DARK.mark_default, dark.surface);
    assert!(
        dark.peak_ratio < data_ink,
        "the dark mesh measures {:.2}:1 against the default mark ink's \
         {data_ink:.2}:1. Scaffolding is not allowed to out-shout data, and a \
         mesh above this line is what the light literal drew on a dark canvas",
        dark.peak_ratio
    );
}
