//! Chart-internal layout — margins, plot area rect.
//!
//! Uses a fixed margin model with Observable Plot defaults.

/// Chart layout computed from element bounds and margin settings.
#[derive(Debug, Clone, Copy)]
pub struct ChartLayout {
    /// Total width of the chart element in pixels.
    pub width: f64,
    /// Total height of the chart element in pixels.
    pub height: f64,
    /// Margins around the plot area.
    pub margins: Margins,
}

/// Margins around the plot area. Observable Plot defaults.
#[derive(Debug, Clone, Copy)]
pub struct Margins {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl Default for Margins {
    fn default() -> Self {
        // Observable Plot defaults.
        Self {
            top: 20.0,
            right: 20.0,
            bottom: 30.0,
            left: 40.0,
        }
    }
}

impl ChartLayout {
    /// Create a chart layout with the given total dimensions and default margins.
    pub fn new(width: f64, height: f64) -> Self {
        Self {
            width,
            height,
            margins: Margins::default(),
        }
    }

    /// Create a chart layout with custom margins.
    pub fn with_margins(width: f64, height: f64, margins: Margins) -> Self {
        Self {
            width,
            height,
            margins,
        }
    }

    /// The plot area x-start (left edge of the data region).
    pub fn plot_x_start(&self) -> f64 {
        self.margins.left
    }

    /// The plot area x-end (right edge of the data region).
    pub fn plot_x_end(&self) -> f64 {
        self.width - self.margins.right
    }

    /// The plot area y-start (top edge of the data region).
    pub fn plot_y_start(&self) -> f64 {
        self.margins.top
    }

    /// The plot area y-end (bottom edge of the data region).
    pub fn plot_y_end(&self) -> f64 {
        self.height - self.margins.bottom
    }

    /// The plot area width.
    pub fn plot_width(&self) -> f64 {
        self.plot_x_end() - self.plot_x_start()
    }

    /// The plot area height.
    pub fn plot_height(&self) -> f64 {
        self.plot_y_end() - self.plot_y_start()
    }

    /// X pixel range for scales: (left_margin, width - right_margin).
    pub fn x_range(&self) -> (f64, f64) {
        (self.plot_x_start(), self.plot_x_end())
    }

    /// Y pixel range for scales: (bottom, top) — inverted because screen Y goes down.
    /// Scales map domain_min -> y_end (bottom) and domain_max -> y_start (top).
    pub fn y_range(&self) -> (f64, f64) {
        (self.plot_y_end(), self.plot_y_start())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_layout_defaults_match_observable_plot() {
        let m = Margins::default();
        assert!((m.top - 20.0).abs() < f64::EPSILON);
        assert!((m.right - 20.0).abs() < f64::EPSILON);
        assert!((m.bottom - 30.0).abs() < f64::EPSILON);
        assert!((m.left - 40.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gpu_layout_plot_area() {
        let layout = ChartLayout::new(640.0, 480.0);
        assert!((layout.plot_x_start() - 40.0).abs() < f64::EPSILON);
        assert!((layout.plot_x_end() - 620.0).abs() < f64::EPSILON);
        assert!((layout.plot_y_start() - 20.0).abs() < f64::EPSILON);
        assert!((layout.plot_y_end() - 450.0).abs() < f64::EPSILON);
        assert!((layout.plot_width() - 580.0).abs() < f64::EPSILON);
        assert!((layout.plot_height() - 430.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gpu_layout_ranges() {
        let layout = ChartLayout::new(640.0, 480.0);
        let (x0, x1) = layout.x_range();
        let (y0, y1) = layout.y_range();
        // x range: left margin to right edge
        assert!((x0 - 40.0).abs() < f64::EPSILON);
        assert!((x1 - 620.0).abs() < f64::EPSILON);
        // y range: inverted (bottom to top)
        assert!((y0 - 450.0).abs() < f64::EPSILON);
        assert!((y1 - 20.0).abs() < f64::EPSILON);
    }
}
