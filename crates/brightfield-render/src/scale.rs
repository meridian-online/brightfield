//! Scale types and inference — mapping data domains to pixel ranges.
//!
//! Supports linear (numeric), band (categorical), and time (timestamp) scales.
//! Scale inference examines Arrow column types to determine the appropriate
//! scale type for each encoding channel.

use std::collections::HashMap;

use arrow::array::{Array, Float64Array, StringArray, TimestampMicrosecondArray};
use arrow::datatypes::{DataType, TimeUnit};
use arrow::record_batch::RecordBatch;

use crate::channel::{Channel, ChannelMap};

/// A single scale mapping a data domain to a pixel range.
#[derive(Debug, Clone)]
pub enum Scale {
    /// Numeric linear scale: maps [min, max] -> [range_start, range_end].
    Linear {
        domain_min: f64,
        domain_max: f64,
        range_start: f64,
        range_end: f64,
    },
    /// Categorical band scale: maps discrete categories to equal-width bands.
    Band {
        categories: Vec<String>,
        range_start: f64,
        range_end: f64,
        padding: f64,
    },
    /// Time scale: maps [min_us, max_us] microsecond timestamps to pixel range.
    Time {
        domain_min_us: i64,
        domain_max_us: i64,
        range_start: f64,
        range_end: f64,
    },
    /// Colour scale: maps categories to colours from a palette.
    Colour {
        categories: Vec<String>,
        palette: Vec<[f32; 4]>,
    },
    /// Sequential colour scale: maps a numeric magnitude to an interpolated
    /// colour ramp. `stops` are evenly-spaced RGBA control points (low → high);
    /// [`Scale::map_continuous`] normalises a value into `[0, 1]` and
    /// piecewise-lerps between the bracketing pair.
    Sequential {
        domain_min: f64,
        domain_max: f64,
        stops: Vec<[f32; 4]>,
    },
}

impl Scale {
    /// Map a numeric value to a pixel position (linear and time scales).
    pub fn map_f64(&self, value: f64) -> f64 {
        match self {
            Self::Linear {
                domain_min,
                domain_max,
                range_start,
                range_end,
            } => {
                if (domain_max - domain_min).abs() < f64::EPSILON {
                    return (*range_start + *range_end) / 2.0;
                }
                let t = (value - domain_min) / (domain_max - domain_min);
                range_start + t * (range_end - range_start)
            }
            Self::Time {
                domain_min_us,
                domain_max_us,
                range_start,
                range_end,
            } => {
                let span = (*domain_max_us - *domain_min_us) as f64;
                if span.abs() < f64::EPSILON {
                    return (*range_start + *range_end) / 2.0;
                }
                let t = (value - *domain_min_us as f64) / span;
                range_start + t * (range_end - range_start)
            }
            _ => value, // Band/Colour scales don't use f64 mapping.
        }
    }

    /// Map a numeric value to an interpolated ramp colour (Sequential scales).
    ///
    /// Clamps `value` into the domain, normalises to `t ∈ [0, 1]`, and lerps
    /// per-channel between the two `stops` bracketing `t·(n-1)`. Endpoints return
    /// the first/last stop exactly; a degenerate (`domain_min == domain_max`)
    /// domain returns the top stop (mirroring how `map_f64` collapses a zero-span
    /// linear domain). Returns opaque black for a non-Sequential scale — callers
    /// only invoke this on the Fill Sequential scale.
    pub fn map_continuous(&self, value: f64) -> [f32; 4] {
        let Self::Sequential {
            domain_min,
            domain_max,
            stops,
        } = self
        else {
            return [0.0, 0.0, 0.0, 1.0];
        };
        let Some(&top) = stops.last() else {
            return [0.0, 0.0, 0.0, 1.0];
        };
        let span = domain_max - domain_min;
        if span.abs() < f64::EPSILON {
            return top;
        }
        let t = ((value - domain_min) / span).clamp(0.0, 1.0);
        let n = stops.len();
        if n == 1 {
            return stops[0];
        }
        let scaled = t * (n - 1) as f64;
        let i = (scaled.floor() as usize).min(n - 2);
        let frac = (scaled - i as f64) as f32;
        let a = stops[i];
        let b = stops[i + 1];
        [
            a[0] + (b[0] - a[0]) * frac,
            a[1] + (b[1] - a[1]) * frac,
            a[2] + (b[2] - a[2]) * frac,
            a[3] + (b[3] - a[3]) * frac,
        ]
    }

    /// Map a pixel position back to a data value (inverse of `map_f64`).
    ///
    /// Returns `Some` for continuous scales (Linear, Time), `None` for discrete
    /// scales (Band, Colour) where continuous inversion is undefined.
    /// For Time scales, the returned f64 represents microsecond timestamp.
    pub fn inverse_f64(&self, pixel: f64) -> Option<f64> {
        match self {
            Self::Linear {
                domain_min,
                domain_max,
                range_start,
                range_end,
            } => {
                let range_span = range_end - range_start;
                if range_span.abs() < f64::EPSILON {
                    return Some((*domain_min + *domain_max) / 2.0);
                }
                let t = (pixel - range_start) / range_span;
                Some(domain_min + t * (domain_max - domain_min))
            }
            Self::Time {
                domain_min_us,
                domain_max_us,
                range_start,
                range_end,
            } => {
                let range_span = range_end - range_start;
                if range_span.abs() < f64::EPSILON {
                    return Some((*domain_min_us + *domain_max_us) as f64 / 2.0);
                }
                let t = (pixel - range_start) / range_span;
                let domain_span = (*domain_max_us - *domain_min_us) as f64;
                Some(*domain_min_us as f64 + t * domain_span)
            }
            Self::Band { .. } | Self::Colour { .. } | Self::Sequential { .. } => None,
        }
    }

    /// Map a category to a band centre position.
    pub fn map_category(&self, category: &str) -> Option<f64> {
        match self {
            Self::Band {
                categories,
                range_start,
                range_end,
                padding,
            } => {
                let idx = categories.iter().position(|c| c == category)?;
                let n = categories.len() as f64;
                let total_range = range_end - range_start;
                let band_width = total_range / n;
                let padded_start = range_start + band_width * *padding / 2.0;
                Some(padded_start + band_width * idx as f64 + band_width * (1.0 - *padding) / 2.0)
            }
            _ => None,
        }
    }

    /// Get the band width for bar rendering.
    pub fn band_width(&self) -> Option<f64> {
        match self {
            Self::Band {
                categories,
                range_start,
                range_end,
                padding,
            } => {
                let n = categories.len() as f64;
                if n == 0.0 {
                    return None;
                }
                let total_range = range_end - range_start;
                let band_width = total_range / n;
                Some(band_width * (1.0 - padding))
            }
            _ => None,
        }
    }

    /// Look up the colour for a category.
    pub fn map_colour(&self, category: &str) -> Option<[f32; 4]> {
        match self {
            Self::Colour {
                categories,
                palette,
            } => {
                let idx = categories.iter().position(|c| c == category)?;
                Some(palette[idx % palette.len()])
            }
            _ => None,
        }
    }

    /// Domain min for linear/time/sequential scales. A Sequential's extent feeds
    /// the gradient-legend min tick label.
    pub fn domain_min(&self) -> Option<f64> {
        match self {
            Self::Linear { domain_min, .. } => Some(*domain_min),
            Self::Time { domain_min_us, .. } => Some(*domain_min_us as f64),
            Self::Sequential { domain_min, .. } => Some(*domain_min),
            _ => None,
        }
    }

    /// Domain max for linear/time/sequential scales. A Sequential's extent feeds
    /// the gradient-legend max tick label.
    pub fn domain_max(&self) -> Option<f64> {
        match self {
            Self::Linear { domain_max, .. } => Some(*domain_max),
            Self::Time { domain_max_us, .. } => Some(*domain_max_us as f64),
            Self::Sequential { domain_max, .. } => Some(*domain_max),
            _ => None,
        }
    }

    /// Range start.
    pub fn range_start(&self) -> f64 {
        match self {
            Self::Linear { range_start, .. }
            | Self::Band { range_start, .. }
            | Self::Time { range_start, .. } => *range_start,
            // Colour ramps carry no positional pixel range.
            Self::Colour { .. } | Self::Sequential { .. } => 0.0,
        }
    }

    /// Range end.
    pub fn range_end(&self) -> f64 {
        match self {
            Self::Linear { range_end, .. }
            | Self::Band { range_end, .. }
            | Self::Time { range_end, .. } => *range_end,
            // Colour ramps carry no positional pixel range.
            Self::Colour { .. } | Self::Sequential { .. } => 0.0,
        }
    }
}

/// Optional override of data-inferred scale domains per axis.
///
/// When `Some`, the chart renders only the specified data range on that axis.
/// When `None`, the full data-inferred domain is used.
/// Used by pan/zoom navigation — the interaction layer mutates this struct,
/// the render and engine layers consume it read-only.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewExtent {
    /// Overridden x-axis domain: `Some((min, max))` or `None` for full extent.
    pub x: Option<(f64, f64)>,
    /// Overridden y-axis domain: `Some((min, max))` or `None` for full extent.
    pub y: Option<(f64, f64)>,
}

/// Collection of inferred scales for a chart, keyed by channel.
#[derive(Debug, Clone, Default)]
pub struct ScaleSet {
    scales: HashMap<Channel, Scale>,
}

impl ScaleSet {
    /// Create an empty scale set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a scale for a channel.
    pub fn insert(&mut self, channel: Channel, scale: Scale) {
        self.scales.insert(channel, scale);
    }

    /// Get the scale for a channel.
    pub fn get(&self, channel: Channel) -> Option<&Scale> {
        self.scales.get(&channel)
    }
}

/// A built-in continuous colour scheme. Wire names are lowercase and
/// Mosaic-aligned, so a `colorScheme:` value stays portable across renderers.
///
/// The default is [`SequentialScheme::Viridis`] — a deliberate divergence from
/// Mosaic/Plot's `turbo` quantitative default. Viridis is perceptually uniform
/// and colourblind-safe; `turbo` stays available by name (see `deviations.yaml`
/// DEV-0003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SequentialScheme {
    /// Perceptually-uniform, colourblind-safe (matplotlib/ggplot default).
    #[default]
    Viridis,
    /// Single-hue light → dark sequential (ColorBrewer Blues) — the classic
    /// count map, light-anchored.
    Blues,
    /// Mosaic/Plot's declared quantitative default — a rainbow map, included for
    /// spec fidelity.
    Turbo,
}

impl SequentialScheme {
    /// The lowercase, Mosaic-aligned wire name.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Viridis => "viridis",
            Self::Blues => "blues",
            Self::Turbo => "turbo",
        }
    }

    /// Parse a wire name (case-exact). `None` for an unrecognised scheme — the
    /// caller warns and falls back to the default.
    #[must_use]
    pub fn from_wire(name: &str) -> Option<Self> {
        match name {
            "viridis" => Some(Self::Viridis),
            "blues" => Some(Self::Blues),
            "turbo" => Some(Self::Turbo),
            _ => None,
        }
    }

    /// Evenly-spaced RGBA control points (low → high), interpolated by
    /// [`Scale::map_continuous`]. Nine hand-transcribed points per scheme — enough
    /// to read as the intended ramp; a full 256-entry LUT is a later refinement.
    #[must_use]
    pub fn stops(self) -> Vec<[f32; 4]> {
        match self {
            Self::Viridis => VIRIDIS_STOPS.to_vec(),
            Self::Blues => BLUES_STOPS.to_vec(),
            Self::Turbo => TURBO_STOPS.to_vec(),
        }
    }
}

/// Viridis control points (matplotlib, 9-class), dark purple → bright yellow.
const VIRIDIS_STOPS: &[[f32; 4]] = &[
    [0.267, 0.004, 0.329, 1.0], // #440154
    [0.278, 0.176, 0.482, 1.0], // #472d7b
    [0.231, 0.322, 0.545, 1.0], // #3b528b
    [0.173, 0.447, 0.557, 1.0], // #2c728e
    [0.129, 0.569, 0.549, 1.0], // #21918c
    [0.157, 0.682, 0.502, 1.0], // #28ae80
    [0.369, 0.788, 0.384, 1.0], // #5ec962
    [0.678, 0.863, 0.188, 1.0], // #addc30
    [0.992, 0.906, 0.145, 1.0], // #fde725
];

/// Blues control points (ColorBrewer sequential, 9-class), near-white → navy.
const BLUES_STOPS: &[[f32; 4]] = &[
    [0.969, 0.984, 1.000, 1.0], // #f7fbff
    [0.871, 0.922, 0.969, 1.0], // #deebf7
    [0.776, 0.859, 0.937, 1.0], // #c6dbef
    [0.620, 0.792, 0.882, 1.0], // #9ecae1
    [0.420, 0.682, 0.839, 1.0], // #6baed6
    [0.259, 0.573, 0.776, 1.0], // #4292c6
    [0.129, 0.443, 0.710, 1.0], // #2171b5
    [0.031, 0.318, 0.612, 1.0], // #08519c
    [0.031, 0.188, 0.420, 1.0], // #08306b
];

/// Turbo control points (Google turbo, 9-sample), purple → blue → green →
/// yellow → dark red.
const TURBO_STOPS: &[[f32; 4]] = &[
    [0.190, 0.072, 0.232, 1.0], // #30123b
    [0.246, 0.395, 0.832, 1.0], // #3f65d4
    [0.239, 0.657, 0.985, 1.0], // #3ea8fb
    [0.180, 0.902, 0.769, 1.0], // #2ee6c4
    [0.427, 0.988, 0.475, 1.0], // #6dfc79
    [0.760, 0.965, 0.235, 1.0], // #c2f63c
    [0.973, 0.798, 0.155, 1.0], // #f8cc28
    [0.960, 0.446, 0.104, 1.0], // #f5721a
    [0.480, 0.016, 0.011, 1.0], // #7a0403
];

/// Default colour palette — Observable Plot's categorical10.
const CATEGORICAL_PALETTE: &[[f32; 4]] = &[
    [0.306, 0.475, 0.655, 1.0], // steel blue #4e79a7
    [0.949, 0.557, 0.169, 1.0], // orange #f28e2b
    [0.882, 0.341, 0.349, 1.0], // red #e15759
    [0.463, 0.718, 0.698, 1.0], // teal #76b7b2
    [0.349, 0.631, 0.310, 1.0], // green #59a14f
    [0.929, 0.788, 0.282, 1.0], // yellow #edc948
    [0.690, 0.478, 0.631, 1.0], // purple #b07aa1
    [1.000, 0.616, 0.655, 1.0], // pink #ff9da7
    [0.612, 0.459, 0.373, 1.0], // brown #9c755f
    [0.729, 0.690, 0.675, 1.0], // grey #bab0ac
];

/// Infer scales from a RecordBatch and ChannelMap.
///
/// For each channel in the map, examines the corresponding Arrow column type:
/// - Float64 / Int64 / Int32 / Int16 / numeric -> LinearScale
/// - Utf8 / string -> BandScale
/// - Timestamp -> TimeScale
///
/// `x_range` and `y_range` are the pixel ranges for x and y axes respectively.
/// Fold any literal channel values (e.g. `y: 0`) into the scale set so a
/// constant-positioned mark (like a baseline rule) is placed correctly. A
/// literal on a positional axis extends an existing Linear scale's domain to
/// include the value (so an off-range literal stays on-plot), or — when no
/// column gave that axis a scale — synthesises a Linear scale around the value.
/// Non-linear (Band/Time/Colour) scales are left unchanged.
fn extend_scales_with_literals<I: Iterator<Item = (Channel, f64)>>(
    set: &mut ScaleSet,
    literals: I,
    x_range: (f64, f64),
    y_range: (f64, f64),
) {
    for (channel, value) in literals {
        let (range_start, range_end) = match channel {
            Channel::X | Channel::X1 | Channel::X2 => x_range,
            Channel::Y | Channel::Y1 | Channel::Y2 => y_range,
            _ => continue, // literals only position on x/y axes
        };
        let new_scale = match set.get(channel) {
            Some(Scale::Linear {
                domain_min,
                domain_max,
                ..
            }) => Some(Scale::Linear {
                domain_min: domain_min.min(value),
                domain_max: domain_max.max(value),
                range_start,
                range_end,
            }),
            Some(_) => None, // non-linear axis: can't merge a numeric literal
            None => {
                // No column scale on this axis — synthesise one spanning 0..value
                // (baseline-friendly), guarding against a zero span.
                let (lo, hi) = (value.min(0.0), value.max(0.0));
                let (lo, hi) = if (hi - lo).abs() < f64::EPSILON {
                    (value - 1.0, value + 1.0)
                } else {
                    (lo, hi)
                };
                Some(Scale::Linear {
                    domain_min: lo,
                    domain_max: hi,
                    range_start,
                    range_end,
                })
            }
        };
        if let Some(s) = new_scale {
            set.insert(channel, s);
        }
    }
}

/// Insert or widen a Linear scale on `channel` so its domain spans `[min, max]`
/// over the given pixel `range`.
///
/// Statistical marks build positional scales from emitted data extents rather
/// than an inferable column (e.g. regression's x/y come from `x_min`/`x_max`
/// aggregates — the executed batch has no raw x/y column). When a sibling mark
/// already established a Linear scale on the channel, the domain is unioned so
/// co-rendered marks share one axis; an existing non-Linear (Band/Time/Colour)
/// scale is left untouched.
pub fn merge_linear_scale(
    set: &mut ScaleSet,
    channel: Channel,
    min: f64,
    max: f64,
    range: (f64, f64),
) {
    let (domain_min, domain_max) = match set.get(channel) {
        Some(Scale::Linear {
            domain_min,
            domain_max,
            ..
        }) => (domain_min.min(min), domain_max.max(max)),
        Some(_) => return, // non-linear axis already established
        None => (min, max),
    };
    set.insert(
        channel,
        Scale::Linear {
            domain_min,
            domain_max,
            range_start: range.0,
            range_end: range.1,
        },
    );
}

pub fn infer_scales(
    batch: &RecordBatch,
    channel_map: &ChannelMap,
    x_range: (f64, f64),
    y_range: (f64, f64),
) -> ScaleSet {
    let mut set = ScaleSet::new();

    for (channel, col_name) in channel_map.iter() {
        let col_idx = match batch.schema().index_of(col_name) {
            Ok(idx) => idx,
            Err(_) => continue,
        };
        let col = batch.column(col_idx);
        let (range_start, range_end) = match channel {
            Channel::X | Channel::X1 | Channel::X2 => x_range,
            Channel::Y | Channel::Y1 | Channel::Y2 => y_range,
            _ => (0.0, 0.0),
        };

        let scale = infer_column_scale(col.as_ref(), range_start, range_end, *channel);
        if let Some(s) = scale {
            set.insert(*channel, s);
        }
    }

    extend_scales_with_literals(&mut set, channel_map.literals_iter(), x_range, y_range);
    set
}

/// Infer scales from multiple (RecordBatch, ChannelMap) pairs with unioned domains.
///
/// For each channel that appears in any channel map, collects domain values from
/// all batches and produces a single scale spanning the combined range:
/// - Linear: min(all_mins), max(all_maxes)
/// - Band/Colour: set union of categories (preserving insertion order)
/// - Time: min(all_mins), max(all_maxes)
///
/// The existing `infer_scales()` is unchanged.
pub fn infer_scales_multi(
    entries: &[(&RecordBatch, &ChannelMap)],
    x_range: (f64, f64),
    y_range: (f64, f64),
) -> ScaleSet {
    // Collect all channels across all entries.
    let mut all_channels: Vec<Channel> = Vec::new();
    for (_, cm) in entries {
        for (ch, _) in cm.iter() {
            if !all_channels.contains(ch) {
                all_channels.push(*ch);
            }
        }
    }

    let mut set = ScaleSet::new();

    for channel in &all_channels {
        let (range_start, range_end) = match channel {
            Channel::X | Channel::X1 | Channel::X2 => x_range,
            Channel::Y | Channel::Y1 | Channel::Y2 => y_range,
            _ => (0.0, 0.0),
        };

        // Collect per-batch scales for this channel and union them.
        let mut scales_for_channel: Vec<Scale> = Vec::new();
        for (batch, cm) in entries {
            if let Some(col_name) = cm.get(*channel) {
                let col_idx = match batch.schema().index_of(col_name) {
                    Ok(idx) => idx,
                    Err(_) => continue,
                };
                let col = batch.column(col_idx);
                if let Some(s) = infer_column_scale(col.as_ref(), range_start, range_end, *channel)
                {
                    scales_for_channel.push(s);
                }
            }
        }

        if let Some(merged) = union_scales(&scales_for_channel, range_start, range_end) {
            set.insert(*channel, merged);
        }
    }

    for (_, cm) in entries {
        extend_scales_with_literals(&mut set, cm.literals_iter(), x_range, y_range);
    }
    set
}

/// Union a list of scales of the same type into a single scale.
fn union_scales(scales: &[Scale], range_start: f64, range_end: f64) -> Option<Scale> {
    if scales.is_empty() {
        return None;
    }

    match &scales[0] {
        Scale::Linear { .. } => {
            let mut min = f64::INFINITY;
            let mut max = f64::NEG_INFINITY;
            for s in scales {
                if let Scale::Linear {
                    domain_min,
                    domain_max,
                    ..
                } = s
                {
                    if *domain_min < min {
                        min = *domain_min;
                    }
                    if *domain_max > max {
                        max = *domain_max;
                    }
                }
            }
            if min.is_infinite() {
                None
            } else {
                Some(Scale::Linear {
                    domain_min: min,
                    domain_max: max,
                    range_start,
                    range_end,
                })
            }
        }
        Scale::Band { padding, .. } => {
            let padding = *padding;
            let mut categories: Vec<String> = Vec::new();
            for s in scales {
                if let Scale::Band {
                    categories: cats, ..
                } = s
                {
                    for cat in cats {
                        if !categories.contains(cat) {
                            categories.push(cat.clone());
                        }
                    }
                }
            }
            Some(Scale::Band {
                categories,
                range_start,
                range_end,
                padding,
            })
        }
        Scale::Colour { palette, .. } => {
            let palette = palette.clone();
            let mut categories: Vec<String> = Vec::new();
            for s in scales {
                if let Scale::Colour {
                    categories: cats, ..
                } = s
                {
                    for cat in cats {
                        if !categories.contains(cat) {
                            categories.push(cat.clone());
                        }
                    }
                }
            }
            Some(Scale::Colour {
                categories,
                palette,
            })
        }
        Scale::Time { .. } => {
            let mut min = i64::MAX;
            let mut max = i64::MIN;
            for s in scales {
                if let Scale::Time {
                    domain_min_us,
                    domain_max_us,
                    ..
                } = s
                {
                    if *domain_min_us < min {
                        min = *domain_min_us;
                    }
                    if *domain_max_us > max {
                        max = *domain_max_us;
                    }
                }
            }
            if min == i64::MAX {
                None
            } else {
                Some(Scale::Time {
                    domain_min_us: min,
                    domain_max_us: max,
                    range_start,
                    range_end,
                })
            }
        }
        Scale::Sequential { stops, .. } => {
            // Union the ramp extents by min-of-mins / max-of-maxes, keeping the
            // first scale's stops (co-rendered rasters share one scheme).
            let stops = stops.clone();
            let mut min = f64::INFINITY;
            let mut max = f64::NEG_INFINITY;
            for s in scales {
                if let Scale::Sequential {
                    domain_min,
                    domain_max,
                    ..
                } = s
                {
                    if *domain_min < min {
                        min = *domain_min;
                    }
                    if *domain_max > max {
                        max = *domain_max;
                    }
                }
            }
            if min.is_infinite() {
                None
            } else {
                Some(Scale::Sequential {
                    domain_min: min,
                    domain_max: max,
                    stops,
                })
            }
        }
    }
}

fn infer_column_scale(
    col: &dyn Array,
    range_start: f64,
    range_end: f64,
    channel: Channel,
) -> Option<Scale> {
    match col.data_type() {
        DataType::Float64 => {
            let arr = col.as_any().downcast_ref::<Float64Array>()?;
            let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
            for i in 0..arr.len() {
                if !arr.is_null(i) {
                    let v = arr.value(i);
                    if v < min {
                        min = v;
                    }
                    if v > max {
                        max = v;
                    }
                }
            }
            if min.is_infinite() {
                return None;
            }
            Some(Scale::Linear {
                domain_min: min,
                domain_max: max,
                range_start,
                range_end,
            })
        }
        DataType::Int64 => {
            let arr = col.as_any().downcast_ref::<arrow::array::Int64Array>()?;
            let (mut min, mut max) = (i64::MAX, i64::MIN);
            for i in 0..arr.len() {
                if !arr.is_null(i) {
                    let v = arr.value(i);
                    if v < min {
                        min = v;
                    }
                    if v > max {
                        max = v;
                    }
                }
            }
            if min == i64::MAX {
                return None;
            }
            Some(Scale::Linear {
                domain_min: min as f64,
                domain_max: max as f64,
                range_start,
                range_end,
            })
        }
        DataType::Int32 => {
            let arr = col.as_any().downcast_ref::<arrow::array::Int32Array>()?;
            let (mut min, mut max) = (i32::MAX, i32::MIN);
            for i in 0..arr.len() {
                if !arr.is_null(i) {
                    let v = arr.value(i);
                    if v < min {
                        min = v;
                    }
                    if v > max {
                        max = v;
                    }
                }
            }
            if min == i32::MAX {
                return None;
            }
            Some(Scale::Linear {
                domain_min: min as f64,
                domain_max: max as f64,
                range_start,
                range_end,
            })
        }
        DataType::Int16 => {
            let arr = col.as_any().downcast_ref::<arrow::array::Int16Array>()?;
            let (mut min, mut max) = (i16::MAX, i16::MIN);
            for i in 0..arr.len() {
                if !arr.is_null(i) {
                    let v = arr.value(i);
                    if v < min {
                        min = v;
                    }
                    if v > max {
                        max = v;
                    }
                }
            }
            if min == i16::MAX {
                return None;
            }
            Some(Scale::Linear {
                domain_min: min as f64,
                domain_max: max as f64,
                range_start,
                range_end,
            })
        }
        DataType::Utf8 => {
            let arr = col.as_any().downcast_ref::<StringArray>().unwrap();
            let mut categories = Vec::new();
            for i in 0..arr.len() {
                if !arr.is_null(i) {
                    let v = arr.value(i).to_string();
                    if !categories.contains(&v) {
                        categories.push(v);
                    }
                }
            }
            if matches!(channel, Channel::Fill | Channel::Stroke) {
                Some(Scale::Colour {
                    palette: CATEGORICAL_PALETTE.to_vec(),
                    categories,
                })
            } else {
                Some(Scale::Band {
                    categories,
                    range_start,
                    range_end,
                    padding: 0.1,
                })
            }
        }
        DataType::Timestamp(TimeUnit::Microsecond, _) => {
            let arr = col
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()?;
            let (mut min, mut max) = (i64::MAX, i64::MIN);
            for i in 0..arr.len() {
                if !arr.is_null(i) {
                    let v = arr.value(i);
                    if v < min {
                        min = v;
                    }
                    if v > max {
                        max = v;
                    }
                }
            }
            if min == i64::MAX {
                return None;
            }
            Some(Scale::Time {
                domain_min_us: min,
                domain_max_us: max,
                range_start,
                range_end,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Float64Array, StringArray, TimestampMicrosecondArray};
    use arrow::datatypes::{Field, Schema};
    use arrow::record_batch::RecordBatch;
    use std::sync::Arc;

    fn make_numeric_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap()
    }

    fn make_categorical_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["a", "b", "c"])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap()
    }

    fn make_time_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "ts",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                false,
            ),
            Field::new("value", DataType::Float64, false),
        ]));
        RecordBatch::try_new(
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
        .unwrap()
    }

    #[test]
    fn gpu_ac02_infer_linear_scales() {
        let batch = make_numeric_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (400.0, 20.0));

        let x = scales.get(Channel::X).expect("x scale should exist");
        match x {
            Scale::Linear {
                domain_min,
                domain_max,
                ..
            } => {
                assert!((domain_min - 1.0).abs() < f64::EPSILON);
                assert!((domain_max - 3.0).abs() < f64::EPSILON);
            }
            other => panic!("expected Linear scale for x, got: {other:?}"),
        }

        let y = scales.get(Channel::Y).expect("y scale should exist");
        match y {
            Scale::Linear {
                domain_min,
                domain_max,
                ..
            } => {
                assert!((domain_min - 10.0).abs() < f64::EPSILON);
                assert!((domain_max - 30.0).abs() < f64::EPSILON);
            }
            other => panic!("expected Linear scale for y, got: {other:?}"),
        }
    }

    #[test]
    fn gpu_ac02_infer_band_scale() {
        let batch = make_categorical_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "category".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (400.0, 20.0));

        let x = scales.get(Channel::X).expect("x scale should exist");
        match x {
            Scale::Band { categories, .. } => {
                assert_eq!(categories, &["a", "b", "c"]);
            }
            other => panic!("expected Band scale for x, got: {other:?}"),
        }
    }

    #[test]
    fn gpu_ac02_infer_time_scale() {
        let batch = make_time_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "ts".to_string());
        cm.insert(Channel::Y, "value".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (400.0, 20.0));

        let x = scales.get(Channel::X).expect("x scale should exist");
        match x {
            Scale::Time {
                domain_min_us,
                domain_max_us,
                ..
            } => {
                assert_eq!(*domain_min_us, 1_000_000);
                assert_eq!(*domain_max_us, 4_000_000);
            }
            other => panic!("expected Time scale for x, got: {other:?}"),
        }
    }

    #[test]
    fn gpu_ac02_infer_colour_scale_for_fill_channel() {
        let batch = make_categorical_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::Fill, "category".to_string());

        let scales = infer_scales(&batch, &cm, (0.0, 0.0), (0.0, 0.0));

        let fill = scales.get(Channel::Fill).expect("fill scale should exist");
        match fill {
            Scale::Colour { categories, palette } => {
                assert_eq!(categories, &["a", "b", "c"]);
                assert!(!palette.is_empty());
            }
            other => panic!("expected Colour scale for fill, got: {other:?}"),
        }
    }

    #[test]
    fn gpu_ac02_linear_scale_maps_correctly() {
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: 0.0,
            range_end: 500.0,
        };
        assert!((scale.map_f64(0.0) - 0.0).abs() < f64::EPSILON);
        assert!((scale.map_f64(50.0) - 250.0).abs() < f64::EPSILON);
        assert!((scale.map_f64(100.0) - 500.0).abs() < f64::EPSILON);
    }

    // --- nav_ac01: ViewExtent ---

    #[test]
    fn nav_ac01_view_extent_with_both_axes() {
        let ve = ViewExtent {
            x: Some((10.0, 50.0)),
            y: Some((100.0, 200.0)),
        };
        assert_eq!(ve.x, Some((10.0, 50.0)));
        assert_eq!(ve.y, Some((100.0, 200.0)));
    }

    #[test]
    fn nav_ac01_view_extent_with_none_axes() {
        let ve = ViewExtent::default();
        assert_eq!(ve.x, None);
        assert_eq!(ve.y, None);
    }

    #[test]
    fn nav_ac01_view_extent_partial() {
        let ve = ViewExtent {
            x: Some((1.0, 2.0)),
            y: None,
        };
        assert!(ve.x.is_some());
        assert!(ve.y.is_none());
    }

    // --- nav_ac02: Scale::inverse_f64 ---

    #[test]
    fn nav_ac02_linear_inverse_at_endpoints() {
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: 0.0,
            range_end: 500.0,
        };
        let inv_min = scale.inverse_f64(0.0).expect("linear should return Some");
        let inv_max = scale.inverse_f64(500.0).expect("linear should return Some");
        assert!((inv_min - 0.0).abs() < f64::EPSILON);
        assert!((inv_max - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn nav_ac02_linear_inverse_at_midpoint() {
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: 0.0,
            range_end: 500.0,
        };
        let inv_mid = scale.inverse_f64(250.0).expect("linear should return Some");
        assert!((inv_mid - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn nav_ac02_linear_inverse_roundtrip() {
        let scale = Scale::Linear {
            domain_min: 10.0,
            domain_max: 90.0,
            range_start: 40.0,
            range_end: 600.0,
        };
        let value = 55.0;
        let pixel = scale.map_f64(value);
        let roundtrip = scale.inverse_f64(pixel).unwrap();
        assert!((roundtrip - value).abs() < 1e-10);
    }

    #[test]
    fn nav_ac02_time_inverse() {
        let scale = Scale::Time {
            domain_min_us: 1_000_000,
            domain_max_us: 4_000_000,
            range_start: 0.0,
            range_end: 300.0,
        };
        let inv = scale.inverse_f64(100.0).expect("time should return Some");
        // At 1/3 of the range => 1/3 of domain span
        let expected = 1_000_000.0 + (3_000_000.0 / 3.0);
        assert!((inv - expected).abs() < 1.0);
    }

    #[test]
    fn nav_ac02_band_inverse_returns_none() {
        let scale = Scale::Band {
            categories: vec!["a".to_string(), "b".to_string()],
            range_start: 0.0,
            range_end: 200.0,
            padding: 0.1,
        };
        assert!(scale.inverse_f64(100.0).is_none());
    }

    #[test]
    fn nav_ac02_colour_inverse_returns_none() {
        let scale = Scale::Colour {
            categories: vec!["a".to_string()],
            palette: vec![[1.0, 0.0, 0.0, 1.0]],
        };
        assert!(scale.inverse_f64(50.0).is_none());
    }

    #[test]
    fn gpu_ac02_band_scale_maps_categories() {
        let scale = Scale::Band {
            categories: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            range_start: 0.0,
            range_end: 300.0,
            padding: 0.1,
        };
        let a_pos = scale.map_category("a").expect("a should map");
        let b_pos = scale.map_category("b").expect("b should map");
        let c_pos = scale.map_category("c").expect("c should map");
        assert!(a_pos < b_pos);
        assert!(b_pos < c_pos);
        assert!(scale.map_category("d").is_none());

        let bw = scale.band_width().expect("should have band width");
        assert!(bw > 0.0);
        assert!(bw < 100.0); // each band is 100px wide, with 10% padding -> 90px
    }

    // --- msv ac-02: infer_scales_multi ---

    #[test]
    fn msv_ac02_multi_unions_linear_domains() {
        // Batch 1: x in [1, 5], y in [10, 50]
        let schema1 = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch1 = RecordBatch::try_new(
            schema1,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 5.0])),
                Arc::new(Float64Array::from(vec![10.0, 50.0])),
            ],
        )
        .unwrap();

        // Batch 2: x in [3, 8], y in [5, 30]
        let schema2 = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch2 = RecordBatch::try_new(
            schema2,
            vec![
                Arc::new(Float64Array::from(vec![3.0, 8.0])),
                Arc::new(Float64Array::from(vec![5.0, 30.0])),
            ],
        )
        .unwrap();

        let mut cm1 = ChannelMap::new();
        cm1.insert(Channel::X, "x".to_string());
        cm1.insert(Channel::Y, "y".to_string());
        let mut cm2 = ChannelMap::new();
        cm2.insert(Channel::X, "x".to_string());
        cm2.insert(Channel::Y, "y".to_string());

        let entries: Vec<(&RecordBatch, &ChannelMap)> = vec![(&batch1, &cm1), (&batch2, &cm2)];
        let scales = infer_scales_multi(&entries, (40.0, 600.0), (450.0, 20.0));

        let x = scales.get(Channel::X).expect("x scale should exist");
        match x {
            Scale::Linear {
                domain_min,
                domain_max,
                ..
            } => {
                assert!((domain_min - 1.0).abs() < f64::EPSILON, "x min should be 1.0");
                assert!((domain_max - 8.0).abs() < f64::EPSILON, "x max should be 8.0");
            }
            other => panic!("expected Linear scale for x, got: {other:?}"),
        }

        let y = scales.get(Channel::Y).expect("y scale should exist");
        match y {
            Scale::Linear {
                domain_min,
                domain_max,
                ..
            } => {
                assert!((domain_min - 5.0).abs() < f64::EPSILON, "y min should be 5.0");
                assert!((domain_max - 50.0).abs() < f64::EPSILON, "y max should be 50.0");
            }
            other => panic!("expected Linear scale for y, got: {other:?}"),
        }
    }

    #[test]
    fn msv_ac02_multi_unions_categorical_fill() {
        let schema1 = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
        ]));
        let batch1 = RecordBatch::try_new(
            schema1,
            vec![Arc::new(StringArray::from(vec!["red", "blue"]))],
        )
        .unwrap();

        let schema2 = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
        ]));
        let batch2 = RecordBatch::try_new(
            schema2,
            vec![Arc::new(StringArray::from(vec!["blue", "green"]))],
        )
        .unwrap();

        let mut cm1 = ChannelMap::new();
        cm1.insert(Channel::Fill, "category".to_string());
        let mut cm2 = ChannelMap::new();
        cm2.insert(Channel::Fill, "category".to_string());

        let entries: Vec<(&RecordBatch, &ChannelMap)> = vec![(&batch1, &cm1), (&batch2, &cm2)];
        let scales = infer_scales_multi(&entries, (0.0, 0.0), (0.0, 0.0));

        let fill = scales.get(Channel::Fill).expect("fill scale should exist");
        match fill {
            Scale::Colour { categories, .. } => {
                // Union of {red, blue} and {blue, green} = {red, blue, green}
                assert_eq!(categories.len(), 3);
                assert!(categories.contains(&"red".to_string()));
                assert!(categories.contains(&"blue".to_string()));
                assert!(categories.contains(&"green".to_string()));
            }
            other => panic!("expected Colour scale for fill, got: {other:?}"),
        }
    }

    #[test]
    fn msv_ac02_multi_single_entry_matches_infer_scales() {
        let batch = make_numeric_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let single = infer_scales(&batch, &cm, (40.0, 600.0), (400.0, 20.0));
        let multi = infer_scales_multi(&[(&batch, &cm)], (40.0, 600.0), (400.0, 20.0));

        // Both should produce identical domains
        let sx = single.get(Channel::X).unwrap();
        let mx = multi.get(Channel::X).unwrap();
        assert!((sx.domain_min().unwrap() - mx.domain_min().unwrap()).abs() < f64::EPSILON);
        assert!((sx.domain_max().unwrap() - mx.domain_max().unwrap()).abs() < f64::EPSILON);
    }

    #[test]
    fn merge_linear_scale_inserts_unions_and_skips_nonlinear() {
        let range = (0.0, 100.0);

        // Absent → insert.
        let mut set = ScaleSet::new();
        merge_linear_scale(&mut set, Channel::X, 2.0, 8.0, range);
        match set.get(Channel::X).expect("x scale inserted") {
            Scale::Linear {
                domain_min,
                domain_max,
                range_start,
                range_end,
            } => {
                assert_eq!((*domain_min, *domain_max), (2.0, 8.0));
                assert_eq!((*range_start, *range_end), (0.0, 100.0));
            }
            other => panic!("expected Linear, got {other:?}"),
        }

        // Present Linear → union (widen) on both ends.
        merge_linear_scale(&mut set, Channel::X, 1.0, 5.0, range);
        merge_linear_scale(&mut set, Channel::X, 4.0, 12.0, range);
        match set.get(Channel::X).unwrap() {
            Scale::Linear {
                domain_min,
                domain_max,
                ..
            } => assert_eq!((*domain_min, *domain_max), (1.0, 12.0)),
            other => panic!("expected Linear, got {other:?}"),
        }

        // Present non-Linear → left untouched (don't clobber a Band axis).
        let mut set2 = ScaleSet::new();
        set2.insert(
            Channel::X,
            Scale::Band {
                categories: vec!["a".to_string(), "b".to_string()],
                range_start: 0.0,
                range_end: 10.0,
                padding: 0.1,
            },
        );
        merge_linear_scale(&mut set2, Channel::X, 1.0, 9.0, range);
        assert!(
            matches!(set2.get(Channel::X).unwrap(), Scale::Band { .. }),
            "non-linear scale must not be overwritten by merge_linear_scale"
        );
    }

    // --- scs_ac01: Scale::Sequential + map_continuous ---

    #[test]
    fn scs_ac01_map_continuous_interpolates_and_clamps() {
        let black = [0.0, 0.0, 0.0, 1.0];
        let white = [1.0, 1.0, 1.0, 1.0];
        let scale = Scale::Sequential {
            domain_min: 0.0,
            domain_max: 10.0,
            stops: vec![black, white],
        };

        // Endpoints return the first/last stop exactly.
        assert_eq!(scale.map_continuous(0.0), black);
        assert_eq!(scale.map_continuous(10.0), white);

        // Midpoint of a 2-stop ramp is the channel-wise average.
        let mid = scale.map_continuous(5.0);
        for c in 0..3 {
            assert!((mid[c] - 0.5).abs() < 1e-6, "channel {c} mid = {}", mid[c]);
        }

        // Out-of-domain values clamp to the endpoints.
        assert_eq!(scale.map_continuous(-5.0), black);
        assert_eq!(scale.map_continuous(42.0), white);

        // A degenerate (min == max) domain returns the top stop.
        let degenerate = Scale::Sequential {
            domain_min: 3.0,
            domain_max: 3.0,
            stops: vec![black, white],
        };
        assert_eq!(degenerate.map_continuous(3.0), white);
    }

    #[test]
    fn scs_ac01_map_continuous_locates_correct_bracket() {
        // Three stops over [0, 2]: red, green, blue. A value at t=0.75 sits in the
        // second segment (green → blue) three-quarters along.
        let scale = Scale::Sequential {
            domain_min: 0.0,
            domain_max: 2.0,
            stops: vec![
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0, 1.0],
            ],
        };
        let c = scale.map_continuous(1.5); // t = 0.75 → seg 1, frac 0.5
        assert!((c[0] - 0.0).abs() < 1e-6);
        assert!((c[1] - 0.5).abs() < 1e-6, "green = {}", c[1]);
        assert!((c[2] - 0.5).abs() < 1e-6, "blue = {}", c[2]);
    }

    // --- scs_ac02: SequentialScheme ---

    #[test]
    fn scs_ac02_scheme_stops_and_wire_roundtrip() {
        for scheme in [
            SequentialScheme::Viridis,
            SequentialScheme::Blues,
            SequentialScheme::Turbo,
        ] {
            let stops = scheme.stops();
            assert!(stops.len() >= 5, "{scheme:?} has >= 5 stops");
            for s in &stops {
                for &c in s {
                    assert!((0.0..=1.0).contains(&c), "{scheme:?} component {c} in range");
                }
            }
            assert_eq!(
                SequentialScheme::from_wire(scheme.wire_name()),
                Some(scheme),
                "{scheme:?} round-trips through its wire name"
            );
        }
        // Unknown / wrong-case names yield None; the caller warns + defaults.
        assert_eq!(SequentialScheme::from_wire("magma"), None);
        assert_eq!(SequentialScheme::from_wire("Viridis"), None);
        // The default scheme is viridis.
        assert_eq!(SequentialScheme::default(), SequentialScheme::Viridis);
    }

    // --- scs_ac03: adding Sequential leaves every exhaustive match decided ---

    #[test]
    fn scs_ac03_sequential_match_arms_decided() {
        let stops = SequentialScheme::Viridis.stops();
        let a = Scale::Sequential {
            domain_min: 0.0,
            domain_max: 10.0,
            stops: stops.clone(),
        };
        let b = Scale::Sequential {
            domain_min: 0.0,
            domain_max: 25.0,
            stops: stops.clone(),
        };

        // union_scales unions by min-of-mins / max-of-maxes.
        let unioned = union_scales(&[a.clone(), b], 0.0, 0.0).expect("union yields a scale");
        match unioned {
            Scale::Sequential {
                domain_min,
                domain_max,
                ..
            } => {
                assert!((domain_min - 0.0).abs() < f64::EPSILON);
                assert!((domain_max - 25.0).abs() < f64::EPSILON);
            }
            other => panic!("expected Sequential, got {other:?}"),
        }

        // compute_ticks returns no positional ticks; domain_max reads the extent.
        assert!(crate::axis::compute_ticks(&a, 5).is_empty());
        assert_eq!(a.domain_min(), Some(0.0));
        assert_eq!(a.domain_max(), Some(10.0));
        // A colour ramp has no positional pixel range and cannot invert.
        assert_eq!(a.range_start(), 0.0);
        assert_eq!(a.range_end(), 0.0);
        assert!(a.inverse_f64(5.0).is_none());
    }
}
