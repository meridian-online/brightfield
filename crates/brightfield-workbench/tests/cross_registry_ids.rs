//! What one window's tile tree does with an item id two registries both claim.
//!
//! [`ItemRegistry::new`] refuses a duplicate id inside its own spec list. A
//! *window* is built from more than one registry — one per document — and
//! [`window_tree`] takes their placements together, so the id space that has
//! to be unique is the window's rather than any one registry's, and no
//! constructor is handed the whole of it. That was harmless while each
//! document had a tile tree of its own: two panes in different trees sharing
//! an id addressed different tiles. One tree removes the separation.
//!
//! **Where the property is asserted is not here.** Only a caller that knows
//! which registries a window builds can compare them, so the shell asserts it
//! over its own two in `crates/brightfield-shell/tests/one_window.rs`
//! (`no_item_id_is_declared_by_both_of_the_windows_registries`, and the
//! vocabulary-arithmetic in
//! `a_window_publishes_both_registries_and_nothing_else`). This file is the
//! other half of the same question: what the tree and the layout file do when
//! that property is broken, so the cost is written down rather than argued
//! about.
//!
//! # Why the id space belongs to the tree and not to the registry pair
//!
//! Read from where the ids are consumed. A [`PaneKey`] is a *tile address*:
//! [`Workspace::tile_of`] resolves one to a single [`egui_tiles::TileId`],
//! `Workspace::panes` documents that tile order is not stable, and the layout
//! file records a pane as the bare id string. Each of those breaks on a
//! duplicate, and each of them is downstream of the one tree — not of a
//! registry, which is already self-consistent, and not of a "pair", which is
//! not a thing the code has: `default_layout` in
//! `crates/brightfield-shell/src/startup.rs` concatenates placements and a
//! third document would be a third `extend`.
//!
//! # Its own test binary
//!
//! [`ItemId::publish`] writes a process-global vocabulary, so a file that
//! publishes has to be alone with it — the reason
//! `crates/brightfield-workbench/tests/layout_file.rs` gives for being one
//! ordered test. A separate integration test is a separate process, so the
//! vocabulary here starts empty and the publish below is the only one.

use brightfield_keys::BindingContext;
use brightfield_workbench::persist::{self, LoadOutcome, SavedLayout};
use brightfield_workbench::{
    window_tree, DockSide, EmptyState, Icon, Item, ItemCtx, ItemId, ItemRegistry, ItemSpec,
    PaneKey, Slot, Subject, Workspace,
};

// ---------------------------------------------------------------------------
// Two documents, each with a registry, in the shape a window is built from
// ---------------------------------------------------------------------------

/// The first document's centre pane.
const ALPHA_CANVAS: ItemId = ItemId::new("alpha-canvas");
/// The second document's centre pane.
const BETA_CANVAS: ItemId = ItemId::new("beta-canvas");
/// The id both documents come to claim. Declared once here so the fixtures
/// and the assertions cannot drift into two spellings of it.
const SHARED: ItemId = ItemId::new("claimed-by-both");

#[derive(Default)]
struct Doc;

struct Pane(ItemId);

impl Item<Doc> for Pane {
    fn item_id(&self) -> ItemId {
        self.0
    }
    fn empty_state(&self, _doc: &Doc) -> Option<EmptyState> {
        Some(EmptyState::new(
            Icon("pane"),
            "Nothing here yet",
            "A fixture pane with no content of its own",
        ))
    }
    fn describe(&self, _doc: &Doc) -> Subject {
        Subject::new("Pane", Icon("pane"), BindingContext::Workspace)
    }
    fn ui(&mut self, _doc: &mut Doc, _ui: &mut egui::Ui, _cx: &mut ItemCtx<'_>) {}
}

/// `make` is a `fn` pointer, not a closure, so each spec names its own
/// constructor rather than capturing an id.
fn spec(id: ItemId, slot: Slot, make: fn() -> Box<dyn Item<Doc>>) -> ItemSpec<Doc> {
    ItemSpec {
        id,
        slot,
        toggle: None,
        make,
    }
}

fn rail(side: DockSide) -> Slot {
    Slot::Rail { side, share: 0.25 }
}

fn alpha() -> ItemRegistry<Doc> {
    ItemRegistry::new(vec![
        spec(ALPHA_CANVAS, Slot::Centre, || Box::new(Pane(ALPHA_CANVAS))),
        spec(SHARED, rail(DockSide::Right), || Box::new(Pane(SHARED))),
    ])
}

/// The second document as it was before it grew a pane at [`SHARED`] — the
/// build whose layout file the load below was written by.
fn beta_before() -> ItemRegistry<Doc> {
    ItemRegistry::new(vec![spec(BETA_CANVAS, Slot::Centre, || {
        Box::new(Pane(BETA_CANVAS))
    })])
}

/// The second document after it declares an id the first already uses. Each
/// registry on its own is legal: the id appears once in this spec list, so
/// [`ItemRegistry::new`] has nothing to complain about.
fn beta_after() -> ItemRegistry<Doc> {
    ItemRegistry::new(vec![
        spec(BETA_CANVAS, Slot::Centre, || Box::new(Pane(BETA_CANVAS))),
        spec(SHARED, rail(DockSide::Left), || Box::new(Pane(SHARED))),
    ])
}

/// One window over both documents' panes, the way the shell's
/// `default_layout` builds one: placements concatenated, then a single
/// [`window_tree`].
fn window_of(a: &ItemRegistry<Doc>, b: &ItemRegistry<Doc>) -> Workspace {
    let mut placements = a.placements();
    placements.extend(b.placements());
    Workspace::new(window_tree(&placements))
}

/// This build's default arrangement — both documents, both claiming
/// [`SHARED`]. A named function rather than a closure because
/// `persist::from_json` consumes the one it is given and the assertions need
/// it again afterwards.
fn defaults_after() -> SavedLayout {
    SavedLayout::new(window_of(&alpha(), &beta_after()))
}

/// Every tile in `ws` holding `key`. [`Workspace::tile_of`] answers with one
/// tile and has no way to say there was a second, which is the point of
/// counting them here instead.
fn tiles_holding(ws: &Workspace, key: PaneKey) -> Vec<egui_tiles::TileId> {
    ws.tree()
        .tiles
        .iter()
        .filter_map(|(id, tile)| match tile {
            egui_tiles::Tile::Pane(k) if *k == key => Some(*id),
            egui_tiles::Tile::Pane(_) | egui_tiles::Tile::Container(_) => None,
        })
        .collect()
}

fn count_of(panes: &[PaneKey], key: PaneKey) -> usize {
    panes.iter().filter(|k| **k == key).count()
}

// ---------------------------------------------------------------------------
// The registry's own rule, and where it stops
// ---------------------------------------------------------------------------

/// One registry still refuses the same id twice, at construction.
///
/// The control for everything below: the rule exists one level down and is
/// unchanged, so a window that admits a duplicate did not lose this check —
/// it was never in a position to run it.
///
/// Would catch: the duplicate assertion in [`ItemRegistry::new`] being
/// dropped or weakened, which would make the failure this file measures
/// reachable from a single spec list as well as from two.
#[test]
#[should_panic(expected = "duplicate item id")]
fn one_registry_still_refuses_the_same_id_twice() {
    let _ = ItemRegistry::new(vec![
        spec(ALPHA_CANVAS, Slot::Centre, || Box::new(Pane(ALPHA_CANVAS))),
        spec(SHARED, rail(DockSide::Right), || Box::new(Pane(SHARED))),
        spec(SHARED, rail(DockSide::Left), || Box::new(Pane(SHARED))),
    ]);
}

/// Two registries sharing an id put two tiles at one address, and one of them
/// cannot be reached.
///
/// This is the mechanism behind the whole file. Neither registry is
/// ill-formed, so nothing refuses them; [`window_tree`] inserts a pane per
/// placement, so the tree ends up with two tiles carrying the same
/// [`PaneKey`]. [`Workspace::tile_of`] resolves a key to one tile, so from
/// that point on one document's pane has no address at all — the shell drives
/// panes through `tile_of` to hide them, to head them and to find the tab
/// strip a pane sits in, and each of those would act on whichever tile came
/// back.
///
/// Would catch: [`window_tree`] gaining a de-duplication step that quietly
/// drops the second placement — the tile count below would fall to one, which
/// is a pane vanishing rather than a pane being unaddressable, and no louder.
#[test]
fn two_registries_that_share_an_id_put_two_tiles_at_one_address() {
    let key = PaneKey::new(SHARED);

    let separate = window_of(&alpha(), &beta_before());
    assert_eq!(
        tiles_holding(&separate, key).len(),
        1,
        "the control window already has two tiles at this id, so the \
         comparison below measures nothing"
    );

    let window = window_of(&alpha(), &beta_after());
    let tiles = tiles_holding(&window, key);
    assert_eq!(
        tiles.len(),
        2,
        "one placement per declaration is what makes the address ambiguous; \
         with {} tile(s) it is not",
        tiles.len()
    );
    assert_eq!(
        count_of(&window.panes(), key),
        2,
        "the window reports one pane at an id two documents declared"
    );

    let reached = window
        .tile_of(key)
        .expect("a tile was inserted for the shared id");
    assert!(
        tiles.contains(&reached),
        "tile_of answered with a tile that is not one of the two holding this key"
    );
    let unreachable: Vec<egui_tiles::TileId> =
        tiles.iter().copied().filter(|t| *t != reached).collect();
    assert_eq!(
        unreachable.len(),
        1,
        "one of the two tiles has no way to be named, and that is the defect; \
         instead {unreachable:?} are unreachable"
    );
}

// ---------------------------------------------------------------------------
// The layout file, against a default arrangement that places one id twice
// ---------------------------------------------------------------------------

/// A saved layout naming an id two panes now claim is restored as written,
/// and the window comes up with one tile where two panes are declared.
///
/// The upgrade shape, end to end. The file was written by a build where one
/// document declared the id; it is read by a build where both do. It parses,
/// because the id is published and a pane key is just that id. Then
/// `persist::from_json` asks whether the file is short of any pane the
/// default arrangement places — and it asks by membership, so one tile named
/// `claimed-by-both` satisfies a default that places two. The load reports
/// [`LoadOutcome::Restored`], and the pane belonging to whichever document
/// loses the race in `tile_of` is not on screen and nothing says so.
///
/// **This test records the outcome, it does not endorse it.** The
/// arrangement a reader would want is the one an incomplete file already
/// gets: [`LoadOutcome::Incomplete`], the default arrangement, both panes
/// placed. Getting it means comparing the two pane lists as multisets rather
/// than by membership, in `Workspace::panes_missing_from` — a change to a
/// file outside this one. A build that makes it must invert the two
/// assertions flagged below; they are here so that it is a visible decision
/// rather than a silent one.
///
/// Would catch: `panes_missing_from` becoming multiset-aware (the outcome
/// turns `Incomplete`), and the completeness check being dropped from
/// `from_json` altogether (the restored pane count stops being the file's).
#[test]
fn a_saved_layout_naming_an_id_two_panes_claim_is_restored_short_a_pane() {
    let key = PaneKey::new(SHARED);

    // As boot does: every registry publishes before any file is read.
    ItemId::publish(Box::leak(
        vec![ALPHA_CANVAS, BETA_CANVAS, SHARED].into_boxed_slice(),
    ));
    for id in [ALPHA_CANVAS, BETA_CANVAS, SHARED] {
        assert!(
            ItemId::known().contains(&id),
            "{id} did not publish, so the load below would report Corrupt for \
             a reason that has nothing to do with this test"
        );
    }

    // The file the previous build wrote: one tile at the shared id.
    let written = SavedLayout::new(window_of(&alpha(), &beta_before()));
    assert_eq!(
        count_of(&written.workspace.panes(), key),
        1,
        "the saved file already carries the duplicate, so the load below is \
         not the upgrade case it claims to be"
    );
    let json = written.to_json().expect("a layout serialises");

    // This build's default arrangement: two.
    assert_eq!(
        count_of(&defaults_after().workspace.panes(), key),
        2,
        "the default arrangement does not place the shared id twice, so there \
         is no shortfall for the load to miss"
    );

    let (loaded, outcome) = persist::from_json(Some(&json), defaults_after);

    // The line to invert when `panes_missing_from` compares multisets: the
    // wanted outcome is `Incomplete`, and the wanted arrangement is the
    // default one.
    assert_eq!(
        outcome,
        LoadOutcome::Restored,
        "the load path grew a defence against an id two panes claim — good; \
         this assertion and the two below pin the old behaviour and are the \
         ones to invert. Reported: {}",
        outcome.reason()
    );

    // The consequence, on the workspace the caller is handed: the default
    // places the pane twice and the restored window holds it once, so one
    // document's pane is not drawn and no [`LoadOutcome`] says why.
    assert_eq!(
        count_of(&loaded.workspace.panes(), key),
        1,
        "the restored window does not carry what the file it came from did"
    );
    assert_eq!(
        loaded.workspace.panes().len() + 1,
        defaults_after().workspace.panes().len(),
        "the restored window is short exactly the second tile, and the load \
         reported success"
    );
}
