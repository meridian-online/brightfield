//! **A rebind by key and the same rebind by click save the same file, and
//! reopening draws both.**
//!
//! The analyst puts `median_income` on x and `median_house_value` on colour —
//! by the shelf band's keys (`e`, a channel's letter, the list's `j` and `k`,
//! `Enter`), or by the pointer (a click on the band's cell, a click on the
//! list's row) — and Saves. The chart file holds the same bytes whichever
//! route made the edits, so its diff shows what the analyst did and not how
//! they did it. Closing the window and opening the Protocol again draws the
//! hero with both columns on their channels, and the band names both.
//!
//! Every read is of what a window drew or what Save wrote: the hero's
//! composition (the columns its top layer encodes), the text a frame put in the
//! band's cells, and the chart file's bytes on disk.

use std::path::PathBuf;

use brightfield_protocol::layout::Flow;
use brightfield_render::channel::Channel;
use brightfield_shell::app::CHART;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::PlotHandle;
use brightfield_shell::protocol::SpineRole;
use brightfield_shell::startup::default_layout;
use brightfield_shell::text_ink;
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_workbench::channel::ShelfChannel;
use brightfield_workbench::PaneKey;

const HOUSING_FILE: &str = "california_housing_sample.csv";
const INCOME: &str = "median_income";
const VALUE: &str = "median_house_value";

fn housing() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(HOUSING_FILE)
}

/// A directory of this test's own, removed when the test ends: a Save writes
/// beside the data file, and the data file is copied here so that is not the
/// repository.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-shelf-save-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn key_down(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn button(at: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// One headless window over a copy of the housing sample, the hero's pane
/// focused.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    /// The data file's folder, where Save writes `arcform.yaml` and `panels/`.
    folder: PathBuf,
    /// The text the frame before drew.
    texts: Vec<text_ink::DrawnText>,
    /// Held last, so the folder outlives the window.
    _root: TempDir,
}

impl Window {
    /// A window over a copy of the sample in a folder of `name`'s own.
    fn housing(name: &str) -> Self {
        Self::housing_over(TempDir::new(name))
    }

    /// A window over a copy of the sample in `root`'s own folder.
    fn housing_over(root: TempDir) -> Self {
        let folder = root.0.join("data");
        std::fs::create_dir_all(&folder).expect("the data file's folder");
        let data = folder.join(HOUSING_FILE);
        std::fs::copy(housing(), &data).expect("the housing fixture copies");
        let boot = Boot::data_file(data.to_str().expect("utf-8 path")).expect("the sample opens");
        Self::around(boot, folder, root)
    }

    fn around(boot: Boot, folder: PathBuf, root: TempDir) -> Self {
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
            folder,
            texts: Vec::new(),
            _root: root,
        };
        win.settle();
        assert!(
            win.app.focus_pane(PaneKey::new(CHART)),
            "the dashboard's pane takes focus"
        );
        win.settle();
        win
    }

    /// Close this window and keep the folder it worked in.
    fn into_root(self) -> TempDir {
        self._root
    }

    /// Close this window and open the Protocol it saved, into a new one: the
    /// route a command line and a front-door row both reach.
    fn reopen(self) -> Self {
        let manifest = self
            .folder
            .join("arcform.yaml")
            .to_str()
            .expect("utf-8 path")
            .to_owned();
        let Self {
            folder,
            _root: root,
            ..
        } = self;
        let boot = Boot::open(&manifest, Flow::Vertical, None)
            .unwrap_or_else(|e| panic!("reopen {manifest}: {e}"));
        Self::around(boot, folder, root)
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            events,
            ..Default::default()
        };
        let mut texts = Vec::new();
        let _ = self.ctx.run_ui(raw, |ui| {
            self.app.draw(ui);
            texts = text_ink::frame_text(ui.ctx());
        });
        self.texts = texts;
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    /// Press a letter, as a keyboard brings it, and let the frame after run.
    fn type_letter(&mut self, key: egui::Key, text: &str) {
        self.run(vec![key_down(key), egui::Event::Text(text.to_owned())]);
        self.run(Vec::new());
    }

    fn press(&mut self, key: egui::Key) {
        self.run(vec![key_down(key)]);
        self.run(Vec::new());
    }

    /// A click at `at`: the pointer moved there, pressed, released, and the
    /// frames after it settled.
    fn click(&mut self, at: egui::Pos2) {
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.run(vec![button(at, true)]);
        self.run(vec![button(at, false)]);
        self.settle();
    }

    /// The column under the open list's cursor.
    fn cursor(&self) -> Option<String> {
        self.app
            .protocol_model()
            .column_list()
            .and_then(|l| l.cursor().map(str::to_owned))
    }

    /// Move the open list's cursor to `column` by its keys, `k` up the rows or
    /// `j` down, as many presses as it takes.
    fn walk_to(&mut self, column: &str, key: egui::Key, text: &str) {
        for _ in 0..12 {
            if self.cursor().as_deref() == Some(column) {
                return;
            }
            self.type_letter(key, text);
        }
        panic!(
            "the list's cursor did not reach {column} by `{text}`: it is on {:?}",
            self.cursor()
        );
    }

    /// The hero's plot, as the page composed it.
    fn hero(&self) -> &PlotHandle {
        self.app
            .chart_doc()
            .composed
            .plots
            .first()
            .expect("the page composed its hero")
    }

    /// The column the hero's top layer draws on `channel`, as the composition
    /// that ran records it.
    fn hero_column(&self, channel: Channel) -> Option<String> {
        self.hero()
            .hover
            .as_ref()
            .and_then(|layer| layer.column(channel).map(str::to_owned))
    }

    /// Whether the window title carries the unsaved mark.
    fn marked_unsaved(&self) -> bool {
        self.app.title().contains(UNSAVED_MARK)
    }

    /// Whether `channel`'s cell of the band drew `column` as its column: whole,
    /// or cut to the cell's room and ended in an ellipsis, as the band draws a
    /// name longer than the room it has.
    fn cell_names(&self, channel: ShelfChannel, column: &str) -> bool {
        let cell = self.app.shelf_drawn().expect("the band drew").cells[channel.index()];
        self.texts
            .iter()
            .filter(|t| cell.contains_rect(t.visible) && t.visible.is_positive())
            .any(|t| {
                t.text == column
                    || (t.elided && {
                        let kept = t.text.trim_end_matches('\u{2026}');
                        kept.len() > column.len() / 2 && column.starts_with(kept)
                    })
            })
    }

    /// The text the last frame drew in `channel`'s cell of the band.
    fn cell_text(&self, channel: ShelfChannel) -> Vec<String> {
        let cell = self.app.shelf_drawn().expect("the band drew").cells[channel.index()];
        self.texts
            .iter()
            .filter(|t| cell.contains_rect(t.visible) && t.visible.is_positive())
            .map(|t| t.text.clone())
            .collect()
    }

    /// The Outline's row for `column` in the open list, as the frame drew it.
    fn list_row(&self, column: &str) -> egui::Rect {
        self.app
            .spine_rows()
            .iter()
            .find(|r| r.role == SpineRole::Column && r.label == column)
            .unwrap_or_else(|| panic!("the Outline's list drew no row for {column}"))
            .rect
    }

    /// **Save, through the gesture a person has**: the chart palette on
    /// `space`, the verb typed, confirmed with enter.
    fn save(&mut self) {
        assert!(
            self.app.has_protocol_to_save(),
            "this window has no Protocol behind it to save"
        );
        self.key_out_of_the_band();
        self.press(egui::Key::Space);
        assert_eq!(
            self.app.open_overlay(),
            Some("palette"),
            "space did not open the palette"
        );
        self.settle();
        self.run(vec![egui::Event::Text("save-spec".to_owned())]);
        self.run(Vec::new());
        self.press(egui::Key::Enter);
        assert_eq!(
            self.app.open_overlay(),
            None,
            "confirming save-spec did not close the palette"
        );
        self.settle();
    }

    /// `Esc` hands the band's keys back to the pane, as a person leaving the
    /// band does before reaching for the palette.
    fn key_out_of_the_band(&mut self) {
        self.press(egui::Key::Escape);
    }

    /// The chart file Save wrote under `panels/`, as bytes.
    fn chart_bytes(&self) -> Vec<u8> {
        let chart = std::fs::read_dir(self.folder.join("panels"))
            .expect("Save wrote a panels folder")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "yaml"))
            .expect("Save wrote a chart file");
        std::fs::read(&chart).unwrap_or_else(|e| panic!("read {}: {e}", chart.display()))
    }

    fn chart_text(&self) -> String {
        String::from_utf8(self.chart_bytes()).expect("the chart file is utf-8")
    }

    /// The text the generator wrote at open, which a first Save places its
    /// edits into.
    fn generated_text(&self) -> String {
        let path = self
            .app
            .chart_doc()
            .spec_path
            .clone()
            .expect("a generated dashboard carries its spec file");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read the generated spec {}: {e}", path.display()))
    }
}

/// A channel's letter, the list's cursor walked to `column` by `walk` (`k` up
/// the rows, `j` down), and `Enter`: the band's keys, with `e` already typed.
fn keep_by_keys(
    win: &mut Window,
    letter: (egui::Key, &str),
    column: &str,
    walk: (egui::Key, &str),
) {
    win.type_letter(letter.0, letter.1);
    win.walk_to(column, walk.0, walk.1);
    win.press(egui::Key::Enter);
}

/// The two rebinds by the band's keys, from the opened map: `median_income` is
/// above the map's `longitude` in the list and `median_house_value` below the
/// first row of colour's.
fn rebind_by_keys(win: &mut Window) {
    win.type_letter(egui::Key::E, "e");
    keep_by_keys(win, (egui::Key::X, "x"), INCOME, (egui::Key::K, "k"));
    keep_by_keys(win, (egui::Key::C, "c"), VALUE, (egui::Key::J, "j"));
}

/// The two rebinds by the pointer: a click on the band's cell opens its list,
/// a click on the list's row keeps the column.
fn rebind_by_pointer(win: &mut Window) {
    for (channel, column) in [(ShelfChannel::X, INCOME), (ShelfChannel::Colour, VALUE)] {
        let cell = win.app.shelf_drawn().expect("the band drew").cells[channel.index()];
        win.click(cell.center());
        assert_eq!(
            win.app.protocol_model().column_list().map(|l| l.channel()),
            Some(channel),
            "a click on the {} cell opens its list",
            channel.word()
        );
        let row = win.list_row(column);
        win.click(row.center());
        assert!(
            win.hero_column(channel_of(channel)).as_deref() == Some(column),
            "a click on {column}'s row did not keep it on {}",
            channel.word()
        );
    }
}

/// The renderer's channel a shelf channel draws on.
fn channel_of(channel: ShelfChannel) -> Channel {
    match channel {
        ShelfChannel::X => Channel::X,
        ShelfChannel::Colour => Channel::Fill,
        other => panic!("this file drives x and colour, not {}", other.word()),
    }
}

/// Both columns on their channels in the hero, and named in the band.
fn assert_both_drawn(win: &Window, when: &str) {
    assert_eq!(
        win.hero_column(Channel::X).as_deref(),
        Some(INCOME),
        "{when}: the hero's x"
    );
    assert_eq!(
        win.hero_column(Channel::Fill).as_deref(),
        Some(VALUE),
        "{when}: the hero's colour"
    );
    assert!(
        win.cell_names(ShelfChannel::X, INCOME),
        "{when}: the x cell does not name {INCOME}: it drew {:?}",
        win.cell_text(ShelfChannel::X)
    );
    assert!(
        win.cell_names(ShelfChannel::Colour, VALUE),
        "{when}: the colour cell does not name {VALUE}: it drew {:?}",
        win.cell_text(ShelfChannel::Colour)
    );
}

/// Make both rebinds by `route` in `win`, then Save.
fn rebind_and_save(win: &mut Window, route: fn(&mut Window)) {
    route(win);
    assert_both_drawn(win, "before Save");
    assert!(
        win.marked_unsaved(),
        "two kept columns left the title unmarked"
    );
    win.save();
    assert!(
        !win.marked_unsaved(),
        "a Save that wrote the chart left the mark"
    );
}

/// A window over a copy of the sample, rebound by `route` and saved.
fn saved_by(name: &str, route: fn(&mut Window)) -> Window {
    let mut win = Window::housing(name);
    rebind_and_save(&mut win, route);
    win
}

fn lines_with<'a>(text: &'a str, needle: &str) -> Vec<&'a str> {
    text.lines().filter(|l| l.contains(needle)).collect()
}

/// **AC1.** `median_income` on x and `median_house_value` on colour by keys,
/// then Save, writes a chart file holding both. The test counts the lines the
/// file carries: `x: 'median_income'` where the generated map had
/// `x: 'longitude'` and `fill: median_house_value` once.
#[test]
fn by_keys_median_income_on_x_and_median_house_value_on_colour_save_a_chart_file_holding_both() {
    let mut win = Window::housing("keys");
    let generated = win.generated_text();
    let layers = lines_with(&generated, "x: 'longitude'").len();
    assert!(layers > 0, "the generated map names no x of longitude");

    rebind_and_save(&mut win, rebind_by_keys);
    assert!(
        win.folder.join("arcform.yaml").exists(),
        "Save wrote no manifest"
    );
    let written = win.chart_text();
    assert_eq!(
        lines_with(&written, &format!("x: '{INCOME}'")).len(),
        layers,
        "the chart file's x is not {INCOME} on each layer the map's x was longitude on:\n{written}"
    );
    assert!(
        lines_with(&written, "x: 'longitude'").is_empty(),
        "the chart file still holds longitude on x:\n{written}"
    );
    assert_eq!(
        written
            .lines()
            .filter(|l| l.trim() == format!("fill: {VALUE}"))
            .count(),
        1,
        "the chart file does not hold one fill of {VALUE}:\n{written}"
    );
}

/// **AC2.** The same two edits by pointer, from the same start, then Save,
/// write a chart file with the same bytes as the keys' did. The start is one
/// data file: the pointer's window opens over the folder the keys' window saved
/// into, with what that Save wrote taken away, so the chart file's `file:` line
/// names one path in both.
#[test]
fn by_pointer_the_same_two_edits_save_the_same_bytes_as_by_keys() {
    let mut by_keys = Window::housing_over(TempDir::new("same"));
    rebind_and_save(&mut by_keys, rebind_by_keys);
    let keys = by_keys.chart_text();

    let root = by_keys.into_root();
    let data_folder = root.0.join("data");
    std::fs::remove_file(data_folder.join("arcform.yaml")).expect("Save wrote a manifest");
    std::fs::remove_dir_all(data_folder.join("panels")).expect("Save wrote a panels folder");

    let mut by_pointer = Window::housing_over(root);
    rebind_and_save(&mut by_pointer, rebind_by_pointer);
    let pointer = by_pointer.chart_text();

    if keys != pointer {
        let differ: Vec<(&str, &str)> = keys
            .lines()
            .zip(pointer.lines())
            .filter(|(a, b)| a != b)
            .take(12)
            .collect();
        panic!(
            "the pointer saved different bytes from the keys ({} against {} bytes): {differ:#?}",
            keys.len(),
            pointer.len()
        );
    }
}

/// **AC3.** Closing the window and reopening the saved Protocol draws the hero
/// with both columns, and its band names both.
#[test]
fn reopening_the_saved_protocol_draws_the_hero_with_both_and_the_band_names_both() {
    for (name, route) in [
        ("reopen-keys", rebind_by_keys as fn(&mut Window)),
        ("reopen-pointer", rebind_by_pointer as fn(&mut Window)),
    ] {
        let saved = saved_by(name, route);
        let written = saved.chart_bytes();
        let reopened = saved.reopen();
        assert_both_drawn(&reopened, &format!("{name}, reopened"));
        assert!(
            !reopened.marked_unsaved(),
            "{name}: the reopened window is marked unsaved before an edit"
        );
        assert_eq!(
            reopened.chart_bytes(),
            written,
            "{name}: reopening rewrote the chart file"
        );
    }
}
