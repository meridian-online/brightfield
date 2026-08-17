//! Grid line rendering — draws horizontal and vertical grid lines
//! aligned with axis ticks inside the plot area.

use kurbo::{Affine, Line, Point};
use vello::Scene;

use crate::axis::Tick;
use crate::ink::ChartInk;
use crate::layout::ChartLayout;

/// Render vertical grid lines at x-axis tick positions, in `ink`'s gridline
/// hairline for the mode the plot is drawn in.
pub fn render_x_grid(scene: &mut Scene, layout: &ChartLayout, ticks: &[Tick], ink: ChartInk) {
    let stroke = kurbo::Stroke::new(0.5);
    for tick in ticks {
        let line = Line::new(
            Point::new(tick.position, layout.plot_y_start()),
            Point::new(tick.position, layout.plot_y_end()),
        );
        scene.stroke(&stroke, Affine::IDENTITY, ink.grid, None, &line);
    }
}

/// Render horizontal grid lines at y-axis tick positions.
pub fn render_y_grid(scene: &mut Scene, layout: &ChartLayout, ticks: &[Tick], ink: ChartInk) {
    let stroke = kurbo::Stroke::new(0.5);
    for tick in ticks {
        let line = Line::new(
            Point::new(layout.plot_x_start(), tick.position),
            Point::new(layout.plot_x_end(), tick.position),
        );
        scene.stroke(&stroke, Affine::IDENTITY, ink.grid, None, &line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::axis::Tick;
    use crate::layout::ChartLayout;

    #[test]
    fn grid_lines_render() {
        let layout = ChartLayout::new(640.0, 480.0);
        let ticks = vec![
            Tick {
                value: 0.0,
                label: "0".to_string(),
                position: 100.0,
            },
            Tick {
                value: 50.0,
                label: "50".to_string(),
                position: 300.0,
            },
            Tick {
                value: 100.0,
                label: "100".to_string(),
                position: 500.0,
            },
        ];

        let mut scene = Scene::new();
        render_x_grid(&mut scene, &layout, &ticks, ChartInk::LIGHT);
        render_y_grid(&mut scene, &layout, &ticks, ChartInk::LIGHT);

        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "grid should produce scene content"
        );
    }

    /// **The gridline a dark plot draws is the dark gridline.** The scene
    /// carries its brushes as raw components, so this reads the ink the grid
    /// actually laid down rather than the ink it was handed.
    #[test]
    fn the_grid_draws_the_mode_it_is_given() {
        let layout = ChartLayout::new(640.0, 480.0);
        let ticks = vec![Tick {
            value: 0.0,
            label: "0".to_string(),
            position: 100.0,
        }];
        let drawn = |ink: ChartInk| {
            let mut scene = Scene::new();
            render_x_grid(&mut scene, &layout, &ticks, ink);
            scene.encoding().draw_data.clone()
        };
        assert_ne!(
            drawn(ChartInk::LIGHT),
            drawn(ChartInk::DARK),
            "the grid drew identical bytes in both modes, so it is not reading \
             the ink it was handed"
        );
    }
}
