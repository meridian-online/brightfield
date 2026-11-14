//! **Drawing the sources browser**: its three columns, the preview of what is under the cursor, a
//! folder's canvas of joins, and the keys along its foot, in the window's theme.
//!
//! The browser's state and keys are [`crate::sources_browser`]'s. This module draws a
//! [`Browser`] and turns a frame's clicks and keystrokes into its verbs. Every action has a key
//! and a click: a row is clicked to move to it and double-clicked as Enter, a join is clicked as
//! Space on it, and the keys the focused column answers stand along the foot, where a click on
//! one does what the key does. The colours are the window's theme tokens
//! ([`meridian_design::semantic()`]) in light and dark (`it_takes_the_window_theme`).
//!
//! What the preview shows is read off the frame thread, as the grid's summaries are: a file's
//! first page of rows, and the joins dovetail proposes between a folder's tables. [`Previews`]
//! starts a read the first time the frame asks for one, on a thread of its own and one at a time,
//! and the frame draws what has arrived, so a slow read never holds the window
//! (`a_file_preview_shows_its_first_rows_without_blocking_the_frame`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

use brightfield_keys::{keymap_bindings, registry, BindingContext};
use meridian_design::semantic;
use meridian_egui::{key_chip, list_row, ListRow, MeridianUi as _, RowHeight};

use crate::design::{to_color32, Mode};
use crate::sources_browser::{
    Browser, Disk, Effect, Entry, Kind, Listing, Pane, PreviewMode, ProposedJoin, ADD, CLOSE, FILL,
    GO_IN_OR_ADD, GO_UP, MARK_OR_ACCEPT, MOVE_DOWN, MOVE_UP, NEXT_PANE, SWITCH_PREVIEW, TYPE_GLOB,
};

/// How many rows a file's preview reads: the first page.
pub const PAGE_ROWS: u64 = 50;

/// The most data files at a folder's top that a preview asks dovetail about. A folder holding more
/// is previewed as its list of files, and dovetail proposes its joins once it is added.
pub const PREVIEW_TABLE_LIMIT: usize = 16;

/// The first page of a data file, as its preview shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    /// The column names, in the file's order.
    pub columns: Vec<String>,
    /// The first rows, each a cell per column, written as text.
    pub rows: Vec<Vec<String>>,
    /// For a workbook or a database, the tables it holds, listed by name and not read.
    pub tables: Vec<String>,
}

/// What reads a file's first page. The window's reads the file with DuckDB ([`read_first_page`]);
/// a test's can wait to answer.
pub type ReadPage = Arc<dyn Fn(&Path) -> Result<Page, String> + Send + Sync>;

/// What proposes the joins between a folder's tables. The window's runs dovetail's sample pass
/// over a scratch Protocol of the folder ([`propose_joins`]); a test's answers what it is given.
pub type ProposeJoins = Arc<dyn Fn(&Path) -> Result<Vec<ProposedJoin>, String> + Send + Sync>;

/// What reads one kind of preview for a path.
type Read<T> = Arc<dyn Fn(&Path) -> Result<T, String> + Send + Sync>;

/// Reads of one kind, one at a time off the frame thread, kept by path once they answer.
struct Reads<T> {
    read: Read<T>,
    done: HashMap<PathBuf, Result<T, String>>,
    running: Option<(PathBuf, Receiver<Result<T, String>>)>,
}

impl<T: Send + 'static> Reads<T> {
    fn new(read: Read<T>) -> Self {
        Self {
            read,
            done: HashMap::new(),
            running: None,
        }
    }

    /// What the read of `path` answered, once it has; until then the read is started, when no
    /// other is running, and `None` is answered. `wake` is asked for a frame when it lands.
    fn get(&mut self, path: &Path, wake: Option<&egui::Context>) -> Option<&Result<T, String>> {
        self.poll();
        if !self.done.contains_key(path) && self.running.is_none() {
            let (tx, rx) = mpsc::channel();
            let read = Arc::clone(&self.read);
            let at = path.to_path_buf();
            let wake = wake.cloned();
            std::thread::spawn(move || {
                let _ = tx.send(read(&at));
                if let Some(ctx) = wake {
                    ctx.request_repaint();
                }
            });
            self.running = Some((path.to_path_buf(), rx));
        }
        self.done.get(path)
    }

    /// Whether a read is running.
    fn busy(&mut self) -> bool {
        self.poll();
        self.running.is_some()
    }

    fn poll(&mut self) {
        let Some((path, rx)) = &self.running else {
            return;
        };
        let answered = match rx.try_recv() {
            Ok(answer) => answer,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the read stopped before it answered".into()),
        };
        self.done.insert(path.clone(), answered);
        self.running = None;
    }
}

/// The previews' reads: files' first pages and folders' proposed joins.
pub struct Previews {
    pages: Reads<Page>,
    joins: Reads<Vec<ProposedJoin>>,
    /// The files of each folder previewed as its list, read once.
    files: HashMap<PathBuf, Vec<Entry>>,
}

impl Previews {
    /// Previews read by `read_page` and `propose`.
    #[must_use]
    pub fn new(read_page: ReadPage, propose: ProposeJoins) -> Self {
        Self {
            pages: Reads::new(read_page),
            joins: Reads::new(propose),
            files: HashMap::new(),
        }
    }

    /// Take `joins` as what dovetail proposes for `folder`, with no read: the window knows them
    /// already when the folder is one it has just made a Protocol of.
    pub fn know_joins(&mut self, folder: &Path, joins: Vec<ProposedJoin>) {
        self.joins.done.insert(folder.to_path_buf(), Ok(joins));
    }
}

/// What the preview column showed on the last frame.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Shown {
    /// Nothing is under the cursor.
    #[default]
    Nothing,
    /// A read under way, in the sentence the preview says.
    Waiting(String),
    /// A file's first rows: its columns and how many rows were drawn.
    Rows {
        /// The column names.
        columns: Vec<String>,
        /// The rows drawn.
        rows: usize,
    },
    /// A workbook's or a database's tables, by name.
    Tables(Vec<String>),
    /// A folder's canvas: each join as its row reads.
    Canvas(Vec<String>),
    /// A folder's files, by name.
    Files(Vec<String>),
    /// A sentence in place of a preview: a read that failed, or a file Brightfield does not read.
    Said(String),
}

/// Where the last frame drew what a click acts on, and what it said: the window's tests and the
/// view's read these.
#[derive(Debug, Clone, Default)]
pub struct Drawn {
    /// The sheet's own rectangle.
    pub sheet: Option<egui::Rect>,
    /// Each place's row.
    pub places: Vec<egui::Rect>,
    /// Each row of the list.
    pub rows: Vec<egui::Rect>,
    /// Each join's row on a folder's canvas.
    pub joins: Vec<egui::Rect>,
    /// The foot's keys, each as it reads: the keys, then what they do.
    pub foot: Vec<String>,
    /// Where each of the foot's keys was drawn, in [`Drawn::foot`]'s order.
    pub foot_rects: Vec<egui::Rect>,
    /// What the preview column showed.
    pub preview: Shown,
}

/// The sources browser as the window holds it: its state, its previews, the glob being typed,
/// and what the last frame drew.
pub struct Sheet {
    /// The browser's state and keys.
    pub browser: Browser,
    previews: Previews,
    /// The glob's text as typed, while the glob is open.
    glob_text: String,
    /// Whether the glob's field holds the keys: from `/` (or a click on the field) until `Enter`
    /// or `Esc`. Kept here rather than read from egui's focus, which `Tab` and the arrows move
    /// from widget to widget and the browser answers those keys itself.
    glob_typing: bool,
    /// The list's cursor the last frame scrolled to, so a key that moves it scrolls once.
    scrolled_to: Option<(PathBuf, usize)>,
    drawn: Drawn,
}

impl Sheet {
    /// A sheet over `browser`, its previews read by `previews`.
    #[must_use]
    pub fn new(browser: Browser, previews: Previews) -> Self {
        Self {
            browser,
            previews,
            glob_text: String::new(),
            glob_typing: false,
            scrolled_to: None,
            drawn: Drawn::default(),
        }
    }

    /// The previews, to tell them what the window knows.
    pub fn previews_mut(&mut self) -> &mut Previews {
        &mut self.previews
    }

    /// What the last frame drew.
    #[must_use]
    pub fn drawn(&self) -> &Drawn {
        &self.drawn
    }

    /// Whether a preview's read is under way.
    pub fn reading(&mut self) -> bool {
        self.previews.pages.busy() || self.previews.joins.busy()
    }
}

/// The sheet's frame: the floating plane's surface, its hairline, the panel radius and the modal
/// padding, from the window's theme tokens, as a modal card's.
pub fn frame(ui: &egui::Ui) -> egui::Frame {
    let sem = semantic(ui.visuals().dark_mode);
    let t = ui.tokens();
    egui::Frame::new()
        .fill(to_color32(sem.surfaces.overlay))
        .stroke(egui::Stroke::new(1.0, to_color32(sem.borders.default_)))
        .corner_radius(t.radius_panel)
        .inner_margin(egui::Margin::same(t.modal_padding as i8))
        .shadow(ui.visuals().window_shadow)
}

/// **Draw the browser into `ui`**, in its frame and the theme of `mode`, and answer what its keys
/// and clicks asked of the window this frame.
///
/// The keys are read before the sheet is drawn and taken out of the frame's input, so a `j`
/// moves the browser's cursor and not the grid's behind the sheet, and `Esc` closes the browser
/// rather than a layer under it. While the glob's field holds the keys they type into it, and `Enter`
/// and `Esc` leave it. A key with a modifier held, such as the window's `Cmd+S`, is left in the
/// input for the window.
pub fn show(ui: &mut egui::Ui, sheet: &mut Sheet, mode: Mode) -> Effect {
    let dark = mode.is_dark();
    let fs = Disk;
    let wake = ui.ctx().clone();
    sheet.drawn = Drawn::default();
    // A row is the click target, so the words on it are not selectable text that would take the
    // click from the row under them.
    ui.style_mut().interaction.selectable_labels = false;

    let glob_id = ui.id().with("bf-sources-browser-glob");
    // A glob let go some other way, by a place chosen or a folder gone into, is no longer typed.
    if sheet.browser.glob().is_none() {
        sheet.glob_typing = false;
    }
    let mut effect = if sheet.glob_typing {
        glob_keys(ui, sheet, &fs)
    } else {
        browser_keys(ui, sheet, &fs)
    };

    // The canvas takes dovetail's joins for the folder under the cursor once they are proposed.
    sync_joins(sheet, Some(&wake));

    let sem = semantic(dark);
    let shown = frame(ui).show(ui, |ui| {
        let t = ui.tokens();
        let gap = t.section_gap;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Add sources")
                    .strong()
                    .color(to_color32(sem.text.primary)),
            );
            ui.add_space(t.control_gap);
            ui.label(
                egui::RichText::new(where_shown(&sheet.browser))
                    .font(crate::protocol::mono_font())
                    .color(to_color32(sem.text.muted)),
            )
            .on_hover_text(sheet.browser.dir().display().to_string());
        });
        if sheet.browser.glob().is_some() {
            glob_field(ui, sheet, glob_id, &fs);
        }
        ui.add_space(gap);

        let foot_height = 2.0 * t.rows[3] + gap;
        let body = egui::vec2(
            ui.available_width(),
            (ui.available_height() - foot_height).max(t.rows[3] * 4.0),
        );
        let clicked = ui
            .allocate_ui_with_layout(body, egui::Layout::left_to_right(egui::Align::Min), |ui| {
                columns(ui, sheet, body, Some(&wake), &fs)
            })
            .inner;
        if let Some(e) = clicked {
            effect = e;
        }
        ui.add_space(gap);
        if let Some(note) = &sheet.browser.note {
            ui.label(egui::RichText::new(note).color(to_color32(sem.text.secondary)));
        }
        if let Some(e) = foot(ui, sheet, &fs) {
            effect = e;
        }
    });
    sheet.drawn.sheet = Some(shown.response.rect);
    // The browser answers `Tab` and the arrows itself, so egui's own focus is kept off the
    // sheet's rows and buttons, where those keys would move it and `Space` would click it. The
    // glob's field keeps it while the glob is being typed.
    let focused = ui.ctx().memory(egui::Memory::focused);
    if let Some(id) = focused.filter(|id| *id != glob_id || !sheet.glob_typing) {
        ui.ctx().memory_mut(|m| m.surrender_focus(id));
    }
    effect
}

/// Where the list is, as the header names it: the place it is in and the path below that place,
/// such as `Documents / datasets / shop`, or the whole path where no place holds it.
fn where_shown(browser: &Browser) -> String {
    let dir = browser.dir();
    match browser
        .places
        .get(browser.place)
        .filter(|p| dir.starts_with(&p.path))
    {
        Some(place) => {
            let below: Vec<String> = dir
                .strip_prefix(&place.path)
                .map(|rest| {
                    rest.components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            std::iter::once(place.label.clone())
                .chain(below)
                .collect::<Vec<_>>()
                .join(" / ")
        }
        None => dir.display().to_string(),
    }
}

/// The registry's keystroke for an egui key pressed with no modifier, as the Browser context
/// spells it.
pub(crate) fn token(key: egui::Key) -> Option<&'static str> {
    use egui::Key;
    Some(match key {
        Key::J => "j",
        Key::K => "k",
        Key::H => "h",
        Key::L => "l",
        Key::A => "a",
        Key::G => "g",
        Key::T => "t",
        Key::V => "v",
        Key::Space => "space",
        Key::Tab => "tab",
        Key::Enter => "enter",
        Key::Escape => "escape",
        Key::Slash => "/",
        Key::ArrowDown => "down",
        Key::ArrowUp => "up",
        Key::ArrowLeft => "left",
        Key::ArrowRight => "right",
        _ => return None,
    })
}

/// Answer the frame's keystrokes with the browser's verbs, in the order they were pressed, and
/// take each one answered out of the input. The first that asks something of the window ends the
/// frame's keys.
fn browser_keys(ui: &mut egui::Ui, sheet: &mut Sheet, fs: &dyn Listing) -> Effect {
    let pressed: Vec<(usize, &'static str)> = ui.input(|i| {
        i.events
            .iter()
            .enumerate()
            .filter_map(|(at, e)| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.is_none() => token(*key).map(|t| (at, t)),
                _ => None,
            })
            .collect()
    });
    if pressed.is_empty() {
        return Effect::None;
    }
    let mut effect = Effect::None;
    let mut taken = Vec::new();
    let was_open = sheet.browser.glob().is_some();
    let mut slash = false;
    for (at, keystroke) in pressed {
        taken.push(at);
        slash |= keystroke == "/";
        effect = sheet.browser.press(keystroke, fs);
        if effect != Effect::None {
            break;
        }
    }
    if slash && sheet.browser.glob().is_some() {
        // `/` opened the glob, or went back to it: its field takes the keys from the next frame,
        // so the `/` itself is not typed into it.
        if !was_open {
            sheet.glob_text.clear();
        }
        sheet.glob_typing = true;
    }
    let mut index = 0;
    ui.input_mut(|i| {
        i.events.retain(|_| {
            let keep = !taken.contains(&index);
            index += 1;
            keep
        });
    });
    ui.ctx().request_repaint();
    effect
}

/// While the glob's field holds the keys: `Enter` gives the keys back to the list with the glob
/// in force, and `Esc` lets the glob go; other keys type into the field.
fn glob_keys(ui: &mut egui::Ui, sheet: &mut Sheet, fs: &dyn Listing) -> Effect {
    let (enter, escape) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
        )
    });
    if enter || escape {
        sheet.glob_typing = false;
    }
    if escape {
        sheet.browser.focus = Pane::List;
        // `h` on the list lets a glob go, as it does when the glob is not being typed.
        return sheet.browser.key(GO_UP, fs);
    }
    Effect::None
}

/// The glob's field under the header: `/` and the text typed, which lists what it matches as it
/// changes, and how many files that is. A click on the field gives it the keys.
fn glob_field(ui: &mut egui::Ui, sheet: &mut Sheet, glob_id: egui::Id, fs: &dyn Listing) {
    let sem = semantic(ui.visuals().dark_mode);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("/")
                .font(crate::protocol::mono_font())
                .color(to_color32(sem.text.secondary)),
        );
        let field = ui.add(
            egui::TextEdit::singleline(&mut sheet.glob_text)
                .id(glob_id)
                .font(crate::protocol::mono_font())
                .hint_text("*.csv, or **/*.parquet for every folder below")
                .desired_width(ui.available_width() * 0.6),
        );
        if field.clicked() {
            sheet.glob_typing = true;
        }
        if sheet.glob_typing && !field.has_focus() {
            field.request_focus();
        }
        changed = field.changed();
        if sheet.browser.glob_stopped() {
            ui.label(
                egui::RichText::new("stopped looking; type a narrower glob")
                    .small()
                    .color(to_color32(sem.text.muted)),
            );
        } else if sheet.browser.active_glob().is_some() {
            let n = sheet.browser.entries().len();
            let files = if n == 1 { "file" } else { "files" };
            ui.label(
                egui::RichText::new(format!("{n} {files}"))
                    .small()
                    .color(to_color32(sem.text.muted)),
            );
        }
    });
    if changed {
        let text = sheet.glob_text.clone();
        sheet.browser.set_glob(&text, fs);
    }
}

/// Put dovetail's proposal for the folder under the cursor on its canvas, once it has answered,
/// starting the proposal when it has not been asked for. A folder previewed as its files is not
/// asked about.
fn sync_joins(sheet: &mut Sheet, wake: Option<&egui::Context>) {
    if sheet.browser.preview != PreviewMode::Canvas {
        return;
    }
    let Some(folder) = sheet.browser.canvas_folder().map(Path::to_path_buf) else {
        return;
    };
    if let Some(Ok(joins)) = sheet.previews.joins.get(&folder, wake) {
        let joins = joins.clone();
        sheet.browser.set_joins(&folder, joins);
    }
}

/// The places, the list and the preview side by side, or the preview alone while it fills the
/// browser. Answers what a click asked.
fn columns(
    ui: &mut egui::Ui,
    sheet: &mut Sheet,
    body: egui::Vec2,
    wake: Option<&egui::Context>,
    fs: &dyn Listing,
) -> Option<Effect> {
    let gap = ui.tokens().section_gap;
    let mut effect = None;
    if sheet.browser.full {
        ui.allocate_ui(body, |ui| {
            ui.set_min_size(body);
            if let Some(e) = preview(ui, sheet, wake, fs) {
                effect = Some(e);
            }
        });
        return effect;
    }
    // The columns sit edge to edge but for the gaps, so their widths add up to the body's.
    ui.spacing_mut().item_spacing.x = 0.0;
    let places_w = (body.x * 0.18).max(120.0);
    let list_w = (body.x * 0.32).max(180.0);
    let preview_w = (body.x - places_w - list_w - 2.0 * gap).max(120.0);
    column(ui, egui::vec2(places_w, body.y), |ui| places(ui, sheet, fs));
    ui.add_space(gap);
    if let Some(e) = column(ui, egui::vec2(list_w, body.y), |ui| list(ui, sheet, fs)) {
        effect = Some(e);
    }
    ui.add_space(gap);
    if let Some(e) = column(ui, egui::vec2(preview_w, body.y), |ui| {
        preview(ui, sheet, wake, fs)
    }) {
        effect = Some(e);
    }
    effect
}

/// One column of `size`, on the sunken surface with its hairline, its rows from the top.
fn column<R>(ui: &mut egui::Ui, size: egui::Vec2, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let sem = semantic(ui.visuals().dark_mode);
    let t = ui.tokens();
    let hairline = 1.0;
    // The frame adds its margin and its hairline outside what it holds, on each side.
    let outside = 2.0 * (t.space[2] + hairline);
    ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
        egui::Frame::new()
            .fill(to_color32(sem.surfaces.raised))
            .stroke(egui::Stroke::new(hairline, to_color32(sem.borders.subtle)))
            .inner_margin(egui::Margin::same(t.space[2] as i8))
            .show(ui, |ui| {
                let inner = size - egui::Vec2::splat(outside);
                ui.set_min_size(inner);
                ui.set_max_size(inner);
                ui.spacing_mut().item_spacing.x = t.space[1];
                body(ui)
            })
            .inner
    })
    .inner
}

/// The places column. A click on a place goes there, as `l` on it does.
fn places(ui: &mut egui::Ui, sheet: &mut Sheet, fs: &dyn Listing) -> Option<Effect> {
    let sem = semantic(ui.visuals().dark_mode);
    let mut chosen = None;
    let focused = sheet.browser.focus == Pane::Places;
    for (at, place) in sheet.browser.places.iter().enumerate() {
        let row = list_row(
            ui,
            ListRow::new(RowHeight::Grid).selected(focused && at == sheet.browser.place),
            |ui, _| {
                let ink = if sheet.browser.dir().starts_with(&place.path) {
                    sem.text.primary
                } else {
                    sem.text.secondary
                };
                ui.label(egui::RichText::new(&place.label).color(to_color32(ink)));
            },
        );
        sheet.drawn.places.push(row.response.rect);
        if row.clicked() {
            chosen = Some(at);
        }
    }
    let at = chosen?;
    sheet.browser.focus = Pane::Places;
    sheet.browser.place = at;
    Some(sheet.browser.key(GO_IN_OR_ADD, fs))
}

/// The list column: the folder's entries or the glob's matches, the cursor's row marked, the
/// marked files ticked. A click moves the cursor to the row; a double-click is `Enter` on it.
fn list(ui: &mut egui::Ui, sheet: &mut Sheet, fs: &dyn Listing) -> Option<Effect> {
    let sem = semantic(ui.visuals().dark_mode);
    let entries: Vec<Entry> = sheet.browser.entries().to_vec();
    if entries.is_empty() {
        let said = if sheet.browser.active_glob().is_some() {
            "No data files below this folder match the glob."
        } else {
            "This folder is empty."
        };
        ui.label(egui::RichText::new(said).color(to_color32(sem.text.muted)));
        return None;
    }
    let cursor = sheet.browser.current().map(|e| e.path.clone());
    let at_cursor = entries
        .iter()
        .position(|e| Some(&e.path) == cursor.as_ref());
    let scroll_now = at_cursor.is_some_and(|at| {
        sheet.scrolled_to.as_ref() != Some(&(sheet.browser.dir().to_path_buf(), at))
    });
    let list_focused = sheet.browser.focus == Pane::List;
    let mut clicked = None;
    let mut double = false;
    let glob = sheet.browser.active_glob().is_some();
    let dir = sheet.browser.dir().to_path_buf();
    egui::ScrollArea::vertical()
        .id_salt("bf-sources-browser-list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (at, entry) in entries.iter().enumerate() {
                let under = Some(at) == at_cursor;
                let marked = sheet.browser.marks.contains(&entry.path);
                let row = list_row(
                    ui,
                    ListRow::new(RowHeight::Grid).selected(under && list_focused),
                    |ui, _| {
                        let mark = if marked { "\u{2713} " } else { "  " };
                        ui.label(
                            egui::RichText::new(mark)
                                .font(crate::protocol::mono_font())
                                .color(to_color32(sem.text.link)),
                        );
                        let name = if glob {
                            entry
                                .path
                                .strip_prefix(&dir)
                                .map_or_else(|_| entry.name(), |p| p.display().to_string())
                        } else {
                            entry.name()
                        };
                        let (shown, ink) = match entry.kind {
                            Kind::Folder => (format!("{name}/"), sem.text.primary),
                            Kind::Data => (name, sem.text.primary),
                            Kind::Other => (name, sem.text.muted),
                        };
                        let ink = if under && !list_focused {
                            sem.text.link
                        } else {
                            ink
                        };
                        ui.label(egui::RichText::new(shown).color(to_color32(ink)));
                    },
                );
                if under && scroll_now {
                    row.response.scroll_to_me(None);
                }
                sheet.drawn.rows.push(row.response.rect);
                if row.response.double_clicked() {
                    clicked = Some(at);
                    double = true;
                } else if row.clicked() {
                    clicked = Some(at);
                }
            }
        });
    if let Some(at) = at_cursor {
        sheet.scrolled_to = Some((dir, at));
    }
    let at = clicked?;
    sheet.browser.focus = Pane::List;
    sheet.browser.cursor = at;
    sheet.browser.join_cursor = 0;
    double.then(|| sheet.browser.key(GO_IN_OR_ADD, fs))
}

/// The preview column: what is under the list's cursor.
fn preview(
    ui: &mut egui::Ui,
    sheet: &mut Sheet,
    wake: Option<&egui::Context>,
    fs: &dyn Listing,
) -> Option<Effect> {
    let sem = semantic(ui.visuals().dark_mode);
    let muted = |s: &str| egui::RichText::new(s).color(to_color32(sem.text.muted));
    let Some(entry) = sheet.browser.current().cloned() else {
        sheet.drawn.preview = Shown::Nothing;
        return None;
    };
    let name = entry.name();
    match entry.kind {
        Kind::Folder if sheet.browser.active_glob().is_none() => match sheet.browser.preview {
            PreviewMode::Canvas => return canvas(ui, sheet, &entry.path, &name),
            PreviewMode::Files => {
                let files = sheet
                    .previews
                    .files
                    .entry(entry.path.clone())
                    .or_insert_with(|| fs.list(&entry.path))
                    .clone();
                ui.label(egui::RichText::new(format!("{name}/")).strong());
                let names: Vec<String> = files.iter().map(Entry::name).collect();
                egui::ScrollArea::vertical()
                    .id_salt("bf-sources-browser-files")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for file in &files {
                            let ink = if file.kind == Kind::Other {
                                sem.text.muted
                            } else {
                                sem.text.primary
                            };
                            ui.label(egui::RichText::new(file.name()).color(to_color32(ink)));
                        }
                    });
                sheet.drawn.preview = Shown::Files(names);
            }
        },
        // A glob lists data files alone, so a folder is not among its rows.
        Kind::Folder => sheet.drawn.preview = Shown::Nothing,
        Kind::Data => match sheet.previews.pages.get(&entry.path, wake) {
            None => {
                let said = format!("Reading the first rows of {name}\u{2026}");
                ui.label(muted(&said));
                sheet.drawn.preview = Shown::Waiting(said);
            }
            Some(Err(why)) => {
                let said = format!("{name} could not be previewed: {why}");
                ui.label(muted(&said));
                sheet.drawn.preview = Shown::Said(said);
            }
            Some(Ok(page)) if !page.tables.is_empty() => {
                let tables = page.tables.clone();
                ui.label(egui::RichText::new(&name).strong());
                ui.label(muted(&format!(
                    "Holds {} {}, each added as a source:",
                    tables.len(),
                    if tables.len() == 1 { "table" } else { "tables" }
                )));
                for table in &tables {
                    ui.label(
                        egui::RichText::new(table)
                            .font(crate::protocol::mono_font())
                            .color(to_color32(sem.text.primary)),
                    );
                }
                sheet.drawn.preview = Shown::Tables(tables);
            }
            Some(Ok(page)) => {
                let page = page.clone();
                rows_table(ui, &name, &page);
                sheet.drawn.preview = Shown::Rows {
                    columns: page.columns.clone(),
                    rows: page.rows.len(),
                };
            }
        },
        Kind::Other => {
            let said = format!(
                "{name} is not a file Brightfield reads. It reads CSV, TSV and Parquet files, \
                 Excel workbooks, and DuckDB and SQLite databases."
            );
            ui.label(muted(&said));
            sheet.drawn.preview = Shown::Said(said);
        }
    }
    None
}

/// A file's first page: its name, then a grid of its columns and first rows.
fn rows_table(ui: &mut egui::Ui, name: &str, page: &Page) {
    let sem = semantic(ui.visuals().dark_mode);
    ui.label(egui::RichText::new(name).strong());
    ui.label(
        egui::RichText::new(format!(
            "The first {} rows, read where the file is",
            page.rows.len()
        ))
        .small()
        .color(to_color32(sem.text.muted)),
    );
    let cell = |s: &str| {
        let shown: String = if s.chars().count() > 40 {
            s.chars()
                .take(39)
                .chain(std::iter::once('\u{2026}'))
                .collect()
        } else {
            s.to_string()
        };
        egui::RichText::new(shown).font(crate::protocol::mono_font())
    };
    egui::ScrollArea::both()
        .id_salt("bf-sources-browser-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let t = ui.tokens();
            egui::Grid::new("bf-sources-browser-rows-grid")
                .striped(true)
                .spacing([t.space[5], t.space[1]])
                .show(ui, |ui| {
                    for column in &page.columns {
                        ui.label(cell(column).color(to_color32(sem.rows.header_foreground)));
                    }
                    ui.end_row();
                    for row in &page.rows {
                        for value in row {
                            ui.label(cell(value).color(to_color32(sem.text.primary)));
                        }
                        ui.end_row();
                    }
                });
        });
}

/// How a join reads on its row: `orders.customer_id → customers.id`.
#[must_use]
pub fn join_label(join: &ProposedJoin) -> String {
    format!(
        "{}.{} \u{2192} {}.{}",
        join.left.0, join.left.1, join.right.0, join.right.1
    )
}

/// A folder's canvas: its tables, one curve in the gutter for each join dovetail proposes between
/// two of them, and a row per join saying whether it is accepted. The join under the canvas's
/// cursor is marked, and a click on a join's row accepts or drops it, as `Space` on it does.
fn canvas(ui: &mut egui::Ui, sheet: &mut Sheet, folder: &Path, name: &str) -> Option<Effect> {
    let sem = semantic(ui.visuals().dark_mode);
    let muted = |s: &str| egui::RichText::new(s).color(to_color32(sem.text.muted));
    ui.label(egui::RichText::new(format!("{name}/")).strong());
    let joins: Vec<(ProposedJoin, bool)> = sheet.browser.canvas_joins().to_vec();
    let proposal = sheet.previews.joins.done.get(folder).cloned();
    match (&proposal, joins.is_empty()) {
        (None, _) => {
            let said = if sheet.previews.joins.running.is_some() {
                format!("dovetail is looking for joins between the tables in {name}\u{2026}")
            } else {
                format!("Looking for joins between the tables in {name}\u{2026}")
            };
            ui.label(muted(&said));
            sheet.drawn.preview = Shown::Waiting(said);
            return None;
        }
        (Some(Err(why)), _) => {
            let said = format!("No joins are proposed for {name}: {why}");
            ui.label(muted(&said));
            sheet.drawn.preview = Shown::Said(said);
            return None;
        }
        (Some(Ok(_)), true) => {
            let said = format!(
                "dovetail proposes no joins between the tables in {name}. Press a to add it, or \
                 v to see its files."
            );
            ui.label(muted(&said));
            sheet.drawn.preview = Shown::Canvas(Vec::new());
            return None;
        }
        (Some(Ok(_)), false) => {}
    }

    let canvas: Vec<CanvasJoin> = joins
        .iter()
        .map(|(join, accepted)| CanvasJoin {
            left_table: join.left.0.clone(),
            right_table: join.right.0.clone(),
            label: join_label(join),
            accepted: *accepted,
        })
        .collect();
    let on_canvas = sheet.browser.focus == Pane::Canvas;
    let drawn = draw_join_canvas(
        ui,
        &canvas,
        sheet.browser.join_cursor,
        on_canvas,
        "accepted",
    );
    sheet.drawn.joins.extend(drawn.rows);
    sheet.drawn.preview = Shown::Canvas(drawn.labels);
    if let Some(at) = drawn.clicked {
        sheet.browser.focus = Pane::Canvas;
        sheet.browser.join_cursor = at;
        sheet.browser.toggle_join(at);
    }
    None
}

/// One join as a join canvas draws it: the two tables it joins, how its row reads, and whether
/// it is ticked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasJoin {
    /// The table the join reads from.
    pub left_table: String,
    /// The table the join reads into.
    pub right_table: String,
    /// How its row reads, such as `orders.customer_id → customers.id`.
    pub label: String,
    /// Whether it is ticked.
    pub accepted: bool,
}

/// What [`draw_join_canvas`] drew: each join's row, what each row says, and the row clicked.
#[derive(Debug, Clone, Default)]
pub struct CanvasDrawn {
    /// Each join's row, in the order given.
    pub rows: Vec<egui::Rect>,
    /// Each row as it is spoken: its label, then `accepted_word` or `proposed`.
    pub labels: Vec<String>,
    /// The row clicked this frame, if one was.
    pub clicked: Option<usize>,
}

/// **Draw a join canvas**: the joins' tables, each a box down the left in the order the joins
/// first name them, one curve in the gutter per join, and a row per join with its tick, its label
/// and `accepted_word` or `proposed`. The join at `cursor` is marked while `focused`. The sources
/// browser's canvas and the organise sheet both draw with it.
pub fn draw_join_canvas(
    ui: &mut egui::Ui,
    joins: &[CanvasJoin],
    cursor: usize,
    focused: bool,
    accepted_word: &str,
) -> CanvasDrawn {
    let sem = semantic(ui.visuals().dark_mode);
    let t = ui.tokens();
    let mut tables: Vec<String> = Vec::new();
    for join in joins {
        for table in [&join.left_table, &join.right_table] {
            if !tables.contains(table) {
                tables.push(table.clone());
            }
        }
    }
    let row_h = t.rows[1];
    let pitch = row_h + t.space[2];
    let box_w = (ui.available_width() * 0.4).clamp(96.0, 220.0);
    // Each join's curve reaches further into the gutter than the one before, so two joins
    // between the same tables are told apart.
    let reach_of = |at: usize| t.space[5] + t.space[3] * at as f32;
    let height = pitch * tables.len() as f32;
    let (area, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(area.expand(2.0));
    let centre_of = |table: &str| -> f32 {
        let at = tables.iter().position(|t| t == table).unwrap_or(0);
        area.top() + pitch * at as f32 + row_h / 2.0
    };
    for (at, table) in tables.iter().enumerate() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(area.left(), area.top() + pitch * at as f32),
            egui::vec2(box_w, row_h),
        );
        painter.rect(
            rect,
            t.radius_control,
            to_color32(sem.surfaces.sunken),
            egui::Stroke::new(1.0, to_color32(sem.borders.default_)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.left_center() + egui::vec2(t.space[2], 0.0),
            egui::Align2::LEFT_CENTER,
            table,
            crate::protocol::mono_font(),
            to_color32(sem.text.primary),
        );
    }
    let cursor = cursor.min(joins.len().saturating_sub(1));
    for (at, join) in joins.iter().enumerate() {
        let (a, b) = (centre_of(&join.left_table), centre_of(&join.right_table));
        let x = area.left() + box_w;
        let reach = reach_of(at);
        let ink = if join.accepted {
            sem.text.link
        } else {
            sem.text.muted
        };
        let width = if focused && at == cursor { 2.5 } else { 1.25 };
        let curve = egui::epaint::CubicBezierShape::from_points_stroke(
            [
                egui::pos2(x, a),
                egui::pos2(x + reach, a),
                egui::pos2(x + reach, b),
                egui::pos2(x, b),
            ],
            false,
            egui::Color32::TRANSPARENT,
            egui::Stroke::new(width, to_color32(ink)),
        );
        painter.add(curve);
    }

    ui.add_space(t.section_gap);
    let mut drawn = CanvasDrawn::default();
    for (at, join) in joins.iter().enumerate() {
        let word = if join.accepted {
            accepted_word
        } else {
            "proposed"
        };
        let row = list_row(
            ui,
            ListRow::new(RowHeight::Grid).selected(focused && at == cursor),
            |ui, _| {
                let (mark, ink) = if join.accepted {
                    ("\u{2611} ", sem.text.link)
                } else {
                    ("\u{2610} ", sem.text.secondary)
                };
                ui.label(
                    egui::RichText::new(mark)
                        .font(crate::protocol::mono_font())
                        .color(to_color32(ink)),
                );
                ui.label(
                    egui::RichText::new(&join.label)
                        .font(crate::protocol::mono_font())
                        .color(to_color32(sem.text.primary)),
                );
                ui.label(
                    egui::RichText::new(word)
                        .small()
                        .color(to_color32(sem.text.muted)),
                );
            },
        );
        drawn.rows.push(row.response.rect);
        drawn.labels.push(format!("{} {word}", join.label));
        if row.clicked() {
            drawn.clicked = Some(at);
        }
    }
    drawn
}

/// The keys the focused column answers, each with what it does there: what the foot shows.
fn foot_keys(sheet: &Sheet) -> Vec<(&'static str, &'static str)> {
    let folder_under = sheet.browser.canvas_folder().is_some();
    match sheet.browser.focus {
        Pane::Places => vec![
            (MOVE_DOWN, "next place"),
            (MOVE_UP, "place above"),
            (GO_IN_OR_ADD, "go there"),
            (NEXT_PANE, "next column"),
            (CLOSE, "close"),
        ],
        Pane::List => {
            let mut keys = vec![
                (MOVE_DOWN, "next"),
                (MOVE_UP, "above"),
                (GO_UP, "up a folder"),
                (GO_IN_OR_ADD, "open or add"),
                (ADD, "add"),
                (MARK_OR_ACCEPT, "mark"),
                (TYPE_GLOB, "glob"),
                (NEXT_PANE, "next column"),
                (FILL, "fill"),
            ];
            if folder_under {
                keys.push((SWITCH_PREVIEW, "canvas or files"));
            }
            keys.push((CLOSE, "close"));
            keys
        }
        Pane::Canvas => vec![
            (MOVE_DOWN, "next join"),
            (MOVE_UP, "join above"),
            (MARK_OR_ACCEPT, "accept or drop"),
            (GO_IN_OR_ADD, "add the folder"),
            (GO_UP, "back to the list"),
            (NEXT_PANE, "next column"),
            (FILL, "fill"),
            (CLOSE, "close"),
        ],
    }
}

/// The keystrokes the Browser context binds `verb` to, as keycaps show them, leaving out the
/// arrows, which twin `h` `j` `k` `l` for an analyst who has not learnt them and would crowd the
/// foot: `l Enter` for the verb that goes in or adds.
fn keys_of(verb: &str) -> String {
    keymap_bindings(&registry())
        .iter()
        .filter(|b| b.context == BindingContext::Browser && b.longname == verb)
        .map(|b| b.keystrokes)
        .filter(|k| !["down", "up", "left", "right"].contains(k))
        .map(|k| match k {
            "space" => "Space".to_string(),
            "escape" => "Esc".to_string(),
            "enter" => "Enter".to_string(),
            "tab" => "Tab".to_string(),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The foot: the keys of the focused column, each a keycap and what it does, read from the
/// registry so a key moved there moves here. A click on one does what its key does.
fn foot(ui: &mut egui::Ui, sheet: &mut Sheet, fs: &dyn Listing) -> Option<Effect> {
    let sem = semantic(ui.visuals().dark_mode);
    let t = ui.tokens();
    let keys = foot_keys(sheet);
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        for (at, (verb, what)) in keys.iter().enumerate() {
            let shown = keys_of(verb);
            let pair = ui
                .horizontal(|ui| {
                    key_chip(ui, &shown);
                    ui.add_space(t.space[1]);
                    ui.label(
                        egui::RichText::new(*what)
                            .small()
                            .color(to_color32(sem.text.muted)),
                    );
                })
                .response;
            let hit = ui.interact(
                pair.rect,
                ui.id().with(("bf-sources-browser-foot", at)),
                egui::Sense::click(),
            );
            sheet.drawn.foot.push(format!("{shown} {what}"));
            sheet.drawn.foot_rects.push(pair.rect);
            if hit.clicked() {
                clicked = Some(*verb);
            }
            ui.add_space(t.control_gap);
        }
    });
    let verb = clicked?;
    let was_open = sheet.browser.glob().is_some();
    let effect = sheet.browser.key(verb, fs);
    if verb == TYPE_GLOB {
        if !was_open {
            sheet.glob_text.clear();
        }
        sheet.glob_typing = true;
    }
    Some(effect)
}

/// **The first page of the data file at `path`**, read with DuckDB on a connection of its own, as
/// the grid's previews are, under their limits on size and time. A workbook or a database lists
/// its tables instead.
///
/// # Errors
///
/// The reason the file could not be read, in DuckDB's words or the preview's limits.
pub fn read_first_page(path: &Path) -> Result<Page, String> {
    let Some(reader) = brightfield_engine::FileReader::for_path(path) else {
        let found = brightfield_engine::sources::discover(path)?;
        return Ok(Page {
            tables: found.tables.into_iter().map(|t| t.name).collect(),
            ..Page::default()
        });
    };
    let batches = brightfield_engine::preview_first_page(path, reader, "preview", &[], PAGE_ROWS)
        .map_err(|e| e.to_string())?;
    let mut page = Page::default();
    for batch in &batches {
        if page.columns.is_empty() {
            page.columns = batch
                .schema()
                .fields()
                .iter()
                .map(|f| f.name().clone())
                .collect();
        }
        for row in 0..batch.num_rows() {
            page.rows.push(
                batch
                    .columns()
                    .iter()
                    .map(|column| {
                        duckdb::arrow::util::display::array_value_to_string(column, row)
                            .unwrap_or_default()
                    })
                    .collect(),
            );
        }
    }
    Ok(page)
}

/// **The joins dovetail proposes between the tables at the top of `folder`**, by its sample pass
/// over a scratch Protocol of the folder made in the system's temporary folder, run with
/// `runner` and taken away again with arc's working database for it. A folder with fewer than two
/// data files at its top has nothing to join; one with more than [`PREVIEW_TABLE_LIMIT`] of them,
/// or whose data files come to more than a preview reads of one file
/// ([`brightfield_engine::PREVIEW_FILE_LIMIT_BYTES`]), is not read for a preview, and says so.
///
/// # Errors
///
/// The reason the folder's tables could not be loaded, or dovetail's pass failed.
pub fn propose_joins(
    folder: &Path,
    runner: &crate::run::Runner,
) -> Result<Vec<ProposedJoin>, String> {
    let files: Vec<Entry> = Disk
        .list(folder)
        .into_iter()
        .filter(|e| e.kind == Kind::Data)
        .collect();
    let data = files.len();
    if data < 2 {
        return Ok(Vec::new());
    }
    if data > PREVIEW_TABLE_LIMIT {
        return Err(format!(
            "it holds {data} data files, more than a preview reads; add it to see the joins \
             dovetail proposes in the model"
        ));
    }
    // The scratch Protocol loads each table whole, so a folder larger than one file's preview may
    // be is left to the add, which loads it once.
    let bytes: u64 = files
        .iter()
        .filter_map(|e| std::fs::metadata(&e.path).ok())
        .map(|m| m.len())
        .sum();
    if bytes > brightfield_engine::PREVIEW_FILE_LIMIT_BYTES {
        return Err(
            "its data files are too large to read for a preview; add it to see the joins \
             dovetail proposes in the model"
                .to_string(),
        );
    }
    let scratch =
        std::env::temp_dir().join(format!("brightfield-folder-preview-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let mut protocol = crate::own_files::create_scratch_in(&scratch, folder)?;
    let dir = protocol.dir.clone();
    let proposed = (|| -> Result<Vec<ProposedJoin>, String> {
        // The scratch Protocol's database is its own, so the run takes no lock the window's
        // writes to a real Protocol wait on.
        let finished = crate::run::Run::begin(runner, &protocol.dir, || {}).wait();
        if !finished.succeeded {
            return Err(finished.log);
        }
        crate::own_files::profile(&mut protocol)?;
        crate::own_files::write_packages(&protocol)?;
        let dovetail = crate::discovery::Dovetail::new(runner);
        let model = crate::discovery::sample_pass(&dovetail, &protocol, None)?;
        Ok(model
            .relationships
            .iter()
            .filter_map(
                |r| match (r.from_columns.as_slice(), r.to_columns.as_slice()) {
                    ([from], [to]) => Some(ProposedJoin::new((&r.from, from), (&r.to, to))),
                    _ => None,
                },
            )
            .collect())
    })();
    if let Ok(db) = arc::spec::protocol_db_path(&dir) {
        let _ = std::fs::remove_file(&db);
        let mut wal = db.into_os_string();
        wal.push(".wal");
        let _ = std::fs::remove_file(wal);
    }
    let _ = std::fs::remove_dir_all(&dir);
    // The per-process folder too, once no other preview's Protocol is in it.
    let _ = std::fs::remove_dir(&scratch);
    proposed
}
