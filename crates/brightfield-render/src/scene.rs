//! Scene builder — orchestrates data -> scales -> marks + axes + legend
//! into a single vello::Scene.

use arrow::record_batch::RecordBatch;
use vello::Scene;

use crate::axis::{compute_ticks, render_x_axis, render_y_axis};
use crate::channel::{Channel, ChannelMap};
use crate::grid::{render_x_grid, render_y_grid};
use crate::layout::ChartLayout;
use crate::legend::render_colour_legend;
use crate::mark::MarkRenderer;
use crate::scale::{infer_scales, ScaleSet};

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
}

/// Build a complete chart scene from data and configuration.
///
/// Orchestrates: infer scales -> render grid -> render marks -> render axes -> render legend.
/// Returns the scene and the inferred scales (for interaction coordinate mapping).
pub fn build_chart_scene(data: &ChartData<'_>) -> (Scene, ScaleSet) {
    let mut scene = Scene::new();

    let scales = infer_scales(
        data.batch,
        data.channel_map,
        data.layout.x_range(),
        data.layout.y_range(),
    );

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
        .render(&mut scene, data.batch, data.channel_map, &scales);

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
        };

        let (scene, _scales) = build_chart_scene(&data);

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "line chart scene should have content"
        );
    }
}
