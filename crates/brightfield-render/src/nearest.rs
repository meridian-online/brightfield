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

        if dist <= max_dist && best.as_ref().is_none_or(|b| dist < b.distance) {
            best = Some(NearestHit {
                row: i,
                point: Point::new(px, py),
                distance: dist,
            });
        }
    }

    best
}

/// Map a data value to pixel position using a scale.
fn map_value(scale: &Scale, value: f64) -> f64 {
    scale.map_f64(value)
}

/// Extract f64 values from a column regardless of source numeric type.
///
/// Covers every signed/unsigned integer width, both float widths, and
/// microsecond timestamps — the same breadth as the mark renderer's coercion.
/// DuckDB types inline YAML integers as `Int32`, so omitting the narrower widths
/// silently made `find_nearest` (and thus hover / point-selection) a no-op on
/// integer columns.
fn column_as_f64(batch: &RecordBatch, col_name: &str) -> Option<Vec<Option<f64>>> {
    use arrow::array::{
        Float32Array, Int16Array, Int32Array, Int8Array, UInt16Array, UInt32Array, UInt64Array,
        UInt8Array,
    };
    let idx = batch.schema().index_of(col_name).ok()?;
    let col = batch.column(idx);

    macro_rules! cast_numeric {
        ($arr_ty:ty) => {{
            let arr = col.as_any().downcast_ref::<$arr_ty>()?;
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
        }};
    }

    match col.data_type() {
        DataType::Float64 => cast_numeric!(Float64Array),
        DataType::Float32 => cast_numeric!(Float32Array),
        DataType::Int64 => cast_numeric!(Int64Array),
        DataType::Int32 => cast_numeric!(Int32Array),
        DataType::Int16 => cast_numeric!(Int16Array),
        DataType::Int8 => cast_numeric!(Int8Array),
        DataType::UInt64 => cast_numeric!(UInt64Array),
        DataType::UInt32 => cast_numeric!(UInt32Array),
        DataType::UInt16 => cast_numeric!(UInt16Array),
        DataType::UInt8 => cast_numeric!(UInt8Array),
        DataType::Timestamp(TimeUnit::Microsecond, _) => cast_numeric!(TimestampMicrosecondArray),
        _ => None,
    }
}

/// The numeric value of a single cell as `f64`, coerced from any supported
/// column type. `None` for a null, an out-of-range row, or a non-numeric column.
///
/// Point selection reads a datum's EXACT stored value this way: the
/// dispatched predicate is `col = value`, and equating the continuous click
/// coordinate would never match a discrete datum.
pub fn column_value_at(batch: &RecordBatch, col_name: &str, row: usize) -> Option<f64> {
    column_as_f64(batch, col_name)?.get(row).copied().flatten()
}

/// Whether `col` is a **continuous numeric** type whose value a point predicate
/// can equate as a bare SQL literal (`col = value`).
///
/// Integers and floats qualify. Temporal columns do NOT — a microsecond integer
/// literal won't equal a `TIMESTAMP`. Strings do NOT — they need a quoted,
/// escaped literal (and categorical nearest-resolution). Point selection is
/// scoped to numeric axes for now; callers skip the rest (see the point-
/// selection follow-up memo). `false` for a missing column.
pub fn is_numeric_point_column(batch: &RecordBatch, col_name: &str) -> bool {
    match batch.schema().field_with_name(col_name) {
        Ok(f) => matches!(
            f.data_type(),
            DataType::Int8
                | DataType::Int16
                | DataType::Int32
                | DataType::Int64
                | DataType::UInt8
                | DataType::UInt16
                | DataType::UInt32
                | DataType::UInt64
                | DataType::Float32
                | DataType::Float64
        ),
        Err(_) => false,
    }
}

/// A resolved point-selection value, carrying enough type to format a **correct
/// SQL literal** for `col = <literal>`.
///
/// - `Number` (float column) and `Int` (integer column) → a bare literal. `Int`
///   preserves the exact stored integer (an `f64` round-trip rounds keys above
///   2^53).
/// - `Text` (string / categorical column) → single-quoted, with embedded quotes
///   SQL-escaped by doubling.
/// - `Timestamp` (naive `TIMESTAMP`, microsecond epoch) → `make_timestamp(us)`. A
///   bare integer would raise a binder error — DuckDB has no implicit
///   `BIGINT → TIMESTAMP` cast.
/// - `TimestampTz` (`TIMESTAMP WITH TIME ZONE`, UTC microsecond epoch) →
///   `make_timestamp(us) AT TIME ZONE 'UTC'`. The bare `make_timestamp(us)` is a
///   *naive* value that DuckDB compares to a tz column by shifting through the
///   session's TimeZone (machine-local by default), so a plain literal silently
///   matches zero rows under any non-UTC session.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectionValue {
    /// A floating-point value (bare literal).
    Number(f64),
    /// An exact integer value (bare literal).
    Int(i64),
    /// A string / categorical value (quoted + escaped).
    Text(String),
    /// A naive-`TIMESTAMP` microsecond epoch (`make_timestamp(us)`).
    Timestamp(i64),
    /// A `TIMESTAMPTZ` UTC microsecond epoch (`make_timestamp(us) AT TIME ZONE 'UTC'`).
    TimestampTz(i64),
}

impl SelectionValue {
    /// Format as a SQL literal for the right-hand side of `col = <literal>`.
    #[must_use]
    pub fn literal(&self) -> String {
        match self {
            SelectionValue::Number(n) => n.to_string(),
            SelectionValue::Int(i) => i.to_string(),
            // Escape single quotes by doubling (standard SQL string escaping).
            SelectionValue::Text(s) => format!("'{}'", s.replace('\'', "''")),
            SelectionValue::Timestamp(us) => format!("make_timestamp({us})"),
            // Anchor the UTC epoch as a tz value so it compares to a TIMESTAMPTZ
            // column regardless of the session TimeZone.
            SelectionValue::TimestampTz(us) => format!("make_timestamp({us}) AT TIME ZONE 'UTC'"),
        }
    }
}

/// Read a datum's EXACT stored value as a [`SelectionValue`], preserving its type
/// so the dispatched `col = value` predicate is well-formed for the column.
///
/// `None` for a null, an out-of-range row, or an unsupported column type. Floats
/// that are non-finite (NaN/±inf) are `None` — they can't form a valid literal.
pub fn column_typed_value_at(
    batch: &RecordBatch,
    col_name: &str,
    row: usize,
) -> Option<SelectionValue> {
    use arrow::array::{
        Float32Array, Int16Array, Int32Array, Int8Array, LargeStringArray, StringArray,
        UInt16Array, UInt32Array, UInt64Array, UInt8Array,
    };
    let idx = batch.schema().index_of(col_name).ok()?;
    let col = batch.column(idx);
    if row >= col.len() || col.is_null(row) {
        return None;
    }

    macro_rules! int_val {
        ($arr_ty:ty) => {{
            let arr = col.as_any().downcast_ref::<$arr_ty>()?;
            Some(SelectionValue::Int(arr.value(row) as i64))
        }};
    }

    match col.data_type() {
        DataType::Float64 => {
            let v = col.as_any().downcast_ref::<Float64Array>()?.value(row);
            v.is_finite().then_some(SelectionValue::Number(v))
        }
        DataType::Float32 => {
            let v = f64::from(col.as_any().downcast_ref::<Float32Array>()?.value(row));
            v.is_finite().then_some(SelectionValue::Number(v))
        }
        DataType::Int64 => int_val!(Int64Array),
        DataType::Int32 => int_val!(Int32Array),
        DataType::Int16 => int_val!(Int16Array),
        DataType::Int8 => int_val!(Int8Array),
        DataType::UInt64 => int_val!(UInt64Array),
        DataType::UInt32 => int_val!(UInt32Array),
        DataType::UInt16 => int_val!(UInt16Array),
        DataType::UInt8 => int_val!(UInt8Array),
        DataType::Utf8 => Some(SelectionValue::Text(
            col.as_any()
                .downcast_ref::<StringArray>()?
                .value(row)
                .to_string(),
        )),
        DataType::LargeUtf8 => Some(SelectionValue::Text(
            col.as_any()
                .downcast_ref::<LargeStringArray>()?
                .value(row)
                .to_string(),
        )),
        // A tz field (TIMESTAMPTZ) stores UTC microseconds and needs a tz-anchored
        // literal; a naive TIMESTAMP (no tz) uses the plain form.
        DataType::Timestamp(TimeUnit::Microsecond, tz) => {
            let us = col
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()?
                .value(row);
            Some(if tz.is_some() {
                SelectionValue::TimestampTz(us)
            } else {
                SelectionValue::Timestamp(us)
            })
        }
        _ => None,
    }
}

/// The category whose band centre is nearest `pixel` on a [`Scale::Band`] axis —
/// a pixel→category inverse. `None` for a non-Band scale or an empty domain.
///
/// A categorical point selection resolves the clicked category this way (there is
/// no datum to snap to in the numeric sense — the axis position *is* the value),
/// then dispatches `col = 'category'`.
#[must_use]
pub fn band_category_at(scale: &Scale, pixel: f64) -> Option<String> {
    match scale {
        Scale::Band { categories, .. } => categories
            .iter()
            .min_by(|a, b| {
                let da = (scale.map_category(a).unwrap_or(f64::INFINITY) - pixel).abs();
                let db = (scale.map_category(b).unwrap_or(f64::INFINITY) - pixel).abs();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned(),
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

    // NearestMode variants are exhaustive
    #[test]
    fn nearest_mode_variants() {
        let modes = [NearestMode::X, NearestMode::Y, NearestMode::XY];
        assert_eq!(modes.len(), 3);
    }

    #[test]
    fn nearest_hit_fields() {
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

    // find_nearest functionality
    #[test]
    fn exact_hit_returns_some() {
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
        assert!(
            hit.distance < 1.0,
            "distance should be ~0, got {}",
            hit.distance
        );
    }

    #[test]
    fn far_away_returns_none() {
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
    fn mode_x_ignores_y_distance() {
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
        assert!(
            hit.is_some(),
            "X mode should find point at same x regardless of y"
        );
        assert_eq!(hit.unwrap().row, 2);
    }

    #[test]
    fn mode_y_ignores_x_distance() {
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
        assert!(
            hit.is_some(),
            "Y mode should find point at same y regardless of x"
        );
        assert_eq!(hit.unwrap().row, 2);
    }

    #[test]
    fn custom_max_distance() {
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
        assert!(
            miss_x.is_none(),
            "10px offset with 5px max should miss in X mode"
        );

        // With max_distance=15, should hit
        let hit = find_nearest(
            Point::new(px, 235.0),
            &batch,
            &cm,
            &scales,
            NearestMode::X,
            Some(15.0),
        );
        assert!(
            hit.is_some(),
            "10px offset with 15px max should hit in X mode"
        );
    }

    #[test]
    fn selection_value_literals_are_type_correct() {
        assert_eq!(SelectionValue::Int(42).literal(), "42");
        assert_eq!(SelectionValue::Number(3.5).literal(), "3.5");
        // Whole floats print without a decimal point (matches point_predicate tests).
        assert_eq!(SelectionValue::Number(3.0).literal(), "3");
        // Strings are quoted; embedded single quotes are doubled.
        assert_eq!(
            SelectionValue::Text("Athletics".into()).literal(),
            "'Athletics'"
        );
        assert_eq!(SelectionValue::Text("O'Hara".into()).literal(), "'O''Hara'");
        // Naive timestamp → make_timestamp, never a bare integer.
        assert_eq!(
            SelectionValue::Timestamp(1_700_000_000_000_000).literal(),
            "make_timestamp(1700000000000000)"
        );
        // tz timestamp → anchored to UTC so it matches a TIMESTAMPTZ column under
        // any session TimeZone.
        assert_eq!(
            SelectionValue::TimestampTz(1_700_000_000_000_000).literal(),
            "make_timestamp(1700000000000000) AT TIME ZONE 'UTC'"
        );
    }

    #[test]
    fn column_typed_value_at_dispatches_on_arrow_type() {
        use arrow::array::{Int32Array, StringArray};
        use arrow::datatypes::{DataType, Field, Schema};
        use std::sync::Arc;

        let schema = Arc::new(Schema::new(vec![
            Field::new("i", DataType::Int32, true),
            Field::new("s", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int32Array::from(vec![Some(7), None])),
                Arc::new(StringArray::from(vec![Some("beta"), Some("gamma")])),
            ],
        )
        .unwrap();

        // Int32 (DuckDB's inline-int type) → exact Int, not a rounded f64.
        assert_eq!(
            column_typed_value_at(&batch, "i", 0),
            Some(SelectionValue::Int(7))
        );
        // Utf8 → Text.
        assert_eq!(
            column_typed_value_at(&batch, "s", 0),
            Some(SelectionValue::Text("beta".into()))
        );
        // Null cell → None; out-of-range row → None; missing column → None.
        assert_eq!(column_typed_value_at(&batch, "i", 1), None);
        assert_eq!(column_typed_value_at(&batch, "i", 99), None);
        assert_eq!(column_typed_value_at(&batch, "nope", 0), None);
    }

    #[test]
    fn band_category_at_maps_pixel_to_nearest_category() {
        let scale = Scale::Band {
            categories: vec!["a".into(), "b".into(), "c".into()],
            range_start: 0.0,
            range_end: 300.0,
            padding: 0.1,
        };
        // Each category's own centre round-trips back to that category.
        for cat in ["a", "b", "c"] {
            let px = scale.map_category(cat).unwrap();
            assert_eq!(band_category_at(&scale, px).as_deref(), Some(cat));
        }
        // A non-band scale yields None.
        let linear = Scale::Linear {
            domain_min: 0.0,
            domain_max: 1.0,
            range_start: 0.0,
            range_end: 1.0,
        };
        assert_eq!(band_category_at(&linear, 0.5), None);
    }
}
