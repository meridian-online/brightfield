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
    /// Per-side pixel insets pulled off the positional scale RANGES (card 0008
    /// axis-inset round). Applied only in [`ChartLayout::x_range`] /
    /// [`ChartLayout::y_range`] so edge marks render whole inside the frame;
    /// the plot rect (`plot_x_start`..`plot_x_end`, used for the frame clip and
    /// axis lines) is deliberately left un-inset. Default: all zero.
    pub insets: Insets,
}

/// Per-side pixel insets pulled off the positional scale ranges so an edge mark
/// (e.g. a dot sitting at a domain bound) renders its whole disc inside the
/// frame clip instead of clipping to a sliver. Range-side only — domains and
/// the frame clip are untouched. Resolved once at assembly and carried
/// identically by both layout models (render here, ui `chart_layout`), pinned
/// equal by a cross-model agreement test.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Insets {
    /// Pulled off the x range start (left edge).
    pub left: f64,
    /// Pulled off the x range end (right edge).
    pub right: f64,
    /// Pulled off the y range end (top edge — screen y grows downward).
    pub top: f64,
    /// Pulled off the y range start (bottom edge).
    pub bottom: f64,
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
            insets: Insets::default(),
        }
    }

    /// Create a chart layout with custom margins.
    pub fn with_margins(width: f64, height: f64, margins: Margins) -> Self {
        Self {
            width,
            height,
            margins,
            insets: Insets::default(),
        }
    }

    /// Create a chart layout with default margins and the given range insets.
    /// The margins (hence the frame clip and axis lines) are unchanged; only
    /// the positional scale ranges pull inward — see [`Insets`].
    pub fn with_insets(width: f64, height: f64, insets: Insets) -> Self {
        Self {
            width,
            height,
            margins: Margins::default(),
            insets,
        }
    }

    /// The resolved per-side range insets carried by this layout.
    pub fn insets(&self) -> Insets {
        self.insets
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

    /// X pixel range for scales, pulled inward by the per-side insets:
    /// `(plot_x_start + inset.left, plot_x_end - inset.right)`. The plot rect
    /// itself (`plot_x_start`/`plot_x_end`) is unchanged, so the frame clip and
    /// axis line stay put while the scale range — and thus every mark and
    /// tick — sits inside them.
    pub fn x_range(&self) -> (f64, f64) {
        (
            self.plot_x_start() + self.insets.left,
            self.plot_x_end() - self.insets.right,
        )
    }

    /// Y pixel range for scales: (bottom, top) — inverted because screen Y goes
    /// down. Scales map domain_min -> range start (bottom) and domain_max ->
    /// range end (top); the insets pull that range inward, so `inset.bottom`
    /// raises the (larger) start pixel and `inset.top` lowers the (smaller) end
    /// pixel.
    pub fn y_range(&self) -> (f64, f64) {
        (
            self.plot_y_end() - self.insets.bottom,
            self.plot_y_start() + self.insets.top,
        )
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
        // x range: left margin to right edge (zero-inset default)
        assert!((x0 - 40.0).abs() < f64::EPSILON);
        assert!((x1 - 620.0).abs() < f64::EPSILON);
        // y range: inverted (bottom to top)
        assert!((y0 - 450.0).abs() < f64::EPSILON);
        assert!((y1 - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gpu_layout_ranges_inset() {
        // Per-side insets pull the RANGE inward; the plot rect (frame/axes)
        // stays put. y_range is inverted: bottom (start) rises by inset.bottom,
        // top (end) drops by inset.top.
        let insets = Insets {
            left: 5.0,
            right: 7.0,
            top: 3.0,
            bottom: 9.0,
        };
        let layout = ChartLayout::with_insets(640.0, 480.0, insets);

        // Frame/plot rect is unchanged — the clip and axis lines don't move.
        assert!((layout.plot_x_start() - 40.0).abs() < f64::EPSILON);
        assert!((layout.plot_x_end() - 620.0).abs() < f64::EPSILON);
        assert!((layout.plot_y_start() - 20.0).abs() < f64::EPSILON);
        assert!((layout.plot_y_end() - 450.0).abs() < f64::EPSILON);

        // Scale ranges pull inward by the resolved insets.
        let (x0, x1) = layout.x_range();
        assert!((x0 - 45.0).abs() < f64::EPSILON); // 40 + left 5
        assert!((x1 - 613.0).abs() < f64::EPSILON); // 620 - right 7
        let (y0, y1) = layout.y_range();
        assert!((y0 - 441.0).abs() < f64::EPSILON); // 450 - bottom 9
        assert!((y1 - 23.0).abs() < f64::EPSILON); // 20 + top 3

        assert_eq!(layout.insets(), insets);
    }
}
