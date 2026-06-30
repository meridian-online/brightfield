//! ChannelMap — typed channel extraction from mark options.
//!
//! A ChannelMap maps visual encoding channels (x, y, fill, stroke, size, etc.)
//! to column names in the RecordBatch. This bridges the spec's mark options
//! and the rendering pipeline.

use std::collections::HashMap;

use brightfield_spec::ast::{Mark, SpecValue, ValueOrParamRef};

/// Visual encoding channels recognised by the rendering pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    X,
    Y,
    Fill,
    Stroke,
    Size,
    X1,
    Y1,
    X2,
    Y2,
    /// Text content channel (the label column for text marks).
    Text,
}

impl Channel {
    /// The wire name as it appears in Mosaic mark options.
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
            Self::Fill => "fill",
            Self::Stroke => "stroke",
            Self::Size => "size",
            Self::X1 => "x1",
            Self::Y1 => "y1",
            Self::X2 => "x2",
            Self::Y2 => "y2",
            Self::Text => "text",
        }
    }

    /// All known channel wire names.
    pub fn all() -> &'static [Self] {
        &[
            Self::X,
            Self::Y,
            Self::Fill,
            Self::Stroke,
            Self::Size,
            Self::X1,
            Self::Y1,
            Self::X2,
            Self::Y2,
            Self::Text,
        ]
    }

    /// Look up a channel by wire name.
    pub fn from_wire(name: &str) -> Option<Self> {
        match name {
            "x" => Some(Self::X),
            "y" => Some(Self::Y),
            "fill" => Some(Self::Fill),
            "stroke" => Some(Self::Stroke),
            "size" => Some(Self::Size),
            "x1" => Some(Self::X1),
            "y1" => Some(Self::Y1),
            "x2" => Some(Self::X2),
            "y2" => Some(Self::Y2),
            "text" => Some(Self::Text),
            _ => None,
        }
    }
}

/// Maps visual encoding channels to column names in the RecordBatch, plus any
/// channels bound to a constant **literal** value (e.g. `y: 0` for a baseline
/// rule). Columns and literals are kept in separate maps so existing
/// column-based renderers are unaffected; renderers that accept constants read
/// [`ChannelMap::literal`].
#[derive(Debug, Clone, Default)]
pub struct ChannelMap {
    map: HashMap<Channel, String>,
    literals: HashMap<Channel, f64>,
}

impl ChannelMap {
    /// Create an empty channel map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a channel -> column mapping.
    pub fn insert(&mut self, channel: Channel, column: String) {
        self.map.insert(channel, column);
    }

    /// Insert a channel -> constant literal value (e.g. `y: 0`).
    pub fn insert_literal(&mut self, channel: Channel, value: f64) {
        self.literals.insert(channel, value);
    }

    /// Look up the column name for a channel.
    pub fn get(&self, channel: Channel) -> Option<&str> {
        self.map.get(&channel).map(|s| s.as_str())
    }

    /// Look up the constant literal value bound to a channel, if any.
    pub fn literal(&self, channel: Channel) -> Option<f64> {
        self.literals.get(&channel).copied()
    }

    /// Iterator over channels bound to a literal value.
    pub fn literals_iter(&self) -> impl Iterator<Item = (Channel, f64)> + '_ {
        self.literals.iter().map(|(c, v)| (*c, *v))
    }

    /// True if the channel is mapped to a column.
    pub fn has(&self, channel: Channel) -> bool {
        self.map.contains_key(&channel)
    }

    /// Extract a ChannelMap from a mark's options.
    ///
    /// Scans the mark's options for known channel names (x, y, fill, etc.)
    /// and maps them to column name strings. ParamRef channels are skipped
    /// with a warning — they require reactive parameter resolution which is
    /// not yet implemented.
    pub fn from_mark(mark: &Mark) -> Self {
        let mut cm = Self::new();
        for ch in Channel::all() {
            if let Some(val) = mark.options.get(ch.wire_name()) {
                match val {
                    ValueOrParamRef::Value(SpecValue::String(col)) => {
                        cm.insert(*ch, col.clone());
                    }
                    // Numeric literals bind the channel to a constant for all
                    // rows (e.g. `y: 0` for a baseline rule).
                    ValueOrParamRef::Value(SpecValue::Integer(i)) => {
                        cm.insert_literal(*ch, *i as f64);
                    }
                    ValueOrParamRef::Value(SpecValue::Float(f)) => {
                        cm.insert_literal(*ch, *f);
                    }
                    ValueOrParamRef::Param(param_ref) => {
                        eprintln!(
                            "warning: skipping channel `{}` bound to param `{}` \
                             — reactive parameters not yet supported",
                            ch.wire_name(),
                            param_ref.to_wire()
                        );
                    }
                    _ => {}
                }
            }
        }
        cm
    }

    /// Iterator over all mapped channels.
    pub fn iter(&self) -> impl Iterator<Item = (&Channel, &String)> {
        self.map.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_channel_from_wire_round_trips() {
        for ch in Channel::all() {
            assert_eq!(Channel::from_wire(ch.wire_name()), Some(*ch));
        }
        assert_eq!(Channel::from_wire("unknown"), None);
    }

    #[test]
    fn gpu_channel_map_insert_and_get() {
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "col_x".to_string());
        cm.insert(Channel::Fill, "species".to_string());
        assert_eq!(cm.get(Channel::X), Some("col_x"));
        assert_eq!(cm.get(Channel::Fill), Some("species"));
        assert_eq!(cm.get(Channel::Y), None);
        assert!(cm.has(Channel::X));
        assert!(!cm.has(Channel::Y));
    }

    #[test]
    fn msv_ac06_from_mark_skips_param_ref() {
        use brightfield_spec::ast::{Mark, ParamRef, SpecValue, ValueOrParamRef};
        use brightfield_spec::vocab::{ImplStatus, MarkKind};

        let mut options: indexmap::IndexMap<String, ValueOrParamRef<SpecValue>> =
            Default::default();
        // x is a ParamRef — should be skipped
        options.insert(
            "x".to_string(),
            ValueOrParamRef::Param(ParamRef::new("slider1")),
        );
        // y is a literal string — should be included
        options.insert(
            "y".to_string(),
            ValueOrParamRef::Value(SpecValue::String("col_y".to_string())),
        );

        let mark = Mark {
            kind: MarkKind::Dot,
            status: ImplStatus::Unimplemented,
            data: None,
            options,
        };

        let cm = ChannelMap::from_mark(&mark);
        // x should be absent (ParamRef skipped)
        assert!(!cm.has(Channel::X), "ParamRef channel should be skipped");
        // y should be present (literal string)
        assert_eq!(cm.get(Channel::Y), Some("col_y"));
    }

    #[test]
    fn from_mark_captures_numeric_literal_channel() {
        use brightfield_spec::ast::{Mark, SpecValue, ValueOrParamRef};
        use brightfield_spec::vocab::{ImplStatus, MarkKind};

        let mut options: indexmap::IndexMap<String, ValueOrParamRef<SpecValue>> =
            Default::default();
        // x is a column reference; y is a numeric literal (e.g. a baseline).
        options.insert(
            "x".to_string(),
            ValueOrParamRef::Value(SpecValue::String("col_x".to_string())),
        );
        options.insert("y".to_string(), ValueOrParamRef::Value(SpecValue::Integer(0)));

        let mark = Mark {
            kind: MarkKind::RuleY,
            status: ImplStatus::Implemented,
            data: None,
            options,
        };

        let cm = ChannelMap::from_mark(&mark);
        assert_eq!(cm.get(Channel::X), Some("col_x"), "x is a column");
        assert_eq!(cm.get(Channel::Y), None, "a literal y is not a column");
        assert_eq!(cm.literal(Channel::Y), Some(0.0), "literal y captured as 0.0");
        assert_eq!(cm.literal(Channel::X), None, "x has no literal");
    }
}
