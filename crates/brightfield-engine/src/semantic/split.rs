//! A FineType bundle shipped as two files, joined into a cache copy to load.
//!
//! A DuckDB extension is a shared library with DuckDB's metadata trailer
//! appended after the end of the Mach-O. `codesign` cannot sign that file,
//! because the trailer sits outside anything the signature can cover, so the
//! signed app carries the library cut at its end and the trailer beside it.
//! DuckDB needs the two as one file again, so they are joined here into a copy
//! outside the app bundle, which a signed app may not write into.
//!
//! The copy's directory is named by the hash of the joined bytes. Two bundles
//! or two Brightfield versions therefore never share a copy by accident, and a
//! copy never needs invalidating: a different bundle is a different directory.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use super::{EXTENSION_FILE, SPLIT_LIBRARY_FILE, SPLIT_TRAILER_FILE};

/// The folder every cache copy lives under, from the process's `HOME`.
///
/// # Errors
///
/// When `HOME` is unset, empty or not absolute.
pub fn cache_root() -> Result<PathBuf, String> {
    cache_root_from(std::env::var_os("HOME").as_deref())
}

/// [`cache_root`] with `HOME` passed in, so a test need not change the
/// process's environment to reach each case.
///
/// A relative `HOME` is refused rather than resolved, because it would put the
/// copy under whatever directory the app happened to be started from.
pub(crate) fn cache_root_from(home: Option<&OsStr>) -> Result<PathBuf, String> {
    let home = home.filter(|h| !h.is_empty()).ok_or_else(|| {
        "HOME is not set, so there is no cache folder for the extension".to_string()
    })?;
    let home = Path::new(home);
    if !home.is_absolute() {
        return Err(format!(
            "HOME is {}, which is not an absolute path, so there is no cache folder for the \
             extension",
            home.display()
        ));
    }
    Ok(home.join("Library/Caches/online.meridian.brightfield/finetype"))
}

/// Join the split bundle in `dir` into a copy under `cache_root`, and return
/// the copy's path.
///
/// An existing copy is reused only when its bytes still hash to its
/// directory's name, so a copy changed since it was written is written again
/// rather than loaded. A new copy is written to a temporary file in the same
/// directory and renamed into place, so two processes starting at once each
/// end with a whole, correct file and neither ever sees half of one.
///
/// # Errors
///
/// A part that cannot be read, or a cache folder that cannot be written. The
/// latter names the folder.
pub fn materialise_split(dir: &Path, cache_root: &Path) -> Result<PathBuf, String> {
    let read = |name: &str| {
        std::fs::read(dir.join(name)).map_err(|e| format!("{name}: cannot be read: {e}"))
    };
    let mut joined = read(SPLIT_LIBRARY_FILE)?;
    joined.extend_from_slice(&read(SPLIT_TRAILER_FILE)?);

    let key = sha256_hex(&joined);
    let folder = cache_root.join(&key);
    let copy = folder.join(EXTENSION_FILE);
    if std::fs::read(&copy).is_ok_and(|existing| sha256_hex(&existing) == key) {
        return Ok(copy);
    }

    let cannot = |e: std::io::Error| {
        format!(
            "cannot write the extension's cache copy in {}: {e}",
            folder.display()
        )
    };
    std::fs::create_dir_all(&folder).map_err(cannot)?;
    // Unique per process and per call, so two writers never share a temporary
    // file even when they are threads of one process.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temp = folder.join(format!(
        ".{EXTENSION_FILE}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let written = std::fs::write(&temp, &joined).and_then(|()| std::fs::rename(&temp, &copy));
    if let Err(e) = written {
        std::fs::remove_file(&temp).ok();
        return Err(cannot(e));
    }
    Ok(copy)
}

/// The lowercase hex SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::{EXTENSION_FILE, SPLIT_LIBRARY_FILE, SPLIT_TRAILER_FILE};
    use std::ffi::OsStr;

    const BODY: &[u8] = b"\xcf\xfa\xed\xfe a Mach-O cut at the end of its __LINKEDIT";
    const TRAILER: &[u8] = b"five hundred and twelve bytes of DuckDB metadata, near enough";

    fn split_bundle() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join(SPLIT_LIBRARY_FILE), BODY).unwrap();
        std::fs::write(dir.path().join(SPLIT_TRAILER_FILE), TRAILER).unwrap();
        dir
    }

    fn joined() -> Vec<u8> {
        [BODY, TRAILER].concat()
    }

    #[test]
    fn split_parts_join_back_byte_for_byte() {
        let bundle = split_bundle();
        let cache = tempfile::tempdir().unwrap();
        let copy = materialise_split(bundle.path(), cache.path()).expect("materialised");

        assert_eq!(std::fs::read(&copy).unwrap(), joined());
        // DuckDB derives the extension's entry point from its file name, so the
        // copy must carry the joined bundle's name exactly.
        assert_eq!(copy.file_name(), Some(OsStr::new(EXTENSION_FILE)));
        let key = copy.parent().and_then(Path::file_name).unwrap();
        assert_eq!(key, OsStr::new(&sha256_hex(&joined())));
        assert!(key
            .to_str()
            .unwrap()
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert_eq!(copy.parent().and_then(Path::parent), Some(cache.path()));
    }

    #[test]
    fn a_second_materialise_reuses_the_copy() {
        let bundle = split_bundle();
        let cache = tempfile::tempdir().unwrap();
        let first = materialise_split(bundle.path(), cache.path()).unwrap();
        let written = std::fs::metadata(&first).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));

        let second = materialise_split(bundle.path(), cache.path()).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            std::fs::metadata(&second).unwrap().modified().unwrap(),
            written,
            "a copy whose bytes match was written again"
        );
    }

    #[test]
    fn a_cache_copy_whose_bytes_changed_is_rewritten_before_load() {
        let bundle = split_bundle();
        let cache = tempfile::tempdir().unwrap();
        let first = materialise_split(bundle.path(), cache.path()).unwrap();
        std::fs::write(&first, b"somebody else's library").unwrap();

        let second = materialise_split(bundle.path(), cache.path()).unwrap();
        assert_eq!(first, second);
        assert_eq!(std::fs::read(&second).unwrap(), joined());
    }

    #[test]
    fn concurrent_materialise_leaves_one_correct_copy() {
        let bundle = split_bundle();
        let cache = tempfile::tempdir().unwrap();
        let (b, c) = (bundle.path(), cache.path());
        let (one, two) = std::thread::scope(|s| {
            let one = s.spawn(|| materialise_split(b, c));
            let two = s.spawn(|| materialise_split(b, c));
            (one.join().unwrap(), two.join().unwrap())
        });
        let one = one.expect("first thread");
        let two = two.expect("second thread");
        assert_eq!(one, two);
        assert_eq!(std::fs::read(&one).unwrap(), joined());
        // No temporary file is left beside the copy.
        let entries = std::fs::read_dir(one.parent().unwrap()).unwrap().count();
        assert_eq!(entries, 1);
    }

    #[cfg(unix)]
    #[test]
    fn an_unwritable_cache_names_the_folder() {
        use std::os::unix::fs::PermissionsExt;
        let bundle = split_bundle();
        let cache = tempfile::tempdir().unwrap();
        std::fs::set_permissions(cache.path(), std::fs::Permissions::from_mode(0o500)).unwrap();

        let result = materialise_split(bundle.path(), cache.path());
        // Restored before asserting, so the temp dir can still be removed.
        std::fs::set_permissions(cache.path(), std::fs::Permissions::from_mode(0o700)).unwrap();

        let err = result.expect_err("a read-only cache folder cannot take the copy");
        assert!(
            err.contains(&cache.path().display().to_string()),
            "the error does not name the cache folder: {err}"
        );
    }

    #[test]
    fn cache_root_refuses_a_missing_or_relative_home() {
        assert!(cache_root_from(None).is_err());
        assert!(cache_root_from(Some(OsStr::new(""))).is_err());
        assert!(cache_root_from(Some(OsStr::new("rel/home"))).is_err());
        assert_eq!(
            cache_root_from(Some(OsStr::new("/h"))),
            Ok(PathBuf::from(
                "/h/Library/Caches/online.meridian.brightfield/finetype"
            ))
        );
    }
}
