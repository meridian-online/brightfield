//! Spec-file save (card 0017, aws_ac04) — framework-free.
//!
//! The editor's cmd-s handler is built from the pure pieces here:
//! [`decide_save`] (the conflict/truncation guard over the three texts in
//! play), [`save_spec_atomic`] (the temp+rename write), and
//! [`should_reseed`] (whether an external change replaces a pristine
//! buffer). The write is the ONLY bridge between the editor and the render
//! loop: the existing 300ms mtime watcher notices the rename exactly as it
//! notices any external editor's save, so the reload machinery changes zero
//! lines. Rename atomicity (same directory, same filesystem) means the
//! watcher can never read a half-written spec — the tabletop's
//! kill-condition-3 mitigation. No gpui import may enter this file
//! (semantic-layer rule).
//!
//! Metadata semantics of temp+rename: the destination is resolved through
//! symlinks first (so saving "through" a symlinked spec replaces the TARGET
//! and keeps the link), and the original file's permission bits are copied
//! onto the temp before the rename (so e.g. a 0600 spec stays 0600 instead
//! of picking up the process umask). One tradeoff is inherent to the
//! technique and accepted: rename replaces the directory entry, so any
//! OTHER hard links to the old inode diverge — they keep the pre-save
//! content. An in-place write would preserve hard links but reintroduce the
//! partial-write race the rename exists to prevent.

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
///
/// The destination is canonicalised first so a symlinked spec path saves
/// through to the link's target (the link survives; renaming over the link
/// itself would silently replace it with a regular file). When the path
/// does not resolve (e.g. the file does not exist yet), the raw path is
/// used as-is. The original file's permissions are copied onto the temp
/// before the rename, so the replacement keeps the file's mode bits.
pub fn save_spec_atomic(buffer: &str, path: &Path) -> io::Result<SaveOutcome> {
    let dest = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    if let Ok(current) = fs::read_to_string(&dest) {
        if current == buffer {
            return Ok(SaveOutcome::Unchanged);
        }
    }

    let dir = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "spec path has no file name"))?;
    // Same directory as the destination so the rename stays on one
    // filesystem (rename is only atomic within a filesystem).
    let tmp = dir.join(format!(".{file_name}.bf-save-{}", std::process::id()));

    fs::write(&tmp, buffer)?;
    // Preserve the destination's permission bits: the temp was created with
    // umask-default permissions, and the rename would otherwise clobber e.g.
    // an owner-only 0600 spec up to 0644.
    if let Ok(meta) = fs::metadata(&dest) {
        if let Err(e) = fs::set_permissions(&tmp, meta.permissions()) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }
    match fs::rename(&tmp, &dest) {
        Ok(()) => Ok(SaveOutcome::Written),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// What the editor's cmd-s should do — decided framework-free from the
/// three texts in play: the editor `buffer`, the file's current contents
/// (`file_now`, `None` when unreadable/missing), and `last_synced` (the
/// file text the buffer was seeded from or last successfully saved as;
/// `None` when the editor NEVER successfully synced with the file).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveDecision {
    /// The buffer supersedes the file: write it.
    Write,
    /// The buffer matches the file byte-for-byte: nothing to write.
    Unchanged,
    /// The file changed on disk since the editor last synced with it —
    /// writing now would silently discard the external edit. The shell
    /// warns; a SECOND consecutive save forces the write.
    ExternalConflict,
    /// The editor never successfully seeded from the file (the boot read
    /// failed): its buffer must not overwrite a spec it never held.
    RefuseUnseeded,
}

/// The two-writer guard for cmd-s: refuse an unseeded editor, flag a file
/// that changed under us, and otherwise write/no-op as before.
#[must_use]
pub fn decide_save(buffer: &str, file_now: Option<&str>, last_synced: Option<&str>) -> SaveDecision {
    let Some(last_synced) = last_synced else {
        return SaveDecision::RefuseUnseeded;
    };
    // Belt-and-braces truncation guard: an empty buffer must never clobber
    // a non-empty file the editor does not own. (The general conflict rule
    // below already catches this case; the explicit check keeps the
    // truncation guarantee standing even if that rule is ever reordered.)
    if buffer.is_empty() {
        if let Some(now) = file_now {
            if !now.is_empty() && now != last_synced {
                return SaveDecision::ExternalConflict;
            }
        }
    }
    match file_now {
        Some(now) if now == buffer => SaveDecision::Unchanged,
        Some(now) if now != last_synced => SaveDecision::ExternalConflict,
        // File matches what we last synced (or is missing/unreadable —
        // deleted under us; recreating it from the buffer is the save).
        _ => SaveDecision::Write,
    }
}

/// Whether an external file change should replace the editor buffer: only
/// a PRISTINE buffer follows the disk (buffer == the text it was seeded
/// from / last saved as — nothing of the author's is lost), and a
/// never-seeded editor adopts the contents only while still untouched.
/// `file_now == buffer` is always a no-op — that is our own save's mtime
/// echo (or an identical external write), and reseeding would pointlessly
/// reset the cursor and scroll position.
#[must_use]
pub fn should_reseed(buffer: &str, last_synced: Option<&str>, file_now: &str) -> bool {
    if file_now == buffer {
        return false;
    }
    match last_synced {
        Some(synced) => buffer == synced,
        None => buffer.is_empty(),
    }
}

/// Whether a command-log COMMIT may proceed (card 0023, clg-ac07 pristine-buffer
/// gate). A commit re-serialises the working Spec and `set_value`s it OVER the
/// editor buffer; [`decide_save`] does NOT guard this — it only flags an
/// EXTERNAL disk change (`file_now` vs `last_synced`), and for a DIRTY in-app
/// buffer with no external change it returns [`SaveDecision::Write`], so
/// `set_value` would silently CLOBBER the author's hand-typed edits before
/// `save` ever runs. So the commit checks the buffer is PRISTINE first: a
/// pristine buffer (`buffer == last_synced` — the `should_reseed` dirtiness
/// test) may commit; a DIRTY buffer (or a never-synced editor) REFUSES with a
/// reason (the author saves or discards the manual edit first). `decide_save`
/// stays the downstream external-file-conflict guard, NOT the dirty-buffer guard.
///
/// Its live caller (the deliberate commit action) is the deferred macOS-eyeball
/// surface (card 0023); the gate logic is unit-tested now (clg-ac07).
#[allow(dead_code)]
#[must_use]
pub fn commit_is_allowed(buffer: &str, last_synced: Option<&str>) -> bool {
    match last_synced {
        Some(synced) => buffer == synced,
        // Never synced (boot seed failed): refuse — we can't know what a
        // set_value would clobber, exactly as decide_save refuses an unseeded editor.
        None => false,
    }
}

/// The refusal reason a dirty-buffer commit surfaces (card 0023, clg-ac07).
/// Consumed by the deferred commit action (macOS-eyeball surface).
#[allow(dead_code)]
pub const DIRTY_BUFFER_COMMIT_REFUSAL: &str =
    "unsaved editor edits — save or discard them before committing command-log edits";

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

    /// aws_ac04 (two-writer guard): a file that changed on disk since the
    /// editor last synced flags an external conflict instead of silently
    /// overwriting; a file matching the buffer is Unchanged; a file
    /// matching last_synced (nothing external happened) writes; a file
    /// deleted under the editor rewrites from the buffer.
    #[test]
    fn aws_ac04_decide_save_flags_external_conflict() {
        // External edit landed ("c") since we seeded ("a"): conflict.
        assert_eq!(
            decide_save("b", Some("c"), Some("a")),
            SaveDecision::ExternalConflict
        );
        // File already holds the buffer's text: nothing to write.
        assert_eq!(decide_save("b", Some("b"), Some("a")), SaveDecision::Unchanged);
        // File is exactly what we last synced with: normal write.
        assert_eq!(decide_save("b", Some("a"), Some("a")), SaveDecision::Write);
        // File vanished under us: the buffer recreates it.
        assert_eq!(decide_save("b", None, Some("a")), SaveDecision::Write);
    }

    /// aws_ac04 (truncation guard): an empty buffer facing a non-empty
    /// file it does not own (last_synced does not match the file) must
    /// never truncate it — the empty-seed data-loss window.
    #[test]
    fn aws_ac04_decide_save_empty_buffer_refuses_truncation() {
        assert_eq!(
            decide_save("", Some("plot:\n  - mark: dot\n"), Some("something else")),
            SaveDecision::ExternalConflict,
            "empty buffer + non-empty foreign file → no truncation"
        );
        // An emptying the AUTHOR made (file still holds exactly what we
        // synced with) is a legitimate write.
        assert_eq!(
            decide_save("", Some("plot:\n"), Some("plot:\n")),
            SaveDecision::Write
        );
    }

    /// aws_ac04 (unseeded refusal): an editor whose boot seed failed
    /// (last_synced = None) refuses to save at all — whatever sits in the
    /// buffer, it never held the file's contents, so writing could only
    /// destroy them.
    #[test]
    fn aws_ac04_decide_save_unseeded_editor_refuses() {
        assert_eq!(
            decide_save("", Some("plot:\n"), None),
            SaveDecision::RefuseUnseeded,
            "unreadable seed + untouched buffer → refuse"
        );
        assert_eq!(
            decide_save("typed: something\n", Some("plot:\n"), None),
            SaveDecision::RefuseUnseeded,
            "typing into an unseeded editor still refuses"
        );
        assert_eq!(decide_save("x", None, None), SaveDecision::RefuseUnseeded);
    }

    /// aws_ac04 (reseed decision): only a pristine buffer follows external
    /// disk changes — a dirty buffer keeps the author's edits (the save
    /// path's conflict guard handles those); our own save's mtime echo
    /// (file == buffer) never reseeds; a never-seeded editor adopts the
    /// contents only while still empty.
    #[test]
    fn aws_ac04_should_reseed_only_pristine_buffers_follow_disk() {
        // Pristine (buffer == last synced) → follow the disk.
        assert!(should_reseed("a", Some("a"), "b"));
        // Dirty → keep the author's edits.
        assert!(!should_reseed("edited", Some("a"), "b"));
        // Our own save's echo (disk == buffer) → nothing to adopt.
        assert!(!should_reseed("b", Some("b"), "b"));
        // Never seeded, untouched → adopt (recovers an unreadable boot seed).
        assert!(should_reseed("", None, "plot:\n"));
        // Never seeded but typed into → refuse to clobber.
        assert!(!should_reseed("typed", None, "plot:\n"));
    }

    /// clg-ac07 (pristine-buffer commit gate): a commit proceeds only over a
    /// PRISTINE buffer (buffer == last_synced); a DIRTY buffer refuses (so
    /// set_value never clobbers the author's hand-typed edits), and a
    /// never-synced editor refuses too. Distinct from decide_save (which returns
    /// Write for that same dirty buffer — the exact hole this gate closes).
    #[test]
    fn clg_ac07_commit_gate_allows_pristine_refuses_dirty() {
        // Pristine: the buffer is exactly what we last synced with -> commit.
        assert!(commit_is_allowed("plot:\n  - mark: dot\n", Some("plot:\n  - mark: dot\n")));
        // Dirty: hand-typed edits sit in the buffer -> refuse the commit.
        assert!(!commit_is_allowed("plot:\n  - mark: bar\n", Some("plot:\n  - mark: dot\n")));
        // Never synced (boot seed failed) -> refuse.
        assert!(!commit_is_allowed("anything", None));
        // The hole this closes: decide_save would WRITE that same dirty buffer.
        assert_eq!(
            decide_save("plot:\n  - mark: bar\n", Some("plot:\n  - mark: dot\n"), Some("plot:\n  - mark: dot\n")),
            SaveDecision::Write,
            "decide_save is NOT the dirty-buffer guard — the commit gate is"
        );
    }

    /// aws_ac04 (permission preservation): an owner-only 0600 spec keeps
    /// its mode bits across the temp+rename — the temp inherits the
    /// destination's permissions before the rename.
    #[cfg(unix)]
    #[test]
    fn aws_ac04_save_preserves_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let path = temp_spec("perms.yaml");
        fs::write(&path, "plot:\n  - mark: dot\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        let outcome = save_spec_atomic("plot:\n  - mark: line\n", &path).expect("save succeeds");
        assert_eq!(outcome, SaveOutcome::Written);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "0600 must survive the rename, got {mode:o}");

        let _ = fs::remove_file(&path);
    }

    /// aws_ac04 (symlinked spec): saving through a symlink updates the
    /// TARGET file and leaves the link in place — the destination is
    /// canonicalised before the temp+rename, so the rename replaces the
    /// target, never the link itself.
    #[cfg(unix)]
    #[test]
    fn aws_ac04_save_through_symlink_updates_target_keeps_link() {
        let target = temp_spec("link-target.yaml");
        fs::write(&target, "plot:\n  - mark: dot\n").unwrap();
        let link = temp_spec("link.yaml");
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let new_buffer = "plot:\n  - mark: line\n";
        let outcome = save_spec_atomic(new_buffer, &link).expect("save succeeds");
        assert_eq!(outcome, SaveOutcome::Written);

        assert!(
            fs::symlink_metadata(&link).unwrap().file_type().is_symlink(),
            "the link survives the save"
        );
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            new_buffer,
            "the target received the write"
        );
        assert_eq!(fs::read_to_string(&link).unwrap(), new_buffer);

        let _ = fs::remove_file(&link);
        let _ = fs::remove_file(&target);
    }
}
