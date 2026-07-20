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

use std::collections::BTreeMap;

use brightfield_keys::BindingContext;
use brightfield_workbench::persist::SavedLayout;
use brightfield_workbench::{
    chrome, DirtyTracker, DockSide, EmptyState, Icon, Item, ItemCtx, ItemId, ItemMap, ItemRegistry,
    ItemSpec, Mode, PaneChrome, PaneKey, Request, Slot, Subject, ViewKind, Workspace,
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
}

fn toggle() -> brightfield_workbench::Verb {
    brightfield_workbench::Verb::new("toggle-focus")
}

struct Canvas;
impl Item<Doc> for Canvas {
    fn item_id(&self) -> ItemId {
        CANVAS
    }
    fn subject(&self, doc: &Doc) -> Subject {
        Subject::new(doc.title.clone(), Icon("chart"), BindingContext::Workspace)
    }
    fn ui(&mut self, doc: &mut Doc, ui: &mut egui::Ui, _cx: &mut ItemCtx<'_>) {
        doc.draws += 1;
        ui.label("canvas");
    }
}

struct Notes;
impl Item<Doc> for Notes {
    fn item_id(&self) -> ItemId {
        NOTES
    }
    fn subject(&self, _doc: &Doc) -> Subject {
        Subject::new("Notes", Icon("note"), BindingContext::Editor)
    }
    fn ui(&mut self, doc: &mut Doc, _ui: &mut egui::Ui, _cx: &mut ItemCtx<'_>) {
        doc.draws += 1;
    }
}

/// Empty exactly when the document has no rows, so one document field flips
/// the pane between "the item draws" and "the shell draws instead".
struct Rail;
impl Item<Doc> for Rail {
    fn item_id(&self) -> ItemId {
        RAIL
    }
    fn subject(&self, doc: &Doc) -> Subject {
        let s = Subject::new("Detail", Icon("inspector"), BindingContext::Workspace);
        if doc.rows.is_empty() {
            s.empty(EmptyState::new(
                Icon("inspector"),
                "Nothing selected",
                "Pick a row to see what it is made of",
            ))
        } else {
            s
        }
    }
    fn ui(&mut self, doc: &mut Doc, _ui: &mut egui::Ui, _cx: &mut ItemCtx<'_>) {
        doc.draws += 1;
    }
}

fn registry(view: ViewKind) -> ItemRegistry<Doc> {
    ItemRegistry::new(
        view,
        vec![
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
        ],
    )
}

fn workspace() -> Workspace {
    let mut trees = BTreeMap::new();
    for view in ViewKind::ALL {
        trees.insert(view, registry(view).default_tree());
    }
    Workspace::new(trees)
}

fn key(view: ViewKind, item: ItemId) -> PaneKey {
    PaneKey::new(view, item)
}

const SCREEN: egui::Rect = egui::Rect {
    min: egui::pos2(0.0, 0.0),
    max: egui::pos2(900.0, 600.0),
};

/// Draw the active view of `ws` for one real frame, and hand back the
/// requests the panes raised plus the tessellated output.
fn draw(
    ws: &mut Workspace,
    doc: &mut Doc,
    items: &mut ItemMap<Doc>,
    input: egui::RawInput,
) -> (Vec<Request>, Vec<egui::ClippedPrimitive>) {
    let ctx = egui::Context::default();
    let mut requests = Vec::new();
    let view = ws.active();
    let tabbed = ws.tabbed_tiles(view);
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
            );
            ws.tree_mut(view).ui(&mut behavior, ui);
        }
    });
    let primitives = ctx.tessellate(full.shapes, full.pixels_per_point);
    (requests, primitives)
}

// ---------------------------------------------------------------------------
// The workspace
// ---------------------------------------------------------------------------

/// The property the whole per-view-tree shape exists to deliver.
///
/// Would catch: a `Workspace` that keeps one tree and swaps its panes on a
/// view switch, or that rebuilds the incoming view's tree from the registry.
/// Both compile, both look right on screen, and both silently discard the
/// arrangement of the view you just left — which is the failure this design
/// is a reaction to.
#[test]
fn switching_views_leaves_the_view_you_left_untouched() {
    let mut ws = workspace();

    // Arrange the Charts view: move a share and focus a pane.
    let charts_root = ws.tree(ViewKind::Charts).root().expect("a root");
    if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) =
        ws.tree_mut(ViewKind::Charts).tiles.get_mut(charts_root)
    {
        let rail = lin.children[1];
        lin.shares.set_share(rail, 0.42);
    }
    assert!(ws.set_focus(key(ViewKind::Charts, RAIL)));
    let arranged = ws.tree(ViewKind::Charts).clone();

    ws.set_active(ViewKind::Protocol);
    assert_eq!(ws.active(), ViewKind::Protocol);
    assert_eq!(
        ws.tree(ViewKind::Charts),
        &arranged,
        "leaving a view changed its tree"
    );
    assert_eq!(
        ws.focus_in(ViewKind::Charts),
        Some(key(ViewKind::Charts, RAIL)),
        "leaving a view forgot where focus was"
    );
    assert_eq!(
        ws.focus(),
        None,
        "the view being entered inherited the other view's focus"
    );

    // …and arranging the view we switched *to* does not reach back.
    assert!(ws.set_focus(key(ViewKind::Protocol, NOTES)));
    ws.set_active(ViewKind::Charts);
    assert_eq!(ws.tree(ViewKind::Charts), &arranged);
    assert_eq!(ws.focus(), Some(key(ViewKind::Charts, RAIL)));
    assert_eq!(
        ws.focus_in(ViewKind::Protocol),
        Some(key(ViewKind::Protocol, NOTES))
    );
}

/// Would catch: a single global `focused: Option<PaneKey>` — the obvious
/// first implementation, which passes every single-view test and loses your
/// cursor the moment there are two views; and a focus request applied without
/// checking the pane is still there, which parks the window chrome on a
/// `Subject` nobody can see.
#[test]
fn focus_is_per_view_and_a_pane_that_is_not_there_cannot_take_it() {
    let mut ws = workspace();
    assert_eq!(ws.focus(), None, "a fresh workspace focuses nothing");

    assert!(ws.set_focus(key(ViewKind::Charts, CANVAS)));
    assert!(ws.set_focus(key(ViewKind::Protocol, RAIL)));
    assert_eq!(ws.focus(), Some(key(ViewKind::Charts, CANVAS)));
    ws.set_active(ViewKind::Protocol);
    assert_eq!(ws.focus(), Some(key(ViewKind::Protocol, RAIL)));

    // A pane the view does not hold is refused, and refusal changes nothing.
    let ghost = PaneKey::new(ViewKind::Protocol, ItemId::new("fixture-ghost"));
    assert!(!ws.set_focus(ghost));
    assert_eq!(ws.focus(), Some(key(ViewKind::Protocol, RAIL)));

    ws.clear_focus(ViewKind::Protocol);
    assert_eq!(ws.focus(), None);
    assert_eq!(
        ws.focus_in(ViewKind::Charts),
        Some(key(ViewKind::Charts, CANVAS)),
        "clearing one view's focus cleared another's"
    );
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
    let tabbed = ws.tabbed_tiles(ViewKind::Charts);
    let tree = ws.tree(ViewKind::Charts);

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
        let tile = ws.tile_of(key(ViewKind::Charts, item)).expect("a tile");
        expected.insert(tile);
    }
    assert_eq!(tabbed, expected);
    let rail = ws.tile_of(key(ViewKind::Charts, RAIL)).expect("a tile");
    assert!(!tabbed.contains(&rail), "the rail is not in a tab strip");
}

// ---------------------------------------------------------------------------
// The behaviour
// ---------------------------------------------------------------------------

/// The case the no-document-handle design exists to permit: a title for a
/// pane that is *not* drawing, read live from the document.
///
/// Would catch: a title cached at construction, or one derived from the
/// `PaneKey` rather than the `Subject` — both of which stop tracking the
/// document and would show a stale name on every tab but the active one.
/// That it compiles at all is the other half of the proof: `PaneChrome` holds
/// `&mut D` and `&mut ItemMap<D>` at once, which is what an item holding its
/// own document handle could not have done.
#[test]
fn a_tab_title_is_read_from_the_document_for_a_pane_that_is_not_drawing() {
    let ws = workspace();
    let mut doc = Doc {
        title: "Revenue by quarter".into(),
        ..Doc::default()
    };
    let mut items = registry(ViewKind::Charts).instantiate();
    let tabbed = ws.tabbed_tiles(ViewKind::Charts);
    let mut requests = Vec::new();
    let mut behavior = PaneChrome::new(
        &mut doc,
        &mut items,
        Mode::Light,
        None,
        &tabbed,
        &mut requests,
    );

    use egui_tiles::Behavior as _;
    let canvas = key(ViewKind::Charts, CANVAS);
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
    );
    assert_eq!(
        behavior.tab_title_for_pane(&canvas).text(),
        "Revenue by region"
    );

    // A pane no item was registered for is named rather than blank.
    let ghost = PaneKey::new(ViewKind::Charts, ItemId::new("fixture-ghost"));
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
        draws: 0,
    };
    let mut items = registry(ViewKind::Charts).instantiate();
    let tabbed = ws.tabbed_tiles(ViewKind::Charts);
    let canvas_tile = ws.tile_of(key(ViewKind::Charts, CANVAS)).expect("a tile");

    // Drawn as the registry arranges it — the canvas is in the tab strip.
    let mut tree = ws.tree(ViewKind::Charts).clone();
    let mut requests = Vec::new();
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
                    );
                    tree.ui(&mut behavior, ui);
                }
            },
        );
        ((), ctx.tessellate(full.shapes, full.pixels_per_point))
    };

    // …and again with nothing declared tabbed, so every pane takes a band.
    let empty = std::collections::HashSet::new();
    let mut tree = ws.tree(ViewKind::Charts).clone();
    let mut requests = Vec::new();
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
#[test]
fn an_empty_pane_is_drawn_by_the_shell_instead_of_by_the_item() {
    let mut ws = workspace();
    let mut items = registry(ViewKind::Charts).instantiate();

    // No rows: the rail declares itself empty, so only the two panes in the
    // tab strip can draw — and only the active one of those.
    let mut doc = Doc {
        title: "Chart".into(),
        rows: Vec::new(),
        draws: 0,
    };
    draw(&mut ws, &mut doc, &mut items, egui::RawInput::default());
    let empty_draws = doc.draws;

    // Rows: the rail is no longer empty and its item draws too.
    doc.rows.push("a row");
    doc.draws = 0;
    draw(&mut ws, &mut doc, &mut items, egui::RawInput::default());
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
    let mut ws = workspace();
    let mut items = registry(ViewKind::Charts).instantiate();
    let mut doc = Doc {
        title: "Chart".into(),
        rows: vec!["a"],
        draws: 0,
    };

    // A frame with no pointer at all raises nothing.
    let (quiet, _) = draw(&mut ws, &mut doc, &mut items, egui::RawInput::default());
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
    let (requests, _) = draw(&mut ws, &mut doc, &mut items, input);
    let asked: Vec<PaneKey> = requests
        .iter()
        .filter_map(|r| match r {
            Request::Focus(k) => Some(*k),
            _ => None,
        })
        .collect();
    assert_eq!(
        asked,
        vec![key(ViewKind::Charts, RAIL)],
        "a press in the rail did not ask for the rail's focus"
    );

    // And the workspace, not the behaviour, is what actually moves it.
    assert_eq!(ws.focus(), None, "pane_ui moved focus itself");
    for request in requests {
        if let Request::Focus(k) = request {
            assert!(ws.set_focus(k));
        }
    }
    assert_eq!(ws.focus(), Some(key(ViewKind::Charts, RAIL)));
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
#[test]
fn drawing_the_tree_does_not_by_itself_dirty_the_layout() {
    let mut items = registry(ViewKind::Charts).instantiate();
    let mut doc = Doc {
        title: "Chart".into(),
        rows: vec!["a"],
        draws: 0,
    };
    let mut tracker = DirtyTracker::new(SavedLayout::new(workspace()));
    assert!(!tracker.is_dirty(), "a fresh tracker is not dirty");

    for _ in 0..3 {
        let ws = tracker.workspace_mut();
        draw(ws, &mut doc, &mut items, egui::RawInput::default());
        assert!(
            !tracker.is_dirty(),
            "drawing a frame was read as a layout change"
        );
    }

    // Focus is state, but transient state: moving it must not be a write.
    assert!(tracker
        .workspace_mut()
        .set_focus(key(ViewKind::Charts, RAIL)));
    assert!(
        !tracker.is_dirty(),
        "moving focus was read as a layout change, so every click would write the layout file"
    );

    // A real arrangement change, however, must be seen — otherwise the test
    // above passes trivially with a comparison that can never be true.
    let root = tracker
        .live()
        .workspace
        .tree(ViewKind::Charts)
        .root()
        .expect("a root");
    if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) = tracker
        .workspace_mut()
        .tree_mut(ViewKind::Charts)
        .tiles
        .get_mut(root)
    {
        let rail = lin.children[1];
        lin.shares.set_share(rail, 0.44);
    }
    assert!(tracker.is_dirty(), "dragging a splitter was not noticed");

    // As must switching views, which is part of the saved arrangement.
    let mut tracker = DirtyTracker::new(SavedLayout::new(workspace()));
    tracker.workspace_mut().set_active(ViewKind::Protocol);
    assert!(tracker.is_dirty(), "the active view is not tracked");
}
