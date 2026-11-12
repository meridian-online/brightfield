//! Renaming a Protocol, moving it to another folder, and finding its sources again after it was
//! moved in the Finder.
//!
//! A Protocol is a folder, so its name is its folder's name, and the name in its `arcform.yaml`
//! is set beside it. Three things depend on where the folder is, and each move takes them along:
//!
//! - **Source paths written relative to the folder that point outside it.** A data package names
//!   its file from its own folder (`../../../data/sales.csv`), so a folder that moves to another
//!   parent no longer finds it. [`repair`] writes each such path again from the new place. A
//!   relative path inside the folder moves with it and is left alone, and so is an absolute path
//!   outside it.
//! - **Absolute paths inside the folder**: a source file kept in the Protocol's own folder, which
//!   its load step and `.sources.json` name by its absolute path. [`repair`] points each at the
//!   same place in the new folder.
//! - **arc's working database.** arc keeps it outside the folder, in its data folder, under a key
//!   made from the folder's path, so a moved folder would find none and run every step again. The
//!   database is moved to the new path's key; when it cannot be moved it is deleted, and the next
//!   run builds it again.
//! - **The grid copy a saved chart reads**, which Save names by its absolute path inside the
//!   folder, and which [`repair`] points inside the new one.
//! - **The path the folder was last seen at**, recorded as `dir` in `.sources.json`, which is how
//!   an open notices a Protocol moved in the Finder ([`moved_since`]) and repairs it first
//!   ([`follow_a_move`]).
//!
//! The chart versions follow on their own: arc finds a file's snapshots again through the
//! version log inside the folder, which travels with it.

use std::io;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::discovery::MODEL_FILE;
use crate::own_files::SOURCES_FILE;

/// The file beside a database in arc's data folder that names the Protocol folder it is keyed to.
const PROTOCOL_PATH_FILE: &str = "protocol-path";

/// The manifest every Protocol folder holds.
const MANIFEST: &str = "arcform.yaml";

/// What a move did: where the Protocol is now, and, after a copy to another volume whose original
/// could not all be removed, the sentence naming what was left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    /// The Protocol's folder now.
    pub to: PathBuf,
    /// The original folder, or what is left of it, when a copy to another volume could not
    /// remove it all. The move is done all the same: the copy was checked first.
    pub left_behind: Option<String>,
    /// What did not follow the folder, each in a sentence: sources that could not all be pointed
    /// at from the new place, a place not recorded, a working database that will be built again.
    /// The move is done all the same.
    pub warnings: Vec<String>,
}

/// What a source repair that failed after a move says.
pub const SOURCES_NOT_FOLLOWED: &str = "Its sources could not all be found from the new place";

/// What a place not recorded after a move says.
pub const PLACE_NOT_RECORDED: &str =
    "Its new place was not recorded, so the next open repairs its sources again";

/// What a working database that did not move says.
pub const DATABASE_NOT_FOLLOWED: &str = "Its working data will be rebuilt on the next run";

/// What a trust record that did not follow the move says.
pub const TRUST_NOT_FOLLOWED: &str =
    "Its trust was not recorded at the new place, so it opens restricted until trusted again";

/// The two folder operations a move makes, so a test can make either fail: renaming the folder,
/// and removing the original once a copy to another volume has been checked.
pub struct FolderOps<'a> {
    /// Rename a folder, as [`std::fs::rename`] does.
    pub rename: &'a dyn Fn(&Path, &Path) -> io::Result<()>,
    /// Remove a folder with what it holds, as [`std::fs::remove_dir_all`] does.
    pub remove: &'a dyn Fn(&Path) -> io::Result<()>,
}

fn system_rename(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::rename(from, to)
}

fn system_remove(dir: &Path) -> io::Result<()> {
    std::fs::remove_dir_all(dir)
}

impl FolderOps<'static> {
    /// The file system's own operations.
    #[must_use]
    pub fn system() -> Self {
        Self {
            rename: &system_rename,
            remove: &system_remove,
        }
    }
}

/// Where a Location typed as `typed` names, read from the folder the Protocol is in now,
/// `parent`: a leading `~` is the home folder, and a relative path is read from `parent`.
#[must_use]
pub fn resolve_location(typed: &str, parent: &Path) -> PathBuf {
    let typed = typed.trim();
    let home = || std::env::home_dir().unwrap_or_default();
    let named = if typed == "~" {
        home()
    } else if let Some(rest) = typed.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(typed)
    };
    if named.is_absolute() {
        folded(&named)
    } else {
        folded(&parent.join(named))
    }
}

/// Rename the Protocol in `dir` to `new_name`: its folder within its parent and the `name:` in its
/// `arcform.yaml` together, then repair its sources ([`repair`]) and move arc's database to the
/// new path. Answers the new folder.
///
/// A name another folder in the parent already has is refused, and the folder is left as it was:
/// a rename names one Protocol, so it does not take `-2` as a new Protocol does.
///
/// # Errors
///
/// A name that is empty, holds a `/`, starts with a `.` or is taken; a folder with no parent; and
/// a folder or manifest that cannot be read or moved.
pub fn rename(dir: &Path, new_name: &str) -> Result<PathBuf, String> {
    rename_with(dir, new_name, &FolderOps::system()).map(|m| m.to)
}

/// [`rename`], with the folder operations `ops`, answering what the move did.
///
/// # Errors
///
/// As [`rename`].
pub fn rename_with(dir: &Path, new_name: &str, ops: &FolderOps) -> Result<Moved, String> {
    let moved = rename_unmarked(dir, new_name, ops)?;
    // The analyst has named the Protocol, so its first source no longer names it, whatever name
    // was chosen.
    let _ = std::fs::remove_file(moved.to.join(crate::own_files::UNTITLED_FILE));
    Ok(moved)
}

/// Rename the Protocol in `dir` after `stem`, within the folder it is in, taking `stem-2`,
/// `stem-3` and so on when that name is taken, as a new Protocol is named
/// ([`crate::protocols_dir::claim`]): what the first source added to an Untitled Protocol does.
/// Otherwise as [`rename`]: the `name:` changes with the folder, its sources are repaired and
/// arc's database moved.
///
/// # Errors
///
/// A `stem` that cannot name a folder, a folder with no parent, and a folder or manifest that
/// cannot be read, claimed or moved.
pub fn rename_claiming(dir: &Path, stem: &str) -> Result<Moved, String> {
    let stem = checked_name(stem)?;
    let dir = std::path::absolute(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let parent = dir
        .parent()
        .ok_or_else(|| format!("{}: has no folder above it", dir.display()))?;
    let (to, name) = crate::protocols_dir::claim(parent, stem).map_err(|e| e.to_string())?;
    let (left_behind, warnings) = move_to(&dir, &to, Some(&name), true, &FolderOps::system())?;
    Ok(Moved {
        to,
        left_behind,
        warnings,
    })
}

/// [`rename_with`], leaving the record of the name New Protocol gave the folder as it is.
fn rename_unmarked(dir: &Path, new_name: &str, ops: &FolderOps) -> Result<Moved, String> {
    let name = checked_name(new_name)?;
    let dir = std::path::absolute(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let parent = dir
        .parent()
        .ok_or_else(|| format!("{}: has no folder above it", dir.display()))?;
    let to = parent.join(name);
    if dir.file_name().is_some_and(|n| n == name) {
        // The folder has the name already; the manifest may not, and arc names the database
        // after it.
        let database = working_database(&dir);
        set_manifest_name(&dir, name)?;
        let canonical = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        let warnings = move_database(database, &canonical)
            .err()
            .map(|e| format!("{DATABASE_NOT_FOLLOWED}: {e}"))
            .into_iter()
            .collect();
        return Ok(Moved {
            to: dir,
            left_behind: None,
            warnings,
        });
    }
    // A change of case alone, on a file system that ignores case, names the folder itself.
    let itself = to.exists() && same_folder(&to, &dir);
    if !itself {
        match std::fs::create_dir(&to) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(format!(
                    "{} already holds something named {name}, so the Protocol keeps its name.",
                    parent.display()
                ))
            }
            Err(e) => return Err(format!("{}: {e}", to.display())),
        }
    }
    let (left_behind, warnings) = move_to(&dir, &to, Some(name), !itself, ops)?;
    Ok(Moved {
        to,
        left_behind,
        warnings,
    })
}

/// Move the Protocol in `dir` into `new_parent`, keeping its name, or taking `-2`, `-3` and so on
/// when `new_parent` holds that name already, then repair its sources and move arc's database as
/// [`rename`] does. Answers the new folder. A move into the folder it is in already does nothing.
///
/// # Errors
///
/// A `new_parent` that is not a folder or is inside the Protocol, and a folder or manifest that
/// cannot be read, claimed or moved.
pub fn relocate(dir: &Path, new_parent: &Path) -> Result<PathBuf, String> {
    relocate_with(dir, new_parent, &FolderOps::system()).map(|m| m.to)
}

/// [`relocate`], with the folder operations `ops`, so a test can make the rename fail as a move to
/// another volume does, or the removal after it. When the rename fails with
/// [`io::ErrorKind::CrossesDevices`], the folder is copied, the copy checked against it by the
/// path and size of each file in it (not by content), and then the original removed
/// (`relocate_across_volumes_copies_then_removes`). Once the copy is checked the move is done: an
/// original that cannot all be removed is named in [`Moved::left_behind`]
/// (`a_checked_copy_whose_original_cannot_all_be_removed_is_still_a_move`).
///
/// # Errors
///
/// As [`relocate`], and a copy that cannot be made or does not match the original, which is then
/// taken away again with the original left as it was.
pub fn relocate_with(dir: &Path, new_parent: &Path, ops: &FolderOps) -> Result<Moved, String> {
    let dir = std::path::absolute(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let was = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let new_parent =
        std::path::absolute(new_parent).map_err(|e| format!("{}: {e}", new_parent.display()))?;
    if !new_parent.is_dir() {
        return Err(format!("There is no folder at {}.", new_parent.display()));
    }
    let parent = new_parent
        .canonicalize()
        .map_err(|e| format!("{}: {e}", new_parent.display()))?;
    if parent.starts_with(&was) {
        return Err(format!(
            "{} is inside the Protocol, which cannot be moved into itself.",
            new_parent.display()
        ));
    }
    if was.parent() == Some(parent.as_path()) {
        return Ok(Moved {
            to: dir,
            left_behind: None,
            warnings: Vec::new(),
        });
    }
    let stem = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| format!("{}: names no folder", dir.display()))?;
    let (to, name) = crate::protocols_dir::claim(&new_parent, &stem).map_err(|e| e.to_string())?;
    let renamed = (name != stem).then_some(name.as_str());
    let (left_behind, warnings) = move_to(&dir, &to, renamed, true, ops)?;
    // A move is not a naming: an Untitled Protocol that took `-2` in its new folder is still
    // waiting for its first source to name it.
    let marker = to.join(crate::own_files::UNTITLED_FILE);
    if marker.is_file() {
        let _ = std::fs::write(&marker, format!("{name}\n"));
    }
    Ok(Moved {
        to,
        left_behind,
        warnings,
    })
}

/// The folder the Protocol in `dir` was last recorded at, when that is not where it is now: it was
/// moved or copied in the Finder since. `None` for a folder that records no place, as one made
/// before the place was recorded does, and for one that has not moved.
#[must_use]
pub fn moved_since(dir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(dir.join(SOURCES_FILE)).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let recorded = PathBuf::from(value.get("dir")?.as_str()?);
    let now = dir.canonicalize().ok()?;
    (recorded != now).then_some(recorded)
}

/// Before the Protocol in `dir` is opened: when it was moved since it was last recorded
/// ([`moved_since`]), repair its sources from where it was and move arc's database to its new
/// path, then record where it is now. A folder copied rather than moved, whose original is still
/// there, keeps the original's database where it is, so the copy builds its own. A folder that
/// records no place has its place recorded.
///
/// Answers what did not follow the move, in the words an in-app move uses
/// ([`SOURCES_NOT_FOLLOWED`], [`PLACE_NOT_RECORDED`], [`DATABASE_NOT_FOLLOWED`]), so the window
/// can say so (`a_finder_move_whose_repair_fails_says_so_on_open`). Sources that could not be
/// repaired leave the old place recorded, so the next open tries again. A folder that had not
/// moved answers no warning: a place it could not record is said on stderr.
#[must_use]
pub fn follow_a_move(dir: &Path) -> Vec<String> {
    let Some(was) = moved_since(dir) else {
        // Nothing moved, so a place not recorded is no move left unfollowed: it is said on
        // stderr, as before, and not under the banner for a move.
        if dir.join(SOURCES_FILE).is_file() {
            if let Err(e) = record_dir(dir) {
                eprintln!("warning: {}: {PLACE_NOT_RECORDED}: {e}", dir.display());
            }
        }
        return Vec::new();
    };
    let repaired = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))
        .and_then(|now| repair(&now, &was).map(|_| now));
    let now = match repaired {
        Ok(now) => now,
        Err(e) => return vec![format!("{SOURCES_NOT_FOLLOWED}: {e}")],
    };
    let mut warnings = Vec::new();
    if !was.join(MANIFEST).is_file() {
        if let Err(e) = move_database(database_keyed_to(&was, &now), &now) {
            warnings.push(format!("{DATABASE_NOT_FOLLOWED}: {e}"));
        }
    }
    if let Err(e) = record_dir(&now) {
        warnings.push(format!("{PLACE_NOT_RECORDED}: {e}"));
    }
    warnings
}

/// Rewrite each source path in the Protocol in `dir` that is written relative to its folder and
/// points outside it, so it points from `dir` at the file it pointed at from `was`: the paths in
/// `packages/*/datapackage.json`, in `.sources.json`, and in the model. The data each saved chart
/// under `panels/` reads, an absolute path inside the folder, is pointed inside `dir`, and so is a
/// source file inside the folder that a load step's SQL or `.sources.json` names by its absolute
/// path (`a_source_inside_the_protocol_follows_a_rename`). Answers how many were rewritten. A
/// file that is not there is passed over.
///
/// `was` and `dir` are both read as written, so give them in one spelling (both canonical).
///
/// # Errors
///
/// A file that is there and cannot be read, parsed or written.
pub fn repair(dir: &Path, was: &Path) -> Result<usize, String> {
    let mut count = 0;
    count += repair_packages(dir, was)?;
    count += repair_sources(dir, was)?;
    count += repair_model(dir, was)?;
    count += repair_panels(&dir.join("panels"), dir, was)?;
    count += repair_step_sql(&dir.join("models"), dir, was)?;
    Ok(count)
}

/// The string literals in each step's SQL under `models/` that name a file inside `was` by its
/// absolute path, as a load step names a source kept in the Protocol's folder: each is written
/// again inside `dir`.
fn repair_step_sql(models: &Path, dir: &Path, was: &Path) -> Result<usize, String> {
    let Ok(entries) = std::fs::read_dir(models) else {
        return Ok(0);
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "sql") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (fixed, changed) = moved_sql_literals(&text, dir, was);
        if changed > 0 {
            std::fs::write(&path, fixed).map_err(|e| format!("{}: {e}", path.display()))?;
            count += changed;
        }
    }
    Ok(count)
}

/// `sql` with each single-quoted literal naming a path inside `was` written again inside `dir`,
/// and how many were. A doubled quote inside a literal is a quote, as SQL writes it.
fn moved_sql_literals(sql: &str, dir: &Path, was: &Path) -> (String, usize) {
    let mut out = String::with_capacity(sql.len());
    let mut changed = 0;
    let mut rest = sql;
    while let Some(open) = rest.find('\'') {
        out.push_str(&rest[..open]);
        let body = &rest[open + 1..];
        // The closing quote: the first not doubled.
        let mut end = None;
        let mut i = 0;
        let bytes = body.as_bytes();
        while i < bytes.len() {
            if bytes[i] == b'\'' {
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                end = Some(i);
                break;
            }
            i += 1;
        }
        let Some(end) = end else {
            out.push_str(&rest[open..]);
            return (out, changed);
        };
        let written = &body[..end];
        let value = written.replace("''", "'");
        match inside_of(Path::new(&value), was) {
            Some(inside) => {
                let fixed = dir.join(inside).to_string_lossy().replace('\'', "''");
                if fixed != written {
                    changed += 1;
                }
                out.push('\'');
                out.push_str(&fixed);
                out.push('\'');
            }
            None => {
                out.push('\'');
                out.push_str(written);
                out.push('\'');
            }
        }
        rest = &body[end + 1..];
    }
    out.push_str(rest);
    (out, changed)
}

/// The data a saved chart reads, which Save writes as an absolute path to the grid copy inside the
/// Protocol (`.grid/<table>.parquet`): each `file:` naming a place inside `was` is written again
/// inside `dir`. Every chart file under `panels/` is read, the folders of table and join grids'
/// charts included; each other line is kept as it was.
fn repair_panels(panels: &Path, dir: &Path, was: &Path) -> Result<usize, String> {
    let Ok(entries) = std::fs::read_dir(panels) else {
        return Ok(0);
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            count += repair_panels(&path, dir, was)?;
            continue;
        }
        if path.extension().is_none_or(|e| e != "yaml") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut changed = 0;
        let lines: Vec<String> = text
            .split('\n')
            .map(|line| match moved_file_line(line, dir, was) {
                Some(fixed) => {
                    changed += 1;
                    fixed
                }
                None => line.to_string(),
            })
            .collect();
        if changed > 0 {
            std::fs::write(&path, lines.join("\n"))
                .map_err(|e| format!("{}: {e}", path.display()))?;
            count += changed;
        }
    }
    Ok(count)
}

/// `line` with the absolute path its `file:` key names written again inside `dir`, when that path
/// is inside `was` and reads differently from inside `dir`. `None` for a line needing no change.
///
/// The key is `file:` where a YAML key starts (at the line's start after its indent or a `- `, or
/// after a flow mapping's `{` or `,`), followed by a space or the line's end, so neither
/// `profile:` nor a `file:///` address inside a value is taken for it. The value is read as YAML
/// reads a scalar, a single-quoted one's `''` and a double-quoted one's escapes included, and
/// written back in the same style, escaped as that style needs.
fn moved_file_line(line: &str, dir: &Path, was: &Path) -> Option<String> {
    let mut from = 0;
    while let Some(found) = line[from..].find("file:") {
        let key = from + found;
        from = key + "file:".len();
        if let Some(fixed) = file_value_at(line, key, dir, was) {
            return Some(fixed);
        }
    }
    None
}

/// [`moved_file_line`] for the `file:` at byte `key` of `line`.
fn file_value_at(line: &str, key: usize, dir: &Path, was: &Path) -> Option<String> {
    let before = line[..key].trim_end_matches(' ');
    let starts_a_key = before.is_empty()
        || before.ends_with('{')
        || before.ends_with(',')
        || before.trim_start() == "-"
        || (before.ends_with('-') && before[..before.len() - 1].trim().is_empty());
    let after = key + "file:".len();
    if !starts_a_key || !line[after..].is_empty() && !line[after..].starts_with(' ') {
        return None;
    }
    let value_at = after + (line[after..].len() - line[after..].trim_start().len());
    let in_flow = before.ends_with('{') || before.ends_with(',');
    let (named, end, style) = read_scalar(&line[value_at..], in_flow)?;
    let inside = inside_of(Path::new(&named), was)?;
    let fixed = dir.join(inside).to_string_lossy().into_owned();
    if fixed == named {
        return None;
    }
    Some(format!(
        "{}{}{}",
        &line[..value_at],
        write_scalar(&fixed, style),
        &line[value_at + end..]
    ))
}

/// How a scalar was written.
#[derive(Clone, Copy)]
enum Style {
    Plain,
    Single,
    Double,
}

/// The scalar at the start of `text`, unescaped, with how many bytes of `text` it took and its
/// style, read inside a flow mapping when `in_flow`. `None` for a quoted scalar left open.
fn read_scalar(text: &str, in_flow: bool) -> Option<(String, usize, Style)> {
    let mut chars = text.char_indices().peekable();
    match chars.peek()?.1 {
        '\'' => {
            chars.next();
            let mut out = String::new();
            while let Some((at, c)) = chars.next() {
                if c == '\'' {
                    if chars.peek().is_some_and(|(_, n)| *n == '\'') {
                        chars.next();
                        out.push('\'');
                    } else {
                        return Some((out, at + 1, Style::Single));
                    }
                } else {
                    out.push(c);
                }
            }
            None
        }
        '"' => {
            chars.next();
            let mut out = String::new();
            while let Some((at, c)) = chars.next() {
                match c {
                    '"' => return Some((out, at + 1, Style::Double)),
                    '\\' => match chars.next()?.1 {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        other => out.push(other),
                    },
                    other => out.push(other),
                }
            }
            None
        }
        _ => {
            // A `,` or `}` ends a plain scalar only inside a flow mapping; in a block it is part
            // of the value.
            let flow_end = if in_flow { text.find([',', '}']) } else { None };
            let end = flow_end.or_else(|| text.find(" #")).unwrap_or(text.len());
            let mut value = text[..end].trim_end();
            // A flow mapping opened on a line above leaves its `,` or `}` after the value: they
            // are the mapping's, not the path's.
            if !in_flow {
                while let Some(shorter) =
                    value.strip_suffix(',').or_else(|| value.strip_suffix('}'))
                {
                    value = shorter.trim_end();
                }
            }
            Some((value.to_string(), value.len(), Style::Plain))
        }
    }
}

/// `value` written as a scalar in `style`, escaped as it needs; a plain scalar that would not read
/// back as itself is single-quoted.
fn write_scalar(value: &str, style: Style) -> String {
    let plain_safe = !value.is_empty()
        && !value.contains(['\'', '"', '#', ',', '{', '}', '[', ']'])
        && !value.contains(": ")
        && !value.ends_with(':')
        && value.trim() == value;
    match style {
        Style::Plain if plain_safe => value.to_string(),
        Style::Plain | Style::Single => format!("'{}'", value.replace('\'', "''")),
        Style::Double => format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")),
    }
}

/// The part of the absolute `path` inside the folder `was`, which is canonical. On the Mac
/// `/var` and `/tmp` are links into `/private`, and a path written before canonicalising lacks
/// that part, so it is tried both ways.
fn inside_of<'a>(path: &'a Path, was: &Path) -> Option<&'a Path> {
    if !path.is_absolute() {
        return None;
    }
    path.strip_prefix(was).ok().or_else(|| {
        let bare = was.strip_prefix("/private").ok()?;
        path.strip_prefix(Path::new("/").join(bare)).ok()
    })
}

/// The data packages' resource paths, each relative to its package's own folder.
fn repair_packages(dir: &Path, was: &Path) -> Result<usize, String> {
    let Ok(entries) = std::fs::read_dir(dir.join("packages")) else {
        return Ok(0);
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path().join("datapackage.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut package: Value =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let inside = Path::new("packages").join(entry.file_name());
        let (from, to) = (was.join(&inside), dir.join(&inside));
        let mut changed = 0;
        for resource in package
            .get_mut("resources")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            changed += repair_value(resource.get_mut("path"), &from, &to, was);
        }
        if changed > 0 {
            crate::column_record::write_package(&path, &package)?;
            count += changed;
        }
    }
    Ok(count)
}

/// The tables and skipped files `.sources.json` records, relative to the Protocol folder, and those
/// it records by an absolute path inside the old folder.
fn repair_sources(dir: &Path, was: &Path) -> Result<usize, String> {
    let path = dir.join(SOURCES_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(0);
    };
    let mut value: Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut count = 0;
    for (list, key) in [("tables", "file"), ("skipped", "path")] {
        for item in value
            .get_mut(list)
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            count += repair_value(item.get_mut(key), was, dir, was);
            // A source kept in the folder, named by its absolute path, is pointed inside `dir`.
            if let Some(named) = item.get_mut(key) {
                let fixed = named
                    .as_str()
                    .and_then(|p| inside_of(Path::new(p), was))
                    .map(|inside| dir.join(inside).to_string_lossy().into_owned());
                if let Some(fixed) = fixed.filter(|f| named.as_str() != Some(f.as_str())) {
                    *named = Value::String(fixed);
                    count += 1;
                }
            }
        }
    }
    if count > 0 {
        write_json(&path, &value)?;
    }
    Ok(count)
}

/// Each dataset's source file and each skipped source the model records, relative to the
/// Protocol folder.
fn repair_model(dir: &Path, was: &Path) -> Result<usize, String> {
    let path = dir.join(MODEL_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(0);
    };
    let mut model =
        brightfield_ossie::read(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut count = 0;
    for dataset in &mut model.datasets {
        if let Some(fixed) = repaired(&dataset.meta.source_file, was, dir, was) {
            dataset.meta.source_file = fixed;
            count += 1;
        }
    }
    for skipped in &mut model.meta.skipped_sources {
        if let Some(fixed) = repaired(&skipped.path, was, dir, was) {
            skipped.path = fixed;
            count += 1;
        }
    }
    if count > 0 {
        std::fs::write(&path, brightfield_ossie::write(&model))
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(count)
}

/// [`repaired`] over a JSON string in place, answering 1 when it was rewritten.
fn repair_value(value: Option<&mut Value>, from: &Path, to: &Path, was: &Path) -> usize {
    let Some(value) = value else {
        return 0;
    };
    match value.as_str().and_then(|p| repaired(p, from, to, was)) {
        Some(fixed) => {
            *value = Value::String(fixed);
            1
        }
        None => 0,
    }
}

/// `path`, written relative to the folder `from`, written again relative to `to`, when it is
/// relative, points outside the Protocol folder `was`, names no file from `to`, and named a file
/// that is still there from `from`. `None` when it needs no rewriting: absolute, an address,
/// inside the folder, still found from the new place (a parent holding the Protocol and its data
/// moved together, or copied), or found from neither place
/// (`a_path_still_found_from_the_new_place_or_found_from_neither_is_left`).
fn repaired(path: &str, from: &Path, to: &Path, was: &Path) -> Option<String> {
    if path.is_empty() || path.contains("://") || Path::new(path).is_absolute() {
        return None;
    }
    if folded(&to.join(path)).exists() {
        return None;
    }
    let target = folded(&from.join(path));
    if target.starts_with(was) || !target.exists() {
        return None;
    }
    let fixed = crate::own_files::relative_to(&target, to).ok()?;
    (fixed != path).then_some(fixed)
}

/// `path` with each `.` dropped and each `..` taking off the part before it, without asking the
/// file system, since the folder it was written from is no longer there.
fn folded(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The name the analyst typed, trimmed, or why it cannot name a folder.
fn checked_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A Protocol needs a name.".to_string());
    }
    if name.contains('/') || name.contains('\0') {
        return Err(format!(
            "{name} cannot name a Protocol: a name holds no /, since it names a folder."
        ));
    }
    if name.starts_with('.') {
        return Err(format!(
            "{name} cannot name a Protocol: a name starting with . hides its folder."
        ));
    }
    Ok(name)
}

/// Whether `a` and `b` are one folder, reached by two spellings.
fn same_folder(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// Move the Protocol in `dir` to `to`, which is a folder claimed for it when `claimed`, renaming it
/// `name` in its manifest when one is given; then repair, record and move the database. A move that
/// fails, at any step before the folder has moved, puts the manifest back and gives up the claim.
/// Answers what an original copied to another volume left behind, if anything, and what did not
/// follow the folder.
fn move_to(
    dir: &Path,
    to: &Path,
    name: Option<&str>,
    claimed: bool,
    ops: &FolderOps,
) -> Result<(Option<String>, Vec<String>), String> {
    let release = |why: String| {
        if claimed {
            let _ = std::fs::remove_dir(to);
        }
        why
    };
    let was = dir
        .canonicalize()
        .map_err(|e| release(format!("{}: {e}", dir.display())))?;
    let database = working_database(dir);
    let manifest = dir.join(MANIFEST);
    let before = std::fs::read_to_string(&manifest)
        .map_err(|e| release(format!("{}: {e}", manifest.display())))?;
    let give_up = |why: String| {
        let _ = std::fs::write(&manifest, &before);
        release(why)
    };
    if let Some(name) = name {
        set_manifest_name(dir, name).map_err(give_up)?;
    }
    let left_behind = move_folder(dir, to, claimed, ops).map_err(give_up)?;
    let now = to.canonicalize().unwrap_or_else(|_| to.to_path_buf());
    // The folder has moved, so what follows cannot fail the move: each step that fails says what
    // did not follow it.
    let mut warnings = Vec::new();
    // Its trust follows it, so a move the analyst asked for does not leave it restricted.
    if let Err(e) = crate::trust::Store::current().repoint(&was, &now) {
        warnings.push(format!("{TRUST_NOT_FOLLOWED}: {e}"));
    }
    if let Err(e) = repair(&now, &was) {
        warnings.push(format!("{SOURCES_NOT_FOLLOWED}: {e}"));
    }
    if now.join(SOURCES_FILE).is_file() {
        if let Err(e) = record_dir(&now) {
            warnings.push(format!("{PLACE_NOT_RECORDED}: {e}"));
        }
    }
    if let Err(e) = move_database(database, &now) {
        warnings.push(format!("{DATABASE_NOT_FOLLOWED}: {e}"));
    }
    Ok((left_behind, warnings))
}

/// Move the folder `dir` to `to`. A move to another volume is a copy, checked, and then the
/// original removed; a claimed folder in the way, on a system whose rename will not replace one,
/// is given up and the rename tried again.
fn move_folder(
    dir: &Path,
    to: &Path,
    claimed: bool,
    ops: &FolderOps,
) -> Result<Option<String>, String> {
    let failed = |e: io::Error| {
        format!(
            "{} could not be moved to {}: {e}",
            dir.display(),
            to.display()
        )
    };
    match (ops.rename)(dir, to) {
        Ok(()) => Ok(None),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => copy_then_remove(dir, to, ops),
        Err(_) if claimed => {
            let _ = std::fs::remove_dir(to);
            match (ops.rename)(dir, to) {
                Ok(()) => Ok(None),
                Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                    std::fs::create_dir(to).map_err(failed)?;
                    copy_then_remove(dir, to, ops)
                }
                Err(e) => Err(failed(e)),
            }
        }
        Err(e) => Err(failed(e)),
    }
}

/// Copy the folder `dir` into the empty folder `to`, check the copy holds each file of `dir` by its
/// path and size (not by its content), and only then remove `dir`. A copy that fails or does not
/// match is taken away again, leaving `to` empty and `dir` as it was. Once it matches, the move
/// is done: an original that cannot all be removed is answered as the sentence naming it.
fn copy_then_remove(dir: &Path, to: &Path, ops: &FolderOps) -> Result<Option<String>, String> {
    let copied = copy_tree(dir, to).and_then(|()| {
        if listing(dir)? == listing(to)? {
            Ok(())
        } else {
            Err(io::Error::other(
                "the copy does not hold every file the folder does",
            ))
        }
    });
    if let Err(e) = copied {
        if let Ok(entries) = std::fs::read_dir(to) {
            for entry in entries.flatten() {
                let path = entry.path();
                let _ = if path.is_dir() && !path.is_symlink() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
            }
        }
        return Err(format!(
            "{} could not be copied to {}: {e}",
            dir.display(),
            to.display()
        ));
    }
    Ok((ops.remove)(dir).err().map(|e| {
        format!(
            "{} was copied to {}, but the original could not all be removed: {e}. \
             Remove what is left of it in the Finder.",
            dir.display(),
            to.display()
        )
    }))
}

/// Copy everything in `from` into the folder `to`, links as links.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            let points_at = std::fs::read_link(&source)?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(points_at, &target)?;
            #[cfg(not(unix))]
            std::fs::copy(source.parent().unwrap_or(from).join(points_at), &target).map(|_| ())?;
        } else if kind.is_dir() {
            std::fs::create_dir(&target)?;
            copy_tree(&source, &target)?;
        } else {
            std::fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

/// Every file and folder under `dir`, by its path inside it, with a file's size.
fn listing(dir: &Path) -> io::Result<Vec<(PathBuf, Option<u64>)>> {
    fn walk(root: &Path, at: &Path, out: &mut Vec<(PathBuf, Option<u64>)>) -> io::Result<()> {
        for entry in std::fs::read_dir(at)? {
            let entry = entry?;
            let path = entry.path();
            let inside = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            let kind = entry.file_type()?;
            if kind.is_dir() {
                out.push((inside, None));
                walk(root, &path, out)?;
            } else {
                out.push((inside, Some(std::fs::symlink_metadata(&path)?.len())));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

/// Set the top-level `name:` in the manifest of the Protocol in `dir` to `name`, keeping every
/// other line as it was.
fn set_manifest_name(dir: &Path, name: &str) -> Result<(), String> {
    let path = dir.join(MANIFEST);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value = serde_yaml::to_string(name).map_err(|e| e.to_string())?;
    let line = format!("name: {}", value.trim_end());
    let mut found = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|l| {
            if !found && l.starts_with("name:") {
                found = true;
                line.clone()
            } else {
                l.to_string()
            }
        })
        .collect();
    if !found {
        out.insert(0, line);
    }
    let mut written = out.join("\n");
    if text.ends_with('\n') || text.is_empty() {
        written.push('\n');
    }
    if written == text {
        return Ok(());
    }
    std::fs::write(&path, written).map_err(|e| format!("{}: {e}", path.display()))
}

/// Record the folder `dir` is in now, canonical, as `dir` in its `.sources.json`, keeping the
/// rest of the record as it was.
fn record_dir(dir: &Path) -> Result<(), String> {
    let path = dir.join(SOURCES_FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut value: Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let now = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let Some(map) = value.as_object_mut() else {
        return Err(format!("{}: not a record of tables", path.display()));
    };
    let place = Value::String(now.to_string_lossy().into_owned());
    if map.get("dir") == Some(&place) {
        return Ok(());
    }
    map.insert("dir".to_string(), place);
    write_json(&path, &value)
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

/// The database arc keeps for the Protocol in `dir` in its data folder, outside the folder.
/// `None` for one whose manifest names its own `db:` inside the folder, which moves with it, and
/// for one whose manifest arc cannot read.
fn working_database(dir: &Path) -> Option<PathBuf> {
    let db = arc::spec::protocol_db_path(dir).ok()?;
    (!db.starts_with(dir)).then_some(db)
}

/// The database arc keyed to the folder `was` that the Protocol now in `now` had there: the
/// database of the same name, in the key folder of arc's data folder whose `protocol-path` names
/// `was`. arc names the folder in the clear beside each database for this reading.
fn database_keyed_to(was: &Path, now: &Path) -> Option<PathBuf> {
    let new = working_database(now)?;
    let root = new.parent()?.parent()?;
    let wanted = format!("{}\n", was.display());
    std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|key| {
            std::fs::read_to_string(key.join(PROTOCOL_PATH_FILE)).is_ok_and(|named| named == wanted)
        })
        .map(|key| key.join(new.file_name().unwrap_or_default()))
}

/// Move the database `old`, when there is one, to where arc looks for the database of the Protocol
/// in `now`, with the log DuckDB writes beside it, and name `now` beside it as arc does. A
/// database that cannot be moved is deleted, so the next run builds it again rather than a run
/// at the old key finding a database whose Protocol has gone.
fn move_database(old: Option<PathBuf>, now: &Path) -> Result<(), String> {
    let Some(old) = old else {
        return Ok(());
    };
    let Some(new) = working_database(now) else {
        return Ok(());
    };
    if new == old {
        return Ok(());
    }
    let _writing = crate::own_files::database_lock();
    let wal = |db: &Path| {
        let mut name = db.as_os_str().to_os_string();
        name.push(".wal");
        PathBuf::from(name)
    };
    let mut result = Ok(());
    if old.is_file() {
        let moved = new
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| {
                // A database left at the new key belongs to a Protocol no longer there.
                let _ = std::fs::remove_file(wal(&new));
                std::fs::rename(&old, &new)
            })
            .and_then(|()| match std::fs::rename(wal(&old), wal(&new)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            })
            .and_then(|()| {
                let marker = new.parent().unwrap_or(now).join(PROTOCOL_PATH_FILE);
                std::fs::write(marker, format!("{}\n", now.display()))
            });
        if let Err(e) = moved {
            let _ = std::fs::remove_file(&old);
            let _ = std::fs::remove_file(wal(&old));
            let _ = std::fs::remove_file(&new);
            let _ = std::fs::remove_file(wal(&new));
            result = Err(format!(
                "its working database could not be moved, so the next run builds it again: {e}"
            ));
        }
    }
    // The key folder the old path had, once nothing of a database is left in it.
    if let Some(key) = old.parent() {
        let left: Vec<_> = std::fs::read_dir(key)
            .map(|entries| entries.flatten().map(|e| e.file_name()).collect())
            .unwrap_or_default();
        if left.iter().all(|n| n == PROTOCOL_PATH_FILE) {
            let _ = std::fs::remove_file(key.join(PROTOCOL_PATH_FILE));
            let _ = std::fs::remove_dir(key);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch tree under the system's temporary folder, made afresh for `name`.
    fn tree(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("bf-rename-unit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("p/old")).unwrap();
        std::fs::create_dir_all(root.join("p/data")).unwrap();
        std::fs::write(root.join("p/data/x.csv"), "a\n1\n").unwrap();
        root
    }

    #[test]
    fn a_path_leaving_the_folder_is_written_from_the_new_place() {
        let root = tree("leaving");
        let was = root.join("p/old");
        let fixed = repaired("../data/x.csv", &was, &root.join("p/group/old"), &was);
        assert_eq!(fixed.as_deref(), Some("../../data/x.csv"));
    }

    #[test]
    fn a_path_inside_the_folder_an_absolute_path_and_an_address_are_left() {
        let root = tree("left");
        let was = root.join("p/old");
        let to = root.join("q/new");
        assert_eq!(repaired("data/x.csv", &was, &to, &was), None);
        assert_eq!(repaired("/data/x.csv", &was, &to, &was), None);
        assert_eq!(repaired("https://example.org/x.csv", &was, &to, &was), None);
        assert_eq!(repaired("../old/x.csv", &was, &to, &was), None);
    }

    #[test]
    fn a_path_still_found_from_the_new_place_or_found_from_neither_is_left() {
        let root = tree("found");
        let was = root.join("p/old");
        // A rename within the parent reads the same.
        assert_eq!(
            repaired("../data/x.csv", &was, &root.join("p/new"), &was),
            None
        );
        // The parent and its data copied together: the copy finds its own data.
        std::fs::create_dir_all(root.join("copy/data")).unwrap();
        std::fs::write(root.join("copy/data/x.csv"), "a\n1\n").unwrap();
        assert_eq!(
            repaired("../data/x.csv", &was, &root.join("copy/old"), &was),
            None
        );
        // Nothing at the old place either: nothing better to point at.
        assert_eq!(
            repaired("../data/gone.csv", &was, &root.join("q/new"), &was),
            None
        );
    }

    #[test]
    fn a_file_key_is_read_and_written_as_yaml_reads_it() {
        let was = Path::new("/p/hugh's old");
        let dir = Path::new("/p/new's");
        assert_eq!(
            moved_file_line("    file: '/p/hugh''s old/.grid/t.parquet'", dir, was).as_deref(),
            Some("    file: '/p/new''s/.grid/t.parquet'")
        );
        assert_eq!(
            moved_file_line(r#"  - file: "/p/hugh's old/a \"b\".parquet""#, dir, was).as_deref(),
            Some(r#"  - file: "/p/new's/a \"b\".parquet""#)
        );
        assert_eq!(
            moved_file_line(
                "  { file: /p/hugh's old/t.parquet }",
                Path::new("/q/n"),
                was
            )
            .as_deref(),
            Some("  { file: /q/n/t.parquet }")
        );
        // A comma in a block's plain value is part of it, and the value is quoted once it needs to be.
        let fixed = moved_file_line("    file: /p/hugh's old/a,b.parquet", dir, was)
            .expect("the line is rewritten");
        assert_eq!(fixed, "    file: '/p/new''s/a,b.parquet'");
        let read: serde_yaml::Value = serde_yaml::from_str(fixed.trim()).expect("it parses");
        assert_eq!(read["file"].as_str(), Some("/p/new's/a,b.parquet"));
        // A flow mapping spread over lines: its `,` and `}` stay the mapping's.
        let spread = "data: {\n  file: /p/hugh's old/a.parquet,\n  format: parquet }\n";
        let fixed: Vec<String> = spread
            .split('\n')
            .map(|l| moved_file_line(l, Path::new("/q/n"), was).unwrap_or_else(|| l.to_string()))
            .collect();
        assert_eq!(fixed[1], "  file: /q/n/a.parquet,");
        let read: serde_yaml::Value = serde_yaml::from_str(&fixed.join("\n")).expect("it parses");
        assert_eq!(read["data"]["file"].as_str(), Some("/q/n/a.parquet"));
        let last =
            moved_file_line("  file: /p/hugh's old/b.parquet }", dir, was).expect("rewritten");
        assert_eq!(last, "  file: '/p/new''s/b.parquet' }");
        // Not the key: another key ending in it, and an address inside a value.
        assert_eq!(
            moved_file_line("    profile: '/p/hugh''s old/x'", dir, was),
            None
        );
        assert_eq!(
            moved_file_line("    url: 'file:///p/hugh''s old/x'", dir, was),
            None
        );
    }

    #[test]
    fn a_location_is_read_from_the_folder_the_protocol_is_in() {
        let parent = Path::new("/p/protocols");
        assert_eq!(
            resolve_location("group", parent),
            Path::new("/p/protocols/group")
        );
        assert_eq!(resolve_location("../other", parent), Path::new("/p/other"));
        assert_eq!(resolve_location(" /q/abs ", parent), Path::new("/q/abs"));
        let home = std::env::home_dir().unwrap_or_default();
        assert_eq!(resolve_location("~/Work", parent), home.join("Work"));
        assert_eq!(resolve_location("~", parent), home);
    }

    #[test]
    fn names_that_cannot_name_a_folder_are_refused() {
        for bad in ["", "  ", "a/b", ".hidden", ".", ".."] {
            assert!(checked_name(bad).is_err(), "{bad:?}");
        }
        assert_eq!(checked_name("  sales 2024 ").unwrap(), "sales 2024");
    }
}
