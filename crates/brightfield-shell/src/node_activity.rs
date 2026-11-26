//! **The activities of the node in focus**: what a node can be looked at as,
//! and which of those the window can show now.
//!
//! A table or view is looked at as its rows (Grid) or as the chart that reads
//! it (Chart). A chart is looked at as itself, or as the rows it reads. The
//! model step is looked at as the model. Each activity is one verb in the key
//! registry, so the keys, the menus and the ribbon perform the same command,
//! and a greyed activity says the same sentence wherever it is drawn.

use brightfield_keys::activity::{RELATE, SHOW_CHART, SHOW_GRID};
use brightfield_protocol::graph::AssetId;

use crate::window::MeridianApp;

/// One way of looking at the node in focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeActivity {
    /// The node's rows, or the rows a chart reads.
    Grid,
    /// The chart that reads the node, or the chart itself.
    Chart,
    /// The model that relates the Protocol's sources.
    Model,
}

impl NodeActivity {
    /// The registry verb that switches to this activity.
    #[must_use]
    pub const fn verb(self) -> &'static str {
        match self {
            Self::Grid => SHOW_GRID,
            Self::Chart => SHOW_CHART,
            Self::Model => RELATE,
        }
    }
}

/// The node whose activities the window offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FocusedNode {
    /// A table or view, by its asset id.
    Table(AssetId),
    /// A chart, and the table it reads.
    Chart {
        /// The chart's own node.
        node: AssetId,
        /// The table it reads.
        table: AssetId,
    },
    /// The model step.
    Model,
}

impl FocusedNode {
    /// The table whose rows the node's Grid and Chart look at, when it has one.
    #[must_use]
    pub const fn table(&self) -> Option<&AssetId> {
        match self {
            Self::Table(table) | Self::Chart { table, .. } => Some(table),
            Self::Model => None,
        }
    }

    /// The activities a node of this kind has, in the order they are offered.
    #[must_use]
    pub const fn activities(&self) -> &'static [NodeActivity] {
        match self {
            Self::Table(_) | Self::Chart { .. } => &[NodeActivity::Grid, NodeActivity::Chart],
            Self::Model => &[NodeActivity::Model],
        }
    }
}

/// One activity offered for the node in focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityOffer {
    /// Which activity.
    pub activity: NodeActivity,
    /// The verb that switches to it.
    pub verb: &'static str,
    /// Why it cannot be taken now, when it cannot: the sentence its verb's
    /// declared needs say, read through the window's availability, so the
    /// switcher, the ribbon and the menus say one sentence.
    pub unavailable: Option<String>,
}

/// The activities the node in focus offers in `app`, each with the sentence it
/// says when it cannot be taken. Empty when no node is in focus.
#[must_use]
pub fn offers(app: &MeridianApp) -> Vec<ActivityOffer> {
    let Some(node) = app.node_in_focus() else {
        return Vec::new();
    };
    node.activities()
        .iter()
        .map(|&activity| ActivityOffer {
            activity,
            verb: activity.verb(),
            unavailable: match app.availability_of(activity.verb()) {
                Some(brightfield_keys::Availability::Missing(sentence)) => Some(sentence),
                _ => None,
            },
        })
        .collect()
}
