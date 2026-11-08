//! Every way into the window, as one queue: what was asked to open, in the order it was asked.
//!
//! The picker, a path named on the command line, and later the Mac's own open events and the
//! `brightfield://open` link all push a request here, and the window takes them all at the start
//! of its next frame. One queue means one way of opening, so a CSV and a folder of tables cannot
//! drift into two behaviours. The queue is cheap to clone and safe to push from another thread,
//! because the operating system delivers its open events outside the frame, sometimes before the
//! first one.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

/// One thing asked to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenRequest {
    /// A file or folder on this Mac: data to make a Protocol of, or a Protocol to open.
    Path(PathBuf),
    /// A `brightfield://open` link.
    Link(String),
    /// A file or folder to add as a source to the Protocol open now.
    AddSource(PathBuf),
}

/// Where a path goes when it is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// A Protocol directory, opened where it is.
    Existing(PathBuf),
    /// Data that a new Protocol is made of, in the Protocols folder.
    New(PathBuf),
    /// A zip of a Protocol directory.
    Archive(PathBuf),
}

/// The manifest that makes a folder a Protocol.
const MANIFEST: &str = "arcform.yaml";

/// Where `path` goes: [`Route::Existing`] with the Protocol directory when `path` or its parent
/// holds `arcform.yaml`, [`Route::Archive`] for a `.zip` whose single top-level folder holds one,
/// and [`Route::New`] otherwise.
///
/// The parent counts so that a data file an earlier Brightfield saved a Protocol beside, and the
/// manifest itself, open that Protocol rather than making another.
#[must_use]
pub fn route(path: &Path) -> Route {
    if path.is_dir() && path.join(MANIFEST).is_file() {
        return Route::Existing(path.to_path_buf());
    }
    if let Some(parent) = path.parent().filter(|p| p.join(MANIFEST).is_file()) {
        return Route::Existing(parent.to_path_buf());
    }
    if is_protocol_archive(path) {
        return Route::Archive(path.to_path_buf());
    }
    Route::New(path.to_path_buf())
}

/// Whether `path` is a `.zip` whose entries all sit under one top-level folder holding
/// `arcform.yaml`. A zip that will not read is not one.
fn is_protocol_archive(path: &Path) -> bool {
    let zipped = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    if !zipped || !path.is_file() {
        return false;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(archive) = zip::ZipArchive::new(file) else {
        return false;
    };
    let mut top: Option<String> = None;
    let mut manifest = false;
    for name in archive.file_names() {
        let mut parts = name.split('/').filter(|p| !p.is_empty());
        let Some(first) = parts.next() else {
            continue;
        };
        match &top {
            Some(t) if t != first => return false,
            Some(_) => {}
            None => top = Some(first.to_string()),
        }
        let rest: Vec<&str> = parts.collect();
        if rest == [MANIFEST] {
            manifest = true;
        }
    }
    manifest
}

/// The queue every way in fills, shared by clone.
#[derive(Debug, Clone, Default)]
pub struct OpenQueue(Arc<Mutex<VecDeque<OpenRequest>>>);

impl OpenQueue {
    /// Ask for `request` to open on the window's next frame, after anything asked before it.
    pub fn push(&self, request: OpenRequest) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(request);
    }

    /// Take every request waiting, oldest first, leaving the queue empty.
    #[must_use]
    pub fn drain(&self) -> Vec<OpenRequest> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }
}
