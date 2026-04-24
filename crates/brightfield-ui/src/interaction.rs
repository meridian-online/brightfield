//! Interaction state — tracks brush rect and hovered point for overlay
//! rendering without triggering DuckDB re-query.
//!
//! Two-tier interaction model:
//! - Immediate: overlay renders during drag (brush rect, highlight) — pure GPU, no I/O
//! - Deferred: DuckDB re-query fires on brush release via session.update_param()

use std::time::{Duration, Instant};

use kurbo::{Affine, Point, Rect};
use peniko::{Color, Fill};
use vello::Scene;

use brightfield_render::scale::ViewExtent;
use brightfield_spec::vocab::InteractorKind;

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

/// Typed axis-lock and pan/zoom capability derived from `InteractorKind`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavigationConfig {
    /// Whether panning is enabled.
    pub pan: bool,
    /// Whether zooming is enabled.
    pub zoom: bool,
    /// Whether the x-axis is navigable.
    pub x_navigable: bool,
    /// Whether the y-axis is navigable.
    pub y_navigable: bool,
}

impl NavigationConfig {
    /// Derive a `NavigationConfig` from an `InteractorKind`.
    ///
    /// Returns `Some` for the six pan/zoom interactor kinds, `None` for all others.
    pub fn from_interactor_kind(kind: InteractorKind) -> Option<Self> {
        match kind {
            InteractorKind::Pan => Some(Self {
                pan: true,
                zoom: false,
                x_navigable: true,
                y_navigable: true,
            }),
            InteractorKind::PanX => Some(Self {
                pan: true,
                zoom: false,
                x_navigable: true,
                y_navigable: false,
            }),
            InteractorKind::PanY => Some(Self {
                pan: true,
                zoom: false,
                x_navigable: false,
                y_navigable: true,
            }),
            InteractorKind::PanZoom => Some(Self {
                pan: true,
                zoom: true,
                x_navigable: true,
                y_navigable: true,
            }),
            InteractorKind::PanZoomX => Some(Self {
                pan: true,
                zoom: true,
                x_navigable: true,
                y_navigable: false,
            }),
            InteractorKind::PanZoomY => Some(Self {
                pan: true,
                zoom: true,
                x_navigable: false,
                y_navigable: true,
            }),
            _ => None,
        }
    }
}

/// Mutable navigation state for pan/zoom interaction.
#[derive(Debug, Clone)]
pub struct NavigationState {
    /// Current view extent (None = full data extent).
    pub view_extent: ViewExtent,
    /// Navigation configuration (axis lock, pan/zoom capabilities).
    pub config: NavigationConfig,
    /// Timestamp of the last pan/zoom event (for debounce).
    last_event: Option<Instant>,
    /// Debounce duration (default 150ms).
    debounce_duration: Duration,
    /// Whether a re-query is pending (settle not yet fired).
    pub requery_pending: bool,
}

impl NavigationState {
    /// Create a new navigation state with the given config.
    pub fn new(config: NavigationConfig) -> Self {
        Self {
            view_extent: ViewExtent::default(),
            config,
            last_event: None,
            debounce_duration: Duration::from_millis(150),
            requery_pending: false,
        }
    }

    /// Create a navigation state with a custom debounce duration.
    pub fn with_debounce(config: NavigationConfig, debounce: Duration) -> Self {
        Self {
            debounce_duration: debounce,
            ..Self::new(config)
        }
    }

    /// Apply a pan gesture (normalised pixel delta).
    ///
    /// `dx_norm` and `dy_norm` are normalised deltas: `px_delta / range_width`.
    /// Axis lock is respected: non-navigable axes are ignored.
    pub fn apply_pan(
        &mut self,
        dx_norm: f64,
        dy_norm: f64,
        x_domain: (f64, f64),
        y_domain: (f64, f64),
    ) {
        if self.config.pan && self.config.x_navigable {
            let x_span = x_domain.1 - x_domain.0;
            let (x_min, x_max) = self
                .view_extent
                .x
                .unwrap_or(x_domain);
            let new_min = x_min - dx_norm * x_span;
            let new_max = x_max - dx_norm * x_span;
            self.view_extent.x = Some((new_min, new_max));
        }
        if self.config.pan && self.config.y_navigable {
            let y_span = y_domain.1 - y_domain.0;
            let (y_min, y_max) = self
                .view_extent
                .y
                .unwrap_or(y_domain);
            let new_min = y_min - dy_norm * y_span;
            let new_max = y_max - dy_norm * y_span;
            self.view_extent.y = Some((new_min, new_max));
        }
        self.last_event = Some(Instant::now());
        self.requery_pending = true;
    }

    /// Apply a zoom gesture around a cursor position.
    ///
    /// `cursor_norm_x` and `cursor_norm_y` are the cursor's normalised position
    /// within the range [0, 1]. `zoom_factor` > 1.0 zooms in, < 1.0 zooms out.
    pub fn apply_zoom(
        &mut self,
        cursor_norm_x: f64,
        cursor_norm_y: f64,
        zoom_factor: f64,
        x_domain: (f64, f64),
        y_domain: (f64, f64),
    ) {
        if self.config.zoom && self.config.x_navigable {
            let (x_min, x_max) = self.view_extent.x.unwrap_or(x_domain);
            let x_span = x_max - x_min;
            let cursor_data = x_min + cursor_norm_x * x_span;
            let new_span = x_span / zoom_factor;
            let new_min = cursor_data - cursor_norm_x * new_span;
            let new_max = cursor_data + (1.0 - cursor_norm_x) * new_span;
            self.view_extent.x = Some((new_min, new_max));
        }
        if self.config.zoom && self.config.y_navigable {
            let (y_min, y_max) = self.view_extent.y.unwrap_or(y_domain);
            let y_span = y_max - y_min;
            let cursor_data = y_min + cursor_norm_y * y_span;
            let new_span = y_span / zoom_factor;
            let new_min = cursor_data - cursor_norm_y * new_span;
            let new_max = cursor_data + (1.0 - cursor_norm_y) * new_span;
            self.view_extent.y = Some((new_min, new_max));
        }
        self.last_event = Some(Instant::now());
        self.requery_pending = true;
    }

    /// Reset the view extent to None (full data extent).
    pub fn reset(&mut self) {
        if self.config.x_navigable {
            self.view_extent.x = None;
        }
        if self.config.y_navigable {
            self.view_extent.y = None;
        }
        self.last_event = None;
        self.requery_pending = true;
    }

    /// Check if the debounce timer has settled (enough time since last event).
    ///
    /// Returns `true` if a re-query should fire. Calling this also clears the
    /// pending flag.
    pub fn check_settle(&mut self) -> bool {
        if !self.requery_pending {
            return false;
        }
        if let Some(last) = self.last_event {
            if last.elapsed() >= self.debounce_duration {
                self.requery_pending = false;
                return true;
            }
        }
        false
    }

    /// Get the current view extent (for passing to build_chart_scene).
    pub fn current_extent(&self) -> Option<&ViewExtent> {
        if self.view_extent.x.is_some() || self.view_extent.y.is_some() {
            Some(&self.view_extent)
        } else {
            None
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

    // --- nav_ac04: NavigationConfig ---

    #[test]
    fn nav_ac04_pan_config() {
        let cfg = NavigationConfig::from_interactor_kind(InteractorKind::Pan).unwrap();
        assert!(cfg.pan);
        assert!(!cfg.zoom);
        assert!(cfg.x_navigable);
        assert!(cfg.y_navigable);
    }

    #[test]
    fn nav_ac04_pan_x_config() {
        let cfg = NavigationConfig::from_interactor_kind(InteractorKind::PanX).unwrap();
        assert!(cfg.pan);
        assert!(!cfg.zoom);
        assert!(cfg.x_navigable);
        assert!(!cfg.y_navigable);
    }

    #[test]
    fn nav_ac04_pan_y_config() {
        let cfg = NavigationConfig::from_interactor_kind(InteractorKind::PanY).unwrap();
        assert!(cfg.pan);
        assert!(!cfg.zoom);
        assert!(!cfg.x_navigable);
        assert!(cfg.y_navigable);
    }

    #[test]
    fn nav_ac04_pan_zoom_config() {
        let cfg = NavigationConfig::from_interactor_kind(InteractorKind::PanZoom).unwrap();
        assert!(cfg.pan);
        assert!(cfg.zoom);
        assert!(cfg.x_navigable);
        assert!(cfg.y_navigable);
    }

    #[test]
    fn nav_ac04_pan_zoom_x_config() {
        let cfg = NavigationConfig::from_interactor_kind(InteractorKind::PanZoomX).unwrap();
        assert!(cfg.pan);
        assert!(cfg.zoom);
        assert!(cfg.x_navigable);
        assert!(!cfg.y_navigable);
    }

    #[test]
    fn nav_ac04_pan_zoom_y_config() {
        let cfg = NavigationConfig::from_interactor_kind(InteractorKind::PanZoomY).unwrap();
        assert!(cfg.pan);
        assert!(cfg.zoom);
        assert!(!cfg.x_navigable);
        assert!(cfg.y_navigable);
    }

    #[test]
    fn nav_ac04_non_navigation_returns_none() {
        assert!(NavigationConfig::from_interactor_kind(InteractorKind::Nearest).is_none());
        assert!(NavigationConfig::from_interactor_kind(InteractorKind::Highlight).is_none());
        assert!(NavigationConfig::from_interactor_kind(InteractorKind::Toggle).is_none());
    }

    // --- nav_ac05: Pan gesture handler ---

    #[test]
    fn nav_ac05_pan_x_only() {
        let config = NavigationConfig {
            pan: true,
            zoom: false,
            x_navigable: true,
            y_navigable: false,
        };
        let mut state = NavigationState::new(config);
        let x_domain = (0.0, 100.0);
        let y_domain = (0.0, 50.0);

        // Pan right by 10% of the range
        state.apply_pan(0.1, 0.1, x_domain, y_domain);

        // X should have shifted
        assert!(state.view_extent.x.is_some());
        let (x_min, x_max) = state.view_extent.x.unwrap();
        assert!((x_min - (-10.0)).abs() < f64::EPSILON);
        assert!((x_max - 90.0).abs() < f64::EPSILON);

        // Y should be unchanged (axis locked)
        assert!(state.view_extent.y.is_none());
    }

    #[test]
    fn nav_ac05_pan_both_axes() {
        let config = NavigationConfig {
            pan: true,
            zoom: false,
            x_navigable: true,
            y_navigable: true,
        };
        let mut state = NavigationState::new(config);
        state.apply_pan(0.2, 0.3, (0.0, 100.0), (0.0, 50.0));

        assert!(state.view_extent.x.is_some());
        assert!(state.view_extent.y.is_some());
        let (x_min, _) = state.view_extent.x.unwrap();
        let (y_min, _) = state.view_extent.y.unwrap();
        assert!((x_min - (-20.0)).abs() < f64::EPSILON);
        assert!((y_min - (-15.0)).abs() < f64::EPSILON);
    }

    // --- nav_ac06: Zoom gesture handler ---

    #[test]
    fn nav_ac06_zoom_in_center_narrows_symmetrically() {
        let config = NavigationConfig {
            pan: true,
            zoom: true,
            x_navigable: true,
            y_navigable: true,
        };
        let mut state = NavigationState::new(config);

        // Zoom 2x at center (cursor_norm = 0.5)
        state.apply_zoom(0.5, 0.5, 2.0, (0.0, 100.0), (0.0, 50.0));

        let (x_min, x_max) = state.view_extent.x.unwrap();
        assert!((x_min - 25.0).abs() < f64::EPSILON);
        assert!((x_max - 75.0).abs() < f64::EPSILON);

        let (y_min, y_max) = state.view_extent.y.unwrap();
        assert!((y_min - 12.5).abs() < f64::EPSILON);
        assert!((y_max - 37.5).abs() < f64::EPSILON);
    }

    #[test]
    fn nav_ac06_zoom_y_locked() {
        let config = NavigationConfig {
            pan: true,
            zoom: true,
            x_navigable: true,
            y_navigable: false,
        };
        let mut state = NavigationState::new(config);
        state.apply_zoom(0.5, 0.5, 2.0, (0.0, 100.0), (0.0, 50.0));

        assert!(state.view_extent.x.is_some());
        assert!(state.view_extent.y.is_none());
    }

    // --- nav_ac07: Reset ---

    #[test]
    fn nav_ac07_reset_clears_extent() {
        let config = NavigationConfig {
            pan: true,
            zoom: true,
            x_navigable: true,
            y_navigable: true,
        };
        let mut state = NavigationState::new(config);
        state.view_extent.x = Some((25.0, 75.0));
        state.view_extent.y = Some((10.0, 40.0));

        state.reset();

        assert!(state.view_extent.x.is_none());
        assert!(state.view_extent.y.is_none());
    }

    // --- nav_ac08: Debounce ---

    #[test]
    fn nav_ac08_debounce_not_settled_immediately() {
        let config = NavigationConfig {
            pan: true,
            zoom: true,
            x_navigable: true,
            y_navigable: true,
        };
        let mut state = NavigationState::with_debounce(config, Duration::from_millis(100));
        state.apply_zoom(0.5, 0.5, 2.0, (0.0, 100.0), (0.0, 50.0));

        // Immediately after zoom, settle should not fire
        assert!(!state.check_settle());
        assert!(state.requery_pending);
    }

    #[test]
    fn nav_ac08_debounce_settles_after_duration() {
        let config = NavigationConfig {
            pan: true,
            zoom: true,
            x_navigable: true,
            y_navigable: true,
        };
        // Use 1ms debounce for test speed
        let mut state = NavigationState::with_debounce(config, Duration::from_millis(1));
        state.apply_zoom(0.5, 0.5, 2.0, (0.0, 100.0), (0.0, 50.0));

        // Wait for debounce
        std::thread::sleep(Duration::from_millis(5));

        assert!(state.check_settle());
        // After settle, pending should be cleared
        assert!(!state.requery_pending);
    }

    #[test]
    fn nav_ac08_debounce_resets_on_new_event() {
        let config = NavigationConfig {
            pan: true,
            zoom: true,
            x_navigable: true,
            y_navigable: true,
        };
        let mut state = NavigationState::with_debounce(config, Duration::from_millis(50));
        state.apply_zoom(0.5, 0.5, 2.0, (0.0, 100.0), (0.0, 50.0));

        // Fire a new event — timer should reset
        state.apply_pan(0.1, 0.0, (0.0, 100.0), (0.0, 50.0));

        // Immediately after new event, not settled
        assert!(!state.check_settle());
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
