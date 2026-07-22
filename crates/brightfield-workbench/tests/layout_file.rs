//! The layout file, through a real file on a real disk.
//!
//! # Why this is one test and not eleven
//!
//! Deserialising a [`PaneKey`] validates its [`ItemId`] against the process's
//! published vocabulary, and that vocabulary is a **process global**
//! ([`ItemId::publish`] documents why it has to be). So every assertion below
//! is order-dependent with respect to every other: "an unpublished id fails to
//! load" is only true *before* the ids are published, and "a saved layout
//! round-trips" is only true after. Split across `#[test]`s they would pass
//! under a thread-per-test runner and fail intermittently under
//! `--test-threads=1`, which is what this repo mandates, and an
//! order-dependent test is a way of learning nothing slowly.
//!
//! So the order is written down instead. Each step says what it would catch.

use std::collections::BTreeMap;

use brightfield_keys::BindingContext;
use brightfield_workbench::persist::{
    self, LoadOutcome, SavedLayout, WindowGeometry, LAYOUT_FILE, LAYOUT_VERSION, SAVE_DEBOUNCE_MS,
};
use brightfield_workbench::{
    DirtyTracker, DockSide, EmptyState, Icon, Item, ItemCtx, ItemId, ItemRegistry, ItemSpec,
    PaneKey, Slot, Subject, ViewKind, Workspace,
};

// ---------------------------------------------------------------------------
// A fixture view, thin enough to be all arrangement and no content
// ---------------------------------------------------------------------------

const CANVAS: ItemId = ItemId::new("persist-canvas");
const RAIL: ItemId = ItemId::new("persist-rail");

#[derive(Default)]
struct Doc;

struct Pane(ItemId, &'static str);
impl Item<Doc> for Pane {
    fn item_id(&self) -> ItemId {
        self.0
    }
    fn empty_state(&self, _doc: &Doc) -> Option<EmptyState> {
        Some(EmptyState::new(
            Icon("pane"),
            "Nothing here yet",
            "This pane fills in once a protocol has run",
        ))
    }
    fn describe(&self, _doc: &Doc) -> Subject {
        Subject::new(self.1, Icon("pane"), BindingContext::Workspace)
    }
    fn ui(&mut self, _doc: &mut Doc, _ui: &mut egui::Ui, _cx: &mut ItemCtx<'_>) {}
}

fn registry(view: ViewKind) -> ItemRegistry<Doc> {
    ItemRegistry::new(
        view,
        vec![
            ItemSpec {
                id: CANVAS,
                slot: Slot::Centre,
                toggle: None,
                make: || Box::new(Pane(CANVAS, "Canvas")),
            },
            ItemSpec {
                id: RAIL,
                slot: Slot::Rail {
                    side: DockSide::Right,
                    share: 0.25,
                },
                toggle: Some(brightfield_workbench::Verb::new("toggle-focus")),
                make: || Box::new(Pane(RAIL, "Rail")),
            },
        ],
    )
}

fn defaults() -> SavedLayout {
    let mut trees = BTreeMap::new();
    for view in ViewKind::ALL {
        trees.insert(view, registry(view).default_tree());
    }
    SavedLayout::new(Workspace::new(trees))
}

/// Give the rail a distinctive share, so "the saved arrangement came back"
/// is a claim about an actual arrangement rather than about a default that
/// would have been rebuilt identically anyway.
fn arrange(layout: &mut SavedLayout, share: f32) {
    let root = layout
        .workspace
        .tree(ViewKind::Charts)
        .root()
        .expect("a root");
    let tiles = &mut layout.workspace.tree_mut(ViewKind::Charts).tiles;
    if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) =
        tiles.get_mut(root)
    {
        let rail = lin.children[1];
        lin.shares.set_share(rail, share);
    }
}

fn share_of(layout: &SavedLayout) -> f32 {
    let tree = layout.workspace.tree(ViewKind::Charts);
    let root = tree.root().expect("a root");
    match tree.tiles.get(root) {
        Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) => {
            lin.shares[lin.children[1]]
        }
        _ => panic!("the fixture's root is a linear container"),
    }
}

/// A scratch directory that removes itself, so a failing run cannot poison
/// the next one with a file it left behind.
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
// The one ordered test
// ---------------------------------------------------------------------------

#[test]
fn the_layout_file_survives_a_restart_and_every_way_it_can_be_broken() {
    let scratch = Scratch::new("layout");
    let path = scratch.file();

    // -----------------------------------------------------------------
    // Before anything is published.
    //
    // Would catch: a `Deserialize` for `ItemId` that accepts any string.
    // With one, an upgrade that renames a pane loads a layout naming a pane
    // this build cannot construct, and the window comes up with a hole in
    // it instead of with the default arrangement.
    // -----------------------------------------------------------------
    assert!(
        ItemId::known().is_empty(),
        "another test in this binary published first — the order below is void"
    );
    let unpublished = defaults();
    let json = unpublished.to_json().expect("a layout serialises");
    let (recovered, outcome) = persist::from_json(Some(&json), defaults);
    assert_eq!(
        outcome,
        LoadOutcome::Corrupt,
        "a layout naming ids this build has not published was accepted"
    );
    assert_eq!(recovered.version, LAYOUT_VERSION);

    // -----------------------------------------------------------------
    // Publish the vocabulary, as boot does — every view, before the file
    // is read.
    // -----------------------------------------------------------------
    for view in ViewKind::ALL {
        ItemId::publish(Box::leak(registry(view).ids().into_boxed_slice()));
    }
    assert!(ItemId::known().contains(&CANVAS));
    assert!(ItemId::known().contains(&RAIL));

    // -----------------------------------------------------------------
    // A real round trip through a real file.
    //
    // Would catch: an `egui_tiles` pulled with `default-features = false`,
    // which silently drops its `serde` feature — its only feature and its
    // default — and takes the whole tree out of the envelope with it. That
    // is exactly what blocked the spike, and an in-memory clone test would
    // not have noticed.
    // -----------------------------------------------------------------
    let mut saved = defaults();
    arrange(&mut saved, 0.37);
    saved.window = WindowGeometry {
        size: [1440.0, 900.0],
        position: Some([12.0, 34.0]),
    };
    saved.workspace.set_active(ViewKind::Protocol);
    saved.save(&path).expect("the layout writes");
    assert!(path.exists(), "save() reported success without a file");

    let (restored, outcome) = persist::load(&path, defaults);
    assert_eq!(outcome, LoadOutcome::Restored, "{}", outcome.reason());
    assert_eq!(restored, saved, "the layout did not round trip");
    assert!((share_of(&restored) - 0.37).abs() < 1e-6);
    assert_eq!(restored.workspace.active(), ViewKind::Protocol);
    assert_eq!(restored.window.position, Some([12.0, 34.0]));
    assert_eq!(
        restored.workspace.panes(ViewKind::Charts),
        saved.workspace.panes(ViewKind::Charts)
    );

    // Focus is deliberately not persisted: restoring a cursor the user does
    // not remember leaving is worse than not restoring one.
    //
    // Would catch: dropping the `serde(skip)`, which also puts focus into
    // the equality the dirty tracker uses and turns every click into a write.
    let mut with_focus = saved.clone();
    assert!(with_focus
        .workspace
        .set_focus(PaneKey::new(ViewKind::Charts, RAIL)));
    assert_eq!(
        with_focus, saved,
        "focus counts as a layout difference, so every click would rewrite the file"
    );
    with_focus.save(&path).expect("writes");
    let (back, _) = persist::load(&path, defaults);
    assert_eq!(back.workspace.focus_in(ViewKind::Charts), None);

    // -----------------------------------------------------------------
    // Every way the file can be broken lands on the default arrangement.
    //
    // Would catch: any of these propagating an error to the caller. A shell
    // that refuses to open because a JSON file has a stray brace in it is
    // strictly worse than one that forgets your splitter positions.
    // -----------------------------------------------------------------
    std::fs::write(&path, "{ this is not json").expect("writes");
    let (fallback, outcome) = persist::load(&path, defaults);
    assert_eq!(outcome, LoadOutcome::Corrupt, "{}", outcome.reason());
    assert_eq!(fallback, defaults(), "a corrupt file did not fall back");

    // The right JSON shape, the wrong version.
    let mut stale: serde_json::Value =
        serde_json::from_str(&saved.to_json().expect("serialises")).expect("valid json");
    stale["version"] = serde_json::json!(LAYOUT_VERSION + 1);
    std::fs::write(&path, stale.to_string()).expect("writes");
    let (fallback, outcome) = persist::load(&path, defaults);
    assert_eq!(
        outcome,
        LoadOutcome::VersionMismatch,
        "{}",
        outcome.reason()
    );
    assert_eq!(fallback, defaults(), "a stale file did not fall back");
    assert!(
        (share_of(&fallback) - 0.37).abs() > 1e-6,
        "the fallback is the arranged layout, so this proves nothing"
    );

    // A future version whose *shape* also differs — which is the only kind of
    // version bump there is a reason to make, and the case the field exists
    // for. Bumping `version` on an otherwise-current envelope, as above, is
    // not it: that one parses, so a version check made after the parse would
    // still pass it.
    //
    // Would catch: reading the version off the deserialised envelope instead
    // of off the raw JSON, which reports a file from a later build as
    // `Corrupt` — a wrong log line, and one that sends whoever reads it
    // looking for a disk fault instead of an upgrade.
    std::fs::write(
        &path,
        serde_json::json!({
            "version": LAYOUT_VERSION + 1,
            "windows": [{ "size": [800.0, 600.0] }],
            "workspaces": []
        })
        .to_string(),
    )
    .expect("writes");
    let (fallback, outcome) = persist::load(&path, defaults);
    assert_eq!(
        outcome,
        LoadOutcome::VersionMismatch,
        "a later version this build cannot even parse was reported as {}",
        outcome.reason()
    );
    assert_eq!(fallback, defaults());

    // JSON with no version field at all is not a layout file, whatever else
    // it is.
    std::fs::write(&path, serde_json::json!({ "hello": "world" }).to_string()).expect("writes");
    let (fallback, outcome) = persist::load(&path, defaults);
    assert_eq!(outcome, LoadOutcome::Corrupt, "{}", outcome.reason());
    assert_eq!(fallback, defaults());

    // -----------------------------------------------------------------
    // A file that parses, is current, and is still short a view.
    //
    // This is the shape a layout written before a `ViewKind` was added will
    // have on the day one is: `trees` is a map, so it simply lacks the new
    // key, and no version bump is warranted for adding an enum variant.
    // `Workspace` derives `Deserialize`, so `Workspace::new`'s completeness
    // assertion never runs on this path.
    //
    // Would catch: trusting the parse. Without the completeness check the
    // outcome below is `Restored` and the *next* `tree()` for the missing
    // view panics — a well-formed file taking the window down, on the first
    // view switch after an upgrade, for every existing user at once.
    // -----------------------------------------------------------------
    let mut short: serde_json::Value =
        serde_json::from_str(&saved.to_json().expect("serialises")).expect("valid json");
    short["workspace"]["trees"]
        .as_object_mut()
        .expect("trees is a map")
        .remove("Protocol")
        .expect("Protocol was in the file");
    std::fs::write(&path, short.to_string()).expect("writes");
    let (repaired, outcome) = persist::load(&path, defaults);
    assert_eq!(outcome, LoadOutcome::Incomplete, "{}", outcome.reason());
    assert!(
        !outcome.restored(),
        "a partial restore reported as a full one"
    );
    assert!(
        repaired.workspace.missing_views().is_empty(),
        "the load handed back a workspace that panics on its next tree()"
    );
    // The repair is a repair, not a reset: what the file *did* hold is still
    // there, and the view it did not is the default one.
    assert!(
        (share_of(&repaired) - 0.37).abs() < 1e-6,
        "filling a missing view threw away the arrangement of the views the file had"
    );
    assert_eq!(
        repaired.workspace.panes(ViewKind::Protocol),
        defaults().workspace.panes(ViewKind::Protocol),
        "the filled view is not the default arrangement"
    );
    // …and it is reachable, which is the whole point.
    let mut switched = repaired;
    switched.workspace.set_active(ViewKind::Protocol);
    assert_eq!(
        switched.workspace.active_tree().tiles.iter().count(),
        defaults()
            .workspace
            .tree(ViewKind::Protocol)
            .tiles
            .iter()
            .count()
    );

    // The degenerate case of the same thing: every view missing.
    let mut empty: serde_json::Value =
        serde_json::from_str(&saved.to_json().expect("serialises")).expect("valid json");
    empty["workspace"]["trees"] = serde_json::json!({});
    std::fs::write(&path, empty.to_string()).expect("writes");
    let (repaired, outcome) = persist::load(&path, defaults);
    assert_eq!(outcome, LoadOutcome::Incomplete, "{}", outcome.reason());
    assert!(repaired.workspace.missing_views().is_empty());
    for view in ViewKind::ALL {
        assert_eq!(
            repaired.workspace.panes(view),
            defaults().workspace.panes(view)
        );
    }

    // Valid JSON of the right version naming a pane this build has not got.
    let ghost = saved
        .to_json()
        .expect("serialises")
        .replace("persist-rail", "persist-ghost");
    assert!(ghost.contains("persist-ghost"), "the rename did not take");
    std::fs::write(&path, &ghost).expect("writes");
    let (fallback, outcome) = persist::load(&path, defaults);
    assert_eq!(outcome, LoadOutcome::Corrupt, "{}", outcome.reason());
    assert_eq!(fallback, defaults());

    // No file at all.
    std::fs::remove_file(&path).expect("removes");
    let (fallback, outcome) = persist::load(&path, defaults);
    assert_eq!(outcome, LoadOutcome::NoFile, "{}", outcome.reason());
    assert_eq!(fallback, defaults());

    // A path that cannot be read as a file — a directory standing where the
    // layout should be.
    std::fs::create_dir_all(&path).expect("a directory in the file's place");
    let (fallback, outcome) = persist::load(&path, defaults);
    assert_eq!(outcome, LoadOutcome::Unreadable, "{}", outcome.reason());
    assert_eq!(fallback, defaults());

    // -----------------------------------------------------------------
    // The debounce, and the bug the spike had.
    // -----------------------------------------------------------------
    let write_path = scratch.0.join("debounced.json");
    let mut tracker = DirtyTracker::new(defaults());

    // Clean: nothing is armed and nothing is written, however long we wait.
    assert!(tracker.poll(0, &write_path).is_none());
    assert!(!tracker.is_armed());
    assert!(tracker.poll(SAVE_DEBOUNCE_MS * 10, &write_path).is_none());
    assert!(!write_path.exists());

    // A change arms the countdown but does not fire it.
    //
    // Would catch: writing on every change — a drag is a change per frame,
    // so a splitter drag would be a hundred file writes.
    arrange(tracker.live_mut(), 0.30);
    assert!(tracker.is_dirty());
    assert!(tracker.poll(1_000, &write_path).is_none());
    assert!(tracker.is_armed());
    assert!(tracker
        .poll(1_000 + SAVE_DEBOUNCE_MS - 1, &write_path)
        .is_none());
    assert!(!write_path.exists(), "the debounce fired early");

    // Still moving: the countdown restarts, so a drag burst collapses into
    // one write at the end rather than one part-way through.
    arrange(tracker.live_mut(), 0.31);
    assert!(tracker.poll(1_500, &write_path).is_none());
    assert!(tracker
        .poll(1_500 + SAVE_DEBOUNCE_MS - 1, &write_path)
        .is_none());
    assert!(
        !write_path.exists(),
        "a change did not restart the countdown"
    );

    // Still, and due: one write.
    let due = 1_500 + SAVE_DEBOUNCE_MS;
    assert_eq!(tracker.poll(due, &write_path), Some(Ok(())));
    assert!(write_path.exists());
    assert!(
        !tracker.is_dirty(),
        "a successful write left the tracker dirty"
    );
    assert!(tracker.poll(due + 1, &write_path).is_none());

    let (written, outcome) = persist::load(&write_path, defaults);
    assert_eq!(outcome, LoadOutcome::Restored);
    assert!((share_of(&written) - 0.31).abs() < 1e-6);

    // A write that fails must leave the tracker dirty and retry.
    //
    // Would catch: exactly the spike's bug — clearing the marker before
    // checking the result, which loses that layout change permanently
    // because the tracker then believes the bytes are on disk.
    let blocked = scratch.0.join("blocked");
    std::fs::create_dir_all(&blocked).expect("a directory where a file should go");
    arrange(tracker.live_mut(), 0.32);
    assert!(tracker.poll(due + 2, &blocked).is_none());
    let failed = tracker
        .poll(due + 2 + SAVE_DEBOUNCE_MS, &blocked)
        .expect("the debounce was due");
    assert!(failed.is_err(), "writing over a directory reported success");
    assert!(
        tracker.is_dirty(),
        "a FAILED write cleared the dirty marker — that layout change is now lost for good"
    );
    // …and it backs off rather than retrying on every frame for as long as
    // the disk stays unwritable.
    assert!(
        tracker.poll(due + 3 + SAVE_DEBOUNCE_MS, &blocked).is_none(),
        "a failed write left the deadline in the past, so this retries every frame"
    );

    // …and the retry, at a path that works, writes what was pending.
    assert_eq!(tracker.flush(&write_path), Some(Ok(())));
    assert!(!tracker.is_dirty());
    let (retried, _) = persist::load(&write_path, defaults);
    assert!(
        (share_of(&retried) - 0.32).abs() < 1e-6,
        "the retry wrote something other than the pending change"
    );

    // Flushing a clean tracker writes nothing at all.
    assert!(tracker.flush(&write_path).is_none());

    // -----------------------------------------------------------------
    // A failed write must not take the layout that was already on disk
    // down with it.
    //
    // Would catch: `std::fs::write`, which truncates the destination before
    // it writes a byte. Half a write then leaves a file that loads as
    // `Corrupt` — survivable, but it survives by discarding the *last good*
    // arrangement, which a temp-file-and-rename keeps. Blocking the temp
    // rather than the destination is what makes this a claim about the
    // destination: the write fails, and the good file was never opened.
    // -----------------------------------------------------------------
    let temp_blocker = {
        let mut name = write_path.file_name().expect("a file name").to_os_string();
        name.push(".tmp");
        write_path.with_file_name(name)
    };
    std::fs::create_dir_all(&temp_blocker).expect("a directory where the temp file goes");
    arrange(tracker.live_mut(), 0.33);
    assert!(
        tracker
            .flush(&write_path)
            .expect("dirty, so a flush attempts a write")
            .is_err(),
        "the write went somewhere other than a temp file beside the destination"
    );
    let (survivor, outcome) = persist::load(&write_path, defaults);
    assert_eq!(outcome, LoadOutcome::Restored, "{}", outcome.reason());
    assert!(
        (share_of(&survivor) - 0.32).abs() < 1e-6,
        "a failed write destroyed the good layout that was already on disk"
    );
    std::fs::remove_dir_all(&temp_blocker).expect("removes");

    // With the way clear it writes, and leaves no temp file behind.
    assert_eq!(tracker.flush(&write_path), Some(Ok(())));
    assert!(
        !temp_blocker.exists(),
        "the temp file was left beside the layout"
    );
    let (after, _) = persist::load(&write_path, defaults);
    assert!((share_of(&after) - 0.33).abs() < 1e-6);

    // -----------------------------------------------------------------
    // Sub-point jitter in the window geometry is not a layout change.
    //
    // Would catch: a derived `PartialEq` on `WindowGeometry`. The shell
    // feeds live size and position every frame, through a scale factor, so
    // an untouched window can report 819.9999 then 820.0001 — compared
    // exactly, a change that never settles, and therefore a write every
    // debounce window for the rest of the session.
    // -----------------------------------------------------------------
    let mut settled = defaults();
    settled.window = WindowGeometry {
        size: [1280.0, 820.0],
        position: Some([0.0, 0.0]),
    };
    let mut jitter = DirtyTracker::new(settled);
    jitter.live_mut().window = WindowGeometry {
        size: [1280.0004, 819.9997],
        position: Some([0.0002, -0.0003]),
    };
    assert!(
        !jitter.is_dirty(),
        "sub-point window jitter was read as a layout change"
    );
    jitter.live_mut().window.size = [1281.0, 820.0];
    assert!(
        jitter.is_dirty(),
        "a window that actually moved by a whole point was not noticed"
    );

    // -----------------------------------------------------------------
    // A drag longer than one debounce window writes nothing until it stops.
    //
    // Not a bug and not an oversight: the countdown restarts on every
    // observed change, so there is no maximum-staleness cap, and the only
    // thing that loses the pending change is a crash mid-drag. Asserted so
    // that the behaviour is a decision on the record rather than something
    // discovered later.
    // -----------------------------------------------------------------
    let long_drag = scratch.0.join("long-drag.json");
    let mut dragging = DirtyTracker::new(defaults());
    let mut share = 0.10_f32;
    let mut now = 0;
    while now < SAVE_DEBOUNCE_MS * 3 {
        share += 0.001;
        arrange(dragging.live_mut(), share);
        assert!(
            dragging.poll(now, &long_drag).is_none(),
            "a moving layout was written at {now}ms"
        );
        now += 100;
    }
    assert!(
        !long_drag.exists(),
        "three debounce windows of continuous movement produced a write"
    );
    assert_eq!(
        dragging.poll(now + SAVE_DEBOUNCE_MS, &long_drag),
        Some(Ok(())),
        "the drag stopped and the layout was still not written"
    );
    assert!(!dragging.is_dirty());

    // -----------------------------------------------------------------
    // Where the file goes.
    //
    // Would catch: the spike's hardcoded macOS path, which put the layout
    // under `~/Library` on Linux and Windows. Pure in its inputs, so all
    // three rules are checkable from one platform.
    // -----------------------------------------------------------------
    assert_eq!(
        persist::layout_path(Some("/tmp/cfg"), None, None),
        Some(std::path::PathBuf::from("/tmp/cfg").join(LAYOUT_FILE)),
        "an explicit override must win, or tests and portable installs cannot steer it"
    );
    let platform = persist::layout_path(None, Some("/xdg"), Some("/home/x"));
    if cfg!(target_os = "macos") {
        assert_eq!(
            platform,
            Some(
                std::path::PathBuf::from("/home/x")
                    .join("Library/Application Support/Brightfield")
                    .join(LAYOUT_FILE)
            )
        );
    } else {
        assert_eq!(
            platform,
            Some(std::path::PathBuf::from("/xdg/brightfield").join(LAYOUT_FILE))
        );
        assert_eq!(
            persist::layout_path(None, None, Some("/home/x")),
            Some(std::path::PathBuf::from("/home/x/.config/brightfield").join(LAYOUT_FILE))
        );
    }
    assert_eq!(
        persist::layout_path(None, None, None),
        None,
        "no home and no config dir must be an answer, not a panic"
    );
    assert_eq!(
        persist::layout_path(Some(""), None, None),
        None,
        "an empty override is not an override"
    );
}
