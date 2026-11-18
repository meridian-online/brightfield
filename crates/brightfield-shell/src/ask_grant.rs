//! Asking the analyst for a folder, and keeping what the system hands over: how a path reaches
//! the grant store ([`crate::grants`]).
//!
//! Under App Sandbox the managed edition reaches a path in one of two ways. The system grants it
//! when the analyst chooses it in a panel, opens it from the Finder or drops it on the app, and
//! [`keep`] saves that grant so the next launch has it too. Or Brightfield asks for it: when an
//! open needs a folder no grant covers, [`ask_for`] opens the panel on that folder with one
//! sentence of why, and grants what the analyst chooses.
//!
//! Both do nothing in an edition that reads ungranted paths, so a caller needs no edition check
//! of its own and the full edition writes no grant store.

use std::path::{Path, PathBuf};

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

/// **Ask for `folder`: open the panel on it, saying `why`, and grant the folder chosen.**
/// Answers the folder granted, which may not be `folder` if the analyst chose another, or `None`
/// when they cancelled or the grant could not be made. Blocks until the panel is answered, so it
/// is called after a frame, as the window's other panels are.
#[must_use]
pub fn ask_for(folder: &Path, why: &str) -> Option<PathBuf> {
    let chosen = rfd::FileDialog::new()
        .set_title(why)
        .set_directory(nearest_folder(folder))
        .pick_folder()?;
    grant_chosen(&chosen)
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
/// Answers the folder named, which the caller makes and grants
/// ([`crate::protocols_dir::adopt`]), or `None` when the analyst cancelled.
///
/// It is a save panel because an open panel can choose only a folder that is already there and
/// cannot propose a name. The system grants what a save panel names, so the folder may be made
/// there under App Sandbox. When a folder of the proposed name can be seen there already, such as
/// one the full edition made, an open panel is opened on it instead, so that Open chooses it and
/// the save panel's question about replacing it is not asked. Blocks until it is answered, so it
/// is called after a frame.
#[must_use]
pub fn ask_for_protocols_folder(on: &Path, proposed: &str) -> Option<PathBuf> {
    let existing = on.join(proposed);
    if existing.is_dir() {
        return rfd::FileDialog::new()
            .set_title(WHY_PROTOCOLS_EXISTING)
            .set_directory(&existing)
            .set_can_create_directories(true)
            .pick_folder();
    }
    rfd::FileDialog::new()
        .set_title(WHY_PROTOCOLS)
        .set_directory(nearest_folder(on))
        .set_file_name(proposed)
        .set_can_create_directories(true)
        .save_file()
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
