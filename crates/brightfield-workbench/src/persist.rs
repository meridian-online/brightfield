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
//! The last of those failure modes is not this module's doing.
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
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    /// Inner size, in points.
    pub size: [f32; 2],
    /// Outer position, in points, if known.
    pub position: Option<[f32; 2]>,
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self {
            size: [1280.0, 820.0],
            position: None,
        }
    }
}

/// The whole persisted envelope: the version, the window, and the workspace
/// (which carries the per-view trees and the active view).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedLayout {
    /// See [`LAYOUT_VERSION`].
    pub version: u32,
    /// Where and how big the window was.
    pub window: WindowGeometry,
    /// The arrangement.
    pub workspace: Workspace,
}

impl SavedLayout {
    /// A layout at the current version, around `workspace`.
    #[must_use]
    pub fn new(workspace: Workspace) -> Self {
        Self {
            version: LAYOUT_VERSION,
            window: WindowGeometry::default(),
            workspace,
        }
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
    /// # Errors
    ///
    /// On any serialisation or I/O failure, with the path in the message —
    /// the caller logs it and, because [`DirtyTracker`] stays dirty, retries.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let json = self.to_json().map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| format!("{}: {e}", path.display()))
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
    /// Whether the saved arrangement was used.
    #[must_use]
    pub const fn restored(self) -> bool {
        matches!(self, LoadOutcome::Restored)
    }

    /// A line for the boot log.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            LoadOutcome::Restored => "restored the saved layout",
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
#[must_use]
pub fn from_json(
    raw: Option<&str>,
    default: impl FnOnce() -> SavedLayout,
) -> (SavedLayout, LoadOutcome) {
    let Some(raw) = raw else {
        return (default(), LoadOutcome::NoFile);
    };
    match serde_json::from_str::<SavedLayout>(raw) {
        Ok(saved) if saved.version == LAYOUT_VERSION => (saved, LoadOutcome::Restored),
        Ok(_) => (default(), LoadOutcome::VersionMismatch),
        Err(_) => (default(), LoadOutcome::Corrupt),
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
/// `brightfield-app`'s `dock_state_file::dock_state_path`, reimplemented
/// rather than imported: that module lives in a crate that pulls gpui, and
/// this one is framework-free by construction. Only the policy is shared, and
/// it is small enough that duplicating it costs less than the dependency.
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
    /// failed write leaves the tracker dirty and re-armed, so the next tick
    /// or the exit flush retries it. That is the spike's bug, not inherited.
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
            Some((_, due)) if now_ms >= *due => Some(self.write_to(path)),
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
