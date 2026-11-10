//! The Protocols a data file already has: the known Protocols that read it as a source.
//!
//! Opening a lone data file used to make a new Protocol every time, so a CSV double-clicked five
//! times left five Protocols of it. Before a file makes one, the window looks here for the
//! Protocols that already use it, and reopens the one, or asks which, rather than making another.
//!
//! A Protocol is known when it is in the Protocols folder's tree ([`crate::door_tree::read`],
//! bounded as the door's reading is) or among the Recents the window remembers. Its sources are
//! the files its `.sources.json` records, compared with the file opened by their resolved absolute
//! paths, so a path spelled through a link or with `..` in it still matches.

use std::path::{Path, PathBuf};

/// A known Protocol that reads the file being opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileProtocol {
    /// The Protocol's name, as the door and the locator band name it: its folder's.
    pub name: String,
    /// The Protocol folder.
    pub dir: PathBuf,
    /// How many source files it reads, the file being opened among them.
    pub sources: usize,
}

/// What opening a lone data file does, given the known Protocols that already read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileOpen {
    /// No known Protocol reads it: make a new one, as every open used to.
    New,
    /// Exactly one does, and it reads nothing else: reopen it.
    Reopen(PathBuf),
    /// Several do, or the one that does reads other files too: ask which, or a new Protocol.
    Ask(Vec<FileProtocol>),
}

/// What opening `file` does, its known Protocols being those in the Protocols folder's tree and
/// those whose folders `recents` name.
#[must_use]
pub fn decide(file: &Path, recents: &[PathBuf]) -> FileOpen {
    let mut using = protocols_using(file, recents);
    match using.as_slice() {
        [] => FileOpen::New,
        [only] if only.sources == 1 => FileOpen::Reopen(using.remove(0).dir),
        _ => FileOpen::Ask(using),
    }
}

/// The known Protocols that read `file`, by name; see [`decide`] for which are known.
#[must_use]
pub fn protocols_using(file: &Path, recents: &[PathBuf]) -> Vec<FileProtocol> {
    let target = resolved(file);
    let mut candidates: Vec<PathBuf> = Vec::new();
    collect_protocols(
        &crate::door_tree::read(&crate::protocols_dir::root()),
        &mut candidates,
    );
    candidates.extend(recents.iter().cloned());
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut found = Vec::new();
    for dir in candidates {
        let key = resolved(&dir);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let Some(files) = crate::own_files::source_files(&dir) else {
            continue;
        };
        let files: Vec<PathBuf> = files.iter().map(|f| resolved(f)).collect();
        if files.contains(&target) {
            found.push(FileProtocol {
                name: dir.file_name().map_or_else(
                    || dir.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                ),
                sources: files.len(),
                dir,
            });
        }
    }
    found.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.dir.cmp(&b.dir))
    });
    found
}

/// The Protocol folders in `nodes`, groups walked into.
fn collect_protocols(nodes: &[crate::door_tree::Node], out: &mut Vec<PathBuf>) {
    for node in nodes {
        match node {
            crate::door_tree::Node::Group { children, .. } => collect_protocols(children, out),
            crate::door_tree::Node::Protocol { dir, .. } => out.push(dir.clone()),
        }
    }
}

/// `path` with its links and `..` resolved where it exists, and made absolute where it does not.
fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}
