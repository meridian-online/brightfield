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
            _ => None,
        }
    }
}

/// Maps visual encoding channels to column names in the RecordBatch.
#[derive(Debug, Clone, Default)]
pub struct ChannelMap {
    map: HashMap<Channel, String>,
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

    /// Look up the column name for a channel.
    pub fn get(&self, channel: Channel) -> Option<&str> {
        self.map.get(&channel).map(|s| s.as_str())
    }

    /// True if the channel is mapped.
    pub fn has(&self, channel: Channel) -> bool {
        self.map.contains_key(&channel)
    }

    /// Extract a ChannelMap from a mark's options.
    ///
    /// Scans the mark's options for known channel names (x, y, fill, etc.)
    /// and maps them to column name strings.
    pub fn from_mark(mark: &Mark) -> Self {
        let mut cm = Self::new();
        for ch in Channel::all() {
            if let Some(val) = mark.options.get(ch.wire_name()) {
                if let ValueOrParamRef::Value(SpecValue::String(col)) = val {
                    cm.insert(*ch, col.clone());
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
}
