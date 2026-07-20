//! Which top-level view a pane belongs to.
//!
//! A view is a whole arrangement of panes with its own tile tree, not a tab
//! within one: the charts view and the protocol view have different panes,
//! different key contexts and different documents. Each gets its own
//! `egui_tiles::Tree`, which is what lets them coexist in one window at all —
//! every `Tree` owns its own tile arena, so two views' tile ids cannot
//! collide even though both start numbering from the same place.
//!
//! The `Workspace` that holds those trees, tracks focus per view and persists
//! the arrangement is the next step and is deliberately not here yet. This
//! module carries only the identity, because [`crate::PaneKey`] needs it.

use serde::{Deserialize, Serialize};

/// A top-level view: one arrangement of panes over one document.
///
/// Serialised into the layout file as part of every [`crate::PaneKey`], so
/// the variant names are a compatibility surface. Renaming one invalidates
/// saved layouts — which is survivable, because a layout that fails to
/// deserialise is discarded in favour of the default arrangement rather than
/// migrated, but it is not free.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ViewKind {
    /// The chart workbench: a Vello canvas over a composed spec.
    Charts,
    /// The protocol panel: the asset graph, its outline, steps and inspector.
    Protocol,
}

impl ViewKind {
    /// Every view, in the order the view switcher offers them.
    pub const ALL: [ViewKind; 2] = [ViewKind::Charts, ViewKind::Protocol];

    /// The view's name, as the switcher and the window title spell it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            ViewKind::Charts => "Charts",
            ViewKind::Protocol => "Protocol",
        }
    }
}
