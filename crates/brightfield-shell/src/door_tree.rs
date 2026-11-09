//! The tree of the Protocols folder the front door draws beneath Recents, and the run state each
//! of its rows says.
//!
//! The tree is read from disk, and nothing about it is kept anywhere else: an ordinary folder is a
//! group and a folder holding `arcform.yaml` is a Protocol. Groups nest to any depth. A Protocol is
//! a leaf, and nothing inside one is looked at for others, so a Protocol that keeps a copy of
//! another among its files is still one row. Arranging Protocols is the Finder's work; the door
//! shows the result the next time it is shown.

use std::path::{Path, PathBuf};

use brightfield_protocol::contract::Outcome;
use brightfield_protocol::{ContractView, SeamStatus};
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

/// How many groups deep the tree is read. A folder deeper than this is not listed.
pub const MAX_DEPTH: usize = 16;

/// How many directory entries one reading of the tree looks at before it stops.
pub const MAX_ENTRIES: usize = 5_000;

/// The tree under `root`: at each level the groups first and then the Protocols, each sorted by
/// name with case ignored. Hidden folders, whose names start with a dot, are skipped, and so is
/// anything that is not a folder. A root that is missing or cannot be read is an empty tree, and
/// is not made.
///
/// It is read on the window's own thread each time the door is shown, so it is bounded. A link to
/// a folder is not followed, since a link back up the tree would be read for ever, and the walk
/// stops quietly, keeping what it has read, past [`MAX_DEPTH`] groups deep or [`MAX_ENTRIES`]
/// entries.
#[must_use]
pub fn read(root: &Path) -> Vec<Node> {
    let mut budget = MAX_ENTRIES;
    read_level(root, 0, &mut budget)
}

fn read_level(dir: &Path, depth: usize, budget: &mut usize) -> Vec<Node> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut groups = Vec::new();
    let mut protocols = Vec::new();
    for entry in entries.flatten() {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        // The entry's own type, not the type of what it points at: a link is neither a group
        // nor a Protocol here.
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let path = entry.path();
        if path.join(arc::spec::MANIFEST_FILENAME).is_file() {
            protocols.push(Node::Protocol { name, dir: path });
        } else {
            let children = if depth + 1 < MAX_DEPTH {
                read_level(&path, depth + 1, budget)
            } else {
                Vec::new()
            };
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

/// What the Protocol in `dir` reads as on the door: what its newest whole run's record says, read
/// now. Runs Brightfield started itself to preview the sources ([`crate::run::mark_preview`]) are
/// not whole runs and are passed over (`a_previewed_protocol_reads_previewed_not_run` in
/// `tests/door_tree.rs`), and records of another Protocol's `name:` are passed over too, since
/// two Protocols in one folder write their records into one place.
///
/// - A run whose outcome is an error or partial is [`RunState::Failed`].
/// - A run that never finished, stopped before arc completed its record, counts as no run.
/// - Otherwise the steps decide: one that failed is [`RunState::Failed`]; steps that ran or were
///   skipped as current are [`RunState::Fresh`], or [`RunState::StaleOwnEdit`] when the manifest
///   is no longer the one the run ran.
/// - With no whole run, a Protocol that has been previewed (a preview run, or arc's working
///   database) is [`RunState::Previewed`], and one that has not, or a folder whose manifest will
///   not read, is [`RunState::NeverRun`].
#[must_use]
pub fn run_state(dir: &Path) -> RunState {
    let Ok(manifest) = arc::spec::Manifest::load(dir) else {
        return RunState::NeverRun;
    };
    let not_run = || {
        let previewed = !crate::run::preview_ids(dir).is_empty()
            || arc::spec::protocol_db_path(dir).is_ok_and(|db| db.is_file());
        if previewed {
            RunState::Previewed
        } else {
            RunState::NeverRun
        }
    };
    let Some((path, view)) = newest_whole_run(dir, &manifest.name) else {
        return not_run();
    };
    if !view.run.complete {
        return not_run();
    }
    match view.run.outcome {
        Outcome::Error | Outcome::Partial => return RunState::Failed,
        Outcome::Success | Outcome::Unknown => {}
    }
    let statuses = view.seam_statuses();
    let ran = if statuses.values().any(|s| *s == SeamStatus::Failed) {
        RunState::Failed
    } else if statuses
        .values()
        .any(|s| matches!(s, SeamStatus::Ok | SeamStatus::Skipped))
        || view.run.outcome == Outcome::Success
    {
        RunState::Fresh
    } else {
        return not_run();
    };
    if ran == RunState::Fresh && !ran_this_manifest(&path, dir) {
        return RunState::StaleOwnEdit;
    }
    ran
}

/// When the newest whole run of the Protocol named `protocol` in `dir` was recorded: its record's
/// modification time, or `None` when it has no whole run on record.
#[must_use]
pub fn newest_whole_run_at(dir: &Path, protocol: &str) -> Option<std::time::SystemTime> {
    let (path, _) = newest_whole_run(dir, protocol)?;
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The newest whole run of the Protocol named `protocol` in `dir`: its record's path and the view
/// read from it. A record that will not parse is passed over.
fn newest_whole_run(dir: &Path, protocol: &str) -> Option<(PathBuf, ContractView)> {
    crate::run::whole_run_paths_newest_first(dir)
        .into_iter()
        .find_map(|path| match brightfield_protocol::load_contract(&path) {
            Ok(view) if view.run.protocol == protocol => Some((path, view)),
            _ => None,
        })
}

/// Whether the run recorded at `record` ran the manifest `dir` holds now: arc records the SHA-256
/// of the manifest's bytes on each run. A record with no hash, or a manifest that cannot be read,
/// is taken as the same, so a row is not called stale on evidence that is not there.
fn ran_this_manifest(record: &Path, dir: &Path) -> bool {
    use sha2::Digest;
    let recorded = std::fs::read(record)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|json| {
            json.pointer("/run/protocol/manifest_sha256")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        });
    let (Some(recorded), Ok(bytes)) = (
        recorded,
        std::fs::read(dir.join(arc::spec::MANIFEST_FILENAME)),
    ) else {
        return true;
    };
    format!("{:x}", sha2::Sha256::digest(&bytes)) == recorded
}
