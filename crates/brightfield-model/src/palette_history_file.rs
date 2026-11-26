//! The palette's history between sessions — framework-free.
//!
//! The commands run from the palette, and when, are kept in one small file in
//! the user config dir, so the ones used often and lately rank first in the
//! next session's palette too. Like the dock layout, it is the shell's own
//! state and never part of a Protocol. A file that is missing, empty, corrupt
//! or written by a later build in a format this one does not know loads as no
//! history, without a word to the analyst, and the next pick overwrites it.

use std::io;
use std::path::{Path, PathBuf};

use brightfield_keys::Frecency;

/// The history file's name inside the config dir.
pub const PALETTE_HISTORY_FILE: &str = "palette-history.json";

/// Where the history file is inside the config dir `dir`.
#[must_use]
pub fn palette_history_path(dir: &Path) -> PathBuf {
    dir.join(PALETTE_HISTORY_FILE)
}

/// The history kept in the config dir `dir`, or none at all when its file
/// cannot be read or does not hold a history this build reads.
#[must_use]
pub fn load_palette_history(dir: &Path) -> Frecency {
    std::fs::read_to_string(palette_history_path(dir))
        .ok()
        .and_then(|text| Frecency::from_json(&text))
        .unwrap_or_default()
}

/// Write `frecency` to its file in the config dir `dir`, making the dir if it
/// is not there. The text goes to a file beside it first and is then renamed
/// over the old one, so a write cut short leaves the old history whole.
///
/// # Errors
///
/// The dir could not be made, or the file could not be written or renamed.
pub fn save_palette_history(dir: &Path, frecency: &Frecency) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = palette_history_path(dir);
    let staged = dir.join(format!("{PALETTE_HISTORY_FILE}.tmp"));
    std::fs::write(&staged, frecency.to_json())?;
    std::fs::rename(&staged, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_764_115_200;

    /// A fresh dir under the system temp dir, named for the test and this
    /// process, removed first in case an earlier run left it behind.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "brightfield-palette-history-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn palette_history_round_trips_through_its_file() {
        let dir = scratch("round-trip").join("not-yet-made");
        let mut frecency = Frecency::new();
        frecency.record("zoom-in", NOW - 60);
        frecency.record("pan-left", NOW);
        save_palette_history(&dir, &frecency).expect("it saves, making the dir");
        assert_eq!(load_palette_history(&dir), frecency);
        assert!(
            !dir.join(format!("{PALETTE_HISTORY_FILE}.tmp")).exists(),
            "the staged file was renamed into place"
        );

        // Saving again replaces the file.
        frecency.record("zoom-in", NOW);
        save_palette_history(&dir, &frecency).expect("it saves over the old file");
        assert_eq!(load_palette_history(&dir), frecency);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn a_corrupt_missing_or_future_palette_history_loads_empty() {
        let dir = scratch("corrupt");
        assert_eq!(load_palette_history(&dir), Frecency::new(), "missing dir");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load_palette_history(&dir), Frecency::new(), "missing file");
        for text in ["", "{", r#"{"version":99}"#] {
            std::fs::write(palette_history_path(&dir), text).unwrap();
            assert_eq!(load_palette_history(&dir), Frecency::new(), "{text:?}");
        }

        // And the next save overwrites what could not be read.
        let mut frecency = Frecency::new();
        frecency.record("zoom-in", NOW);
        save_palette_history(&dir, &frecency).unwrap();
        assert_eq!(load_palette_history(&dir), frecency);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
