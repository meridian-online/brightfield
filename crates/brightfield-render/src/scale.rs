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
            Self::Band { .. } | Self::Colour { .. } => None,
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

    /// Domain min for linear/time scales.
    pub fn domain_min(&self) -> Option<f64> {
        match self {
            Self::Linear { domain_min, .. } => Some(*domain_min),
            Self::Time { domain_min_us, .. } => Some(*domain_min_us as f64),
            _ => None,
        }
    }

    /// Domain max for linear/time scales.
    pub fn domain_max(&self) -> Option<f64> {
        match self {
            Self::Linear { domain_max, .. } => Some(*domain_max),
            Self::Time { domain_max_us, .. } => Some(*domain_max_us as f64),
            _ => None,
        }
    }

    /// Range start.
    pub fn range_start(&self) -> f64 {
        match self {
            Self::Linear { range_start, .. }
            | Self::Band { range_start, .. }
            | Self::Time { range_start, .. } => *range_start,
            Self::Colour { .. } => 0.0,
        }
    }

    /// Range end.
    pub fn range_end(&self) -> f64 {
        match self {
            Self::Linear { range_end, .. }
            | Self::Band { range_end, .. }
            | Self::Time { range_end, .. } => *range_end,
            Self::Colour { .. } => 0.0,
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
/// - Float64 / Int64 / numeric -> LinearScale
/// - Utf8 / string -> BandScale
/// - Timestamp -> TimeScale
///
/// `x_range` and `y_range` are the pixel ranges for x and y axes respectively.
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

    set
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
}
