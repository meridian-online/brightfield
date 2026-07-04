//! Axis rendering — tick computation, tick marks, labels, and axis lines.
//!
//! Tick computation is a pure function: compute_ticks(scale, target_count) ->
//! Vec<Tick>. The scene builder draws ticks as lines and labels as text.

use kurbo::{Affine, Line, Point};
use peniko::Color;
use vello::Scene;

use crate::layout::ChartLayout;
use crate::scale::Scale;
use crate::text::{draw_text, TextAnchor, LABEL_COLOUR, LABEL_SIZE};

/// A computed tick mark with its position and label.
#[derive(Debug, Clone)]
pub struct Tick {
    /// The data value this tick represents.
    pub value: f64,
    /// Human-readable label string.
    pub label: String,
    /// Pixel position along the axis.
    pub position: f64,
}

/// Default tick colour.
const TICK_COLOUR: Color = Color::new([0.2, 0.2, 0.2, 1.0]);

/// Axis line colour.
const AXIS_COLOUR: Color = Color::new([0.2, 0.2, 0.2, 1.0]);

/// Tick mark length in pixels.
const TICK_LENGTH: f64 = 5.0;

/// Compute ticks for a scale.
///
/// Returns tick marks with positions and labels appropriate for the scale type.
pub fn compute_ticks(scale: &Scale, target_count: usize) -> Vec<Tick> {
    match scale {
        Scale::Linear {
            domain_min,
            domain_max,
            range_start,
            range_end,
        } => compute_linear_ticks(*domain_min, *domain_max, *range_start, *range_end, target_count),
        Scale::Band {
            categories,
            range_start,
            range_end,
            padding,
        } => compute_band_ticks(categories, *range_start, *range_end, *padding),
        Scale::Time {
            domain_min_us,
            domain_max_us,
            range_start,
            range_end,
        } => compute_time_ticks(*domain_min_us, *domain_max_us, *range_start, *range_end, target_count),
        // Colour ramps (categorical or sequential) have no positional axis ticks.
        Scale::Colour { .. } | Scale::Sequential { .. } => Vec::new(),
    }
}

fn compute_linear_ticks(
    domain_min: f64,
    domain_max: f64,
    range_start: f64,
    range_end: f64,
    target_count: usize,
) -> Vec<Tick> {
    let span = domain_max - domain_min;
    if span.abs() < f64::EPSILON || target_count == 0 {
        return vec![];
    }

    let step = nice_step(span, target_count);
    let first = (domain_min / step).ceil() * step;

    let mut ticks = Vec::new();
    let mut value = first;
    while value <= domain_max + step * 0.001 {
        let t = (value - domain_min) / span;
        let position = range_start + t * (range_end - range_start);
        let label = format_number(value);
        ticks.push(Tick {
            value,
            label,
            position,
        });
        value += step;
    }
    ticks
}

fn compute_band_ticks(
    categories: &[String],
    range_start: f64,
    range_end: f64,
    padding: f64,
) -> Vec<Tick> {
    let n = categories.len() as f64;
    if n == 0.0 {
        return vec![];
    }
    let total = range_end - range_start;
    let band = total / n;

    categories
        .iter()
        .enumerate()
        .map(|(i, cat)| {
            let centre = range_start + band * (padding / 2.0) + band * i as f64 + band * (1.0 - padding) / 2.0;
            Tick {
                value: i as f64,
                label: cat.clone(),
                position: centre,
            }
        })
        .collect()
}

fn compute_time_ticks(
    domain_min_us: i64,
    domain_max_us: i64,
    range_start: f64,
    range_end: f64,
    target_count: usize,
) -> Vec<Tick> {
    let span_us = (domain_max_us - domain_min_us) as f64;
    if span_us.abs() < f64::EPSILON || target_count == 0 {
        return vec![];
    }

    let step_us = nice_step(span_us, target_count);
    let first = ((domain_min_us as f64 / step_us).ceil() * step_us) as i64;

    let mut ticks = Vec::new();
    let mut value_us = first;
    while value_us <= domain_max_us {
        let t = (value_us - domain_min_us) as f64 / span_us;
        let position = range_start + t * (range_end - range_start);
        // Format as seconds for simplicity in v1.
        let seconds = value_us as f64 / 1_000_000.0;
        let label = format!("{seconds:.1}s");
        ticks.push(Tick {
            value: value_us as f64,
            label,
            position,
        });
        value_us += step_us as i64;
    }
    ticks
}

/// Compute a "nice" step size for tick spacing.
fn nice_step(span: f64, target_count: usize) -> f64 {
    let raw_step = span / target_count as f64;
    let magnitude = 10_f64.powf(raw_step.log10().floor());
    let residual = raw_step / magnitude;

    let nice = if residual <= 1.5 {
        1.0
    } else if residual <= 3.5 {
        2.0
    } else if residual <= 7.5 {
        5.0
    } else {
        10.0
    };

    nice * magnitude
}

/// Format a number for tick labels.
pub(crate) fn format_number(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

/// Render the x-axis into the scene.
pub fn render_x_axis(scene: &mut Scene, layout: &ChartLayout, ticks: &[Tick]) {
    let y = layout.plot_y_end();
    let stroke = kurbo::Stroke::new(1.0);

    // Axis line.
    let axis_line = Line::new(
        Point::new(layout.plot_x_start(), y),
        Point::new(layout.plot_x_end(), y),
    );
    scene.stroke(&stroke, Affine::IDENTITY, AXIS_COLOUR, None, &axis_line);

    // Tick marks.
    for tick in ticks {
        let tick_line = Line::new(
            Point::new(tick.position, y),
            Point::new(tick.position, y + TICK_LENGTH),
        );
        scene.stroke(&stroke, Affine::IDENTITY, TICK_COLOUR, None, &tick_line);

        // Tick label, centred under the tick mark.
        draw_text(
            scene,
            &tick.label,
            tick.position,
            y + TICK_LENGTH + f64::from(LABEL_SIZE),
            LABEL_SIZE,
            LABEL_COLOUR,
            TextAnchor::Middle,
        );
    }
}

/// Render the y-axis into the scene.
pub fn render_y_axis(scene: &mut Scene, layout: &ChartLayout, ticks: &[Tick]) {
    let x = layout.plot_x_start();
    let stroke = kurbo::Stroke::new(1.0);

    // Axis line.
    let axis_line = Line::new(
        Point::new(x, layout.plot_y_start()),
        Point::new(x, layout.plot_y_end()),
    );
    scene.stroke(&stroke, Affine::IDENTITY, AXIS_COLOUR, None, &axis_line);

    // Tick marks and labels.
    for tick in ticks {
        let tick_line = Line::new(
            Point::new(x - TICK_LENGTH, tick.position),
            Point::new(x, tick.position),
        );
        scene.stroke(&stroke, Affine::IDENTITY, TICK_COLOUR, None, &tick_line);

        // Label, right-aligned in the left margin and vertically centred on the tick.
        draw_text(
            scene,
            &tick.label,
            x - TICK_LENGTH - 3.0,
            tick.position + f64::from(LABEL_SIZE) / 3.0,
            LABEL_SIZE,
            LABEL_COLOUR,
            TextAnchor::End,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::ChartLayout;
    use crate::scale::Scale;

    #[test]
    fn gpu_ac06_compute_linear_ticks() {
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: 40.0,
            range_end: 600.0,
        };
        let ticks = compute_ticks(&scale, 5);
        assert!(!ticks.is_empty(), "should produce ticks");
        // All tick positions should be within the range.
        for tick in &ticks {
            assert!(tick.position >= 40.0 - 0.1, "tick at {:.1} below range start", tick.position);
            assert!(tick.position <= 600.0 + 0.1, "tick at {:.1} above range end", tick.position);
        }
        // Labels should be numeric strings.
        for tick in &ticks {
            assert!(!tick.label.is_empty(), "tick should have a label");
        }
    }

    #[test]
    fn gpu_ac06_compute_band_ticks() {
        let scale = Scale::Band {
            categories: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            range_start: 40.0,
            range_end: 600.0,
            padding: 0.1,
        };
        let ticks = compute_ticks(&scale, 3);
        assert_eq!(ticks.len(), 3, "should produce one tick per category");
        assert_eq!(ticks[0].label, "a");
        assert_eq!(ticks[1].label, "b");
        assert_eq!(ticks[2].label, "c");
        // Positions should be in order.
        assert!(ticks[0].position < ticks[1].position);
        assert!(ticks[1].position < ticks[2].position);
    }

    #[test]
    fn gpu_ac06_compute_time_ticks() {
        let scale = Scale::Time {
            domain_min_us: 1_000_000,
            domain_max_us: 4_000_000,
            range_start: 40.0,
            range_end: 600.0,
        };
        let ticks = compute_ticks(&scale, 5);
        assert!(!ticks.is_empty(), "should produce time ticks");
        for tick in &ticks {
            assert!(tick.position >= 40.0 - 0.1);
            assert!(tick.position <= 600.0 + 0.1);
            assert!(tick.label.contains('s'), "time tick label should contain 's': {}", tick.label);
        }
    }

    #[test]
    fn gpu_ac06_render_x_axis_produces_scene_content() {
        let layout = ChartLayout::new(640.0, 480.0);
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: layout.plot_x_start(),
            range_end: layout.plot_x_end(),
        };
        let ticks = compute_ticks(&scale, 5);

        let mut scene = Scene::new();
        render_x_axis(&mut scene, &layout, &ticks);

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "x-axis should produce scene content"
        );
    }

    #[test]
    fn gpu_ac06_render_y_axis_produces_scene_content() {
        let layout = ChartLayout::new(640.0, 480.0);
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: layout.plot_y_end(),
            range_end: layout.plot_y_start(),
        };
        let ticks = compute_ticks(&scale, 5);

        let mut scene = Scene::new();
        render_y_axis(&mut scene, &layout, &ticks);

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "y-axis should produce scene content"
        );
    }

    #[test]
    fn gpu_ac06_nice_step_produces_human_readable_intervals() {
        // 0-100 with ~5 ticks should give step=20
        let step = nice_step(100.0, 5);
        assert!((step - 20.0).abs() < f64::EPSILON, "expected step 20, got {step}");

        // 0-1000 with ~5 ticks should give step=200
        let step = nice_step(1000.0, 5);
        assert!((step - 200.0).abs() < f64::EPSILON, "expected step 200, got {step}");
    }
}
