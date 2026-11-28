//! Asking the analyst for a folder, and keeping what the system hands over: how a path reaches
//! the grant store ([`crate::grants`]).
//!
//! Under App Sandbox the managed edition reaches a path in one of two ways. The system grants it
//! when the analyst chooses it in a panel, opens it from the Finder or drops it on the app, and
//! [`keep`] saves that grant so the next launch has it too. Or Brightfield asks for it: when an
//! open needs a folder no grant covers, [`begin_ask_for`] opens the panel on that folder with one
//! sentence of why, and what the analyst chooses is granted.
//!
//! Both do nothing in an edition that reads ungranted paths, so a caller needs no edition check
//! of its own and the full edition writes no grant store.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use crate::edition::{self, Capability};

/// What stands in for the panel: called with the folder to open it on and the sentence of why,
/// it answers the folder the analyst chose, or `None` when they cancelled. A window is given one
/// in a test, so the ask can be answered without a panel.
pub type Asker = dyn FnMut(&Path, &str) -> Option<PathBuf>;

/// Whether this edition reads only what is granted, so that grants are kept and asked for.
fn grants_apply() -> bool {
    !edition::current().allows(Capability::UngrantedPaths)
}

/// **Save what the system granted with `paths`**: the items an open panel chose, the files the
/// Finder opened the app with, or a drop. Each is granted as it is, a folder as a folder and a
/// file as that file alone. A path that cannot be granted is passed over; the open that follows
/// says what it still needs. Does nothing in the full edition.
pub fn keep(paths: &[PathBuf]) {
    if !grants_apply() {
        return;
    }
    for path in paths {
        if let Err(e) = crate::grants::store().grant(path) {
            eprintln!("warning: {}: its grant was not kept: {e}", path.display());
        }
    }
}

/// **A panel's answer, which arrives on a later frame**: nothing until the panel is answered,
/// then the folder chosen, or `None` for a cancel. The panel's completion writes it and the window
/// reads it, so a panel never runs a modal loop inside the frame that raised it.
#[derive(Debug, Clone, Default)]
pub struct PanelAnswer(Arc<Mutex<Option<Option<PathBuf>>>>);

impl PanelAnswer {
    /// The answer, once the panel has given it, taken so it is read once.
    #[must_use]
    pub fn take(&self) -> Option<Option<PathBuf>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    /// Give the answer `chosen`: what the panel's completion does.
    pub fn give(&self, chosen: Option<PathBuf>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(chosen);
    }
}

/// What stands in for the Mac's panel where its answer arrives on a later frame: called with the
/// folder to open it on and the sentence of why, it answers where the answer will land. A window
/// is given one in a test, to answer the ask frames after it was raised.
pub type PanelAsker = dyn FnMut(&Path, &str) -> PanelAnswer;

/// How an ask was put to the analyst.
#[derive(Debug)]
pub enum Asked {
    /// Answered at once: the folder chosen, not yet granted, or `None` for a cancel.
    Now(Option<PathBuf>),
    /// A panel is up, and its answer lands here.
    Later(PanelAnswer),
    /// Another panel is up, so this one waits: ask again on a later frame.
    Busy,
}

/// **Ask for `folder`: open the panel on it, saying `why`.** On the Mac the panel is raised and
/// this returns at once ([`Asked::Later`]); `wake` is called when it is answered, so the window
/// draws a frame to read the answer. Elsewhere the panel blocks until it is answered. What is
/// chosen, which may not be `folder` if the analyst chose another, is granted by the caller
/// ([`grant_chosen`]).
pub fn begin_ask_for(folder: &Path, why: &str, wake: impl Fn() + 'static) -> Asked {
    let on = nearest_folder(folder);
    #[cfg(target_os = "macos")]
    {
        raised(
            why,
            crate::mac_open::Panel::Folder {
                on: &on,
                create: false,
            },
            wake,
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = wake;
        Asked::Now(
            rfd::FileDialog::new()
                .set_title(why)
                .set_directory(on)
                .pick_folder(),
        )
    }
}

/// Raise `panel` showing `message`, its answer landing in the [`PanelAnswer`] returned.
#[cfg(target_os = "macos")]
fn raised(message: &str, panel: crate::mac_open::Panel<'_>, wake: impl Fn() + 'static) -> Asked {
    let answer = PanelAnswer::default();
    let given = answer.clone();
    match crate::mac_open::begin_panel(message, panel, move |chosen| {
        given.give(chosen);
        wake();
    }) {
        Ok(()) => Asked::Later(answer),
        Err(crate::mac_open::NotRaised::PanelUp) => Asked::Busy,
        // No panel can be raised here, which is a cancel.
        Err(crate::mac_open::NotRaised::OffMainThread) => Asked::Now(None),
    }
}

/// What stands in for the first launch's panel: called with the folder it opens on and the name
/// it proposes, it answers the folder the analyst named, which may not exist yet, or `None` when
/// they cancelled. A window is given one in a test.
pub type ProtocolsAsker = dyn FnMut(&Path, &str) -> Option<PathBuf>;

/// The sentence the first launch's panel shows.
pub const WHY_PROTOCOLS: &str =
    "Choose where Brightfield keeps your Protocols. Save makes the folder there and grants it.";

/// **Ask where the Protocols folder is**: a panel opened on `on` with a folder named `proposed`
/// filled in, so the common answer is one click, and any other place or name may be chosen.
/// Answers, as [`begin_ask_for`] does, the folder named, which the caller makes and grants
/// ([`crate::protocols_dir::adopt`]), or `None` when the analyst cancelled.
///
/// It is a save panel because an open panel can choose only a folder that is already there and
/// cannot propose a name. The system grants what a save panel names, so the folder may be made
/// there under App Sandbox. When a folder of the proposed name can be seen there already, such as
/// one the full edition made, an open panel is opened on it instead, so that Open chooses it and
/// the save panel's question about replacing it is not asked.
pub fn begin_ask_for_protocols_folder(
    on: &Path,
    proposed: &str,
    wake: impl Fn() + 'static,
) -> Asked {
    let existing = on.join(proposed);
    let near = nearest_folder(on);
    #[cfg(target_os = "macos")]
    {
        if existing.is_dir() {
            return raised(
                WHY_PROTOCOLS_EXISTING,
                crate::mac_open::Panel::Folder {
                    on: &existing,
                    create: true,
                },
                wake,
            );
        }
        raised(
            WHY_PROTOCOLS,
            crate::mac_open::Panel::Save {
                on: &near,
                name: proposed,
            },
            wake,
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = wake;
        if existing.is_dir() {
            return Asked::Now(
                rfd::FileDialog::new()
                    .set_title(WHY_PROTOCOLS_EXISTING)
                    .set_directory(&existing)
                    .set_can_create_directories(true)
                    .pick_folder(),
            );
        }
        Asked::Now(
            rfd::FileDialog::new()
                .set_title(WHY_PROTOCOLS)
                .set_directory(near)
                .set_file_name(proposed)
                .set_can_create_directories(true)
                .save_file(),
        )
    }
}

/// The sentence the first launch's panel shows when a Protocols folder is there already.
pub const WHY_PROTOCOLS_EXISTING: &str =
    "Choose where Brightfield keeps your Protocols. Open uses this folder and grants it.";

/// The sentence the panel shows when the Protocols folder chosen before, `folder`, is out of
/// reach this launch.
#[must_use]
pub fn why_protocols_again(folder: &Path) -> String {
    format!(
        "Brightfield needs access to {} again to keep Protocols there. Choose Open to grant it.",
        display(folder)
    )
}

/// Grant `chosen`, which a panel or its stand-in has just handed over: the folder, once the store
/// has it, or `None` with the reason printed.
#[must_use]
pub fn grant_chosen(chosen: &Path) -> Option<PathBuf> {
    match crate::grants::store().grant(chosen) {
        Ok(()) => Some(chosen.to_path_buf()),
        Err(e) => {
            eprintln!(
                "warning: {}: it could not be granted: {e}",
                chosen.display()
            );
            None
        }
    }
}

/// `folder`, or the nearest folder above it that exists, for a panel to open on: a path typed in
/// the Location field may not exist yet.
fn nearest_folder(folder: &Path) -> PathBuf {
    folder
        .ancestors()
        .find(|a| a.is_dir())
        .map_or_else(|| folder.to_path_buf(), Path::to_path_buf)
}

/// The sentence the panel shows when an open needs `folder`.
#[must_use]
pub fn why_open(folder: &Path) -> String {
    format!(
        "Brightfield needs access to {} to open what you asked for. Choose Open to grant it.",
        display(folder)
    )
}

/// The sentence the panel shows when a source is added from `folder`.
#[must_use]
pub fn why_source(folder: &Path) -> String {
    format!(
        "Brightfield needs access to {} to read this source. Choose Open to grant it.",
        display(folder)
    )
}

/// The sentence the panel shows when the Protocol is moved into `folder`.
#[must_use]
pub fn why_location(folder: &Path) -> String {
    format!(
        "Brightfield needs access to {} to move the Protocol there. Choose Open to grant it.",
        display(folder)
    )
}

/// The sentence the panel shows for the sources browser's "Grant a folder…".
pub const WHY_BROWSE: &str = "Choose a folder for Brightfield to read sources from.";

fn display(folder: &Path) -> String {
    folder.file_name().map_or_else(
        || folder.display().to_string(),
        |n| format!("the folder {}", n.to_string_lossy()),
    )
}

/// The folder to ask for when opening the DuckDB database `path` failed with `error` because the
/// database needs a file beside it, such as its write-ahead log, that a grant on the database
/// alone does not reach. `None` for any other file or failure, and in the full edition.
#[must_use]
pub fn database_needs_folder(path: &Path, error: &str) -> Option<PathBuf> {
    if !grants_apply() {
        return None;
    }
    let database = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("duckdb") || e.eq_ignore_ascii_case("db"));
    let refused = ["Operation not permitted", "Permission denied", ".wal"]
        .iter()
        .any(|said| error.contains(said));
    if !database || !refused {
        return None;
    }
    let folder = path.parent()?.to_path_buf();
    crate::grants::store()
        .needs(std::slice::from_ref(&folder))
        .into_iter()
        .next()
}
