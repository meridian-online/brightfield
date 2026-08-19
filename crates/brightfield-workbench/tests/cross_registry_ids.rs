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
//! (`no_item_id_is_declared_by_two_of_the_windows_registries`, and the
//! vocabulary-arithmetic in
//! `a_window_publishes_both_registries_and_nothing_else`). This file is the
//! other half of the same question: what the tree does when that property is
//! broken, and what the layout file is required to do about it — the tile
//! count is a measurement, and the load is a rule the last test holds.
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
use brightfield_workbench::persist::{self, LoadOutcome, SavedLayout, WindowGeometry, LAYOUT_FILE};
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
/// [`ItemRegistry::new`] sees no duplicate.
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

/// A directory of this test's own to write a layout file into, removed on
/// drop. The load below goes through a real file rather than through
/// `persist::from_json` so that "boots on it" covers the read as well as the
/// parse.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "brightfield-workbench-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        Self(dir)
    }
    fn file(&self) -> std::path::PathBuf {
        self.0.join(LAYOUT_FILE)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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
/// that point on one document's pane has no address a caller can reach — the
/// shell drives panes through `tile_of` to hide them, to head them and to
/// find the tab strip a pane sits in, and each of those would act on
/// whichever tile came back.
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

/// A saved layout naming an id two panes now claim is discarded, and the
/// window opens on the default arrangement with both panes placed.
///
/// The upgrade shape, end to end, through a real file. It was written by a
/// build where one document declared the id and is read by a build where both
/// do. It parses, because the id is published and a pane key is just that id.
/// Then `persist::from_json` asks whether the file is short of any pane the
/// default arrangement places, and — because [`Workspace::panes_missing_from`]
/// consumes each match rather than asking `contains` — one tile named
/// `claimed-by-both` does not cover a default that places two. The file is
/// short a pane, so it goes the way any other short file goes:
/// [`LoadOutcome::Incomplete`], and the default arrangement in its place.
///
/// The outcome enum is the smaller half of that claim and is asserted
/// alongside the larger one: the workspace the caller is handed is compared
/// against the default itself, and against the file it came from, so
/// "discarded in favour of the default" is a statement about the arrangement
/// the window ends up with rather than about a label. Which is the difference
/// that matters — an [`LoadOutcome::Incomplete`] that reported honestly and
/// handed back the file anyway would leave a pane undrawn just the same.
///
/// The window geometry is asserted to survive, because the discard is scoped
/// to the arrangement: the size and position the analyst left are not what was
/// wrong with the file, and resizing their window over a pane they never saw
/// would be a change they did not ask for. The shell's half of that rule is
/// its `kept_window_geometry`.
///
/// Would catch: `panes_missing_from` going back to a membership test (the
/// outcome turns `Restored` and the file's own short arrangement comes back);
/// the completeness check being dropped from `from_json`; and `from_json`
/// replacing more than the arrangement.
#[test]
fn a_saved_layout_naming_an_id_two_panes_claim_is_discarded_for_the_default() {
    let key = PaneKey::new(SHARED);
    let scratch = Scratch::new("cross-registry");
    let path = scratch.file();

    // As boot does: each registry publishes before any file is read.
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

    // The file the previous build wrote: one tile at the shared id, and a
    // window geometry the analyst chose.
    let mut written = SavedLayout::new(window_of(&alpha(), &beta_before()));
    written.window = WindowGeometry {
        size: [1440.0, 900.0],
        position: Some([12.0, 34.0]),
    };
    assert_eq!(
        count_of(&written.workspace.panes(), key),
        1,
        "the saved file already carries the duplicate, so the load below is \
         not the upgrade case it claims to be"
    );
    written.save(&path).expect("the layout writes");

    // This build's default arrangement places the shared id twice, which is
    // the shortfall the file has to be measured against.
    assert_eq!(
        count_of(&defaults_after().workspace.panes(), key),
        2,
        "the default arrangement does not place the shared id twice, so there \
         is no shortfall for the load to miss"
    );
    assert_ne!(
        written.workspace,
        defaults_after().workspace,
        "the file and the default are the same arrangement, so nothing below \
         can tell which one came back"
    );

    let (loaded, outcome) = persist::load(&path, defaults_after);

    assert_eq!(
        outcome,
        LoadOutcome::Incomplete,
        "a file short a pane the window declares was reported as {}",
        outcome.reason()
    );
    assert!(
        !outcome.restored(),
        "an arrangement the analyst did not make was reported as a restored one"
    );

    // The arrangement the window ends up with — the half of the claim the
    // enum does not carry.
    assert_eq!(
        loaded.workspace,
        defaults_after().workspace,
        "the load reported Incomplete and did not put the default arrangement \
         in place, so a pane is still undrawn"
    );
    assert_ne!(
        loaded.workspace, written.workspace,
        "the file's own arrangement came back, which is the pane loss this \
         load exists to prevent"
    );
    assert_eq!(
        count_of(&loaded.workspace.panes(), key),
        2,
        "the window opened with one tile at an id two documents declare, so \
         one of them draws nowhere"
    );

    // The arrangement, and only the arrangement: what the analyst chose about
    // the window itself was not what was wrong with this file.
    assert_eq!(
        loaded.window, written.window,
        "a pane the file predates also resized the window the analyst left"
    );
}
