//! The route from an item to a pane.
//!
//! Everything downstream reads this one list: the default arrangement is
//! built from it, the live item map is instantiated from it, and the item
//! vocabulary a saved layout is validated against is published from it.
//!
//! It is the door every pane is *meant* to come through, not one it is forced
//! through — `egui_tiles::Tiles::insert_pane` is public and will take a
//! [`PaneKey`] from anyone. So [`audit`] covers
//! every item that went through the registry, which is every item anyone has a
//! reason to add; a pane inserted around it leaves the contract and the
//! audit's reach in the same move. Worth knowing before leaning on the gate
//! for a guarantee it cannot give.

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

    /// Publish this view's item ids into the process's layout vocabulary, so a
    /// saved layout naming one of these panes can be validated against a build
    /// that has it.
    ///
    /// This is what makes "the registry is the only declaration of a view's
    /// shape" true rather than aspirational. The protocol view used to hand
    /// [`ItemId::publish`] a hand-written `static [ItemId; 4]` sitting beside
    /// the registry, which is a second declaration by definition: adding a
    /// fifth pane to the registry and forgetting the array compiled, ran, and
    /// produced a pane whose saved layout would silently not load.
    ///
    /// # Why it leaks, and why that is bounded
    ///
    /// [`ItemId::publish`] takes `&'static [ItemId]` — see its docs for why —
    /// and the ids here are computed, so the slice has to be leaked. The early
    /// return holds that to at most one leak per view per process: a second
    /// call over an already-published vocabulary allocates the `Vec`, finds
    /// every id known, and drops it.
    pub fn publish_ids(&self) {
        let ids = self.ids();
        if ids.iter().all(|id| ItemId::known().contains(id)) {
            return;
        }
        ItemId::publish(Vec::leak(ids));
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

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// Check every item in `reg` against the contract, over a document with
/// nothing in it.
///
/// Each item is constructed, asked for its [`crate::Subject`] over
/// `empty_doc`, and required to: report the id its spec claims; declare an
/// empty state; write that empty state's prose to the house style; and declare
/// only verbs the keyboard registry has. Unless it is the centre pane, its
/// spec must also name the verb that shows and hides it — and that verb is
/// held to the same registration rule as the subject's own.
///
/// # Why this lives in the crate rather than in a test
///
/// [`crate::Item::empty_state`] is a *required* method, so a pane that never
/// decided what it shows when empty no longer compiles — but `None` over an
/// empty document still does, and that is the wrong *answer* to a question
/// the type system can only make sure was asked. Rendering a header and
/// silence was the single most common defect in the shell this crate
/// replaces, and the two worst instances of it were the two nobody had
/// written at all. The gate is the thing that catches the answer, so
/// shipping it as a test helper would mean every view re-implementing it,
/// which is the per-surface drift this crate exists to end. One function,
/// one call per view.
///
/// # Why it takes a document rather than requiring `Default`
///
/// A view's document may own something with no sensible `Default` — the
/// charts document owns a canvas host. Requiring `D: Default` would make the
/// gate inapplicable to exactly the view most worth gating, so the caller
/// supplies whatever "empty" means for its own document.
///
/// # Errors
///
/// The reason the first failing item failed, naming the item and the rule, so
/// a caller's assertion can pin *which* rule bit rather than merely that
/// something did.
pub fn audit<D: ?Sized>(reg: &ItemRegistry<D>, empty_doc: &D) -> Result<(), String> {
    for spec in reg.specs() {
        let id = spec.id;
        let item = (spec.make)();
        if item.item_id() != id {
            return Err(format!(
                "{id}: constructed item reports id {}",
                item.item_id()
            ));
        }

        let subject = item.subject(empty_doc);
        // `subject()` is provided glue over `describe()` + `empty_state()`,
        // and overriding it is how the two channels could diverge again. The
        // compare catches an override that answers differently from the
        // required method; an override that faithfully reproduces the glue is
        // indistinguishable and harmless.
        if subject.empty_state != item.empty_state(empty_doc) {
            return Err(format!(
                "{id}: subject() and empty_state() disagree — Item::subject is \
                 provided glue and must not be overridden"
            ));
        }
        let Some(empty) = &subject.empty_state else {
            return Err(format!("{id}: shows no empty state on an empty document"));
        };
        if empty.headline.is_empty() {
            return Err(format!("{id}: empty headline"));
        }
        if empty.headline.ends_with('.') {
            return Err(format!("{id}: headline takes no terminal period"));
        }
        if !empty
            .headline
            .chars()
            .next()
            .is_some_and(char::is_uppercase)
        {
            return Err(format!("{id}: headline is sentence case"));
        }
        if empty.body.is_empty() {
            return Err(format!("{id}: an empty state names what fills it"));
        }

        for verb in subject.declared_verbs() {
            if !verb.is_registered() {
                return Err(format!(
                    "{id}: declares unregistered verb {:?}",
                    verb.as_str()
                ));
            }
        }

        // The toggle verb is on the *spec*, not the subject, so
        // `declared_verbs` above never sees it. Checked here for the same
        // reason: a rail whose show/hide verb the keyboard registry does not
        // have is a rail that cannot be reopened once closed.
        match &spec.toggle {
            Some(verb) if !verb.is_registered() => {
                return Err(format!(
                    "{id}: declares unregistered verb {:?}",
                    verb.as_str()
                ));
            }
            None if spec.slot != Slot::Centre => {
                return Err(format!("{id}: a rail or tab with no toggle verb"));
            }
            _ => {}
        }
    }
    Ok(())
}
