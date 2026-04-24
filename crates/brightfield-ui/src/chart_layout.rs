//! Coordinate mapping pipeline for chart elements.
//!
//! ChartLayout defines the plot area bounds within a chart element,
//! accounting for margins. It provides the mapping pipeline:
//!
//! ```text
//! window_pos - element_origin → local_px
//!   → check local_px within plot_area
//!   → scale.inverse_f64(local_px) → data_value
//! ```

use kurbo::{Point, Rect};

/// Default margins (left, top, right, bottom) in pixels.
const DEFAULT_MARGIN_LEFT: f64 = 40.0;
const DEFAULT_MARGIN_TOP: f64 = 20.0;
const DEFAULT_MARGIN_RIGHT: f64 = 20.0;
const DEFAULT_MARGIN_BOTTOM: f64 = 30.0;

/// Layout dimensions and plot area bounds for a chart element.
///
/// The plot area is the region inside the margins where data marks are drawn.
/// Axes, labels, and legends live outside the plot area but inside the element bounds.
#[derive(Debug, Clone, Copy)]
pub struct ChartLayout {
    /// Total element width in pixels.
    pub width: f64,
    /// Total element height in pixels.
    pub height: f64,
    /// Left margin in pixels (space for y-axis labels).
    pub margin_left: f64,
    /// Top margin in pixels.
    pub margin_top: f64,
    /// Right margin in pixels.
    pub margin_right: f64,
    /// Bottom margin in pixels (space for x-axis labels).
    pub margin_bottom: f64,
}

impl ChartLayout {
    /// Create a layout with default margins.
    pub fn new(width: f64, height: f64) -> Self {
        Self {
            width,
            height,
            margin_left: DEFAULT_MARGIN_LEFT,
            margin_top: DEFAULT_MARGIN_TOP,
            margin_right: DEFAULT_MARGIN_RIGHT,
            margin_bottom: DEFAULT_MARGIN_BOTTOM,
        }
    }

    /// Create a layout with custom margins.
    pub fn with_margins(
        width: f64,
        height: f64,
        left: f64,
        top: f64,
        right: f64,
        bottom: f64,
    ) -> Self {
        Self {
            width,
            height,
            margin_left: left,
            margin_top: top,
            margin_right: right,
            margin_bottom: bottom,
        }
    }

    /// The plot area rectangle (inside margins).
    ///
    /// Returns `(x0, y0, x1, y1)` where (x0, y0) is the top-left and
    /// (x1, y1) is the bottom-right of the plot area.
    pub fn plot_area(&self) -> Rect {
        Rect::new(
            self.margin_left,
            self.margin_top,
            self.width - self.margin_right,
            self.height - self.margin_bottom,
        )
    }

    /// Plot area width in pixels.
    pub fn plot_width(&self) -> f64 {
        self.width - self.margin_left - self.margin_right
    }

    /// Plot area height in pixels.
    pub fn plot_height(&self) -> f64 {
        self.height - self.margin_top - self.margin_bottom
    }

    /// Transform a window-space point to element-local coordinates.
    ///
    /// Subtracts the element origin from the window position.
    pub fn window_to_local(&self, window_pos: Point, element_origin: Point) -> Point {
        Point::new(
            window_pos.x - element_origin.x,
            window_pos.y - element_origin.y,
        )
    }

    /// Check whether a local-space point falls within the plot area.
    pub fn contains(&self, local_pos: Point) -> bool {
        self.plot_area().contains(local_pos)
    }

    /// Transform a local pixel position within the plot area to normalised
    /// coordinates [0, 1] relative to the plot area.
    ///
    /// Returns `None` if the point is outside the plot area.
    pub fn local_to_normalised(&self, local_pos: Point) -> Option<(f64, f64)> {
        let area = self.plot_area();
        if !area.contains(local_pos) {
            return None;
        }
        let nx = (local_pos.x - area.x0) / (area.x1 - area.x0);
        let ny = (local_pos.y - area.y0) / (area.y1 - area.y0);
        Some((nx, ny))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Point;

    // --- gmr_ac08: Coordinate mapping pipeline ---

    #[test]
    fn gmr_ac08_plot_area_with_default_margins() {
        let layout = ChartLayout::new(640.0, 480.0);
        let area = layout.plot_area();
        assert!((area.x0 - 40.0).abs() < f64::EPSILON);
        assert!((area.y0 - 20.0).abs() < f64::EPSILON);
        assert!((area.x1 - 620.0).abs() < f64::EPSILON);
        assert!((area.y1 - 450.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gmr_ac08_plot_area_with_custom_margins() {
        // margins: left=40, top=20, right=20, bottom=30
        let layout = ChartLayout::with_margins(640.0, 480.0, 40.0, 20.0, 20.0, 30.0);
        let area = layout.plot_area();
        assert!((area.x0 - 40.0).abs() < f64::EPSILON);
        assert!((area.y0 - 20.0).abs() < f64::EPSILON);
        assert!((area.x1 - 620.0).abs() < f64::EPSILON);
        assert!((area.y1 - 450.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gmr_ac08_plot_dimensions() {
        let layout = ChartLayout::with_margins(640.0, 480.0, 40.0, 20.0, 20.0, 30.0);
        assert!((layout.plot_width() - 580.0).abs() < f64::EPSILON);
        assert!((layout.plot_height() - 430.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gmr_ac08_window_to_local_subtracts_origin() {
        let layout = ChartLayout::new(640.0, 480.0);
        let window_pos = Point::new(150.0, 250.0);
        let element_origin = Point::new(50.0, 100.0);
        let local = layout.window_to_local(window_pos, element_origin);
        assert!((local.x - 100.0).abs() < f64::EPSILON);
        assert!((local.y - 150.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gmr_ac08_contains_inside_plot_area() {
        let layout = ChartLayout::new(640.0, 480.0);
        // Point inside plot area
        assert!(layout.contains(Point::new(300.0, 200.0)));
    }

    #[test]
    fn gmr_ac08_contains_outside_plot_area() {
        let layout = ChartLayout::new(640.0, 480.0);
        // Point in left margin (x=10 < margin_left=40)
        assert!(!layout.contains(Point::new(10.0, 200.0)));
        // Point below plot area (y=460 > 450)
        assert!(!layout.contains(Point::new(300.0, 460.0)));
    }

    #[test]
    fn gmr_ac08_local_to_normalised_inside() {
        let layout = ChartLayout::with_margins(640.0, 480.0, 40.0, 20.0, 20.0, 30.0);
        // Midpoint of plot area: x=(40+620)/2=330, y=(20+450)/2=235
        let result = layout.local_to_normalised(Point::new(330.0, 235.0));
        let (nx, ny) = result.expect("point is inside plot area");
        assert!((nx - 0.5).abs() < 0.01);
        assert!((ny - 0.5).abs() < 0.01);
    }

    #[test]
    fn gmr_ac08_local_to_normalised_outside() {
        let layout = ChartLayout::new(640.0, 480.0);
        let result = layout.local_to_normalised(Point::new(10.0, 200.0));
        assert!(result.is_none());
    }

    #[test]
    fn gmr_ac08_inverse_scale_transform() {
        // Given a linear scale mapping [0, 100] → [40, 620] (plot area x range),
        // verify inverse transform produces correct data values.
        use brightfield_render::scale::Scale;

        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: 40.0,
            range_end: 620.0,
        };

        // At pixel 40 (left edge) → data value 0
        let val = scale.inverse_f64(40.0).expect("linear should return Some");
        assert!((val - 0.0).abs() < 0.1);

        // At pixel 620 (right edge) → data value 100
        let val = scale.inverse_f64(620.0).expect("linear should return Some");
        assert!((val - 100.0).abs() < 0.1);

        // At pixel 330 (midpoint) → data value 50
        let val = scale.inverse_f64(330.0).expect("linear should return Some");
        assert!((val - 50.0).abs() < 0.1);
    }
}
