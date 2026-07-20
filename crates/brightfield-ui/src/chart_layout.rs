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

use brightfield_render::layout::Insets;
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
    /// Per-side range insets (axis-inset round), carried identically
    /// to the render `ChartLayout` so hit-testing and brush inversion use the
    /// same inset pixels as the rendered scale range. Applied inside
    /// [`ChartLayout::plot_area`]; a cross-model agreement test pins render
    /// `x_range`/`y_range` equal to this plot area for the same dims + insets.
    pub insets: Insets,
}

impl ChartLayout {
    /// Create a layout with default margins and no insets.
    pub fn new(width: f64, height: f64) -> Self {
        Self {
            width,
            height,
            margin_left: DEFAULT_MARGIN_LEFT,
            margin_top: DEFAULT_MARGIN_TOP,
            margin_right: DEFAULT_MARGIN_RIGHT,
            margin_bottom: DEFAULT_MARGIN_BOTTOM,
            insets: Insets::default(),
        }
    }

    /// Create a layout with custom margins and no insets.
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
            insets: Insets::default(),
        }
    }

    /// Create a layout with default margins and the given range insets.
    pub fn with_insets(width: f64, height: f64, insets: Insets) -> Self {
        Self {
            width,
            height,
            margin_left: DEFAULT_MARGIN_LEFT,
            margin_top: DEFAULT_MARGIN_TOP,
            margin_right: DEFAULT_MARGIN_RIGHT,
            margin_bottom: DEFAULT_MARGIN_BOTTOM,
            insets,
        }
    }

    /// Create a layout composing custom (title-grown) margins with range insets
    /// — the title-aware runtime path. Mirrors the render
    /// `ChartLayout::with_margins_and_insets`: the grown margins reserve the
    /// title bands (outer budget) while the insets pull the positional range
    /// inward. Both models fed the same values keep `plot_area` == render
    /// `x_range`/`y_range`, so a titled plot's hit-testing matches its scene.
    pub fn with_margins_and_insets(
        width: f64,
        height: f64,
        left: f64,
        top: f64,
        right: f64,
        bottom: f64,
        insets: Insets,
    ) -> Self {
        Self {
            width,
            height,
            margin_left: left,
            margin_top: top,
            margin_right: right,
            margin_bottom: bottom,
            insets,
        }
    }

    /// The per-side range insets carried by this layout.
    pub fn insets(&self) -> Insets {
        self.insets
    }

    /// The plot area rectangle (inside margins, pulled inward by the range
    /// insets).
    ///
    /// Returns `(x0, y0, x1, y1)` where (x0, y0) is the top-left and
    /// (x1, y1) is the bottom-right. The insets match the render layout's
    /// `x_range`/`y_range` so pointer→data inversion lands on the same geometry
    /// the marks were drawn against.
    pub fn plot_area(&self) -> Rect {
        Rect::new(
            self.margin_left + self.insets.left,
            self.margin_top + self.insets.top,
            self.width - self.margin_right - self.insets.right,
            self.height - self.margin_bottom - self.insets.bottom,
        )
    }

    /// The frame rectangle — margins ONLY, with NO range insets.
    ///
    /// `plot_area` pulls inward by the axis-inset band (#55) so an edge
    /// mark renders whole inside the frame clip; a brush clamped to `plot_area`
    /// therefore can't reach into that band to enclose a boundary dot whose 4px
    /// disc overflows it. Clamping the move/resize/drag to `frame_area` instead
    /// recovers the full band symmetrically, so the brush rectangle can enclose a
    /// boundary dot. GLOBAL: every interval brush inherits this clamp target.
    /// The enclosure guarantee holds precisely when the per-side inset >=
    /// `DOT_RADIUS`; `plot_area` and the axis-inset range pull are UNCHANGED.
    pub fn frame_area(&self) -> Rect {
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
    use brightfield_render::layout::ChartLayout as RenderLayout;
    use kurbo::Point;

    // --- Coordinate mapping pipeline ---

    #[test]
    fn plot_area_with_default_margins() {
        let layout = ChartLayout::new(640.0, 480.0);
        let area = layout.plot_area();
        assert!((area.x0 - 40.0).abs() < f64::EPSILON);
        assert!((area.y0 - 20.0).abs() < f64::EPSILON);
        assert!((area.x1 - 620.0).abs() < f64::EPSILON);
        assert!((area.y1 - 450.0).abs() < f64::EPSILON);
    }

    #[test]
    fn plot_area_with_custom_margins() {
        // margins: left=40, top=20, right=20, bottom=30
        let layout = ChartLayout::with_margins(640.0, 480.0, 40.0, 20.0, 20.0, 30.0);
        let area = layout.plot_area();
        assert!((area.x0 - 40.0).abs() < f64::EPSILON);
        assert!((area.y0 - 20.0).abs() < f64::EPSILON);
        assert!((area.x1 - 620.0).abs() < f64::EPSILON);
        assert!((area.y1 - 450.0).abs() < f64::EPSILON);
    }

    #[test]
    fn plot_dimensions() {
        let layout = ChartLayout::with_margins(640.0, 480.0, 40.0, 20.0, 20.0, 30.0);
        assert!((layout.plot_width() - 580.0).abs() < f64::EPSILON);
        assert!((layout.plot_height() - 430.0).abs() < f64::EPSILON);
    }

    #[test]
    fn window_to_local_subtracts_origin() {
        let layout = ChartLayout::new(640.0, 480.0);
        let window_pos = Point::new(150.0, 250.0);
        let element_origin = Point::new(50.0, 100.0);
        let local = layout.window_to_local(window_pos, element_origin);
        assert!((local.x - 100.0).abs() < f64::EPSILON);
        assert!((local.y - 150.0).abs() < f64::EPSILON);
    }

    #[test]
    fn contains_inside_plot_area() {
        let layout = ChartLayout::new(640.0, 480.0);
        // Point inside plot area
        assert!(layout.contains(Point::new(300.0, 200.0)));
    }

    #[test]
    fn contains_outside_plot_area() {
        let layout = ChartLayout::new(640.0, 480.0);
        // Point in left margin (x=10 < margin_left=40)
        assert!(!layout.contains(Point::new(10.0, 200.0)));
        // Point below plot area (y=460 > 450)
        assert!(!layout.contains(Point::new(300.0, 460.0)));
    }

    #[test]
    fn local_to_normalised_inside() {
        let layout = ChartLayout::with_margins(640.0, 480.0, 40.0, 20.0, 20.0, 30.0);
        // Midpoint of plot area: x=(40+620)/2=330, y=(20+450)/2=235
        let result = layout.local_to_normalised(Point::new(330.0, 235.0));
        let (nx, ny) = result.expect("point is inside plot area");
        assert!((nx - 0.5).abs() < 0.01);
        assert!((ny - 0.5).abs() < 0.01);
    }

    #[test]
    fn local_to_normalised_outside() {
        let layout = ChartLayout::new(640.0, 480.0);
        let result = layout.local_to_normalised(Point::new(10.0, 200.0));
        assert!(result.is_none());
    }

    // --- interaction parity under insets ---

    #[test]
    fn plot_area_pulls_inward_by_insets() {
        let insets = Insets {
            left: 5.0,
            right: 7.0,
            top: 3.0,
            bottom: 9.0,
        };
        let layout = ChartLayout::with_insets(640.0, 480.0, insets);
        let area = layout.plot_area();
        assert!((area.x0 - 45.0).abs() < f64::EPSILON); // 40 + 5
        assert!((area.y0 - 23.0).abs() < f64::EPSILON); // 20 + 3
        assert!((area.x1 - 613.0).abs() < f64::EPSILON); // 620 - 7
        assert!((area.y1 - 441.0).abs() < f64::EPSILON); // 450 - 9
        assert_eq!(layout.insets(), insets);
    }

    #[test]
    fn cross_model_agreement() {
        // The load-bearing pin: render x_range()/y_range() must equal the ui
        // plot-area-derived ranges for the same dims + insets, so the two
        // duplicated layout models can never agree-by-luck for insets. y_range
        // is inverted: render (start, end) == (plot_area.y1, plot_area.y0).
        let insets = Insets {
            left: 5.0,
            right: 7.0,
            top: 3.0,
            bottom: 9.0,
        };
        let render = RenderLayout::with_insets(640.0, 480.0, insets);
        let ui = ChartLayout::with_insets(640.0, 480.0, insets);
        let area = ui.plot_area();
        let (rx0, rx1) = render.x_range();
        let (ry0, ry1) = render.y_range();
        assert!((rx0 - area.x0).abs() < f64::EPSILON);
        assert!((rx1 - area.x1).abs() < f64::EPSILON);
        assert!((ry0 - area.y1).abs() < f64::EPSILON);
        assert!((ry1 - area.y0).abs() < f64::EPSILON);
        assert_eq!(render.insets(), ui.insets());
    }

    #[test]
    fn cross_model_agreement_with_grown_title_margins() {
        // The assembly glue: ONE ResolvedTitles → grow_margins → fed to BOTH
        // layout models must still agree per-side (render x_range/y_range == ui
        // plot_area) under per-side-DISTINCT grown margins + insets, so a titled
        // plot's brush inversion / hit-testing matches its rendered scale ranges.
        use brightfield_render::layout::{ChartLayout as RL, Margins};
        use brightfield_render::{grow_margins, ResolvedTitles};

        let titles = ResolvedTitles {
            x: Some("x".into()),    // grows the bottom margin
            y: Some("y".into()),    // grows the left margin
            plot: Some("t".into()), // grows the top margin
        };
        let grown = grow_margins(Margins::default(), &titles);
        // Per-side distinct: left/bottom/top all differ (right never grows).
        assert!(grown.left != grown.bottom && grown.bottom != grown.top);
        let insets = Insets {
            left: 5.0,
            right: 7.0,
            top: 3.0,
            bottom: 9.0,
        };

        let render = RL::with_margins_and_insets(640.0, 480.0, grown, insets);
        let ui = ChartLayout::with_margins_and_insets(
            640.0, 480.0, grown.left, grown.top, grown.right, grown.bottom, insets,
        );
        let area = ui.plot_area();
        let (rx0, rx1) = render.x_range();
        let (ry0, ry1) = render.y_range();
        assert!((rx0 - area.x0).abs() < f64::EPSILON);
        assert!((rx1 - area.x1).abs() < f64::EPSILON);
        assert!((ry0 - area.y1).abs() < f64::EPSILON);
        assert!((ry1 - area.y0).abs() < f64::EPSILON);
    }

    /// `frame_area` is `plot_area` grown by the per-side insets exactly
    /// (margins only, no insets), so a boundary dot's 4px disc — overflowing UP
    /// into the 5px inset band — sits INSIDE frame_area and is enclosable when the
    /// per-side inset >= DOT_RADIUS. A zero-inset plot collapses frame_area onto
    /// plot_area (the conceded degenerate case, where the disc does not fit).
    #[test]
    fn frame_area_recovers_the_inset_band() {
        const DOT_RADIUS: f64 = 4.0; // mark.rs
        let insets = Insets {
            left: 5.0,
            right: 7.0,
            top: 5.0,
            bottom: 9.0,
        };
        let layout = ChartLayout::with_insets(640.0, 480.0, insets);
        let plot = layout.plot_area();
        let frame = layout.frame_area();

        // frame_area == plot_area grown by the per-side insets exactly.
        assert!((frame.x0 - (plot.x0 - insets.left)).abs() < f64::EPSILON);
        assert!((frame.y0 - (plot.y0 - insets.top)).abs() < f64::EPSILON);
        assert!((frame.x1 - (plot.x1 + insets.right)).abs() < f64::EPSILON);
        assert!((frame.y1 - (plot.y1 + insets.bottom)).abs() < f64::EPSILON);
        // Margins only: frame_area is exactly the margin box.
        assert_eq!(frame, Rect::new(40.0, 20.0, 620.0, 450.0));

        // With the 5px top inset (>= DOT_RADIUS), a boundary dot centred at the
        // inset range end (plot.y0) has its whole disc inside frame_area.
        assert!(
            plot.y0 - DOT_RADIUS >= frame.y0,
            "the boundary disc fits inside frame_area when inset >= DOT_RADIUS"
        );

        // Zero-inset plot → frame_area == plot_area (and the disc does NOT fit).
        let zero = ChartLayout::with_insets(640.0, 480.0, Insets::default());
        assert_eq!(zero.frame_area(), zero.plot_area());
        assert!(zero.plot_area().y0 - DOT_RADIUS < zero.frame_area().y0);
    }

    #[test]
    fn inversion_round_trips_at_inset_ends() {
        // A pixel at the inset range end inverts to the domain end — the brush
        // edge lands on the domain bound, not a sliver past it.
        use brightfield_render::scale::Scale;
        let insets = Insets {
            left: 5.0,
            right: 5.0,
            top: 5.0,
            bottom: 5.0,
        };
        let ui = ChartLayout::with_insets(640.0, 480.0, insets);
        let area = ui.plot_area();
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: area.x0,
            range_end: area.x1,
        };
        let v_hi = scale.inverse_f64(area.x1).expect("linear is invertible");
        assert!((v_hi - 100.0).abs() < 1e-9);
        let v_lo = scale.inverse_f64(area.x0).expect("linear is invertible");
        assert!((v_lo - 0.0).abs() < 1e-9);

        // The un-inset frame edge must invert PAST the domain max — proof the 5px
        // inset actually pulled the range end inward. With zero insets the frame
        // edge is the range end and would invert to exactly 100.0, so this
        // assertion fails if insets no-op.
        let frame_x_end = 640.0 - DEFAULT_MARGIN_RIGHT; // 620, pre-inset edge
        let v_frame = scale.inverse_f64(frame_x_end).expect("linear is invertible");
        assert!(
            v_frame > 100.0,
            "un-inset frame edge {frame_x_end} inverts to {v_frame}, must be past domain max"
        );
    }

    #[test]
    fn titled_plot_area_inverts_on_the_grown_range() {
        // The review HIGH, closed at the geometry level: with title-grown
        // margins the ui plot_area IS the render scale range, so a brush edge /
        // click at a plot-area bound inverts to the domain bound — hit-testing
        // matches the titled scene, not a default-margin one.
        use brightfield_render::layout::{ChartLayout as RL, Margins};
        use brightfield_render::scale::Scale;
        use brightfield_render::{grow_margins, ResolvedTitles};

        let titles = ResolvedTitles {
            x: Some("x".into()),
            y: Some("y".into()),
            plot: None,
        };
        let grown = grow_margins(Margins::default(), &titles);
        let ui = ChartLayout::with_margins_and_insets(
            640.0, 480.0, grown.left, grown.top, grown.right, grown.bottom, Insets::default(),
        );
        let render = RL::with_margins_and_insets(640.0, 480.0, grown, Insets::default());
        let area = ui.plot_area();
        let (rx0, rx1) = render.x_range();
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: rx0,
            range_end: rx1,
        };
        // The ui plot-area edges invert to the domain bounds on the grown range.
        assert!((scale.inverse_f64(area.x0).unwrap() - 0.0).abs() < 1e-9);
        assert!((scale.inverse_f64(area.x1).unwrap() - 100.0).abs() < 1e-9);
        // The y-title grew the left margin, shifting the range start off default 40.
        assert!(area.x0 > 40.0, "grown left margin moved the range start inward");
    }

    #[test]
    fn inverse_scale_transform() {
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
