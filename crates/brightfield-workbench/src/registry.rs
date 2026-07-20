//! The only route from an item to a pane.
//!
//! Everything downstream reads this one list: the default arrangement is
//! built from it, the live item map is instantiated from it, and the item
//! vocabulary a saved layout is validated against is published from it. An
//! item that is not in a registry has no way into a tile tree at all — which
//! is what turns the empty-state gate from a posture into a gate. "The list
//! is hand-written, something could escape it" stops being true once the list
//! is the only door.

use std::collections::BTreeSet;

use crate::item::{Item, ItemId, ItemMap, PaneKey};
use crate::workspace::ViewKind;

/// Which edge a rail pane docks to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DockSide {
    /// The left rail.
    Left,
    /// The right rail.
    Right,
    /// The bottom rail.
    Bottom,
}

/// Where a pane sits in its view's default arrangement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slot {
    /// The view's main surface. Exactly one per view.
    Centre,
    /// A tab alongside [`Slot::Centre`] in the centre tab strip.
    CentreTab,
    /// A rail on one side, taking `share` of the root linear container.
    Rail {
        /// Which edge.
        side: DockSide,
        /// Its share of the root container, as `egui_tiles` means share.
        share: f32,
    },
}

/// How to make one kind of pane, and where it goes.
///
/// `make` is a plain function pointer with no arguments, which is possible
/// only because an [`Item`] holds no document handle. That is what collapses
/// what would otherwise be three lists — a construction table, a default
/// layout, and a test roster — into this one.
pub struct ItemSpec<D: ?Sized> {
    /// The item's id.
    pub id: ItemId,
    /// Where it sits by default.
    pub slot: Slot,
    /// The verb that shows and hides this pane.
    ///
    /// Required for every rail and every centre tab, for the same reason
    /// [`crate::HideAffordance`] is required on a status entry: a pane the
    /// user can close and cannot reopen, or cannot close at all, is a trap.
    /// `None` is legal only for [`Slot::Centre`], which is always present.
    pub toggle: Option<crate::subject::Verb>,
    /// A zero-argument constructor.
    pub make: fn() -> Box<dyn Item<D>>,
}

impl<D: ?Sized> std::fmt::Debug for ItemSpec<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ItemSpec")
            .field("id", &self.id)
            .field("slot", &self.slot)
            .field("toggle", &self.toggle)
            .finish_non_exhaustive()
    }
}

/// One view's item vocabulary.
pub struct ItemRegistry<D: ?Sized> {
    view: ViewKind,
    specs: Vec<ItemSpec<D>>,
}

impl<D: ?Sized> ItemRegistry<D> {
    /// Build a registry.
    ///
    /// # Panics
    ///
    /// If two specs share an id, or if the view does not have exactly one
    /// [`Slot::Centre`]. Both are structural mistakes that would otherwise
    /// surface as a pane that silently never appears, so they fail loudly at
    /// construction — which happens at boot and in every contract test.
    #[must_use]
    pub fn new(view: ViewKind, specs: Vec<ItemSpec<D>>) -> Self {
        let mut seen = BTreeSet::new();
        for spec in &specs {
            assert!(
                seen.insert(spec.id),
                "{view:?}: duplicate item id {}",
                spec.id
            );
        }
        let centres = specs
            .iter()
            .filter(|s| s.slot == Slot::Centre)
            .collect::<Vec<_>>();
        assert!(
            centres.len() == 1,
            "{view:?}: a view needs exactly one centre pane, found {}",
            centres.len()
        );
        Self { view, specs }
    }

    /// Which view this registry describes.
    #[must_use]
    pub const fn view(&self) -> ViewKind {
        self.view
    }

    /// Every spec, in declaration order.
    #[must_use]
    pub fn specs(&self) -> &[ItemSpec<D>] {
        &self.specs
    }

    /// Every item id in this view.
    #[must_use]
    pub fn ids(&self) -> Vec<ItemId> {
        self.specs.iter().map(|s| s.id).collect()
    }

    /// The pane key for an item in this view.
    #[must_use]
    pub const fn pane_key(&self, item: ItemId) -> PaneKey {
        PaneKey::new(self.view, item)
    }

    /// Construct every item in this view.
    #[must_use]
    pub fn instantiate(&self) -> ItemMap<D> {
        self.specs
            .iter()
            .map(|spec| (self.pane_key(spec.id), (spec.make)()))
            .collect()
    }

    /// The default arrangement for this view.
    ///
    /// Left rails, then the centre tab strip, then right rails, then bottom
    /// rails below the lot. Shares come from the specs; the centre takes
    /// whatever is left over.
    ///
    /// This is the *only* place a default layout is written, which is what
    /// makes the registry the single declaration of a view's shape. The old
    /// shell declared its rail widths twice — once as pixel constants used to
    /// size the window, once as tile shares used to lay it out — and the two
    /// had already drifted apart by the time anyone noticed.
    #[must_use]
    pub fn default_tree(&self) -> egui_tiles::Tree<PaneKey> {
        let mut tiles = egui_tiles::Tiles::default();

        let mut left = Vec::new();
        let mut right = Vec::new();
        let mut bottom = Vec::new();
        let mut centre_tabs = Vec::new();
        let mut centre = None;
        let mut shares: Vec<(egui_tiles::TileId, f32)> = Vec::new();

        for spec in &self.specs {
            let tile = tiles.insert_pane(self.pane_key(spec.id));
            match spec.slot {
                Slot::Centre => centre = Some(tile),
                Slot::CentreTab => centre_tabs.push(tile),
                Slot::Rail { side, share } => {
                    shares.push((tile, share));
                    match side {
                        DockSide::Left => left.push(tile),
                        DockSide::Right => right.push(tile),
                        DockSide::Bottom => bottom.push(tile),
                    }
                }
            }
        }

        let centre = centre.expect("ItemRegistry::new guarantees exactly one centre pane");
        let centre_tile = if centre_tabs.is_empty() {
            centre
        } else {
            let mut children = vec![centre];
            children.extend(centre_tabs);
            let tabs = tiles.insert_tab_tile(children);
            if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(t))) =
                tiles.get_mut(tabs)
            {
                // The centre pane is the default tab; a view whose first sight
                // is one of its secondary tabs is disorienting.
                t.set_active(centre);
            }
            tabs
        };

        // The centre's share is whatever the rails leave, so the rails' shares
        // stay the single declaration and no arithmetic has to be kept in sync.
        let rail_total: f32 = shares
            .iter()
            .filter(|(t, _)| left.contains(t) || right.contains(t))
            .map(|(_, s)| *s)
            .sum();
        let centre_share = (1.0 - rail_total).max(0.1);

        let mut row = left.clone();
        row.push(centre_tile);
        row.extend(right.iter().copied());
        let root_row = if row.len() == 1 {
            centre_tile
        } else {
            let r = tiles.insert_horizontal_tile(row);
            if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) =
                tiles.get_mut(r)
            {
                for (tile, share) in &shares {
                    lin.shares.set_share(*tile, *share);
                }
                lin.shares.set_share(centre_tile, centre_share);
            }
            r
        };

        let root = if bottom.is_empty() {
            root_row
        } else {
            let mut column = vec![root_row];
            column.extend(bottom.iter().copied());
            let c = tiles.insert_vertical_tile(column);
            if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) =
                tiles.get_mut(c)
            {
                let bottom_total: f32 = shares
                    .iter()
                    .filter(|(t, _)| bottom.contains(t))
                    .map(|(_, s)| *s)
                    .sum();
                lin.shares
                    .set_share(root_row, (1.0 - bottom_total).max(0.1));
                for (tile, share) in &shares {
                    if bottom.contains(tile) {
                        lin.shares.set_share(*tile, *share);
                    }
                }
            }
            c
        };

        egui_tiles::Tree::new(egui::Id::new(("brightfield-view", self.view)), root, tiles)
    }
}
