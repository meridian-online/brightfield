//! The folder new Protocols are made in, and how a new one is named.
//!
//! An open makes a Protocol, so they accumulate somewhere the analyst can see and reach with
//! the Finder: `~/Documents/Protocols`, or the folder chosen in Preferences, or the folder
//! `BRIGHTFIELD_PROTOCOLS_DIR` names. Tests point the variable, or the setting, at a temporary
//! folder so none of them writes under the home directory.
//!
//! The setting is kept in its own small file in the config directory, beside the layout file,
//! rather than in the layout: the layout is read once at launch and written by the window, while
//! the Protocols folder is read wherever a Protocol is made. A build that predates the setting
//! never reads the file, and a rewrite of one field keeps every other, so a later build's fields
//! survive an earlier build's change of folder.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde_json::{Map, Value};

use crate::rename::FolderOps;

/// The environment variable that relocates the Protocols folder, for tests and portable installs.
pub const PROTOCOLS_DIR_VAR: &str = "BRIGHTFIELD_PROTOCOLS_DIR";

/// Why a Protocol's folder could not be claimed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimError {
    /// The system refused the write in this folder, which on macOS means Brightfield has not been
    /// given access to it.
    Refused(PathBuf),
    /// A failure other than a refusal, in the file system's words.
    Io(String),
}

impl fmt::Display for ClaimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(parent) => write!(
                f,
                "Brightfield cannot make a Protocol in {}. Allow it under System Settings → \
                 Privacy & Security → Files and Folders, then open the file again.",
                parent.display()
            ),
            Self::Io(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ClaimError {}

/// The file in the config directory that keeps the Preferences.
pub const PREFERENCES_FILE: &str = "preferences.json";

/// The field of the Preferences file that holds the Protocols folder the analyst chose.
const PROTOCOLS_DIR_FIELD: &str = "protocols_dir";

/// The file an unreadable Preferences file is set aside as, once a folder is chosen again.
pub const UNREADABLE_SUFFIX: &str = "unreadable";

/// How the Preferences file was found when it was read: its path, the moment and size it had
/// then, and the setting it held, or why it could not be read.
type Held = (
    PathBuf,
    Option<(std::time::SystemTime, u64)>,
    Result<Option<PathBuf>, String>,
);

/// The setting as last read or written, so the door, which asks for the folder on each frame it
/// draws, does not read the file each time. A file changed since it was read is read again.
static SETTING: Mutex<Option<Held>> = Mutex::new(None);

/// Where the Preferences file is: in the config directory the layout file is in, which
/// `BRIGHTFIELD_CONFIG_DIR` moves. `None` on a machine with no config directory.
fn preferences_path() -> Option<PathBuf> {
    brightfield_workbench::persist::config_dir(
        std::env::var(crate::startup::CONFIG_DIR_VAR)
            .ok()
            .as_deref(),
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
    .map(|dir| dir.join(PREFERENCES_FILE))
}

/// The moment the file at `path` was last changed and its size; `None` for a missing file.
fn stamp(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// The Preferences file's fields, read as an object so that writing one field back keeps every
/// other, including any a later build wrote. A missing file holds none.
///
/// # Errors
///
/// Why a file that is there could not be read, or is not a JSON object.
fn read_preferences(path: &Path) -> Result<Map<String, Value>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(e.to_string()),
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(fields)) => Ok(fields),
        Ok(_) => Err("it does not hold a set of named settings".to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// The Protocols folder Preferences holds, `Ok(None)` when none has been chosen, or why the file
/// that should hold it could not be read.
fn read_setting() -> Option<Result<Option<PathBuf>, String>> {
    let path = preferences_path()?;
    let now = stamp(&path);
    let mut held = SETTING.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((from, when, setting)) = held.as_ref() {
        if *from == path && *when == now {
            return Some(setting.clone());
        }
    }
    let setting = read_preferences(&path).map(|fields| {
        fields
            .get(PROTOCOLS_DIR_FIELD)
            .and_then(Value::as_str)
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
    });
    *held = Some((path, now, setting.clone()));
    Some(setting)
}

/// The Protocols folder Preferences holds, when one has been chosen and its file reads.
fn setting() -> Option<PathBuf> {
    read_setting().and_then(Result::ok).flatten()
}

/// What to say when the Preferences file is there but could not be read: new Protocols then go to
/// the default folder for now, which is not what the analyst chose. `None` when it reads, when
/// the file is missing, and while `BRIGHTFIELD_PROTOCOLS_DIR` decides the folder anyway.
#[must_use]
pub fn setting_problem() -> Option<String> {
    if std::env::var_os(PROTOCOLS_DIR_VAR).is_some_and(|v| !v.is_empty()) {
        return None;
    }
    let why = read_setting()?.err()?;
    let path = preferences_path()?;
    Some(format!(
        "The Protocols folder setting in {} could not be read ({why}), so new Protocols go to {} \
         for now. Choose the folder again in Preferences to set it; the file is then kept beside \
         it as {}.{UNREADABLE_SUFFIX}.",
        path.display(),
        root().display(),
        PREFERENCES_FILE
    ))
}

/// Where new Protocols are made: `$BRIGHTFIELD_PROTOCOLS_DIR`, else the folder chosen in
/// Preferences, else `~/Documents/Protocols`.
#[must_use]
pub fn root() -> PathBuf {
    if let Some(dir) = std::env::var_os(PROTOCOLS_DIR_VAR).filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = setting() {
        return dir;
    }
    // A launch from the Mac's desktop always has a home directory.
    std::env::home_dir()
        .unwrap_or_default()
        .join("Documents")
        .join("Protocols")
}

/// Make `dir` the Protocols folder in Preferences, after checking Brightfield may write in it:
/// the folder is made if it is missing, and a folder is made and removed in it. A folder inside
/// the Protocols folder now, or one holding it, is refused, since moving the Protocols across
/// would move them into or out of themselves. Choosing the folder it is already is accepted and
/// changes nothing.
///
/// `BRIGHTFIELD_PROTOCOLS_DIR` still wins over the setting while it is set.
///
/// # Errors
///
/// A sentence saying why, and the setting is left as it was: a refused folder names System
/// Settings → Privacy & Security → Files and Folders, as making a Protocol does.
pub fn set_root(dir: &Path) -> Result<(), String> {
    set_root_saying(dir).map(|_| ())
}

/// [`set_root`], answering what else it did that the analyst should be told: a Preferences file
/// that could not be read is not written over, but set aside beside it as
/// `preferences.json.unreadable` before the new one is written, and the sentence says so.
///
/// # Errors
///
/// As [`set_root`], and a file that could not be read and could not be set aside either, which is
/// then left as it is.
pub fn set_root_saying(dir: &Path) -> Result<Option<String>, String> {
    let dir = std::path::absolute(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let old = root();
    let refused = || {
        format!(
            "Brightfield cannot use {} as the Protocols folder. Allow it under System Settings → \
             Privacy & Security → Files and Folders, then choose it again.",
            dir.display()
        )
    };
    let check = |e: &std::io::Error| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            refused()
        } else {
            format!("{}: {e}", dir.display())
        }
    };
    std::fs::create_dir_all(&dir).map_err(|e| check(&e))?;
    let canonical = dir.canonicalize().map_err(|e| check(&e))?;
    if let Ok(was) = old.canonicalize() {
        if was != canonical {
            if canonical.starts_with(&was) {
                return Err(format!(
                    "{} is inside the Protocols folder now, {}, so its Protocols cannot move \
                     there. Choose a folder outside it.",
                    dir.display(),
                    old.display()
                ));
            }
            if was.starts_with(&canonical) {
                return Err(format!(
                    "The Protocols folder now, {}, is inside {}, so its Protocols cannot move \
                     there. Choose a folder outside it.",
                    old.display(),
                    dir.display()
                ));
            }
        }
    }
    // A folder the system lets Brightfield list may still refuse a write, so one is tried.
    let probe = dir.join(format!(".brightfield-check-{}", std::process::id()));
    std::fs::create_dir(&probe).map_err(|e| check(&e))?;
    let _ = std::fs::remove_dir(&probe);

    let path = preferences_path()
        .ok_or_else(|| "This Mac has no folder to keep Preferences in.".to_string())?;
    let mut said = None;
    let mut preferences = match read_preferences(&path) {
        Ok(fields) => fields,
        Err(why) => {
            let aside = set_aside(&path).map_err(|e| {
                format!(
                    "{} could not be read ({why}) and could not be set aside ({e}), so it was \
                     left as it is and the Protocols folder was not changed.",
                    path.display()
                )
            })?;
            said = Some(format!(
                "The Preferences file that could not be read was kept as {}.",
                aside.display()
            ));
            Map::new()
        }
    };
    preferences.insert(
        PROTOCOLS_DIR_FIELD.to_string(),
        Value::String(dir.to_string_lossy().into_owned()),
    );
    let text =
        serde_json::to_string_pretty(&Value::Object(preferences)).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    // Written beside and renamed over, so a failed write leaves the old setting whole.
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, text).map_err(|e| format!("{}: {e}", partial.display()))?;
    std::fs::rename(&partial, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    let now = stamp(&path);
    *SETTING.lock().unwrap_or_else(PoisonError::into_inner) = Some((path, now, Ok(Some(dir))));
    Ok(said)
}

/// Why a Protocol was left in the old Protocols folder rather than moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Left {
    /// It is open with unsaved edits, and the analyst chose to keep editing.
    Draft,
    /// A run is writing into its folder.
    Running,
}

/// Move the unreadable Preferences file at `path` aside, to `preferences.json.unreadable`, or
/// `-2`, `-3` and so on after it when that name is taken, never replacing a file kept aside
/// before. Answers the name used.
///
/// The claim is a hard link, which fails when the name is taken, so two set-asides cannot take one
/// name; the original is removed once linked. A file system without hard links is checked for the
/// name and then renamed.
fn set_aside(path: &Path) -> std::io::Result<PathBuf> {
    for n in 1u32.. {
        let suffix = if n == 1 {
            UNREADABLE_SUFFIX.to_string()
        } else {
            format!("{UNREADABLE_SUFFIX}-{n}")
        };
        let aside = path.with_extension(format!("json.{suffix}"));
        match std::fs::hard_link(path, &aside) {
            Ok(()) => {
                std::fs::remove_file(path)?;
                return Ok(aside);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) if !aside.exists() => {
                std::fs::rename(path, &aside)?;
                return Ok(aside);
            }
            Err(_) => {}
        }
    }
    unreachable!("an unbounded range")
}

/// Whether `a` and `b` name one folder, as written or once both are resolved.
fn same_folder(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// The Protocols a move into a new Protocols folder leaves where they are: each folder in
/// `running`, which a run is writing into, and `drafted`, the open Protocol whose analyst kept
/// editing, unless it is one of those already. Each folder is listed once however its path is
/// written (`a_folder_running_and_drafted_is_left_once`).
#[must_use]
pub fn left_behind(running: &[PathBuf], drafted: Option<&Path>) -> Vec<(PathBuf, Left)> {
    let mut leave: Vec<(PathBuf, Left)> = Vec::new();
    for dir in running {
        if !leave.iter().any(|(kept, _)| same_folder(kept, dir)) {
            leave.push((dir.clone(), Left::Running));
        }
    }
    if let Some(open) = drafted {
        if !leave.iter().any(|(kept, _)| same_folder(kept, open)) {
            leave.push((open.to_path_buf(), Left::Draft));
        }
    }
    leave
}

/// What moving the old Protocols folder's Protocols into a new one did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoveReport {
    /// Each Protocol moved, as the folder it was in and the folder it is in now, in the order
    /// they moved.
    pub moved: Vec<(PathBuf, PathBuf)>,
    /// The Protocol whose move failed, which stopped the rest, and why.
    pub failed: Option<(PathBuf, String)>,
    /// The Protocols left where they are, each with why: the open one with a draft whose analyst
    /// chose to keep editing, and any a run is writing into.
    pub left: Vec<(PathBuf, Left)>,
    /// What did not follow a Protocol that moved, each in a sentence, as [`crate::rename::Moved`]
    /// words it. The moves are done all the same.
    pub warnings: Vec<String>,
}

impl MoveReport {
    /// What the move did, in sentences for a banner: how many moved and which, which failed and
    /// why, and which was left open.
    #[must_use]
    pub fn said(&self) -> String {
        let name = |dir: &Path| {
            dir.file_name()
                .map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into())
        };
        let names = |dirs: &[(PathBuf, PathBuf)]| {
            dirs.iter()
                .map(|(from, _)| name(from))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut said = Vec::new();
        match &self.failed {
            Some((dir, why)) => {
                let tried = self.moved.len() + 1;
                if self.moved.is_empty() {
                    said.push(format!(
                        "No Protocol moved: {} could not move. {why}",
                        name(dir)
                    ));
                } else {
                    said.push(format!(
                        "{} of {tried} tried moved ({}), then {} could not move, and the rest \
                         stayed. {why}",
                        self.moved.len(),
                        names(&self.moved),
                        name(dir)
                    ));
                }
            }
            None if self.moved.is_empty() => said.push("No Protocol moved.".to_string()),
            None => said.push(format!(
                "Moved {} Protocol{}: {}.",
                self.moved.len(),
                if self.moved.len() == 1 { "" } else { "s" },
                names(&self.moved)
            )),
        }
        for (dir, why) in &self.left {
            let place = dir
                .parent()
                .map_or_else(String::new, |p| p.display().to_string());
            said.push(match why {
                Left::Draft => format!(
                    "{} is open with unsaved edits, so it stayed in {place}.",
                    name(dir)
                ),
                Left::Running => format!(
                    "{} stayed in {place}: a run is writing into its folder. Move it with \
                     Location once the run finishes.",
                    name(dir)
                ),
            });
        }
        said.extend(self.warnings.iter().cloned());
        said.join(" ")
    }
}

/// Move each Protocol in the Protocols folder `from` into `to`, keeping the groups it is in, one
/// at a time and stopping at the first that fails. See [`move_all_with`].
#[must_use]
pub fn move_all(from: &Path, to: &Path) -> MoveReport {
    move_all_with(from, to, &[], &FolderOps::system())
}

/// [`move_all`], leaving each Protocol in `leave` where it is and naming it with its reason in
/// [`MoveReport::left`], with the folder operations `ops` so a test can make a move fail.
///
/// The Protocols are the ones the door's tree reads in `from` ([`crate::door_tree::read`]), in its
/// order. Each moves as the Location field moves one ([`crate::rename::relocate_with`]), into the
/// same group path under `to`, made if missing: it takes `-2` on a clash, its working database
/// and source paths follow, and its Recents row is the caller's to repoint from
/// [`MoveReport::moved`]. A group left empty in `from` stays, as the Finder's to tidy.
#[must_use]
pub fn move_all_with(
    from: &Path,
    to: &Path,
    leave: &[(PathBuf, Left)],
    ops: &FolderOps,
) -> MoveReport {
    let mut report = MoveReport::default();
    let mut protocols = Vec::new();
    protocols_in(&crate::door_tree::read(from), from, &mut protocols);
    let same = same_folder;
    for dir in protocols {
        if let Some((_, why)) = leave.iter().find(|(kept, _)| same(kept, &dir)) {
            report.left.push((dir, *why));
            continue;
        }
        let group = dir
            .parent()
            .and_then(|p| p.strip_prefix(from).ok())
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let parent = to.join(group);
        let moved = std::fs::create_dir_all(&parent)
            .map_err(|e| classify(&e, &parent).to_string())
            .and_then(|()| crate::rename::relocate_with(&dir, &parent, ops));
        match moved {
            Ok(moved) => {
                report.warnings.extend(
                    moved
                        .warnings
                        .iter()
                        .map(|w| format!("{}: {w}.", moved.to.display())),
                );
                report.warnings.extend(moved.left_behind);
                report.moved.push((dir, moved.to));
            }
            Err(why) => {
                report.failed = Some((dir, why));
                break;
            }
        }
    }
    report
}

/// The Protocols' folders in `nodes`, read under `at`, groups first as the door draws them.
fn protocols_in(nodes: &[crate::door_tree::Node], at: &Path, into: &mut Vec<PathBuf>) {
    for node in nodes {
        match node {
            crate::door_tree::Node::Group { name, children } => {
                protocols_in(children, &at.join(name), into);
            }
            crate::door_tree::Node::Protocol { dir, .. } => into.push(dir.clone()),
        }
    }
}

/// Make `parent` if it is missing, then claim a new folder in it named `stem`, or `stem-2`,
/// `stem-3` and so on when that is taken. The first Protocol takes the bare name.
///
/// `create_dir` is the claim: it fails if the folder exists, so two claims at once cannot both
/// take a name. Returns the folder and its name.
///
/// # Errors
///
/// [`ClaimError::Refused`] when the system denies the write in `parent`; [`ClaimError::Io`] for
/// any other failure.
pub fn claim(parent: &Path, stem: &str) -> Result<(PathBuf, String), ClaimError> {
    std::fs::create_dir_all(parent).map_err(|e| classify(&e, parent))?;
    (1..)
        .find_map(|n| {
            let name = if n == 1 {
                stem.to_string()
            } else {
                format!("{stem}-{n}")
            };
            let dir = parent.join(&name);
            match std::fs::create_dir(&dir) {
                Ok(()) => Some(Ok((dir, name))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(classify(&e, parent))),
            }
        })
        .expect("an unbounded range")
}

fn classify(error: &std::io::Error, parent: &Path) -> ClaimError {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        ClaimError::Refused(parent.to_path_buf())
    } else {
        ClaimError::Io(format!("{}: {error}", parent.display()))
    }
}
