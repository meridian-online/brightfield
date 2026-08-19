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
//!
//! # Two registries, one pattern
//!
//! [`ItemSpec`] / [`ItemRegistry`] answer *which panes a document brings to
//! the window*. Below them sits a second instance of the same shape —
//! [`ChartKind`] / [`ChartKindRegistry`] — answering *which chart a module
//! draws*. They are not the same registry under two names: an item is a pane
//! and a chart kind is what one pane can be filled with.
//!
//! There is a registry per **document**, not per window: the shell ships two
//! unrelated document types and a pane reads exactly one of them. The window
//! itself holds one tile tree over the union of their panes — see
//! [`window_tree`], which is what [`crate::Workspace`] is built around.
//!
//! The reason the second exists is that a caller which has to *choose* a chart
//! for a column can only do so cheaply if a chart kind is data. So a kind is
//! an [`Icon`], a description, a list of [`FieldSlot`]s saying what it takes,
//! a list of [`ModuleControl`]s saying what it hangs on itself, and a function
//! returning a spec — with no component of its own. [`ChartModule`] is the one
//! component all of them draw through, and [`audit_chart_kinds`] is the
//! conformance gate, the way [`audit`] is for items.
//!
//! [`ChartModule`]: crate::item::ChartModule

use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::item::{Item, ItemId, ItemMap, PaneKey};
use crate::subject::{Icon, ToolbarEntry, Verb};

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

/// Where a pane sits in the default arrangement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slot {
    /// This document's main surface. Exactly one per registry; a window built
    /// from several registries has one per document, and they share the
    /// centre tab strip.
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

/// One document's item vocabulary.
pub struct ItemRegistry<D: ?Sized> {
    specs: Vec<ItemSpec<D>>,
}

impl<D: ?Sized> ItemRegistry<D> {
    /// Build a registry.
    ///
    /// # Panics
    ///
    /// If two specs share an id, or if this registry does not have exactly one
    /// [`Slot::Centre`]. Both are structural mistakes that would otherwise
    /// surface as a pane that silently never appears, so they fail loudly at
    /// construction — which happens at boot and in every contract test. The
    /// ids in the message are what name the offending registry; there is no
    /// registry name to print, and a duplicate id identifies it anyway.
    #[must_use]
    pub fn new(specs: Vec<ItemSpec<D>>) -> Self {
        let mut seen = BTreeSet::new();
        for spec in &specs {
            assert!(seen.insert(spec.id), "duplicate item id {}", spec.id);
        }
        let centres = specs
            .iter()
            .filter(|s| s.slot == Slot::Centre)
            .collect::<Vec<_>>();
        assert!(
            centres.len() == 1,
            "a registry needs exactly one centre pane, found {} ({:?})",
            centres.len(),
            self::ids_of(&specs),
        );
        Self { specs }
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

    /// Publish this registry's item ids into the process's layout vocabulary,
    /// so a saved layout naming one of these panes can be validated against a
    /// build that has it.
    ///
    /// This is what makes "the registry is the only declaration of a
    /// document's panes" true rather than aspirational. The protocol side used
    /// to hand
    /// [`ItemId::publish`] a hand-written `static [ItemId; 4]` sitting beside
    /// the registry, which is a second declaration by definition: adding a
    /// fifth pane to the registry and forgetting the array compiled, ran, and
    /// produced a pane whose saved layout would silently not load.
    ///
    /// # Why it leaks, and why that is bounded
    ///
    /// [`ItemId::publish`] takes `&'static [ItemId]` — see its docs for why —
    /// and the ids here are computed, so the slice has to be leaked. The early
    /// return holds that to at most one leak per registry per process: a
    /// second call over an already-published vocabulary allocates the `Vec`,
    /// finds every id known, and drops it.
    pub fn publish_ids(&self) {
        let ids = self.ids();
        if ids.iter().all(|id| ItemId::known().contains(id)) {
            return;
        }
        ItemId::publish(Vec::leak(ids));
    }

    /// The pane key for an item.
    ///
    /// A pane's address is its item id and nothing else — see [`PaneKey`] —
    /// so this is a rename rather than a lookup. Kept as a method because the
    /// call sites read as *this registry's pane for this item*, and because a
    /// build that gave a key a second field would want them all to go through
    /// one place again.
    #[must_use]
    pub const fn pane_key(&self, item: ItemId) -> PaneKey {
        PaneKey::new(item)
    }

    /// Construct every item this registry declares.
    #[must_use]
    pub fn instantiate(&self) -> ItemMap<D> {
        self.specs
            .iter()
            .map(|spec| (self.pane_key(spec.id), (spec.make)()))
            .collect()
    }

    /// Where each of this registry's panes sits, free of the document they
    /// read — what [`window_tree`] takes so a window can lay out the panes of
    /// several documents in one tree.
    #[must_use]
    pub fn placements(&self) -> Vec<(ItemId, Slot)> {
        self.specs.iter().map(|spec| (spec.id, spec.slot)).collect()
    }

    /// The default arrangement for this registry's own panes.
    ///
    /// [`window_tree`] over [`Self::placements`] — the one-document case of
    /// the window's tree, which is what the contract tests read a registry's
    /// declared shape off. The live window builds a single tree over every
    /// registry's placements instead; see [`window_tree`].
    #[must_use]
    pub fn default_tree(&self) -> egui_tiles::Tree<PaneKey> {
        window_tree(&self.placements())
    }
}

/// Every id in `specs`, for a panic message that has to name the registry it
/// is complaining about and has no other name to use.
fn ids_of<D: ?Sized>(specs: &[ItemSpec<D>]) -> Vec<ItemId> {
    specs.iter().map(|spec| spec.id).collect()
}

/// The egui id the window's tile tree is built under.
///
/// One window, one tree, one id. Two trees drawn in one frame under one id
/// would collide in `egui::Memory`, which is why the id was keyed by something
/// when there was more than one tree; there is not.
const TREE_ID: &str = "brightfield-window";

/// The window's tile tree, over every pane placed in it.
///
/// Left rails, then the centre tab strip, then right rails, then bottom rails
/// below the lot. Shares come from the placements; the centre takes whatever
/// is left over.
///
/// This is the *only* place a default layout is written, which is what makes
/// the registries the single declaration of the window's panes. The old shell
/// declared its rail widths twice — once as pixel constants used to size the
/// window, once as tile shares used to lay it out — and the two had already
/// drifted apart by the time anyone noticed.
///
/// # More than one [`Slot::Centre`] is expected here, and is not in a registry
///
/// [`ItemRegistry::new`] asserts exactly one centre, because a document with
/// two main surfaces is a declaration mistake. A *window* built from several
/// documents has one centre each, and they belong in the same strip: the
/// canvas region shows one occupant at a time, and which one is the window's
/// question rather than a document's. The first centre in placement order is
/// the strip's active tab, for the reason the single-centre case had one — a
/// window whose first sight is a secondary tab is disorienting.
///
/// # Panics
///
/// If `placements` declares no [`Slot::Centre`] at all. A window with no main
/// surface has nowhere to put the thing it exists to show.
#[must_use]
pub fn window_tree(placements: &[(ItemId, Slot)]) -> egui_tiles::Tree<PaneKey> {
    let mut tiles = egui_tiles::Tiles::default();

    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut bottom = Vec::new();
    let mut centres = Vec::new();
    let mut centre_tabs = Vec::new();
    let mut shares: Vec<(egui_tiles::TileId, f32)> = Vec::new();

    for (id, slot) in placements {
        let tile = tiles.insert_pane(PaneKey::new(*id));
        match slot {
            Slot::Centre => centres.push(tile),
            Slot::CentreTab => centre_tabs.push(tile),
            Slot::Rail { side, share } => {
                shares.push((tile, *share));
                match side {
                    DockSide::Left => left.push(tile),
                    DockSide::Right => right.push(tile),
                    DockSide::Bottom => bottom.push(tile),
                }
            }
        }
    }

    let first_centre = *centres
        .first()
        .expect("a window needs at least one centre pane");
    let centre_tile = if centres.len() == 1 && centre_tabs.is_empty() {
        first_centre
    } else {
        let mut children = centres.clone();
        children.extend(centre_tabs);
        let tabs = tiles.insert_tab_tile(children);
        if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(t))) =
            tiles.get_mut(tabs)
        {
            t.set_active(first_centre);
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

    egui_tiles::Tree::new(egui::Id::new(TREE_ID), root, tiles)
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
/// shipping it as a test helper would mean every caller re-implementing it,
/// which is the per-surface drift this crate exists to end. One function,
/// one call per registry.
///
/// # Why it takes a document rather than requiring `Default`
///
/// A document may own something with no sensible `Default` — the chart
/// document owns a canvas host. Requiring `D: Default` would make the gate
/// inapplicable to exactly the document most worth gating, so the caller
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

// ---------------------------------------------------------------------------
// A chart kind, as data
// ---------------------------------------------------------------------------

/// What a column holds, as far as choosing a chart for it goes.
///
/// Coarser than any engine type on purpose: a chart kind cares whether a
/// column is a number, a name, a moment or a flag, not whether the number
/// arrived as a 32-bit or a 64-bit float. Narrowing an engine type down to one
/// of these is the caller's job, and it is the only place the two vocabularies
/// meet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FieldType {
    /// A measured number.
    Quantitative,
    /// A name, a label, a member of a set.
    Categorical,
    /// A date, a time or an instant.
    Temporal,
    /// True or false.
    Boolean,
}

/// One column offered to a chart kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// The column's name, as its source spells it.
    pub name: String,
    /// What it holds.
    pub ty: FieldType,
}

impl Field {
    /// A field.
    #[must_use]
    pub fn new(name: impl Into<String>, ty: FieldType) -> Self {
        Self {
            name: name.into(),
            ty,
        }
    }
}

/// A slot a chart kind needs filled, and what may fill it.
///
/// This is the type-constraint half of "a chart kind is data". A kind does not
/// validate its own inputs inside its builder — it *declares* what it takes,
/// and [`ChartKind::bind`] does the checking once, for every kind. That plus
/// the id and slot checks in [`ChartKind::spec`] is what makes the builder a
/// plain function rather than a component with an error path; see [`Bound`] for
/// how the two are handed to it as one argument, and for where that stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldSlot {
    /// What the field means to this kind: `"x"`, `"y"`, `"colour"`. Unique
    /// within a kind — [`audit_chart_kinds`] rejects a repeat, because
    /// [`FieldBinding::field`] answers with the first slot under the role and
    /// leaves the second unreachable.
    pub role: &'static str,
    /// The field types this slot will take. An empty list takes nothing, which
    /// the audit rejects.
    pub accepts: &'static [FieldType],
    /// Whether the kind can be drawn without it.
    pub required: bool,
}

impl FieldSlot {
    /// A slot the kind cannot be drawn without.
    #[must_use]
    pub const fn required(role: &'static str, accepts: &'static [FieldType]) -> Self {
        Self {
            role,
            accepts,
            required: true,
        }
    }

    /// A slot the kind draws without when nothing fits it.
    #[must_use]
    pub const fn optional(role: &'static str, accepts: &'static [FieldType]) -> Self {
        Self {
            role,
            accepts,
            required: false,
        }
    }

    /// Whether this slot will take `field`.
    #[must_use]
    pub fn takes(&self, field: &Field) -> bool {
        self.accepts.contains(&field.ty)
    }
}

/// Which column filled which of a chart kind's slots.
///
/// **[`ChartKind::bind`] is the only thing that makes one**, and that is half
/// of what lets a builder be a plain function: a binding which exists has
/// already satisfied the slots of *the kind that produced it*. Only half,
/// because that kind need not be the kind about to build from it — two kinds'
/// bindings are the same Rust type. [`ChartKind::spec`] closes that gap on the
/// route through it: it checks the id, re-checks the columns against its own
/// slots, and hands the builder a [`Bound`], which nothing else makes.
/// [`Bound`] states where that stops.
///
/// The alternative — handing the builder a bag of columns and asking it to
/// check — puts a copy of the constraint in every kind, which is the per-kind
/// drift a registry exists to end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldBinding {
    kind: ChartKindId,
    bound: Vec<(&'static str, Field)>,
}

impl FieldBinding {
    /// Which kind's slots this binding satisfies.
    #[must_use]
    pub const fn kind(&self) -> ChartKindId {
        self.kind
    }

    /// The field bound to `role`, if one was.
    #[must_use]
    pub fn field(&self, role: &str) -> Option<&Field> {
        self.bound.iter().find(|(r, _)| *r == role).map(|(_, f)| f)
    }

    /// The name of the column bound to `role`, if one was.
    #[must_use]
    pub fn name(&self, role: &str) -> Option<&str> {
        self.field(role).map(|f| f.name.as_str())
    }

    /// Every filled role, in slot order.
    pub fn roles(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.bound.iter().map(|(r, _)| *r)
    }
}

/// A [`FieldBinding`] that has been checked against the kind whose
/// [`ChartKind::spec`] made it.
///
/// `spec` is the only thing that makes one, and a kind's builder takes one, so
/// no binding reaches a builder without some kind's id check and that same
/// kind's slot check having run over it. [`ChartKind::build`] is a `pub` field
/// and a caller can read it; without a `Bound` there is nothing to call it
/// with.
///
/// It derefs to the binding, so a builder written `|binding, _|
/// binding.name("x")` reads exactly as it would over a [`FieldBinding`]:
/// closing the direct route costs a kind no change of shape.
///
/// # The route this closes
///
/// The checked route builds a spec:
///
/// ```
/// use brightfield_workbench::registry::{
///     Bound, ChartKind, ChartKindId, Field, FieldSlot, FieldType, ModuleOptions,
/// };
/// use brightfield_workbench::Icon;
///
/// const BAR_SLOTS: &[FieldSlot] = &[
///     FieldSlot::required("x", &[FieldType::Categorical]),
///     FieldSlot::required("y", &[FieldType::Quantitative]),
/// ];
/// let bars = ChartKind::<String> {
///     id: ChartKindId::new("bars"),
///     icon: Icon("chart-bar"),
///     description: "Ranks a category by a measure",
///     slots: BAR_SLOTS,
///     controls: Vec::new,
///     build: |binding: &Bound<'_>, _| {
///         format!("bar {:?} {:?}", binding.name("x"), binding.name("y"))
///     },
/// };
/// let columns = [
///     Field::new("sector", FieldType::Categorical),
///     Field::new("revenue", FieldType::Quantitative),
/// ];
/// let binding = bars.bind(&columns).unwrap();
/// assert_eq!(
///     bars.spec(&binding, &ModuleOptions::default()).unwrap(),
///     r#"bar Some("sector") Some("revenue")"#
/// );
/// ```
///
/// The same setup, and the unchecked route does not compile — the builder
/// wants a `Bound`, and nothing outside this crate can make one. The block
/// above is that setup, visible and passing, so this one fails for the line
/// it is here to refuse rather than for a typo above it:
///
/// ```compile_fail
/// # use brightfield_workbench::registry::{
/// #     Bound, ChartKind, ChartKindId, Field, FieldSlot, FieldType, ModuleOptions,
/// # };
/// # use brightfield_workbench::Icon;
/// # const BAR_SLOTS: &[FieldSlot] = &[
/// #     FieldSlot::required("x", &[FieldType::Categorical]),
/// #     FieldSlot::required("y", &[FieldType::Quantitative]),
/// # ];
/// # let bars = ChartKind::<String> {
/// #     id: ChartKindId::new("bars"),
/// #     icon: Icon("chart-bar"),
/// #     description: "Ranks a category by a measure",
/// #     slots: BAR_SLOTS,
/// #     controls: Vec::new,
/// #     build: |binding: &Bound<'_>, _| {
/// #         format!("bar {:?} {:?}", binding.name("x"), binding.name("y"))
/// #     },
/// # };
/// # let columns = [
/// #     Field::new("sector", FieldType::Categorical),
/// #     Field::new("revenue", FieldType::Quantitative),
/// # ];
/// # let binding = bars.bind(&columns).unwrap();
/// let _ = (bars.build)(&binding, &ModuleOptions::default());
/// ```
///
/// # The route it does not close
///
/// The kind that made a `Bound` need not be the kind whose builder reads it. A
/// `build` closure may call another kind's `build`, and what it forwards was
/// checked against *its own* id and slots, not the receiver's — so a builder
/// can be reached with a column no slot it declares would take. Neither the
/// type system nor [`audit_chart_kinds`] tells a forwarded call from a direct
/// one: a `fn` pointer carries no kind identity, and a closure that forwards is
/// a different pointer from the one it calls.
///
/// So the guarantee to write a builder against is *this kind's declaration*,
/// and delegating to another kind's builder steps outside it.
/// `crates/brightfield-workbench/tests/chart_kind_bound.rs` holds both sides:
/// the id and slot checks biting from outside the crate — including on a kind
/// that shares this one's id — and the forwarded call that neither covers.
#[derive(Debug)]
pub struct Bound<'a>(&'a FieldBinding);

impl std::ops::Deref for Bound<'_> {
    type Target = FieldBinding;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

/// A control a chart kind hangs on its own module.
///
/// A log/linear toggle is the case this exists for: it changes *this module's*
/// spec and nothing else on the surface around it, so it belongs to the module
/// rather than to the canvas. It is declared as data beside the icon and the
/// slots, drawn inside the module's own rect by
/// [`module_frame`](crate::chrome::module_frame), and performed by the module
/// — [`ChartModule`](crate::item::ChartModule) answers
/// [`Handled::Yes`](crate::Handled) for the verb, so it never reaches the
/// workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleControl {
    /// Stable id, and the key this control's state is held under in
    /// [`ModuleOptions`].
    pub id: &'static str,
    /// The visible label.
    pub label: String,
    /// The registered command this control runs.
    ///
    /// Checked by [`audit_chart_kinds`] for the reason [`ItemSpec::toggle`] is
    /// checked by [`audit`]: a control bound to a verb the keyboard registry
    /// does not have is a control with no keyboard route to it.
    pub verb: Verb,
    /// Whether the option starts on.
    pub on_by_default: bool,
    /// Hover text. The keystroke is appended by the chrome from
    /// [`Verb::keys`]; do not spell it here.
    pub tooltip: Option<String>,
}

impl ModuleControl {
    /// A control that starts off.
    #[must_use]
    pub fn new(id: &'static str, label: impl Into<String>, verb: Verb) -> Self {
        Self {
            id,
            label: label.into(),
            verb,
            on_by_default: false,
            tooltip: None,
        }
    }

    /// Start this control on.
    #[must_use]
    pub const fn on_by_default(mut self, on: bool) -> Self {
        self.on_by_default = on;
        self
    }

    /// Give this control hover text.
    #[must_use]
    pub fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// This control as the toolbar entry the chrome draws.
    ///
    /// A [`ToolbarEntry`] rather than a second control vocabulary, so a
    /// module's own controls are drawn by the same
    /// [`toolbar_button`](crate::chrome::toolbar_button) as a pane's — a
    /// second spelling of the button is how a surface stops feeling like one
    /// thing.
    #[must_use]
    pub fn entry(&self, on: bool) -> ToolbarEntry {
        let entry = ToolbarEntry::button(self.id, self.label.clone(), self.verb).toggle(on);
        match &self.tooltip {
            Some(tip) => ToolbarEntry {
                tooltip: Some(tip.clone()),
                ..entry
            },
            None => entry,
        }
    }
}

/// The state of one module's own controls.
///
/// Keyed by [`ModuleControl::id`], so a kind can read its own switches out of
/// the spec builder without the module and the kind sharing a struct
/// definition — which is what would make adding a kind a type change again.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleOptions {
    on: BTreeSet<&'static str>,
}

impl ModuleOptions {
    /// Whether the control with this id is on. An id no control declares is
    /// off, which is the answer that keeps a renamed control from silently
    /// reading as on.
    #[must_use]
    pub fn is_on(&self, id: &str) -> bool {
        self.on.contains(id)
    }

    /// Set a control's state.
    pub fn set(&mut self, id: &'static str, on: bool) {
        if on {
            self.on.insert(id);
        } else {
            self.on.remove(id);
        }
    }

    /// Flip a control's state and report what it became.
    pub fn toggle(&mut self, id: &'static str) -> bool {
        let now = !self.is_on(id);
        self.set(id, now);
        now
    }
}

/// A stable name for a kind of chart.
///
/// The string is what a module holds to name the kind it draws and what
/// [`ChartKindRegistry::find`] resolves against, so renaming a kind renames it
/// for every module pointing at one. It is not a serialisation format: it
/// derives no serde, and [`SavedLayout`](crate::persist::SavedLayout) carries
/// [`PaneKey`]s rather than modules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChartKindId(&'static str);

impl ChartKindId {
    /// A chart-kind id.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// The id's string form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for ChartKindId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// One kind of chart, entirely as data.
///
/// `S` is the view's **spec** type — whatever a module hands its host to be
/// drawn. Generic for the same reason [`ItemSpec`] is generic over its
/// document: this crate carries no engine, no renderer and no spec language,
/// and a chart-kind registry that named a concrete spec type would drag one
/// in. The view that owns the registry names `S` once.
///
/// There is no component here and none is written per kind. Adding a kind is a
/// value of this struct in a list; [`ChartModule`](crate::item::ChartModule)
/// draws it.
pub struct ChartKind<S> {
    /// The kind's id.
    pub id: ChartKindId,
    /// The icon a picker shows for it.
    pub icon: Icon,
    /// One line saying what it is, for a picker and for a generator's
    /// reasoning. House style, and [`audit_chart_kinds`] holds it there:
    /// sentence case, no terminal period — the same rule an
    /// [`EmptyState`](crate::EmptyState) headline is held to.
    pub description: &'static str,
    /// What it takes, and of what type.
    pub slots: &'static [FieldSlot],
    /// The controls it hangs on its own module.
    ///
    /// A function rather than a `&'static [ModuleControl]` because
    /// [`ModuleControl::label`] is a `String`, which forecloses a `const`
    /// array on its own. The [`Verb`] each control carries is checked against
    /// the keyboard registry when it is made, so the list is built at run time
    /// on that count too.
    ///
    /// Called on every read rather than cached, at four sites: once per module
    /// at construction by [`ChartKind::options`], **every frame** a module
    /// draws by [`ChartKind::toolbar`], on **every verb dispatch** by
    /// [`ChartModule`](crate::item::ChartModule), and once per kind by
    /// [`audit_chart_kinds`]. Return a list of literals; put nothing here you
    /// would not pay for per frame.
    pub controls: fn() -> Vec<ModuleControl>,
    /// The spec, from the bound columns and the module's own switches.
    ///
    /// No error path, and that is a property of the argument rather than of
    /// the builder: [`ChartKind::spec`] hands over a [`Bound`] only once the
    /// binding has matched this kind's id *and* been re-checked against this
    /// kind's own slots, and it is the only thing that makes one. Both are
    /// checked; neither is inferred from the other. A builder reached that way
    /// has nothing to reject.
    ///
    /// Forwarding to another kind's builder is not that way; [`Bound`] says
    /// what a forwarded one carries.
    pub build: fn(&Bound<'_>, &ModuleOptions) -> S,
}

impl<S> std::fmt::Debug for ChartKind<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChartKind")
            .field("id", &self.id)
            .field("icon", &self.icon)
            .field("description", &self.description)
            .field("slots", &self.slots)
            .finish_non_exhaustive()
    }
}

impl<S> ChartKind<S> {
    /// Bind `fields` to this kind's slots, first fit in slot order.
    ///
    /// Greedy and order-sensitive by design: the slots are declared in the
    /// order the kind wants them filled, and a caller that wants a particular
    /// column in a particular role hands the fields in that order. A column is
    /// used at most once.
    ///
    /// # Errors
    ///
    /// Names the first required slot nothing offered could fill, and what it
    /// would have taken — so a caller can say why a kind was not applicable
    /// rather than only that it was not.
    pub fn bind(&self, fields: &[Field]) -> Result<FieldBinding, String> {
        let mut used = vec![false; fields.len()];
        let mut bound = Vec::new();
        for slot in self.slots {
            let found = fields
                .iter()
                .enumerate()
                .find(|(i, f)| !used[*i] && slot.takes(f));
            match found {
                Some((i, field)) => {
                    used[i] = true;
                    bound.push((slot.role, field.clone()));
                }
                None if slot.required => {
                    let mut wanted = String::new();
                    for (i, ty) in slot.accepts.iter().enumerate() {
                        let sep = if i == 0 { "" } else { " or " };
                        let _ = write!(wanted, "{sep}{ty:?}");
                    }
                    return Err(format!(
                        "{}: nothing left to fill slot {:?}, which takes {}",
                        self.id, slot.role, wanted
                    ));
                }
                None => {}
            }
        }
        Ok(FieldBinding {
            kind: self.id,
            bound,
        })
    }

    /// Whether these columns can fill this kind's required slots.
    #[must_use]
    pub fn accepts(&self, fields: &[Field]) -> bool {
        self.bind(fields).is_ok()
    }

    /// This kind's controls in their declared starting state.
    #[must_use]
    pub fn options(&self) -> ModuleOptions {
        let mut options = ModuleOptions::default();
        for control in (self.controls)() {
            options.set(control.id, control.on_by_default);
        }
        options
    }

    /// This kind's controls as the toolbar entries the chrome draws, each
    /// carrying its current state.
    #[must_use]
    pub fn toolbar(&self, options: &ModuleOptions) -> Vec<ToolbarEntry> {
        (self.controls)()
            .iter()
            .map(|c| c.entry(options.is_on(c.id)))
            .collect()
    }

    /// Check `binding`'s columns against this kind's own slot declaration.
    ///
    /// The id check above says which kind *claimed* the binding. It does not
    /// say that kind declared the slots this one does: [`ChartKindId`] wraps a
    /// `&'static str`, so two [`ChartKind`] values can carry one id and
    /// different `slots`. [`ChartKindRegistry::new`] rejects a duplicate id
    /// within one registry; a kind built outside a registry, or a second
    /// registry's divergent declaration, is not covered by that. So the fit is
    /// re-checked here rather than inferred from the id.
    ///
    /// A role resolves to the first slot declaring it, which is exact for any
    /// kind [`audit_chart_kinds`] passes — the audit rejects a repeated role,
    /// for the same reason [`FieldBinding::field`] answers with the first.
    fn check_slots(&self, binding: &FieldBinding) -> Result<(), String> {
        for (role, field) in &binding.bound {
            let Some(slot) = self.slots.iter().find(|s| s.role == *role) else {
                return Err(format!(
                    "{}: handed a binding filling {role:?}, a slot this kind does not declare",
                    self.id
                ));
            };
            if !slot.takes(field) {
                return Err(format!(
                    "{}: handed {:?} in slot {:?}, which takes {:?} and not {:?}",
                    self.id, field.name, role, slot.accepts, field.ty
                ));
            }
        }
        for slot in self.slots.iter().filter(|s| s.required) {
            if binding.field(slot.role).is_none() {
                return Err(format!(
                    "{}: handed a binding with nothing in required slot {:?}",
                    self.id, slot.role
                ));
            }
        }
        Ok(())
    }

    /// The spec for one module of this kind, and the way into
    /// [`ChartKind::build`]: the builder takes a [`Bound`], and `spec` is the
    /// only thing that makes one. *Which* kind's `spec` made the one a builder
    /// ends up reading is a further question — see [`Bound`].
    ///
    /// # Errors
    ///
    /// If `binding` was produced by a different kind, or if its columns do not
    /// fit *this* kind's slots. Neither is something [`ChartKind::bind`]'s type
    /// can carry — two kinds' bindings are the same Rust type — so both are
    /// checked here rather than assumed. The second is not implied by the
    /// first — an id says which kind claimed a binding, not what that kind
    /// declared — and is done by the private `check_slots`, which says why.
    pub fn spec(&self, binding: &FieldBinding, options: &ModuleOptions) -> Result<S, String> {
        if binding.kind != self.id {
            return Err(format!(
                "{}: handed a binding made for {}",
                self.id, binding.kind
            ));
        }
        self.check_slots(binding)?;
        Ok((self.build)(&Bound(binding), options))
    }
}

/// One view's chart vocabulary.
///
/// The list a picker offers and a generator chooses from, and the thing a
/// module resolves its [`ChartKindId`] against every frame it draws.
pub struct ChartKindRegistry<S> {
    kinds: Vec<ChartKind<S>>,
}

impl<S> ChartKindRegistry<S> {
    /// Build a registry.
    ///
    /// # Panics
    ///
    /// If two kinds share an id. [`ChartKindRegistry::find`] answers with the
    /// first kind declared under the id, so the second is unreachable and a
    /// module naming it draws the other one — hence the failure is at
    /// construction rather than at a draw.
    #[must_use]
    pub fn new(kinds: Vec<ChartKind<S>>) -> Self {
        let mut seen = BTreeSet::new();
        for kind in &kinds {
            assert!(seen.insert(kind.id), "duplicate chart kind id {}", kind.id);
        }
        Self { kinds }
    }

    /// Every kind, in declaration order.
    #[must_use]
    pub fn kinds(&self) -> &[ChartKind<S>] {
        &self.kinds
    }

    /// Every kind's id, in declaration order.
    #[must_use]
    pub fn ids(&self) -> Vec<ChartKindId> {
        self.kinds.iter().map(|k| k.id).collect()
    }

    /// The kind with this id.
    #[must_use]
    pub fn find(&self, id: ChartKindId) -> Option<&ChartKind<S>> {
        self.kinds.iter().find(|k| k.id == id)
    }

    /// Every kind these columns can fill, in declaration order.
    ///
    /// The choosing half of the registry: a caller with a column and no
    /// opinion asks this and takes the first answer, and a caller with an
    /// opinion asks this and picks. Declaration order is therefore a
    /// preference order, and it is the registry's to state.
    #[must_use]
    pub fn applicable(&self, fields: &[Field]) -> Vec<ChartKindId> {
        self.kinds
            .iter()
            .filter(|k| k.accepts(fields))
            .map(|k| k.id)
            .collect()
    }
}

/// Check every chart kind in `reg` against the contract.
///
/// The chart-kind twin of [`audit`], and it exists for the same reason: when a
/// kind is data rather than a component, nothing about it is checked by the
/// compiler beyond its types, and the mistakes that remain — a slot that takes
/// nothing, a control on a verb the keyboard does not have, a description
/// written in a different voice from every other one — are exactly the ones
/// that make a picker read as a pile of one-offs.
///
/// Each kind must: carry a non-empty id and a non-empty icon name; describe
/// itself in the house style, which starts with describing itself at all;
/// declare at least one slot and at least one required slot; give every
/// slot a unique role and at least one accepted type; give every control a
/// unique id and a registered verb; and **build a spec from its own
/// declaration** — the audit synthesises one column per required slot, binds
/// it, and calls the builder twice, once in the controls' declared starting
/// state and once with every declared control flipped out of it, so a builder
/// that cannot survive its own constraints fails here rather than at a user's
/// first click.
///
/// # Errors
///
/// The reason the first failing kind failed, naming the kind and the rule, so
/// a caller's assertion can pin *which* rule bit.
pub fn audit_chart_kinds<S>(reg: &ChartKindRegistry<S>) -> Result<(), String> {
    for kind in reg.kinds() {
        let id = kind.id;
        if id.as_str().is_empty() {
            return Err("a chart kind with an empty id".to_string());
        }
        if kind.icon.as_str().is_empty() {
            return Err(format!("{id}: no icon"));
        }

        let description = kind.description;
        if description.is_empty() {
            return Err(format!("{id}: no description"));
        }
        if description.ends_with('.') {
            return Err(format!("{id}: a description takes no terminal period"));
        }
        if !description.chars().next().is_some_and(char::is_uppercase) {
            return Err(format!("{id}: a description is sentence case"));
        }

        if kind.slots.is_empty() {
            return Err(format!("{id}: takes no fields at all"));
        }
        let mut roles = BTreeSet::new();
        for slot in kind.slots {
            if slot.accepts.is_empty() {
                return Err(format!("{id}: slot {:?} accepts nothing", slot.role));
            }
            if !roles.insert(slot.role) {
                return Err(format!("{id}: two slots share the role {:?}", slot.role));
            }
        }
        if !kind.slots.iter().any(|s| s.required) {
            return Err(format!(
                "{id}: every slot is optional, so nothing decides whether this \
                 kind applies to a column"
            ));
        }

        let controls = (kind.controls)();
        let mut control_ids = BTreeSet::new();
        for control in &controls {
            if !control.verb.is_registered() {
                return Err(format!(
                    "{id}: control {:?} declares unregistered verb {:?}",
                    control.id,
                    control.verb.as_str()
                ));
            }
            if !control_ids.insert(control.id) {
                return Err(format!("{id}: two controls share the id {:?}", control.id));
            }
        }

        // The kind against its own declaration: one column per required slot,
        // of the first type that slot accepts.
        let fields: Vec<Field> = kind
            .slots
            .iter()
            .filter(|s| s.required)
            .enumerate()
            .map(|(i, s)| Field::new(format!("audit_{i}"), s.accepts[0]))
            .collect();
        let binding = kind
            .bind(&fields)
            .map_err(|e| format!("{id}: cannot bind its own required slots — {e}"))?;

        let mut options = kind.options();
        kind.spec(&binding, &options)?;
        for control in &controls {
            options.set(control.id, !options.is_on(control.id));
        }
        kind.spec(&binding, &options)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests — a chart kind as data, and the audit that keeps it honest
// ---------------------------------------------------------------------------

#[cfg(test)]
mod chart_kind_tests {
    use super::*;

    /// A stand-in for a view's spec type. The registry never names one — see
    /// [`ChartKind`] — so a test has to bring its own, and a comparable struct
    /// is what makes "these two paths build the same chart" an equality rather
    /// than a screenshot.
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Spec {
        mark: &'static str,
        x: String,
        y: String,
        y_scale: &'static str,
    }

    const BARS: ChartKindId = ChartKindId::new("bars");
    const POINTS: ChartKindId = ChartKindId::new("points");

    /// The control id and the verb are declared once, here, and read by the
    /// kind and by the assertions alike — a test that spelled the id a second
    /// time would pass while the product was broken.
    const LOG_SCALE: &str = "log-scale";

    fn log_verb() -> Verb {
        Verb::new("cycle-axis-lock")
    }

    /// A slot list has to be a `const` item rather than an inline `&[..]`:
    /// `FieldSlot::required` is a `const fn`, but a call is not promoted to
    /// `'static` inside a struct literal.
    const BAR_SLOTS: &[FieldSlot] = &[
        FieldSlot::required("x", &[FieldType::Categorical]),
        FieldSlot::required("y", &[FieldType::Quantitative]),
    ];

    const POINT_SLOTS: &[FieldSlot] = &[
        FieldSlot::required("x", &[FieldType::Quantitative]),
        FieldSlot::required("y", &[FieldType::Quantitative]),
        FieldSlot::optional("colour", &[FieldType::Categorical]),
    ];

    /// A kind, whole, as data. Nothing here is a type: the icon, the gloss,
    /// what it takes, what it hangs on itself and how it builds are five
    /// fields of one value.
    fn bars() -> ChartKind<Spec> {
        ChartKind {
            id: BARS,
            icon: Icon("chart-bar"),
            description: "Ranks a category by a measure",
            slots: BAR_SLOTS,
            controls: || {
                vec![ModuleControl::new(LOG_SCALE, "Log", log_verb())
                    .with_tooltip("Log scale the measure axis")]
            },
            build: |binding, options| Spec {
                mark: "bar",
                x: binding.name("x").unwrap_or_default().to_string(),
                y: binding.name("y").unwrap_or_default().to_string(),
                y_scale: if options.is_on(LOG_SCALE) {
                    "log"
                } else {
                    "linear"
                },
            },
        }
    }

    /// The second kind. The whole of adding it is this function — no component,
    /// no trait implementation, no branch anywhere else.
    fn points() -> ChartKind<Spec> {
        ChartKind {
            id: POINTS,
            icon: Icon("chart-dots"),
            description: "Shows one measure against another",
            slots: POINT_SLOTS,
            controls: Vec::new,
            build: |binding, _| Spec {
                mark: "dot",
                x: binding.name("x").unwrap_or_default().to_string(),
                y: binding.name("y").unwrap_or_default().to_string(),
                y_scale: "linear",
            },
        }
    }

    fn registry() -> ChartKindRegistry<Spec> {
        ChartKindRegistry::new(vec![bars(), points()])
    }

    fn category(name: &str) -> Field {
        Field::new(name, FieldType::Categorical)
    }

    fn measure(name: &str) -> Field {
        Field::new(name, FieldType::Quantitative)
    }

    /// A kind carries all four things a picker and a generator need, and the
    /// fourth is a function rather than a component.
    #[test]
    fn a_kind_is_an_icon_a_gloss_a_set_of_constraints_and_a_builder() {
        let kind = bars();
        assert_eq!(kind.icon, Icon("chart-bar"));
        assert_eq!(kind.description, "Ranks a category by a measure");
        assert_eq!(
            kind.slots
                .iter()
                .map(|s| (s.role, s.accepts, s.required))
                .collect::<Vec<_>>(),
            vec![
                ("x", &[FieldType::Categorical][..], true),
                ("y", &[FieldType::Quantitative][..], true),
            ]
        );

        let fields = vec![category("sector"), measure("revenue")];
        let binding = kind.bind(&fields).expect("the columns fit the slots");
        assert_eq!(
            kind.spec(&binding, &kind.options())
                .expect("its own binding"),
            Spec {
                mark: "bar",
                x: "sector".to_string(),
                y: "revenue".to_string(),
                y_scale: "linear",
            }
        );
    }

    /// The type constraints are enforced by [`ChartKind::bind`], once, rather
    /// than by each builder — which is why a builder takes bound columns
    /// instead of checking raw ones. What keeps those columns attached to this
    /// kind is in [`ChartKind::spec`] — the id check, pinned by
    /// `a_binding_belongs_to_the_kind_that_made_it` below, and the slot
    /// re-check that the id does not imply; both, and where they stop, are
    /// pinned by `tests/chart_kind_bound.rs`.
    #[test]
    fn a_slot_refuses_a_column_of_the_wrong_type() {
        let kind = bars();
        let err = kind
            .bind(&[measure("revenue"), measure("margin")])
            .expect_err("no categorical column, so no bars");
        assert!(err.contains("\"x\""), "the error names the slot: {err}");
        assert!(err.contains("Categorical"), "and what it wanted: {err}");
        assert!(kind.bind(&[category("sector")]).is_err(), "y is required");
    }

    /// An optional slot left empty still binds, and reads back as absent.
    #[test]
    fn an_optional_slot_is_optional() {
        let kind = points();
        let binding = kind
            .bind(&[measure("revenue"), measure("margin")])
            .expect("two measures are enough");
        assert_eq!(binding.name("x"), Some("revenue"));
        assert_eq!(binding.name("y"), Some("margin"));
        assert_eq!(binding.field("colour"), None);
        assert_eq!(binding.roles().collect::<Vec<_>>(), vec!["x", "y"]);
    }

    /// The choosing half: hand the registry columns and it says which kinds
    /// they can fill, in declaration order — which is therefore a preference
    /// order.
    #[test]
    fn the_registry_says_which_kinds_a_set_of_columns_can_fill() {
        let reg = registry();
        assert_eq!(reg.ids(), vec![BARS, POINTS]);
        assert_eq!(
            reg.applicable(&[category("sector"), measure("revenue")]),
            vec![BARS]
        );
        assert_eq!(
            reg.applicable(&[measure("revenue"), measure("margin")]),
            vec![POINTS]
        );
        assert_eq!(
            reg.applicable(&[category("sector")]),
            Vec::<ChartKindId>::new(),
            "one category fills neither kind's required slots"
        );
    }

    /// A binding is stamped with the kind that made it, so it cannot be handed
    /// to a different builder — the one thing the type cannot carry, since two
    /// kinds' bindings are the same Rust type.
    #[test]
    fn a_binding_belongs_to_the_kind_that_made_it() {
        let fields = vec![measure("revenue"), measure("margin")];
        let binding = points().bind(&fields).expect("two measures");
        assert_eq!(binding.kind(), POINTS);
        let err = bars()
            .spec(&binding, &ModuleOptions::default())
            .expect_err("bars did not make this binding");
        assert!(err.contains("points"), "the error names the maker: {err}");
    }

    /// A module's own controls start where the kind said they start, and
    /// flipping one changes the spec that kind builds. This is the whole of
    /// "the toggle belongs to the module": no other kind and no other module
    /// moved.
    #[test]
    fn a_control_the_kind_declares_changes_that_kinds_spec() {
        let kind = bars();
        let fields = vec![category("sector"), measure("revenue")];
        let binding = kind.bind(&fields).expect("the columns fit");

        let mut options = kind.options();
        assert!(!options.is_on(LOG_SCALE), "declared off");
        assert_eq!(
            kind.spec(&binding, &options)
                .expect("its own binding")
                .y_scale,
            "linear"
        );

        let entries = kind.toolbar(&options);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, LOG_SCALE);
        assert!(!entries[0].on, "the entry reports the option's state");
        assert_eq!(entries[0].verb, log_verb());

        assert!(options.toggle(LOG_SCALE));
        assert_eq!(
            kind.spec(&binding, &options)
                .expect("its own binding")
                .y_scale,
            "log"
        );
        assert!(kind.toolbar(&options)[0].on);

        // The kind with no controls is unmoved by any of it.
        assert!(points().toolbar(&options).is_empty());
    }

    /// The conformance gate over a registry that satisfies it.
    #[test]
    fn the_audit_passes_a_registry_that_keeps_the_contract() {
        audit_chart_kinds(&registry()).expect("both kinds keep the contract");
    }

    /// …and fails each rule it claims to hold, one at a time. A gate nobody
    /// has watched fail is a gate nobody knows still works.
    #[test]
    fn the_audit_fails_each_rule_it_claims_to_hold() {
        const ACCEPTS_NOTHING: &[FieldSlot] = &[FieldSlot::required("x", &[])];
        const ROLE_TWICE: &[FieldSlot] = &[
            FieldSlot::required("x", &[FieldType::Categorical]),
            FieldSlot::optional("x", &[FieldType::Quantitative]),
        ];
        const ALL_OPTIONAL: &[FieldSlot] = &[FieldSlot::optional("x", &[FieldType::Categorical])];

        let cases: Vec<(&str, ChartKind<Spec>, &str)> = vec![
            (
                "an empty id",
                ChartKind {
                    id: ChartKindId::new(""),
                    ..bars()
                },
                "empty id",
            ),
            (
                "no description at all",
                ChartKind {
                    description: "",
                    ..bars()
                },
                "no description",
            ),
            (
                "terminal period",
                ChartKind {
                    description: "Ranks a category by a measure.",
                    ..bars()
                },
                "terminal period",
            ),
            (
                "lower case",
                ChartKind {
                    description: "ranks a category by a measure",
                    ..bars()
                },
                "sentence case",
            ),
            (
                "no icon",
                ChartKind {
                    icon: Icon(""),
                    ..bars()
                },
                "no icon",
            ),
            (
                "a slot that accepts nothing",
                ChartKind {
                    slots: ACCEPTS_NOTHING,
                    ..bars()
                },
                "accepts nothing",
            ),
            (
                "two slots under one role",
                ChartKind {
                    slots: ROLE_TWICE,
                    ..bars()
                },
                "share the role",
            ),
            (
                "nothing required",
                ChartKind {
                    slots: ALL_OPTIONAL,
                    ..bars()
                },
                "every slot is optional",
            ),
            (
                "no slots at all",
                ChartKind {
                    slots: &[],
                    ..bars()
                },
                "takes no fields",
            ),
            // The unregistered-verb rule is deliberately absent from this
            // list, for the reason `subject_contract.rs`'s
            // `the_longname_the_unit_test_smuggles_in_is_genuinely_unregistered`
            // states about the same rule on a subject's verbs: `Verb`'s field
            // is private to the `subject` module, so nothing outside it can
            // build a verb that bypassed `Verb::new`'s debug assertion. The
            // rule here is the release-mode backstop, identical in shape to
            // the one `audit` applies to `ItemSpec::toggle`.
            (
                "two controls under one id",
                ChartKind {
                    controls: || {
                        vec![
                            ModuleControl::new("same", "A", Verb::new("cycle-axis-lock")),
                            ModuleControl::new("same", "B", Verb::new("toggle-presentation")),
                        ]
                    },
                    ..bars()
                },
                "share the id",
            ),
        ];

        for (name, kind, expected) in cases {
            let reg = ChartKindRegistry::new(vec![kind]);
            let err = audit_chart_kinds(&reg).expect_err(&format!("the audit let {name} through"));
            assert!(
                err.contains(expected),
                "{name}: expected an error naming {expected:?}, got {err:?}"
            );
        }
    }

    /// The audit builds each kind from its own declaration, so a builder that
    /// cannot survive its own constraints fails at the gate rather than at a
    /// user's first click.
    #[test]
    fn the_audit_runs_each_builder_against_the_slots_that_kind_declares() {
        // A builder that refuses anything but a fully bound binding, and that
        // records which control states it was called with. Passing the audit
        // is what proves the synthesised binding is a real one and that both
        // states of every declared control were exercised — the half a single
        // call would miss.
        static SCALES: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
        let kind: ChartKind<Spec> = ChartKind {
            build: |binding, options| {
                let x = binding.name("x").expect("x was not bound");
                let y = binding.name("y").expect("y was not bound");
                let y_scale = if options.is_on(LOG_SCALE) {
                    "log"
                } else {
                    "linear"
                };
                SCALES.lock().expect("uncontended").push(y_scale);
                Spec {
                    mark: "bar",
                    x: x.to_string(),
                    y: y.to_string(),
                    y_scale,
                }
            },
            ..bars()
        };
        audit_chart_kinds(&ChartKindRegistry::new(vec![kind])).expect("the builder survives");
        assert_eq!(
            SCALES.lock().expect("uncontended").as_slice(),
            ["linear", "log"],
            "this kind's control is declared off, so the starting state is off \
             and the flip is on"
        );
    }

    /// The audit's first call is the controls' *declared* starting state, not
    /// "off". A kind whose control starts on is therefore called on first and
    /// off second — the reverse of the case above, which is why that one
    /// cannot pin the order: with a control declared off the two readings
    /// coincide.
    #[test]
    fn the_audit_starts_a_control_where_the_kind_declared_it() {
        static SCALES: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
        let kind: ChartKind<Spec> = ChartKind {
            controls: || vec![ModuleControl::new(LOG_SCALE, "Log", log_verb()).on_by_default(true)],
            build: |binding, options| {
                let y_scale = if options.is_on(LOG_SCALE) {
                    "log"
                } else {
                    "linear"
                };
                SCALES.lock().expect("uncontended").push(y_scale);
                Spec {
                    mark: "bar",
                    x: binding.name("x").unwrap_or_default().to_string(),
                    y: binding.name("y").unwrap_or_default().to_string(),
                    y_scale,
                }
            },
            ..bars()
        };
        assert!(kind.options().is_on(LOG_SCALE), "declared on");

        audit_chart_kinds(&ChartKindRegistry::new(vec![kind])).expect("the builder survives");
        assert_eq!(
            SCALES.lock().expect("uncontended").as_slice(),
            ["log", "linear"],
            "a control declared on is exercised on first and off second"
        );
    }

    /// Two kinds under one id would leave the second unreachable through
    /// `find`, which surfaces as a module drawing the other one.
    #[test]
    #[should_panic(expected = "duplicate chart kind id")]
    fn a_registry_refuses_two_kinds_under_one_id() {
        let _ = ChartKindRegistry::new(vec![bars(), bars()]);
    }
}
