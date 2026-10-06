//! A chart file's versions, kept in arcform's local history.
//!
//! brightfield writes a chart file beside a Protocol's spec, and until this
//! module nothing kept the text a Save replaced: after two Saves the first
//! Save's text was gone. arcform's local history keeps each file's own list of
//! versions, keyed by the file's path ([`LocalHistory::record_save_for_file`]
//! and its siblings), and this module is the one place brightfield calls it.
//!
//! **Three kinds of entry, all for the chart file only.** The text a Save
//! replaces is recorded as a checkpoint before the write, and the text it
//! writes is recorded as a save after. The text a Save would have written,
//! when a window closes without saving, is recorded as arcform's unsaved kind
//! ([`record_unsaved`]), which leaves the chart file as it was. The Protocol's
//! own `arcform.yaml` is not recorded here, so its history lists what it listed
//! before.
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
//! Opening the store makes its root when the root is missing.
//!
//! **Listing and reading back.** [`HistoryStore::versions`] lists a chart
//! file's versions newest first, each with what changed since the version
//! before it, and [`HistoryStore::read_version`] returns one version's text.
//! Neither writes under the chart file's folder: a listing reads the store,
//! whose root opening it makes when that is missing, and the chart file's
//! folder is not touched. The change is data, a [`ChartChange`], and the words for it are
//! the window's.
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

use arc::spec::{HISTORY_MAX_ENTRIES, HISTORY_MERGE_WINDOW};
use brightfield_spec::layout::collect_plot_nodes;
use brightfield_spec::{
    parse_spec, serialise_value, Component, Format, Mark, PlotNode, Spec, SpecValue,
    ValueOrParamRef,
};

// Re-exported so a caller that reads the store back, as a test does, names
// arcform's types through this crate and not through a second `arc` pin.
pub use arc::spec::{HistoryEntry, HistoryKind, HistoryWay, LocalHistory};

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

/// Why a chart file's versions could not be listed or one read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotListed {
    reason: String,
}

impl NotListed {
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

impl std::fmt::Display for NotListed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for NotListed {}

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

/// **Record `text`, the chart as a Save would have written it, as a version of
/// the chart file at `file` that was never written to it**: arcform's unsaved
/// kind, recorded when a window closes without saving.
///
/// The chart file keeps its bytes, and a chart file that is not there is not
/// made. arcform keys a file's versions by its folder's canonical path, so a
/// folder that is not there yet, as `panels/` is before a Protocol's first
/// Save, is made empty to key the version by, and taken away again when the
/// version could not be recorded. A text the newest version already holds is
/// not recorded again, and that is not a failure: the store holds it.
///
/// # Errors
///
/// [`NotRecorded`] with the reason, for a store that cannot be opened, a
/// folder that cannot be made, and an entry that cannot be written.
pub fn record_unsaved(store: &HistoryStore, file: &Path, text: &str) -> Result<(), NotRecorded> {
    let history = store.open()?;
    let folder = file
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty());
    let made = match folder {
        Some(folder) if !folder.exists() => {
            std::fs::create_dir(folder).map_err(|e| {
                NotRecorded::new(format!("the folder {} cannot be made: {e}", folder.display()))
            })?;
            Some(folder)
        }
        _ => None,
    };
    match history.record_unsaved_for_file(file, text) {
        Ok(_) => Ok(()),
        Err(e) => {
            if let Some(folder) = made {
                // Only the folder made here, and only while it is empty.
                let _ = std::fs::remove_dir(folder);
            }
            Err(NotRecorded::new(e.to_string()))
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

// ------------------------------------------------------------ listing a file's versions

/// A chart file's versions as the store holds them, newest first.
#[derive(Debug, Clone, PartialEq)]
pub struct VersionList {
    /// The versions kept for the file, the newest first.
    pub versions: Vec<ChartVersion>,
    /// The most versions the store keeps for one file; recording past it
    /// drops the file's oldest.
    pub bound: usize,
    /// The store's folder. arcform's store is outside the chart file's own
    /// folder; a store rooted at a chosen folder ([`HistoryStore::At`]) is
    /// wherever that folder is, inside the chart file's own or not.
    pub folder: PathBuf,
}

impl VersionList {
    /// How many versions are kept for the file.
    #[must_use]
    pub fn kept(&self) -> usize {
        self.versions.len()
    }
}

/// One version of a chart file: what the store recorded, and what changed in
/// it since the version before.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartVersion {
    /// The id that [`HistoryStore::read_version`] reads this version by.
    pub id: String,
    /// A save, or the text before a write, as the store recorded it. A Save
    /// within the merge window of the Save before it is recorded as the text
    /// before a write ([`ChartVersions::finish`]), and the store does not say
    /// which it was.
    pub kind: HistoryKind,
    /// The way it was reached: `brightfield`, `terminal`, `mcp`, or no way for
    /// an entry written by a handle that was not given one.
    pub way: Option<String>,
    /// The time the store recorded it, to the millisecond.
    pub at: SystemTime,
    /// The version's size in bytes.
    pub bytes: u64,
    /// What changed since the version before.
    pub change: VersionChange,
}

/// What changed in a version since the version before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionChange {
    /// The oldest version kept: no version before it to compare with.
    NoPredecessor,
    /// The changes since the version before, in the order the chart reads: each
    /// plot in turn, its attributes and then its marks' channels, and the
    /// edit outside them last. Empty when the two texts are the same.
    Changes(Vec<ChartChange>),
}

/// One change between two versions of a chart file. A value is as the chart
/// writes it: a column on a channel is the column's name, and `None` is a
/// channel or attribute the chart does not carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChartChange {
    /// A mark's channel moved to another column, gained one, or lost it.
    Channel {
        /// The plot's path, as `root/hconcat[0]/vconcat[1]` names it.
        plot: String,
        /// The mark's ordinal among the plot's marks, from 0.
        mark: usize,
        /// The channel's key, as the chart spells it (`x`).
        channel: String,
        /// The column before.
        before: Option<String>,
        /// The column after.
        after: Option<String>,
    },
    /// A plot's own attribute changed, was added, or was taken out.
    PlotAttribute {
        /// The plot's path, as [`Self::Channel`] names it.
        plot: String,
        /// The attribute's key, as the chart spells it (`yScale`).
        key: String,
        /// The value before.
        before: Option<String>,
        /// The value after.
        after: Option<String>,
    },
    /// The text changed somewhere other than a channel or a plot attribute: a
    /// comment, a mark added or removed, the data, or a text one of the two
    /// versions could not be read as a chart from.
    EditedOutsideChannels {
        /// How many lines changed. The two texts are compared by longest
        /// common subsequence, and a run of lines taken out and put in between
        /// lines that stay counts as the larger of the two numbers, so three
        /// comment lines changed in place are three.
        lines: usize,
    },
}

impl HistoryStore {
    /// List the versions kept for the chart file at `file`, the newest first,
    /// each with what changed since the version before it.
    ///
    /// A file with no recorded version lists none. The listing reads the store
    /// and each version's text and writes nothing under the chart file's
    /// folder.
    ///
    /// **What a change is.** Two versions are parsed as charts. A plot is the
    /// same plot in both when its path is the same, and a mark is the same mark
    /// when its ordinal among the plot's marks is the same. For each such
    /// plot, each attribute and each channel whose value differs is a
    /// [`ChartChange`] naming it. The text changed outside them, and is
    /// reported as [`ChartChange::EditedOutsideChannels`], when the two charts
    /// differ somewhere other than those attributes and channels (a mark
    /// added, the data, a legend), when no attribute or channel differs and the
    /// text does, or when either text does not parse as a chart. When a
    /// version carries both kinds, its count is of the lines that differ,
    /// the lines of the named changes included. A comment edited beside a
    /// named change is not counted: the chart does not carry it, so a comment
    /// is reported by a version whose named changes are empty.
    ///
    /// # Errors
    ///
    /// [`NotListed`] with the reason the store gives for a store that cannot be
    /// opened, and with the reason reading gives for a chart file whose folder
    /// does not exist or a version whose text cannot be read.
    pub fn versions(&self, file: &Path) -> Result<VersionList, NotListed> {
        let history = self.open().map_err(|e| NotListed::new(e.reason()))?;
        let entries = history
            .entries_for_file(file)
            .map_err(|e| NotListed::new(e.to_string()))?;
        let mut texts = Vec::with_capacity(entries.len());
        for entry in &entries {
            texts.push(
                history
                    .read_for_file(file, &entry.id)
                    .map_err(|e| NotListed::new(e.to_string()))?,
            );
        }
        let mut versions = Vec::with_capacity(entries.len());
        for (i, entry) in entries.iter().enumerate() {
            let change = match i.checked_sub(1) {
                None => VersionChange::NoPredecessor,
                Some(before) => VersionChange::Changes(changes_between(&texts[before], &texts[i])),
            };
            versions.push(ChartVersion {
                id: entry.id.clone(),
                kind: entry.kind,
                way: entry.way.as_ref().map(|way| way.as_str().to_string()),
                at: entry.at,
                bytes: entry.bytes,
                change,
            });
        }
        versions.reverse();
        Ok(VersionList {
            versions,
            bound: HISTORY_MAX_ENTRIES,
            folder: history.root().to_path_buf(),
        })
    }

    /// The exact bytes the version `id` of the chart file at `file` recorded,
    /// read from the id [`Self::versions`] lists.
    ///
    /// # Errors
    ///
    /// [`NotListed`] with the reason the store gives for a store that cannot be
    /// opened, and with the reason reading gives for an id the file has no
    /// version under.
    pub fn read_version(&self, file: &Path, id: &str) -> Result<String, NotListed> {
        let history = self.open().map_err(|e| NotListed::new(e.reason()))?;
        history
            .read_for_file(file, id)
            .map_err(|e| NotListed::new(e.to_string()))
    }
}

/// What changed from `before` to `after`, as [`HistoryStore::versions`] says.
///
/// Public so a surface that holds a text no version records yet, the chart as
/// an unsaved edit would write it, names its change by the same rule a listed
/// version does.
#[must_use]
pub fn changes_between(before: &str, after: &str) -> Vec<ChartChange> {
    let parsed = (
        parse_spec(before, Format::Yaml),
        parse_spec(after, Format::Yaml),
    );
    let (Ok(old), Ok(new)) = parsed else {
        return outside(before, after);
    };
    let (old, new) = (old.spec, new.spec);
    let mut changes = named_changes(&old, &new);
    let beyond = without_channels(&old) != without_channels(&new);
    if beyond || (changes.is_empty() && before != after) {
        changes.extend(outside(before, after));
    }
    changes
}

/// The one change of a text that differs beyond what the chart names, or an
/// empty list when the texts are the same.
fn outside(before: &str, after: &str) -> Vec<ChartChange> {
    if before == after {
        return Vec::new();
    }
    vec![ChartChange::EditedOutsideChannels {
        lines: lines_changed(before, after),
    }]
}

/// Every plot attribute and mark channel that differs between two charts, for
/// the plots and marks the two share.
fn named_changes(old: &Spec, new: &Spec) -> Vec<ChartChange> {
    let new_plots = collect_plot_nodes(new);
    let mut changes = Vec::new();
    for (path, old_plot) in collect_plot_nodes(old) {
        let Some((_, new_plot)) = new_plots.iter().find(|(p, _)| *p == path) else {
            continue;
        };
        for (key, before, after) in diff_keys(
            old_plot.attributes.iter().map(|(k, v)| (k, show(v))),
            new_plot.attributes.iter().map(|(k, v)| (k, show(v))),
        ) {
            changes.push(ChartChange::PlotAttribute {
                plot: path.clone(),
                key,
                before,
                after,
            });
        }
        for (ordinal, (old_mark, new_mark)) in marks(old_plot).zip(marks(new_plot)).enumerate() {
            for (channel, before, after) in diff_keys(
                old_mark.options.iter().map(|(k, v)| (k, show_option(v))),
                new_mark.options.iter().map(|(k, v)| (k, show_option(v))),
            ) {
                changes.push(ChartChange::Channel {
                    plot: path.clone(),
                    mark: ordinal,
                    channel,
                    before,
                    after,
                });
            }
        }
    }
    changes
}

/// The marks among a plot's items, in order. A legend or an interactor is not
/// one, which is how a [`ChartEdit`](brightfield_spec::edit::ChartEdit) counts
/// a mark's ordinal.
fn marks(plot: &PlotNode) -> impl Iterator<Item = &Mark> {
    plot.items.iter().filter_map(|item| match item {
        Component::Mark(mark) => Some(mark),
        _ => None,
    })
}

/// The keys whose shown value differs between two maps, with the value before
/// and after: the keys of the first map in its order, then the keys the
/// second map adds.
fn diff_keys<'a>(
    old: impl Iterator<Item = (&'a String, String)>,
    new: impl Iterator<Item = (&'a String, String)>,
) -> Vec<(String, Option<String>, Option<String>)> {
    let old: Vec<(&String, String)> = old.collect();
    let new: Vec<(&String, String)> = new.collect();
    let value_in = |side: &[(&String, String)], key: &String| {
        side.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, shown)| shown.clone())
    };
    let keys = old.iter().map(|(k, _)| *k).chain(
        new.iter()
            .map(|(k, _)| *k)
            .filter(|k| value_in(&old, k).is_none()),
    );
    keys.filter_map(|key| {
        let (before, after) = (value_in(&old, key), value_in(&new, key));
        (before != after).then(|| (key.clone(), before, after))
    })
    .collect()
}

/// A value as the chart writes it: a string is itself, which makes a column the
/// column's name.
fn show(value: &SpecValue) -> String {
    match value {
        SpecValue::String(text) => text.clone(),
        SpecValue::Param(param) => param.to_wire(),
        SpecValue::Aggregate { func, column } => {
            format!("{}({})", func.wire_name(), column.as_deref().unwrap_or(""))
        }
        other => serialise_value(other)
            .map_or_else(|_| format!("{other:?}"), |text| text.trim().to_string()),
    }
}

fn show_option(value: &ValueOrParamRef<SpecValue>) -> String {
    match value {
        ValueOrParamRef::Value(value) => show(value),
        ValueOrParamRef::Param(param) => param.to_wire(),
    }
}

/// `spec` with the plot attributes and mark channels that
/// [`named_changes`] compares taken out, so what remains is what the two
/// charts must share for the named changes to be their whole difference.
fn without_channels(spec: &Spec) -> Spec {
    let mut stripped = spec.clone();
    if let Some(root) = &mut stripped.root {
        strip(root);
    }
    stripped
}

/// Takes the attributes and channels out of the plots [`collect_plot_nodes`]
/// reaches, which are the plots [`named_changes`] pairs by path.
fn strip(component: &mut Component) {
    match component {
        Component::Plot(plot) => {
            plot.attributes.clear();
            for item in &mut plot.items {
                if let Component::Mark(mark) = item {
                    mark.options.clear();
                }
            }
        }
        Component::HConcat(concat) | Component::VConcat(concat) => {
            concat.items.iter_mut().for_each(strip);
        }
        _ => {}
    }
}

/// The most cells the line comparison builds before it stops comparing.
const LINE_COMPARE_CELLS: usize = 4_000_000;

/// How many lines changed from `before` to `after`.
///
/// The lines are compared by longest common subsequence, so a line added or
/// taken out does not read as every line below it changing. The lines that
/// differ fall into runs between lines that stay, and a run of `r` lines taken
/// out and `a` lines put in counts as the larger of the two: a line changed in
/// place is one line, and three comment lines changed in place are three.
///
/// Past [`LINE_COMPARE_CELLS`] cells of comparison, the lines between the first
/// and the last that differ count as one run.
fn lines_changed(before: &str, after: &str) -> usize {
    let (a, b): (Vec<&str>, Vec<&str>) = (before.lines().collect(), after.lines().collect());
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a[head..]
        .iter()
        .rev()
        .zip(b[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (a, b) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    if a.len().saturating_mul(b.len()) > LINE_COMPARE_CELLS {
        return a.len().max(b.len());
    }
    let width = b.len() + 1;
    let mut common = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            common[i * width + j] = if a[i] == b[j] {
                common[(i + 1) * width + j + 1] + 1
            } else {
                common[(i + 1) * width + j].max(common[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let (mut taken, mut put, mut total) = (0, 0, 0);
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            total += taken.max(put);
            (taken, put) = (0, 0);
            i += 1;
            j += 1;
        } else if j == b.len()
            || (i < a.len() && common[(i + 1) * width + j] >= common[i * width + j + 1])
        {
            taken += 1;
            i += 1;
        } else {
            put += 1;
            j += 1;
        }
    }
    total + taken.max(put)
}
