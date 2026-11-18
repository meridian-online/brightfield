//! Whether the managed DuckDB build on disk is the one the current inputs make.
//!
//! A cold build is 30 to 90 minutes, so the nightly keeps its output between
//! runs and rebuilds only when an input moved. The inputs are the files
//! `managed-duckdb.yml` keys its cache on. The stamp is a hash of them,
//! written beside the build once it succeeds and removed before a rebuild
//! starts, so a build that died half way is never taken for a current one.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// The stamp's file name inside the build directory.
const STAMP: &str = "inputs.hash";

/// A single file and a directory whose files, at any depth, decide what is
/// built. These are the paths `managed-duckdb.yml` hashes for its cache key.
const INPUT_FILES: &[&str] = &["scripts/build-duckdb.sh", "packaging/finetype-pin.env"];
const INPUT_DIRS: &[&str] = &["scripts/duckdb-managed"];

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .path();
        if path.is_dir() {
            collect(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

/// A hash of the build's input files under `root`: their paths, sorted, and
/// their bytes. It is stable between runs of one toolchain, which is all a
/// stamp read back by the same binary needs.
pub fn inputs_hash(root: &Path) -> Result<String, String> {
    let mut files: Vec<PathBuf> = INPUT_FILES.iter().map(|f| root.join(f)).collect();
    for dir in INPUT_DIRS {
        collect(&root.join(dir), &mut files)?;
    }
    files.sort();
    let mut hasher = DefaultHasher::new();
    for path in &files {
        let bytes = fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let relative = path.strip_prefix(root).unwrap_or(path);
        relative.to_string_lossy().hash(&mut hasher);
        bytes.hash(&mut hasher);
    }
    Ok(format!("{:016x}", hasher.finish()))
}

/// Whether `dir` holds a stamp equal to `hash`.
pub fn build_is_current(dir: &Path, hash: &str) -> bool {
    fs::read_to_string(dir.join(STAMP)).is_ok_and(|stamp| stamp.trim() == hash)
}

/// Write the stamp for a build that has just succeeded.
pub fn record_build(dir: &Path, hash: &str) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = dir.join(STAMP);
    fs::write(&path, format!("{hash}\n"))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Remove the stamp before a rebuild starts. No stamp is not an error.
pub fn forget_build(dir: &Path) -> Result<(), String> {
    let path = dir.join(STAMP);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot remove {}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("xtask-stamp-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("scripts/duckdb-managed")).unwrap();
        fs::create_dir_all(root.join("packaging")).unwrap();
        fs::write(root.join("scripts/build-duckdb.sh"), "build").unwrap();
        fs::write(root.join("scripts/duckdb-managed/x"), "one").unwrap();
        fs::write(root.join("packaging/finetype-pin.env"), "pin").unwrap();
        fs::write(root.join("README"), "unrelated").unwrap();
        root
    }

    #[test]
    fn the_hash_changes_when_an_input_changes_and_not_otherwise() {
        let root = tree("hash");
        let first = inputs_hash(&root).unwrap();
        assert_eq!(first, inputs_hash(&root).unwrap());
        fs::write(root.join("scripts/duckdb-managed/x"), "two").unwrap();
        let second = inputs_hash(&root).unwrap();
        assert_ne!(first, second);
        fs::write(root.join("README"), "edited").unwrap();
        assert_eq!(second, inputs_hash(&root).unwrap());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_recorded_build_is_current_until_the_hash_moves() {
        let dir = tree("current");
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        assert!(!build_is_current(&out, "h"));
        record_build(&out, "h").unwrap();
        assert!(build_is_current(&out, "h"));
        assert!(!build_is_current(&out, "other"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn starting_a_rebuild_forgets_the_old_stamp() {
        let dir = tree("forget");
        let out = dir.join("out");
        record_build(&out, "h").unwrap();
        forget_build(&out).unwrap();
        assert!(!build_is_current(&out, "h"));
        forget_build(&out).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}
