//! **The sources browser's state and keys**, with no drawing.
//!
//! The sources browser is the sheet an analyst adds files, folders and databases to a Protocol
//! from. It follows yazi: three columns, the places, the current folder's list, and a preview of
//! what is under the cursor. A file previews its first rows, and a folder previews as the canvas
//! of the joins dovetail proposes between its tables, where the analyst accepts the joins the
//! folder is added with. This module holds what the browser shows and answers its keys; drawing
//! it, and reading a file's rows or a folder's joins off the frame thread, are the view's work.
//!
//! The keys are the registry's, in its Browser context. [`Browser::press`] resolves a keystroke
//! there and [`Browser::key`] answers the verb it names, so a key moved in the registry moves
//! here. What a key asks of the window comes back as an [`Effect`]: sources to add to the
//! Protocol, or the browser closed. The disk is read through [`Listing`], so the tests drive the
//! browser over a disk held in memory and write nothing.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use brightfield_keys::{keymap_bindings, registry, BindingContext, BoundKey};

use crate::own_files::AcceptedJoin;

/// The registry's verb that moves the focused column's cursor down a row, or to the next join on
/// a folder's canvas.
pub const MOVE_DOWN: &str = "move-browser-cursor-down";
/// The registry's verb that moves the focused column's cursor up a row, or to the join before.
pub const MOVE_UP: &str = "move-browser-cursor-up";
/// The registry's verb that goes up a folder, clears a glob, or leaves the canvas for the list.
pub const GO_UP: &str = "go-up-a-folder";
/// The registry's verb for `l` and `Enter`: into a folder, or add the file.
pub const GO_IN_OR_ADD: &str = "go-in-or-add";
/// The registry's verb that adds what is highlighted without going into it.
pub const ADD: &str = "add-sources";
/// The registry's verb that marks a file, or accepts or drops a join on a folder's canvas.
pub const MARK_OR_ACCEPT: &str = "mark-or-accept";
/// The registry's verb that moves between the places, the list and a folder's canvas.
pub const NEXT_PANE: &str = "next-browser-pane";
/// The registry's verb that opens the glob for typing.
pub const TYPE_GLOB: &str = "type-glob";
/// The registry's verb that fills the browser with the preview, or gives the columns back.
pub const FILL: &str = "fill-browser-with-preview";
/// The registry's verb that previews a folder as its canvas or as its list of files.
pub const SWITCH_PREVIEW: &str = "switch-folder-preview";
/// The registry's verb that closes the browser.
pub const CLOSE: &str = "close-sources-browser";

/// The verbs [`Browser::key`] answers, in the registry's order.
pub const VERBS: [&str; 11] = [
    MOVE_DOWN,
    MOVE_UP,
    GO_UP,
    GO_IN_OR_ADD,
    ADD,
    MARK_OR_ACCEPT,
    NEXT_PANE,
    TYPE_GLOB,
    FILL,
    SWITCH_PREVIEW,
    CLOSE,
];

/// The most folders a glob reads before it stops looking, so a `**` typed in the home folder
/// lists what it has found so far rather than reading the whole disk on the frame thread.
const GLOB_FOLDER_LIMIT: usize = 2_000;

/// Which of the browser's three columns holds the keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    /// The places: Home, the Protocols folder and the other folders an analyst starts from.
    Places,
    /// The current folder's list, or the files a glob matches.
    List,
    /// The preview of what is under the list's cursor. For a folder it is the canvas of the joins
    /// dovetail proposes, where `j` and `k` move between joins and `Space` accepts one.
    Canvas,
}

/// How a folder under the cursor is previewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewMode {
    /// The canvas of its tables and the joins proposed between them.
    Canvas,
    /// The list of the files it holds.
    Files,
}

/// What a row of a folder's list is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A folder, which `l` and `Enter` go into and `a` adds whole.
    Folder,
    /// A file Brightfield reads as a source: a CSV, TSV or Parquet file, a workbook, or a database.
    Data,
    /// Any other file, listed so the folder reads as the Finder shows it, and not added.
    Other,
}

impl Kind {
    /// The kind of the file at `path`, by its extension: one of the extensions the front door
    /// opens ([`crate::data_file::OPENABLE_EXTENSIONS`]) is data.
    #[must_use]
    pub fn of_file(path: &Path) -> Self {
        let data = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .is_some_and(|e| crate::data_file::OPENABLE_EXTENSIONS.contains(&e.as_str()));
        if data {
            Self::Data
        } else {
            Self::Other
        }
    }
}

/// A row of a folder's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Where it is, absolute.
    pub path: PathBuf,
    /// What it is.
    pub kind: Kind,
}

impl Entry {
    /// The name the list shows: the last part of its path.
    #[must_use]
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    }
}

/// What the browser reads the disk through: the entries of one folder, folders first, then by
/// name. The window reads the Mac's disk ([`Disk`]); a test reads one held in memory.
pub trait Listing {
    /// The entries of `dir`, folders first and then by name, leaving hidden files out. A folder
    /// that cannot be read lists as empty.
    fn list(&self, dir: &Path) -> Vec<Entry>;
}

/// The Mac's own disk, read as the browser shows it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Disk;

impl Listing for Disk {
    fn list(&self, dir: &Path) -> Vec<Entry> {
        let Ok(read) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut entries: Vec<Entry> = read
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .map(|e| {
                let path = e.path();
                // `metadata` follows a link, so a link to a folder lists as the folder it reaches.
                let kind = if std::fs::metadata(&path).is_ok_and(|m| m.is_dir()) {
                    Kind::Folder
                } else {
                    Kind::of_file(&path)
                };
                Entry { path, kind }
            })
            .collect();
        sort_entries(&mut entries);
        entries
    }
}

/// `entries` in the order a list shows them: folders first, then by name with case ignored, as
/// the Finder sorts them, and by the exact name where two differ in case alone.
pub fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        let folder = |e: &Entry| e.kind != Kind::Folder;
        folder(a)
            .cmp(&folder(b))
            .then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
            .then_with(|| a.name().cmp(&b.name()))
    });
}

/// A folder in the places column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// The name the column shows.
    pub label: String,
    /// The folder it goes to.
    pub path: PathBuf,
}

impl Place {
    /// The place `label` going to `path`.
    #[must_use]
    pub fn new(label: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            label: label.into(),
            path: path.into(),
        }
    }
}

/// The places the browser offers on this Mac: the home folder, its Desktop, Documents and
/// Downloads, the Protocols folder, and each volume mounted under `/Volumes` (a removable disk
/// among them), leaving out a folder that is not there.
#[must_use]
pub fn places() -> Vec<Place> {
    let home = std::env::home_dir().unwrap_or_default();
    let mut places = vec![Place::new("Home", home.clone())];
    for name in ["Desktop", "Documents", "Downloads"] {
        places.push(Place::new(name, home.join(name)));
    }
    places.push(Place::new("Protocols", crate::protocols_dir::root()));
    if let Ok(volumes) = std::fs::read_dir("/Volumes") {
        let mut found: Vec<PathBuf> = volumes
            .flatten()
            // The start-up disk is a link to `/` there, and Home already reaches it.
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect();
        found.sort();
        for path in found {
            let label = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            places.push(Place::new(label, path));
        }
    }
    let mut seen = BTreeSet::new();
    places.retain(|p| p.path.is_dir() && seen.insert(p.path.clone()));
    places
}

/// A join dovetail proposes between two tables of a folder under the cursor, each side a table
/// and one of its columns, as [`AcceptedJoin`] records one the analyst accepted.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProposedJoin {
    /// The table and column the join reads from, the side with many rows to each of the other's.
    pub left: (String, String),
    /// The table and column it finds a row of for each.
    pub right: (String, String),
}

impl ProposedJoin {
    /// The join `left` to `right`, each a table and a column.
    #[must_use]
    pub fn new(left: (&str, &str), right: (&str, &str)) -> Self {
        Self {
            left: (left.0.to_string(), left.1.to_string()),
            right: (right.0.to_string(), right.1.to_string()),
        }
    }

    /// The join as one the analyst accepted.
    #[must_use]
    pub fn accepted(&self) -> AcceptedJoin {
        AcceptedJoin {
            left: self.left.clone(),
            right: self.right.clone(),
        }
    }
}

/// What a key asks of the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Nothing beyond what the browser shows.
    None,
    /// Add these to the Protocol as sources, with the joins the analyst accepted written into its
    /// model, kept at once; the browser closes.
    Add(Vec<PathBuf>, Vec<AcceptedJoin>),
    /// Close the browser, adding nothing more.
    Close,
}

/// The sources browser: where it is, what is under its cursor and marked, and which column holds
/// the keys.
///
/// The rows the list shows are read from the disk when the list changes folder or glob, and again
/// on [`Browser::refresh`], not on each key or frame: a glob over many folders is walked once
/// when it is typed (`a_glob_reads_the_disk_when_typed_not_on_each_key`).
#[derive(Debug, Clone)]
pub struct Browser {
    /// The places column.
    pub places: Vec<Place>,
    /// The place under the places column's cursor.
    pub place: usize,
    /// The folder the list shows.
    dir: PathBuf,
    /// The rows the list shows: the folder's entries, or the files the glob matches.
    rows: Vec<Entry>,
    /// The row under the list's cursor. Read through [`Browser::current`], which keeps it inside
    /// the list as it stands.
    pub cursor: usize,
    /// The files marked with `Space`, in any folder: going into another folder keeps them.
    pub marks: BTreeSet<PathBuf>,
    /// The column that holds the keys.
    pub focus: Pane,
    /// The glob typed after `/`, while one is: the list shows the files under the folder it
    /// matches. `Some` and empty while it is being typed and holds no text yet.
    glob: Option<String>,
    /// Whether the glob stopped reading folders at `glob_limit` with more left to read.
    glob_stopped: bool,
    /// The most folders a glob reads: [`GLOB_FOLDER_LIMIT`], and fewer in a test.
    glob_limit: usize,
    /// How a folder under the cursor is previewed.
    pub preview: PreviewMode,
    /// Whether the preview fills the browser.
    pub full: bool,
    /// The joins proposed for the folder `joins_for` names, each with whether the analyst
    /// accepted it.
    joins: Vec<(ProposedJoin, bool)>,
    /// The folder `joins` were proposed for. They are the canvas's while it is the folder under
    /// the cursor.
    joins_for: Option<PathBuf>,
    /// The joins the analyst accepted, by the folder they were proposed for, kept while the
    /// browser is open, so a folder proposed again keeps its answers
    /// (`accepted_joins_are_kept_for_a_folder_proposed_again`).
    accepted: BTreeMap<PathBuf, BTreeSet<ProposedJoin>>,
    /// The join under the canvas's cursor.
    pub join_cursor: usize,
    /// A sentence for the foot when a key could not do what it asked, until the next key.
    pub note: Option<String>,
    /// The first key of a chord pressed, such as the `g` of `g t`, waiting for the second.
    pending: Option<String>,
}

impl Browser {
    /// The browser opened on `first_source`, in its folder with it under the cursor, or on the
    /// home folder when there is no source, over this Mac's [`places`].
    #[must_use]
    pub fn open_at(first_source: Option<&Path>, fs: &dyn Listing) -> Self {
        let home = std::env::home_dir().unwrap_or_default();
        Self::open_among(places(), &home, first_source, fs)
    }

    /// [`Self::open_at`] over `places`, with `home` the folder it opens on when there is no
    /// source. A folder as the first source opens in its parent with the folder under the
    /// cursor, so the preview shows its canvas, as a dropped folder's does.
    #[must_use]
    pub fn open_among(
        places: Vec<Place>,
        home: &Path,
        first_source: Option<&Path>,
        fs: &dyn Listing,
    ) -> Self {
        let source =
            first_source.map(|s| std::path::absolute(s).unwrap_or_else(|_| s.to_path_buf()));
        let (dir, under) = match source.as_deref().and_then(|s| s.parent().map(|p| (p, s))) {
            Some((parent, source)) if !parent.as_os_str().is_empty() => {
                (parent.to_path_buf(), Some(source.to_path_buf()))
            }
            _ => (home.to_path_buf(), None),
        };
        let place = place_for(&places, &dir).unwrap_or(0);
        let mut browser = Self {
            places,
            place,
            dir,
            rows: Vec::new(),
            cursor: 0,
            marks: BTreeSet::new(),
            focus: Pane::List,
            glob: None,
            glob_stopped: false,
            glob_limit: GLOB_FOLDER_LIMIT,
            preview: PreviewMode::Canvas,
            full: false,
            joins: Vec::new(),
            joins_for: None,
            accepted: BTreeMap::new(),
            join_cursor: 0,
            note: None,
            pending: None,
        };
        browser.read_rows(fs);
        if let Some(under) = under {
            browser.cursor = browser.row_of(&under).unwrap_or(0);
        }
        browser
    }

    /// The folder the list shows.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The glob as typed after `/`, while one is open.
    #[must_use]
    pub fn glob(&self) -> Option<&str> {
        self.glob.as_deref()
    }

    /// The glob in force: the one typed, when it holds more than spaces.
    #[must_use]
    pub fn active_glob(&self) -> Option<&str> {
        self.glob
            .as_deref()
            .map(str::trim)
            .filter(|g| !g.is_empty())
    }

    /// Whether the glob in force stopped reading folders before it had read each folder it could
    /// match in, so the list may be missing files it matches; an add is then refused
    /// (`a_glob_that_stops_reading_says_so_and_adds_nothing`).
    #[must_use]
    pub fn glob_stopped(&self) -> bool {
        self.glob_stopped
    }

    /// The rows the list shows: the files under the folder the glob matches while one is in
    /// force, and the folder's entries otherwise.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.rows
    }

    /// The row under the list's cursor, kept inside the list as it now stands.
    #[must_use]
    pub fn current(&self) -> Option<&Entry> {
        let last = self.rows.len().checked_sub(1)?;
        self.rows.get(self.cursor.min(last))
    }

    /// The data files under the folder the glob in force matches, by their path from it, in that
    /// path's order with case ignored. `*` and `?` match within one folder's names and `**`
    /// matches any depth of folders, zero included, so `**/*.csv` matches the CSVs at the top as
    /// well as those below (`a_glob_matches_by_name_and_a_double_star_crosses_folders`). Names
    /// are matched with case ignored, as the Mac's own disk treats them.
    #[must_use]
    pub fn matches(&self) -> Vec<PathBuf> {
        if self.active_glob().is_none() {
            return Vec::new();
        }
        self.rows.iter().map(|e| e.path.clone()).collect()
    }

    /// Put `text` in the glob, as the analyst types it after `/`, and list what it matches, with
    /// the list's cursor on its first match.
    pub fn set_glob(&mut self, text: &str, fs: &dyn Listing) {
        self.glob = Some(text.to_string());
        self.cursor = 0;
        self.note = None;
        self.read_rows(fs);
    }

    /// Read the list's rows from the disk again, keeping the cursor on the row it was on while
    /// that row is still listed: a file the Finder added or took away shows.
    pub fn refresh(&mut self, fs: &dyn Listing) {
        let at = self.current().map(|e| e.path.clone());
        self.read_rows(fs);
        if let Some(row) = at.and_then(|at| self.row_of(&at)) {
            self.cursor = row;
        }
    }

    /// The folder under the list's cursor, whose joins the canvas previews: none while a glob is
    /// in force, since the list then holds files.
    #[must_use]
    pub fn canvas_folder(&self) -> Option<&Path> {
        if self.active_glob().is_some() {
            return None;
        }
        self.current()
            .filter(|e| e.kind == Kind::Folder)
            .map(|e| e.path.as_path())
    }

    /// Take the joins dovetail proposes between the tables of `folder`. A join the analyst
    /// accepted on the folder before, while the browser has been open, comes back accepted while
    /// it is still proposed; one no longer proposed is let go, so it cannot come back unseen
    /// (`an_add_carries_the_accepted_joins_the_canvas_shows`).
    pub fn set_joins(&mut self, folder: &Path, proposed: Vec<ProposedJoin>) {
        if let Some(kept) = self.accepted.get_mut(folder) {
            kept.retain(|join| proposed.contains(join));
        }
        let kept = self.accepted.get(folder);
        self.joins = proposed
            .into_iter()
            .map(|join| {
                let accepted = kept.is_some_and(|k| k.contains(&join));
                (join, accepted)
            })
            .collect();
        if self.joins_for.as_deref() != Some(folder) {
            self.join_cursor = 0;
        }
        self.join_cursor = self.join_cursor.min(self.joins.len().saturating_sub(1));
        self.joins_for = Some(folder.to_path_buf());
    }

    /// The joins on the canvas: those proposed for the folder under the cursor, once they are,
    /// each with whether the analyst accepted it.
    #[must_use]
    pub fn canvas_joins(&self) -> &[(ProposedJoin, bool)] {
        match (&self.joins_for, self.canvas_folder()) {
            (Some(for_folder), Some(folder)) if for_folder == folder => &self.joins,
            _ => &[],
        }
    }

    /// Accept the canvas's join `at`, or drop it if it was accepted, as `Space` on it does and a
    /// click on it will.
    pub fn toggle_join(&mut self, at: usize) {
        let Some(folder) = self.canvas_folder().map(Path::to_path_buf) else {
            return;
        };
        if self.joins_for.as_deref() != Some(folder.as_path()) {
            return;
        }
        let Some((join, accepted)) = self.joins.get_mut(at) else {
            return;
        };
        *accepted = !*accepted;
        let kept = self.accepted.entry(folder).or_default();
        if *accepted {
            kept.insert(join.clone());
        } else {
            kept.remove(join);
        }
    }

    /// Answer a keystroke, as the registry spells it (`"j"`, `"enter"`, `"space"`, `"g"`), by
    /// the verb the Browser context binds to it. The first key of a chord waits for the second; a
    /// second key that makes no chord with it is answered on its own. A keystroke the Browser
    /// context does not bind, such as the window's `cmd-s`, changes nothing here
    /// (`a_key_the_browser_does_not_bind_does_nothing`).
    pub fn press(&mut self, keystroke: &str, fs: &dyn Listing) -> Effect {
        if let Some(first) = self.pending.take() {
            if let Some(verb) = browser_verb(&format!("{first} {keystroke}")) {
                return self.key(verb, fs);
            }
        }
        if let Some(verb) = browser_verb(keystroke) {
            return self.key(verb, fs);
        }
        let opens_a_chord = browser_keys().iter().any(|b| {
            b.keystrokes
                .strip_prefix(keystroke)
                .is_some_and(|rest| rest.starts_with(' '))
        });
        if opens_a_chord {
            self.pending = Some(keystroke.to_string());
        }
        Effect::None
    }

    /// Answer the registry verb `verb` in the focused column. A verb the browser does not answer
    /// does nothing.
    pub fn key(&mut self, verb: &str, fs: &dyn Listing) -> Effect {
        self.note = None;
        match verb {
            CLOSE => return Effect::Close,
            NEXT_PANE => {
                self.next_pane();
                return Effect::None;
            }
            FILL => {
                self.fill();
                return Effect::None;
            }
            SWITCH_PREVIEW => {
                self.preview = match self.preview {
                    PreviewMode::Canvas => PreviewMode::Files,
                    PreviewMode::Files => PreviewMode::Canvas,
                };
                return Effect::None;
            }
            TYPE_GLOB => {
                // An empty glob lists the folder as it is, so the rows need no reading again.
                self.glob.get_or_insert_with(String::new);
                self.focus = Pane::List;
                self.full = false;
                return Effect::None;
            }
            _ => {}
        }
        match self.focus {
            Pane::Places => self.places_key(verb, fs),
            Pane::List => self.list_key(verb, fs),
            Pane::Canvas => self.canvas_key(verb, fs),
        }
    }

    fn places_key(&mut self, verb: &str, fs: &dyn Listing) -> Effect {
        let last = self.places.len().saturating_sub(1);
        match verb {
            MOVE_DOWN => self.place = (self.place + 1).min(last),
            MOVE_UP => self.place = self.place.saturating_sub(1),
            GO_IN_OR_ADD => {
                if let Some(place) = self.places.get(self.place) {
                    let path = place.path.clone();
                    self.go_to(&path, None, fs);
                    self.focus = Pane::List;
                }
            }
            _ => {}
        }
        Effect::None
    }

    fn list_key(&mut self, verb: &str, fs: &dyn Listing) -> Effect {
        let last = self.rows.len().saturating_sub(1);
        match verb {
            MOVE_DOWN => {
                self.cursor = (self.cursor + 1).min(last);
                self.join_cursor = 0;
            }
            MOVE_UP => {
                self.cursor = self.cursor.min(last).saturating_sub(1);
                self.join_cursor = 0;
            }
            GO_UP => {
                if self.glob.is_some() {
                    self.glob = None;
                    self.cursor = 0;
                    self.read_rows(fs);
                } else if let Some(parent) = self.dir.parent().map(Path::to_path_buf) {
                    let from = self.dir.clone();
                    self.go_to(&parent, Some(&from), fs);
                }
            }
            GO_IN_OR_ADD => {
                if self.active_glob().is_some() {
                    return self.add(fs);
                }
                match self.current().cloned() {
                    Some(Entry {
                        path,
                        kind: Kind::Folder,
                    }) => self.go_to(&path, None, fs),
                    Some(_) => return self.add(fs),
                    None => {}
                }
            }
            ADD => return self.add(fs),
            MARK_OR_ACCEPT => {
                if let Some(path) = self
                    .current()
                    .filter(|e| e.kind == Kind::Data)
                    .map(|e| e.path.clone())
                {
                    if !self.marks.remove(&path) {
                        self.marks.insert(path);
                    }
                    self.cursor = (self.cursor + 1).min(last);
                }
            }
            _ => {}
        }
        Effect::None
    }

    fn canvas_key(&mut self, verb: &str, fs: &dyn Listing) -> Effect {
        let joins = if self.preview == PreviewMode::Canvas {
            self.canvas_joins().len()
        } else {
            0
        };
        match verb {
            MOVE_DOWN if joins > 0 => self.join_cursor = (self.join_cursor + 1).min(joins - 1),
            MOVE_UP if joins > 0 => self.join_cursor = self.join_cursor.saturating_sub(1),
            MARK_OR_ACCEPT if joins > 0 => self.toggle_join(self.join_cursor.min(joins - 1)),
            GO_UP => {
                self.focus = Pane::List;
                self.full = false;
            }
            GO_IN_OR_ADD | ADD => {
                if let Some(folder) = self.canvas_folder().map(Path::to_path_buf) {
                    return self.add_folder(&folder, fs);
                }
            }
            _ => {}
        }
        Effect::None
    }

    /// Whether `Tab` and `g t` can put the keys on the canvas: a folder is under the cursor,
    /// previewed as its canvas, with joins proposed on it to move between.
    fn canvas_reachable(&self) -> bool {
        self.preview == PreviewMode::Canvas && !self.canvas_joins().is_empty()
    }

    fn next_pane(&mut self) {
        let canvas = self.canvas_reachable();
        self.focus = if self.full {
            if self.focus == Pane::Canvas || !canvas {
                Pane::List
            } else {
                Pane::Canvas
            }
        } else {
            match self.focus {
                Pane::Places => Pane::List,
                Pane::List if canvas => Pane::Canvas,
                Pane::List | Pane::Canvas => Pane::Places,
            }
        };
    }

    fn fill(&mut self) {
        self.full = !self.full;
        if self.full && self.canvas_reachable() {
            self.focus = Pane::Canvas;
        } else if !self.full && self.focus == Pane::Canvas {
            self.focus = Pane::List;
        }
    }

    /// The row of the list whose path is `path`.
    fn row_of(&self, path: &Path) -> Option<usize> {
        self.rows.iter().position(|e| e.path == path)
    }

    /// Read the rows the list shows: the files the glob in force matches, walked breadth first so
    /// the folders nearest the top are read before deep ones, or the folder's entries.
    fn read_rows(&mut self, fs: &dyn Listing) {
        if let Some(glob) = self.active_glob().map(str::to_string) {
            let (found, stopped) = glob_walk(fs, &self.dir, &glob, self.glob_limit);
            self.rows = found
                .into_iter()
                .map(|path| Entry {
                    path,
                    kind: Kind::Data,
                })
                .collect();
            self.glob_stopped = stopped;
        } else {
            self.rows = fs.list(&self.dir);
            self.glob_stopped = false;
        }
    }

    /// Show `dir` in the list, with `under` under the cursor when it is one of its rows and the
    /// first row otherwise. The glob is let go; the marks, and the joins accepted, stay.
    fn go_to(&mut self, dir: &Path, under: Option<&Path>, fs: &dyn Listing) {
        self.dir = dir.to_path_buf();
        self.glob = None;
        self.join_cursor = 0;
        if self.focus == Pane::Canvas {
            self.focus = Pane::List;
        }
        self.read_rows(fs);
        self.cursor = under.and_then(|u| self.row_of(u)).unwrap_or(0);
        if let Some(place) = place_for(&self.places, &self.dir) {
            self.place = place;
        }
    }

    /// What `a` adds from the list: the files the glob matches, with the marked files; else the
    /// marked files; else what is under the cursor, a folder with the joins accepted on its
    /// canvas.
    fn add(&mut self, fs: &dyn Listing) -> Effect {
        if let Some(glob) = self.active_glob().map(str::to_string) {
            if self.glob_stopped {
                self.note = Some(format!(
                    "{glob} stopped after reading {} folders, so it may not have found each file \
                     it matches. Type a narrower glob, such as */*.csv, to add what it matches.",
                    self.glob_limit
                ));
                return Effect::None;
            }
            let mut found = self.matches();
            if found.is_empty() {
                self.note = Some(format!(
                    "No files in {} or below match {glob}.",
                    self.dir.display()
                ));
                return Effect::None;
            }
            let more: Vec<PathBuf> = self
                .marks
                .iter()
                .filter(|m| !found.contains(m))
                .cloned()
                .collect();
            found.extend(more);
            return Effect::Add(found, Vec::new());
        }
        if !self.marks.is_empty() {
            return Effect::Add(self.marks.iter().cloned().collect(), Vec::new());
        }
        let Some(entry) = self.current().cloned() else {
            return Effect::None;
        };
        match entry.kind {
            Kind::Data => Effect::Add(vec![entry.path], Vec::new()),
            Kind::Folder => self.add_folder(&entry.path, fs),
            Kind::Other => {
                self.note = Some(format!(
                    "{} is not a file Brightfield reads. It reads CSV, TSV and Parquet files, \
                     Excel workbooks, and DuckDB and SQLite databases.",
                    entry.name()
                ));
                Effect::None
            }
        }
    }

    /// `folder` added whole, with the joins its canvas shows accepted and the marked files outside
    /// its top level; a folder with no data file at its top adds nothing and says how to reach
    /// what is below. Before its joins are proposed the canvas shows none, and it is added with
    /// none.
    fn add_folder(&mut self, folder: &Path, fs: &dyn Listing) -> Effect {
        if !fs.list(folder).iter().any(|e| e.kind == Kind::Data) {
            let name = folder.file_name().map_or_else(
                || folder.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            self.note = Some(format!(
                "{name} holds no data files at its top. Press l or Enter to go in, or / to type a \
                 glob such as */*.csv."
            ));
            return Effect::None;
        }
        let joins = if self.joins_for.as_deref() == Some(folder) {
            self.joins
                .iter()
                .filter(|(_, accepted)| *accepted)
                .map(|(join, _)| join.accepted())
                .collect()
        } else {
            Vec::new()
        };
        let mut paths = vec![folder.to_path_buf()];
        paths.extend(
            self.marks
                .iter()
                .filter(|m| m.parent() != Some(folder))
                .cloned(),
        );
        Effect::Add(paths, joins)
    }
}

/// The place `dir` is in: the one whose folder is the longest part of its path.
fn place_for(places: &[Place], dir: &Path) -> Option<usize> {
    places
        .iter()
        .enumerate()
        .filter(|(_, p)| dir.starts_with(&p.path))
        .max_by_key(|(_, p)| p.path.components().count())
        .map(|(i, _)| i)
}

/// The registry's bindings in the Browser context.
fn browser_keys() -> &'static [BoundKey] {
    static KEYS: OnceLock<Vec<BoundKey>> = OnceLock::new();
    KEYS.get_or_init(|| {
        keymap_bindings(&registry())
            .into_iter()
            .filter(|b| b.context == BindingContext::Browser)
            .collect()
    })
}

/// The verb the Browser context binds to `keystrokes`, if it binds one.
fn browser_verb(keystrokes: &str) -> Option<&'static str> {
    browser_keys()
        .iter()
        .find(|b| b.keystrokes == keystrokes)
        .map(|b| b.longname)
}

/// The data files under `top` whose path from it matches `glob`, in that path's order with case
/// ignored, and whether the walk stopped with folders left to read. Folders are read breadth
/// first, as deep as the glob's parts reach, and the walk stops once it has read `limit` folders.
fn glob_walk(fs: &dyn Listing, top: &Path, glob: &str, limit: usize) -> (Vec<PathBuf>, bool) {
    let pattern: Vec<&str> = glob.split('/').filter(|s| !s.is_empty()).collect();
    let depth = if pattern.contains(&"**") {
        usize::MAX
    } else {
        pattern.len()
    };
    let mut queue = VecDeque::from([(top.to_path_buf(), String::new(), depth)]);
    let mut read = 0;
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    let mut stopped = false;
    while let Some((dir, rel, depth)) = queue.pop_front() {
        if read == limit {
            stopped = true;
            break;
        }
        read += 1;
        for entry in fs.list(&dir) {
            let name = entry.name();
            let here = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            match entry.kind {
                Kind::Data => {
                    let parts: Vec<&str> = here.split('/').collect();
                    if segments_match(&pattern, &parts) {
                        found.push((here, entry.path));
                    }
                }
                Kind::Folder if depth > 1 => queue.push_back((entry.path, here, depth - 1)),
                Kind::Folder | Kind::Other => {}
            }
        }
    }
    found.sort_by(|(a, _), (b, _)| a.to_lowercase().cmp(&b.to_lowercase()).then(a.cmp(b)));
    (found.into_iter().map(|(_, path)| path).collect(), stopped)
}

/// Whether the path `parts` matches the glob `pattern`, a part each: `**` takes any number of
/// parts, zero included, and each other part of the pattern matches one of the path's.
fn segments_match(pattern: &[&str], parts: &[&str]) -> bool {
    match pattern.split_first() {
        None => parts.is_empty(),
        Some((&"**", rest)) => (0..=parts.len()).any(|skip| segments_match(rest, &parts[skip..])),
        Some((first, rest)) => parts
            .split_first()
            .is_some_and(|(part, after)| name_matches(first, part) && segments_match(rest, after)),
    }
}

/// Whether the name `name` matches the glob part `pattern`, with case ignored: `*` takes any run
/// of characters and `?` one.
fn name_matches(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let s: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut si) = (0, 0);
    // The last `*` met, and where in the name it was tried from, to try it one character longer.
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, si));
            pi += 1;
        } else if let Some((at, from)) = star {
            pi = at + 1;
            si = from + 1;
            star = Some((at, from + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// A disk held in memory, made from the paths of its files: a path ending in `/` is an
    /// empty folder, and each folder above a path is made.
    struct Memory(BTreeMap<PathBuf, Vec<Entry>>);

    impl Memory {
        fn of(paths: &[&str]) -> Self {
            let mut folders: BTreeMap<PathBuf, BTreeSet<(bool, PathBuf)>> = BTreeMap::new();
            for path in paths {
                let mut folder = path.ends_with('/');
                let mut at = PathBuf::from(path.trim_end_matches('/'));
                if folder {
                    folders.entry(at.clone()).or_default();
                }
                while let Some(parent) = at.parent().map(Path::to_path_buf) {
                    folders
                        .entry(parent.clone())
                        .or_default()
                        .insert((folder, at));
                    at = parent;
                    folder = true;
                }
            }
            let listed = folders
                .into_iter()
                .map(|(dir, children)| {
                    let mut entries: Vec<Entry> = children
                        .into_iter()
                        .map(|(folder, path)| Entry {
                            kind: if folder {
                                Kind::Folder
                            } else {
                                Kind::of_file(&path)
                            },
                            path,
                        })
                        .collect();
                    sort_entries(&mut entries);
                    (dir, entries)
                })
                .collect();
            Self(listed)
        }
    }

    impl Listing for Memory {
        fn list(&self, dir: &Path) -> Vec<Entry> {
            self.0.get(dir).cloned().unwrap_or_default()
        }
    }

    /// A disk that counts the folders read from it.
    struct Counting<'a>(&'a Memory, Cell<usize>);

    impl Listing for Counting<'_> {
        fn list(&self, dir: &Path) -> Vec<Entry> {
            self.1.set(self.1.get() + 1);
            self.0.list(dir)
        }
    }

    const HOME: &str = "/home/ana";
    const DATA: &str = "/home/ana/data";
    const SHOP: &str = "/home/ana/data/shop";
    const TIDES: &str = "/home/ana/data/tides";

    const FILES: &[&str] = &[
        "/home/ana/Desktop/",
        "/home/ana/data/archive/2023/q1.csv",
        "/home/ana/data/shop/customers.csv",
        "/home/ana/data/shop/orders.csv",
        "/home/ana/data/shop/products.parquet",
        "/home/ana/data/tides/stations.csv",
        "/home/ana/data/tides/tides.csv",
        "/home/ana/data/earthquakes.csv",
        "/home/ana/data/notes.txt",
    ];

    fn disk() -> Memory {
        Memory::of(FILES)
    }

    fn places_here() -> Vec<Place> {
        vec![
            Place::new("Home", HOME),
            Place::new("data", DATA),
            Place::new("Desktop", "/home/ana/Desktop"),
        ]
    }

    /// The browser opened in the data folder with `name` under the cursor.
    fn at(name: &str, fs: &Memory) -> Browser {
        let path = Path::new(DATA).join(name);
        Browser::open_among(places_here(), Path::new(HOME), Some(&path), fs)
    }

    fn path(p: &str) -> PathBuf {
        PathBuf::from(p)
    }

    fn shop_joins() -> Vec<ProposedJoin> {
        vec![
            ProposedJoin::new(("orders", "customer_id"), ("customers", "customer_id")),
            ProposedJoin::new(("orders", "product_id"), ("products", "product_id")),
        ]
    }

    /// The browser on the shop folder, with its two joins proposed.
    fn on_shop(fs: &Memory) -> Browser {
        let mut b = at("shop", fs);
        b.set_joins(Path::new(SHOP), shop_joins());
        b
    }

    fn under_cursor(b: &Browser) -> String {
        b.current().map(Entry::name).unwrap_or_default()
    }

    fn answers(b: &Browser) -> Vec<bool> {
        b.canvas_joins().iter().map(|(_, yes)| *yes).collect()
    }

    #[test]
    fn it_opens_on_the_first_source_in_its_folder_with_it_under_the_cursor() {
        let fs = disk();
        let b = at("earthquakes.csv", &fs);
        assert_eq!(b.dir(), Path::new(DATA));
        assert_eq!(under_cursor(&b), "earthquakes.csv");
        assert_eq!(b.places[b.place].label, "data", "the place it is in");
        assert_eq!(b.focus, Pane::List);
    }

    #[test]
    fn it_opens_on_a_folder_source_in_its_parent_so_the_canvas_previews_it() {
        let fs = disk();
        let b = at("shop", &fs);
        assert_eq!(b.dir(), Path::new(DATA));
        assert_eq!(b.canvas_folder(), Some(Path::new(SHOP)));
    }

    #[test]
    fn it_opens_on_the_home_folder_without_a_source() {
        let fs = disk();
        let b = Browser::open_among(places_here(), Path::new(HOME), None, &fs);
        assert_eq!(b.dir(), Path::new(HOME));
        assert_eq!(b.cursor, 0);
        assert_eq!(b.places[b.place].label, "Home");
    }

    #[test]
    fn j_and_k_move_and_stop_at_the_ends() {
        let fs = disk();
        let mut b = Browser::open_among(places_here(), Path::new(DATA), None, &fs);
        // archive, shop, tides, earthquakes.csv, notes.txt
        assert_eq!(under_cursor(&b), "archive");
        b.press("k", &fs);
        assert_eq!(under_cursor(&b), "archive", "k stops at the top");
        b.press("j", &fs);
        b.press("down", &fs);
        assert_eq!(under_cursor(&b), "tides");
        for _ in 0..9 {
            b.press("j", &fs);
        }
        assert_eq!(under_cursor(&b), "notes.txt", "j stops at the foot");
        b.press("up", &fs);
        assert_eq!(under_cursor(&b), "earthquakes.csv");
    }

    #[test]
    fn l_and_enter_go_into_a_folder() {
        let fs = disk();
        for key in ["l", "enter", "right"] {
            let mut b = at("tides", &fs);
            assert_eq!(b.press(key, &fs), Effect::None, "{key}");
            assert_eq!(b.dir(), Path::new(TIDES), "{key} went into the folder");
            assert_eq!(under_cursor(&b), "stations.csv", "{key}");
        }
    }

    #[test]
    fn l_and_enter_add_a_file() {
        let fs = disk();
        for key in ["l", "enter"] {
            let mut b = at("earthquakes.csv", &fs);
            assert_eq!(
                b.press(key, &fs),
                Effect::Add(vec![path("/home/ana/data/earthquakes.csv")], Vec::new()),
                "{key}"
            );
        }
    }

    #[test]
    fn a_file_brightfield_does_not_read_is_not_added_and_the_foot_says_why() {
        let fs = disk();
        let mut b = at("notes.txt", &fs);
        assert_eq!(b.press("enter", &fs), Effect::None);
        let note = b.note.clone().expect("a sentence for the foot");
        assert!(
            note.starts_with("notes.txt is not a file Brightfield reads"),
            "{note}"
        );
        b.press("k", &fs);
        assert_eq!(b.note, None, "the next key clears it");
    }

    #[test]
    fn h_goes_up() {
        let fs = disk();
        let mut b = at("tides", &fs);
        b.press("l", &fs);
        assert_eq!(b.dir(), Path::new(TIDES));
        b.press("h", &fs);
        assert_eq!(b.dir(), Path::new(DATA));
        assert_eq!(
            under_cursor(&b),
            "tides",
            "the folder left is under the cursor"
        );
        b.press("left", &fs);
        assert_eq!(b.dir(), Path::new(HOME));
        assert_eq!(under_cursor(&b), "data");
        assert_eq!(b.places[b.place].label, "Home");
    }

    #[test]
    fn a_adds_the_highlighted_folder_with_its_accepted_joins() {
        let fs = disk();
        let mut b = on_shop(&fs);
        assert_eq!(
            b.clone().press("a", &fs),
            Effect::Add(vec![path(SHOP)], Vec::new()),
            "no join is accepted until the analyst accepts it"
        );
        b.press("tab", &fs);
        assert_eq!(b.focus, Pane::Canvas);
        b.press("j", &fs);
        b.press("space", &fs);
        b.press("h", &fs);
        assert_eq!(b.focus, Pane::List, "h leaves the canvas for the list");
        assert_eq!(
            b.press("a", &fs),
            Effect::Add(vec![path(SHOP)], vec![shop_joins()[1].accepted()])
        );
        assert_eq!(
            b.dir(),
            Path::new(DATA),
            "a adds the folder without going into it"
        );
    }

    #[test]
    fn a_folder_with_no_data_at_its_top_adds_nothing_and_says_how_to_reach_it() {
        let fs = disk();
        let mut b = at("archive", &fs);
        assert_eq!(b.press("a", &fs), Effect::None);
        let note = b.note.clone().expect("a sentence for the foot");
        assert!(note.contains("no data files at its top"), "{note}");
    }

    #[test]
    fn a_adds_the_marked_files_across_folders() {
        let fs = disk();
        let mut b = at("tides", &fs);
        b.press("enter", &fs);
        b.press("j", &fs);
        b.press("space", &fs);
        assert_eq!(
            b.marks,
            BTreeSet::from([path("/home/ana/data/tides/tides.csv")])
        );
        b.press("h", &fs);
        b.press("k", &fs);
        assert_eq!(under_cursor(&b), "shop");
        b.press("l", &fs);
        b.press("j", &fs);
        b.press("space", &fs);
        assert_eq!(
            b.marks.len(),
            2,
            "the mark made in tides survived going into shop"
        );
        assert_eq!(
            b.press("a", &fs),
            Effect::Add(
                vec![
                    path("/home/ana/data/shop/orders.csv"),
                    path("/home/ana/data/tides/tides.csv"),
                ],
                Vec::new()
            )
        );
    }

    #[test]
    fn space_marks_a_file() {
        let fs = disk();
        let mut b = at("earthquakes.csv", &fs);
        b.press("space", &fs);
        assert_eq!(
            b.marks,
            BTreeSet::from([path("/home/ana/data/earthquakes.csv")])
        );
        assert_eq!(
            under_cursor(&b),
            "notes.txt",
            "the cursor moves on, as yazi's does"
        );
        b.press("space", &fs);
        assert_eq!(
            b.marks.len(),
            1,
            "a file Brightfield does not read is not marked"
        );
        b.press("k", &fs);
        b.press("space", &fs);
        assert!(b.marks.is_empty(), "Space again unmarks it");
        let mut on_folder = at("shop", &fs);
        on_folder.press("space", &fs);
        assert!(
            on_folder.marks.is_empty(),
            "a folder is added with a, not marked"
        );
    }

    #[test]
    fn space_on_the_canvas_toggles_the_join() {
        let fs = disk();
        let mut b = on_shop(&fs);
        b.press("tab", &fs);
        b.press("space", &fs);
        assert_eq!(answers(&b), [true, false]);
        b.press("space", &fs);
        assert_eq!(answers(&b), [false, false]);
        assert!(b.marks.is_empty(), "Space on the canvas marks no file");
    }

    #[test]
    fn j_k_on_the_canvas_move_between_joins() {
        let fs = disk();
        let mut b = on_shop(&fs);
        b.press("tab", &fs);
        b.press("j", &fs);
        assert_eq!(b.join_cursor, 1);
        b.press("j", &fs);
        assert_eq!(b.join_cursor, 1, "j stops at the last join");
        b.press("k", &fs);
        b.press("k", &fs);
        assert_eq!(b.join_cursor, 0, "k stops at the first");
        assert_eq!(
            under_cursor(&b),
            "shop",
            "the list's cursor stays on the folder"
        );
    }

    #[test]
    fn tab_cycles_places_list_canvas() {
        let fs = disk();
        let mut b = on_shop(&fs);
        assert_eq!(b.focus, Pane::List);
        let mut seen = Vec::new();
        for _ in 0..3 {
            b.press("tab", &fs);
            seen.push(b.focus);
        }
        assert_eq!(seen, [Pane::Canvas, Pane::Places, Pane::List]);

        // A file has no canvas to move to, so Tab passes it.
        let mut on_file = at("earthquakes.csv", &fs);
        on_file.press("tab", &fs);
        assert_eq!(on_file.focus, Pane::Places);
        on_file.press("tab", &fs);
        assert_eq!(on_file.focus, Pane::List);
    }

    #[test]
    fn the_places_column_goes_to_a_place() {
        let fs = disk();
        let mut b = at("earthquakes.csv", &fs);
        b.press("tab", &fs);
        assert_eq!(b.focus, Pane::Places);
        b.press("k", &fs);
        assert_eq!(b.places[b.place].label, "Home");
        b.press("j", &fs);
        b.press("j", &fs);
        b.press("enter", &fs);
        assert_eq!(b.dir(), Path::new("/home/ana/Desktop"));
        assert_eq!(b.focus, Pane::List, "the keys go to the place's list");
    }

    #[test]
    fn slash_glob_counts_matches() {
        let fs = disk();
        let mut b = at("earthquakes.csv", &fs);
        b.press("/", &fs);
        assert_eq!(b.glob(), Some(""), "the glob is open to type");
        assert_eq!(b.entries().len(), 5, "an empty glob filters nothing");
        b.set_glob("*.csv", &fs);
        assert_eq!(b.matches(), [path("/home/ana/data/earthquakes.csv")]);
        b.set_glob("*/*.CSV", &fs);
        assert_eq!(b.matches().len(), 4, "shop's two CSVs and tides' two");
        b.set_glob("**/*.csv", &fs);
        assert_eq!(b.matches().len(), 6, "the CSVs at each depth");
        assert_eq!(b.entries().len(), 6, "the list shows the matches");
        assert!(!b.glob_stopped());
        let Effect::Add(added, joins) = b.clone().press("enter", &fs) else {
            panic!("Enter with a glob adds what it matches");
        };
        assert_eq!(added, b.matches());
        assert!(joins.is_empty());
        b.set_glob("*.json", &fs);
        assert_eq!(b.press("a", &fs), Effect::None);
        assert!(b
            .note
            .as_deref()
            .is_some_and(|n| n.contains("match *.json")));
        b.press("h", &fs);
        assert_eq!(b.glob(), None, "h clears the glob");
        assert_eq!(b.dir(), Path::new(DATA), "and stays in the folder");
        assert_eq!(b.entries().len(), 5, "and lists the folder again");
    }

    #[test]
    fn a_glob_that_stops_reading_says_so_and_adds_nothing() {
        let fs = disk();
        let mut b = at("earthquakes.csv", &fs);
        b.glob_limit = 2;
        b.set_glob("**/*.csv", &fs);
        assert!(b.glob_stopped(), "two folders do not hold the whole tree");
        assert_eq!(
            b.matches(),
            [path("/home/ana/data/earthquakes.csv")],
            "read breadth first, the top folder's match is found before the deep ones"
        );
        assert_eq!(b.press("a", &fs), Effect::None);
        let note = b.note.clone().expect("a sentence for the foot");
        assert!(note.contains("stopped after reading 2 folders"), "{note}");
        b.glob_limit = GLOB_FOLDER_LIMIT;
        b.set_glob("**/*.csv", &fs);
        assert!(!b.glob_stopped());
    }

    #[test]
    fn a_glob_reads_the_disk_when_typed_not_on_each_key() {
        let memory = disk();
        let fs = Counting(&memory, Cell::new(0));
        let mut b = Browser::open_among(places_here(), Path::new(DATA), None, &fs);
        b.set_glob("**/*.csv", &fs);
        let read = fs.1.get();
        assert!(read > 1, "the glob read the folders below");
        for key in ["j", "space", "k", "tab", "tab", "v", "v"] {
            b.press(key, &fs);
        }
        let _ = (b.entries(), b.current(), b.matches());
        assert_eq!(
            fs.1.get(),
            read,
            "the keys and the reads after it walked nothing again"
        );
    }

    #[test]
    fn refresh_reads_the_folder_again_and_keeps_the_cursor_on_its_row() {
        let fs = disk();
        let mut b = at("tides", &fs);
        let mut more = FILES.to_vec();
        more.push("/home/ana/data/aardvark.csv");
        more.push("/home/ana/data/abc/");
        let grown = Memory::of(&more);
        b.refresh(&grown);
        assert_eq!(b.entries().len(), 7, "the files made since are listed");
        assert_eq!(under_cursor(&b), "tides", "the cursor stays on its row");
    }

    #[test]
    fn a_glob_matches_by_name_and_a_double_star_crosses_folders() {
        assert!(name_matches("*.csv", "orders.csv"));
        assert!(name_matches("ord?rs.*", "Orders.CSV"));
        assert!(!name_matches("*.csv", "orders.csv.bak"));
        assert!(name_matches("*s*s.csv", "stations.csv"));
        assert!(segments_match(&["**", "*.csv"], &["a.csv"]));
        assert!(segments_match(&["**", "*.csv"], &["x", "y", "a.csv"]));
        assert!(!segments_match(&["*", "*.csv"], &["a.csv"]));
        assert!(!segments_match(&["*.csv"], &["x", "a.csv"]));
    }

    #[test]
    fn g_t_fills() {
        let fs = disk();
        let mut b = on_shop(&fs);
        assert_eq!(b.press("g", &fs), Effect::None);
        assert!(!b.full, "g alone waits for the second key");
        b.press("t", &fs);
        assert!(b.full);
        assert_eq!(
            b.focus,
            Pane::Canvas,
            "the keys go to the canvas it fills with"
        );
        b.press("g", &fs);
        b.press("t", &fs);
        assert!(!b.full);
        assert_eq!(b.focus, Pane::List);
        // A g that makes no chord is let go, and the next key is answered on its own.
        b.press("g", &fs);
        b.press("j", &fs);
        assert!(!b.full);
        assert_eq!(under_cursor(&b), "tides");
    }

    #[test]
    fn v_switches_preview() {
        let fs = disk();
        let mut b = on_shop(&fs);
        assert_eq!(b.preview, PreviewMode::Canvas);
        b.press("v", &fs);
        assert_eq!(b.preview, PreviewMode::Files);
        b.press("tab", &fs);
        assert_eq!(
            b.focus,
            Pane::Places,
            "a file list has no joins to move between"
        );
        b.press("v", &fs);
        assert_eq!(b.preview, PreviewMode::Canvas);
    }

    #[test]
    fn enter_on_the_canvas_adds_the_folder() {
        let fs = disk();
        let mut b = at("earthquakes.csv", &fs);
        b.press("space", &fs);
        b.press("k", &fs);
        b.press("k", &fs);
        b.press("k", &fs);
        assert_eq!(under_cursor(&b), "shop");
        b.set_joins(Path::new(SHOP), shop_joins());
        b.press("tab", &fs);
        b.press("space", &fs);
        // Switching the preview to the files and back keeps the keys and the accepted join.
        b.press("v", &fs);
        b.press("v", &fs);
        assert_eq!(
            b.press("enter", &fs),
            Effect::Add(
                vec![path(SHOP), path("/home/ana/data/earthquakes.csv")],
                vec![shop_joins()[0].accepted()]
            ),
            "the folder with the join accepted on its canvas, and the file marked outside it"
        );
    }

    #[test]
    fn accepted_joins_are_kept_for_a_folder_proposed_again() {
        let fs = disk();
        let mut b = on_shop(&fs);
        b.press("tab", &fs);
        b.press("space", &fs);
        b.press("h", &fs);
        // The cursor moves to tides, whose joins are proposed, and back to shop, proposed again.
        b.press("j", &fs);
        b.set_joins(Path::new(TIDES), Vec::new());
        assert!(b.canvas_joins().is_empty(), "tides' joins are not shop's");
        b.press("k", &fs);
        assert!(
            b.canvas_joins().is_empty(),
            "joins proposed for tides are not drawn on shop"
        );
        assert_eq!(
            b.clone().press("a", &fs),
            Effect::Add(vec![path(SHOP)], Vec::new()),
            "before shop is proposed again its canvas shows no join, and none is added"
        );
        b.set_joins(Path::new(SHOP), shop_joins());
        assert_eq!(answers(&b), [true, false]);
        assert_eq!(
            b.press("a", &fs),
            Effect::Add(vec![path(SHOP)], vec![shop_joins()[0].accepted()])
        );
    }

    #[test]
    fn an_add_carries_the_accepted_joins_the_canvas_shows() {
        let fs = disk();
        let mut b = on_shop(&fs);
        b.press("tab", &fs);
        b.press("space", &fs);
        b.press("j", &fs);
        b.press("space", &fs);
        assert_eq!(answers(&b), [true, true]);
        // dovetail proposes again without the first join, as when a table it named was renamed.
        b.set_joins(Path::new(SHOP), vec![shop_joins()[1].clone()]);
        assert_eq!(answers(&b), [true]);
        assert_eq!(
            b.clone().press("enter", &fs),
            Effect::Add(vec![path(SHOP)], vec![shop_joins()[1].accepted()]),
            "the join no longer proposed is not added"
        );
        // Nor does it come back accepted if it is proposed once more.
        b.set_joins(Path::new(SHOP), shop_joins());
        assert_eq!(answers(&b), [false, true]);
    }

    #[test]
    fn esc_closes() {
        let fs = disk();
        for focus in [Pane::Places, Pane::List, Pane::Canvas] {
            let mut b = on_shop(&fs);
            b.focus = focus;
            assert_eq!(b.press("escape", &fs), Effect::Close, "{focus:?}");
        }
    }

    #[test]
    fn a_key_the_browser_does_not_bind_does_nothing() {
        let fs = disk();
        let mut b = at("earthquakes.csv", &fs);
        let before = (b.dir().to_path_buf(), b.cursor, b.focus);
        assert_eq!(b.press("cmd-s", &fs), Effect::None);
        assert_eq!(b.press("x", &fs), Effect::None);
        assert_eq!((b.dir().to_path_buf(), b.cursor, b.focus), before);
    }

    /// Each verb the browser answers is bound in the registry's Browser context, and each verb
    /// bound there is one it answers, so a verb renamed or added in the registry reddens here.
    #[test]
    fn the_browser_answers_the_verbs_its_context_binds() {
        let bound: BTreeSet<&str> = browser_keys().iter().map(|b| b.longname).collect();
        let answered: BTreeSet<&str> = VERBS.into_iter().collect();
        assert_eq!(bound, answered);
    }

    #[test]
    fn the_disk_lists_folders_first_and_leaves_hidden_files_out() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        std::fs::create_dir(dir.path().join("Zed")).unwrap();
        std::fs::write(dir.path().join("a.csv"), "x\n1\n").unwrap();
        std::fs::write(dir.path().join("B.txt"), "").unwrap();
        std::fs::write(dir.path().join(".hidden.csv"), "").unwrap();
        let listed: Vec<(String, Kind)> = Disk
            .list(dir.path())
            .into_iter()
            .map(|e| (e.name(), e.kind))
            .collect();
        assert_eq!(
            listed,
            [
                ("Zed".to_string(), Kind::Folder),
                ("a.csv".to_string(), Kind::Data),
                ("B.txt".to_string(), Kind::Other),
            ]
        );
        assert!(Disk.list(&dir.path().join("missing")).is_empty());
    }
}
