//! Renaming a Protocol, moving it to another folder, and finding its sources again after it was
//! moved in the Finder.
//!
//! A Protocol is a folder, so its name is its folder's name, and the name in its `arcform.yaml`
//! is set beside it. Three things depend on where the folder is, and each move takes them along:
//!
//! - **Source paths written relative to the folder that point outside it.** A data package names
//!   its file from its own folder (`../../../data/sales.csv`), so a folder that moves to another
//!   parent no longer finds it. [`repair`] writes each such path again from the new place. A path
//!   inside the folder moves with it and is left alone, and so is an absolute path.
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
        if let Err(e) = move_database(database, &canonical) {
            eprintln!("warning: {}: {e}", dir.display());
        }
        return Ok(dir);
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
    move_to(&dir, &to, Some(name), !itself, &|from, to| {
        std::fs::rename(from, to)
    })?;
    Ok(to)
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
    relocate_with(dir, new_parent, &|from, to| std::fs::rename(from, to))
}

/// [`relocate`], moving the folder with `rename_folder` in place of [`std::fs::rename`], so a test
/// can make it fail as a move to another volume does. When it fails with
/// [`io::ErrorKind::CrossesDevices`], the folder is copied, the copy checked against it, and only
/// then the original removed.
///
/// # Errors
///
/// As [`relocate`], and a copy that cannot be made or does not match the original, which is then
/// left where it was.
pub fn relocate_with(
    dir: &Path,
    new_parent: &Path,
    rename_folder: &dyn Fn(&Path, &Path) -> io::Result<()>,
) -> Result<PathBuf, String> {
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
        return Ok(dir);
    }
    let stem = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| format!("{}: names no folder", dir.display()))?;
    let (to, name) = crate::protocols_dir::claim(&new_parent, &stem).map_err(|e| e.to_string())?;
    let renamed = (name != stem).then_some(name.as_str());
    move_to(&dir, &to, renamed, true, rename_folder)?;
    Ok(to)
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
/// records no place has its place recorded. Answers how many source paths were repaired, `None`
/// when it had not moved.
///
/// # Errors
///
/// A package, record or model that cannot be read or written.
pub fn follow_a_move(dir: &Path) -> Result<Option<usize>, String> {
    let Some(was) = moved_since(dir) else {
        if dir.join(SOURCES_FILE).is_file() {
            record_dir(dir)?;
        }
        return Ok(None);
    };
    let now = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let repaired = repair(&now, &was)?;
    if !was.join(MANIFEST).is_file() {
        if let Err(e) = move_database(database_keyed_to(&was, &now), &now) {
            eprintln!("warning: {}: {e}", now.display());
        }
    }
    record_dir(&now)?;
    Ok(Some(repaired))
}

/// Rewrite each source path in the Protocol in `dir` that is written relative to its folder and
/// points outside it, so it points from `dir` at the file it pointed at from `was`: the paths in
/// `packages/*/datapackage.json`, in `.sources.json`, and in the model. The data each saved chart
/// under `panels/` reads, an absolute path inside the folder, is pointed inside `dir`. Answers
/// how many were rewritten. A file that is not there is passed over.
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
    Ok(count)
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

/// `line` with the absolute path its `file:` names written again inside `dir`, when that path is
/// inside `was` and reads differently from inside `dir`. `None` for a line needing no change.
fn moved_file_line(line: &str, dir: &Path, was: &Path) -> Option<String> {
    let key = line.find("file:")?;
    let after = key + "file:".len();
    let rest = &line[after..];
    let value_at = after + (rest.len() - rest.trim_start().len());
    let value = &line[value_at..];
    let (quote, body) = match value.chars().next()? {
        q @ ('\'' | '"') => (Some(q), &value[1..]),
        _ => (None, value),
    };
    let end = match quote {
        Some(q) => body.find(q)?,
        None => body
            .find(|c: char| c.is_whitespace() || c == ',' || c == '}')
            .unwrap_or(body.len()),
    };
    let named = &body[..end];
    let inside = inside_of(Path::new(named), was)?;
    let fixed = dir.join(inside).to_string_lossy().into_owned();
    if fixed == named {
        return None;
    }
    let start = value_at + usize::from(quote.is_some());
    Some(format!(
        "{}{}{}",
        &line[..start],
        fixed,
        &line[start + end..]
    ))
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

/// The tables and skipped files `.sources.json` records, relative to the Protocol folder.
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
/// relative, points outside the Protocol folder `was`, and reads differently from `to`. `None`
/// when it needs no rewriting: absolute, an address, inside the folder, or the same from both.
fn repaired(path: &str, from: &Path, to: &Path, was: &Path) -> Option<String> {
    if path.is_empty() || path.contains("://") || Path::new(path).is_absolute() {
        return None;
    }
    let target = folded(&from.join(path));
    if target.starts_with(was) {
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
/// fails puts the manifest back and gives up the claim.
fn move_to(
    dir: &Path,
    to: &Path,
    name: Option<&str>,
    claimed: bool,
    rename_folder: &dyn Fn(&Path, &Path) -> io::Result<()>,
) -> Result<(), String> {
    let was = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let database = working_database(dir);
    let manifest = dir.join(MANIFEST);
    let before =
        std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let give_up = |why: String| {
        let _ = std::fs::write(&manifest, &before);
        if claimed {
            let _ = std::fs::remove_dir(to);
        }
        why
    };
    if let Some(name) = name {
        set_manifest_name(dir, name).map_err(give_up)?;
    }
    move_folder(dir, to, claimed, rename_folder).map_err(give_up)?;
    let now = to
        .canonicalize()
        .map_err(|e| format!("{}: {e}", to.display()))?;
    // The folder has moved, so what follows cannot fail the move: each step says what it left.
    if let Err(e) = repair(&now, &was) {
        eprintln!(
            "warning: {}: a source path was not repaired: {e}",
            now.display()
        );
    }
    if now.join(SOURCES_FILE).is_file() {
        if let Err(e) = record_dir(&now) {
            eprintln!("warning: {e}");
        }
    }
    if let Err(e) = move_database(database, &now) {
        eprintln!("warning: {}: {e}", now.display());
    }
    Ok(())
}

/// Move the folder `dir` to `to`. A move to another volume is a copy, checked, and then the
/// original removed; a claimed folder in the way, on a system whose rename will not replace one,
/// is given up and the rename tried again.
fn move_folder(
    dir: &Path,
    to: &Path,
    claimed: bool,
    rename_folder: &dyn Fn(&Path, &Path) -> io::Result<()>,
) -> Result<(), String> {
    let failed = |e: io::Error| {
        format!(
            "{} could not be moved to {}: {e}",
            dir.display(),
            to.display()
        )
    };
    match rename_folder(dir, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => copy_then_remove(dir, to),
        Err(_) if claimed => {
            let _ = std::fs::remove_dir(to);
            match rename_folder(dir, to) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                    std::fs::create_dir(to).map_err(failed)?;
                    copy_then_remove(dir, to)
                }
                Err(e) => Err(failed(e)),
            }
        }
        Err(e) => Err(failed(e)),
    }
}

/// Copy the folder `dir` into the empty folder `to`, check the copy holds every file at its size,
/// and only then remove `dir`. A copy that fails or does not match is taken away again, leaving
/// `to` empty and `dir` as it was.
fn copy_then_remove(dir: &Path, to: &Path) -> Result<(), String> {
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
    std::fs::remove_dir_all(dir).map_err(|e| {
        format!(
            "{} was copied to {}, but the original could not be removed: {e}",
            dir.display(),
            to.display()
        )
    })
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

    #[test]
    fn a_path_leaving_the_folder_is_written_from_the_new_place() {
        let was = Path::new("/p/old");
        let fixed = repaired("../data/x.csv", was, Path::new("/p/group/old"), was);
        assert_eq!(fixed.as_deref(), Some("../../data/x.csv"));
    }

    #[test]
    fn a_path_inside_the_folder_an_absolute_path_and_an_address_are_left() {
        let was = Path::new("/p/old");
        let to = Path::new("/q/new");
        assert_eq!(repaired("data/x.csv", was, to, was), None);
        assert_eq!(repaired("/data/x.csv", was, to, was), None);
        assert_eq!(repaired("https://example.org/x.csv", was, to, was), None);
        assert_eq!(repaired("../old/x.csv", was, to, was), None);
    }

    #[test]
    fn a_rename_within_the_parent_leaves_a_path_that_reads_the_same() {
        let was = Path::new("/p/old");
        assert_eq!(
            repaired("../data/x.csv", was, Path::new("/p/new"), was),
            None
        );
    }

    #[test]
    fn names_that_cannot_name_a_folder_are_refused() {
        for bad in ["", "  ", "a/b", ".hidden", ".", ".."] {
            assert!(checked_name(bad).is_err(), "{bad:?}");
        }
        assert_eq!(checked_name("  sales 2024 ").unwrap(), "sales 2024");
    }
}
