//! Spec-file save (card 0017, aws_ac04) — framework-free.
//!
//! The editor's cmd-s handler is this ONE pure function: (buffer, path) →
//! atomic write via temp+rename. The write is the ONLY bridge between the
//! editor and the render loop: the existing 300ms mtime watcher notices the
//! rename exactly as it notices any external editor's save, so the reload
//! machinery changes zero lines. Rename atomicity (same directory, same
//! filesystem) means the watcher can never read a half-written spec — the
//! tabletop's kill-condition-3 mitigation. No gpui import may enter this
//! file (semantic-layer rule).

use std::fs;
use std::io;
use std::path::Path;

/// What a save call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    /// The buffer differed: the file was atomically replaced.
    Written,
    /// The buffer matched the file byte-for-byte: nothing touched — an
    /// unchanged save must not bump the mtime and trigger a pointless
    /// watcher reload.
    Unchanged,
}

/// Atomically write `buffer` to `path`: write a sibling temp file, then
/// rename over the destination. On a rename failure the temp file is
/// cleaned up and the destination is left as it was.
pub fn save_spec_atomic(buffer: &str, path: &Path) -> io::Result<SaveOutcome> {
    if let Ok(current) = fs::read_to_string(path) {
        if current == buffer {
            return Ok(SaveOutcome::Unchanged);
        }
    }

    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "spec path has no file name"))?;
    // Same directory as the destination so the rename stays on one
    // filesystem (rename is only atomic within a filesystem).
    let tmp = dir.join(format!(".{file_name}.bf-save-{}", std::process::id()));

    fs::write(&tmp, buffer)?;
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(SaveOutcome::Written),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_spec(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bf-aws-ac04-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    /// aws_ac04 (atomic write): the buffer replaces the file's contents in
    /// full, and no temp file survives the save — the temp+rename pair
    /// leaves exactly one artefact, the destination.
    #[test]
    fn aws_ac04_save_replaces_file_atomically_via_temp_rename() {
        let path = temp_spec("replace.yaml");
        fs::write(&path, "plot:\n  - mark: dot\n").unwrap();

        let new_buffer = "plot:\n  - mark: line\n    stroke: steelblue\n";
        let outcome = save_spec_atomic(new_buffer, &path).expect("save succeeds");
        assert_eq!(outcome, SaveOutcome::Written);
        assert_eq!(fs::read_to_string(&path).unwrap(), new_buffer, "full buffer lands");

        // No `.bf-save` temp remnant beside the destination.
        let dir = path.parent().unwrap();
        let leftovers: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".bf-save-"))
            .collect();
        assert!(leftovers.is_empty(), "temp file renamed away, not left behind");

        let _ = fs::remove_file(&path);
    }

    /// aws_ac04 (creation): saving to a not-yet-existing path writes it
    /// (the read-compare simply finds nothing to compare against).
    #[test]
    fn aws_ac04_save_creates_a_missing_file() {
        let path = temp_spec("fresh.yaml");
        let _ = fs::remove_file(&path);
        let outcome = save_spec_atomic("data: {}\n", &path).expect("save succeeds");
        assert_eq!(outcome, SaveOutcome::Written);
        assert_eq!(fs::read_to_string(&path).unwrap(), "data: {}\n");
        let _ = fs::remove_file(&path);
    }

    /// aws_ac04 (unchanged no-op): an identical buffer touches nothing —
    /// same mtime, `Unchanged` outcome — so cmd-s on a clean buffer never
    /// triggers a watcher reload.
    #[test]
    fn aws_ac04_unchanged_buffer_is_a_no_op() {
        let path = temp_spec("unchanged.yaml");
        let buffer = "plot:\n  - mark: dot\n";
        fs::write(&path, buffer).unwrap();
        let before = fs::metadata(&path).unwrap().modified().unwrap();

        let outcome = save_spec_atomic(buffer, &path).expect("save succeeds");
        assert_eq!(outcome, SaveOutcome::Unchanged);
        let after = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(before, after, "an unchanged save must not bump the mtime");

        let _ = fs::remove_file(&path);
    }
}
