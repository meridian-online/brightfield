//! Colour legend rendering — draws colour-to-value mapping swatches
//! with labels when a mark encodes a data column as fill colour.

use kurbo::{Affine, Rect};
use peniko::{Color, Fill};
use vello::Scene;

use crate::layout::ChartLayout;
use crate::scale::Scale;

/// Swatch size in pixels.
const SWATCH_SIZE: f64 = 12.0;

/// Vertical spacing between legend entries.
const ENTRY_SPACING: f64 = 18.0;

/// Legend left margin from the plot area's right edge.
const LEGEND_MARGIN: f64 = 10.0;

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

    // Position: right of plot area.
    let legend_x = layout.plot_x_end() + LEGEND_MARGIN;
    let legend_y_start = layout.plot_y_start();

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

        // Text label placeholder (proportional to label length).
        let label_x = legend_x + SWATCH_SIZE + 4.0;
        let label_width = cat.len() as f64 * 5.0;
        let label_rect = Rect::new(label_x, y, label_x + label_width, y + SWATCH_SIZE);
        // Transparent rect as text placeholder.
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::new([0.0, 0.0, 0.0, 0.0]),
            None,
            &label_rect,
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
