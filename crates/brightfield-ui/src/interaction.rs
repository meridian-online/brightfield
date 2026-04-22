//! Interaction state — tracks brush rect and hovered point for overlay
//! rendering without triggering DuckDB re-query.
//!
//! Two-tier interaction model:
//! - Immediate: overlay renders during drag (brush rect, highlight) — pure GPU, no I/O
//! - Deferred: DuckDB re-query fires on brush release via session.update_param()

use kurbo::{Affine, Point, Rect};
use peniko::{Color, Fill};
use vello::Scene;

/// Current interaction mode.
#[derive(Debug, Clone)]
pub enum InteractionState {
    /// No active interaction.
    Idle,
    /// User is dragging a brush selection.
    Brushing {
        /// Start point of the brush in chart coordinates.
        start: Point,
        /// Current drag point in chart coordinates.
        current: Point,
    },
    /// User is hovering over a data point.
    Hovering {
        /// The hovered point in chart coordinates.
        point: Point,
    },
}

/// Brush overlay colour (semi-transparent blue).
const BRUSH_COLOUR: Color = Color::new([0.306, 0.475, 0.655, 0.251]);

/// Brush border colour.
const BRUSH_BORDER_COLOUR: Color = Color::new([0.306, 0.475, 0.655, 0.753]);

/// Hover highlight radius.
const HOVER_RADIUS: f64 = 8.0;

/// Hover highlight colour (semi-transparent orange).
const HOVER_COLOUR: Color = Color::new([0.949, 0.557, 0.169, 0.376]);

impl InteractionState {
    /// Begin a brush at the given point.
    pub fn start_brush(point: Point) -> Self {
        Self::Brushing {
            start: point,
            current: point,
        }
    }

    /// Update the brush's current drag position.
    pub fn update_brush(&mut self, current: Point) {
        if let Self::Brushing {
            current: ref mut c, ..
        } = self
        {
            *c = current;
        }
    }

    /// Get the brush rectangle (if brushing).
    pub fn brush_rect(&self) -> Option<Rect> {
        match self {
            Self::Brushing { start, current } => {
                let x0 = start.x.min(current.x);
                let y0 = start.y.min(current.y);
                let x1 = start.x.max(current.x);
                let y1 = start.y.max(current.y);
                Some(Rect::new(x0, y0, x1, y1))
            }
            _ => None,
        }
    }

    /// Render the interaction overlay into the scene.
    ///
    /// This is pure rendering — no DuckDB query, no I/O. The overlay
    /// renders immediately during drag at frame rate.
    pub fn render_overlay(&self, scene: &mut Scene) {
        match self {
            Self::Idle => {}
            Self::Brushing { start, current } => {
                let rect = Rect::new(
                    start.x.min(current.x),
                    start.y.min(current.y),
                    start.x.max(current.x),
                    start.y.max(current.y),
                );
                // Semi-transparent fill.
                scene.fill(Fill::NonZero, Affine::IDENTITY, BRUSH_COLOUR, None, &rect);
                // Border stroke.
                let stroke = kurbo::Stroke::new(1.5);
                scene.stroke(&stroke, Affine::IDENTITY, BRUSH_BORDER_COLOUR, None, &rect);
            }
            Self::Hovering { point } => {
                let circle = kurbo::Circle::new(*point, HOVER_RADIUS);
                scene.fill(Fill::NonZero, Affine::IDENTITY, HOVER_COLOUR, None, &circle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Point;

    #[test]
    fn gpu_ac10_brush_state_tracks_rect() {
        let mut state = InteractionState::start_brush(Point::new(10.0, 20.0));
        state.update_brush(Point::new(100.0, 200.0));

        let rect = state.brush_rect().expect("should have brush rect");
        assert!((rect.x0 - 10.0).abs() < f64::EPSILON);
        assert!((rect.y0 - 20.0).abs() < f64::EPSILON);
        assert!((rect.x1 - 100.0).abs() < f64::EPSILON);
        assert!((rect.y1 - 200.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gpu_ac10_brush_overlay_renders_without_query() {
        let state = InteractionState::Brushing {
            start: Point::new(10.0, 20.0),
            current: Point::new(100.0, 200.0),
        };

        let mut scene = Scene::new();
        state.render_overlay(&mut scene);

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "brush overlay should produce scene content"
        );
        // Key assertion: this test proves overlay renders without any engine
        // dependency — no DuckDB, no execute_mark call. Pure scene rendering.
    }

    #[test]
    fn gpu_ac10_hover_overlay_renders() {
        let state = InteractionState::Hovering {
            point: Point::new(50.0, 50.0),
        };

        let mut scene = Scene::new();
        state.render_overlay(&mut scene);

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "hover overlay should produce scene content"
        );
    }

    #[test]
    fn gpu_ac10_idle_overlay_is_empty() {
        let state = InteractionState::Idle;
        let mut scene = Scene::new();
        state.render_overlay(&mut scene);

        let encoding = scene.encoding();
        assert_eq!(
            encoding.path_tags.len(),
            0,
            "idle state should not produce overlay content"
        );
    }
}
