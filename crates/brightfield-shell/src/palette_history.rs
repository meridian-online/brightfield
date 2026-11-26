//! Where the palette's history is kept between sessions.
//!
//! The commands run from the palette, and when, rank its next session's rows:
//! [`brightfield_model::palette_history_file`] reads and writes the file, and
//! this module says which folder it is in. The app names its config folder at
//! launch ([`use_config_dir`]). A process that has not, such as a test, keeps
//! the history in the config folder `BRIGHTFIELD_CONFIG_DIR` names when it is
//! set, and otherwise only in the window's memory, so no test writes under the
//! home directory.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use brightfield_keys::Frecency;
use brightfield_model::palette_history_file::{load_palette_history, save_palette_history};

/// The config folder the app named at launch, if it has.
static INSTALLED: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Keep the palette's history in the app's config folder from now on: what the
/// app does at launch.
pub fn use_config_dir() {
    let dir = brightfield_workbench::persist::config_dir(
        std::env::var(crate::startup::CONFIG_DIR_VAR)
            .ok()
            .as_deref(),
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    );
    *INSTALLED.lock().unwrap_or_else(PoisonError::into_inner) = dir;
}

/// The folder the history is kept in: the one the app named at launch, else the
/// one `BRIGHTFIELD_CONFIG_DIR` names, else none.
fn dir() -> Option<PathBuf> {
    INSTALLED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .or_else(|| {
            std::env::var(crate::startup::CONFIG_DIR_VAR)
                .ok()
                .filter(|d| !d.is_empty())
                .map(PathBuf::from)
        })
}

/// The history the last session left, or none when there is no folder for it
/// or its file does not read.
#[must_use]
pub fn load() -> Frecency {
    dir().map_or_else(Frecency::new, |dir| load_palette_history(&dir))
}

/// Keep `frecency` for the next session. A history that cannot be written is
/// left unsaved without a word: it ranks the palette's rows and nothing else,
/// and this session's copy in memory still ranks them.
pub fn save(frecency: &Frecency) {
    if let Some(dir) = dir() {
        let _ = save_palette_history(&dir, frecency);
    }
}
