//! One window's arrangement: a tree per view, and focus.
//!
//! # Why a tree per view rather than one tree with more tabs
//!
//! A view is a whole arrangement of panes with its own document, not a tab
//! within one: the charts view and the protocol view have different panes,
//! different key contexts and different state behind them. Each therefore
//! gets its own `egui_tiles::Tree`, and that is what lets them coexist in a
//! single window at all — every `Tree` owns its own `Tiles` arena and numbers
//! tiles from the same place, so two views sharing one arena would collide.
//!
//! The consequence worth stating, because it is the property the shell is
//! being rebuilt to get: switching views draws a different tree and touches
//! nothing else. The view being left keeps its splits, its shares, its active
//! tab and its focused pane exactly as the user left them, because nothing in
//! a switch reaches into it.
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
//! Focus is **per view**, for the same reason the trees are: returning to a
//! view you left should not silently move your cursor. It is also neither
//! persisted nor part of this type's equality — see [`Workspace`].

use std::collections::{BTreeMap, HashSet};

use egui_tiles::{Container, Tile, TileId, Tree};
use serde::{Deserialize, Serialize};

use crate::item::PaneKey;

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

/// One window's arrangement: which view is active, a tile tree per view, and
/// which pane holds focus in each.
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
/// Everything else here *is* compared, which is what lets the dirty tracker
/// be that plain compare: `egui_tiles` 0.16 offers no layout-changed callback
/// of any kind, so equality against the last-written clone is the signal.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workspace {
    /// The view currently drawn.
    active: ViewKind,
    /// One dock tree per view. A `BTreeMap` so the JSON key order is stable
    /// and the file diffs cleanly.
    trees: BTreeMap<ViewKind, Tree<PaneKey>>,
    /// The focused pane in each view. Transient — see the type docs.
    #[serde(skip)]
    focus: BTreeMap<ViewKind, PaneKey>,
}

impl PartialEq for Workspace {
    fn eq(&self, other: &Self) -> bool {
        let Self {
            active,
            trees,
            focus: _, // transient — see the type docs
        } = self;
        active == &other.active && trees == &other.trees
    }
}

impl Workspace {
    /// Build a workspace from one tree per view.
    ///
    /// # Panics
    ///
    /// If any [`ViewKind`] has no tree. A view with no tree is a switcher
    /// entry that opens a blank window — a structural mistake worth failing
    /// at boot rather than discovering by clicking on it.
    #[must_use]
    pub fn new(trees: BTreeMap<ViewKind, Tree<PaneKey>>) -> Self {
        for view in ViewKind::ALL {
            assert!(
                trees.contains_key(&view),
                "{view:?} has no tile tree; every view needs one"
            );
        }
        Self {
            active: ViewKind::ALL[0],
            trees,
            focus: BTreeMap::new(),
        }
    }

    /// The view currently drawn.
    #[must_use]
    pub const fn active(&self) -> ViewKind {
        self.active
    }

    /// Switch views.
    ///
    /// Nothing else happens. The view being left is not torn down, not
    /// rebuilt and not touched: it simply stops being the one handed to
    /// `Tree::ui`.
    pub fn set_active(&mut self, view: ViewKind) {
        self.active = view;
    }

    /// The views this workspace holds no tree for.
    ///
    /// Empty for a workspace built through [`Workspace::new`], which asserts
    /// it. **Not** empty in general: `trees` is a map and `Workspace` derives
    /// `Deserialize`, so a load never passes through `new`, and a file whose
    /// map is short one key parses perfectly well. That is why
    /// [`crate::persist::from_json`] checks this and repairs it — see
    /// [`Workspace::fill_missing_views_from`].
    #[must_use]
    pub fn missing_views(&self) -> Vec<ViewKind> {
        ViewKind::ALL
            .into_iter()
            .filter(|view| !self.trees.contains_key(view))
            .collect()
    }

    /// Copy a tree from `donor` for every view this workspace lacks, and
    /// return the views that were filled.
    ///
    /// The repair path for a layout file written before a [`ViewKind`] was
    /// added. Discarding the whole file is the other option, and it would
    /// throw away the arrangement of every view that *is* in it for the sake
    /// of one that could not have been: a `BTreeMap` simply lacks the new
    /// key, and no plausible reading of
    /// [`crate::persist::LAYOUT_VERSION`]'s own rule — bump it "when the
    /// persisted shape changes incompatibly" — makes adding an enum variant
    /// a bump.
    ///
    /// Views already present are left exactly as they were.
    pub fn fill_missing_views_from(&mut self, donor: &Self) -> Vec<ViewKind> {
        let mut filled = Vec::new();
        for view in self.missing_views() {
            if let Some(tree) = donor.trees.get(&view) {
                self.trees.insert(view, tree.clone());
                filled.push(view);
            }
        }
        filled
    }

    /// One view's tile tree.
    ///
    /// # Panics
    ///
    /// If `view` has no tree. Both routes to a `Workspace` that the shell
    /// uses guarantee there is one: [`Workspace::new`] asserts completeness,
    /// and [`crate::persist::from_json`] fills any view the file lacked
    /// before handing the workspace back. Neither guarantee reaches a
    /// `Workspace` deserialised directly with `serde`, bypassing `persist` —
    /// [`Workspace::missing_views`] is how such a caller checks.
    #[must_use]
    pub fn tree(&self, view: ViewKind) -> &Tree<PaneKey> {
        self.trees.get(&view).expect("every view has a tree")
    }

    /// One view's tile tree, mutably.
    ///
    /// # Panics
    ///
    /// As [`Workspace::tree`].
    pub fn tree_mut(&mut self, view: ViewKind) -> &mut Tree<PaneKey> {
        self.trees.get_mut(&view).expect("every view has a tree")
    }

    /// The active view's tile tree.
    #[must_use]
    pub fn active_tree(&self) -> &Tree<PaneKey> {
        self.tree(self.active)
    }

    /// The active view's tile tree, mutably — the one handed to `Tree::ui`.
    pub fn active_tree_mut(&mut self) -> &mut Tree<PaneKey> {
        let active = self.active;
        self.tree_mut(active)
    }

    /// The focused pane in `view`, if it has one.
    #[must_use]
    pub fn focus_in(&self, view: ViewKind) -> Option<PaneKey> {
        self.focus.get(&view).copied()
    }

    /// The focused pane in the active view — whose [`crate::Subject`] the
    /// shell draws window chrome from.
    #[must_use]
    pub fn focus(&self) -> Option<PaneKey> {
        self.focus_in(self.active)
    }

    /// Move focus to `key`, in `key`'s own view.
    ///
    /// Returns whether the move was accepted. It is refused when that view
    /// holds no such pane: a [`crate::Request::Focus`] is raised during a
    /// frame and applied after it, so by the time it is applied the pane may
    /// have been closed — and parking focus on a pane that is not there would
    /// leave the window chrome reading from a `Subject` nobody can see.
    ///
    /// Focusing a pane in a *non-active* view is legal and leaves the active
    /// view alone. That is how a view remembers where you were.
    pub fn set_focus(&mut self, key: PaneKey) -> bool {
        if !self.panes(key.view).contains(&key) {
            return false;
        }
        self.focus.insert(key.view, key);
        true
    }

    /// Drop the focus record for `view`, if any.
    pub fn clear_focus(&mut self, view: ViewKind) {
        self.focus.remove(&view);
    }

    /// Every pane in `view`, in pane-key order.
    ///
    /// Sorted rather than in tile order because `Tiles` is backed by a hash
    /// map, so tile order is not a stable thing for a caller to rely on.
    #[must_use]
    pub fn panes(&self, view: ViewKind) -> Vec<PaneKey> {
        let mut keys: Vec<PaneKey> = self
            .tree(view)
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

    /// The tiles in `view` whose parent is a tab container.
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
    pub fn tabbed_tiles(&self, view: ViewKind) -> HashSet<TileId> {
        tabbed_tiles_of(self.tree(view))
    }

    /// The tile a pane occupies in its own view, if it is still there.
    ///
    /// See also the free [`tile_of`], which answers the same question for a
    /// bare tree.
    #[must_use]
    pub fn tile_of(&self, key: PaneKey) -> Option<TileId> {
        self.tree(key.view)
            .tiles
            .iter()
            .find_map(|(id, tile)| match tile {
                Tile::Pane(k) if *k == key => Some(*id),
                _ => None,
            })
    }
}

// ---------------------------------------------------------------------------
// Tree queries, for a shell that has no workspace yet
// ---------------------------------------------------------------------------

/// The tiles in `tree` whose parent is a tab container — the header
/// de-duplication rule [`crate::chrome::pane_frame`] takes as its `header`
/// argument.
///
/// [`Workspace::tabbed_tiles`] is this over one view's tree, and delegates to
/// it. Both exist because a view can be hosted before the window has a
/// [`Workspace`]: the protocol panel keeps its own window while the one-app
/// migration is pending, and it needs this rule now. A second hand-written
/// spelling of it in that shell is exactly the drift the workbench exists to
/// end, so the rule stays here and the shell borrows it.
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
/// A tab strip is the one container whose *active* child is state a view may
/// need to drive — the protocol panel's steps sheet is a tab, and its `S` verb
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
