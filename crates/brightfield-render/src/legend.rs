//! Colour legend rendering — draws colour-to-value mapping swatches
//! with labels when a mark encodes a data column as fill colour.

use kurbo::{Affine, Rect};
use peniko::{Color, Fill};
use vello::Scene;

use crate::axis::format_number;
use crate::layout::ChartLayout;
use crate::scale::Scale;
use crate::text::{draw_text, measure_width, TextAnchor, LABEL_COLOUR, LABEL_SIZE};

/// Swatch size in pixels.
const SWATCH_SIZE: f64 = 12.0;

/// Vertical spacing between legend entries.
const ENTRY_SPACING: f64 = 18.0;

/// Inset of the legend panel from the plot-area corner.
const LEGEND_INSET: f64 = 8.0;

/// Padding inside the legend panel.
const LEGEND_PADDING: f64 = 6.0;

/// Gap between a swatch and its label.
const LEGEND_LABEL_GAP: f64 = 4.0;

/// Gradient bar width in pixels (continuous legend).
const BAR_WIDTH: f64 = 14.0;

/// Gradient bar height in pixels (continuous legend).
const BAR_HEIGHT: f64 = 96.0;

/// Number of stacked quads sampling the ramp down the bar. Enough to read as a
/// smooth gradient without a gradient-brush dependency.
const BAR_SAMPLES: usize = 48;

/// The pixel size `(width, height)` a colour legend panel needs for the given
/// scale, or `None` for a non-colour / empty scale. Dispatches on the scale
/// kind: a categorical [`Scale::Colour`] sizes a swatch column, a continuous
/// [`Scale::Sequential`] sizes a gradient bar.
#[must_use]
pub fn colour_legend_size(colour_scale: &Scale) -> Option<(f64, f64)> {
    match colour_scale {
        Scale::Colour { .. } => swatch_legend_size(colour_scale),
        Scale::Sequential { .. } => sequential_legend_size(colour_scale),
        _ => None,
    }
}

/// The pixel size of the swatch column for a categorical [`Scale::Colour`].
fn swatch_legend_size(colour_scale: &Scale) -> Option<(f64, f64)> {
    let categories = match colour_scale {
        Scale::Colour { categories, .. } if !categories.is_empty() => categories,
        _ => return None,
    };
    let max_label = categories
        .iter()
        .map(|c| measure_width(c, LABEL_SIZE))
        .fold(0.0_f64, f64::max);
    let n = categories.len() as f64;
    let width = LEGEND_PADDING * 2.0 + SWATCH_SIZE + LEGEND_LABEL_GAP + max_label;
    let height = LEGEND_PADDING * 2.0 + (n - 1.0) * ENTRY_SPACING + SWATCH_SIZE;
    Some((width, height))
}

/// The clickable entry rects of a categorical swatch legend whose panel
/// top-left is at `(box_x, box_y)` — one rect per category, in category
/// order. Row `i` sits at `LEGEND_PADDING + i * ENTRY_SPACING` below the
/// panel top, is `SWATCH_SIZE` tall, and spans the swatch plus the gap and
/// the label's measured extent, so clicking the category text also hits.
///
/// Pure geometry mirroring [`render_swatch_legend_at`]'s layout from the
/// same private constants, so the hit-boxes cannot drift from the drawing.
/// Empty for a non-`Colour` (or empty) scale. Card 0009 (legend
/// click-to-filter), lcf ac-02.
#[must_use]
pub fn swatch_entry_rects(box_x: f64, box_y: f64, colour_scale: &Scale) -> Vec<Rect> {
    let categories = match colour_scale {
        Scale::Colour { categories, .. } => categories,
        _ => return Vec::new(),
    };
    let legend_x = box_x + LEGEND_PADDING;
    let legend_y_start = box_y + LEGEND_PADDING;
    categories
        .iter()
        .enumerate()
        .map(|(i, cat)| {
            let y = legend_y_start + i as f64 * ENTRY_SPACING;
            let label_w = measure_width(cat, LABEL_SIZE);
            Rect::new(
                legend_x,
                y,
                legend_x + SWATCH_SIZE + LEGEND_LABEL_GAP + label_w,
                y + SWATCH_SIZE,
            )
        })
        .collect()
}

/// The pixel size `(width, height)` of the gradient bar for a continuous
/// [`Scale::Sequential`], sized to hold the bar plus its min/mid/max tick labels;
/// `None` for any other scale.
#[must_use]
pub fn sequential_legend_size(scale: &Scale) -> Option<(f64, f64)> {
    let (dmin, dmax) = match scale {
        Scale::Sequential { domain_min, domain_max, .. } => (*domain_min, *domain_max),
        _ => return None,
    };
    let max_label = [dmin, (dmin + dmax) / 2.0, dmax]
        .iter()
        .map(|v| measure_width(&format_number(*v), LABEL_SIZE))
        .fold(0.0_f64, f64::max);
    let width = LEGEND_PADDING * 2.0 + BAR_WIDTH + LEGEND_LABEL_GAP + max_label;
    let height = LEGEND_PADDING * 2.0 + BAR_HEIGHT;
    Some((width, height))
}

/// Render a colour legend for a mark's fill encoding, anchored inside the plot
/// area's top-right corner (the right margin is too narrow to hold the legend,
/// and an inside panel reads as intentional). This is the plot's *inline*
/// legend; a standalone `legend:` node uses [`render_colour_legend_at`].
pub fn render_colour_legend(
    scene: &mut Scene,
    layout: &ChartLayout,
    colour_scale: &Scale,
) {
    let Some((box_width, _)) = colour_legend_size(colour_scale) else {
        return;
    };
    let box_x = layout.plot_x_end() - box_width - LEGEND_INSET;
    let box_y = layout.plot_y_start() + LEGEND_INSET;
    render_colour_legend_at(scene, box_x, box_y, colour_scale);
}

/// Render a colour legend at `(box_x, box_y)`, dispatching on the scale kind: a
/// categorical [`Scale::Colour`] draws swatches, a continuous
/// [`Scale::Sequential`] draws a gradient bar. No-op for any other scale.
///
/// This is the single entry point both the inline and standalone paths call so
/// the swatch/bar choice lives in one place.
pub fn render_colour_legend_at(
    scene: &mut Scene,
    box_x: f64,
    box_y: f64,
    colour_scale: &Scale,
) {
    match colour_scale {
        Scale::Colour { .. } => render_swatch_legend_at(scene, box_x, box_y, colour_scale),
        Scale::Sequential { .. } => render_sequential_legend_at(scene, box_x, box_y, colour_scale),
        _ => {}
    }
}

/// Render a categorical swatch legend with its panel's top-left corner at
/// `(box_x, box_y)`. Each entry is a coloured swatch and its category label.
/// No-op for a non-colour or empty scale.
fn render_swatch_legend_at(
    scene: &mut Scene,
    box_x: f64,
    box_y: f64,
    colour_scale: &Scale,
) {
    let (categories, palette) = match colour_scale {
        Scale::Colour {
            categories,
            palette,
        } => (categories, palette),
        _ => return,
    };

    if categories.is_empty() {
        return;
    }

    let Some((box_width, box_height)) = swatch_legend_size(colour_scale) else {
        return;
    };

    // Translucent background panel + thin border for legibility over marks/grid.
    let panel = Rect::new(box_x, box_y, box_x + box_width, box_y + box_height);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::new([1.0, 1.0, 1.0, 0.85]),
        None,
        &panel,
    );
    scene.stroke(
        &kurbo::Stroke::new(0.5),
        Affine::IDENTITY,
        Color::new([0.8, 0.8, 0.8, 1.0]),
        None,
        &panel,
    );

    let legend_x = box_x + LEGEND_PADDING;
    let legend_y_start = box_y + LEGEND_PADDING;

    for (i, (cat, colour)) in categories.iter().zip(palette.iter().cycle()).enumerate() {
        let y = legend_y_start + i as f64 * ENTRY_SPACING;

        // Colour swatch.
        let swatch = Rect::new(legend_x, y, legend_x + SWATCH_SIZE, y + SWATCH_SIZE);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::new(*colour),
            None,
            &swatch,
        );

        // Entry label, vertically centred on the swatch.
        draw_text(
            scene,
            cat,
            legend_x + SWATCH_SIZE + LEGEND_LABEL_GAP,
            y + SWATCH_SIZE * 0.5 + f64::from(LABEL_SIZE) / 3.0,
            LABEL_SIZE,
            LABEL_COLOUR,
            TextAnchor::Start,
        );
    }
}

/// Render a continuous gradient-bar legend for a [`Scale::Sequential`], panel
/// top-left at `(box_x, box_y)`. The bar is a vertical ramp — high value at the
/// top — drawn as [`BAR_SAMPLES`] stacked sampled quads (no gradient-brush
/// dependency), with min / mid / max numeric tick labels beside it read from the
/// scale's domain extent. No-op for any non-Sequential scale.
pub fn render_sequential_legend_at(
    scene: &mut Scene,
    box_x: f64,
    box_y: f64,
    scale: &Scale,
) {
    let (dmin, dmax) = match scale {
        Scale::Sequential { domain_min, domain_max, .. } => (*domain_min, *domain_max),
        _ => return,
    };
    let Some((box_width, box_height)) = sequential_legend_size(scale) else {
        return;
    };

    // Translucent background panel + thin border, matching the swatch legend.
    let panel = Rect::new(box_x, box_y, box_x + box_width, box_y + box_height);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::new([1.0, 1.0, 1.0, 0.85]),
        None,
        &panel,
    );
    scene.stroke(
        &kurbo::Stroke::new(0.5),
        Affine::IDENTITY,
        Color::new([0.8, 0.8, 0.8, 1.0]),
        None,
        &panel,
    );

    let bar_x = box_x + LEGEND_PADDING;
    let bar_top = box_y + LEGEND_PADDING;
    let slice_h = BAR_HEIGHT / BAR_SAMPLES as f64;

    // Stack sampled quads top (max) → bottom (min). Each slice samples the ramp
    // at its centre; the top slice reads the ramp's high end.
    for s in 0..BAR_SAMPLES {
        let y = bar_top + s as f64 * slice_h;
        let t = 1.0 - (s as f64 + 0.5) / BAR_SAMPLES as f64;
        let value = dmin + t * (dmax - dmin);
        let quad = Rect::new(bar_x, y, bar_x + BAR_WIDTH, y + slice_h + 0.5);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::new(scale.map_continuous(value)),
            None,
            &quad,
        );
    }

    // Thin border around the bar so a light-anchored ramp reads against the panel.
    let bar = Rect::new(bar_x, bar_top, bar_x + BAR_WIDTH, bar_top + BAR_HEIGHT);
    scene.stroke(
        &kurbo::Stroke::new(0.5),
        Affine::IDENTITY,
        Color::new([0.6, 0.6, 0.6, 1.0]),
        None,
        &bar,
    );

    // Min / mid / max tick labels beside the bar (max at top, min at bottom).
    let label_x = bar_x + BAR_WIDTH + LEGEND_LABEL_GAP;
    let baseline_nudge = f64::from(LABEL_SIZE) / 3.0;
    for (frac, value) in [
        (0.0, dmax),
        (0.5, (dmin + dmax) / 2.0),
        (1.0, dmin),
    ] {
        draw_text(
            scene,
            &format_number(value),
            label_x,
            bar_top + frac * BAR_HEIGHT + baseline_nudge,
            LABEL_SIZE,
            LABEL_COLOUR,
            TextAnchor::Start,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::ChartLayout;
    use crate::scale::Scale;

    #[test]
    fn gpu_ac07_colour_legend_4_categories() {
        let layout = ChartLayout::new(800.0, 480.0);
        let colour_scale = Scale::Colour {
            categories: vec![
                "cat_a".to_string(),
                "cat_b".to_string(),
                "cat_c".to_string(),
                "cat_d".to_string(),
            ],
            palette: vec![
                [0.306, 0.475, 0.655, 1.0],
                [0.949, 0.557, 0.169, 1.0],
                [0.882, 0.341, 0.349, 1.0],
                [0.463, 0.718, 0.698, 1.0],
            ],
        };

        let mut scene = Scene::new();
        render_colour_legend(&mut scene, &layout, &colour_scale);

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "legend should produce scene content for 4 categories"
        );
    }

    #[test]
    fn gpu_ac07_legend_skips_non_colour_scale() {
        let layout = ChartLayout::new(800.0, 480.0);
        let linear_scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: 0.0,
            range_end: 500.0,
        };

        let mut scene = Scene::new();
        render_colour_legend(&mut scene, &layout, &linear_scale);

        let encoding = scene.encoding();
        assert_eq!(
            encoding.path_tags.len(),
            0,
            "legend should not render for non-colour scale"
        );
    }

    fn colour_scale_3() -> Scale {
        Scale::Colour {
            categories: vec!["a".to_string(), "bb".to_string(), "ccc".to_string()],
            palette: vec![
                [0.3, 0.4, 0.6, 1.0],
                [0.9, 0.5, 0.1, 1.0],
                [0.8, 0.3, 0.3, 1.0],
            ],
        }
    }

    // Standalone-legend hosting: the positioned variant draws content at an
    // arbitrary origin (its layout rect), independent of any plot.
    #[test]
    fn standalone_legend_at_origin_draws_content() {
        let mut scene = Scene::new();
        render_colour_legend_at(&mut scene, 640.0, 40.0, &colour_scale_3());
        assert!(
            scene.encoding().path_tags.len() > 0,
            "positioned legend should draw swatches + panel at its origin"
        );
    }

    #[test]
    fn standalone_legend_at_skips_non_colour_scale() {
        let mut scene = Scene::new();
        let linear = Scale::Linear {
            domain_min: 0.0,
            domain_max: 1.0,
            range_start: 0.0,
            range_end: 1.0,
        };
        render_colour_legend_at(&mut scene, 10.0, 10.0, &linear);
        assert_eq!(
            scene.encoding().path_tags.len(),
            0,
            "no content for a non-colour scale"
        );
    }

    #[test]
    fn colour_legend_size_scales_with_entries() {
        let (w, h) = colour_legend_size(&colour_scale_3()).expect("colour scale has a size");
        assert!(w > 0.0 && h > 0.0);
        // Height grows with entry count; 3 entries taller than 1.
        let one = Scale::Colour {
            categories: vec!["a".to_string()],
            palette: vec![[0.3, 0.4, 0.6, 1.0]],
        };
        let (_, h1) = colour_legend_size(&one).unwrap();
        assert!(h > h1, "3-entry legend is taller than 1-entry");
        assert!(colour_legend_size(&one).is_some());
    }

    // --- scs_ac06: continuous gradient-bar legend ---

    fn sequential_scale() -> Scale {
        Scale::Sequential {
            domain_min: 0.0,
            domain_max: 42.0,
            stops: vec![
                [0.0, 0.0, 0.0, 1.0],
                [0.5, 0.5, 0.5, 1.0],
                [1.0, 1.0, 1.0, 1.0],
            ],
        }
    }

    #[test]
    fn scs_ac06_sequential_legend_draws_bar_and_dispatches() {
        // The bar (sampled quads + border) produces scene content.
        let mut scene = Scene::new();
        render_sequential_legend_at(&mut scene, 40.0, 40.0, &sequential_scale());
        assert!(
            scene.encoding().path_tags.len() > 0,
            "gradient bar should draw sampled quads at its origin"
        );

        // It is a no-op for a non-Sequential scale.
        let mut scene2 = Scene::new();
        let linear = Scale::Linear {
            domain_min: 0.0,
            domain_max: 1.0,
            range_start: 0.0,
            range_end: 1.0,
        };
        render_sequential_legend_at(&mut scene2, 10.0, 10.0, &linear);
        assert_eq!(scene2.encoding().path_tags.len(), 0, "no bar for a linear scale");

        // sequential_legend_size is Some for Sequential, None for Band.
        assert!(sequential_legend_size(&sequential_scale()).is_some());
        let band = Scale::Band {
            categories: vec!["a".into()],
            range_start: 0.0,
            range_end: 10.0,
            padding: 0.1,
        };
        assert!(sequential_legend_size(&band).is_none());

        // colour_legend_size dispatches a Sequential to the bar size.
        assert_eq!(
            colour_legend_size(&sequential_scale()),
            sequential_legend_size(&sequential_scale()),
            "colour_legend_size routes Sequential to the gradient-bar size"
        );

        // The shared entry point routes a Sequential to the bar (draws content).
        let mut scene3 = Scene::new();
        render_colour_legend_at(&mut scene3, 40.0, 40.0, &sequential_scale());
        assert!(scene3.encoding().path_tags.len() > 0, "dispatch draws the bar");
    }

    // --- lcf_ac02 (card 0009): swatch entry hit-geometry ---

    /// lcf_ac02: `swatch_entry_rects` yields one rect per category at exactly
    /// the positions the layout constants place the drawn swatches — row i at
    /// padding + i*ENTRY_SPACING, SWATCH_SIZE tall, spanning swatch + gap +
    /// label extent — and is empty for Sequential/Band scales.
    #[test]
    fn lcf_ac02_swatch_entry_rects_match_rendered_layout() {
        let scale = colour_scale_3(); // categories "a", "bb", "ccc"
        let (box_x, box_y) = (640.0, 40.0);
        let rects = swatch_entry_rects(box_x, box_y, &scale);
        assert_eq!(rects.len(), 3, "one rect per category");

        for (i, (rect, cat)) in rects.iter().zip(["a", "bb", "ccc"]).enumerate() {
            // Same arithmetic as render_swatch_legend_at: the row origin.
            let x0 = box_x + LEGEND_PADDING;
            let y0 = box_y + LEGEND_PADDING + i as f64 * ENTRY_SPACING;
            assert_eq!((rect.x0, rect.y0), (x0, y0), "entry {i} origin");
            assert_eq!(rect.height(), SWATCH_SIZE, "entry {i} is swatch-height");
            assert_eq!(
                rect.width(),
                SWATCH_SIZE + LEGEND_LABEL_GAP + measure_width(cat, LABEL_SIZE),
                "entry {i} spans swatch + gap + label extent"
            );
        }
        // Wider label → wider clickable rect (the label extent is included).
        assert!(rects[2].width() > rects[0].width(), "\"ccc\" entry wider than \"a\"");

        // Rows are ENTRY_SPACING apart, so consecutive rects never overlap
        // (SWATCH_SIZE < ENTRY_SPACING) — a click resolves to at most one entry.
        assert!(rects[0].y1 < rects[1].y0, "rows are disjoint");

        // Non-Colour scales have no discrete entries: empty.
        assert!(swatch_entry_rects(0.0, 0.0, &sequential_scale()).is_empty());
        let band = Scale::Band {
            categories: vec!["a".into()],
            range_start: 0.0,
            range_end: 10.0,
            padding: 0.1,
        };
        assert!(swatch_entry_rects(0.0, 0.0, &band).is_empty());
    }
}
