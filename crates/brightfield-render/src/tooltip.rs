//! Tooltip content extraction from Arrow RecordBatch rows.
//!
//! This module provides data extraction only — the GPUI tooltip element
//! that renders this content is deferred to a follow-up card.

use arrow::array::{Array, Float64Array, Int64Array, StringArray, TimestampMicrosecondArray};
use arrow::datatypes::{DataType, TimeUnit};
use arrow::record_batch::RecordBatch;

/// Tooltip content: field name/value pairs extracted from a RecordBatch row.
#[derive(Debug, Clone, PartialEq)]
pub struct TooltipContent {
    /// Field name/value pairs (column name, formatted value string).
    pub fields: Vec<(String, String)>,
}

impl TooltipContent {
    /// Extract tooltip content from a RecordBatch at the given row index.
    ///
    /// Reads all columns and formats their values as human-readable strings.
    /// Returns `None` if the row index is out of bounds.
    pub fn from_row(batch: &RecordBatch, row: usize) -> Option<Self> {
        if row >= batch.num_rows() {
            return None;
        }

        let mut fields = Vec::with_capacity(batch.num_columns());
        for (i, field) in batch.schema().fields().iter().enumerate() {
            let col = batch.column(i);
            let value = format_cell(col.as_ref(), row);
            fields.push((field.name().clone(), value));
        }

        Some(TooltipContent { fields })
    }
}

/// Format a single cell value as a string.
fn format_cell(col: &dyn Array, row: usize) -> String {
    if col.is_null(row) {
        return "null".to_string();
    }

    match col.data_type() {
        DataType::Float64 => {
            let arr = col.as_any().downcast_ref::<Float64Array>().unwrap();
            format!("{:.2}", arr.value(row))
        }
        DataType::Int64 => {
            let arr = col.as_any().downcast_ref::<Int64Array>().unwrap();
            arr.value(row).to_string()
        }
        DataType::Utf8 => {
            let arr = col.as_any().downcast_ref::<StringArray>().unwrap();
            arr.value(row).to_string()
        }
        DataType::Timestamp(TimeUnit::Microsecond, _) => {
            let arr = col
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap();
            // Format as seconds since epoch for now.
            let us = arr.value(row);
            format!("{}us", us)
        }
        _ => format!("<{}>", col.data_type()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Float64Array;
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use std::sync::Arc;

    #[test]
    fn tooltip_from_row() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("label", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
                Arc::new(StringArray::from(vec!["a", "b", "c"])),
            ],
        )
        .unwrap();

        let content = TooltipContent::from_row(&batch, 1).unwrap();
        assert_eq!(content.fields.len(), 3);
        assert_eq!(content.fields[0], ("x".to_string(), "2.00".to_string()));
        assert_eq!(content.fields[1], ("y".to_string(), "20.00".to_string()));
        assert_eq!(content.fields[2], ("label".to_string(), "b".to_string()));
    }

    #[test]
    fn tooltip_out_of_bounds() {
        let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Float64, false)]));
        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(Float64Array::from(vec![1.0]))]).unwrap();

        assert!(TooltipContent::from_row(&batch, 5).is_none());
    }

    #[test]
    fn tooltip_field_names_match() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("timestamp", DataType::Float64, false),
            Field::new("price", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1000.0])),
                Arc::new(Float64Array::from(vec![42.5])),
            ],
        )
        .unwrap();

        let content = TooltipContent::from_row(&batch, 0).unwrap();
        assert_eq!(content.fields[0].0, "timestamp");
        assert_eq!(content.fields[1].0, "price");
    }
}
