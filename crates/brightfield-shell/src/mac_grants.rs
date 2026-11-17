//! Security-scoped bookmarks: the Foundation calls behind [`crate::grants`].
//!
//! Under App Sandbox, a folder the analyst chose in a panel or opened from Finder is reachable
//! until the app quits. A security-scoped bookmark is how the app keeps it reachable after a
//! relaunch: [`make_bookmark`] records one while the folder is reachable, and
//! [`resolve_and_start`] turns it back into the folder's current path and starts access to it.
//! Outside the sandbox the same calls work and change nothing about what the app may read.

use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_foundation::{
    NSData, NSString, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions, NSURL,
};

/// A security-scoped bookmark of `path`, which this process must be able to reach now.
///
/// # Errors
///
/// Foundation's description of why the bookmark could not be made.
pub fn make_bookmark(path: &Path) -> Result<Vec<u8>, String> {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    url.bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
        NSURLBookmarkCreationOptions::WithSecurityScope,
        None,
        None,
    )
    .map(|data| data.to_vec())
    .map_err(|e| e.localizedDescription().to_string())
}

/// Resolve the bookmark `data` to where its folder is now, whether the bookmark is stale and
/// should be made again, and start access to the folder for the rest of the process.
///
/// Resolution neither shows a dialog nor mounts a volume, so a bookmark on a drive that is not
/// plugged in fails at once rather than asking the analyst for it.
///
/// Access is started on the URL resolution returns, which is the one that carries the grant, and
/// is never stopped: the URL is kept for the life of the process, so the grant stays open for the
/// runs and join proposals that use it.
///
/// # Errors
///
/// Foundation's description of why the bookmark did not resolve, or that it resolved to a path
/// with no file there.
pub fn resolve_and_start(data: &[u8]) -> Result<(PathBuf, bool), String> {
    let bytes = NSData::with_bytes(data);
    let mut stale = Bool::NO;
    // SAFETY: `stale` is a valid pointer to a `Bool` that outlives the call.
    let url = unsafe {
        NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
            &bytes,
            NSURLBookmarkResolutionOptions::WithSecurityScope
                | NSURLBookmarkResolutionOptions::WithoutUI
                | NSURLBookmarkResolutionOptions::WithoutMounting,
            None,
            &mut stale,
        )
    }
    .map_err(|e| e.localizedDescription().to_string())?;
    let path = url
        .path()
        .map(|p| PathBuf::from(p.to_string()))
        .ok_or_else(|| "the bookmark resolved to a URL with no path".to_string())?;
    if !path.exists() {
        return Err(format!("{}: no such file or folder", path.display()));
    }
    // SAFETY: a plain message to a live URL; access is balanced by never stopping it, below.
    let _started = unsafe { url.startAccessingSecurityScopedResource() };
    // Kept for the life of the process, so the access it started is never ended by its release.
    std::mem::forget::<Retained<NSURL>>(url);
    Ok((path, stale.as_bool()))
}
