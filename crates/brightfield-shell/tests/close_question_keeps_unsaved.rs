//! **A close without saving records the unsaved chart as a version, and the
//! question names its three answers.**
//!
//! Closing a window with an unsaved chart edit asked Save, Discard or Cancel,
//! said only that an edit was not saved, and Discard closed without keeping
//! anything. The question now names the chart file, counts the unsaved edits,
//! lists each in the shelf's words and says what closing without saving keeps;
//! its answers are *Save and close*, *Close without saving* and *Keep editing*,
//! on Enter, D and Esc. *Close without saving* records the chart as a Save would
//! have written it, as arcform's unsaved kind of version, and writes no file in
//! the data folder.
//!
//! Each test drives a window over the housing file by the gestures a person has
//! — clicks on the Outline's chips, keys, a close request raised as the
//! operating system raises it — and reads what the window painted, what it
//! sent the operating system, the files in the data folder and the store.
//! **No test writes under the home directory**: each window is given a store
//! inside its own temporary folder, outside the data folder.
//!
//! An edit is counted as the Versions panel words it: a column put on the
//! Map's x is two changes there, the projection the Map drew taken out and the
//! column moved, and two lines here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use brightfield_protocol::chart_history::{HistoryKind, LocalHistory};
use brightfield_protocol::HistoryStore;
use brightfield_shell::app::NO_STORE;
use brightfield_shell::design::Mode;
use brightfield_shell::overlays::{CloseQuestion, CLOSE_LIST_ROWS, CLOSE_QUESTION_TITLE};
use brightfield_shell::protocol::SpineRole;
use brightfield_shell::shelf::ShelfChannels;
use brightfield_shell::text_ink;
use brightfield_shell::versions::{Clock, Listing, CLOSED_UNSAVED};
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_spec::layout::ScaleType;
use brightfield_workbench::arrangement::LEDGER_RAIL;
use brightfield_workbench::channel::ShelfChannel;

const HOUSING_FILE: &str = "california_housing_sample.csv";

/// The chart file's name: the Protocol's name, which is the data file's stem.
const CHART_FILE: &str = "california_housing_sample.yaml";

/// The chart file as the question names it, relative to the Protocol's folder.
const CHART_NAMED: &str = "panels/california_housing_sample.yaml";

/// The overlay's name for the question, as `MeridianApp::open_overlay` says it.
const QUESTION: &str = "close-question";

/// The egui area the question's card is drawn in.
const QUESTION_AREA: &str = "bf-overlay-close-question";

/// The Versions panel's place in the ledger strip.
const VERSIONS_AT: usize = 4;

/// The local time the newest version reads under the pinned clock, 14:02:50.
const NEWEST_READS: i64 = 20_731 * 86_400 + 14 * 3_600 + 2 * 60 + 50;

/// *Now* under the pinned clock: three minutes after the newest version.
const NOW_READS: i64 = NEWEST_READS + 3 * 60;

/// What a column put on the Map's x reads as, once the window holds it.
const X_PUT: [&str; 2] = [
    "Map \u{b7} projection type: equirectangular removed",
    "Map \u{b7} x axis: longitude \u{2192} median_income",
];

fn housing() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv")
}

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-close-keeps-{name}-{}-{}",
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

/// What store a window is given.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Store {
    /// A store at `<root>/.arcform/history`, outside the data folder.
    Kept,
    /// No store at all.
    None,
    /// A store whose folder cannot be made, because a file stands where its
    /// parent should be.
    Unopenable,
}

/// Every file under `dir`, path relative to it, with its bytes.
fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, base: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).expect("a readable folder").flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, base, into);
            } else {
                into.insert(
                    path.strip_prefix(base)
                        .expect("under the folder")
                        .to_string_lossy()
                        .into_owned(),
                    std::fs::read(&path).expect("a readable file"),
                );
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(dir, dir, &mut found);
    found
}

/// A window over the housing file copied into a folder of its own, kept alive
/// with one `egui::Context`, because a click is resolved against the widget id
/// a *previous* frame registered.
struct Session {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    /// The data file's folder: where `arcform.yaml` and `panels/` land.
    folder: PathBuf,
    /// The root of the store the window records into, outside `folder`.
    history_root: PathBuf,
    /// The text the last frame drew.
    texts: Vec<text_ink::DrawnText>,
    /// Whether a frame since the last take sent `CancelClose`, and `Close`.
    cancelled_close: bool,
    closed: bool,
    root: std::rc::Rc<TempDir>,
}

impl Session {
    fn open(name: &str) -> Self {
        Self::open_as(name, Store::Kept)
    }

    fn open_as(name: &str, store: Store) -> Self {
        let root = std::rc::Rc::new(TempDir::new(name));
        let folder = root.0.join("data");
        std::fs::create_dir_all(&folder).expect("the data file's folder");
        std::fs::copy(housing(), folder.join(HOUSING_FILE)).expect("the housing fixture copies");
        let history_root = match store {
            Store::Unopenable => {
                std::fs::write(root.0.join("blocked"), "a file, not a folder")
                    .expect("the file that blocks the store");
                root.0.join("blocked").join("history")
            }
            Store::Kept | Store::None => root.0.join(".arcform").join("history"),
        };
        Self::over(root, folder, history_root, store != Store::None)
    }

    /// A second window over the same data file and the same store: what a
    /// person sees opening the file again.
    fn reopen(&self) -> Self {
        Self::over(
            self.root.clone(),
            self.folder.clone(),
            self.history_root.clone(),
            true,
        )
    }

    fn over(
        root: std::rc::Rc<TempDir>,
        folder: PathBuf,
        history_root: PathBuf,
        kept: bool,
    ) -> Self {
        let data = folder.join(HOUSING_FILE);
        let boot = Boot::data_file(data.to_str().expect("utf-8 path"))
            .unwrap_or_else(|e| panic!("open {}: {e}", data.display()));
        let app = MeridianApp::headless(boot, Mode::Light);
        let app = if kept {
            app.keeping_history(Some(HistoryStore::At(history_root.clone())))
        } else {
            app
        };
        let mut session = Self {
            app,
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
            folder,
            history_root,
            texts: Vec::new(),
            cancelled_close: false,
            closed: false,
            root,
        };
        session
            .app
            .set_versions_env(Clock::System, Some(session.root.0.clone()));
        session.settle();
        session
    }

    fn chart_file(&self) -> PathBuf {
        self.folder.join("panels").join(CHART_FILE)
    }

    fn arcform_file(&self) -> PathBuf {
        self.folder.join("arcform.yaml")
    }

    fn history(&self) -> LocalHistory {
        LocalHistory::at_root(&self.history_root)
    }

    /// The chart file's versions as the store holds them, oldest first: none
    /// before the file's folder is there.
    fn entries(&self) -> Vec<brightfield_protocol::chart_history::HistoryEntry> {
        if !self.chart_file().parent().is_some_and(Path::exists) {
            return Vec::new();
        }
        self.history()
            .entries_for_file(&self.chart_file())
            .expect("the chart file's history lists")
    }

    fn marked(&self) -> bool {
        self.app.title().contains(UNSAVED_MARK)
    }

    /// Run one frame with `events`, `close` raising a close request in it.
    fn frame(&mut self, events: Vec<egui::Event>, close: bool) {
        let mut raw = egui::RawInput {
            screen_rect: Some(self.screen),
            events,
            ..Default::default()
        };
        if close {
            raw.viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .events
                .push(egui::ViewportEvent::Close);
        }
        let mut texts = Vec::new();
        let out = self.ctx.run_ui(raw, |ui| {
            self.app.draw(ui);
            texts = text_ink::frame_text(ui.ctx());
        });
        if let Some(root) = out.viewport_output.get(&egui::ViewportId::ROOT) {
            for command in &root.commands {
                match command {
                    egui::ViewportCommand::CancelClose => self.cancelled_close = true,
                    egui::ViewportCommand::Close => self.closed = true,
                    _ => {}
                }
            }
        }
        self.texts = texts;
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        self.frame(events, false);
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    fn press(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }
    }

    fn key(&mut self, key: egui::Key) {
        self.run(vec![Self::press(key)]);
        self.settle();
    }

    fn point(&mut self, at: egui::Pos2) {
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.settle();
    }

    /// Press and release the primary button over `pos`, as five frames.
    fn click(&mut self, pos: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![egui::Event::PointerMoved(pos), button(true)],
            vec![egui::Event::PointerMoved(pos), button(false)],
            Vec::new(),
            Vec::new(),
        ] {
            self.run(events);
        }
    }

    /// Raise a close request, as the close button does, and let the frames
    /// after it draw what it opened.
    fn request_close(&mut self) {
        self.frame(Vec::new(), true);
        self.settle();
    }

    /// Forget what the frames have sent the operating system so far.
    fn forget_sent(&mut self) {
        self.cancelled_close = false;
        self.closed = false;
    }

    /// Put `column` on `channel`, by the chip on its row in the Outline.
    fn put(&mut self, column: &str, channel: ShelfChannel) {
        let row = self
            .app
            .spine_rows()
            .iter()
            .find(|r| r.role == SpineRole::Column && r.depth == 1 && r.label == column)
            .unwrap_or_else(|| panic!("the Outline drew no row for {column}"))
            .clone();
        self.point(row.name_rect.center());
        let chip = self
            .app
            .outline_chips()
            .iter()
            .find(|c| c.column == column && c.channel == channel)
            .unwrap_or_else(|| panic!("{column}'s row drew no {channel:?} chip"))
            .clone();
        self.click(chip.rect.center());
        self.point(egui::pos2(1.0, 1.0));
        assert!(
            self.marked(),
            "a click on {column}'s chip left the title unmarked"
        );
    }

    /// Put the grid in its columns layout, where each histogram tile is on
    /// screen with its scale switch.
    fn transpose(&mut self) {
        use brightfield_shell::app::GridLayout;
        let at = self
            .app
            .chart_doc()
            .grid_layout_switch
            .as_ref()
            .expect("the grid pane drew a layout switch")
            .states
            .iter()
            .find(|(state, _)| *state == GridLayout::Columns)
            .expect("the switch offers a columns state")
            .1
            .center();
        self.click(at);
        self.settle();
        assert_eq!(self.app.grid_layout(), GridLayout::Columns);
    }

    /// Throw the scale switch on `column`'s tile to `kind`, by a click.
    fn throw(&mut self, column: &str, kind: ScaleType) {
        let switch = self
            .app
            .chart_doc()
            .scale_switches
            .iter()
            .find(|s| s.column == column)
            .unwrap_or_else(|| panic!("no scale switch for {column:?}"))
            .clone();
        let at = switch
            .states
            .iter()
            .find(|(state, _)| *state == kind)
            .unwrap_or_else(|| panic!("{column}'s switch offers no {kind:?}"))
            .1
            .center();
        self.click(at);
        self.settle();
        assert!(self.marked(), "the throw on {column}'s tile left no mark");
    }

    /// Save through the chart palette, as a person does.
    fn save(&mut self) {
        self.key(egui::Key::Space);
        assert_eq!(self.app.open_overlay(), Some("palette"));
        self.run(vec![egui::Event::Text("save-spec".to_owned())]);
        self.run(Vec::new());
        self.key(egui::Key::Enter);
        assert_eq!(self.app.open_overlay(), None);
        self.settle();
    }

    /// The question up, with what it says.
    fn question(&self) -> CloseQuestion {
        assert_eq!(self.app.open_overlay(), Some(QUESTION), "no question is up");
        self.app
            .close_question()
            .expect("the question says nothing")
            .clone()
    }

    /// Where the last frame painted `text`, the topmost of them: the question
    /// is drawn over the window, so the last text with these words is its.
    fn painted(&self, text: &str) -> Option<egui::Rect> {
        self.texts
            .iter()
            .filter(|t| t.text == text && t.is_visible())
            .map(|t| t.ink)
            .next_back()
    }

    fn drew(&self, text: &str) -> bool {
        self.painted(text).is_some()
    }

    /// Answer the question by clicking the answer labelled `label`.
    fn answer(&mut self, label: &str) {
        assert_eq!(self.app.open_overlay(), Some(QUESTION), "no question is up");
        let at = self
            .painted(label)
            .unwrap_or_else(|| panic!("the question drew no {label:?}"))
            .center();
        self.click(at);
    }

    /// The card's rect, as egui laid its area out.
    fn card(&self) -> egui::Rect {
        self.ctx
            .memory(|m| m.area_rect(egui::Id::new(QUESTION_AREA)))
            .expect("the question's card was laid out")
    }

    /// Open the ledger on Versions by its strip name, then give the panel the
    /// keys with a press on its first row.
    fn hold_versions(&mut self) {
        let name = self
            .app
            .rail_name_rect(LEDGER_RAIL, VERSIONS_AT)
            .expect("the ledger strip drew a fifth name")
            .center();
        self.click(name);
        self.settle();
        let row = *self
            .app
            .chart_doc()
            .versions()
            .drawn_rows()
            .first()
            .expect("the panel drew no row to press");
        self.click(egui::pos2(row.left() + 40.0, row.center().y));
        self.settle();
    }

    /// Pin the clock so the newest version reads 14:02 and *now* is the same day.
    fn pin_clock(&mut self) {
        let newest = self.entries().last().expect("a version is recorded").at;
        let at = i64::try_from(newest.duration_since(UNIX_EPOCH).unwrap().as_secs()).unwrap();
        let clock = Clock::Fixed {
            now: newest + Duration::from_secs(u64::try_from(NOW_READS - NEWEST_READS).unwrap()),
            offset_secs: i32::try_from(NEWEST_READS - at).unwrap(),
        };
        self.app.set_versions_env(clock, Some(self.root.0.clone()));
        self.app.chart_doc_mut().versions_mut().invalidate();
        self.settle();
    }

    /// The hero's x, y and colour columns on the page drawn.
    fn drawn(&self) -> String {
        let spec = self.app.chart_doc().live_spec().expect("a live page");
        let path = &self.app.chart_doc().composed.plots[0].path;
        let plot = brightfield_spec::edit::plot_at_path(spec, path).expect("the hero's plot");
        let c = ShelfChannels::of_plot(plot).expect("the hero's channels");
        format!(
            "x {:?} \u{b7} y {:?} \u{b7} colour {:?}",
            c.x, c.y, c.colour
        )
    }
}

/// The edit rows the question painted: the lines it draws with a bullet.
fn bullet_rows(s: &Session) -> Vec<String> {
    s.texts
        .iter()
        .filter_map(|t| t.text.strip_prefix("\u{2022}  ").map(str::to_string))
        .collect()
}

// ---------------------------------------------------------------------------
// AC1 — what the question says
// ---------------------------------------------------------------------------

/// **AC1.** The question names the chart file, says how many edits are unsaved,
/// lists each in the shelf's words, `x axis: longitude → median_income` for a
/// column put on x, and says what closing without saving keeps.
#[test]
fn the_question_names_the_file_counts_and_lists_the_edits_and_says_what_closing_keeps() {
    let mut s = Session::open("ac1");
    s.put("median_income", ShelfChannel::X);
    s.request_close();

    assert!(s.drew(CLOSE_QUESTION_TITLE), "no title: {:?}", s.texts);
    let headline = format!("{CHART_NAMED} has 2 chart edits that are not saved.");
    assert!(
        s.drew(&headline),
        "the question does not name the file and count the edits as {headline:?}"
    );
    assert_eq!(
        bullet_rows(&s),
        X_PUT,
        "the question does not list each edit in the shelf's words"
    );
    assert!(
        s.drew("Closing without saving keeps these as a version you can step back to."),
        "the question does not say what closing without saving keeps"
    );
    assert_eq!(s.question().edits, X_PUT);
}

// ---------------------------------------------------------------------------
// AC2 — the answers and their keys
// ---------------------------------------------------------------------------

/// **AC2, the words.** The answers read *Save and close*, *Close without
/// saving* and *Keep editing*, left to right, each with its key in a chip
/// beside it, and the card has no footer stating a key a second time.
#[test]
fn the_answers_read_in_order_each_with_its_key_beside_it_and_no_footer() {
    let mut s = Session::open("ac2-words");
    s.put("median_income", ShelfChannel::X);
    s.request_close();

    let mut right_of_last = f32::MIN;
    for (label, key) in [
        ("Save and close", "Enter"),
        ("Close without saving", "D"),
        ("Keep editing", "Esc"),
    ] {
        let answer = s
            .painted(label)
            .unwrap_or_else(|| panic!("no {label:?} answer"));
        let chip = s
            .painted(key)
            .unwrap_or_else(|| panic!("no {key:?} key beside {label:?}"));
        assert!(
            answer.left() > right_of_last,
            "{label:?} is not right of the answer before it"
        );
        assert!(
            chip.left() > answer.right() && (chip.center().y - answer.center().y).abs() < 4.0,
            "{key:?} is not beside {label:?}: {chip:?} against {answer:?}"
        );
        right_of_last = chip.right();
    }
    let card = s.card();
    for key in ["Enter", "Esc"] {
        let inside = s
            .texts
            .iter()
            .filter(|t| t.text == key && card.contains_rect(t.ink))
            .count();
        assert_eq!(inside, 1, "the card states {key:?} {inside} times");
    }
}

/// **AC2, Enter with nothing focused.** Enter in the frame the question opens,
/// before an answer holds the focus, saves and closes; so does Enter on a
/// later frame, with *Save and close* holding it.
#[test]
fn enter_saves_and_closes_with_nothing_focused_and_with_save_focused() {
    for later in [false, true] {
        let mut s = Session::open(&format!("ac2-enter-{later}"));
        s.put("median_income", ShelfChannel::X);
        s.forget_sent();
        if later {
            s.request_close();
            assert_eq!(s.app.open_overlay(), Some(QUESTION));
            s.key(egui::Key::Enter);
        } else {
            s.frame(vec![Session::press(egui::Key::Enter)], true);
            s.settle();
        }
        assert!(s.closed, "later {later}: Enter did not close the window");
        assert!(
            s.chart_file().exists() && !s.marked(),
            "later {later}: Enter closed without saving the chart"
        );
    }
}

/// **AC2, D and Esc.** D closes without saving, recording the chart; Esc keeps
/// editing, with the question down and the edit still held.
#[test]
fn d_closes_without_saving_and_esc_keeps_editing() {
    let mut d = Session::open("ac2-d");
    d.put("median_income", ShelfChannel::X);
    d.request_close();
    d.forget_sent();
    d.key(egui::Key::D);
    assert!(d.closed, "D did not close the window");
    assert!(!d.chart_file().exists(), "D saved the chart");
    assert_eq!(
        d.entries().last().map(|e| e.kind),
        Some(HistoryKind::Unsaved),
        "D kept no unsaved version"
    );

    let mut esc = Session::open("ac2-esc");
    esc.put("median_income", ShelfChannel::X);
    esc.request_close();
    esc.forget_sent();
    esc.key(egui::Key::Escape);
    assert_eq!(esc.app.open_overlay(), None, "Esc left the question up");
    assert!(!esc.closed, "Esc closed the window");
    assert!(esc.marked(), "Esc dropped the edit");
    assert!(esc.entries().is_empty(), "Esc recorded a version");
}

// ---------------------------------------------------------------------------
// AC3 — what a close without saving writes and records
// ---------------------------------------------------------------------------

/// **AC3.** *Close without saving* closes the window and leaves the chart file
/// and `arcform.yaml` byte-identical; the chart file's versions list one more,
/// the newest, of the unsaved kind, and its text is what Save would have
/// written from the same window.
#[test]
fn close_without_saving_leaves_the_files_and_records_what_save_would_write() {
    let mut s = Session::open("ac3");
    s.save();
    s.put("median_income", ShelfChannel::X);
    let on_disk = files(&s.folder);
    assert!(on_disk.contains_key(&format!("panels/{CHART_FILE}")));
    assert!(on_disk.contains_key("arcform.yaml"));
    let before = s.entries();
    s.request_close();
    s.forget_sent();

    s.answer("Close without saving");

    assert!(s.closed, "the window did not close");
    assert_eq!(
        files(&s.folder),
        on_disk,
        "a file in the data folder was written"
    );
    let after = s.entries();
    assert_eq!(after.len(), before.len() + 1, "not one more version");
    let newest = after.last().expect("a newest version");
    assert_eq!(newest.kind, HistoryKind::Unsaved);
    assert!(
        before.iter().all(|e| e.at <= newest.at),
        "the version recorded is not the newest"
    );
    let kept = s
        .history()
        .read_for_file(&s.chart_file(), &newest.id)
        .expect("the version reads back");

    // What Save writes from this same window, over the same edit.
    s.save();
    let saved = std::fs::read_to_string(s.chart_file()).expect("the Save wrote the chart");
    assert_ne!(
        saved.as_bytes(),
        on_disk[&format!("panels/{CHART_FILE}")].as_slice(),
        "the Save wrote no edit, so the comparison below compares nothing"
    );
    assert_eq!(kept, saved, "the version kept is not what Save writes");
}

/// **AC3, before any Save.** With no chart file and no `arcform.yaml`, a close
/// without saving creates neither and writes no file in the data folder; the
/// version is recorded against the chart file the first Save would write.
#[test]
fn close_without_saving_before_any_save_writes_no_file_and_records_a_version() {
    let mut s = Session::open("ac3-first");
    s.put("median_income", ShelfChannel::X);
    let on_disk = files(&s.folder);
    s.request_close();
    s.answer("Close without saving");

    assert!(s.closed);
    assert_eq!(files(&s.folder), on_disk, "a file was written");
    assert!(!s.chart_file().exists() && !s.arcform_file().exists());
    let entries = s.entries();
    assert_eq!(
        entries.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [HistoryKind::Unsaved],
        "the close kept no version of the chart file"
    );
}

// ---------------------------------------------------------------------------
// AC4 — what reopening shows
// ---------------------------------------------------------------------------

/// **AC4.** Opening the file again draws the chart the file holds. The Versions
/// panel lists the closed chart in a row reading `closed unsaved`; with the
/// cursor on it the hero's header ends `as closed unsaved 14:02`, and Enter
/// steps the chart to it as an unsaved edit.
#[test]
fn reopening_draws_the_file_and_the_panel_steps_to_the_closed_unsaved_version() {
    let mut s = Session::open("ac4");
    s.save();
    let saved = s.drawn();
    s.put("median_income", ShelfChannel::X);
    let unsaved = s.drawn();
    assert_ne!(saved, unsaved);
    s.request_close();
    s.answer("Close without saving");
    assert!(s.closed);
    let kept = s.entries().last().expect("a version").id.clone();

    let mut again = s.reopen();
    assert!(!again.marked(), "the reopened window is marked unsaved");
    assert_eq!(
        again.drawn(),
        saved,
        "the reopened window does not draw the chart the file holds"
    );
    again.pin_clock();
    again.hold_versions();
    let rows = match again.app.chart_doc().versions().listing() {
        Listing::Listed { rows, .. } => rows.clone(),
        other => panic!("the panel lists no rows: {other:?}"),
    };
    assert_eq!(rows[0].id, kept, "the newest row is not the closed chart");
    assert_eq!(rows[0].kind, CLOSED_UNSAVED);
    assert!(
        again.drew(CLOSED_UNSAVED),
        "the panel drew no row reading {CLOSED_UNSAVED:?}"
    );

    again.key(egui::Key::J);
    assert_eq!(
        again.app.chart_doc().versions().cursor(),
        Some(kept.as_str()),
        "the cursor is not on the closed chart's row"
    );
    // The version's hero is not on the coordinate pair, so the header names it
    // `Dot plot` and not `Map`; what this criterion reads is how it ends.
    assert!(
        again
            .texts
            .iter()
            .any(|t| t.text.ends_with(" \u{b7} as closed unsaved 14:02")),
        "the hero's header does not end `as closed unsaved 14:02`"
    );

    again.key(egui::Key::Enter);
    assert_eq!(again.app.chart_doc().stepped_back_to(), Some(kept.as_str()));
    assert!(again.marked(), "the step back is not an unsaved edit");
    assert_eq!(
        again.drawn(),
        unsaved,
        "the chart is not drawn as the closed chart"
    );
}

// ---------------------------------------------------------------------------
// AC5 — a chart that cannot be kept
// ---------------------------------------------------------------------------

/// **AC5.** When the unsaved chart cannot be recorded, *Close without saving*
/// leaves the window open with the question up, saying the chart could not be
/// kept and why; the same answer given again closes with nothing kept. Held for
/// a store that cannot be opened, and for a window given none.
#[test]
fn a_chart_that_cannot_be_kept_keeps_the_window_open_and_the_answer_again_closes() {
    for store in [Store::Unopenable, Store::None] {
        let tag = if store == Store::None {
            "none"
        } else {
            "unopenable"
        };
        let mut s = Session::open_as(&format!("ac5-{tag}"), store);
        s.put("median_income", ShelfChannel::X);
        let on_disk = files(&s.folder);
        s.request_close();
        s.forget_sent();

        s.answer("Close without saving");

        assert!(!s.closed, "{tag}: the window closed over a chart not kept");
        assert_eq!(
            s.app.open_overlay(),
            Some(QUESTION),
            "{tag}: the question went"
        );
        assert!(s.marked(), "{tag}: the edit was dropped");
        let why = s
            .question()
            .not_kept
            .unwrap_or_else(|| panic!("{tag}: the question does not say why"));
        if store == Store::None {
            assert_eq!(why, NO_STORE);
        } else {
            assert!(
                why.contains("blocked"),
                "{tag}: the reason does not name the store's folder: {why:?}"
            );
        }
        let said = format!(
            "The chart could not be kept as a version: {why}. \
             Close without saving again to close with nothing kept."
        );
        assert!(s.drew(&said), "{tag}: the question does not say {said:?}");

        s.answer("Close without saving");

        assert!(s.closed, "{tag}: the answer given again did not close");
        assert_eq!(files(&s.folder), on_disk, "{tag}: a file was written");
        assert!(
            !s.folder.join("panels").exists(),
            "{tag}: a folder was left in the data folder"
        );
    }
}

// ---------------------------------------------------------------------------
// AC6 — more edits than the card lists
// ---------------------------------------------------------------------------

/// **AC6.** With more unsaved edits than the card lists, the list ends in a row
/// that counts the rest, the edits listed and that count add up to the edits
/// the headline counts, and the card is no taller than it is over the two edits
/// the accepted frame draws.
#[test]
fn more_edits_than_the_card_lists_end_in_a_count_and_the_card_is_no_taller() {
    let mut two = Session::open("ac6-two");
    two.put("median_income", ShelfChannel::X);
    two.request_close();
    assert_eq!(bullet_rows(&two), X_PUT, "the two-edit card lists both");
    let framed = two.card();

    let mut many = Session::open("ac6-many");
    many.put("median_income", ShelfChannel::X);
    many.put("median_house_value", ShelfChannel::Colour);
    many.transpose();
    many.throw("population", ScaleType::Log);
    many.throw("avg_occupancy", ScaleType::Log);
    many.request_close();
    let held = many.question().edits.len();
    assert!(held > CLOSE_LIST_ROWS, "only {held} edits are held");
    assert!(
        many.drew(&format!(
            "{CHART_NAMED} has {held} chart edits that are not saved."
        )),
        "the headline does not count {held} edits"
    );

    let rows = bullet_rows(&many);
    assert_eq!(
        rows.len(),
        CLOSE_LIST_ROWS,
        "the list is not cut off: {rows:?}"
    );
    let (listed, count) = rows.split_at(rows.len() - 1);
    let rest: usize = count[0]
        .strip_prefix("and ")
        .and_then(|r| r.strip_suffix(" more edits"))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("the last row does not count the rest: {:?}", count[0]));
    assert_eq!(
        listed.len() + rest,
        held,
        "the edits listed and the count do not add up to the edits held"
    );
    assert_eq!(listed, &many.question().edits[..listed.len()]);
    assert!(
        many.card().height() <= framed.height() + 0.5,
        "the card is {} high over {held} edits and {} over the frame's two",
        many.card().height(),
        framed.height()
    );
}
