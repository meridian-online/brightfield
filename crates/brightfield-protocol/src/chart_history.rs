//! A chart file's versions, kept in arcform's local history.
//!
//! brightfield writes a chart file beside a Protocol's spec, and until this
//! module nothing kept the text a Save replaced: after two Saves the first
//! Save's text was gone. arcform's local history keeps each file's own list of
//! versions, keyed by the file's path ([`LocalHistory::record_save_for_file`]
//! and its siblings), and this module is the one place brightfield calls it.
//!
//! **Two kinds of entry, both for the chart file only.** The text a Save
//! replaces is recorded as a checkpoint before the write, and the text it
//! writes is recorded as a save after. The Protocol's own `arcform.yaml` is not
//! recorded here, so its history lists what it listed before.
//!
//! **A Save is not blocked by the history.** A store that cannot be opened, or
//! an entry that cannot be written, leaves the chart written and is returned as
//! [`NotRecorded`] for the window to say. arcform's own edit roads refuse the
//! write when the checkpoint fails, because their write is a machine's; a Save
//! is the analyst's, and a chart they could not keep is the worse outcome.
//!
//! **Where the store is.** [`HistoryStore::Arcform`] is arcform's conventional
//! root, `$ARCFORM_HISTORY_DIR` when set and `~/.arcform/history` otherwise,
//! and outside the Protocol's folder. [`HistoryStore::At`] names a root,
//! which is how a test keeps its entries out of the home directory.
//!
//! # The debounce, and why a Save can be recorded as a checkpoint
//!
//! arcform merges a save into the newest entry when that entry is itself a
//! save no older than [`HISTORY_MERGE_WINDOW`]: rapid saves debounce to one
//! entry. Two Saves of a chart within that window would then fold the first
//! Save's text away, because the first Save's text is both the newest entry (a
//! save) and the text the second Save replaces (a checkpoint arcform skips as a
//! duplicate of it). arcform's own roads record their after-image with the
//! merge off for exactly this reason, and its file-keyed calls offer no such
//! switch. So when the replaced text is held by a save the next save would
//! merge over, [`ChartVersions::finish`] records the written text as a
//! checkpoint, which does not merge. A Save's text is then a version in that case
//! too, under a different kind.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use arc::spec::{HistoryWay, HISTORY_MERGE_WINDOW};

// Re-exported so a caller that reads the store back, as a test does, names
// arcform's types through this crate and not through a second `arc` pin.
pub use arc::spec::{HistoryEntry, HistoryKind, LocalHistory};

/// The word brightfield names itself by in the store, so a version written
/// here is listed as reached by brightfield and not by arcform's terminal.
pub const WAY: &str = "brightfield";

/// Where a window keeps the versions of the files it writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryStore {
    /// arcform's conventional store: `$ARCFORM_HISTORY_DIR` when set, else
    /// `~/.arcform/history`.
    Arcform,
    /// A store at a chosen root.
    At(PathBuf),
}

/// Why a Save's version was not recorded. The Save itself wrote the chart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotRecorded {
    reason: String,
}

impl NotRecorded {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    /// What went wrong, in the words the window says.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl std::fmt::Display for NotRecorded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for NotRecorded {}

impl HistoryStore {
    /// Open the store, creating its root if it is not there, and name
    /// brightfield as the way the versions it records were reached.
    ///
    /// # Errors
    ///
    /// [`NotRecorded`] when no root resolves or the root cannot be made a
    /// folder.
    pub fn open(&self) -> Result<LocalHistory, NotRecorded> {
        let history = match self {
            Self::Arcform => LocalHistory::open_default().map_err(|e| {
                NotRecorded::new(format!("the history folder cannot be opened: {e}"))
            })?,
            Self::At(root) => LocalHistory::at_root(root),
        };
        let way = HistoryWay::new(WAY)
            .map_err(|e| NotRecorded::new(format!("the history cannot name brightfield: {e}")))?;
        let history = history.reached_by(way);
        std::fs::create_dir_all(history.root()).map_err(|e| {
            NotRecorded::new(format!(
                "the history folder {} cannot be opened: {e}",
                history.root().display()
            ))
        })?;
        Ok(history)
    }
}

/// One Save's claim on the versions of one chart file: begun before the write,
/// finished after it.
#[derive(Debug)]
pub struct ChartVersions {
    file: PathBuf,
    /// `None` when the store could not be opened.
    history: Option<LocalHistory>,
    /// The first thing that went wrong, if anything has.
    failure: Option<NotRecorded>,
    /// The replaced text is held by a save a merging save would fold it into.
    fold_risk: bool,
}

impl ChartVersions {
    /// Record `replaced`, the text on disk the coming write replaces, as a
    /// checkpoint in `file`'s own history. `None` is a write that replaces no
    /// text, the first Save of a chart file.
    ///
    /// Call it before the write. It returns no error: a store that cannot be opened
    /// or an entry that cannot be written is held and answered by
    /// [`Self::finish`].
    #[must_use]
    pub fn begin(store: &HistoryStore, file: &Path, replaced: Option<&str>) -> Self {
        let mut versions = Self {
            file: file.to_path_buf(),
            history: None,
            failure: None,
            fold_risk: false,
        };
        match store.open() {
            Ok(history) => versions.history = Some(history),
            Err(e) => {
                versions.failure = Some(e);
                return versions;
            }
        }
        if let Some(text) = replaced {
            versions.keep_replaced(text);
        }
        versions
    }

    fn keep_replaced(&mut self, text: &str) {
        let Some(history) = &self.history else {
            return;
        };
        match history.record_checkpoint_for_file(&self.file, text) {
            Ok(Some(_)) => {}
            // The newest entry already holds this text. If that entry is a
            // save the next save would merge over, the text would go with it.
            Ok(None) => match history.entries_for_file(&self.file) {
                Ok(entries) => {
                    self.fold_risk = entries
                        .last()
                        .is_some_and(|newest| would_merge_over(newest.kind, newest.at));
                }
                Err(e) => self.fail(&e),
            },
            Err(e) => self.fail(&e),
        }
    }

    /// Record `written`, the text the Save wrote, and answer whether the
    /// versions of this Save were all recorded.
    ///
    /// # Errors
    ///
    /// [`NotRecorded`] with the first reason, for a store that could not be
    /// opened or an entry that could not be written.
    pub fn finish(mut self, written: &str) -> Result<(), NotRecorded> {
        if let Some(history) = &self.history {
            let recorded = if self.fold_risk {
                history.record_checkpoint_for_file(&self.file, written)
            } else {
                history.record_save_for_file(&self.file, written)
            };
            if let Err(e) = recorded {
                self.fail(&e);
            }
        }
        match self.failure {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    }

    fn fail(&mut self, error: &dyn std::fmt::Display) {
        if self.failure.is_none() {
            self.failure = Some(NotRecorded::new(error.to_string()));
        }
    }
}

/// Whether a save recorded now would merge into an entry of `kind` recorded at
/// `at`: arcform's rule, which merges a save into a save no older than its
/// window.
///
/// A clock that went backwards reads as inside the window, the safe side: the
/// text is then recorded as a checkpoint, which only costs the entry its kind.
fn would_merge_over(kind: HistoryKind, at: SystemTime) -> bool {
    kind == HistoryKind::Save
        && SystemTime::now()
            .duration_since(at)
            .map_or(true, |gap| gap <= HISTORY_MERGE_WINDOW)
}
