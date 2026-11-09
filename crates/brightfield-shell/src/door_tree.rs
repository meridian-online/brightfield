//! The tree of the Protocols folder the front door draws beneath Recents, and the run state each
//! of its rows says.
//!
//! The tree is read from disk, and nothing about it is kept anywhere else: an ordinary folder is a
//! group and a folder holding `arcform.yaml` is a Protocol. Groups nest to any depth. A Protocol is
//! a leaf, and nothing inside one is looked at for others, so a Protocol that keeps a copy of
//! another among its files is still one row. Arranging Protocols is the Finder's work; the door
//! shows the result the next time it is shown.

use std::path::{Path, PathBuf};

use brightfield_protocol::SeamStatus;
use brightfield_workbench::RunState;

/// One entry of the tree: a folder of other entries, or a Protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// An ordinary folder, named as the Finder names it, with what it holds.
    Group { name: String, children: Vec<Node> },
    /// A folder holding `arcform.yaml`, named for the folder, at `dir`.
    Protocol { name: String, dir: PathBuf },
}

impl Node {
    /// The name the door draws for this entry.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Node::Group { name, .. } | Node::Protocol { name, .. } => name,
        }
    }
}

/// The tree under `root`: at each level the groups first and then the Protocols, each sorted by
/// name with case ignored. Hidden folders, whose names start with a dot, are skipped, and so is
/// anything that is not a folder. A root that is missing or cannot be read is an empty tree, and
/// is not made.
#[must_use]
pub fn read(root: &Path) -> Vec<Node> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut groups = Vec::new();
    let mut protocols = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        // Followed through a link, as the Finder shows a linked folder's contents.
        if !path.is_dir() {
            continue;
        }
        if path.join(arc::spec::MANIFEST_FILENAME).is_file() {
            protocols.push(Node::Protocol { name, dir: path });
        } else {
            let children = read(&path);
            groups.push(Node::Group { name, children });
        }
    }
    let by_name = |a: &Node, b: &Node| {
        a.name()
            .to_lowercase()
            .cmp(&b.name().to_lowercase())
            .then_with(|| a.name().cmp(b.name()))
    };
    groups.sort_by(by_name);
    protocols.sort_by(by_name);
    groups.extend(protocols);
    groups
}

/// What the Protocol in `dir` reads as on the door: what its newest run record says, read now.
///
/// A record says the Protocol is [`RunState::Failed`] when any step failed, and
/// [`RunState::Fresh`] when its steps ran or were skipped as current. With no record, a Protocol
/// whose loads computed while its sources were added has arc's working database, and is
/// [`RunState::Previewed`]; one without it, or a folder whose manifest will not read, is
/// [`RunState::NeverRun`]. Only records of this Protocol's own `name:` count, since two Protocols
/// in one folder write their records into one place.
#[must_use]
pub fn run_state(dir: &Path) -> RunState {
    let Ok(manifest) = arc::spec::Manifest::load(dir) else {
        return RunState::NeverRun;
    };
    if let Some(view) = crate::run::records_newest_first(dir, &manifest.name).next() {
        let statuses = view.seam_statuses();
        if statuses.values().any(|s| *s == SeamStatus::Failed) {
            return RunState::Failed;
        }
        if statuses
            .values()
            .any(|s| matches!(s, SeamStatus::Ok | SeamStatus::Skipped))
        {
            return RunState::Fresh;
        }
        return match view.run.outcome {
            brightfield_protocol::contract::Outcome::Success => RunState::Fresh,
            _ => RunState::Failed,
        };
    }
    let previewed = arc::spec::protocol_db_path(dir).is_ok_and(|db| db.is_file());
    if previewed {
        RunState::Previewed
    } else {
        RunState::NeverRun
    }
}
