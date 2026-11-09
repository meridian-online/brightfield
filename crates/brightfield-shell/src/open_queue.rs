//! The window's ways in, as one queue: what was asked to open, in the order it was asked.
//!
//! The front door's picker pushes a request here, and so does a folder, workbook, database or
//! own-files Protocol named on the command line, through [`crate::window::Boot::open_later`].
//! Later the Mac's own open events and the `brightfield://open` link push here too. The window
//! takes the requests at the start of its next frame, so a CSV and a folder of tables chosen at
//! the door open by one route. A lone flat file named on the command line does not come here: it
//! opens at once through [`crate::window::Boot::data_file`], which is the route the capture
//! harness and its baselines are drawn through.
//!
//! The queue is cheap to clone and safe to push from another thread, because the operating
//! system delivers its open events outside the frame, sometimes before the first one. Once the
//! window has drawn a frame, a push asks for another, so a request does not wait for input.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// One thing asked to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenRequest {
    /// A file or folder on this Mac: data to make a Protocol of, or a Protocol to open.
    Path(PathBuf),
    /// Several files or folders handed over by one gesture, such as files chosen together in
    /// Finder or dropped together on the door: one Protocol is made of them all.
    Paths(Vec<PathBuf>),
    /// A `brightfield://open` link.
    Link(String),
    /// A file or folder to add as a source to the Protocol open now.
    AddSource(PathBuf),
    /// Several files or folders handed over by one gesture, added together as sources to the
    /// Protocol open now.
    AddSources(Vec<PathBuf>),
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

impl OpenRequest {
    /// The request for `paths`, handed over by one gesture: [`Self::Path`] for one, [`Self::Paths`]
    /// for several, and `None` for an empty list.
    #[must_use]
    pub fn of_paths(mut paths: Vec<PathBuf>) -> Option<Self> {
        match paths.len() {
            0 => None,
            1 => paths.pop().map(Self::Path),
            _ => Some(Self::Paths(paths)),
        }
    }
}

/// The queue the window's ways in fill, shared by clone.
///
/// It carries the Mac's quit as well as its opens: the app menu's Quit and the Dock's arrive
/// outside the frame from the same Apple Event handler the opens do, and the window answers a
/// quit with the close question when a draft is pending.
#[derive(Clone)]
pub struct OpenQueue {
    requests: Arc<Mutex<VecDeque<OpenRequest>>>,
    /// Whether quitting was asked for since the window last looked.
    quit: Arc<AtomicBool>,
    /// Whether Preferences was asked for from the app menu since the window last looked.
    preferences: Arc<AtomicBool>,
    /// Whether the app has finished launching, by which the files the launch was asked to open
    /// are on the queue: see [`Self::awaiting_launch`].
    launched: Arc<AtomicBool>,
    /// The window's context, once it has drawn, so a push can ask for a frame.
    wake: Arc<OnceLock<egui::Context>>,
}

/// A queue whose launch has finished, as one with no system launch to wait for is: a build
/// without the Mac's open events, and a headless window.
impl Default for OpenQueue {
    fn default() -> Self {
        Self {
            requests: Arc::default(),
            quit: Arc::default(),
            preferences: Arc::default(),
            launched: Arc::new(AtomicBool::new(true)),
            wake: Arc::default(),
        }
    }
}

impl OpenQueue {
    /// A queue whose launch has not finished: the files a launch by double-click delivers are
    /// still to come, and [`Self::mark_launched`] says when they have. The Mac's open events are
    /// listened for on this one ([`crate::mac_open::install`]), whose observer of the app having
    /// finished launching marks it.
    #[must_use]
    pub fn awaiting_launch() -> Self {
        let queue = Self::default();
        queue.launched.store(false, Ordering::SeqCst);
        queue
    }

    /// Say the app has finished launching, and ask for a frame, so a window waiting on it decides.
    pub fn mark_launched(&self) {
        self.launched.store(true, Ordering::SeqCst);
        if let Some(ctx) = self.wake.get() {
            ctx.request_repaint();
        }
    }

    /// Whether the app has finished launching ([`Self::awaiting_launch`]).
    #[must_use]
    pub fn has_launched(&self) -> bool {
        self.launched.load(Ordering::SeqCst)
    }

    /// Ask for `request` to open on the window's next frame, after anything asked before it.
    pub fn push(&self, request: OpenRequest) {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(request);
        if let Some(ctx) = self.wake.get() {
            ctx.request_repaint();
        }
    }

    /// Take the oldest request waiting, if there is one.
    #[must_use]
    pub fn pop(&self) -> Option<OpenRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
    }

    /// Whether no request is waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }

    /// Take every request waiting, oldest first, leaving the queue empty.
    #[must_use]
    pub fn drain(&self) -> Vec<OpenRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }

    /// Ask the window to quit on its next frame, as the Mac's Quit does.
    pub fn ask_quit(&self) {
        self.quit.store(true, Ordering::SeqCst);
        if let Some(ctx) = self.wake.get() {
            ctx.request_repaint();
        }
    }

    /// Whether quitting was asked for since the last take, clearing the ask.
    #[must_use]
    pub fn take_quit(&self) -> bool {
        self.quit.swap(false, Ordering::SeqCst)
    }

    /// Ask the window to open Preferences on its next frame, as the app menu's Preferences… does.
    pub fn ask_preferences(&self) {
        self.preferences.store(true, Ordering::SeqCst);
        if let Some(ctx) = self.wake.get() {
            ctx.request_repaint();
        }
    }

    /// Whether Preferences was asked for since the last take, clearing the ask.
    #[must_use]
    pub fn take_preferences(&self) -> bool {
        self.preferences.swap(false, Ordering::SeqCst)
    }

    /// Wake `ctx` on each later push. The first context given is kept.
    pub fn wake_with(&self, ctx: &egui::Context) {
        let _ = self.wake.set(ctx.clone());
    }
}
