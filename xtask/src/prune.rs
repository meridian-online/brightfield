//! Keeping the gate's shared target directory to what a later run reuses.
//!
//! The pull-request gate and the nightly build into one target directory so
//! that third-party dependencies compile once. The workspace's own crates
//! cannot be reused: cargo hashes a path package's location into its artifact
//! names, and each run builds from a new worktree, so every run adds a full
//! set of workspace artifacts (on macOS each test binary keeps its object
//! files for debug info) that no later run reads. The incremental directory is
//! the same: its state belongs to crates that will not be compiled again.
//! [`prune`] removes both after a run, under the gate lock, and keeps the
//! third-party artifacts, as
//! `a_prune_removes_the_workspaces_artifacts_and_keeps_third_party_ones`
//! checks.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Stdio;

use crate::steps::Ctx;

/// The names the workspace's own artifacts carry in a target directory.
#[derive(Debug, Default, PartialEq)]
pub struct Workspace {
    /// Package names, as `.fingerprint/` and `build/` name their entries.
    packages: BTreeSet<String>,
    /// Target crate names (a target's name with `-` read as `_`), as `deps/`
    /// and `examples/` name theirs.
    crates: BTreeSet<String>,
}

/// The workspace's names from `cargo metadata --no-deps` output.
pub fn workspace_from_metadata(metadata_json: &str) -> Result<Workspace, String> {
    let metadata: serde_json::Value = serde_json::from_str(metadata_json)
        .map_err(|e| format!("cargo metadata did not print JSON: {e}"))?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or("cargo metadata printed no packages array")?;
    let mut workspace = Workspace::default();
    for package in packages {
        if let Some(name) = package["name"].as_str() {
            workspace.packages.insert(name.to_owned());
        }
        for target in package["targets"].as_array().into_iter().flatten() {
            if let Some(name) = target["name"].as_str() {
                workspace.crates.insert(name.replace('-', "_"));
            }
        }
    }
    if workspace.packages.is_empty() {
        return Err("cargo metadata listed no workspace packages".to_owned());
    }
    Ok(workspace)
}

/// The workspace of the tree at `root`, read with its own cargo.
pub fn workspace_of(root: &Path) -> Result<Workspace, String> {
    let ctx = Ctx {
        root: root.to_path_buf(),
        cargo: "cargo".into(),
        log: None,
    };
    let output = ctx
        .command("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("could not start cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`cargo metadata --no-deps` exited with {}",
            output.status
        ));
    }
    workspace_from_metadata(&String::from_utf8_lossy(&output.stdout))
}

/// What a prune removed.
#[derive(Debug, Default, PartialEq)]
pub struct Pruned {
    /// Directory entries removed: files, and directories with their contents.
    pub entries: usize,
    /// Bytes those entries held.
    pub bytes: u64,
    /// Entries that could not be removed, with why.
    pub failed: Vec<String>,
}

impl Pruned {
    /// One line for the gate's output.
    pub fn line(&self, target: &Path) -> String {
        let gib = self.bytes as f64 / f64::from(1u32 << 30);
        let mut line = format!(
            "pruned the workspace's own artifacts from {}: {} entries, {gib:.1} GiB",
            target.display(),
            self.entries
        );
        if !self.failed.is_empty() {
            line.push_str(&format!(
                "; {} could not be removed, the first: {}",
                self.failed.len(),
                self.failed[0]
            ));
        }
        line
    }
}

/// The name an artifact entry is filed under: the text before the first `-`
/// that is followed by cargo's 16-digit lowercase hexadecimal hash and then
/// the end or a `.`. `None` for an entry without such a hash.
fn unit_name(entry: &str) -> Option<&str> {
    entry.match_indices('-').find_map(|(at, _)| {
        let rest = &entry.as_bytes()[at + 1..];
        let hashed = rest.len() >= 16
            && rest[..16]
                .iter()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
            && (rest.len() == 16 || rest[16] == b'.');
        hashed.then(|| &entry[..at])
    })
}

/// True when `entry`, in the profile directory's subdirectory `kind`, is the
/// workspace's own and no later run reads it.
fn is_workspace_artifact(workspace: &Workspace, kind: &str, entry: &str) -> bool {
    match kind {
        "incremental" => true,
        ".fingerprint" | "build" => {
            unit_name(entry).is_some_and(|name| workspace.packages.contains(name))
        }
        "deps" | "examples" => unit_name(entry).is_some_and(|name| {
            workspace.crates.contains(name)
                || name
                    .strip_prefix("lib")
                    .is_some_and(|name| workspace.crates.contains(name))
        }),
        _ => false,
    }
}

/// Bytes held at `path`, a file or a directory and everything under it; a
/// symbolic link counts as itself.
fn size_of(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| size_of(&entry.path()))
        .sum()
}

/// The profile directories under `target` (`debug`, `release`, and the same
/// under a target-triple directory): those holding a `.fingerprint`.
fn profile_dirs(target: &Path) -> Vec<std::path::PathBuf> {
    let children = |dir: &Path| -> Vec<std::path::PathBuf> {
        fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
            .map(|entry| entry.path())
            .collect()
    };
    let mut found = Vec::new();
    for child in children(target) {
        if child.join(".fingerprint").is_dir() {
            found.push(child);
        } else {
            found.extend(
                children(&child)
                    .into_iter()
                    .filter(|grandchild| grandchild.join(".fingerprint").is_dir()),
            );
        }
    }
    found.sort();
    found
}

/// Remove the workspace's own artifacts and the incremental state from every
/// profile directory under `target`, keeping every third-party artifact. Hold
/// the gate lock while calling it, since a running build reads what it
/// removes.
pub fn prune(target: &Path, workspace: &Workspace) -> Pruned {
    let mut pruned = Pruned::default();
    for profile in profile_dirs(target) {
        for kind in ["incremental", ".fingerprint", "build", "deps", "examples"] {
            let dir = profile.join(kind);
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if !is_workspace_artifact(workspace, kind, name) {
                    continue;
                }
                let path = entry.path();
                let bytes = size_of(&path);
                let removed = if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
                match removed {
                    Ok(()) => {
                        pruned.entries += 1;
                        pruned.bytes += bytes;
                    }
                    Err(e) => pruned.failed.push(format!("{}: {e}", path.display())),
                }
            }
        }
    }
    pruned
}

/// The workspace named by the first of `trees` whose `cargo metadata`
/// answers: a run's own tree, then the checkout that started it, for a run
/// whose tree was never made.
pub fn first_workspace(trees: &[&Path]) -> Result<Workspace, String> {
    let mut why = Vec::new();
    for tree in trees {
        match workspace_of(tree) {
            Ok(workspace) => return Ok(workspace),
            Err(e) => why.push(format!("{}: {e}", tree.display())),
        }
    }
    Err(why.join("; "))
}

/// Prune `target` after a run, and say what was removed or why nothing was.
pub fn after_run(target: &Path, workspace: Result<Workspace, String>, say: impl Fn(&str)) {
    match workspace {
        Ok(workspace) => say(&prune(target, &workspace).line(target)),
        Err(e) => say(&format!(
            "did not prune {}, since no tree named the workspace: {e}",
            target.display()
        )),
    }
}

/// Remove the pull-request gate's worktrees that a killed run left under
/// `trees`, and git's record of them in `root`'s repository. A gate makes its
/// tree only while it holds the gate lock, so with the lock held any
/// `brightfield-gate-*` tree is stale; the nightly's `nightly-*` trees are its
/// launcher's, made before it takes the lock, and are left alone.
pub fn remove_stale_gate_trees(trees: &Path, root: &Path) -> usize {
    let mut removed = 0;
    for entry in fs::read_dir(trees).into_iter().flatten().flatten() {
        let is_gate_tree = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("brightfield-gate-"));
        if is_gate_tree && fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    if removed > 0 {
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["worktree", "prune"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const METADATA: &str = r#"{"packages":[
        {"name":"brightfield-spec","targets":[
            {"name":"brightfield_spec","kind":["lib"]},
            {"name":"gallery-gate","kind":["test"]}]},
        {"name":"xtask","targets":[{"name":"xtask","kind":["bin"]}]}
    ]}"#;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gate-prune-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![0u8; bytes]).unwrap();
    }

    #[test]
    fn the_workspace_names_are_its_packages_and_its_targets_crate_names() {
        let workspace = workspace_from_metadata(METADATA).unwrap();
        assert_eq!(
            workspace.packages,
            ["brightfield-spec", "xtask"].map(String::from).into()
        );
        assert_eq!(
            workspace.crates,
            ["brightfield_spec", "gallery_gate", "xtask"]
                .map(String::from)
                .into()
        );
        assert!(workspace_from_metadata(r#"{"packages":[]}"#).is_err());
        assert!(workspace_from_metadata("not json").is_err());
    }

    #[test]
    fn an_entry_is_filed_under_the_text_before_its_hash() {
        let hash = "0123456789abcdef";
        for (entry, name) in [
            (format!("brightfield-spec-{hash}"), Some("brightfield-spec")),
            (
                format!("libbrightfield_spec-{hash}.rlib"),
                Some("libbrightfield_spec"),
            ),
            (
                format!("gallery_gate-{hash}.0abc.4.rcgu.o"),
                Some("gallery_gate"),
            ),
            (format!("gallery_gate-{hash}.dSYM"), Some("gallery_gate")),
            ("brightfield-spec".to_owned(), None),
            (format!("short-{}", &hash[..15]), None),
            (format!("upper-{}", hash.to_uppercase()), None),
            (format!("longer-{hash}0"), None),
        ] {
            assert_eq!(unit_name(&entry), name, "{entry}");
        }
    }

    #[test]
    fn a_prune_removes_the_workspaces_artifacts_and_keeps_third_party_ones() {
        let target = scratch("target");
        let workspace = workspace_from_metadata(METADATA).unwrap();
        let (ours, theirs) = ("0123456789abcdef", "fedcba9876543210");
        let removed = [
            format!("debug/deps/libbrightfield_spec-{ours}.rlib"),
            format!("debug/deps/brightfield_spec-{ours}.d"),
            format!("debug/deps/gallery_gate-{ours}"),
            format!("debug/deps/gallery_gate-{ours}.x1.2.rcgu.o"),
            format!("debug/deps/gallery_gate-{ours}.dSYM/Contents/Info.plist"),
            format!("debug/.fingerprint/brightfield-spec-{ours}/lib-brightfield_spec"),
            format!("debug/build/brightfield-spec-{ours}/output"),
            "debug/incremental/brightfield_spec-1abc2def/s-x/dep-graph.bin".to_owned(),
            format!("release/deps/xtask-{ours}"),
            format!("aarch64-apple-darwin/debug/deps/xtask-{ours}"),
        ];
        let kept = [
            format!("debug/deps/libserde-{theirs}.rlib"),
            format!("debug/deps/libc-{theirs}.rlib"),
            format!("debug/.fingerprint/serde-{theirs}/lib-serde"),
            format!("release/.fingerprint/serde-{theirs}/lib-serde"),
            format!("aarch64-apple-darwin/debug/.fingerprint/serde-{theirs}/lib-serde"),
            format!("debug/build/duckdb-{theirs}/output"),
            "debug/brightfield-shell".to_owned(),
            "debug/xtask".to_owned(),
            "tmp/some-test/scratch".to_owned(),
            format!("doc/brightfield_spec-{ours}"),
        ];
        for path in removed.iter().chain(&kept) {
            touch(&target.join(path), 10);
        }

        let pruned = prune(&target, &workspace);

        for path in &removed {
            assert!(!target.join(path).exists(), "{path} should be removed");
        }
        for path in &kept {
            assert!(target.join(path).exists(), "{path} should be kept");
        }
        assert!(pruned.failed.is_empty(), "{:?}", pruned.failed);
        // The ten files, the .dSYM, fingerprint and build directories each
        // counted once with their contents, and the incremental entry.
        assert_eq!(pruned.entries, 10);
        assert_eq!(pruned.bytes, 100);
        assert_eq!(prune(&target, &workspace), Pruned::default());
        fs::remove_dir_all(&target).unwrap();
    }

    #[test]
    fn only_the_gates_own_stale_trees_are_removed() {
        let trees = scratch("trees");
        for name in ["brightfield-gate-466-16040", "nightly-33546dc.abc", "other"] {
            touch(&trees.join(name).join("Cargo.toml"), 1);
        }
        assert_eq!(remove_stale_gate_trees(&trees, &trees), 1);
        assert!(!trees.join("brightfield-gate-466-16040").exists());
        assert!(trees.join("nightly-33546dc.abc").exists());
        assert!(trees.join("other").exists());
        fs::remove_dir_all(&trees).unwrap();
    }
}
