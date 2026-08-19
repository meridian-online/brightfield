//! The layout file: what it holds, where it lives, and when it is written.
//!
//! # A bad file must never take the window down
//!
//! Every failure mode of the file — absent, unreadable, not JSON, JSON of the
//! wrong shape, the right shape at a version this build does not understand,
//! or naming a pane this build cannot construct — resolves to the same thing:
//! [`LoadOutcome`] says why, and the caller gets the default arrangement. No
//! migration and no prompt, which is the policy the gpui-era shell already
//! settled on and had no reason to revisit.
//!
//! One failure mode is **not** on that list, and it is the one that would have
//! taken the window down. [`Workspace`] derives `Deserialize`, so a load never
//! runs `Workspace::new`'s completeness assertion, and `trees` is a map: a
//! file that parses, carries the current version and is simply short a view —
//! an empty map, or one written before a third
//! [`ViewKind`](crate::ViewKind) existed — deserialises without complaint and
//! then panics on the first `tree()` for the view it lacks. [`from_json`]
//! therefore checks completeness itself and fills what is missing from the
//! default arrangement, reporting [`LoadOutcome::Incomplete`]. It is the
//! module's one repair, and it is here because the alternative to repairing is
//! not "discard the file": it is a shell that comes up after an upgrade and
//! dies on the first switch to the new view.
//!
//! The pane-naming failure mode is not this module's doing.
//! [`ItemId`](crate::ItemId)'s `Deserialize` rejects an id no registry
//! published, so a layout naming a renamed pane fails to *load* rather than
//! materialising a pane nothing can draw. It surfaces here as
//! [`LoadOutcome::Corrupt`], and the ordering that makes it work is that every
//! view's registry must publish before the file is read.
//!
//! # When it is written
//!
//! `egui_tiles` 0.16 has no layout-changed callback of any kind: `Behavior`
//! exposes only per-pane and per-tab hooks, and `Tree::ui` returns `()`. So
//! the signal is an equality check against the last-written clone —
//! [`DirtyTracker`]. `Tiles`' `PartialEq` is hand-written and deliberately
//! ignores the per-frame `rects` and the `next_tile_id` counter, and
//! [`Workspace`] excludes focus for the same reason, so the compare is exact:
//! a layout pass alone cannot make it true.
//!
//! A drag produces a change per frame, so writes are debounced. The debounce
//! is self-arming — [`DirtyTracker::poll`] re-arms whenever the layout moved
//! since the previous tick — because the alternative is a `layout_changed()`
//! call every caller has to remember at every mutation site, and the one that
//! gets forgotten loses a layout silently.
//!
//! # The bug the spike had, written down so it is not inherited
//!
//! The spike cleared its dirty marker before checking that the write
//! succeeded, so a failed write lost that layout change permanently: the
//! tracker believed the bytes were on disk and never tried again.
//! [`DirtyTracker::poll`] and [`DirtyTracker::flush`] advance `saved` only on
//! `Ok`, and they advance it to *the snapshot that was written* rather than
//! to whatever `live` has since become.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::subject::RunState;
use crate::workspace::Workspace;

/// The layout file's name, under whichever config directory applies.
pub const LAYOUT_FILE: &str = "workspace-layout.json";

/// Bump when the persisted shape changes incompatibly. A mismatch discards
/// the file and rebuilds the default arrangement.
pub const LAYOUT_VERSION: u32 = 1;

/// How long the layout must sit still before it is written, in milliseconds.
///
/// The same 10s window the gpui-era shell used, and the same one
/// `egui_tiles`' own dock example uses. A quit flushes immediately regardless
/// — see [`DirtyTracker::flush`].
pub const SAVE_DEBOUNCE_MS: u64 = 10_000;

// ---------------------------------------------------------------------------
// What is on disk
// ---------------------------------------------------------------------------

/// The window's size and position, in points.
///
/// `position` is optional because a first boot has none and because a display
/// that has since been unplugged should not pin the window off-screen; the
/// shell is free to ignore a position it cannot honour.
///
/// # Why equality is at whole-point resolution
///
/// This type is inside the [`PartialEq`] the [`DirtyTracker`] compares, and
/// the shell will feed it live window size and position every frame. Window
/// geometry arrives through a scale factor, so a window nobody is touching
/// can report `819.9999` one frame and `820.0001` the next; compared exactly,
/// that is a layout change, and a layout change that never settles is a write
/// every debounce window for the rest of the session.
///
/// So equality is on the values rounded to whole points — an exact
/// equivalence on the rounded value, not a tolerance, so it stays transitive.
/// It bounds the damage to sub-point jitter and nothing more: a window
/// genuinely in motion produces real point-sized changes and *should*
/// eventually be written. That case is the debounce's, not this one's — a
/// drag writes once, when it stops.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct WindowGeometry {
    /// Inner size, in points.
    pub size: [f32; 2],
    /// Outer position, in points, if known.
    pub position: Option<[f32; 2]>,
}

impl PartialEq for WindowGeometry {
    fn eq(&self, other: &Self) -> bool {
        fn points(v: [f32; 2]) -> [i64; 2] {
            [v[0].round() as i64, v[1].round() as i64]
        }
        points(self.size) == points(other.size)
            && self.position.map(points) == other.position.map(points)
    }
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self {
            size: [1280.0, 820.0],
            position: None,
        }
    }
}

/// The whole persisted envelope: the version, the window, the workspace
/// (which carries the per-view trees and the active view), and what the window
/// was last opened on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedLayout {
    /// See [`LAYOUT_VERSION`].
    pub version: u32,
    /// Where and how big the window was.
    pub window: WindowGeometry,
    /// The arrangement.
    pub workspace: Workspace,
    /// The shipped starting point the window last opened, if any.
    ///
    /// # Why the arrangement is not enough
    ///
    /// The rest of this envelope restores *where the panes are*, not *what is
    /// in them*: a shell that restored only [`Self::workspace`] would come up
    /// with the user's splitter positions around panes that are all still
    /// empty. That is not a restored session, and a surface that invites a
    /// first action would be right to go on inviting one.
    ///
    /// So the id of a starting point the binary ships is recorded here and
    /// re-opened at boot. An id rather than a path, deliberately: a path drags
    /// in a file that has since moved or been deleted, and a boot that can
    /// fail is a boot that needs a policy for failing. A shipped start cannot
    /// go missing, and an id this build does not recognise means the same
    /// thing as `None` — open on nothing.
    ///
    /// # Why this is not a [`LAYOUT_VERSION`] bump
    ///
    /// Every file written before this field existed still parses as
    /// [`LoadOutcome::Restored`], so nothing is discarded on upgrade. The
    /// version's own rule is to bump "when the persisted shape changes
    /// incompatibly", and this change is compatible.
    ///
    /// **`#[serde(default)]` is the thing not to drop the day this field stops
    /// being an `Option`.** On an `Option<T>` the attribute is redundant:
    /// serde's derive already reads a missing one as `None`, so removing it
    /// here changes nothing any test can see. On any other type it is the
    /// whole mechanism — a field added at a non-optional type *without* it
    /// makes every file already on every machine fail to parse, which surfaces
    /// as [`LoadOutcome::Corrupt`]: a log line blaming the user's file for a
    /// change this build made, and everybody's arrangement gone. Which is the
    /// opposite way round from how it reads, so it is written down.
    ///
    /// Both halves watched, by adding a second field beside this one at
    /// `u32` and stripping it from a saved file the way
    /// `a_layout_from_before_this_field_existed_still_loads` strips this one:
    /// without the attribute the load reports `Corrupt`, with it `Restored`.
    /// And removing the attribute from this field, an `Option`, leaves that
    /// test green.
    #[serde(default)]
    pub opened: Option<String>,
    /// The Protocols this window has opened, most recent first — what the
    /// front door's Protocols section draws a row from.
    ///
    /// # Why this is not [`Self::opened`] with a longer name
    ///
    /// They answer different questions. `opened` is *what to restore on the
    /// next launch*, so it is one id and the boot path reads it before a
    /// window exists. This is *what the analyst has been working on*, so it is
    /// a list, it outlives the restore, and nothing boots from it. A window
    /// that restores its work and is then sent Home has both: `opened` still
    /// naming the restored start, and this naming it plus everything before
    /// it.
    ///
    /// `#[serde(default)]` is load-bearing here in the way the comment on
    /// `opened` says it is **not** load-bearing there: this is a `Vec`, not an
    /// `Option`, so without the attribute every layout file written before
    /// this field existed fails to parse and reports
    /// [`LoadOutcome::Corrupt`] — everybody's arrangement discarded by an
    /// upgrade. `a_layout_from_before_recents_existed_still_loads` strips the
    /// field from a saved file and holds that.
    ///
    /// One thing to know before adding a variant to
    /// [`RunState`](crate::RunState): a layout file naming a state this build
    /// does not have fails to *parse*, and that discards the whole envelope,
    /// not just the recent. Growing that enum is a [`LAYOUT_VERSION`] question
    /// even though the enum lives nowhere near this file.
    #[serde(default)]
    pub recents: Vec<Recent>,
}

/// One remembered Protocol: what the front door's Protocols section draws a
/// row from, without reopening anything.
///
/// Everything the row needs is *recorded*, not derived at draw time, and that
/// is the point: drawing the door must not load five documents to find out
/// what they say. The name is what the window was titled by while it was
/// open, and the run state is what the run contract said then — a surface
/// reads recorded run state and never computes it, which is
/// [`RunState`](crate::RunState)'s own rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recent {
    /// The shipped starting point this was opened from — the same id
    /// [`SavedLayout::opened`] records, and the id a click on the row
    /// reopens through. An id this build does not recognise is a row that
    /// cannot be reopened, so the door draws no row for it.
    pub id: String,
    /// What to call it: the Protocol's own name, as the window was titled by
    /// while it was open. Recorded rather than looked up from the start,
    /// because a start's `label` is a *verb* for a button ("Open the EDGAR ↔
    /// GLEIF crosswalk") and a row is a noun.
    pub name: String,
    /// The run state the Protocol was last seen in, ingested from the run
    /// contract that was open — never recomputed here. Absent from a file
    /// written before this field existed, which reads as
    /// [`RunState::NeverRun`]: the safe direction, exactly as that enum's
    /// default is.
    #[serde(default)]
    pub run: RunState,
    /// When it was last opened, in whole seconds since the Unix epoch — what
    /// the row's relative time is measured from.
    ///
    /// Seconds rather than a timestamp type so the file stays plain JSON any
    /// build can read, and a clock that has since gone backwards produces a
    /// row reading *just now* rather than a panic — see the shell's
    /// `relative_time`.
    pub opened_at: u64,
}

/// How many [`Recent`]s the layout file keeps.
///
/// Small on purpose. The front door draws all of them, and a door listing
/// twenty rows is a file browser rather than a way back into the last thing
/// you were doing; the cap is also what stops a file that is written on every
/// open from growing without bound.
/// `the_recents_list_is_capped_and_most_recent_first` pins both halves.
pub const RECENTS_KEPT: usize = 6;

impl SavedLayout {
    /// A layout at the current version, around `workspace`, opened on nothing.
    #[must_use]
    pub fn new(workspace: Workspace) -> Self {
        Self {
            version: LAYOUT_VERSION,
            window: WindowGeometry::default(),
            workspace,
            opened: None,
            recents: Vec::new(),
        }
    }

    /// Record that `id` was opened at `opened_at`, at the head of
    /// [`Self::recents`].
    ///
    /// Reopening something already in the list **moves** it rather than
    /// adding a second row: two rows for one Protocol is a list of events,
    /// and the door offers a list of Protocols. The name and the run state
    /// are overwritten with what was just seen, because the older pair is by
    /// construction the more stale of the two.
    ///
    /// Trimmed to [`RECENTS_KEPT`] from the tail, so the entry dropped is
    /// always the least recently opened one.
    pub fn remember(&mut self, id: &str, name: &str, run: RunState, opened_at: u64) {
        self.recents.retain(|r| r.id != id);
        self.recents.insert(
            0,
            Recent {
                id: id.to_string(),
                name: name.to_string(),
                run,
                opened_at,
            },
        );
        self.recents.truncate(RECENTS_KEPT);
    }

    /// Serialise to pretty JSON.
    ///
    /// # Errors
    ///
    /// If serialisation fails. Every field is plain data, so in practice it
    /// does not — but a layout file is not worth an `unwrap`.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Write to `path`, creating the directory.
    ///
    /// Written to a temporary file beside `path` and renamed over it, so the
    /// file at `path` is only ever the whole of some layout. `std::fs::write`
    /// truncates in place, which means a crash or a full disk part-way
    /// through leaves a half-written file — recoverable, since the load path
    /// degrades to [`LoadOutcome::Corrupt`] and the default arrangement, but
    /// recoverable by discarding the *previous* good layout, which a rename
    /// keeps. `rename` is atomic within a filesystem on every platform this
    /// ships to, and the temporary sits in the same directory to stay on the
    /// same filesystem.
    ///
    /// A failure leaves the temporary behind; it is a fixed name, so the next
    /// successful write reuses and consumes it rather than accumulating.
    ///
    /// # Errors
    ///
    /// On any serialisation or I/O failure, with the path in the message —
    /// the caller logs it and, because [`DirtyTracker`] stays dirty, retries.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let json = self.to_json().map_err(|e| e.to_string())?;
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(".tmp");
        let tmp = path.with_file_name(name);
        std::fs::write(&tmp, json).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("{}: {e}", path.display())
        })
    }
}

/// Why a load produced what it produced.
///
/// Reported rather than swallowed so a boot can log the reason. "Your layout
/// was reset" with no cause is the kind of thing nobody ever debugs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadOutcome {
    /// The file was restored as saved.
    Restored,
    /// The file parsed and every view it held was restored, but it was
    /// missing at least one view, which was rebuilt from the default
    /// arrangement. What a layout written before a
    /// [`ViewKind`](crate::ViewKind) was added looks like.
    Incomplete,
    /// There is no file yet — a first boot.
    NoFile,
    /// The file exists but could not be read.
    Unreadable,
    /// The file did not parse as the current envelope. Includes a layout that
    /// names a pane this build does not have, which
    /// [`ItemId`](crate::ItemId)'s `Deserialize` rejects.
    Corrupt,
    /// The file parsed but carries a different [`LAYOUT_VERSION`].
    VersionMismatch,
}

impl LoadOutcome {
    /// Whether the saved arrangement was used **as saved**.
    ///
    /// [`LoadOutcome::Incomplete`] reports `false` even though most of the
    /// file survived: something the user did not arrange is now on screen,
    /// which is the thing a caller asking this question wants to know.
    #[must_use]
    pub const fn restored(self) -> bool {
        matches!(self, LoadOutcome::Restored)
    }

    /// A line for the boot log.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            LoadOutcome::Restored => "restored the saved layout",
            LoadOutcome::Incomplete => {
                "the saved layout was missing a view, rebuilt from the default"
            }
            LoadOutcome::NoFile => "no saved layout",
            LoadOutcome::Unreadable => "the saved layout could not be read",
            LoadOutcome::Corrupt => "the saved layout did not parse",
            LoadOutcome::VersionMismatch => "the saved layout is from another version",
        }
    }
}

/// Parse a layout from JSON text, falling back to `default` on any problem.
///
/// `default` is a closure rather than a value so a healthy load costs nothing
/// to build the arrangement it is not going to use — and building one means
/// instantiating every view's default tree.
///
/// The version is read out of the raw JSON *before* the envelope is
/// deserialised. Checking `saved.version` after a successful parse only works
/// while every version shares a shape: the day one does not, the parse fails
/// first and reports [`LoadOutcome::Corrupt`], which is the wrong answer and
/// the wrong log line for a file that is merely from another build. Reading
/// the field first is what makes [`LoadOutcome::VersionMismatch`] mean
/// anything.
///
/// `default` must return a layout with a tree for every
/// [`ViewKind`](crate::ViewKind) — as one built through
/// [`Workspace::new`](crate::Workspace::new) does, which asserts it. A
/// `default` that does not cannot repair an incomplete file: the outcome is
/// [`LoadOutcome::Corrupt`] and the layout handed back is `default`'s own,
/// still short a view. Nothing downstream can fix that, which is exactly why
/// `Workspace::new` asserts.
#[must_use]
pub fn from_json(
    raw: Option<&str>,
    default: impl FnOnce() -> SavedLayout,
) -> (SavedLayout, LoadOutcome) {
    let Some(raw) = raw else {
        return (default(), LoadOutcome::NoFile);
    };
    // Version first, off the raw value: a future shape may not parse as this
    // build's envelope at all, and "from another version" is the honest
    // reason for it rather than "did not parse".
    match serde_json::from_str::<serde_json::Value>(raw) {
        Err(_) => return (default(), LoadOutcome::Corrupt),
        Ok(value) => match value.get("version").and_then(serde_json::Value::as_u64) {
            Some(v) if v == u64::from(LAYOUT_VERSION) => {}
            Some(_) => return (default(), LoadOutcome::VersionMismatch),
            // No version field at all is not a version this build understands
            // either, but it is also not a well-formed envelope — a file
            // written by something that is not this program. Corrupt.
            None => return (default(), LoadOutcome::Corrupt),
        },
    }
    let Ok(mut saved) = serde_json::from_str::<SavedLayout>(raw) else {
        return (default(), LoadOutcome::Corrupt);
    };
    if saved.workspace.missing_views().is_empty() {
        return (saved, LoadOutcome::Restored);
    }
    let fallback = default();
    saved.workspace.fill_missing_views_from(&fallback.workspace);
    if saved.workspace.missing_views().is_empty() {
        (saved, LoadOutcome::Incomplete)
    } else {
        (fallback, LoadOutcome::Corrupt)
    }
}

/// Read the layout from `path`, falling back to `default`.
///
/// Every view's registry must have published its ids *before* this runs, or
/// a perfectly good file is read as [`LoadOutcome::Corrupt`].
#[must_use]
pub fn load(path: &Path, default: impl FnOnce() -> SavedLayout) -> (SavedLayout, LoadOutcome) {
    match std::fs::read_to_string(path) {
        Ok(raw) => from_json(Some(&raw), default),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (default(), LoadOutcome::NoFile),
        Err(_) => (default(), LoadOutcome::Unreadable),
    }
}

/// Where the layout file lives.
///
/// An explicit override wins, for tests and portable installs; otherwise the
/// platform config directory — macOS `Application Support`, else
/// `$XDG_CONFIG_HOME`, else `~/.config`. That policy is
/// `brightfield-model`'s `dock_state_file::dock_state_path`, reimplemented
/// rather than imported: only the policy is shared, and it is small enough
/// that duplicating it costs less than the cross-crate dependency.
///
/// Pure in its inputs — the environment is passed, never read — so the
/// selection is testable on every platform's rule at once, and `None` (no
/// home, no config dir) is a real answer rather than a panic.
#[must_use]
pub fn layout_path(
    env_override: Option<&str>,
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    if let Some(dir) = env_override.filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(dir).join(LAYOUT_FILE));
    }
    if cfg!(target_os = "macos") {
        home.filter(|s| !s.is_empty()).map(|h| {
            PathBuf::from(h)
                .join("Library/Application Support/Brightfield")
                .join(LAYOUT_FILE)
        })
    } else if let Some(xdg) = xdg_config_home.filter(|s| !s.is_empty()) {
        Some(PathBuf::from(xdg).join("brightfield").join(LAYOUT_FILE))
    } else {
        home.filter(|s| !s.is_empty()).map(|h| {
            PathBuf::from(h)
                .join(".config/brightfield")
                .join(LAYOUT_FILE)
        })
    }
}

// ---------------------------------------------------------------------------
// Dirty tracking
// ---------------------------------------------------------------------------

/// Holds the live layout plus a clone of what is durably on disk, and decides
/// when to reconcile them.
///
/// See the module docs for why the signal is an equality check and why the
/// debounce arms itself.
#[derive(Debug)]
pub struct DirtyTracker {
    live: SavedLayout,
    /// A clone of what the last **successful** write put on disk.
    saved: SavedLayout,
    /// What `live` looked like at the previous poll, and when its deadline
    /// falls. `None` means nothing is armed.
    armed: Option<(SavedLayout, u64)>,
}

impl DirtyTracker {
    /// Start tracking, treating `layout` as already on disk.
    ///
    /// That is the right assumption for both entry points: a restored layout
    /// literally is on disk, and a defaulted one is what a *missing* file
    /// means, so writing it out before the user has arranged anything would
    /// only create a file that says nothing.
    #[must_use]
    pub fn new(layout: SavedLayout) -> Self {
        Self {
            saved: layout.clone(),
            live: layout,
            armed: None,
        }
    }

    /// The live layout — what the UI reads.
    #[must_use]
    pub const fn live(&self) -> &SavedLayout {
        &self.live
    }

    /// The live layout — what the UI mutates.
    pub fn live_mut(&mut self) -> &mut SavedLayout {
        &mut self.live
    }

    /// The live workspace, mutably. The common case of [`Self::live_mut`].
    pub fn workspace_mut(&mut self) -> &mut Workspace {
        &mut self.live.workspace
    }

    /// Whether the live layout differs from what is durably on disk.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.live != self.saved
    }

    /// Whether a debounced write is armed and has not fired.
    #[must_use]
    pub const fn is_armed(&self) -> bool {
        self.armed.is_some()
    }

    /// Tick the debounce at `now_ms`, writing to `path` when the layout has
    /// been dirty and still for [`SAVE_DEBOUNCE_MS`].
    ///
    /// Returns `None` when nothing was written, and the write's result when
    /// something was attempted. The marker advances **only** on `Ok`: a
    /// failed write leaves the tracker dirty, so the exit flush or a later
    /// tick retries it rather than losing that layout change for good. That
    /// is the spike's bug, not inherited.
    ///
    /// A failure re-arms the countdown rather than leaving the deadline in
    /// the past, so a disk that is full or read-only is retried once per
    /// debounce window instead of once per frame.
    ///
    /// There is **no maximum-staleness cap**: because the countdown restarts
    /// on every observed change, a layout that keeps moving is never written,
    /// however long it keeps moving. A drag held for a minute writes nothing
    /// until the mouse stops. That is deliberate rather than overlooked — a
    /// cap would write a layout mid-drag, which is a state the user never
    /// chose and would be restored into if the process died a moment later —
    /// and it is survivable because the only thing that loses the pending
    /// change is a crash *during* the drag: [`DirtyTracker::flush`] covers
    /// quitting, and any ordinary pause writes.
    ///
    /// Costs one clone of the layout per tick while the layout is moving, and
    /// nothing at all while it is still — that clone is the price of not
    /// requiring every mutation site to remember to announce itself, and a
    /// layout is a handful of tiles.
    pub fn poll(&mut self, now_ms: u64, path: &Path) -> Option<Result<(), String>> {
        if !self.is_dirty() {
            self.armed = None;
            return None;
        }
        match &self.armed {
            // Still moving: restart the countdown so a drag burst collapses
            // into one write at the end rather than one per frame.
            Some((snapshot, _)) if *snapshot != self.live => {
                self.armed = Some((self.live.clone(), now_ms + SAVE_DEBOUNCE_MS));
                None
            }
            Some((_, due)) if now_ms >= *due => {
                let result = self.write_to(path);
                if result.is_err() {
                    // Still dirty, and the deadline is now in the past — leave
                    // it there and this would retry on every frame for as long
                    // as the disk stays unwritable.
                    self.armed = Some((self.live.clone(), now_ms + SAVE_DEBOUNCE_MS));
                }
                Some(result)
            }
            Some(_) => None,
            None => {
                self.armed = Some((self.live.clone(), now_ms + SAVE_DEBOUNCE_MS));
                None
            }
        }
    }

    /// Write now if there is anything to write, debounce or not.
    ///
    /// The quit path: a drag followed immediately by a close must not lose
    /// the arrangement to a countdown that had ten seconds left.
    pub fn flush(&mut self, path: &Path) -> Option<Result<(), String>> {
        if !self.is_dirty() {
            self.armed = None;
            return None;
        }
        Some(self.write_to(path))
    }

    fn write_to(&mut self, path: &Path) -> Result<(), String> {
        // Snapshot first: the marker must describe the bytes that actually
        // landed, not whatever `live` becomes afterwards.
        let snapshot = self.live.clone();
        snapshot.save(path)?;
        self.saved = snapshot;
        self.armed = None;
        Ok(())
    }
}
