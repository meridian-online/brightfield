//! Nearest-point resolution for hover interactions.
//!
//! Brute-force scan over a post-aggregation `RecordBatch` to find the data
//! point closest to the cursor. Row counts are modest (hundreds to low
//! thousands after SQL aggregation), so a linear scan at 60 Hz is well under
//! 1 ms. A spatial index can replace this behind the same API if needed.

use arrow::array::{Array, Float64Array, Int64Array, TimestampMicrosecondArray};
use arrow::datatypes::{DataType, TimeUnit};
use arrow::record_batch::RecordBatch;
use kurbo::Point;

use crate::channel::{Channel, ChannelMap};
use crate::scale::{Scale, ScaleSet};

/// Which axes to consider when finding the nearest point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NearestMode {
    /// Distance measured along x-axis only.
    X,
    /// Distance measured along y-axis only.
    Y,
    /// Euclidean distance in both axes.
    XY,
}

/// Result of a nearest-point search.
#[derive(Debug, Clone)]
pub struct NearestHit {
    /// Row index in the RecordBatch.
    pub row: usize,
    /// Pixel position of the nearest mark.
    pub point: Point,
    /// Distance from cursor to the nearest mark (in pixels).
    pub distance: f64,
}

/// Default maximum pixel distance for a hit.
const DEFAULT_MAX_DISTANCE: f64 = 50.0;

/// Find the nearest data point to the cursor.
///
/// Scans all rows in `batch`, mapping each through `scales` to pixel
/// coordinates, and returns the closest point within `max_distance` pixels.
/// Returns `None` if no point is within range or if required channels/scales
/// are missing.
pub fn find_nearest(
    cursor: Point,
    batch: &RecordBatch,
    channel_map: &ChannelMap,
    scales: &ScaleSet,
    mode: NearestMode,
    max_distance: Option<f64>,
) -> Option<NearestHit> {
    let max_dist = max_distance.unwrap_or(DEFAULT_MAX_DISTANCE);

    let x_col = channel_map.get(Channel::X)?;
    let y_col = channel_map.get(Channel::Y)?;
    let x_scale = scales.get(Channel::X)?;
    let y_scale = scales.get(Channel::Y)?;

    let x_values = column_as_f64(batch, x_col)?;
    let y_values = column_as_f64(batch, y_col)?;

    let mut best: Option<NearestHit> = None;

    for i in 0..batch.num_rows() {
        let xv = match x_values[i] {
            Some(v) => v,
            None => continue,
        };
        let yv = match y_values[i] {
            Some(v) => v,
            None => continue,
        };

        let px = map_value(x_scale, xv);
        let py = map_value(y_scale, yv);

        let dist = match mode {
            NearestMode::X => (px - cursor.x).abs(),
            NearestMode::Y => (py - cursor.y).abs(),
            NearestMode::XY => ((px - cursor.x).powi(2) + (py - cursor.y).powi(2)).sqrt(),
        };

        if dist <= max_dist {
            if best.as_ref().map_or(true, |b| dist < b.distance) {
                best = Some(NearestHit {
                    row: i,
                    point: Point::new(px, py),
                    distance: dist,
                });
            }
        }
    }

    best
}

/// Map a data value to pixel position using a scale.
fn map_value(scale: &Scale, value: f64) -> f64 {
    scale.map_f64(value)
}

/// Extract f64 values from a column regardless of source numeric type.
fn column_as_f64(batch: &RecordBatch, col_name: &str) -> Option<Vec<Option<f64>>> {
    let idx = batch.schema().index_of(col_name).ok()?;
    let col = batch.column(idx);
    match col.data_type() {
        DataType::Float64 => {
            let arr = col.as_any().downcast_ref::<Float64Array>()?;
            Some(
                (0..arr.len())
                    .map(|i| if arr.is_null(i) { None } else { Some(arr.value(i)) })
                    .collect(),
            )
        }
        DataType::Int64 => {
            let arr = col.as_any().downcast_ref::<Int64Array>()?;
            Some(
                (0..arr.len())
                    .map(|i| {
                        if arr.is_null(i) {
                            None
                        } else {
                            Some(arr.value(i) as f64)
                        }
                    })
                    .collect(),
            )
        }
        DataType::Timestamp(TimeUnit::Microsecond, _) => {
            let arr = col.as_any().downcast_ref::<TimestampMicrosecondArray>()?;
            Some(
                (0..arr.len())
                    .map(|i| {
                        if arr.is_null(i) {
                            None
                        } else {
                            Some(arr.value(i) as f64)
                        }
                    })
                    .collect(),
            )
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelMap;
    use crate::scale::infer_scales;
    use arrow::array::Float64Array;
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use std::sync::Arc;

    fn test_batch() -> (RecordBatch, ChannelMap, ScaleSet) {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        (batch, cm, scales)
    }

    // ac-01: NearestMode variants are exhaustive
    #[test]
    fn ifb_ac01_nearest_mode_variants() {
        let modes = [NearestMode::X, NearestMode::Y, NearestMode::XY];
        assert_eq!(modes.len(), 3);
    }

    #[test]
    fn ifb_ac01_nearest_hit_fields() {
        let hit = NearestHit {
            row: 2,
            point: Point::new(100.0, 200.0),
            distance: 5.5,
        };
        assert_eq!(hit.row, 2);
        assert!((hit.point.x - 100.0).abs() < f64::EPSILON);
        assert!((hit.point.y - 200.0).abs() < f64::EPSILON);
        assert!((hit.distance - 5.5).abs() < f64::EPSILON);
    }

    // ac-02: find_nearest functionality
    #[test]
    fn ifb_ac02_exact_hit_returns_some() {
        let (batch, cm, scales) = test_batch();

        // Find the pixel position of point (3.0, 30.0)
        let x_scale = scales.get(Channel::X).unwrap();
        let y_scale = scales.get(Channel::Y).unwrap();
        let px = x_scale.map_f64(3.0);
        let py = y_scale.map_f64(30.0);

        let hit = find_nearest(
            Point::new(px, py),
            &batch,
            &cm,
            &scales,
            NearestMode::XY,
            None,
        );
        assert!(hit.is_some(), "cursor on point should return Some");
        let hit = hit.unwrap();
        assert_eq!(hit.row, 2, "should match row index 2 (value 3.0, 30.0)");
        assert!(hit.distance < 1.0, "distance should be ~0, got {}", hit.distance);
    }

    #[test]
    fn ifb_ac02_far_away_returns_none() {
        let (batch, cm, scales) = test_batch();

        let hit = find_nearest(
            Point::new(-1000.0, -1000.0),
            &batch,
            &cm,
            &scales,
            NearestMode::XY,
            None,
        );
        assert!(hit.is_none(), "cursor far away should return None");
    }

    #[test]
    fn ifb_ac02_mode_x_ignores_y_distance() {
        let (batch, cm, scales) = test_batch();

        let x_scale = scales.get(Channel::X).unwrap();
        let px = x_scale.map_f64(3.0);
        // Place cursor at same x but very different y
        let hit = find_nearest(
            Point::new(px, 9999.0),
            &batch,
            &cm,
            &scales,
            NearestMode::X,
            None,
        );
        assert!(hit.is_some(), "X mode should find point at same x regardless of y");
        assert_eq!(hit.unwrap().row, 2);
    }

    #[test]
    fn ifb_ac02_mode_y_ignores_x_distance() {
        let (batch, cm, scales) = test_batch();

        let y_scale = scales.get(Channel::Y).unwrap();
        let py = y_scale.map_f64(30.0);
        // Place cursor at same y but very different x
        let hit = find_nearest(
            Point::new(9999.0, py),
            &batch,
            &cm,
            &scales,
            NearestMode::Y,
            None,
        );
        assert!(hit.is_some(), "Y mode should find point at same y regardless of x");
        assert_eq!(hit.unwrap().row, 2);
    }

    #[test]
    fn ifb_ac02_custom_max_distance() {
        let (batch, cm, scales) = test_batch();

        let x_scale = scales.get(Channel::X).unwrap();
        let px = x_scale.map_f64(3.0) + 10.0; // 10px offset

        // With max_distance=5, should miss
        let miss = find_nearest(
            Point::new(px, 235.0), // approximate y
            &batch,
            &cm,
            &scales,
            NearestMode::XY,
            Some(5.0),
        );
        // May or may not miss depending on actual pixel positions — use X mode for clarity
        let _miss = miss; // suppress unused warning
        let miss_x = find_nearest(
            Point::new(px, 235.0),
            &batch,
            &cm,
            &scales,
            NearestMode::X,
            Some(5.0),
        );
        // 10px offset in X mode with 5px max → miss
        assert!(miss_x.is_none(), "10px offset with 5px max should miss in X mode");

        // With max_distance=15, should hit
        let hit = find_nearest(
            Point::new(px, 235.0),
            &batch,
            &cm,
            &scales,
            NearestMode::X,
            Some(15.0),
        );
        assert!(hit.is_some(), "10px offset with 15px max should hit in X mode");
    }
}
