//! Scene builder — orchestrates data -> scales -> marks + axes + legend
//! into a single vello::Scene.

use arrow::record_batch::RecordBatch;
use vello::Scene;

use crate::axis::{compute_ticks, render_x_axis, render_y_axis};
use crate::channel::{Channel, ChannelMap};
use crate::grid::{render_x_grid, render_y_grid};
use crate::layout::ChartLayout;
use crate::legend::render_colour_legend;
use crate::mark::{HighlightState, MarkRenderer};
use crate::scale::{infer_scales, Scale, ScaleSet, ViewExtent};

/// Input data for building a chart scene.
pub struct ChartData<'a> {
    /// The Arrow record batch containing the data.
    pub batch: &'a RecordBatch,
    /// The channel map (encoding channels -> column names).
    pub channel_map: &'a ChannelMap,
    /// The mark renderer to use.
    pub renderer: &'a dyn MarkRenderer,
    /// Chart layout (dimensions and margins).
    pub layout: ChartLayout,
    /// Optional view extent override for pan/zoom navigation.
    /// When `Some`, scale domains are overridden to the specified range.
    /// When `None`, the full data-inferred domain is used.
    pub view_extent: Option<&'a ViewExtent>,
    /// Optional highlight state for per-row dim/emphasis.
    /// When `Some`, matching rows render at full alpha; non-matching rows are dimmed.
    pub highlight: Option<&'a HighlightState>,
}

/// Build a complete chart scene from data and configuration.
///
/// Orchestrates: infer scales -> render grid -> render marks -> render axes -> render legend.
/// Returns the scene and the inferred scales (for interaction coordinate mapping).
/// Override a scale's domain with the given min/max values.
/// Only applies to continuous scales (Linear, Time). Band and Colour are returned unchanged.
fn override_scale_domain(scale: &Scale, new_min: f64, new_max: f64) -> Scale {
    match scale {
        Scale::Linear {
            range_start,
            range_end,
            ..
        } => Scale::Linear {
            domain_min: new_min,
            domain_max: new_max,
            range_start: *range_start,
            range_end: *range_end,
        },
        Scale::Time {
            range_start,
            range_end,
            ..
        } => Scale::Time {
            domain_min_us: new_min as i64,
            domain_max_us: new_max as i64,
            range_start: *range_start,
            range_end: *range_end,
        },
        // Band and Colour scales are not navigable — return unchanged.
        other => other.clone(),
    }
}

pub fn build_chart_scene(data: &ChartData<'_>) -> (Scene, ScaleSet) {
    let mut scene = Scene::new();

    let mut scales = infer_scales(
        data.batch,
        data.channel_map,
        data.layout.x_range(),
        data.layout.y_range(),
    );

    // Apply view extent override for pan/zoom navigation.
    if let Some(extent) = data.view_extent {
        if let Some((x_min, x_max)) = extent.x {
            if let Some(x_scale) = scales.get(Channel::X) {
                let overridden = override_scale_domain(x_scale, x_min, x_max);
                scales.insert(Channel::X, overridden);
            }
        }
        if let Some((y_min, y_max)) = extent.y {
            if let Some(y_scale) = scales.get(Channel::Y) {
                let overridden = override_scale_domain(y_scale, y_min, y_max);
                scales.insert(Channel::Y, overridden);
            }
        }
    }

    // Grid lines (behind marks).
    if let Some(x_scale) = scales.get(Channel::X) {
        let x_ticks = compute_ticks(x_scale, 5);
        render_x_grid(&mut scene, &data.layout, &x_ticks);
    }
    if let Some(y_scale) = scales.get(Channel::Y) {
        let y_ticks = compute_ticks(y_scale, 5);
        render_y_grid(&mut scene, &data.layout, &y_ticks);
    }

    // Marks.
    data.renderer
        .render(&mut scene, data.batch, data.channel_map, &scales, data.highlight);

    // Axes (on top of grid/marks).
    if let Some(x_scale) = scales.get(Channel::X) {
        let x_ticks = compute_ticks(x_scale, 5);
        render_x_axis(&mut scene, &data.layout, &x_ticks);
    }
    if let Some(y_scale) = scales.get(Channel::Y) {
        let y_ticks = compute_ticks(y_scale, 5);
        render_y_axis(&mut scene, &data.layout, &y_ticks);
    }

    // Colour legend.
    if let Some(fill_scale) = scales.get(Channel::Fill) {
        render_colour_legend(&mut scene, &data.layout, fill_scale);
    }

    (scene, scales)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{Channel, ChannelMap};
    use crate::layout::ChartLayout;
    use crate::mark::{BarRenderer, DotRenderer, LineRenderer};
    use arrow::array::{Float64Array, StringArray, TimestampMicrosecondArray};
    use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
    use arrow::record_batch::RecordBatch;
    use std::sync::Arc;

    #[test]
    fn gpu_ac08_build_dot_chart_scene() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("colour", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 15.0, 25.0, 30.0])),
                Arc::new(StringArray::from(vec!["a", "b", "a", "b", "a"])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        cm.insert(Channel::Fill, "colour".to_string());

        let layout = ChartLayout::new(640.0, 480.0);
        let renderer = DotRenderer;

        let data = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &renderer,
            layout,
            view_extent: None,
            highlight: None,
        };

        let (scene, scales) = build_chart_scene(&data);

        // Scene should be non-empty.
        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "dot chart scene should have content"
        );

        // Scales should include x, y, and fill.
        assert!(scales.get(Channel::X).is_some(), "x scale should exist");
        assert!(scales.get(Channel::Y).is_some(), "y scale should exist");
        assert!(scales.get(Channel::Fill).is_some(), "fill scale should exist");
    }

    #[test]
    fn gpu_ac08_build_bar_chart_scene() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["a", "b", "c"])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "category".to_string());
        cm.insert(Channel::Y, "value".to_string());

        let layout = ChartLayout::new(640.0, 480.0);
        let renderer = BarRenderer;

        let data = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &renderer,
            layout,
            view_extent: None,
            highlight: None,
        };

        let (scene, _scales) = build_chart_scene(&data);

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "bar chart scene should have content"
        );
    }

    #[test]
    fn gpu_ac08_build_line_chart_scene() {
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
                    1_000_000, 2_000_000, 3_000_000, 4_000_000,
                ])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 15.0, 25.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "ts".to_string());
        cm.insert(Channel::Y, "value".to_string());

        let layout = ChartLayout::new(640.0, 480.0);
        let renderer = LineRenderer;

        let data = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &renderer,
            layout,
            view_extent: None,
            highlight: None,
        };

        let (scene, _scales) = build_chart_scene(&data);

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "line chart scene should have content"
        );
    }

    #[test]
    fn nav_ac03_view_extent_overrides_scale_domain() {
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

        let layout = ChartLayout::new(640.0, 480.0);
        let renderer = DotRenderer;

        // Without view extent — full data domain.
        let data_full = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &renderer,
            layout: layout.clone(),
            view_extent: None,
            highlight: None,
        };
        let (_scene, scales_full) = build_chart_scene(&data_full);
        let x_full = scales_full.get(Channel::X).unwrap();
        assert!((x_full.domain_min().unwrap() - 1.0).abs() < f64::EPSILON);
        assert!((x_full.domain_max().unwrap() - 5.0).abs() < f64::EPSILON);

        // With view extent — narrowed x domain.
        let extent = ViewExtent {
            x: Some((2.0, 4.0)),
            y: None,
        };
        let data_zoomed = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &renderer,
            layout,
            view_extent: Some(&extent),
            highlight: None,
        };
        let (_scene, scales_zoomed) = build_chart_scene(&data_zoomed);
        let x_zoomed = scales_zoomed.get(Channel::X).unwrap();
        assert!((x_zoomed.domain_min().unwrap() - 2.0).abs() < f64::EPSILON);
        assert!((x_zoomed.domain_max().unwrap() - 4.0).abs() < f64::EPSILON);

        // Y should be unchanged.
        let y_zoomed = scales_zoomed.get(Channel::Y).unwrap();
        assert!((y_zoomed.domain_min().unwrap() - 10.0).abs() < f64::EPSILON);
        assert!((y_zoomed.domain_max().unwrap() - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn ifb_ac05_build_chart_scene_with_highlight() {
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

        let layout = ChartLayout::new(640.0, 480.0);
        let renderer = DotRenderer;

        let hs = crate::mark::HighlightState {
            predicate: Box::new(|row| row == 1),
            dimmed_alpha: 0.15,
        };

        // With highlight
        let data = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &renderer,
            layout: layout.clone(),
            view_extent: None,
            highlight: Some(&hs),
        };
        let (scene, _scales) = build_chart_scene(&data);
        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "scene with highlight should have content"
        );

        // Without highlight (backward compat)
        let data_no_hl = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &renderer,
            layout,
            view_extent: None,
            highlight: None,
        };
        let (scene2, _) = build_chart_scene(&data_no_hl);
        let encoding2 = scene2.encoding();
        assert!(
            encoding2.path_tags.len() > 0,
            "scene without highlight should also work"
        );
    }
}
