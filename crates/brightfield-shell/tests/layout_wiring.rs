//! The layout file, wired to a real frame.
//!
//! `brightfield-workbench`'s `layout_file.rs` covers the machinery: the
//! envelope, every way a load can fail, the debounce, the failed-write
//! back-off. None of it can cover the wiring, because the wiring is the shell
//! driving that machinery from inside `MeridianApp::draw` — and the two claims
//! that matter are claims about what a **real laid-out frame** does:
//!
//! - a frame in which nobody touched anything must leave the layout clean, or
//!   the debounce fires every ten seconds forever and the file is rewritten
//!   for the rest of the session;
//! - a frame in which something moved must actually reach the disk.
//!
//! Both were argued statically before this file existed — `egui_tiles`' own
//! `PartialEq` skips the per-frame rects, `Linear::layout` only garbage-
//! collects shares — and neither had ever been run through `PaneChrome`,
//! `set_active_tab`'s per-frame `tabs.set_active`, or `sweep`.
//!
//! Its own binary rather than a module of `startup.rs`: that file's one test
//! asserts the item vocabulary is empty on entry, and anything here that built
//! a window first would void it. Ordering-dependent tests are a way of
//! learning nothing slowly.

use brightfield_protocol::layout::Flow;
use brightfield_shell::app::CHART;
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::{CANVAS, OUTLINE, STEPS};
use brightfield_shell::startup::{default_layout, kept_window_geometry, opening_boot};
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::persist::{self, LoadOutcome, LAYOUT_FILE, SAVE_DEBOUNCE_MS};
use brightfield_workbench::workspace::{tabs_holding, tile_of};
use brightfield_workbench::{PaneKey, RunState, RECENTS_KEPT};

/// A scratch directory that removes itself, so a failing run cannot poison the
/// next one with a file it left behind.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "brightfield-shell-{name}-{}-{}",
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

/// The harness window, at a size that is **not** `WindowGeometry::default()`.
///
/// Deliberately: while it was `1280×820`, so was the production default, and
/// `assert_eq!(restored.window.size, [SCREEN.max.x, SCREEN.max.y])` was
/// `f(x) == f(x)` — it held whether `observe_window` ran or had never been
/// written. Every size in this file is now distinct from every other, so an
/// equality between two of them means something.
const SCREEN: egui::Rect = egui::Rect {
    min: egui::Pos2::ZERO,
    max: egui::pos2(1100.0, 764.0),
};

/// The default arrangement, already sitting at [`SCREEN`]'s size.
///
/// A window's geometry is fed to the tracker on every frame, so a layout that
/// starts at a *different* size than the harness draws at is dirty from the
/// first frame — correctly, and uninterestingly, since nothing a user did
/// caused it. Tests that ask what a *quiet* frame does start from here, so the
/// geometry feed is settled before the question is put.
fn seated_layout() -> brightfield_workbench::persist::SavedLayout {
    let mut layout = default_layout();
    layout.window.size = [SCREEN.max.x, SCREEN.max.y];
    layout
}

/// The share `a_saved_share_cannot_move_a_declared_rail` puts in the layout
/// file for the protocol view's outline rail, against the `0.24` its registry
/// declares. Far enough out that a canvas pane laid out from it could not be
/// mistaken for one laid out from the declared extent.
///
/// This test used to drag the *chart* view's controls rail. That rail no
/// longer lays the dock out at all: its pane now draws as a real
/// `Panel::right` beside the dock, at a declared pixel width, so widening
/// its old tile share has nothing left to reach. The outline rail is
/// architecturally the same thing this one used to be — a `Slot::Rail` a
/// user resizes by dragging — so it stands in for the general claim this
/// test makes, which was never about the chart view specifically.
const WIDENED_RAIL: f32 = 0.6;

/// The share the outline rail holds in `layout`.
fn rail_share(layout: &brightfield_workbench::persist::SavedLayout) -> f32 {
    let rail = layout
        .workspace
        .tile_of(PaneKey::new(OUTLINE))
        .expect("the window has an outline rail");
    let tree = layout.workspace.tree();
    let root = tree.root().expect("the window's tree has a root");
    match tree.tiles.get(root) {
        Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) => lin.shares[rail],
        _ => panic!("the window's root is a linear container"),
    }
}

/// Widen the outline rail — the arrangement a user makes by dragging one
/// splitter, and the smallest change that is unmistakably theirs.
fn set_rail_share(layout: &mut brightfield_workbench::persist::SavedLayout, share: f32) {
    let rail = layout
        .workspace
        .tile_of(PaneKey::new(OUTLINE))
        .expect("the window has an outline rail");
    let tree = layout.workspace.tree_mut();
    let root = tree.root().expect("the window's tree has a root");
    match tree.tiles.get_mut(root) {
        Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) => {
            lin.shares.set_share(rail, share);
        }
        _ => panic!("the window's root is a linear container"),
    }
}

/// The pane the window's centre strip has in front, or `None` if the strip has
/// no active tab.
///
/// Read straight out of the serialised tree rather than off the model, because
/// the strip's `active` is the thing that is *in the file* — the model's sheet
/// flag is not persisted at all.
fn front_tab(layout: &brightfield_workbench::persist::SavedLayout) -> Option<PaneKey> {
    let tree = layout.workspace.tree();
    let canvas = tile_of(tree, PaneKey::new(CANVAS))?;
    let tabs_id = tabs_holding(tree, canvas)?;
    match tree.tiles.get(tabs_id) {
        Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))) => {
            match tree.tiles.get(tabs.active?) {
                Some(egui_tiles::Tile::Pane(key)) => Some(*key),
                _ => None,
            }
        }
        _ => panic!("the canvas pane sits in a tab strip"),
    }
}

/// Bring `pane` to the front of the window's centre strip — what a user does
/// by clicking the Steps tab, or by pressing `shift-S`.
fn set_front_tab(layout: &mut brightfield_workbench::persist::SavedLayout, pane: PaneKey) {
    let tree = layout.workspace.tree_mut();
    let canvas = tile_of(tree, PaneKey::new(CANVAS)).expect("the canvas pane exists");
    let want = tile_of(tree, pane).unwrap_or_else(|| panic!("{pane} exists"));
    let tabs_id = tabs_holding(tree, canvas).expect("the canvas sits in a tab strip");
    match tree.tiles.get_mut(tabs_id) {
        Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))) => {
            tabs.set_active(want);
        }
        _ => panic!("the canvas pane sits in a tab strip"),
    }
}

/// Run one frame per entry, feeding that entry's events and then handing the
/// window's own geometry back in exactly as the live host does.
fn run(app: &mut MeridianApp, ctx: &egui::Context, frames: Vec<Vec<egui::Event>>) {
    for events in frames {
        let raw = egui::RawInput {
            screen_rect: Some(SCREEN),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| app.draw(ui));
        app.observe_window(ctx);
    }
}

/// `frames` frames of nothing happening.
fn settle(app: &mut MeridianApp, ctx: &egui::Context, frames: usize) {
    run(app, ctx, vec![Vec::new(); frames]);
}

/// One frame's worth of a pointer move and a primary click at `pos`.
fn click_at(pos: egui::Pos2) -> Vec<egui::Event> {
    let mut events = vec![egui::Event::PointerMoved(pos)];
    for pressed in [true, false] {
        events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
    }
    events
}

/// Frames in which nobody rearranged anything leave the layout exactly as it
/// was on disk — including frames in which the user clicked.
///
/// The failure is a write every ten seconds for the rest of the session, and
/// the way into it here is **clicking**: a press anywhere in a pane raises
/// `Request::Focus`, and focus is excluded from `Workspace`'s equality
/// precisely so that moving it is not a layout change — put it back in and
/// every click rewrites the file.
///
/// **Not** the protocol view's tab strip, which this once claimed. It is true
/// that `set_active_tab` writes to the tile tree on every frame that view is
/// drawn, and true that the frames below draw it — but they draw it from
/// [`seated_layout`], where Canvas is already the strip's active tab, so the
/// only value this fixture can watch `set_active_tab` write is the one that
/// was already there. That is the one input the mechanism cannot get wrong.
/// A saved layout with the Steps sheet in front is a reachable arrangement it
/// *did* get wrong, and `a_restored_steps_tab_survives_the_first_frame` is
/// what holds that.
///
/// **Not** the geometry feed, which this once claimed. `observe_window` hands
/// the tracker the same size on every frame of a window nobody resized, and
/// these windows start at that size ([`seated_layout`]), so a feed that
/// recorded nothing at all would look identical from here. What a broken feed
/// does is pinned by `a_layout_that_moved_is_written_and_reads_back`; the
/// sub-point jitter a live window really produces is pinned by
/// `brightfield-workbench`'s `layout_file.rs`, against `WindowGeometry`'s
/// rounded equality.
///
/// The last block restates the quiet claim through `poll` rather than
/// `flush`, on a window that was never handed a change: a clean tracker that
/// is polled twice, once before the debounce and once past it, must still not
/// have created the file.
///
/// It is **not** evidence that a headless window cannot write — this test
/// writes one twenty lines above, `poll_layout` and `flush_layout` are `pub`
/// and take any `&Path`, and starting that block from `default_layout()`
/// instead of [`seated_layout`] makes it write a real file (the geometry feed
/// hands it a size the fixture was not seeded at, so it is legitimately
/// dirty). What actually keeps `cargo test` and the PNG capture tier off the
/// developer's real `workspace-layout.json` is that nothing but `main` calls
/// `startup::layout_path` — a property of the call graph, checkable with
/// `git grep -n 'layout_path()' -- crates/`, and stated where it lives rather
/// than claimed from here.
///
/// Watched redden, one mutation: putting `focus` back into `Workspace`'s
/// hand-written `PartialEq` fails here at "a click changed the layout".
///
/// Watched **stay green**, and therefore not claimed: returning
/// `SimplificationOptions::default()` from `PaneChrome::simplification_options`
/// changes nothing here. Both views' declared trees are already in the shape
/// simplification would leave them in, so this test cannot see that option's
/// value — the reason to keep it `OFF` is the one written on it, not this.
#[test]
fn frames_nobody_rearranged_write_nothing() {
    let scratch = Scratch::new("quiet");
    let path = scratch.file();
    let ctx = egui::Context::default();
    let mut app = MeridianApp::headless_with_layout(Boot::empty(), seated_layout(), Mode::Light);

    settle(&mut app, &ctx, 4);
    assert!(
        app.flush_layout(&path).is_none(),
        "a frame nobody touched changed the layout, so the file is rewritten \
         every debounce window for the rest of the session"
    );

    // A click in the controls rail — empty, and offering no button, so this
    // reaches nothing but the focus machinery.
    let rail = egui::pos2(SCREEN.max.x - 40.0, SCREEN.max.y / 2.0);
    run(&mut app, &ctx, vec![click_at(rail), Vec::new()]);
    assert!(
        app.flush_layout(&path).is_none(),
        "a click changed the layout, so every click a user makes rewrites the \
         layout file for the rest of the session"
    );

    // And a window nothing ever changed writes nothing when it is *polled*
    // either — not only when it is flushed. The debounce is the path the live
    // host takes on every frame, so a tracker that armed on a quiet window
    // would rewrite the file every ten seconds without anybody flushing.
    let mut fresh = MeridianApp::headless_with_layout(Boot::empty(), seated_layout(), Mode::Light);
    let unwritten = scratch.0.join("never-written.json");
    settle(&mut fresh, &ctx, 4);
    assert!(fresh.poll_layout(0, &unwritten).is_none());
    assert!(fresh
        .poll_layout(SAVE_DEBOUNCE_MS + 1, &unwritten)
        .is_none());
    assert!(
        !unwritten.exists(),
        "a window nobody touched wrote its layout on the host's own debounce \
         tick, so every idle session rewrites the file"
    );
}

/// A layout that moved is armed, then written, and reads back as saved.
///
/// The whole round trip through the shell rather than through the tracker:
/// switching views is a real change a user makes, `poll` is called the way the
/// host calls it, and the file is read back through the same `persist::load`
/// the next boot uses.
///
/// `layout_armed` is asserted because the live host has nothing else to ask.
/// eframe paints on input, not continuously, so without a
/// `request_repaint_after` while a save is armed a user who rearranges panes
/// and then leaves the window alone generates no more frames, the countdown
/// never reaches its deadline, and the change survives only as far as
/// `on_exit`.
///
/// Watched redden, one mutation: deleting the
/// `self.layout.live_mut().opened = …` line from `MeridianApp::open_start`
/// fails at "a changed layout armed no countdown" — which is the honest shape
/// of that bug and not the one it was expected to take. Opening a document is
/// not by itself a *layout* change; recording what was opened is the only
/// thing about the second click that reaches the file at all, so without it
/// there is nothing to save and nothing ever arms.
#[test]
fn a_layout_that_moved_is_written_and_reads_back() {
    let scratch = Scratch::new("moved");
    let path = scratch.file();
    let ctx = egui::Context::default();
    // Seated at the harness's own size, so the click below is the *only*
    // change in this window's life and every arming assertion is about it.
    // What the geometry feed does is a separate question, asked separately by
    // `the_window_geometry_the_user_left_is_what_the_next_boot_reads`.
    let mut app = MeridianApp::headless_with_layout(Boot::empty(), seated_layout(), Mode::Light);
    settle(&mut app, &ctx, 3);
    assert!(!app.graph_on_canvas());
    assert!(
        app.poll_layout(SAVE_DEBOUNCE_MS + 1, &path).is_none(),
        "the window was dirty before anyone touched it, so nothing below can \
         tell the click apart from the boot"
    );

    // A change a user makes, made the way a user makes it: the front door's
    // own button, clicked where the last frame recorded it.
    let target = app
        .affordance_rect(PaneKey::new(CHART))
        .expect("the chart view's front door drew a way in");
    run(&mut app, &ctx, vec![click_at(target.center())]);
    settle(&mut app, &ctx, 2);
    assert!(!app.graph_on_canvas());
    assert!(!app.chart_doc().is_empty());

    // Armed, and not yet written: the debounce collapses a burst of changes
    // into one write at the end rather than one per frame.
    assert!(app.poll_layout(0, &path).is_none());
    assert!(
        app.layout_armed(),
        "a changed layout armed no countdown, so nothing will ever write it \
         and the host has nothing to schedule a repaint for"
    );
    assert!(!path.exists(), "the layout was written before the debounce");

    // Due, and written.
    assert!(app.poll_layout(0, &path).is_none(), "still moving");
    let result = app
        .poll_layout(SAVE_DEBOUNCE_MS + 1, &path)
        .expect("the debounce expired, so a write was attempted");
    result.expect("the write succeeded");
    assert!(!app.layout_armed(), "the countdown outlived its write");

    // And the next boot reads back what this one left.
    let (restored, outcome) = persist::load(&path, default_layout);
    assert_eq!(outcome, LoadOutcome::Restored, "{}", outcome.reason());
    assert_eq!(
        restored.opened.as_deref(),
        Some(brightfield_shell::starts::DASHBOARD),
        "the file did not record what was open, so the next launch restores an \
         arrangement around panes that are all still empty"
    );
}

/// The size the window was last at is what the next boot reads back.
///
/// The save half of the geometry round trip, which nothing else can see. It is
/// its own test because it needs three distinct sizes and the other tests need
/// two of them equal: this window is *seeded* at a size that is neither
/// `WindowGeometry::default()` nor [`SCREEN`], drawn at `SCREEN`, and must come
/// back at `SCREEN`. Match any two of those three and the assertion holds
/// without `observe_window` ever running — which is what it did while the
/// harness screen and the production default were both `1280×820`.
///
/// Watched redden, one mutation: replacing the body of
/// `MeridianApp::observe_window` with `let _ = ctx;` so it records nothing
/// fails here at "the window's own geometry never reached the layout" — the
/// flush finds nothing to write, because the seeded size is still what the
/// tracker was handed.
#[test]
fn the_window_geometry_the_user_left_is_what_the_next_boot_reads() {
    let scratch = Scratch::new("geometry");
    let path = scratch.file();
    let ctx = egui::Context::default();

    let mut seeded = default_layout();
    seeded.window.size = [906.0, 588.0];
    let mut app = MeridianApp::headless_with_layout(Boot::empty(), seeded, Mode::Light);
    settle(&mut app, &ctx, 3);

    app.flush_layout(&path)
        .expect(
            "the window's own geometry never reached the layout — it was \
             seeded at 906x588 and drawn at the harness screen size, so a \
             clean tracker means nothing observed the frame",
        )
        .expect("the write succeeded");

    let (restored, outcome) = persist::load(&path, default_layout);
    assert_eq!(outcome, LoadOutcome::Restored, "{}", outcome.reason());
    assert_eq!(
        restored.window.size,
        [SCREEN.max.x, SCREEN.max.y],
        "the next boot opens at a size nothing was ever drawn at"
    );
    // A position that cannot be read is not recorded, rather than being
    // invented: this context reports no outer rect at all.
    assert_eq!(restored.window.position, None);
}

/// A layout file written before `opened` existed still loads.
///
/// The upgrade path, and the one that is silent when it breaks: a field added
/// to `SavedLayout` without `#[serde(default)]` makes every file already on
/// every machine fail to parse, which surfaces as `Corrupt` — a log line that
/// blames the user's file for a change this build made, and everybody's
/// arrangement gone.
///
/// Watched redden, one mutation: a second field added to `SavedLayout` at a
/// non-optional type (`u32`) with `#[serde(default)]` forgotten, stripped from
/// the file below the way `opened` is, fails here with `Corrupt`. Putting the
/// attribute back on that field turns it green again — so this test is holding
/// the attribute, on the types where the attribute is the mechanism.
///
/// Watched **stay green**, and therefore not claimed: removing
/// `#[serde(default)]` from `opened` while it stays an `Option`. serde's derive
/// already reads a missing `Option<T>` field as `None`, so on *that* type the
/// attribute is redundant — see the note on `SavedLayout::opened`, which now
/// says which way round it is.
#[test]
fn a_layout_from_before_this_field_existed_still_loads() {
    let scratch = Scratch::new("upgrade");
    let path = scratch.file();

    // Publish the vocabulary the way a boot does, so the pane keys below
    // resolve. This binary builds windows in its other tests, but nothing here
    // may depend on which ran first.
    let _ = brightfield_shell::startup::boot_layout(None);

    let mut json = serde_json::to_value(default_layout()).expect("a layout serialises");
    let removed = json
        .as_object_mut()
        .expect("the envelope is an object")
        .remove("opened");
    assert!(
        removed.is_some(),
        "there is no `opened` key to remove, so this test proves nothing"
    );
    std::fs::write(&path, serde_json::to_string(&json).expect("writes")).expect("writes");

    let (restored, outcome) = persist::load(&path, default_layout);
    assert_eq!(
        outcome,
        LoadOutcome::Restored,
        "a layout written before `opened` existed no longer loads ({}) — every \
         file on every machine is discarded on upgrade",
        outcome.reason()
    );
    assert_eq!(restored.opened, None);
}

/// A layout file written before `recents` existed still loads — and this one
/// is the case where `#[serde(default)]` is the whole mechanism rather than a
/// redundancy.
///
/// The sibling above says it in the abstract: on an `Option<T>` serde's derive
/// already reads a missing field as `None`, so removing the attribute from
/// `opened` changes nothing any test can see. `recents` is a `Vec`, which is
/// the type the note warns about, so this is that claim held on a field where
/// it bites.
///
/// Watched redden, one mutation: removing `#[serde(default)]` from
/// `SavedLayout::recents` fails here with `Corrupt` — which is a layout file
/// discarded by an upgrade that added a front-door section, on whatever
/// machine it is sitting on.
#[test]
fn a_layout_from_before_recents_existed_still_loads() {
    let scratch = Scratch::new("upgrade-recents");
    let path = scratch.file();
    let _ = brightfield_shell::startup::boot_layout(None);

    let mut json = serde_json::to_value(default_layout()).expect("a layout serialises");
    let removed = json
        .as_object_mut()
        .expect("the envelope is an object")
        .remove("recents");
    assert!(
        removed.is_some(),
        "there is no `recents` key to remove, so this test proves nothing"
    );
    std::fs::write(&path, serde_json::to_string(&json).expect("writes")).expect("writes");

    let (restored, outcome) = persist::load(&path, default_layout);
    assert_eq!(
        outcome,
        LoadOutcome::Restored,
        "a layout written before `recents` existed no longer loads ({}) — \
         every file on every machine is discarded on upgrade",
        outcome.reason()
    );
    assert!(restored.recents.is_empty());
}

/// The recents list is capped, most recent first, and reopening something
/// moves it rather than adding a second row for it.
///
/// Three claims one method owns, so they are held in one place: a door listing
/// the same Protocol twice is a list of events, a door listing twenty is a
/// file browser, and a list that appends rather than prepends puts the thing
/// you were last in at the bottom.
///
/// Watched redden, two mutations, one per claim. Dropping the `retain` from
/// `SavedLayout::remember` fails at "reopening one added a second row for it"
/// (2 against 1); dropping the `truncate` fails at "the list grows without
/// bound" (9 against 6).
#[test]
fn the_recents_list_is_capped_and_most_recent_first() {
    let mut layout = default_layout();
    for i in 0..(RECENTS_KEPT + 3) {
        layout.remember(
            &format!("start-{i}"),
            &format!("protocol {i}"),
            RunState::NeverRun,
            1_000 + i as u64,
        );
    }
    assert_eq!(
        layout.recents.len(),
        RECENTS_KEPT,
        "the list grows without bound, and it is written on every open"
    );
    assert_eq!(
        layout.recents[0].id,
        format!("start-{}", RECENTS_KEPT + 2),
        "the most recently opened is not at the head"
    );
    assert_eq!(
        layout.recents.last().expect("a tail").id,
        format!("start-{}", 3),
        "the entry dropped is not the least recently opened one"
    );

    // Reopening one already in the list moves it and rewrites what was seen,
    // rather than leaving a second, staler row beside it.
    //
    // The one reopened is **mid-list**, and that is not incidental: reopening
    // the entry at the tail is dropped by the truncation whether or not
    // anything de-duplicates, so a list that appends blindly passes a test
    // written against the tail and ships a door showing the same Protocol
    // twice. Measured — with the `retain` removed, this reopen leaves two rows
    // for `start-6` and reopening `start-3` leaves one.
    let middle = format!("start-{}", RECENTS_KEPT);
    assert!(
        layout.recents.iter().any(|r| r.id == middle)
            && layout.recents.last().expect("a tail").id != middle,
        "the fixture stopped putting {middle} in the middle of the list, which \
         is the only position this assertion can see a missing de-duplication \
         from"
    );
    layout.remember(&middle, "renamed", RunState::Fresh, 9_000);
    assert_eq!(
        layout.recents.iter().filter(|r| r.id == middle).count(),
        1,
        "reopening one added a second row for it"
    );
    assert_eq!(layout.recents[0].id, middle);
    assert_eq!(layout.recents[0].name, "renamed");
    assert_eq!(layout.recents[0].run, RunState::Fresh);
}

/// A layout missing a pane is discarded without resizing the window.
///
/// What such a file is: one written before a pane was added. `persist` reports
/// `Incomplete` and hands back the default arrangement, which answers
/// `restored()` `false` — correctly, because something on screen is not what
/// the user arranged.
///
/// The trap is treating that as "there was nothing to restore" and reaching
/// for a content-derived window size. The file carried the size and position
/// the user last left, nothing about those is wrong, and resizing their window
/// because an upgrade added a pane is a change they did not ask for.
///
/// Watched redden, one mutation: defining `kept_window_geometry` as
/// `outcome.restored()` fails here at "an upgrade that added a pane also
/// resized the window".
#[test]
fn a_layout_missing_a_pane_is_discarded_without_resizing_the_window() {
    let scratch = Scratch::new("incomplete");
    let path = scratch.file();
    let _ = brightfield_shell::startup::boot_layout(None);

    // The window's tree with the outline rail's tile taken out — a file
    // written by a build that did not have that pane. Serialised as JSON and
    // edited there, because `Tiles` offers no removal that leaves a valid
    // parent behind and the point is the *bytes* a previous build wrote.
    let mut saved = default_layout();
    saved.window.size = [744.0, 512.0];
    let mut json = serde_json::to_value(&saved).expect("a layout serialises");
    let tiles = json
        .pointer_mut("/workspace/tree/tiles/tiles")
        .and_then(serde_json::Value::as_object_mut)
        .expect("the workspace carries one tree of tiles");
    let rail_tile = tiles
        .iter()
        .find(|(_, tile)| tile["Pane"] == serde_json::json!(OUTLINE.as_str()))
        .map(|(id, _)| id.clone())
        .expect("the outline rail has a tile in the default arrangement");
    tiles.remove(&rail_tile);
    std::fs::write(&path, serde_json::to_string(&json).expect("writes")).expect("writes");

    let (repaired, outcome) = persist::load(&path, default_layout);
    assert_eq!(outcome, LoadOutcome::Incomplete, "{}", outcome.reason());
    assert_eq!(
        repaired.workspace.panes(),
        default_layout().workspace.panes(),
        "the load handed back an arrangement with a region that draws nothing"
    );
    assert!(
        kept_window_geometry(outcome),
        "an upgrade that added a pane also resized the window the user left"
    );
    assert_eq!(repaired.window.size, [744.0, 512.0]);

    // And the other side of it: a file this build could not read carries no
    // geometry worth keeping, so the boot is free to size itself.
    std::fs::write(&path, "{ not json").expect("writes");
    let (_, outcome) = persist::load(&path, default_layout);
    assert_eq!(outcome, LoadOutcome::Corrupt);
    assert!(!kept_window_geometry(outcome));
}

/// The command line outranks what was remembered, and nothing remembered with
/// nothing named opens on nothing.
///
/// The precedence `main` runs on, asserted through the same function `main`
/// calls rather than through a second spelling of it.
///
/// Watched redden, one mutation: having `opening_boot` ignore its `opened`
/// argument — which is all a shell that persisted only the arrangement can do
/// — fails at "a remembered start was not reopened".
#[test]
fn what_the_window_opens_on_prefers_the_command_line_then_what_was_open() {
    let nothing =
        opening_boot(None, None, Flow::Vertical, None).expect("an empty boot cannot fail");
    assert!(
        nothing.is_empty(),
        "a launch with nothing named opened something"
    );

    let remembered = opening_boot(
        None,
        Some(brightfield_shell::starts::CROSSWALK),
        Flow::Vertical,
        None,
    )
    .expect("the remembered start loads");
    assert!(
        !remembered.is_empty(),
        "a remembered start was not reopened, so the front door shows again \
         over the work it was supposed to restore"
    );
    assert!(
        !remembered.protocol.graph_full.nodes.is_empty(),
        "the remembered start came back without its graph"
    );
    assert!(
        remembered.graph_on_canvas(),
        "the restored crosswalk did not take the canvas, so the window comes \
         up showing an empty chart over a 34-node graph"
    );

    // A named spec wins, even with something remembered.
    let named = opening_boot(
        Some("../../examples/dashboard.yaml"),
        Some(brightfield_shell::starts::CROSSWALK),
        Flow::Vertical,
        None,
    )
    .expect("the named spec opens");
    assert!(!named.graph_on_canvas());
    assert!(named.protocol.graph_full.nodes.is_empty());

    // An id from a build that shipped a start this one does not is dropped,
    // not propagated: a stale config line may not stop the window opening.
    let stale = opening_boot(None, Some("a-start-from-the-future"), Flow::Vertical, None)
        .expect("an unrecognised start still opens a window");
    assert!(stale.is_empty());
}

/// A saved tile share does not move a rail the arrangement declares, and the
/// restored tree still survives the boot.
///
/// **This test's claim is inverted from what it used to make, deliberately.**
/// It used to assert that a rail widened in the layout file reached the drawn
/// frame, because the rails were tiles of the dock and their widths were the
/// tree's shares. They are not any more: each rail is a `Panel` at the extent
/// `brightfield_workbench::arrangement` declares, so the shares in a saved
/// file no longer lay anything out — and a test that went on asserting they
/// did would be asserting a mechanism that had been removed.
///
/// What is worth holding is the other direction, and it is a guard rather
/// than a nicety: a layout file is a file on disk that any process may write,
/// and a share of `0.6` for the outline rail must not be able to squash the
/// canvas. The declared extent wins. Wiring shares back into the draw path —
/// which is the change this is the tripwire for — fails the first assertion.
///
/// The half that survives unchanged is the one about `assemble`: the restored
/// tree is carried, not thrown away and rebuilt from the registries. That is
/// asked of `app.layout()` rather than of the frame, because the frame no
/// longer reads a share.
///
/// Watched redden, one mutation: rebuilding `layout.workspace` from the two
/// registries' `default_tree()`s at the top of `MeridianApp::assemble`,
/// carrying `active` across, fails at "drawing reset the rail to its declared
/// share".
#[test]
fn a_saved_share_cannot_move_a_declared_rail() {
    let scratch = Scratch::new("rearranged");
    let path = scratch.file();
    let ctx = egui::Context::default();

    // Publish the vocabulary the way a boot does, so the pane keys in the file
    // below resolve. This binary builds windows in its other tests, but nothing
    // here may depend on which ran first.
    let _ = brightfield_shell::startup::boot_layout(None);

    let mut arranged = seated_layout();
    assert!(
        (rail_share(&arranged) - WIDENED_RAIL).abs() > 0.1,
        "the fixture's rail already has the share this test widens it to, so \
         nothing below distinguishes a restore from a rebuilt default"
    );
    set_rail_share(&mut arranged, WIDENED_RAIL);
    std::fs::write(&path, arranged.to_json().expect("a layout serialises")).expect("writes");

    let (restored, outcome) = persist::load(&path, default_layout);
    assert_eq!(outcome, LoadOutcome::Restored, "{}", outcome.reason());
    assert!(
        (rail_share(&restored) - WIDENED_RAIL).abs() < 1e-4,
        "the widened rail did not survive the file: {}",
        rail_share(&restored)
    );

    // Over a real crosswalk, because an empty canvas pane draws an empty
    // state instead of a DAG and records no content box — there would be no
    // laid-out rect to ask about.
    let crosswalk =
        || Boot::start(brightfield_shell::starts::CROSSWALK, Flow::Vertical).expect("it loads");
    let mut app = MeridianApp::headless_with_layout(crosswalk(), restored, Mode::Light);
    settle(&mut app, &ctx, 2);

    let seated = seated_layout();
    let mut plain = MeridianApp::headless_with_layout(crosswalk(), seated, Mode::Light);
    settle(&mut plain, &ctx, 2);

    let widened = app
        .canvas_viewport()
        .expect("the canvas pane was laid out")
        .width();
    let declared = plain
        .canvas_viewport()
        .expect("the canvas pane was laid out")
        .width();
    assert!(
        (widened - declared).abs() < 1e-3,
        "a saved share of {WIDENED_RAIL} moved the canvas from {declared} to \
         {widened} — the rails are laid out from the declared arrangement, so \
         a number in a file on disk must not be able to squash one"
    );
    let navigator = app
        .region_rect(brightfield_workbench::arrangement::NAVIGATOR_RAIL)
        .expect("the navigator rail drew");
    assert!(
        (navigator.width() - brightfield_workbench::arrangement::NAVIGATOR_RAIL_WIDTH).abs() < 1e-3,
        "the navigator rail drew {}pt wide against the declared {}pt, with a \
         {WIDENED_RAIL} share sitting in the layout file",
        navigator.width(),
        brightfield_workbench::arrangement::NAVIGATOR_RAIL_WIDTH
    );

    // And the frame did not quietly put the declared share back.
    assert!(
        (rail_share(app.layout()) - WIDENED_RAIL).abs() < 1e-4,
        "drawing reset the rail to its declared share"
    );
    assert_eq!(
        app.layout().workspace.panes(),
        plain.layout().workspace.panes(),
        "the rearranged layout lost or gained a pane"
    );
}

/// A restored Steps tab is still in front after the first frame, and drawing
/// that frame did not dirty the layout.
///
/// The arrangement is one keystroke away — `shift-S` opens the steps sheet, and
/// the strip's active tab is part of the serialised tree — so this is what a
/// user who quits with the sheet open comes back to. What it used to be:
/// `MeridianApp::draw` makes the strip authoritative from `ProtocolModel`'s
/// sheet flag on every frame it draws this view, a freshly constructed model
/// boots with the sheet shut, and so Canvas overwrote the restored Steps
/// before any frame could read it. Both halves of that are asserted, because
/// the second is the worse one: the overwrite is a tile-tree mutation, so a
/// launch nobody touched went dirty and the debounce wrote the reverted
/// arrangement back over the file. The restore was lossy *and* it destroyed
/// the evidence.
///
/// The fixture is the one `frames_nobody_rearranged_write_nothing` cannot
/// supply: that test starts from [`seated_layout`], whose strip already has
/// Canvas in front, so the only value it can watch `set_active_tab` write is
/// the one already there.
///
/// Watched redden, one mutation: deleting the `steps_tab_is_active(…)` seeding
/// from the end of `MeridianApp::assemble` fails here at "the restored Steps
/// tab was reverted to Canvas", and — with that assertion removed as well — at
/// "a launch that restored a Steps tab rewrote the layout file".
#[test]
fn a_restored_steps_tab_survives_the_first_frame() {
    let scratch = Scratch::new("steps-tab");
    let path = scratch.file();
    let ctx = egui::Context::default();
    let _ = brightfield_shell::startup::boot_layout(None);

    let steps = PaneKey::new(STEPS);
    let mut saved = seated_layout();
    assert_ne!(
        front_tab(&saved),
        Some(steps),
        "the declared strip already has Steps in front, so nothing below \
         distinguishes a restore from the model's own default"
    );
    set_front_tab(&mut saved, steps);

    // Through the file, so this is the arrangement a real session leaves
    // rather than one only this test can build.
    std::fs::write(&path, saved.to_json().expect("a layout serialises")).expect("writes");
    let (restored, outcome) = persist::load(&path, default_layout);
    assert_eq!(outcome, LoadOutcome::Restored, "{}", outcome.reason());
    assert_eq!(
        front_tab(&restored),
        Some(steps),
        "the Steps tab did not survive the file, so the loss is in `persist` \
         rather than in the shell wiring this test is about"
    );
    std::fs::remove_file(&path).expect("removes");

    let mut app = MeridianApp::headless_with_layout(Boot::empty(), restored, Mode::Light);
    settle(&mut app, &ctx, 4);

    assert_eq!(
        front_tab(app.layout()),
        Some(steps),
        "the restored Steps tab was reverted to Canvas on the first frame, so \
         quitting with the steps sheet open comes back to a shut one"
    );
    assert!(
        app.protocol_model().show_sheet(),
        "the strip says Steps and the model says the sheet is shut, so the \
         next `shift-S` toggles it the wrong way"
    );
    assert!(
        app.flush_layout(&path).is_none(),
        "a launch that restored a Steps tab rewrote the layout file without \
         the user touching anything — the reverted arrangement is now what is \
         on disk"
    );
    assert!(!path.exists());
}

/// A restored session names its window from the surface that will actually be
/// drawn.
///
/// `main` hands `Boot::title` straight to `eframe::run_native`, which is where
/// the OS window's title comes from, and the only `ViewportCommand::Title` in
/// the workspace is inside `MeridianApp::open_start` — a restore never reaches
/// it. So a title that is wrong at this one call is wrong for the whole
/// session, and the same expression answers the boot summary printed to
/// stderr.
///
/// The trap this pins: the boot has to answer for the document the canvas
/// takes, and a restored crosswalk's chart document is `Composed::empty()`.
/// Anything that answers for the chart regardless titles a restored 34-node
/// crosswalk "Brightfield" and logs "composed 0x0 dashboard" for it, which is
/// what a `Boot` carrying a defaultable view opinion used to do.
///
/// Asserted as an *agreement* between the two answers rather than against a
/// literal: `Boot::title` is only meaningful because it is the same question
/// `MeridianApp::title` answers once the window exists, and a literal on both
/// sides would go on agreeing with itself after either drifted.
///
/// Watched redden, one mutation: replacing `graph_takes_the_canvas`'s body
/// with `false` fails here at "the window is titled for a surface it is not
/// drawing".
#[test]
fn a_restored_session_is_titled_for_the_surface_it_draws() {
    let mut saved = default_layout();
    saved.opened = Some(brightfield_shell::starts::CROSSWALK.to_string());

    // Exactly what `main` does, in the order it does it.
    let boot = opening_boot(None, saved.opened.as_deref(), Flow::Vertical, None)
        .expect("the remembered start loads");
    let title = boot.title();
    let described = boot.describe();

    let ctx = egui::Context::default();
    let mut app = MeridianApp::headless_with_layout(boot, saved, Mode::Light);
    settle(&mut app, &ctx, 2);

    assert!(app.graph_on_canvas());
    assert_eq!(
        title,
        app.title(),
        "the window is titled for a surface it is not drawing, and nothing \
         sends a `ViewportCommand::Title` on a restore, so that name survives \
         the session"
    );
    assert!(
        described.starts_with("protocol "),
        "the boot summary describes something that was not loaded: {described}"
    );
}
