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

/// The setting as last read or written, with the file it came from, so the door, which asks for
/// the folder on each frame it draws, does not read the file each time.
static SETTING: Mutex<Option<(PathBuf, Option<PathBuf>)>> = Mutex::new(None);

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

/// The Preferences file's fields, read as an object so that writing one field back keeps every
/// other, including any a later build wrote. A file missing or unreadable reads as none.
fn read_preferences(path: &Path) -> Map<String, Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| match value {
            Value::Object(fields) => Some(fields),
            _ => None,
        })
        .unwrap_or_default()
}

/// The Protocols folder Preferences holds, when one has been chosen.
fn setting() -> Option<PathBuf> {
    let path = preferences_path()?;
    let mut held = SETTING.lock().unwrap_or_else(PoisonError::into_inner);
    match held.as_ref() {
        Some((from, dir)) if *from == path => dir.clone(),
        _ => {
            let dir = read_preferences(&path)
                .get(PROTOCOLS_DIR_FIELD)
                .and_then(Value::as_str)
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from);
            *held = Some((path, dir.clone()));
            dir
        }
    }
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
    let mut preferences = read_preferences(&path);
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
    *SETTING.lock().unwrap_or_else(PoisonError::into_inner) = Some((path, Some(dir)));
    Ok(())
}

/// What moving the old Protocols folder's Protocols into a new one did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoveReport {
    /// Each Protocol moved, as the folder it was in and the folder it is in now, in the order
    /// they moved.
    pub moved: Vec<(PathBuf, PathBuf)>,
    /// The Protocol whose move failed, which stopped the rest, and why.
    pub failed: Option<(PathBuf, String)>,
    /// The Protocol open with a draft whose analyst chose to keep editing, left where it is.
    pub left_open: Option<PathBuf>,
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
        if let Some(dir) = &self.left_open {
            said.push(format!(
                "{} is open with unsaved edits, so it stayed in {}.",
                name(dir),
                dir.parent()
                    .map_or_else(String::new, |p| p.display().to_string())
            ));
        }
        said.extend(self.warnings.iter().cloned());
        said.join(" ")
    }
}

/// Move each Protocol in the Protocols folder `from` into `to`, keeping the groups it is in, one
/// at a time and stopping at the first that fails. See [`move_all_with`].
#[must_use]
pub fn move_all(from: &Path, to: &Path) -> MoveReport {
    move_all_with(from, to, None, &FolderOps::system())
}

/// [`move_all`], leaving the Protocol in `leave` where it is and naming it in
/// [`MoveReport::left_open`], with the folder operations `ops` so a test can make a move fail.
///
/// The Protocols are the ones the door's tree reads in `from` ([`crate::door_tree::read`]), in its
/// order. Each moves as the Location field moves one ([`crate::rename::relocate_with`]), into the
/// same group path under `to`, made if missing: it takes `-2` on a clash, its working database
/// and source paths follow, and its Recents row is the caller's to repoint from
/// [`MoveReport::moved`]. A group left empty in `from` stays, as the Finder's to tidy.
#[must_use]
pub fn move_all_with(from: &Path, to: &Path, leave: Option<&Path>, ops: &FolderOps) -> MoveReport {
    let mut report = MoveReport::default();
    let mut protocols = Vec::new();
    protocols_in(&crate::door_tree::read(from), from, &mut protocols);
    let same = |a: &Path, b: &Path| {
        a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
    };
    for dir in protocols {
        if leave.is_some_and(|open| same(open, &dir)) {
            report.left_open = Some(dir);
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
