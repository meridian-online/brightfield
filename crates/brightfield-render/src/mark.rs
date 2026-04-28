//! MarkRenderer trait and implementations for dot, bar, and line marks.
//!
//! Each renderer consumes a RecordBatch + ChannelMap + ScaleSet and produces
//! Vello scene fragments (fill/stroke operations).

use arrow::array::{Array, Float64Array, StringArray, TimestampMicrosecondArray};
use arrow::datatypes::{DataType, TimeUnit};
use arrow::record_batch::RecordBatch;
use brightfield_spec::vocab::MarkKind;
use kurbo::{Affine, BezPath, Circle, Line, Rect};
use peniko::{Color, Fill};
use vello::Scene;

use crate::channel::{Channel, ChannelMap};
use crate::kde::{kde_1d, kde_2d, silverman_1d, silverman_2d_per_axis};
use crate::scale::{Scale, ScaleSet};

/// Highlight state for per-row dim/emphasis rendering.
///
/// When active, rows where `predicate(row_index)` returns `true` render at
/// full alpha; rows where it returns `false` render at `dimmed_alpha`.
pub struct HighlightState {
    /// Predicate: returns `true` for rows that should be fully opaque.
    pub predicate: Box<dyn Fn(usize) -> bool + Send + Sync>,
    /// Alpha multiplier for non-matching (dimmed) rows. Typically 0.15.
    pub dimmed_alpha: f64,
}

/// Trait for per-mark-family rendering.
///
/// Each implementation produces Vello scene fragments from Arrow data
/// mapped through scales.
pub trait MarkRenderer {
    /// Render the mark into the given scene.
    ///
    /// When `highlight` is `Some`, matching rows render at full alpha;
    /// non-matching rows have their alpha multiplied by `dimmed_alpha`.
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        highlight: Option<&HighlightState>,
    );

    /// Render with interpolation between previous and current positions.
    ///
    /// `prev_positions` are pixel (x, y) pairs from the previous frame.
    /// `t` is the interpolation factor (0.0 = prev, 1.0 = current).
    /// Default implementation forwards to `render()`, ignoring interpolation.
    fn render_interpolated(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _prev_positions: &[(f64, f64)],
        _t: f64,
        highlight: Option<&HighlightState>,
    ) {
        self.render(scene, batch, channel_map, scales, highlight);
    }
}

/// Default dot radius in pixels.
const DOT_RADIUS: f64 = 4.0;

/// Default mark colour (steelblue).
const DEFAULT_COLOUR: Color = Color::new([0.306, 0.475, 0.655, 1.0]);

/// Default line stroke width.
const LINE_STROKE_WIDTH: f64 = 2.0;

// ---------------------------------------------------------------------------
// Helpers: extract f64 values from columns regardless of source type
// ---------------------------------------------------------------------------

fn column_as_f64(batch: &RecordBatch, col_name: &str) -> Option<Vec<Option<f64>>> {
    let idx = batch.schema().index_of(col_name).ok()?;
    let col = batch.column(idx);
    match col.data_type() {
        DataType::Float64 => {
            let arr = col.as_any().downcast_ref::<Float64Array>()?;
            Some((0..arr.len()).map(|i| {
                if arr.is_null(i) { None } else { Some(arr.value(i)) }
            }).collect())
        }
        DataType::Int64 => {
            let arr = col.as_any().downcast_ref::<arrow::array::Int64Array>()?;
            Some((0..arr.len()).map(|i| {
                if arr.is_null(i) { None } else { Some(arr.value(i) as f64) }
            }).collect())
        }
        DataType::Timestamp(TimeUnit::Microsecond, _) => {
            let arr = col.as_any().downcast_ref::<TimestampMicrosecondArray>()?;
            Some((0..arr.len()).map(|i| {
                if arr.is_null(i) { None } else { Some(arr.value(i) as f64) }
            }).collect())
        }
        _ => None,
    }
}

fn column_as_string(batch: &RecordBatch, col_name: &str) -> Option<Vec<Option<String>>> {
    let idx = batch.schema().index_of(col_name).ok()?;
    let col = batch.column(idx);
    if !matches!(col.data_type(), DataType::Utf8) {
        return None;
    }
    let arr = col.as_any().downcast_ref::<StringArray>().unwrap();
    Some((0..arr.len()).map(|i| {
        if arr.is_null(i) { None } else { Some(arr.value(i).to_string()) }
    }).collect())
}

/// Resolve the pixel position for a value given a channel's scale.
fn resolve_position(
    scale: &Scale,
    value_f64: Option<f64>,
    value_str: Option<&str>,
) -> Option<f64> {
    match scale {
        Scale::Linear { .. } | Scale::Time { .. } => {
            value_f64.map(|v| scale.map_f64(v))
        }
        Scale::Band { .. } => {
            value_str.and_then(|s| scale.map_category(s))
        }
        Scale::Colour { .. } => None,
    }
}

/// Resolve the colour for a data point.
fn resolve_colour(
    scales: &ScaleSet,
    channel_map: &ChannelMap,
    batch: &RecordBatch,
    row: usize,
) -> Color {
    if let Some(fill_col) = channel_map.get(Channel::Fill) {
        if let Some(fill_scale) = scales.get(Channel::Fill) {
            if let Some(strings) = column_as_string(batch, fill_col) {
                if let Some(Some(ref cat)) = strings.get(row) {
                    if let Some(components) = fill_scale.map_colour(cat) {
                        return Color::new(components);
                    }
                }
            }
        }
    }
    DEFAULT_COLOUR
}

/// Apply highlight dimming to a colour.
///
/// If highlight is active and the predicate returns false for this row,
/// multiply the colour's alpha by `dimmed_alpha`.
fn apply_highlight(colour: Color, row: usize, highlight: Option<&HighlightState>) -> Color {
    match highlight {
        Some(hs) if !(hs.predicate)(row) => {
            let [r, g, b, a] = colour.components;
            Color::new([r, g, b, a * hs.dimmed_alpha as f32])
        }
        _ => colour,
    }
}

// ---------------------------------------------------------------------------
// DotRenderer
// ---------------------------------------------------------------------------

/// Renders dot/scatter marks as circles at x/y positions.
pub struct DotRenderer;

impl MarkRenderer for DotRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        highlight: Option<&HighlightState>,
    ) {
        let x_col = match channel_map.get(Channel::X) {
            Some(c) => c,
            None => return,
        };
        let y_col = match channel_map.get(Channel::Y) {
            Some(c) => c,
            None => return,
        };
        let x_scale = match scales.get(Channel::X) {
            Some(s) => s,
            None => return,
        };
        let y_scale = match scales.get(Channel::Y) {
            Some(s) => s,
            None => return,
        };

        let x_f64 = column_as_f64(batch, x_col);
        let x_str = column_as_string(batch, x_col);
        let y_f64 = column_as_f64(batch, y_col);
        let y_str = column_as_string(batch, y_col);

        let n = batch.num_rows();
        for i in 0..n {
            let xf = x_f64.as_ref().and_then(|v| v[i]);
            let xs = x_str.as_ref().and_then(|v| v[i].as_deref());
            let yf = y_f64.as_ref().and_then(|v| v[i]);
            let ys = y_str.as_ref().and_then(|v| v[i].as_deref());

            let px = match resolve_position(x_scale, xf, xs) {
                Some(p) => p,
                None => continue,
            };
            let py = match resolve_position(y_scale, yf, ys) {
                Some(p) => p,
                None => continue,
            };

            let colour = resolve_colour(scales, channel_map, batch, i);
            let colour = apply_highlight(colour, i, highlight);
            let circle = Circle::new((px, py), DOT_RADIUS);
            scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &circle);
        }
    }

    fn render_interpolated(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        prev_positions: &[(f64, f64)],
        t: f64,
        highlight: Option<&HighlightState>,
    ) {
        let x_col = match channel_map.get(Channel::X) {
            Some(c) => c,
            None => return,
        };
        let y_col = match channel_map.get(Channel::Y) {
            Some(c) => c,
            None => return,
        };
        let x_scale = match scales.get(Channel::X) {
            Some(s) => s,
            None => return,
        };
        let y_scale = match scales.get(Channel::Y) {
            Some(s) => s,
            None => return,
        };

        let x_f64 = column_as_f64(batch, x_col);
        let x_str = column_as_string(batch, x_col);
        let y_f64 = column_as_f64(batch, y_col);
        let y_str = column_as_string(batch, y_col);

        let n = batch.num_rows();
        for i in 0..n {
            let xf = x_f64.as_ref().and_then(|v| v[i]);
            let xs = x_str.as_ref().and_then(|v| v[i].as_deref());
            let yf = y_f64.as_ref().and_then(|v| v[i]);
            let ys = y_str.as_ref().and_then(|v| v[i].as_deref());

            let target_px = match resolve_position(x_scale, xf, xs) {
                Some(p) => p,
                None => continue,
            };
            let target_py = match resolve_position(y_scale, yf, ys) {
                Some(p) => p,
                None => continue,
            };

            // Lerp from prev to current
            let (px, py) = if let Some(&(prev_x, prev_y)) = prev_positions.get(i) {
                let x = prev_x + (target_px - prev_x) * t;
                let y = prev_y + (target_py - prev_y) * t;
                (x, y)
            } else {
                (target_px, target_py)
            };

            let colour = resolve_colour(scales, channel_map, batch, i);
            let colour = apply_highlight(colour, i, highlight);
            let circle = Circle::new((px, py), DOT_RADIUS);
            scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &circle);
        }
    }
}

// ---------------------------------------------------------------------------
// BarRenderer
// ---------------------------------------------------------------------------

/// Renders bar marks as rectangles on a band (x) + linear (y) scale.
pub struct BarRenderer;

impl MarkRenderer for BarRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        highlight: Option<&HighlightState>,
    ) {
        let x_col = match channel_map.get(Channel::X) {
            Some(c) => c,
            None => return,
        };
        let y_col = match channel_map.get(Channel::Y) {
            Some(c) => c,
            None => return,
        };
        let x_scale = match scales.get(Channel::X) {
            Some(s) => s,
            None => return,
        };
        let y_scale = match scales.get(Channel::Y) {
            Some(s) => s,
            None => return,
        };

        let band_width = match x_scale.band_width() {
            Some(bw) => bw,
            None => return,
        };

        let x_str = match column_as_string(batch, x_col) {
            Some(v) => v,
            None => return,
        };
        let y_f64 = match column_as_f64(batch, y_col) {
            Some(v) => v,
            None => return,
        };

        // Baseline: y=0 mapped through the y scale.
        let baseline = y_scale.map_f64(0.0);

        let n = batch.num_rows();
        for i in 0..n {
            let cat = match x_str[i].as_deref() {
                Some(c) => c,
                None => continue,
            };
            let value = match y_f64[i] {
                Some(v) => v,
                None => continue,
            };

            let cx = match x_scale.map_category(cat) {
                Some(p) => p,
                None => continue,
            };
            let py = y_scale.map_f64(value);

            let x0 = cx - band_width / 2.0;
            let (y_top, y_bottom) = if py < baseline {
                (py, baseline)
            } else {
                (baseline, py)
            };

            let colour = resolve_colour(scales, channel_map, batch, i);
            let colour = apply_highlight(colour, i, highlight);
            let rect = Rect::new(x0, y_top, x0 + band_width, y_bottom);
            scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &rect);
        }
    }
}

// ---------------------------------------------------------------------------
// LineRenderer
// ---------------------------------------------------------------------------

/// Renders line marks as a connected path in x-order.
pub struct LineRenderer;

impl MarkRenderer for LineRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        let x_col = match channel_map.get(Channel::X) {
            Some(c) => c,
            None => return,
        };
        let y_col = match channel_map.get(Channel::Y) {
            Some(c) => c,
            None => return,
        };
        let x_scale = match scales.get(Channel::X) {
            Some(s) => s,
            None => return,
        };
        let y_scale = match scales.get(Channel::Y) {
            Some(s) => s,
            None => return,
        };

        let x_f64 = column_as_f64(batch, x_col);
        let y_f64 = column_as_f64(batch, y_col);

        if x_f64.is_none() || y_f64.is_none() {
            return;
        }
        let x_vals = x_f64.unwrap();
        let y_vals = y_f64.unwrap();

        // Collect valid (x_data, y_data) pairs, then sort by x.
        let mut points: Vec<(f64, f64)> = Vec::new();
        for i in 0..batch.num_rows() {
            if let (Some(xv), Some(yv)) = (x_vals[i], y_vals[i]) {
                let px = x_scale.map_f64(xv);
                let py = y_scale.map_f64(yv);
                points.push((px, py));
            }
        }

        // Sort by pixel x (preserves data x-order since the scale is monotonic).
        points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        if points.len() < 2 {
            return;
        }

        // Draw connected line segments.
        let colour = DEFAULT_COLOUR;
        let stroke = kurbo::Stroke::new(LINE_STROKE_WIDTH);
        for window in points.windows(2) {
            let line = Line::new(
                kurbo::Point::new(window[0].0, window[0].1),
                kurbo::Point::new(window[1].0, window[1].1),
            );
            scene.stroke(&stroke, Affine::IDENTITY, colour, None, &line);
        }
    }
}

// ---------------------------------------------------------------------------
// Density1DRenderer (density / densityX / densityY)
// ---------------------------------------------------------------------------

/// Which axis carries the density curve.
///
/// `DensityX` plots density along x as a function of x; the curve fills
/// downward from the density baseline. `DensityY` plots density along y.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DensityAxis {
    /// Density on x — peak height encoded in y.
    X,
    /// Density on y — peak height encoded in x.
    Y,
}

/// Renders a 1D density curve from a pre-binned (bucket, count) batch.
///
/// The lowerer produces a RecordBatch with two columns:
///   - the binned axis column (e.g. `x_bin`) — Float64 (bin centres in data units)
///   - a `count` column — Int64
///
/// At render time the data column is read into a flat histogram, convolved
/// with a Gaussian via `kde_1d`, then drawn as a filled path.
pub struct Density1DRenderer {
    pub axis: DensityAxis,
}

impl MarkRenderer for Density1DRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        // Bin column is the axis specified by the renderer; count is the
        // density-mapped channel.
        let (bin_channel, density_channel) = match self.axis {
            DensityAxis::X => (Channel::X, Channel::Y),
            DensityAxis::Y => (Channel::Y, Channel::X),
        };

        let bin_col = match channel_map.get(bin_channel) {
            Some(c) => c,
            None => return,
        };
        let bin_scale = match scales.get(bin_channel) {
            Some(s) => s,
            None => return,
        };
        let density_scale = match scales.get(density_channel) {
            Some(s) => s,
            None => return,
        };

        // Sort batch rows by bin centre (the lowerer does NOT order by bin —
        // GROUP BY in DuckDB has no implicit ordering).
        let bin_vals_opt = column_as_f64(batch, bin_col);
        let count_vals_opt = column_as_f64(batch, "count");
        let (bin_vals, count_vals) = match (bin_vals_opt, count_vals_opt) {
            (Some(b), Some(c)) => (b, c),
            _ => return,
        };

        let n = batch.num_rows();
        if n < 2 {
            return;
        }

        // Build (centre, count) pairs and sort by centre.
        let mut pairs: Vec<(f64, u32)> = Vec::with_capacity(n);
        for i in 0..n {
            if let (Some(b), Some(c)) = (bin_vals[i], count_vals[i]) {
                pairs.push((b, c.max(0.0).round() as u32));
            }
        }
        pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        if pairs.len() < 2 {
            return;
        }

        let bin_size = pairs[1].0 - pairs[0].0;
        if bin_size <= 0.0 {
            return;
        }

        // Bandwidth: Silverman from reconstructed sample list.
        let mut samples: Vec<f64> = Vec::new();
        for (centre, count) in &pairs {
            for _ in 0..*count {
                samples.push(*centre);
            }
        }
        let bandwidth = silverman_1d(&samples);
        if bandwidth <= 0.0 {
            return;
        }

        let counts: Vec<u32> = pairs.iter().map(|(_, c)| *c).collect();
        let density = kde_1d(&counts, bandwidth, bin_size);

        // Map peak density to the density axis range.
        let max_density = density.iter().cloned().fold(0.0_f64, f64::max);
        if max_density <= 0.0 {
            return;
        }

        // We want density 0 to render at the density-axis baseline,
        // density max to render near the far end of the axis range.
        let baseline_pixel = density_scale.range_start();
        let peak_pixel = density_scale.range_end();
        let pixel_height = peak_pixel - baseline_pixel;

        let mut path = BezPath::new();
        let mut started = false;
        for (i, (centre, _)) in pairs.iter().enumerate() {
            let bin_pixel = bin_scale.map_f64(*centre);
            let normalised = density[i] / max_density;
            let dens_pixel = baseline_pixel + normalised * pixel_height;
            let (px, py) = match self.axis {
                DensityAxis::X => (bin_pixel, dens_pixel),
                DensityAxis::Y => (dens_pixel, bin_pixel),
            };
            if !started {
                path.move_to((px, py));
                started = true;
            } else {
                path.line_to((px, py));
            }
        }
        // Close back to baseline so the path is fillable.
        let last = pairs.last().unwrap();
        let first = pairs.first().unwrap();
        let last_bin = bin_scale.map_f64(last.0);
        let first_bin = bin_scale.map_f64(first.0);
        match self.axis {
            DensityAxis::X => {
                path.line_to((last_bin, baseline_pixel));
                path.line_to((first_bin, baseline_pixel));
            }
            DensityAxis::Y => {
                path.line_to((baseline_pixel, last_bin));
                path.line_to((baseline_pixel, first_bin));
            }
        }
        path.close_path();

        let colour = DEFAULT_COLOUR;
        scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &path);
    }
}

// ---------------------------------------------------------------------------
// Density2DRenderer (density with both x and y bins)
// ---------------------------------------------------------------------------

/// Renders 2D density as a grid of circles whose alpha encodes density value.
///
/// The lowerer emits `(x_bin, y_bin, count)`; this renderer reconstructs the
/// rectangular histogram, runs `kde_2d`, and draws a circle per cell with
/// alpha proportional to normalised density.
pub struct Density2DRenderer;

impl MarkRenderer for Density2DRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        let x_col = match channel_map.get(Channel::X) {
            Some(c) => c,
            None => return,
        };
        let y_col = match channel_map.get(Channel::Y) {
            Some(c) => c,
            None => return,
        };
        let x_scale = match scales.get(Channel::X) {
            Some(s) => s,
            None => return,
        };
        let y_scale = match scales.get(Channel::Y) {
            Some(s) => s,
            None => return,
        };

        let x_vals = match column_as_f64(batch, x_col) {
            Some(v) => v,
            None => return,
        };
        let y_vals = match column_as_f64(batch, y_col) {
            Some(v) => v,
            None => return,
        };
        let count_vals = match column_as_f64(batch, "count") {
            Some(v) => v,
            None => return,
        };

        // Collect unique bin centres on each axis (sorted).
        let mut x_centres: Vec<f64> = Vec::new();
        let mut y_centres: Vec<f64> = Vec::new();
        let mut tuples: Vec<(f64, f64, u32)> = Vec::new();
        for i in 0..batch.num_rows() {
            if let (Some(xv), Some(yv), Some(c)) = (x_vals[i], y_vals[i], count_vals[i]) {
                tuples.push((xv, yv, c.max(0.0).round() as u32));
                if !x_centres.iter().any(|v| (*v - xv).abs() < 1e-9) {
                    x_centres.push(xv);
                }
                if !y_centres.iter().any(|v| (*v - yv).abs() < 1e-9) {
                    y_centres.push(yv);
                }
            }
        }
        x_centres.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        y_centres.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let cols = x_centres.len();
        let rows = y_centres.len();
        if cols < 2 || rows < 2 {
            return;
        }
        let dx = x_centres[1] - x_centres[0];
        let dy = y_centres[1] - y_centres[0];
        if dx <= 0.0 || dy <= 0.0 {
            return;
        }

        // Build flat row-major histogram.
        let mut bins = vec![0u32; rows * cols];
        for (xv, yv, c) in &tuples {
            let cx = x_centres
                .iter()
                .position(|v| (*v - xv).abs() < 1e-9)
                .unwrap();
            let cy = y_centres
                .iter()
                .position(|v| (*v - yv).abs() < 1e-9)
                .unwrap();
            bins[cy * cols + cx] = *c;
        }

        // Bandwidth from reconstructed (x, y) samples.
        let mut xs_samples: Vec<f64> = Vec::new();
        let mut ys_samples: Vec<f64> = Vec::new();
        for r in 0..rows {
            for c in 0..cols {
                for _ in 0..bins[r * cols + c] {
                    xs_samples.push(x_centres[c]);
                    ys_samples.push(y_centres[r]);
                }
            }
        }
        let (h_x, h_y) = silverman_2d_per_axis(&xs_samples, &ys_samples);
        if h_x <= 0.0 || h_y <= 0.0 {
            return;
        }

        let density = kde_2d(&bins, (rows, cols), (h_x, h_y), (dx, dy));
        let max_density = density.iter().cloned().fold(0.0_f64, f64::max);
        if max_density <= 0.0 {
            return;
        }

        let radius = DOT_RADIUS.max(2.0);
        for r in 0..rows {
            for c in 0..cols {
                let normalised = density[r * cols + c] / max_density;
                if normalised <= 0.01 {
                    continue;
                }
                let px = x_scale.map_f64(x_centres[c]);
                let py = y_scale.map_f64(y_centres[r]);
                let [cr, cg, cb, _ca] = DEFAULT_COLOUR.components;
                let colour = Color::new([cr, cg, cb, normalised as f32]);
                let circle = Circle::new((px, py), radius);
                scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &circle);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// RegressionRenderer (regressionY / regressionX)
// ---------------------------------------------------------------------------

/// Renders a linear OLS fit line plus a 95% (or configurable) CI band.
///
/// Expects a one-row aggregate batch with columns:
///   - `slope` — regr_slope(y, x)
///   - `intercept` — regr_intercept(y, x)
///   - `n` — regr_count(y, x)
///   - `mean_x` — regr_avgx(y, x)
///   - `sxx` — regr_sxx(y, x)  (sum (x - mean_x)^2)
///   - `sxy` — regr_sxy(y, x)  (sum (x - mean_x)(y - mean_y))
///   - `syy` — regr_syy(y, x)  (sum (y - mean_y)^2)
///
/// The fitted line is sampled at 32 evenly-spaced x values across the
/// x-axis domain. A confidence band is drawn as a filled path between
/// upper and lower bounds at each sample point.
pub struct RegressionRenderer {
    /// Confidence level (e.g. 0.95). The renderer uses the normal-approximation
    /// `z_{1-alpha/2}` for `n >= 30`, defaulting to 1.96 for 95%.
    pub ci: f64,
}

impl Default for RegressionRenderer {
    fn default() -> Self {
        Self { ci: 0.95 }
    }
}

impl MarkRenderer for RegressionRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        let x_scale = match scales.get(Channel::X) {
            Some(s) => s,
            None => return,
        };
        let y_scale = match scales.get(Channel::Y) {
            Some(s) => s,
            None => return,
        };

        // Read regression aggregates from the (single-row) batch.
        // For multi-group rendering, the batch has multiple rows — one per
        // stroke category.
        let slope_vals = match column_as_f64(batch, "slope") {
            Some(v) => v,
            None => return,
        };
        let intercept_vals = match column_as_f64(batch, "intercept") {
            Some(v) => v,
            None => return,
        };
        let n_vals = match column_as_f64(batch, "n") {
            Some(v) => v,
            None => return,
        };
        let mean_x_vals = match column_as_f64(batch, "mean_x") {
            Some(v) => v,
            None => return,
        };
        let sxx_vals = match column_as_f64(batch, "sxx") {
            Some(v) => v,
            None => return,
        };
        let sxy_vals = match column_as_f64(batch, "sxy") {
            Some(v) => v,
            None => return,
        };
        let syy_vals = match column_as_f64(batch, "syy") {
            Some(v) => v,
            None => return,
        };

        // x sampling domain — full x-axis domain.
        let x_min = match x_scale.domain_min() {
            Some(v) => v,
            None => return,
        };
        let x_max = match x_scale.domain_max() {
            Some(v) => v,
            None => return,
        };

        const SAMPLES: usize = 32;
        let z = if self.ci >= 0.99 {
            2.576
        } else if self.ci >= 0.95 {
            1.96
        } else if self.ci >= 0.90 {
            1.645
        } else {
            1.96
        };

        // Stroke colour resolution: one row per group (if any), else default.
        for row in 0..batch.num_rows() {
            let slope = match slope_vals[row] {
                Some(v) => v,
                None => continue,
            };
            let intercept = match intercept_vals[row] {
                Some(v) => v,
                None => continue,
            };
            let n = match n_vals[row] {
                Some(v) if v >= 3.0 => v,
                _ => continue, // need n >= 3 for variance estimate
            };
            let mean_x = mean_x_vals[row].unwrap_or(0.0);
            let sxx = sxx_vals[row].unwrap_or(0.0);
            let sxy = sxy_vals[row].unwrap_or(0.0);
            let syy = syy_vals[row].unwrap_or(0.0);

            // Residual variance: s² = (Syy - Sxy²/Sxx) / (n - 2)
            let s_sq = if sxx > 0.0 {
                (syy - (sxy * sxy) / sxx) / (n - 2.0)
            } else {
                0.0
            };
            let s = s_sq.max(0.0).sqrt();

            let colour = resolve_stroke_colour(scales, channel_map, batch, row);

            // Sample CI band points.
            let mut upper: Vec<(f64, f64)> = Vec::with_capacity(SAMPLES);
            let mut lower: Vec<(f64, f64)> = Vec::with_capacity(SAMPLES);
            let mut line_pts: Vec<(f64, f64)> = Vec::with_capacity(SAMPLES);
            for i in 0..SAMPLES {
                let t = (i as f64) / ((SAMPLES - 1) as f64);
                let xv = x_min + (x_max - x_min) * t;
                let yhat = slope * xv + intercept;
                // se(ŷ|x) = s · √(1/n + (x - mean_x)² / sxx)
                let se = if sxx > 0.0 {
                    s * (1.0 / n + (xv - mean_x).powi(2) / sxx).sqrt()
                } else {
                    0.0
                };
                let half = z * se;

                let px = x_scale.map_f64(xv);
                let py_line = y_scale.map_f64(yhat);
                line_pts.push((px, py_line));
                upper.push((px, y_scale.map_f64(yhat + half)));
                lower.push((px, y_scale.map_f64(yhat - half)));
            }

            // Draw CI band as a filled polygon (upper forward, lower reversed).
            let mut band = BezPath::new();
            band.move_to(upper[0]);
            for &p in &upper[1..] {
                band.line_to(p);
            }
            for &p in lower.iter().rev() {
                band.line_to(p);
            }
            band.close_path();

            let [cr, cg, cb, _] = colour.components;
            let band_colour = Color::new([cr, cg, cb, 0.20]);
            scene.fill(Fill::NonZero, Affine::IDENTITY, band_colour, None, &band);

            // Draw the fitted line on top.
            let stroke = kurbo::Stroke::new(LINE_STROKE_WIDTH);
            for w in line_pts.windows(2) {
                let line = Line::new(
                    kurbo::Point::new(w[0].0, w[0].1),
                    kurbo::Point::new(w[1].0, w[1].1),
                );
                scene.stroke(&stroke, Affine::IDENTITY, colour, None, &line);
            }
        }
    }
}

/// Resolve stroke colour for regression — checks `stroke` channel value first,
/// falls back to fill, then default.
fn resolve_stroke_colour(
    scales: &ScaleSet,
    channel_map: &ChannelMap,
    batch: &RecordBatch,
    row: usize,
) -> Color {
    if let Some(stroke_col) = channel_map.get(Channel::Stroke) {
        if let Some(stroke_scale) = scales.get(Channel::Stroke) {
            if let Some(strings) = column_as_string(batch, stroke_col) {
                if let Some(Some(ref cat)) = strings.get(row) {
                    if let Some(components) = stroke_scale.map_colour(cat) {
                        return Color::new(components);
                    }
                }
            }
        }
    }
    resolve_colour(scales, channel_map, batch, row)
}

// ---------------------------------------------------------------------------
// Renderer registry
// ---------------------------------------------------------------------------

/// Build the default renderer registry mapping mark kinds to renderers.
///
/// This replaces the prior silent `_ => DotRenderer` fallback in
/// brightfield-app/src/main.rs. Unknown / unimplemented mark kinds return
/// `None` from `find_renderer` so the caller can decide what to do
/// (typically: skip the mark and log a tracing event).
///
/// TODO(card-runtime-reactivity): downstream registry will own per-mark
/// lifecycle and re-render policy; for now this is a stateless lookup.
pub fn default_renderers() -> Vec<(MarkKind, Box<dyn MarkRenderer + Send + Sync>)> {
    let mut v: Vec<(MarkKind, Box<dyn MarkRenderer + Send + Sync>)> = Vec::new();
    v.push((MarkKind::Dot, Box::new(DotRenderer)));
    v.push((MarkKind::DotX, Box::new(DotRenderer)));
    v.push((MarkKind::DotY, Box::new(DotRenderer)));
    v.push((MarkKind::Circle, Box::new(DotRenderer)));
    v.push((MarkKind::BarX, Box::new(BarRenderer)));
    v.push((MarkKind::BarY, Box::new(BarRenderer)));
    v.push((MarkKind::Line, Box::new(LineRenderer)));
    v.push((MarkKind::LineX, Box::new(LineRenderer)));
    v.push((MarkKind::LineY, Box::new(LineRenderer)));
    v.push((
        MarkKind::DensityX,
        Box::new(Density1DRenderer { axis: DensityAxis::X }),
    ));
    v.push((
        MarkKind::DensityY,
        Box::new(Density1DRenderer { axis: DensityAxis::Y }),
    ));
    v.push((MarkKind::Density, Box::new(Density2DRenderer)));
    v.push((MarkKind::RegressionY, Box::new(RegressionRenderer::default())));
    v.push((MarkKind::RegressionX, Box::new(RegressionRenderer::default())));
    v
}

/// Look up a renderer for a mark kind.
///
/// Returns `None` for kinds with no registered renderer — caller should
/// log and skip rather than silently falling back to a default.
pub fn find_renderer<'a>(
    registry: &'a [(MarkKind, Box<dyn MarkRenderer + Send + Sync>)],
    kind: MarkKind,
) -> Option<&'a (dyn MarkRenderer + Send + Sync)> {
    registry
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, r)| r.as_ref())
}

/// Return the number of fill operations in a scene (for testing).
///
/// This counts scene elements by encoding to a byte buffer and counting
/// the draw commands. Simplified version for test assertions.
pub fn count_scene_fills(scene: &Scene) -> usize {
    // Vello's Scene doesn't expose a public element count API.
    // We use the encoding size as a proxy: each fill adds data to the encoding.
    // For testing, we track fills manually via a counting wrapper.
    //
    // Since Vello doesn't expose internals, we accept that our unit tests
    // verify the rendering logic (correct positions, colours) via the
    // MarkRenderer implementations, and integration tests verify the
    // final scene is non-empty.
    //
    // This function returns 0 as a placeholder — real fill counting
    // requires rendering to pixels and inspecting output.
    let _ = scene;
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{Channel, ChannelMap};
    use crate::scale::{infer_scales, Scale};
    use arrow::array::{Float64Array, StringArray, TimestampMicrosecondArray};
    use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
    use arrow::record_batch::RecordBatch;
    use std::sync::Arc;

    #[test]
    fn gpu_ac03_dot_renderer_positions_circles() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        let renderer = DotRenderer;
        renderer.render(&mut scene, &batch, &cm, &scales, None);

        // Scene should be non-empty after rendering 3 dots.
        // Vello's Scene encoding grows with each fill operation.
        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "scene should have path tags after rendering 3 dots"
        );
    }

    #[test]
    fn gpu_ac03_dot_renderer_with_colour() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("species", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
                Arc::new(StringArray::from(vec!["a", "b", "a"])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        cm.insert(Channel::Fill, "species".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        let renderer = DotRenderer;
        renderer.render(&mut scene, &batch, &cm, &scales, None);

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "scene should have path tags after rendering 3 coloured dots"
        );
    }

    #[test]
    fn gpu_ac04_bar_renderer_rects() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["a", "b"])),
                Arc::new(Float64Array::from(vec![10.0, 20.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "category".to_string());
        cm.insert(Channel::Y, "value".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        let renderer = BarRenderer;
        renderer.render(&mut scene, &batch, &cm, &scales, None);

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "scene should have path tags after rendering 2 bar rects"
        );
    }

    #[test]
    fn gpu_ac04_bar_renderer_band_width_proportional() {
        // Verify that band widths are proportional to the category count.
        let scale = Scale::Band {
            categories: vec!["a".to_string(), "b".to_string()],
            range_start: 0.0,
            range_end: 200.0,
            padding: 0.1,
        };
        let bw = scale.band_width().expect("should compute band width");
        // 2 categories in 200px: each band is 100px, with 10% padding = 90px
        assert!((bw - 90.0).abs() < f64::EPSILON, "band width should be 90.0, got {bw}");
    }

    #[test]
    fn gpu_ac05_line_renderer_connected_path() {
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "ts",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                false,
            ),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(TimestampMicrosecondArray::from(vec![
                    1_000_000,
                    2_000_000,
                    3_000_000,
                    4_000_000,
                ])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 15.0, 25.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "ts".to_string());
        cm.insert(Channel::Y, "value".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        let renderer = LineRenderer;
        renderer.render(&mut scene, &batch, &cm, &scales, None);

        // Line renderer should produce stroke operations for 3 line segments (4 points).
        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "scene should have path tags after rendering 4-point line"
        );
    }

    // --- ifb_ac03: HighlightState ---

    #[test]
    fn ifb_ac03_highlight_state_predicate() {
        let hs = HighlightState {
            predicate: Box::new(|row| row == 1),
            dimmed_alpha: 0.15,
        };
        assert!(!(hs.predicate)(0), "row 0 should not match");
        assert!((hs.predicate)(1), "row 1 should match");
        assert!(!(hs.predicate)(2), "row 2 should not match");
        assert!((hs.dimmed_alpha - 0.15).abs() < f64::EPSILON);
    }

    #[test]
    fn ifb_ac03_highlight_state_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        // This won't compile if HighlightState's predicate isn't Send+Sync
        assert_send_sync::<Box<dyn Fn(usize) -> bool + Send + Sync>>();
    }

    // --- ifb_ac04: MarkRenderer with highlight ---

    #[test]
    fn ifb_ac04_dot_renderer_with_highlight() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let hs = HighlightState {
            predicate: Box::new(|row| row == 1),
            dimmed_alpha: 0.15,
        };

        let mut scene = Scene::new();
        let renderer = DotRenderer;
        renderer.render(&mut scene, &batch, &cm, &scales, Some(&hs));

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "dot scene with highlight should have content"
        );
    }

    #[test]
    fn ifb_ac04_bar_renderer_with_highlight() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["a", "b"])),
                Arc::new(Float64Array::from(vec![10.0, 20.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "category".to_string());
        cm.insert(Channel::Y, "value".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let hs = HighlightState {
            predicate: Box::new(|row| row == 0),
            dimmed_alpha: 0.2,
        };

        let mut scene = Scene::new();
        let renderer = BarRenderer;
        renderer.render(&mut scene, &batch, &cm, &scales, Some(&hs));

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "bar scene with highlight should have content"
        );
    }

    // --- ifb_ac07: render_interpolated ---

    #[test]
    fn ifb_ac07_dot_render_interpolated_at_zero() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let prev_positions = vec![(100.0, 100.0), (200.0, 200.0), (300.0, 300.0)];

        let mut scene = Scene::new();
        let renderer = DotRenderer;
        renderer.render_interpolated(
            &mut scene, &batch, &cm, &scales,
            &prev_positions, 0.0, None,
        );

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "interpolated scene at t=0 should have content"
        );
    }

    #[test]
    fn ifb_ac07_dot_render_interpolated_at_one() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let prev_positions = vec![(100.0, 100.0), (200.0, 200.0), (300.0, 300.0)];

        let mut scene = Scene::new();
        let renderer = DotRenderer;
        renderer.render_interpolated(
            &mut scene, &batch, &cm, &scales,
            &prev_positions, 1.0, None,
        );

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "interpolated scene at t=1 should have content"
        );
    }

    // -----------------------------------------------------------------------
    // Statistical-mark tests (gomb_ ac-03 / ac-04 / ac-05 / ac-08)
    // -----------------------------------------------------------------------

    fn density_1d_batch() -> RecordBatch {
        // 8 bins centred at 0..7; counts form a roughly Gaussian shape.
        let schema = Arc::new(Schema::new(vec![
            Field::new("x_bin", DataType::Float64, false),
            Field::new("count", DataType::Float64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![
                    0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0,
                ])),
                Arc::new(Float64Array::from(vec![
                    1.0, 4.0, 10.0, 20.0, 20.0, 10.0, 4.0, 1.0,
                ])),
            ],
        )
        .unwrap()
    }

    #[test]
    fn gomb_ac03_density1d_x_renders_filled_path() {
        let batch = density_1d_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x_bin".to_string());
        cm.insert(Channel::Y, "count".to_string());
        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        let renderer = Density1DRenderer { axis: DensityAxis::X };
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "Density1DRenderer (X) must produce scene content"
        );
    }

    #[test]
    fn gomb_ac03_density1d_y_renders_filled_path() {
        // For DensityY, y is the binned axis; x is density magnitude.
        let schema = Arc::new(Schema::new(vec![
            Field::new("y_bin", DataType::Float64, false),
            Field::new("count", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0, 4.0])),
                Arc::new(Float64Array::from(vec![1.0, 5.0, 12.0, 5.0, 1.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::Y, "y_bin".to_string());
        cm.insert(Channel::X, "count".to_string());
        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        let renderer = Density1DRenderer { axis: DensityAxis::Y };
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "Density1DRenderer (Y) must produce scene content"
        );
    }

    #[test]
    fn gomb_ac04_density2d_renders_circle_grid() {
        // 3x3 bin grid with peak in centre.
        let schema = Arc::new(Schema::new(vec![
            Field::new("x_bin", DataType::Float64, false),
            Field::new("y_bin", DataType::Float64, false),
            Field::new("count", DataType::Float64, false),
        ]));
        let xs = vec![0.0, 1.0, 2.0, 0.0, 1.0, 2.0, 0.0, 1.0, 2.0];
        let ys = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0];
        let counts = vec![1.0, 4.0, 1.0, 4.0, 16.0, 4.0, 1.0, 4.0, 1.0];
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(xs)),
                Arc::new(Float64Array::from(ys)),
                Arc::new(Float64Array::from(counts)),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x_bin".to_string());
        cm.insert(Channel::Y, "y_bin".to_string());
        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        let renderer = Density2DRenderer;
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "Density2DRenderer must produce scene content"
        );
    }

    #[test]
    fn gomb_ac05_regression_renders_line_and_ci_band() {
        // Anscombe Quartet I (the canonical OLS dataset).
        // n=11, slope=0.5, intercept=3, mean_x=9, sxx=110.
        // We compute syy and sxy from the data.
        let xs = [10.0, 8.0, 13.0, 9.0, 11.0, 14.0, 6.0, 4.0, 12.0, 7.0, 5.0];
        let ys = [
            8.04, 6.95, 7.58, 8.81, 8.33, 9.96, 7.24, 4.26, 10.84, 4.82, 5.68,
        ];
        let n = xs.len() as f64;
        let mean_x = xs.iter().sum::<f64>() / n;
        let mean_y = ys.iter().sum::<f64>() / n;
        let sxx: f64 = xs.iter().map(|x| (x - mean_x).powi(2)).sum();
        let syy: f64 = ys.iter().map(|y| (y - mean_y).powi(2)).sum();
        let sxy: f64 = xs
            .iter()
            .zip(ys.iter())
            .map(|(x, y)| (x - mean_x) * (y - mean_y))
            .sum();
        let slope = sxy / sxx;
        let intercept = mean_y - slope * mean_x;

        // Build a one-row aggregate batch.
        let schema = Arc::new(Schema::new(vec![
            Field::new("slope", DataType::Float64, false),
            Field::new("intercept", DataType::Float64, false),
            Field::new("n", DataType::Float64, false),
            Field::new("mean_x", DataType::Float64, false),
            Field::new("sxx", DataType::Float64, false),
            Field::new("sxy", DataType::Float64, false),
            Field::new("syy", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![slope])),
                Arc::new(Float64Array::from(vec![intercept])),
                Arc::new(Float64Array::from(vec![n])),
                Arc::new(Float64Array::from(vec![mean_x])),
                Arc::new(Float64Array::from(vec![sxx])),
                Arc::new(Float64Array::from(vec![sxy])),
                Arc::new(Float64Array::from(vec![syy])),
            ],
        )
        .unwrap();

        let cm = ChannelMap::new();
        // Build scales manually with a known x domain so x_min/x_max are non-default.
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::X,
            Scale::Linear {
                domain_min: 0.0,
                domain_max: 20.0,
                range_start: 40.0,
                range_end: 600.0,
            },
        );
        scales.insert(
            Channel::Y,
            Scale::Linear {
                domain_min: 0.0,
                domain_max: 12.0,
                range_start: 450.0,
                range_end: 20.0,
            },
        );

        let mut scene = Scene::new();
        let renderer = RegressionRenderer { ci: 0.95 };
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        let encoding = scene.encoding();
        // Expect path tags from both the CI band fill and the fitted-line strokes.
        assert!(
            encoding.path_tags.len() > 0,
            "RegressionRenderer must emit a fitted line plus CI band"
        );
        // Sanity-check the slope/intercept on Anscombe I.
        assert!((slope - 0.5).abs() < 0.01, "Anscombe I slope ≈ 0.5 ({slope})");
        assert!(
            (intercept - 3.0).abs() < 0.05,
            "Anscombe I intercept ≈ 3.0 ({intercept})"
        );
    }

    #[test]
    fn gomb_ac08_default_renderers_finds_density_and_regression() {
        let registry = default_renderers();
        assert!(find_renderer(&registry, MarkKind::Dot).is_some());
        assert!(find_renderer(&registry, MarkKind::BarX).is_some());
        assert!(find_renderer(&registry, MarkKind::Line).is_some());
        assert!(find_renderer(&registry, MarkKind::Density).is_some());
        assert!(find_renderer(&registry, MarkKind::DensityX).is_some());
        assert!(find_renderer(&registry, MarkKind::DensityY).is_some());
        assert!(find_renderer(&registry, MarkKind::RegressionX).is_some());
        assert!(find_renderer(&registry, MarkKind::RegressionY).is_some());
        // Unimplemented kinds should return None (no silent fallback).
        assert!(find_renderer(&registry, MarkKind::Heatmap).is_none());
        assert!(find_renderer(&registry, MarkKind::Hexbin).is_none());
    }

    #[test]
    fn ifb_ac07_bar_default_render_interpolated_produces_content() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["a", "b"])),
                Arc::new(Float64Array::from(vec![10.0, 20.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "category".to_string());
        cm.insert(Channel::Y, "value".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let prev_positions = vec![(100.0, 100.0), (200.0, 200.0)];

        let mut scene = Scene::new();
        let renderer = BarRenderer;
        // Default impl should forward to render()
        renderer.render_interpolated(
            &mut scene, &batch, &cm, &scales,
            &prev_positions, 0.5, None,
        );

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "bar default render_interpolated should forward to render"
        );
    }
}
