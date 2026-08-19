//! The workspace and the behaviour that draws it, over real frames.
//!
//! `egui::Context` is CPU-only, so a whole view can be laid out, hit-tested
//! and tessellated here with no adapter and no window. Everything below runs
//! the real `Tree::ui` against the real [`PaneChrome`]; nothing is simulated.
//!
//! Nothing here deserialises a [`PaneKey`], so nothing here touches
//! [`brightfield_workbench::ItemId::publish`] and its process-global
//! vocabulary. The tests that do are in `layout_file.rs`, deliberately as one
//! ordered test — see that file.

use std::collections::BTreeSet;

use brightfield_keys::BindingContext;
use brightfield_workbench::persist::SavedLayout;
use brightfield_workbench::{
    chrome, DirtyTracker, DockSide, EmptyState, Icon, Item, ItemCtx, ItemId, ItemMap, ItemRegistry,
    ItemSpec, Mode, PaneChrome, PaneKey, Request, Slot, Subject, Workspace,
};

// ---------------------------------------------------------------------------
// A fixture view: a centre pane with a tab beside it, and a right rail
// ---------------------------------------------------------------------------

const CANVAS: ItemId = ItemId::new("fixture-canvas");
const NOTES: ItemId = ItemId::new("fixture-notes");
const RAIL: ItemId = ItemId::new("fixture-rail");

/// The document every pane in a view shares. No item holds one — the
/// workspace hands it in — which is the property `tab_title_for_pane` below
/// exists to exercise.
#[derive(Default)]
struct Doc {
    /// Read by the centre pane's `subject`, so a test can change what a *tab
    /// title* says without touching the item at all.
    title: String,
    /// Non-empty means the rail has something to show.
    rows: Vec<&'static str>,
    /// Incremented by every `Item::ui` that actually ran. The observable for
    /// "did the shell draw the empty state instead of the pane".
    draws: u32,
    /// Which items drew, not merely how many. A bare count comparison passes
    /// if the extra draw came from somewhere else entirely — a second frame,
    /// a different pane becoming active — so the assertions name the pane.
    drew: BTreeSet<ItemId>,
}

fn toggle() -> brightfield_workbench::Verb {
    brightfield_workbench::Verb::new("toggle-focus")
}

struct Canvas;
impl Item<Doc> for Canvas {
    fn item_id(&self) -> ItemId {
        CANVAS
    }
    fn empty_state(&self, _doc: &Doc) -> Option<EmptyState> {
        None
    }
    fn describe(&self, doc: &Doc) -> Subject {
        Subject::new(doc.title.clone(), Icon("chart"), BindingContext::Workspace)
    }
    fn ui(&mut self, doc: &mut Doc, ui: &mut egui::Ui, _cx: &mut ItemCtx<'_>) {
        doc.draws += 1;
        doc.drew.insert(CANVAS);
        ui.label("canvas");
    }
}

struct Notes;
impl Item<Doc> for Notes {
    fn item_id(&self) -> ItemId {
        NOTES
    }
    fn empty_state(&self, _doc: &Doc) -> Option<EmptyState> {
        None
    }
    fn describe(&self, _doc: &Doc) -> Subject {
        Subject::new("Notes", Icon("note"), BindingContext::Editor)
    }
    fn ui(&mut self, doc: &mut Doc, _ui: &mut egui::Ui, _cx: &mut ItemCtx<'_>) {
        doc.draws += 1;
        doc.drew.insert(NOTES);
    }
}

/// Empty exactly when the document has no rows, so one document field flips
/// the pane between "the item draws" and "the shell draws instead".
struct Rail;
impl Item<Doc> for Rail {
    fn item_id(&self) -> ItemId {
        RAIL
    }
    fn empty_state(&self, doc: &Doc) -> Option<EmptyState> {
        doc.rows.is_empty().then(|| {
            EmptyState::new(
                Icon("inspector"),
                "Nothing selected",
                "Pick a row to see what it is made of",
            )
            // A way in, so this fixture can answer the question
            // `two_pane_chromes_in_one_frame_both_record_their_affordances`
            // asks: an empty state with no affordance records no rect, and a
            // record that is never made cannot be shown to survive.
            .with_next(brightfield_workbench::Affordance::new(
                "Pick a row",
                toggle(),
            ))
        })
    }
    fn describe(&self, _doc: &Doc) -> Subject {
        Subject::new("Detail", Icon("inspector"), BindingContext::Workspace)
    }
    fn ui(&mut self, doc: &mut Doc, _ui: &mut egui::Ui, _cx: &mut ItemCtx<'_>) {
        doc.draws += 1;
        doc.drew.insert(RAIL);
    }
}

fn registry() -> ItemRegistry<Doc> {
    ItemRegistry::new(vec![
        ItemSpec {
            id: CANVAS,
            slot: Slot::Centre,
            toggle: None,
            make: || Box::new(Canvas),
        },
        ItemSpec {
            id: NOTES,
            slot: Slot::CentreTab,
            toggle: Some(toggle()),
            make: || Box::new(Notes),
        },
        ItemSpec {
            id: RAIL,
            slot: Slot::Rail {
                side: DockSide::Right,
                share: 0.25,
            },
            toggle: Some(toggle()),
            make: || Box::new(Rail),
        },
    ])
}

fn workspace() -> Workspace {
    Workspace::new(registry().default_tree())
}

fn key(item: ItemId) -> PaneKey {
    PaneKey::new(item)
}

/// Put a container with exactly one child into the tree, so it has something
/// `SimplificationOptions`' defaults would rewrite.
///
/// `prune_single_child_containers` is on by default, and `Tree::ui` simplifies
/// before it draws. Without a shape like this in the fixture, "simplification
/// is off" is asserted against a tree that cannot exhibit the failure it is
/// meant to catch.
fn wrap_the_rail_in_a_single_child_container(ws: &mut Workspace) {
    let tree = ws.tree_mut();
    let root = tree.root().expect("a root");
    let rail = match tree.tiles.get(root) {
        Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) => lin.children[1],
        _ => panic!("the fixture's root is a linear container"),
    };
    let wrapper = tree.tiles.insert_vertical_tile(vec![rail]);
    // The shares are fixed up here rather than left to the layout pass: a
    // linear container fills in a missing `Shares` entry and drops a stale one
    // the first time it lays out, and both *are* changes to the tree. A
    // fixture that skipped this would fail the assertion below for a reason
    // that has nothing to do with simplification.
    if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) =
        tree.tiles.get_mut(wrapper)
    {
        lin.shares.set_share(rail, 1.0);
    }
    match tree.tiles.get_mut(root) {
        Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) => {
            lin.children[1] = wrapper;
            lin.shares.replace_with(rail, wrapper);
        }
        _ => unreachable!("checked immediately above"),
    }
}

const SCREEN: egui::Rect = egui::Rect {
    min: egui::pos2(0.0, 0.0),
    max: egui::pos2(900.0, 600.0),
};

/// Draw the active view of `ws` for one real frame, and hand back the
/// requests the panes raised plus the tessellated output.
///
/// The `Context` is a parameter rather than built here so a caller can run
/// *consecutive* frames. A fresh `egui::Context` per call is a fresh
/// `egui::Memory`, so a loop over it is N first frames, not N frames — and a
/// first frame is exactly the one where nothing has settled, which is the
/// opposite of what a "drawing does not disturb the tree" claim needs.
fn draw(
    ctx: &egui::Context,
    ws: &mut Workspace,
    doc: &mut Doc,
    items: &mut ItemMap<Doc>,
    input: egui::RawInput,
) -> (Vec<Request>, Vec<egui::ClippedPrimitive>) {
    let mut requests = Vec::new();
    let mut affordances = Vec::new();
    let tabbed = ws.tabbed_tiles();
    let focused = ws.focus();
    let input = egui::RawInput {
        screen_rect: Some(SCREEN),
        ..input
    };
    let mut once = Some(());
    let full = ctx.run_ui(input, |ui| {
        if once.take().is_some() {
            let mut behavior = PaneChrome::new(
                &mut *doc,
                &mut *items,
                Mode::Light,
                focused,
                &tabbed,
                &mut requests,
                &mut affordances,
            );
            ws.tree_mut().ui(&mut behavior, ui);
        }
    });
    let primitives = ctx.tessellate(full.shapes, full.pixels_per_point);
    (requests, primitives)
}

// ---------------------------------------------------------------------------
// The workspace
// ---------------------------------------------------------------------------

/// A pane's address is its item and nothing else, on disk as well as in
/// memory.
///
/// Would catch: a `PaneKey` that grew a second field again, or lost
/// `#[serde(transparent)]` — either of which puts a `{"…": …}` object back in
/// the layout file where a bare item name belongs, and starts the file naming
/// a concept the window does not have. Read off `serde_json` rather than off
/// the type, because the claim is about the bytes.
#[test]
fn a_pane_in_the_layout_file_is_named_by_its_item_and_nothing_else() {
    let json = serde_json::to_string(&key(RAIL)).expect("a pane key serialises");
    assert_eq!(
        json, "\"fixture-rail\"",
        "a pane serialised as {json}, not as its item's own name"
    );
    assert_eq!(
        json,
        serde_json::to_string(&RAIL).expect("an item id serialises"),
        "a pane and its item disagree about what they are called on disk"
    );
}

/// One window, one tree: every pane the window has is in it, and there is no
/// second arrangement for a pane to be hiding in.
///
/// Would catch: a `Workspace` that kept a second tree behind an accessor, or
/// a `window_tree` that dropped a placement it did not recognise — both of
/// which leave a region drawing nothing, which looks exactly like a pane that
/// has nothing to say.
#[test]
fn the_window_holds_one_tree_over_every_pane_it_declares() {
    let ws = workspace();
    let mut declared: Vec<PaneKey> = registry().ids().into_iter().map(PaneKey::new).collect();
    declared.sort_unstable();
    assert_eq!(ws.panes(), declared);
    for item in [CANVAS, NOTES, RAIL] {
        assert!(
            ws.tile_of(key(item)).is_some(),
            "{item} is declared and has no tile, so its region draws nothing"
        );
    }
}

/// Would catch: a focus request applied without checking the pane is still
/// there, which parks the window chrome on a `Subject` nobody can see; and a
/// `clear_focus` that clears something other than the record it is asked for.
#[test]
fn focus_is_one_record_and_a_pane_that_is_not_there_cannot_take_it() {
    let mut ws = workspace();
    assert_eq!(ws.focus(), None, "a fresh workspace focuses nothing");

    assert!(ws.set_focus(key(CANVAS)));
    assert_eq!(ws.focus(), Some(key(CANVAS)));
    assert!(ws.set_focus(key(RAIL)));
    assert_eq!(
        ws.focus(),
        Some(key(RAIL)),
        "a second focus move left the first record standing, so two panes \
         would wear the ring"
    );

    // A pane the tree does not hold is refused, and refusal changes nothing.
    let ghost = PaneKey::new(ItemId::new("fixture-ghost"));
    assert!(!ws.set_focus(ghost));
    assert_eq!(ws.focus(), Some(key(RAIL)));

    ws.clear_focus();
    assert_eq!(ws.focus(), None);
}

/// What a load has to check, because a constructor cannot.
///
/// Would catch: `panes_missing_from` comparing the wrong way round, or
/// answering against its own panes — either of which reports a short file as
/// complete, and `persist::from_json` then restores an arrangement with a
/// region that draws nothing and says nothing about why.
#[test]
fn a_workspace_short_a_pane_says_which_one() {
    let full = workspace();
    let short = Workspace::new(brightfield_workbench::window_tree(&[
        (CANVAS, Slot::Centre),
        (NOTES, Slot::CentreTab),
    ]));
    assert_eq!(
        short.panes_missing_from(&full),
        vec![key(RAIL)],
        "a tree without the rail did not report the rail missing"
    );
    assert!(
        full.panes_missing_from(&short).is_empty(),
        "a tree with everything the donor has reported something missing"
    );
    assert!(full.panes_missing_from(&full).is_empty());
}

/// The header de-duplication rule's input.
///
/// Would catch: `tabbed_tiles` returning the tab *container* ids rather than
/// their children — which type-checks, is one word away from correct, and
/// gives every tabbed pane both a tab title and a header band saying the same
/// thing, the exact drift the chrome file exists to end.
#[test]
fn the_tabbed_set_is_the_children_of_tab_containers_and_nothing_else() {
    let ws = workspace();
    let tabbed = ws.tabbed_tiles();
    let tree = ws.tree();

    let mut expected = std::collections::HashSet::new();
    for (id, tile) in tree.tiles.iter() {
        if let egui_tiles::Tile::Container(egui_tiles::Container::Tabs(_)) = tile {
            assert!(
                !tabbed.contains(id),
                "the tab container itself was reported as tabbed"
            );
        }
    }
    // The registry puts the centre pane and the centre tab in one strip, and
    // the rail outside it.
    for item in [CANVAS, NOTES] {
        let tile = ws.tile_of(key(item)).expect("a tile");
        expected.insert(tile);
    }
    assert_eq!(tabbed, expected);
    let rail = ws.tile_of(key(RAIL)).expect("a tile");
    assert!(!tabbed.contains(&rail), "the rail is not in a tab strip");
}

// ---------------------------------------------------------------------------
// The behaviour
// ---------------------------------------------------------------------------

/// A title for a pane that is *not* drawing, read live from the document.
///
/// Would catch: a title cached at construction, or one derived from the
/// `PaneKey` rather than the `Subject` — both of which stop tracking the
/// document and would show a stale name on every tab but the active one.
///
/// It does **not** demonstrate anything about simultaneous borrows, and the
/// version of this comment that claimed it did was wrong twice over: the two
/// `PaneChrome`s below are built one after the other, and `egui_tiles` calls
/// `tab_title_for_pane` before `pane_ui` rather than during it, so nothing in
/// this crate ever asks for a title mid-draw. The borrow the design actually
/// buys is inside `pane_ui` and is a compile-time fact, not a testable one —
/// see the module docs on `behavior.rs`.
#[test]
fn a_tab_title_is_read_from_the_document_for_a_pane_that_is_not_drawing() {
    let ws = workspace();
    let mut doc = Doc {
        title: "Revenue by quarter".into(),
        ..Doc::default()
    };
    let mut items = registry().instantiate();
    let tabbed = ws.tabbed_tiles();
    let mut requests = Vec::new();
    let mut affordances = Vec::new();
    let mut behavior = PaneChrome::new(
        &mut doc,
        &mut items,
        Mode::Light,
        None,
        &tabbed,
        &mut requests,
        &mut affordances,
    );

    use egui_tiles::Behavior as _;
    let canvas = key(CANVAS);
    assert_eq!(
        behavior.tab_title_for_pane(&canvas).text(),
        "Revenue by quarter"
    );

    // Change the document, not the item: the title must follow. The first
    // behaviour's borrow of the document ends at its last use above, which is
    // the borrow discipline this design buys.
    doc.title = "Revenue by region".into();
    let mut behavior = PaneChrome::new(
        &mut doc,
        &mut items,
        Mode::Light,
        None,
        &tabbed,
        &mut requests,
        &mut affordances,
    );
    assert_eq!(
        behavior.tab_title_for_pane(&canvas).text(),
        "Revenue by region"
    );

    // A pane no item was registered for is named rather than blank.
    let ghost = PaneKey::new(ItemId::new("fixture-ghost"));
    assert_eq!(
        behavior.tab_title_for_pane(&ghost).text(),
        ghost.to_string()
    );
}

/// Would catch: an inverted `header` argument — `tabbed.contains(&tile)`
/// instead of `!tabbed.contains(&tile)`. Inverted, every pane in a strip
/// grows a second title and every pane outside one loses its only one, and
/// nothing fails to compile.
#[test]
fn a_pane_under_a_tab_strip_gets_no_header_band() {
    let header_ink = chrome::colour(meridian_design::semantic(false).surfaces.header);
    let raised = chrome::colour(meridian_design::semantic(false).surfaces.raised);
    assert_ne!(
        header_ink, raised,
        "the header band and the pane body share an ink, so this test cannot see the band"
    );

    let ws = workspace();
    let mut doc = Doc {
        title: "Chart".into(),
        rows: vec!["a"],
        ..Doc::default()
    };
    let mut items = registry().instantiate();
    let tabbed = ws.tabbed_tiles();
    let canvas_tile = ws.tile_of(key(CANVAS)).expect("a tile");

    // Drawn as the registry arranges it — the canvas is in the tab strip.
    let mut tree = ws.tree().clone();
    let mut requests = Vec::new();
    let mut affordances = Vec::new();
    let (_, tabbed_pixels) = {
        let ctx = egui::Context::default();
        let mut once = Some(());
        let full = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(SCREEN),
                ..Default::default()
            },
            |ui| {
                if once.take().is_some() {
                    let mut behavior = PaneChrome::new(
                        &mut doc,
                        &mut items,
                        Mode::Light,
                        None,
                        &tabbed,
                        &mut requests,
                        &mut affordances,
                    );
                    tree.ui(&mut behavior, ui);
                }
            },
        );
        ((), ctx.tessellate(full.shapes, full.pixels_per_point))
    };

    // …and again with nothing declared tabbed, so every pane takes a band.
    let empty = std::collections::HashSet::new();
    let mut tree = ws.tree().clone();
    let mut requests = Vec::new();
    let mut affordances = Vec::new();
    let untabbed_pixels = {
        let ctx = egui::Context::default();
        let mut once = Some(());
        let full = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(SCREEN),
                ..Default::default()
            },
            |ui| {
                if once.take().is_some() {
                    let mut behavior = PaneChrome::new(
                        &mut doc,
                        &mut items,
                        Mode::Light,
                        None,
                        &empty,
                        &mut requests,
                        &mut affordances,
                    );
                    tree.ui(&mut behavior, ui);
                }
            },
        );
        ctx.tessellate(full.shapes, full.pixels_per_point)
    };

    assert!(
        tabbed.contains(&canvas_tile),
        "the fixture's centre pane is supposed to be in a tab strip"
    );
    // The rail is outside the strip either way, so header ink is present in
    // both frames; what changes is the band over the tabbed panes. Assert on
    // the count of distinct band tops instead of mere presence.
    let bands = |primitives: &[egui::ClippedPrimitive]| -> usize {
        let mut corners: std::collections::BTreeSet<(i32, i32)> = std::collections::BTreeSet::new();
        for p in primitives {
            let egui::epaint::Primitive::Mesh(mesh) = &p.primitive else {
                continue;
            };
            for v in mesh.vertices.iter().filter(|v| v.color == header_ink) {
                corners.insert(((v.pos.x * 4.0) as i32, (v.pos.y * 4.0) as i32));
            }
        }
        corners.len()
    };
    assert!(
        bands(&untabbed_pixels) > bands(&tabbed_pixels),
        "suppressing the tabbed panes' header bands painted no less header ink \
         ({} vs {}) — the header flag is not reaching pane_frame",
        bands(&tabbed_pixels),
        bands(&untabbed_pixels)
    );
}

/// The empty-state contract, as behaviour rather than as data.
///
/// Would catch: `pane_ui` calling `item.ui` unconditionally and leaving the
/// empty state to the item — which is what every pane in the shell this
/// replaces did, and why two of them had no empty state at all.
///
/// Asserted on *which* item drew rather than on how many did. A count going
/// from 1 to 2 is also what a second pane becoming active, or a frame drawing
/// twice, looks like; naming the rail is what makes this a claim about the
/// empty state.
#[test]
fn an_empty_pane_is_drawn_by_the_shell_instead_of_by_the_item() {
    let ctx = egui::Context::default();
    let mut ws = workspace();
    let mut items = registry().instantiate();

    // No rows: the rail declares itself empty, so only the two panes in the
    // tab strip can draw — and only the active one of those.
    let mut doc = Doc {
        title: "Chart".into(),
        rows: Vec::new(),
        ..Doc::default()
    };
    draw(
        &ctx,
        &mut ws,
        &mut doc,
        &mut items,
        egui::RawInput::default(),
    );
    let empty_draws = doc.draws;
    assert!(
        !doc.drew.contains(&RAIL),
        "the rail declared itself empty and its item drew anyway"
    );
    assert!(
        doc.drew.contains(&CANVAS),
        "no pane drew at all, so the rail's silence proves nothing"
    );

    // Rows: the rail is no longer empty and its item draws too.
    doc.rows.push("a row");
    doc.draws = 0;
    doc.drew.clear();
    draw(
        &ctx,
        &mut ws,
        &mut doc,
        &mut items,
        egui::RawInput::default(),
    );
    assert!(
        doc.drew.contains(&RAIL),
        "a pane that stopped being empty did not start drawing (drew {:?})",
        doc.drew
    );
    assert!(
        doc.draws > empty_draws,
        "a pane that stopped being empty did not start drawing ({} then {})",
        empty_draws,
        doc.draws
    );
}

/// Would catch: focus decided at draw time (so the last pane drawn always
/// wins), or a focus move applied inside the tile tree's own borrow rather
/// than queued — the re-entrancy `Request` exists to avoid.
#[test]
fn a_press_in_a_pane_asks_the_workspace_for_focus() {
    let ctx = egui::Context::default();
    let mut ws = workspace();
    let mut items = registry().instantiate();
    let mut doc = Doc {
        title: "Chart".into(),
        rows: vec!["a"],
        ..Doc::default()
    };

    // A frame with no pointer at all raises nothing.
    let (quiet, _) = draw(
        &ctx,
        &mut ws,
        &mut doc,
        &mut items,
        egui::RawInput::default(),
    );
    assert!(
        !quiet.iter().any(|r| matches!(r, Request::Focus(_))),
        "focus was claimed with no pointer in the window: {quiet:?}"
    );

    // A press in the right rail asks for the rail.
    let point = egui::pos2(SCREEN.max.x - 20.0, SCREEN.center().y);
    let input = egui::RawInput {
        events: vec![egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        }],
        ..Default::default()
    };
    let (requests, _) = draw(&ctx, &mut ws, &mut doc, &mut items, input);
    let asked: Vec<PaneKey> = requests
        .iter()
        .filter_map(|r| match r {
            Request::Focus(k) => Some(*k),
            _ => None,
        })
        .collect();
    assert_eq!(
        asked,
        vec![key(RAIL)],
        "a press in the rail did not ask for the rail's focus"
    );

    // And the workspace, not the behaviour, is what actually moves it.
    assert_eq!(ws.focus(), None, "pane_ui moved focus itself");
    for request in requests {
        if let Request::Focus(k) = request {
            assert!(ws.set_focus(k));
        }
    }
    assert_eq!(ws.focus(), Some(key(RAIL)));
}

// ---------------------------------------------------------------------------
// The premise the dirty tracker rests on
// ---------------------------------------------------------------------------

/// The single most load-bearing assumption in `persist`: that a *layout pass*
/// does not itself count as a layout change.
///
/// `egui_tiles` 0.16 offers no change callback, so the signal is `live !=
/// saved`. That is only usable if drawing the tree leaves it equal to itself.
/// It is not obvious that it does — `Tiles` caches per-frame rects, allocates
/// tile ids, and its linear containers touch their shares during layout — and
/// if any of that were compared, the shell would rewrite the layout file on
/// the first frame of every boot, for ever.
///
/// Would catch: turning `SimplificationOptions` back on (it prunes containers
/// mid-draw), widening `Workspace`'s `PartialEq` to include focus, or an
/// `egui_tiles` upgrade that starts comparing transient state.
///
/// The simplification half is only a real claim because of
/// [`wrap_the_rail_in_a_single_child_container`]. The registry's own
/// arrangement — a linear root over a tab strip and a rail — has nothing the
/// default options can touch: no empty container, no single-child container,
/// no nested linear. Drawn against `OFF` and against `SimplificationOptions::
/// default()` it comes out byte-identical, so asserted against that fixture
/// alone this test holds whichever is configured, and the premise the whole
/// `live != saved` signal rests on would be untested.
///
/// Frames also run through **one** `egui::Context`. A fresh context per frame
/// is a fresh `Memory`, so a loop over it is three first frames rather than
/// three consecutive ones, and settling is exactly what the claim is about.
#[test]
fn drawing_the_tree_does_not_by_itself_dirty_the_layout() {
    let ctx = egui::Context::default();
    let mut items = registry().instantiate();
    let mut doc = Doc {
        title: "Chart".into(),
        rows: vec!["a"],
        ..Doc::default()
    };
    let mut layout = SavedLayout::new(workspace());
    wrap_the_rail_in_a_single_child_container(&mut layout.workspace);
    let mut tracker = DirtyTracker::new(layout);
    assert!(!tracker.is_dirty(), "a fresh tracker is not dirty");

    for frame in 0..3 {
        let ws = tracker.workspace_mut();
        draw(&ctx, ws, &mut doc, &mut items, egui::RawInput::default());
        assert!(
            !tracker.is_dirty(),
            "drawing frame {frame} was read as a layout change"
        );
    }

    // Focus is state, but transient state: moving it must not be a write.
    assert!(tracker.workspace_mut().set_focus(key(RAIL)));
    assert!(
        !tracker.is_dirty(),
        "moving focus was read as a layout change, so every click would write the layout file"
    );

    // A real arrangement change, however, must be seen — otherwise the test
    // above passes trivially with a comparison that can never be true.
    let root = tracker.live().workspace.tree().root().expect("a root");
    if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) =
        tracker.workspace_mut().tree_mut().tiles.get_mut(root)
    {
        let rail = lin.children[1];
        lin.shares.set_share(rail, 0.44);
    }
    assert!(tracker.is_dirty(), "dragging a splitter was not noticed");
}

/// Two regions of one window, drawn in one frame through two `PaneChrome`s,
/// both keep the record of where their empty panes put a way in.
///
/// The shell draws a navigator rail, an inspector rail, a ledger rail and a
/// canvas in one frame, each against its own document, so each builds its own
/// `PaneChrome`. `PaneChrome::new` used to clear the affordance record on
/// construction: the last one built wiped what the earlier ones had recorded,
/// and `affordance_rect` then answered `None` for a button on screen. The
/// clear belongs to the frame, so the shell does it once.
///
/// The order is the point. The rail is drawn first and is the one that
/// records; the canvas is drawn second, records nothing, and must not take the
/// rail's record with it.
#[test]
fn two_pane_chromes_in_one_frame_both_record_their_affordances() {
    use egui_tiles::Behavior as _;

    let ws = workspace();
    let mut doc = Doc {
        rows: Vec::new(),
        ..Doc::default()
    };
    let mut items = registry().instantiate();
    let tabbed = ws.tabbed_tiles();
    let rail_key = key(RAIL);
    let canvas_key = key(CANVAS);
    let rail_tile = ws.tile_of(rail_key).expect("the rail has a tile");
    let canvas_tile = ws.tile_of(canvas_key).expect("the canvas has a tile");

    let mut requests = Vec::new();
    let mut affordances = Vec::new();
    let ctx = egui::Context::default();
    let mut once = Some(());
    let _ = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(SCREEN),
            ..Default::default()
        },
        |ui| {
            if once.take().is_none() {
                return;
            }
            let left = egui::Rect::from_min_size(SCREEN.min, egui::vec2(240.0, SCREEN.height()));
            let right = egui::Rect::from_min_max(
                egui::pos2(SCREEN.min.x + 240.0, SCREEN.min.y),
                SCREEN.max,
            );
            {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(left));
                let mut behavior = PaneChrome::new(
                    &mut doc,
                    &mut items,
                    Mode::Light,
                    None,
                    &tabbed,
                    &mut requests,
                    &mut affordances,
                );
                let mut pane = rail_key;
                let _ = behavior.pane_ui(&mut child, rail_tile, &mut pane);
            }
            {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(right));
                let mut behavior = PaneChrome::new(
                    &mut doc,
                    &mut items,
                    Mode::Light,
                    None,
                    &tabbed,
                    &mut requests,
                    &mut affordances,
                );
                let mut pane = canvas_key;
                let _ = behavior.pane_ui(&mut child, canvas_tile, &mut pane);
            }
        },
    );

    let recorded: Vec<PaneKey> = affordances.iter().map(|(k, _)| *k).collect();
    assert!(
        recorded.contains(&rail_key),
        "the rail drew a way in and the behaviour built after it wiped the \
         record: {recorded:?}"
    );
}
