//! Colour legend rendering — draws colour-to-value mapping swatches
//! with labels when a mark encodes a data column as fill colour.

use kurbo::{Affine, Rect};
use peniko::{Color, Fill};
use vello::Scene;

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

/// Render a colour legend into the scene.
///
/// The legend is positioned to the right of the plot area within the
/// right margin area. Each entry shows a coloured swatch and a text
/// label placeholder.
pub fn render_colour_legend(
    scene: &mut Scene,
    layout: &ChartLayout,
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

    // Size the panel to the widest label so nothing clips.
    let max_label = categories
        .iter()
        .map(|c| measure_width(c, LABEL_SIZE))
        .fold(0.0_f64, f64::max);
    let n = categories.len() as f64;
    let box_width = LEGEND_PADDING * 2.0 + SWATCH_SIZE + LEGEND_LABEL_GAP + max_label;
    let box_height = LEGEND_PADDING * 2.0 + (n - 1.0) * ENTRY_SPACING + SWATCH_SIZE;

    // Anchor inside the plot area's top-right corner (the right margin is too
    // narrow to hold the legend, and an inside panel reads as intentional).
    let box_x = layout.plot_x_end() - box_width - LEGEND_INSET;
    let box_y = layout.plot_y_start() + LEGEND_INSET;

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
}
