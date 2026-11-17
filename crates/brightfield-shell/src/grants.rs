//! The folders the analyst has granted Brightfield, kept across launches: the grant store.
//!
//! Under App Sandbox the managed edition reaches only what the analyst hands it: a folder or file
//! chosen in a panel, opened from Finder or dropped on the app. That reach ends when the app quits.
//! The store keeps each grant as a security-scoped bookmark in `grants.json`, in the config folder
//! beside the trust store, and at launch resolves every bookmark and starts access to it, so a
//! Protocol opened before a relaunch opens after it without asking again.
//!
//! Every grant is read-write, because Brightfield writes into the folders it opens. A grant on a
//! folder covers everything inside it; a grant on a file covers that file alone.
//!
//! **What a launch does with each bookmark.**
//!
//! - It resolves: access starts. If the folder was renamed or moved in the Finder, the bookmark
//!   follows it, and the store saves the folder's new path and a fresh bookmark.
//! - It does not resolve, and the drive it was on is not mounted: the grant is kept and reported
//!   as [`GrantError::Unmounted`], so the window can say the Protocol needs its drive. Once the
//!   drive is back, the next use of the grant resolves it again.
//! - It does not resolve otherwise, as when the folder was deleted: the grant is dropped and
//!   reported as [`GrantError::Unresolvable`], and the folder needs a grant the next time it is
//!   used.
//!
//! **Children.** A sandboxed app's child processes inherit the grants the app holds when it starts
//! them (`docs/app-sandbox.md`), so nothing is handed to arc, the DuckDB CLI or dovetail. What the
//! app must do is keep access started while a child uses it: access is started at launch and never
//! stopped, and a run or a join proposal takes a [`GrantHold`] on its folders for as long as it
//! goes, so a later change to the store cannot end a grant a child is using.
//!
//! **Editions.** The store works the same in either edition, but only the managed edition reads
//! through it: in the full edition [`Grants::needs`] is always empty, so a caller needs no edition
//! check of its own.
//!
//! **Where the store is.** The app loads it from its config folder at launch
//! ([`load_at_launch`]). A process that has not, such as a test, keeps the store in the config
//! folder `BRIGHTFIELD_CONFIG_DIR` names when it is set, and otherwise in memory.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use serde_json::{json, Value};

use crate::edition::{self, Capability};

/// The file in the config folder that keeps the grant store.
pub const GRANTS_FILE: &str = "grants.json";

/// Why a granted folder cannot be reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantError {
    /// The grant on this folder no longer resolves, as when the folder was deleted, and was
    /// dropped: the folder needs a grant again.
    Unresolvable(PathBuf),
    /// The grant on this folder is kept, but the drive it is on is not mounted.
    Unmounted(PathBuf),
    /// The grant could not be made or saved: the system's or the file's description.
    Io(String),
}

impl std::fmt::Display for GrantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GrantError::Unresolvable(p) => {
                write!(f, "{} can no longer be found; grant it again", p.display())
            }
            GrantError::Unmounted(p) => {
                write!(f, "{} is on a drive that is not connected", p.display())
            }
            GrantError::Io(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for GrantError {}

/// One granted folder or file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// Where it was when the bookmark last resolved, or when it was granted.
    path: PathBuf,
    /// Where the volume it is on was mounted, when it is not the start-up disk's root.
    volume: Option<PathBuf>,
    /// The security-scoped bookmark.
    bookmark: Vec<u8>,
    /// Whether the bookmark has resolved in this process and access to it has started.
    open: bool,
}

/// What the store holds behind its lock.
#[derive(Debug, Default)]
struct Inner {
    entries: Vec<Entry>,
    /// What the last load found it could not reach.
    problems: Vec<GrantError>,
    /// How many holds stand on each folder.
    holds: BTreeMap<PathBuf, usize>,
}

/// The grant store.
#[derive(Debug)]
pub struct Grants {
    /// The file, or `None` for this process's memory.
    file: Option<PathBuf>,
    inner: Mutex<Inner>,
}

/// Keeps access to some granted folders started until it is dropped: what a run or a join
/// proposal takes on the folders it reads and writes.
#[derive(Debug)]
#[must_use = "the hold ends when the guard is dropped"]
pub struct GrantHold<'a> {
    grants: &'a Grants,
    folders: Vec<PathBuf>,
}

impl Drop for GrantHold<'_> {
    fn drop(&mut self) {
        let mut inner = self.grants.lock();
        for folder in &self.folders {
            if let Some(n) = inner.holds.get_mut(folder) {
                *n -= 1;
                if *n == 0 {
                    inner.holds.remove(folder);
                }
            }
        }
    }
}

impl Grants {
    /// The store kept in `config_dir`, with every bookmark in it resolved and its access started.
    /// A grant that moved is followed and saved where it is now; one that no longer resolves is
    /// dropped, and one whose drive is not mounted is kept. Both are in [`Grants::problems`].
    #[must_use]
    pub fn load(config_dir: &Path) -> Self {
        let file = config_dir.join(GRANTS_FILE);
        let mut problems = Vec::new();
        let entries = match read_file(&file) {
            Ok(entries) => entries,
            Err(e) => {
                problems.push(GrantError::Io(e));
                Vec::new()
            }
        };
        let grants = Self {
            file: Some(file),
            inner: Mutex::new(Inner {
                entries,
                problems,
                holds: BTreeMap::new(),
            }),
        };
        {
            let mut inner = grants.lock();
            let mut changed = false;
            let mut kept = Vec::new();
            for mut entry in std::mem::take(&mut inner.entries) {
                match open(&mut entry) {
                    Opened::Same => kept.push(entry),
                    Opened::Moved => {
                        changed = true;
                        kept.push(entry);
                    }
                    Opened::Unmounted => {
                        inner
                            .problems
                            .push(GrantError::Unmounted(entry.path.clone()));
                        kept.push(entry);
                    }
                    Opened::Gone => {
                        changed = true;
                        inner.problems.push(GrantError::Unresolvable(entry.path));
                    }
                }
            }
            inner.entries = kept;
            if changed {
                if let Err(e) = grants.save(&inner) {
                    inner.problems.push(e);
                }
            }
        }
        grants
    }

    /// A store kept in this process's memory, for a process that names no config folder.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            file: None,
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Grant `folder`, which this process must be able to reach now (from a panel, Finder or a
    /// drop), and save the grant. Granting a folder already granted refreshes its bookmark.
    ///
    /// # Errors
    ///
    /// [`GrantError::Io`] when the folder is not there, the bookmark cannot be made, or the store
    /// cannot be saved.
    pub fn grant(&self, folder: &Path) -> Result<(), GrantError> {
        let path = std::fs::canonicalize(folder)
            .map_err(|e| GrantError::Io(format!("{}: {e}", folder.display())))?;
        let bookmark = platform::make_bookmark(&path).map_err(GrantError::Io)?;
        let mut entry = Entry {
            volume: mount_point(&path),
            path,
            bookmark,
            open: false,
        };
        if let Opened::Unmounted | Opened::Gone = open(&mut entry) {
            return Err(GrantError::Io(format!(
                "{}: the grant did not resolve once made",
                entry.path.display()
            )));
        }
        let mut inner = self.lock();
        inner.entries.retain(|e| e.path != entry.path);
        inner.entries.push(entry);
        self.save(&inner)
    }

    /// The granted folder at or above `path`, the nearest one when grants nest. A grant whose drive
    /// was not mounted is tried again here, so a Protocol on a drive plugged back in opens.
    #[must_use]
    pub fn covering(&self, path: &Path) -> Option<PathBuf> {
        let path = canonical(path);
        let mut inner = self.lock();
        let mut changed = false;
        for entry in inner.entries.iter_mut() {
            if !entry.open && path.starts_with(&entry.path) && open(entry) == Opened::Moved {
                changed = true;
            }
        }
        if changed {
            // A failed save leaves the grant open in this process and the old path on disk,
            // which the next launch follows again.
            let _ = self.save(&inner);
        }
        inner
            .entries
            .iter()
            .filter(|e| e.open && path.starts_with(&e.path))
            .max_by_key(|e| e.path.components().count())
            .map(|e| e.path.clone())
    }

    /// Why `path` is out of reach though a grant names it: its drive is not mounted, or, with the
    /// drive back, the grant no longer resolves. `None` when a grant covers it, or none names it.
    #[must_use]
    pub fn unreachable(&self, path: &Path) -> Option<GrantError> {
        if self.covering(path).is_some() {
            return None;
        }
        let path = canonical(path);
        self.lock()
            .entries
            .iter()
            .find(|e| path.starts_with(&e.path))
            .map(|e| match &e.volume {
                Some(volume) if !is_mounted(volume) => GrantError::Unmounted(e.path.clone()),
                _ => GrantError::Unresolvable(e.path.clone()),
            })
    }

    /// The folders still to grant before `paths` can be read: for each path no grant covers, the
    /// path itself when it is a folder and the folder it is in otherwise, each named once. Always
    /// empty in an edition that reads ungranted paths.
    #[must_use]
    pub fn needs(&self, paths: &[PathBuf]) -> Vec<PathBuf> {
        if edition::current().allows(Capability::UngrantedPaths) {
            return Vec::new();
        }
        let mut asked: Vec<PathBuf> = Vec::new();
        for path in paths {
            if self.covering(path).is_some() {
                continue;
            }
            let path = canonical(path);
            let folder = if path.is_dir() {
                path
            } else {
                match path.parent() {
                    Some(parent) => parent.to_path_buf(),
                    None => path,
                }
            };
            if !asked.iter().any(|a| folder.starts_with(a)) {
                asked.retain(|a| !a.starts_with(&folder));
                asked.push(folder);
            }
        }
        asked
    }

    /// Keep access to `folders` started until the returned guard drops. A folder on a drive
    /// plugged back in since launch is opened again first.
    pub fn hold(&self, folders: &[PathBuf]) -> GrantHold<'_> {
        let folders: Vec<PathBuf> = folders.iter().map(|f| canonical(f)).collect();
        for folder in &folders {
            let _ = self.covering(folder);
        }
        let mut inner = self.lock();
        for folder in &folders {
            *inner.holds.entry(folder.clone()).or_default() += 1;
        }
        GrantHold {
            grants: self,
            folders,
        }
    }

    /// The folders a hold stands on now.
    #[must_use]
    pub fn held(&self) -> Vec<PathBuf> {
        self.lock().holds.keys().cloned().collect()
    }

    /// What the load found it could not reach, and why.
    #[must_use]
    pub fn problems(&self) -> Vec<GrantError> {
        self.lock().problems.clone()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn save(&self, inner: &Inner) -> Result<(), GrantError> {
        match &self.file {
            Some(file) => write_file(file, &inner.entries).map_err(GrantError::Io),
            None => Ok(()),
        }
    }
}

/// The process-wide store.
static STORE: OnceLock<Grants> = OnceLock::new();

/// Load the grant store from the app's config folder and start access to every grant in it: what
/// the app does at launch, before anything reads a Protocol. A second call changes nothing.
pub fn load_at_launch() {
    let dir = brightfield_workbench::persist::config_dir(
        std::env::var(crate::startup::CONFIG_DIR_VAR)
            .ok()
            .as_deref(),
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    );
    let _ = STORE.get_or_init(|| dir.map_or_else(Grants::in_memory, |d| Grants::load(&d)));
}

/// The store this process uses: the one the app loaded at launch, else the one in the config
/// folder `BRIGHTFIELD_CONFIG_DIR` names, else one in memory.
pub fn store() -> &'static Grants {
    STORE.get_or_init(|| {
        std::env::var(crate::startup::CONFIG_DIR_VAR)
            .ok()
            .filter(|d| !d.is_empty())
            .map_or_else(Grants::in_memory, |d| Grants::load(Path::new(&d)))
    })
}

/// What resolving an entry's bookmark came to.
#[derive(Debug, PartialEq, Eq)]
enum Opened {
    /// It resolved where it was.
    Same,
    /// It resolved somewhere else, or was stale, and the entry now holds the new path and bookmark.
    Moved,
    /// It did not resolve, and its drive is not mounted.
    Unmounted,
    /// It did not resolve.
    Gone,
}

/// Resolve `entry`'s bookmark, start access, and follow it if it moved.
fn open(entry: &mut Entry) -> Opened {
    match platform::resolve_and_start(&entry.bookmark) {
        Ok((path, stale)) => {
            entry.open = true;
            let path = canonical(&path);
            if path == entry.path && !stale {
                return Opened::Same;
            }
            // Made again from where the folder is now, which the access just started can reach.
            if let Ok(bookmark) = platform::make_bookmark(&path) {
                entry.bookmark = bookmark;
            }
            entry.volume = mount_point(&path);
            entry.path = path;
            Opened::Moved
        }
        Err(_) => {
            entry.open = false;
            match &entry.volume {
                Some(volume) if !is_mounted(volume) => Opened::Unmounted,
                _ => Opened::Gone,
            }
        }
    }
}

/// `path` with its nearest existing ancestor's symbolic links resolved, and the rest kept as
/// written, so `/var/…` and `/private/var/…` compare equal whether or not the file exists yet.
fn canonical(path: &Path) -> PathBuf {
    let mut tail = Vec::new();
    let mut at = path;
    loop {
        if let Ok(real) = std::fs::canonicalize(at) {
            return tail.iter().rev().fold(real, |p, c| p.join(c));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                at = parent;
            }
            _ => return lexical(path),
        }
    }
}

/// `path` with `.` and `..` removed by their names alone.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Where the volume holding `path` is mounted: the nearest ancestor on a different device from its
/// own parent. `None` for the root, and where the system gives no device numbers.
fn mount_point(path: &Path) -> Option<PathBuf> {
    let mut at = path;
    while let Some(parent) = at.parent() {
        if device(at)? != device(parent)? {
            return Some(at.to_path_buf());
        }
        at = parent;
    }
    None
}

/// Whether a volume is mounted at `point`: it is there and on a device its parent is not on.
fn is_mounted(point: &Path) -> bool {
    match (device(point), point.parent().and_then(device)) {
        (Some(here), Some(above)) => here != above,
        _ => false,
    }
}

#[cfg(unix)]
fn device(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| m.dev())
}

#[cfg(not(unix))]
fn device(_path: &Path) -> Option<u64> {
    None
}

/// Read the entries in `file`; none when it does not exist.
fn read_file(file: &Path) -> Result<Vec<Entry>, String> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", file.display())),
    };
    let value: Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    let Some(list) = value["grants"].as_array() else {
        return Ok(Vec::new());
    };
    Ok(list
        .iter()
        .filter_map(|g| {
            Some(Entry {
                path: PathBuf::from(g["path"].as_str()?),
                volume: g["volume"].as_str().map(PathBuf::from),
                bookmark: from_hex(g["bookmark"].as_str()?)?,
                open: false,
            })
        })
        .collect())
}

/// Write `entries` to `file`, through a file beside it renamed into place, so a write cut short
/// leaves the old store whole.
fn write_file(file: &Path, entries: &[Entry]) -> Result<(), String> {
    let value = json!({
        "grants": entries.iter().map(|e| {
            let mut v = json!({ "path": e.path.to_string_lossy(), "bookmark": to_hex(&e.bookmark) });
            if let Some(volume) = &e.volume {
                v["volume"] = json!(volume.to_string_lossy());
            }
            v
        }).collect::<Vec<_>>(),
    });
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())? + "\n";
    let partial = file.with_extension("json.partial");
    std::fs::write(&partial, text).map_err(|e| format!("{}: {e}", partial.display()))?;
    std::fs::rename(&partial, file).map_err(|e| format!("{}: {e}", file.display()))
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(target_os = "macos")]
use crate::mac_grants as platform;

/// Off the Mac there is no sandbox to extend, so a bookmark is the path itself.
#[cfg(not(target_os = "macos"))]
mod platform {
    use std::path::{Path, PathBuf};

    pub fn make_bookmark(path: &Path) -> Result<Vec<u8>, String> {
        Ok(path.to_string_lossy().into_owned().into_bytes())
    }

    pub fn resolve_and_start(data: &[u8]) -> Result<(PathBuf, bool), String> {
        let path = PathBuf::from(String::from_utf8_lossy(data).into_owned());
        if path.exists() {
            Ok((path, false))
        } else {
            Err(format!("{}: no such file or folder", path.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let bytes = vec![0, 1, 0x7f, 0x80, 0xff];
        assert_eq!(from_hex(&to_hex(&bytes)), Some(bytes));
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("zz"), None);
    }

    #[test]
    fn canonical_keeps_a_tail_that_does_not_exist_yet() {
        let tmp = tempfile::tempdir().expect("a scratch folder");
        let real = std::fs::canonicalize(tmp.path()).expect("it resolves");
        assert_eq!(
            canonical(&tmp.path().join("not/yet")),
            real.join("not").join("yet")
        );
    }
}
