//! The boot sequence — what happens before the window exists.
//!
//! # Why this is a binary of its own, and one test
//!
//! The property under test is an **ordering over a process global**. A
//! `PaneKey` in a layout file deserialises against `ItemId::known()`, which is
//! empty until a registry publishes into it, and one unknown id fails the whole
//! envelope — so a read that happens before the publish reports a perfectly
//! good file as `Corrupt` and silently hands back the default arrangement.
//!
//! A test that shared a binary with anything that builds a window would find
//! the vocabulary already published by the time it ran, and would go green
//! whichever order `boot_layout` does its two jobs in. So: its own binary, its
//! own process, and one ordered test rather than the four it wants to be —
//! exactly the reasoning `brightfield-workbench`'s `layout_file.rs` records for
//! itself.
//!
//! Watched redden, two mutations, both at the same assertion and both with
//! the same reason — `Corrupt`, "the saved layout did not load":
//!
//! - moving the two `publish_item_ids()` calls in `startup::boot_layout` to
//!   *after* the `persist::load`;
//! - deleting them entirely.
//!
//! That the two are indistinguishable from here is the point rather than a
//! gap: from a user's side the two bugs are one bug, and it is silent — the
//! window comes up on the default dock with a line in the log blaming their
//! file.

use brightfield_protocol::layout::Flow;
use brightfield_shell::app::chart_registry;
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::protocol_registry;
use brightfield_shell::starts;
use brightfield_shell::startup::{boot_layout, default_layout, opening_boot};
use brightfield_shell::window::MeridianApp;
use brightfield_workbench::persist::LAYOUT_FILE;
use brightfield_workbench::{ItemId, LoadOutcome, ViewKind};

/// A scratch directory that removes itself, so a failing run cannot poison the
/// next one with a file it left behind.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "brightfield-shell-startup-{}-{}",
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

#[test]
fn a_boot_publishes_before_it_reads_and_the_window_opens_where_it_was_left() {
    let scratch = Scratch::new();
    let path = scratch.file();

    // ---- 1. Nothing is published yet, and neither writing a layout nor
    //         building one changes that. Serialising an `ItemId` is a plain
    //         string write; only *reading* one consults the vocabulary. If
    //         this were false the load below would prove nothing, because the
    //         ordering it is testing would already have happened.
    assert!(
        ItemId::known().is_empty(),
        "something published before this test ran — the ordering below is \
         unobservable in this process, so move this test back into a binary \
         of its own"
    );

    let mut saved = default_layout();
    saved.workspace.set_active(ViewKind::Protocol);
    saved.window.size = [903.0, 617.0];
    saved.window.position = Some([41.0, 62.0]);
    // The remembered start deliberately belongs to the view the window was
    // *not* left on. Pairing them — `opened` on the same view as `active` —
    // makes step 4 below unable to see the bug it is there for, because the
    // two rules give the same answer.
    saved.opened = Some(starts::DASHBOARD.to_string());
    std::fs::write(&path, saved.to_json().expect("a layout serialises")).expect("writes");

    assert!(
        ItemId::known().is_empty(),
        "building and writing a layout published the item vocabulary — the \
         load below can no longer tell whether `boot_layout` publishes first"
    );

    // ---- 2. The read. This is the assertion the ordering hangs on: every
    //         pane key in that file resolves only because both registries
    //         published first, and a publish that came second — or not at all
    //         — reports the file as `Corrupt` and quietly reverts the user to
    //         the default dock.
    let (restored, outcome) = boot_layout(Some(&path));
    assert_eq!(
        outcome,
        LoadOutcome::Restored,
        "the saved layout did not load ({}); a layout this build wrote should \
         read back as saved",
        outcome.reason()
    );
    assert_eq!(restored.window.size, [903.0, 617.0]);
    assert_eq!(restored.window.position, Some([41.0, 62.0]));
    assert_eq!(restored.workspace.active(), ViewKind::Protocol);
    assert_eq!(restored.opened.as_deref(), Some(starts::DASHBOARD));

    // ---- 3. And it published *both* views, not only the one the layout is
    //         active on. The tree of the view that is not active sits in the
    //         same file and deserialises in the same pass.
    let known = ItemId::known();
    for id in chart_registry()
        .ids()
        .iter()
        .chain(&protocol_registry().ids())
    {
        assert!(
            known.contains(id),
            "{id} is declared but the boot never published it"
        );
    }

    // ---- 4. A launch that named nothing takes the restored active view —
    //         through `opening_boot`, which is what `main` calls, and not
    //         through a hand-built `Boot::empty()`. The distinction is the
    //         whole test: the file remembers a *charts* start and was left on
    //         the *protocol* view, so a restore that let the start choose the
    //         view lands on Charts and the persisted active view is dead.
    let boot = opening_boot(None, restored.opened.as_deref(), Flow::Vertical)
        .expect("a launch that named nothing cannot fail");
    assert_eq!(
        boot.view, None,
        "a restored start claimed a view, which overrides the one the user \
         left the window on"
    );
    let app = MeridianApp::headless_with_layout(boot, restored.clone(), Mode::Light);
    assert_eq!(
        app.active(),
        ViewKind::Protocol,
        "a no-argument launch overrode the view the layout was left on"
    );
    assert!(
        !app.chart_doc().is_empty(),
        "the remembered start was not restored — deferring the view must not \
         cost the document"
    );
    assert_eq!(app.layout().window.size, [903.0, 617.0]);

    // ---- 5. A spec on the command line wins over it. You asked for that one,
    //         so this is the one boot that is entitled to move the window off
    //         the view it was left on.
    let named = opening_boot(
        Some("../../examples/dashboard.yaml"),
        restored.opened.as_deref(),
        Flow::Vertical,
    )
    .expect("the named spec opens");
    assert_eq!(named.view, Some(ViewKind::Charts));
    let app = MeridianApp::headless_with_layout(named, restored, Mode::Light);
    assert_eq!(
        app.active(),
        ViewKind::Charts,
        "a named spec was overruled by the view the layout was left on"
    );

    // ---- 6. Persistence off is not an error. No config directory anywhere is
    //         a real state — a machine with no `HOME` — and it has to open a
    //         window rather than refuse to start.
    let (fallback, outcome) = boot_layout(None);
    assert_eq!(outcome, LoadOutcome::NoFile);
    assert!(fallback.workspace.missing_views().is_empty());

    // ---- 7. A file this build cannot read never stops the boot either, and
    //         says which kind of unreadable it was.
    std::fs::write(&path, "{ not json at all").expect("writes");
    let (defaulted, outcome) = boot_layout(Some(&path));
    assert_eq!(outcome, LoadOutcome::Corrupt);
    assert!(defaulted.workspace.missing_views().is_empty());
    assert_eq!(
        defaulted.opened, None,
        "a layout that did not parse still claimed something was open"
    );
}
