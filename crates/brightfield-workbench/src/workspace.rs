//! One window's arrangement: its tile tree, and focus.
//!
//! # One tree, because a window is one arrangement
//!
//! A window holds **one** [`Tree`], whatever documents its panes read. The
//! window draws one declared arrangement ([`crate::arrangement`]) whose
//! regions are filled by panes of several documents at once — the spine in
//! the navigator rail, the chart on the canvas, the step list in the ledger
//! rail — in the same frame. A pane therefore belongs to the window, not
//! to a document, and its address ([`PaneKey`]) says so.
//!
//! What that buys: a second arrangement is a second `static` in
//! [`crate::arrangement`] read by the draw path that is already there, rather
//! than a document threaded through each region; and the layout file names
//! no concept the product does not have.
//!
//! The documents themselves have **not** merged and are not meant to.
//! `Tree::ui` takes `&mut dyn Behavior<PaneKey>`, so one tree hands its panes
//! to whichever [`crate::PaneChrome`] owns the document that pane reads; the
//! tree is doc-agnostic and always was.
//!
//! # Why focus is tracked here rather than inferred at draw time
//!
//! The chrome the shell draws around the window — the breadcrumb, the toolbar
//! row, the status rail — comes from *one* pane's [`Subject`](crate::Subject),
//! the focused one. Inferring which that is while drawing would mean deciding
//! it inside the tile tree's own borrow, one pane at a time, with no pane able
//! to see whether another already claimed it. So focus is state the workspace
//! owns, panes *request* changes to it ([`crate::Request::Focus`]), and the
//! shell applies those once the frame's panes have drawn.
//!
//! Focus is one record, for the same reason the tree is one tree: both
//! documents' panes are on screen in the same frame, so two live records
//! would mean two panes wearing the focus ring. It is neither persisted nor
//! part of this type's equality — see [`Workspace`].

use std::collections::HashSet;

use egui_tiles::{Container, Tile, TileId, Tree};
use serde::{Deserialize, Serialize};

use crate::item::PaneKey;

/// One window's arrangement: its tile tree, and which pane holds focus.
///
/// # Equality and serialisation deliberately disagree about focus
///
/// `focus` is `#[serde(skip)]` and excluded from the hand-written
/// [`PartialEq`], and both exclusions are the same decision: focus is
/// transient. Persisting it would restore a cursor the user has no memory of
/// leaving there; counting it as a difference would be worse, because
/// [`crate::persist::DirtyTracker`] is a plain `live != saved` compare, so
/// every focus move would rewrite the layout file for the rest of the
/// session. `egui_tiles` draws exactly this line for exactly this reason —
/// its `Tiles`' hand-written `PartialEq` skips the per-frame `rects`.
///
/// The tree *is* compared, which is what lets the dirty tracker be that plain
/// compare: `egui_tiles` 0.16 offers no layout-changed callback of any kind,
/// so equality against the last-written clone is the signal.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workspace {
    /// The window's dock tree.
    tree: Tree<PaneKey>,
    /// The focused pane. Transient — see the type docs.
    #[serde(skip)]
    focus: Option<PaneKey>,
}

impl PartialEq for Workspace {
    fn eq(&self, other: &Self) -> bool {
        let Self {
            tree,
            focus: _, // transient — see the type docs
        } = self;
        tree == &other.tree
    }
}

impl Workspace {
    /// Build a workspace around one tile tree.
    ///
    /// # Panics
    ///
    /// If `tree` has no root. A tree with no root is a window with nowhere to
    /// put a pane — a structural mistake worth failing at boot rather than
    /// discovering on the first frame.
    ///
    /// The neighbouring question — *is each pane the window draws actually in
    /// here* — is [`crate::persist::from_json`]'s and not this constructor's,
    /// because it is a question about a **file**: `Workspace` derives
    /// `Deserialize`, so a load does not run this. See
    /// [`Workspace::panes_missing_from`].
    #[must_use]
    pub fn new(tree: Tree<PaneKey>) -> Self {
        assert!(
            tree.root().is_some(),
            "a workspace needs a tile tree with a root; this one has no tiles to draw"
        );
        Self { tree, focus: None }
    }

    /// The window's tile tree.
    #[must_use]
    pub const fn tree(&self) -> &Tree<PaneKey> {
        &self.tree
    }

    /// The window's tile tree, mutably.
    pub const fn tree_mut(&mut self) -> &mut Tree<PaneKey> {
        &mut self.tree
    }

    /// The panes `other` holds that this workspace does not, counted rather
    /// than merely looked up.
    ///
    /// The repair check a load needs and a constructor cannot give: `Workspace`
    /// derives `Deserialize`, so [`Workspace::new`] does not run on a file, and a
    /// tree saved by a build with fewer panes parses perfectly well. A region
    /// whose declared pane has no tile is drawn empty, with no message saying
    /// why, so [`crate::persist::from_json`] asks this against the default
    /// arrangement and discards a file that comes up short.
    ///
    /// Answered against a donor workspace rather than against
    /// [`crate::ItemId::known`], which is a superset: an id may be published so
    /// a saved layout naming it still loads while the pane itself is behind a
    /// flag and absent from the default tree.
    ///
    /// # Why the match is consumed
    ///
    /// A tile in `other` that finds its counterpart here takes it out of the
    /// running, so a donor placing one [`PaneKey`] twice needs two tiles here
    /// and not one. Asking `contains` instead reads a second tile as covered
    /// by the first, and the file loads as restored with a pane silently
    /// absent — see
    /// `crates/brightfield-workbench/tests/cross_registry_ids.rs`, which is
    /// also where the arrangement that reaches that state comes from.
    ///
    /// Everywhere ids are unique this is the same answer as `contains`, by
    /// construction rather than by measurement: each key appears once in each
    /// list, so there is never a second match to consume. The two readings can
    /// only part where a donor repeats a key, and the one place a repeat can
    /// enter a default arrangement is
    /// [`window_tree`](crate::window_tree) — the sole caller of
    /// `egui_tiles::Tiles::insert_pane` in this workspace — being handed two
    /// placements with one id, which is a declaration mistake a registry
    /// cannot make on its own.
    ///
    /// The scan is linear per key rather than a merge over the two sorted
    /// lists. A window's panes are counted in single figures, and the
    /// straightforward reading is worth more here than the asymptote.
    #[must_use]
    pub fn panes_missing_from(&self, other: &Self) -> Vec<PaneKey> {
        let mut mine = self.panes();
        other
            .panes()
            .into_iter()
            .filter(|key| match mine.iter().position(|k| k == key) {
                Some(i) => {
                    mine.remove(i);
                    false
                }
                None => true,
            })
            .collect()
    }

    /// The focused pane — whose [`crate::Subject`] the shell draws window
    /// chrome from.
    #[must_use]
    pub const fn focus(&self) -> Option<PaneKey> {
        self.focus
    }

    /// Move focus to `key`.
    ///
    /// Returns whether the move was accepted. It is refused when the tree
    /// holds no such pane: a [`crate::Request::Focus`] is raised during a
    /// frame and applied after it, so by the time it is applied the pane may
    /// have been closed — and parking focus on a pane that is not there would
    /// leave the window chrome reading from a `Subject` nobody can see.
    pub fn set_focus(&mut self, key: PaneKey) -> bool {
        if !self.panes().contains(&key) {
            return false;
        }
        self.focus = Some(key);
        true
    }

    /// Drop the focus record, if any.
    pub const fn clear_focus(&mut self) {
        self.focus = None;
    }

    /// Every pane in the window, in pane-key order.
    ///
    /// Sorted rather than in tile order because `Tiles` is backed by a hash
    /// map, so tile order is not a stable thing for a caller to rely on.
    #[must_use]
    pub fn panes(&self) -> Vec<PaneKey> {
        let mut keys: Vec<PaneKey> = self
            .tree
            .tiles
            .tiles()
            .filter_map(|tile| match tile {
                Tile::Pane(key) => Some(*key),
                Tile::Container(_) => None,
            })
            .collect();
        keys.sort_unstable();
        keys
    }

    /// The tiles whose parent is a tab container.
    ///
    /// This is the header de-duplication rule [`crate::chrome::pane_frame`]
    /// takes as its `header` argument — a pane under a tab strip is already
    /// named by the strip — computed **before** `Tree::ui` borrows the tree
    /// rather than looked up during the draw, because an
    /// `egui_tiles::Behavior` cannot hold a reference to the tree it is being
    /// run against.
    ///
    /// Pre-computing it costs one frame of accuracy, and it is worth writing
    /// down rather than implying it is free: a drag that re-parents a pane
    /// into or out of a tab strip changes the tree *during* `Tree::ui`, after
    /// this set was taken, so for that one frame the moved pane's header band
    /// is wrong — missing if it just left a strip, doubled with the tab title
    /// if it just joined one. The next frame recomputes and it is right. A
    /// header that flickers for 16ms at the end of a drag is a smaller price
    /// than either handing the `Behavior` a second borrow of the tree or
    /// letting a tab title and a header band be answered from two places,
    /// which is the drift this whole file exists to end.
    #[must_use]
    pub fn tabbed_tiles(&self) -> HashSet<TileId> {
        tabbed_tiles_of(&self.tree)
    }

    /// The tile a pane occupies, if it is still there.
    ///
    /// See also the free [`tile_of`], which answers the same question for a
    /// bare tree.
    #[must_use]
    pub fn tile_of(&self, key: PaneKey) -> Option<TileId> {
        tile_of(&self.tree, key)
    }
}

// ---------------------------------------------------------------------------
// Tree queries, for a caller holding a bare tree
// ---------------------------------------------------------------------------

/// The tiles in `tree` whose parent is a tab container — the header
/// de-duplication rule [`crate::chrome::pane_frame`] takes as its `header`
/// argument.
///
/// [`Workspace::tabbed_tiles`] is this over the window's tree, and delegates
/// to it. Both exist because a caller can hold a tree without holding a
/// [`Workspace`]: [`crate::ItemRegistry::default_tree`] hands one back, and
/// the contract tests read the rule off it directly. A second hand-written
/// spelling of the rule in such a caller is exactly the drift the workbench
/// exists to end, so the rule stays here and callers borrow it.
#[must_use]
pub fn tabbed_tiles_of(tree: &Tree<PaneKey>) -> HashSet<TileId> {
    let mut tabbed = HashSet::new();
    for (_, tile) in tree.tiles.iter() {
        if let Tile::Container(Container::Tabs(tabs)) = tile {
            tabbed.extend(tabs.children.iter().copied());
        }
    }
    tabbed
}

/// The tile `key` occupies in `tree`, if it is still there.
///
/// The free twin of [`Workspace::tile_of`], for the same reason
/// [`tabbed_tiles_of`] is free.
#[must_use]
pub fn tile_of(tree: &Tree<PaneKey>, key: PaneKey) -> Option<TileId> {
    tree.tiles.iter().find_map(|(id, tile)| match tile {
        Tile::Pane(k) if *k == key => Some(*id),
        Tile::Pane(_) | Tile::Container(_) => None,
    })
}

/// The tab container in `tree` that holds `child`, if any.
///
/// A tab strip is the one container whose *active* child is state the window
/// may need to drive — the protocol's steps sheet is a tab, and its `S` verb
/// has to be able to activate it. Finding the strip by its child rather than by
/// a remembered [`TileId`] means a layout reload, or a drag that re-parents the
/// pane, cannot leave the shell holding an id that no longer names a strip.
#[must_use]
pub fn tabs_holding(tree: &Tree<PaneKey>, child: TileId) -> Option<TileId> {
    tree.tiles.iter().find_map(|(id, tile)| match tile {
        Tile::Container(Container::Tabs(tabs)) if tabs.children.contains(&child) => Some(*id),
        Tile::Container(_) | Tile::Pane(_) => None,
    })
}
