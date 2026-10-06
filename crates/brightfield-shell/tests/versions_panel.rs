//! **The ledger's fifth panel lists a chart's saved versions and says what
//! changed in each.**
//!
//! The ledger rail had four panels and none listed the versions the store keeps
//! of a chart file. The fifth, Versions, draws one row per version, newest
//! first, under a head line and a header row: when the version was recorded,
//! which kind it is, and what changed in it, said in the shelf's own words.
//!
//! Every test reads the panel **off the drawn frame**: the rows' rectangles the
//! last frame recorded, and the galleys inside them. The text a test expects is
//! written out here, not derived from the module that words it, so a panel that
//! drew the wrong words fails.
//!
//! **No test writes under the home directory.** Each window is given a store at
//! a root inside its own temporary folder, beside the data file's folder, and
//! the panel is given that root as its home, so the head line's `~` stands for
//! it.
//!
//! **A clock that stands still.** arcform stamps a version with the time it is
//! recorded, so a test cannot choose when a version was made. It chooses what
//! the clock reads instead ([`Session::pin_clock`]): an offset that carries the
//! newest version to a chosen local time, and a *now* a chosen distance after
//! it, so `today 14:02` is the same text on a later day and in another time zone.
//!
//! The versions a test reads are seeded into the store from chart texts the
//! window itself wrote ([`recorded_texts`]), so the changes between them are
//! the changes a person's gestures make.
//!
//! # What is covered and what is not
//!
//! Covered: the strip's fifth name and the rail's extents; the rows, their
//! order, their height and their header; the words of a version's time, kind and
//! change, including an oldest version, a text edited outside, a version that
//! differs in line endings alone, a version carrying a named change and a change
//! elsewhere, and a row longer than the panel; the unsaved row; the head line;
//! the empty states; and the panel as pixels, in light and in dark, over four
//! versions with the rail dragged to 236 high. Not covered: the cursor, the redraw of the chart for a
//! version, Enter and the Step back control, which are another card's.

use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use brightfield_protocol::chart_history::LocalHistory;
use brightfield_protocol::HistoryStore;
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::SpineRole;
use brightfield_shell::text_ink;
use brightfield_shell::versions::{Clock, Listing, ROW_HEIGHT};
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_spec::layout::ScaleType;
use brightfield_workbench::arrangement::{self, Extent, LEDGER_RAIL};
use brightfield_workbench::channel::ShelfChannel;

const HOUSING_FILE: &str = "california_housing_sample.csv";

/// The chart file's name: the Protocol's name, which is the data file's stem.
const CHART_FILE: &str = "california_housing_sample.yaml";

/// Where the ledger strip's Versions name is: the fifth.
const VERSIONS_AT: usize = 4;

/// 2026-10-05 00:00 UTC, in seconds from the epoch.
const DAY: i64 = 20_731 * 86_400;

/// The local time the newest version is pinned to read, 14:02:50: late in its
/// minute, so a version recorded a few seconds before it reads 14:02 too.
const NEWEST_READS: i64 = DAY + 14 * 3_600 + 2 * 60 + 50;

/// The local time the clock is pinned to read: the same day.
const SAME_DAY: i64 = DAY + 14 * 3_600 + 5 * 60;

/// The local time the clock is pinned to read for a version of an earlier day.
const THREE_DAYS_ON: i64 = SAME_DAY + 3 * 86_400;

/// The committed table the window is opened over.
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
            "bf-versions-panel-{name}-{}-{}",
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
    /// A store at `<root>/.arcform/history`, with `<root>` the panel's home.
    Kept,
    /// No store at all.
    None,
    /// A store whose folder cannot be made, because a file stands where its
    /// parent should be.
    Unopenable,
}

/// A window over the housing file copied into a folder of its own.
///
/// It keeps one `egui::Context` for its whole life, because a click is resolved
/// against the widget id a *previous* frame registered.
struct Session {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    /// The data file's folder: where `arcform.yaml` and `panels/` land.
    folder: PathBuf,
    /// The root of the store the window records into, outside `folder`.
    history_root: PathBuf,
    /// The text the frame before drew.
    texts: Vec<text_ink::DrawnText>,
    /// The events of every frame the window has run, in order, so a capture can
    /// replay what a probe window was driven through.
    frames: Vec<Vec<egui::Event>>,
    /// Held last so the folders outlive the window.
    root: TempDir,
}

impl Session {
    fn open(name: &str) -> Self {
        Self::open_as(name, Store::Kept, (1440.0, 900.0))
    }

    fn open_as(name: &str, store: Store, size: (f32, f32)) -> Self {
        Self::build(name, store, size, false)
    }

    /// A window as a capture draws it: no shelf band, which is authoring chrome
    /// a picture of the dashboard leaves out.
    fn open_for_capture(name: &str, size: (f32, f32)) -> Self {
        Self::build(name, Store::Kept, size, true)
    }

    fn build(name: &str, store: Store, size: (f32, f32), for_capture: bool) -> Self {
        let root = TempDir::new(name);
        let folder = root.0.join("data");
        std::fs::create_dir_all(&folder).expect("the data file's folder");
        let data = folder.join(HOUSING_FILE);
        std::fs::copy(housing(), &data).expect("the housing fixture copies");
        let boot = Boot::data_file(data.to_str().expect("utf-8 path"))
            .unwrap_or_else(|e| panic!("open {}: {e}", data.display()));
        let history_root = match store {
            Store::Unopenable => {
                std::fs::write(root.0.join("blocked"), "a file, not a folder")
                    .expect("the file that blocks the store");
                root.0.join("blocked").join("history")
            }
            Store::Kept | Store::None => root.0.join(".arcform").join("history"),
        };
        let mut app = MeridianApp::headless(boot, Mode::Light);
        if for_capture {
            app.set_shelf_band_drawn(false);
        }
        let app = match store {
            Store::None => app,
            Store::Kept | Store::Unopenable => {
                app.keeping_history(Some(HistoryStore::At(history_root.clone())))
            }
        };
        let mut session = Self {
            app,
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1)),
            folder,
            history_root,
            texts: Vec::new(),
            frames: Vec::new(),
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

    fn chart_text(&self) -> String {
        std::fs::read_to_string(self.chart_file())
            .unwrap_or_else(|e| panic!("read the chart file {}: {e}", self.chart_file().display()))
    }

    /// arcform's store, opened the way a person's `arc history` would reach it.
    fn history(&self) -> LocalHistory {
        LocalHistory::at_root(&self.history_root)
    }

    fn marked(&self) -> bool {
        self.app.title().contains(UNSAVED_MARK)
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        self.frames.push(events.clone());
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

    fn key(&mut self, key: egui::Key) {
        self.run(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }]);
        self.run(Vec::new());
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

    /// Put the grid in its columns layout, the one where each histogram tile is
    /// on screen with its switch.
    fn transpose(&mut self) {
        use brightfield_shell::app::GridLayout;
        let at = self
            .app
            .chart_doc()
            .grid_layout_switch
            .as_ref()
            .expect("the grid pane's header band drew a layout switch")
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

    /// Throw the scale switch on `column`'s tile to `kind`, by the gesture.
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
        assert!(
            self.marked(),
            "the click at {at:?} did not throw {column}'s switch: the title is {:?}",
            self.app.title()
        );
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
            "a click on {column}'s {channel:?} chip left the title unmarked"
        );
    }

    /// Take back the last column kept, by the click on the status band's line
    /// for it: the control the band offers, named for the key that does the same.
    fn undo(&mut self) {
        let at = self
            .app
            .rail()
            .controls
            .iter()
            .find(|c| c.name.starts_with("undo"))
            .unwrap_or_else(|| panic!("the band offers no undo: {:?}", self.app.rail().controls))
            .rect
            .center();
        self.click(at);
        self.settle();
    }

    /// **Save, through the gesture a person has**: the chart palette on `space`,
    /// the verb typed, confirmed with enter.
    fn save(&mut self) {
        assert!(
            self.app.has_protocol_to_save(),
            "this window has no Protocol behind it to save"
        );
        self.key(egui::Key::Space);
        assert_eq!(self.app.open_overlay(), Some("palette"));
        self.settle();
        self.run(vec![egui::Event::Text("save-spec".to_owned())]);
        self.run(Vec::new());
        self.key(egui::Key::Enter);
        assert_eq!(
            self.app.open_overlay(),
            None,
            "confirming save-spec did not close the palette"
        );
        self.settle();
    }

    /// Seed the chart file's history with `versions`, oldest first: each is a
    /// save where the flag is true and the text before a write where it is not.
    ///
    /// A save is merged into a save no older than ten seconds, so a test that
    /// wants several versions seeds the text before a write for all but one.
    fn seed(&self, versions: &[(bool, &str)]) {
        let history = self.history();
        std::fs::create_dir_all(self.chart_file().parent().expect("a parent")).expect("panels/");
        for (save, text) in versions {
            let recorded = if *save {
                history.record_save_for_file(&self.chart_file(), text)
            } else {
                history.record_checkpoint_for_file(&self.chart_file(), text)
            };
            recorded.expect("the version records");
        }
    }

    /// The clock that reads where `NEWEST_READS` and `now_reads` say: an offset
    /// that makes the newest version's local time `NEWEST_READS`, and a *now*
    /// `now_reads - NEWEST_READS` seconds after it.
    fn clock_reading(&self, now_reads: i64) -> Clock {
        let newest = self
            .history()
            .entries_for_file(&self.chart_file())
            .expect("the chart file's history lists")
            .last()
            .expect("a version is recorded")
            .at;
        let at = i64::try_from(
            newest
                .duration_since(UNIX_EPOCH)
                .expect("after the epoch")
                .as_secs(),
        )
        .expect("a count of seconds");
        let offset = i32::try_from(NEWEST_READS - at).expect("an offset in range");
        let now = newest + Duration::from_secs(u64::try_from(now_reads - NEWEST_READS).unwrap());
        Clock::Fixed {
            now,
            offset_secs: offset,
        }
    }

    /// Carry the clock to where [`Self::clock_reading`] says it reads.
    fn pin_clock(&mut self, now_reads: i64) {
        let clock = self.clock_reading(now_reads);
        self.app.set_versions_env(clock, Some(self.root.0.clone()));
        self.app.chart_doc_mut().versions_mut().invalidate();
    }

    /// Click the ledger strip's Versions name, and settle.
    fn show_versions(&mut self) {
        let at = self
            .app
            .rail_name_rect(LEDGER_RAIL, VERSIONS_AT)
            .expect("the ledger strip drew a fifth name")
            .center();
        self.click(at);
        self.settle();
        assert_eq!(
            self.app.rail_pane_title(LEDGER_RAIL).as_deref(),
            Some("Versions"),
            "a click on the strip's fifth name did not open the rail on Versions"
        );
    }

    fn rail(&self) -> egui::Rect {
        self.app
            .region_rect(LEDGER_RAIL)
            .expect("the ledger rail drew")
    }

    /// Drag the ledger rail's top edge until the rail is `height` high, as a
    /// person does: a press on the edge, a move and a release.
    fn drag_ledger_to(&mut self, height: f32) {
        let rail = self.rail();
        let grab = egui::pos2(rail.center().x, rail.top());
        let to = egui::pos2(grab.x, grab.y - (height - rail.height()));
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.run(vec![egui::Event::PointerMoved(grab)]);
        self.run(vec![egui::Event::PointerMoved(grab), button(grab, true)]);
        self.run(vec![egui::Event::PointerMoved(to)]);
        self.run(vec![egui::Event::PointerMoved(to)]);
        self.run(vec![egui::Event::PointerMoved(to), button(to, false)]);
        self.settle();
        assert!(
            (self.rail().height() - height).abs() < 0.5,
            "a drag of the rail's top edge from {grab:?} to {to:?} left the rail {}pt high, not {height}",
            self.rail().height()
        );
    }

    /// The galleys the ledger rail drew, each with its box, in paint order.
    fn rail_texts(&self) -> Vec<&text_ink::DrawnText> {
        let rail = self.rail();
        self.texts
            .iter()
            .filter(|t| rail.contains_rect(t.ink.shrink(0.5)))
            .collect()
    }

    /// Every row the panel drew, the row for edits not yet saved first: its
    /// rectangle, and the three cells read off the galleys inside it, left to
    /// right.
    ///
    /// A cell is a galley that begins at one of the panel's three column
    /// origins. A confirmation toast is drawn over the foot of the window, and
    /// a galley of its own can sit inside a row's rectangle without being one of
    /// the row's cells.
    fn rows(&self) -> Vec<(egui::Rect, Vec<String>)> {
        const COLUMNS: [f32; 3] = [12.0, 162.0, 312.0];
        self.app
            .chart_doc()
            .versions()
            .drawn_rows()
            .iter()
            .map(|rect| {
                let mut cells: Vec<&text_ink::DrawnText> = self
                    .texts
                    .iter()
                    .filter(|t| rect.contains(t.ink.center()))
                    .filter(|t| {
                        COLUMNS
                            .iter()
                            .any(|x| (t.ink.min.x - (rect.min.x + x)).abs() < 3.0)
                    })
                    .collect();
                cells.sort_by(|a, b| a.ink.min.x.total_cmp(&b.ink.min.x));
                (*rect, cells.iter().map(|t| t.text.clone()).collect())
            })
            .collect()
    }

    /// The row cells only, without the rectangles.
    fn cells(&self) -> Vec<Vec<String>> {
        self.rows().into_iter().map(|(_, cells)| cells).collect()
    }

    /// The galley that reads `text` inside the rail, if one was drawn.
    fn rail_text_reading(&self, text: &str) -> Option<&text_ink::DrawnText> {
        self.rail_texts().into_iter().find(|t| t.text == text)
    }
}

// ---------------------------------------------------------------------------
// The chart texts a person's gestures write
// ---------------------------------------------------------------------------

/// The chart file's text after each of four Saves, oldest first, each made by
/// gesture on a window of its own and each a step on the one before.
struct Texts {
    /// The chart as generated.
    base: String,
    /// `median_income` put on x.
    x_moved: String,
    /// `population`'s scale switched to log.
    scale_switched: String,
    /// `median_house_value` put on y.
    y_moved: String,
}

/// The four texts, written once for the whole suite: each test reads them and
/// none changes them.
fn recorded_texts() -> &'static Texts {
    static TEXTS: std::sync::OnceLock<Texts> = std::sync::OnceLock::new();
    TEXTS.get_or_init(record_texts)
}

/// Write the four texts the way a person does: Save, then an edit and a Save,
/// three times over.
fn record_texts() -> Texts {
    let mut s = Session::open("recording");
    s.save();
    let base = s.chart_text();
    s.put("median_income", ShelfChannel::X);
    s.save();
    let x_moved = s.chart_text();
    s.transpose();
    s.throw("population", ScaleType::Log);
    s.save();
    let scale_switched = s.chart_text();
    s.put("median_house_value", ShelfChannel::Y);
    s.save();
    let y_moved = s.chart_text();
    for pair in [&base, &x_moved, &scale_switched, &y_moved].windows(2) {
        assert_ne!(pair[0], pair[1], "a Save in the recording changed nothing");
    }
    Texts {
        base,
        x_moved,
        scale_switched,
        y_moved,
    }
}

// ---------------------------------------------------------------------------
// AC1 — five names, and a click on the fifth opens the rail on it
// ---------------------------------------------------------------------------

/// **AC1.** The strip names five panels, Log, Quality, Rows, Editor and
/// Versions, in that order, and a click on Versions opens the rail on it at the
/// ledger's own extents, 180 by default and 120 at least.
#[test]
fn the_strip_names_five_panels_and_a_click_on_versions_opens_the_rail_at_the_ledgers_extents() {
    let mut s = Session::open("ac1");
    for (index, want) in ["Log", "Quality", "Rows", "Editor", "Versions"]
        .into_iter()
        .enumerate()
    {
        let rect = s
            .app
            .rail_name_rect(LEDGER_RAIL, index)
            .unwrap_or_else(|| panic!("the strip drew no name at index {index}"));
        assert!(
            s.texts
                .iter()
                .any(|t| t.text == want && rect.contains(t.ink.center())),
            "the strip's name at index {index} does not read {want:?}"
        );
    }

    s.show_versions();
    assert!(
        (s.rail().height() - 180.0).abs() < 1.0,
        "the rail opened on Versions at {}pt, not the ledger's 180",
        s.rail().height()
    );
    let Extent::Rail { default, min } = arrangement::default_arrangement()
        .expect_region(LEDGER_RAIL)
        .extent
    else {
        panic!("the ledger rail's extent is not a rail's");
    };
    assert_eq!(
        (default, min),
        (180.0, 120.0),
        "the ledger's own extents are no longer 180 by default and 120 at least"
    );
}

// ---------------------------------------------------------------------------
// AC2 — two Saves, one edit between: two rows, newest first, under a header
// ---------------------------------------------------------------------------

/// **AC2.** After two Saves of a data file's chart with one edit between them,
/// the panel lists two rows, the newer first, under a header row reading *when*,
/// *kind* and *what changed*, and each row is 24 high.
#[test]
fn after_two_saves_with_an_edit_between_the_panel_lists_two_rows_newest_first_under_a_header() {
    let mut s = Session::open("ac2");
    s.transpose();
    s.save();
    s.throw("population", ScaleType::Log);
    s.save();
    s.pin_clock(SAME_DAY);
    s.show_versions();

    let rows = s.rows();
    assert_eq!(
        rows.len(),
        2,
        "two Saves with an edit between are not two rows: {:?}",
        s.cells()
    );
    for (rect, _) in &rows {
        assert!(
            (rect.height() - ROW_HEIGHT).abs() < 0.01 && ROW_HEIGHT == 24.0,
            "a row drew {}pt high, not 24",
            rect.height()
        );
    }
    assert!(
        rows[0].0.top() < rows[1].0.top(),
        "the rows are not in order down the panel"
    );
    assert!(
        rows[0].1[2].contains("population"),
        "the first row is not the newer version, which switched population's scale: {:?}",
        rows[0].1
    );
    assert_eq!(
        rows[1].1[2], "the first version kept",
        "the second row is not the older version"
    );

    // The header row: its three words on one line, left to right, above the
    // first row.
    let when = s.rail_text_reading("when").expect("a *when* header");
    let kind = s.rail_text_reading("kind").expect("a *kind* header");
    let changed = s
        .rail_text_reading("what changed")
        .expect("a *what changed* header");
    // One line is what fits in one row's height. The ink boxes are compared as
    // a group and not centre to centre, because a box is the glyphs' own and
    // `kind` and `what changed` reach above and below where `when` does.
    let line = when.ink.union(kind.ink).union(changed.ink);
    assert!(
        line.height() < ROW_HEIGHT
            && when.ink.min.x < kind.ink.min.x
            && kind.ink.min.x < changed.ink.min.x,
        "the header's three words are not one line, left to right: {line:?}"
    );
    assert!(
        line.max.y < rows[0].0.top() + 1.0,
        "the header row is not above the first row"
    );
}

// ---------------------------------------------------------------------------
// AC3 — when, and kind
// ---------------------------------------------------------------------------

/// **AC3, the time.** A row's *when* reads `today` and the time for a version
/// recorded on the current day, and the weekday, day, month and time for an
/// earlier one.
#[test]
fn a_rows_when_reads_today_and_its_time_or_the_weekday_day_month_and_time() {
    let t = recorded_texts();
    let mut s = Session::open("ac3-when");
    s.seed(&[(true, &t.base), (false, &t.x_moved)]);

    s.pin_clock(SAME_DAY);
    s.show_versions();
    assert_eq!(
        s.cells().iter().map(|r| r[0].as_str()).collect::<Vec<_>>(),
        ["today 14:02", "today 14:02"],
        "a version of the current day does not read today and its time"
    );

    // Three days on, the same two versions are of an earlier day.
    s.pin_clock(THREE_DAYS_ON);
    s.settle();
    assert_eq!(
        s.cells().iter().map(|r| r[0].as_str()).collect::<Vec<_>>(),
        ["Mon 5 Oct 14:02", "Mon 5 Oct 14:02"],
        "a version of an earlier day does not read its weekday, day, month and time"
    );
}

/// **AC3, the kind.** A row's *kind* reads `saved` for a save and `before a
/// write` for a text recorded before a write.
#[test]
fn a_rows_kind_reads_saved_for_a_save_and_before_a_write_for_a_text_recorded_before_one() {
    let t = recorded_texts();
    let mut s = Session::open("ac3-kind");
    s.seed(&[
        (true, &t.base),
        (false, &t.x_moved),
        (false, &t.scale_switched),
        (true, &t.y_moved),
    ]);
    s.pin_clock(SAME_DAY);
    s.show_versions();
    assert_eq!(
        s.cells().iter().map(|r| r[1].as_str()).collect::<Vec<_>>(),
        ["saved", "before a write", "before a write", "saved"],
        "the kinds are not the store's, newest first"
    );
}

// ---------------------------------------------------------------------------
// AC4 — what changed
// ---------------------------------------------------------------------------

/// **AC4, the shelf's words.** A row's *what changed* leads with the tile the
/// change was made on and says the change in the shelf's words: `x axis:
/// longitude → median_income` for a column put on x and `y scale: linear → log`
/// for a scale switch. The oldest row reads that it is the first version kept.
#[test]
fn a_rows_change_leads_with_its_tile_and_is_said_in_the_shelfs_words() {
    let t = recorded_texts();
    let mut s = Session::open("ac4-words");
    s.seed(&[
        (true, &t.base),
        (false, &t.x_moved),
        (false, &t.scale_switched),
        (true, &t.y_moved),
    ]);
    s.pin_clock(SAME_DAY);
    s.show_versions();
    let changed: Vec<String> = s.cells().into_iter().map(|mut r| r.remove(2)).collect();
    assert_eq!(
        changed,
        [
            "Map \u{b7} y axis: latitude \u{2192} median_house_value",
            "population \u{b7} x scale: linear \u{2192} log",
            "Map \u{b7} projection type: equirectangular removed \u{b7} \
             x axis: longitude \u{2192} median_income",
            "the first version kept",
        ],
        "the rows do not say each change in the shelf's words"
    );
}

/// **AC4, the shelf's word for the other axis.** The population tile's scale is
/// on x, so the recording above reads `x scale`; the same switch on y reads
/// `y scale`, and a scale taken back to the default reads as `linear`.
#[test]
fn a_scale_on_y_reads_y_scale_and_a_scale_taken_out_reads_linear() {
    let t = recorded_texts();
    let on_y = t.scale_switched.replace("xScale: log", "yScale: log");
    assert_ne!(
        on_y, t.scale_switched,
        "the recording carries no xScale: log"
    );
    let mut s = Session::open("ac4-y-scale");
    s.seed(&[
        (true, &t.x_moved),
        (false, &t.scale_switched),
        (true, &on_y),
    ]);
    s.pin_clock(SAME_DAY);
    s.show_versions();
    let changed: Vec<String> = s.cells().into_iter().map(|mut r| r.remove(2)).collect();
    assert_eq!(
        changed,
        [
            "population \u{b7} x scale: log \u{2192} linear \u{b7} \
             y scale: linear \u{2192} log",
            "population \u{b7} x scale: linear \u{2192} log",
            "the first version kept",
        ],
        "a scale does not read in the shelf's words"
    );
}

/// **AC4, an edit outside.** A version edited outside reads `edited outside
/// brightfield` with its count of lines. A version that differs from the one
/// before it in line endings alone reads that count as `0 lines`.
#[test]
fn a_version_edited_outside_reads_its_lines_and_line_endings_alone_read_none() {
    let t = recorded_texts();
    let noted = format!("{}# a note\n# another\n# a third\n", t.base);
    let crlf = noted.replace('\n', "\r\n");
    let mut s = Session::open("ac4-outside");
    s.seed(&[(false, &t.base), (false, &noted), (false, &crlf)]);
    s.pin_clock(SAME_DAY);
    s.show_versions();
    let changed: Vec<String> = s.cells().into_iter().map(|mut r| r.remove(2)).collect();
    assert_eq!(
        changed,
        [
            "edited outside brightfield \u{b7} 0 lines",
            "edited outside brightfield \u{b7} 3 lines",
            "the first version kept",
        ],
        "a text edited outside does not read as one"
    );
}

/// **AC4, both kinds.** A version carrying a named change and a change the
/// chart does not name reads both, the second as a count of lines. The count is
/// of the lines that differ, the lines of the named changes included: here the
/// two `x` lines, the line that drops the projection type, and the title.
#[test]
fn a_version_carrying_a_named_change_and_one_elsewhere_reads_both() {
    let t = recorded_texts();
    let retitled = t.x_moved.replace(
        "title: \"california_housing_sample.csv\"",
        "title: \"housing\"",
    );
    assert_ne!(
        retitled, t.x_moved,
        "the recording carries no title to change"
    );
    let mut s = Session::open("ac4-both");
    s.seed(&[(false, &t.base), (false, &retitled)]);
    s.pin_clock(SAME_DAY);
    s.show_versions();
    let changed: Vec<String> = s.cells().into_iter().map(|mut r| r.remove(2)).collect();
    assert_eq!(
        changed,
        [
            "Map \u{b7} projection type: equirectangular removed \u{b7} \
             x axis: longitude \u{2192} median_income \u{b7} \
             edited outside brightfield \u{b7} 4 lines",
            "the first version kept",
        ],
        "a version carrying both kinds does not read both"
    );
}

/// **AC4, a row longer than the panel** ends in an ellipsis, and one that fits
/// does not.
#[test]
fn a_row_longer_than_the_panel_ends_in_an_ellipsis() {
    let t = recorded_texts();
    let seed = [(true, t.base.as_str()), (false, t.y_moved.as_str())];

    let mut wide = Session::open_as("ac4-wide", Store::Kept, (1800.0, 900.0));
    wide.seed(&seed);
    wide.pin_clock(SAME_DAY);
    wide.show_versions();
    let whole = wide.cells()[0][2].clone();
    assert!(
        !whole.ends_with('\u{2026}') && whole.contains("median_house_value"),
        "the row does not fit a wide panel either: {whole:?}"
    );

    let mut narrow = Session::open_as("ac4-narrow", Store::Kept, (760.0, 900.0));
    narrow.seed(&seed);
    narrow.pin_clock(SAME_DAY);
    narrow.show_versions();
    let cut = narrow.cells()[0][2].clone();
    assert!(
        cut.ends_with('\u{2026}') && whole.starts_with(cut.trim_end_matches('\u{2026}')),
        "a row longer than the panel does not end in an ellipsis: {cut:?}"
    );
    let rail = narrow.rail();
    let cell = narrow
        .texts
        .iter()
        .find(|t| t.text == cut)
        .expect("the cut cell drew");
    assert!(
        cell.ink.max.x <= rail.right(),
        "the cut cell reaches {} past the rail's right edge {}",
        cell.ink.max.x,
        rail.right()
    );
}

// ---------------------------------------------------------------------------
// AC5 — the edits not yet saved
// ---------------------------------------------------------------------------

/// **AC5.** With an unsaved edit held, the first row reads `now` and `unsaved`
/// and names the edits not yet saved. With none held, no such row is drawn.
#[test]
fn an_unsaved_edit_is_the_first_row_and_none_held_draws_no_such_row() {
    let mut s = Session::open("ac5");
    s.save();
    s.show_versions();
    assert_eq!(
        s.cells().len(),
        1,
        "with nothing unsaved the panel draws more than the one saved version: {:?}",
        s.cells()
    );
    assert!(
        s.cells().iter().all(|r| r[0] != "now"),
        "a row reads *now* with no unsaved edit held"
    );

    // An edit made while the panel is on screen appears as a row.
    s.put("median_income", ShelfChannel::X);
    s.settle();
    let cells = s.cells();
    assert_eq!(cells.len(), 2, "the unsaved edit is not a row: {cells:?}");
    assert_eq!(
        cells[0],
        [
            "now",
            "unsaved",
            "Map \u{b7} projection type: equirectangular removed \u{b7} \
             x axis: longitude \u{2192} median_income"
        ],
        "the first row does not name the edit not yet saved"
    );

    // A second edit is named too.
    s.put("median_house_value", ShelfChannel::Y);
    s.settle();
    assert_eq!(
        s.cells()[0][2],
        "Map \u{b7} projection type: equirectangular removed \u{b7} \
         x axis: longitude \u{2192} median_income \u{b7} \
         y axis: latitude \u{2192} median_house_value",
        "the first row does not name both edits not yet saved"
    );

    // Taking the edits back leaves none held, and the row goes with them
    // without a Save: the count of edits not yet saved falls to nothing by an
    // undo as well as by a Save.
    s.undo();
    s.settle();
    assert_eq!(
        s.cells()[0],
        [
            "now",
            "unsaved",
            "Map \u{b7} projection type: equirectangular removed \u{b7} \
             x axis: longitude \u{2192} median_income"
        ],
        "after the last edit was taken back the first row does not name the one left"
    );
    s.undo();
    s.settle();
    let cells = s.cells();
    assert_eq!(
        cells.len(),
        1,
        "a row is still drawn after every edit was taken back: {cells:?}"
    );
    assert!(
        cells.iter().all(|r| r[0] != "now"),
        "a row still reads *now* after every edit was taken back: {cells:?}"
    );

    // An edit again, and the Save writes it: the row goes, and the version it
    // recorded is listed.
    s.put("median_income", ShelfChannel::X);
    s.settle();
    s.save();
    s.settle();
    let cells = s.cells();
    assert!(
        cells.iter().all(|r| r[0] != "now"),
        "a row still reads *now* after the Save wrote the edits: {cells:?}"
    );
    assert_eq!(
        cells.len(),
        2,
        "the Save's version is not listed: {cells:?}"
    );
}

/// The rows of a panel that was not on screen while an edit was taken back and
/// another of the same size made, as a person leaves the panel and returns to
/// it: `hide` takes the panel off the screen and `show` brings it back.
///
/// The panel reads the edits held on the frames it is shown and returns before
/// reading them on the others. A put on x and a put on y add the same number of
/// edits, so a reading kept by that number names the edit that is gone.
fn the_rows_after_a_hidden_retake(
    name: &str,
    hide: fn(&mut Session),
    show: fn(&mut Session),
) -> Vec<Vec<String>> {
    let mut s = Session::open(name);
    s.save();
    s.show_versions();
    s.put("median_income", ShelfChannel::X);
    s.settle();
    let read_shown = s.cells()[0].clone();
    assert_eq!(
        read_shown[..2],
        ["now", "unsaved"],
        "the panel on screen drew no row for the edit held: {read_shown:?}"
    );
    assert!(
        read_shown[2].ends_with("x axis: longitude \u{2192} median_income"),
        "the row read while the panel was on screen does not name the put on x: {read_shown:?}"
    );
    let held = s.app.chart_doc().unsaved_edits().to_vec();

    hide(&mut s);
    s.key(egui::Key::U);
    assert!(
        s.app.chart_doc().unsaved_edits().is_empty(),
        "a keypress on u did not take the put on x back while the panel was not on screen"
    );
    s.put("median_house_value", ShelfChannel::Y);
    s.settle();
    let now_held = s.app.chart_doc().unsaved_edits().to_vec();
    assert_eq!(
        now_held.len(),
        held.len(),
        "a put on y does not add as many edits as the put on x, so this does not drive an \
         edit of the same size"
    );
    assert_ne!(
        now_held, held,
        "the edit held is the one that was taken back"
    );

    show(&mut s);
    assert_eq!(
        s.app.rail_pane_title(LEDGER_RAIL).as_deref(),
        Some("Versions"),
        "the panel was not brought back on screen"
    );
    s.cells()
}

/// What the first row reads for a put on y alone over the housing start: the
/// put changes the hero's marks and drops its projection type, as a put on x
/// does.
const Y_PUT_READS: &str = "Map \u{b7} projection type: equirectangular removed \u{b7} \
                           y axis: latitude \u{2192} median_house_value";

/// **AC5, on the path where the strip showed another panel.** The row for the
/// edits not yet saved names the edit held when the panel is shown again, not
/// the one that was held when it was left.
#[test]
fn the_unsaved_row_after_the_strip_showed_another_panel_names_the_edit_held_now() {
    let rows = the_rows_after_a_hidden_retake(
        "ac5-strip",
        |s| {
            let log = s
                .app
                .rail_name_rect(LEDGER_RAIL, 0)
                .expect("the strip's Log name")
                .center();
            s.click(log);
            s.settle();
            assert_eq!(
                s.app.rail_pane_title(LEDGER_RAIL).as_deref(),
                Some("Log"),
                "a click on the strip's Log name did not show the Log"
            );
        },
        Session::show_versions,
    );
    assert_eq!(
        rows[0],
        ["now", "unsaved", Y_PUT_READS],
        "the panel shown again names an edit it no longer holds, or omits the one it does"
    );
}

/// **AC5, on the path where the rail was collapsed.** A collapsed rail shows no
/// panel, so it takes the same path, and a version recorded beside the window
/// while it was collapsed is listed when the rail opens again: the collapsed
/// rail counts as the panel not being shown, for the listing as for the row.
#[test]
fn the_unsaved_row_after_the_rail_was_collapsed_names_the_edit_held_now() {
    let rows = the_rows_after_a_hidden_retake(
        "ac5-collapsed",
        |s| {
            let at = s
                .app
                .rail_collapse_rect(LEDGER_RAIL)
                .expect("the ledger rail drew a collapse control")
                .center();
            s.click(at);
            s.settle();
            assert!(
                s.app.rail_is_collapsed(LEDGER_RAIL),
                "a click on the ledger rail's collapse control did not collapse it"
            );
            s.seed(&[(false, &recorded_texts().x_moved)]);
        },
        |s| {
            let at = s
                .app
                .rail_collapse_rect(LEDGER_RAIL)
                .expect("the collapsed ledger rail drew an expand control")
                .center();
            s.click(at);
            s.settle();
            assert!(
                !s.app.rail_is_collapsed(LEDGER_RAIL),
                "a click on the collapsed ledger rail's control did not open it"
            );
        },
    );
    assert_eq!(
        rows[0],
        ["now", "unsaved", Y_PUT_READS],
        "the rail opened again names an edit it no longer holds, or omits the one it does"
    );
    assert_eq!(
        rows.len(),
        3,
        "the rail opened again did not list the version recorded while it was collapsed, \
         beside the Save's and the row for the edit held: {rows:?}"
    );
}

// ---------------------------------------------------------------------------
// AC6 — the head line
// ---------------------------------------------------------------------------

/// **AC6.** The head line names the chart file relative to the Protocol's
/// folder, the count of versions kept of 50, that the oldest is dropped first,
/// and the store's folder with the home directory written `~`.
#[test]
fn the_head_line_names_the_file_the_count_the_prune_rule_and_the_store() {
    let t = recorded_texts();
    let mut s = Session::open("ac6");
    s.seed(&[(true, &t.base), (false, &t.x_moved)]);
    s.pin_clock(SAME_DAY);
    s.show_versions();
    let want = format!(
        "panels/{CHART_FILE} \u{b7} 2 versions kept of 50 \u{b7} the oldest is dropped first \
         \u{b7} kept in ~/.arcform/history, outside the data folder"
    );
    assert!(
        s.rail_text_reading(&want).is_some(),
        "the head line is not drawn as {want:?}; the rail drew {:?}",
        s.rail_texts().iter().map(|t| &t.text).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// AC7 — the empty states
// ---------------------------------------------------------------------------

/// What the rail reads, galley by galley, joined.
fn rail_words(s: &Session) -> String {
    s.rail_texts()
        .iter()
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join(" | ")
}

/// **AC7.** Before the first Save, and in a window with no store, the panel
/// draws an empty state that says no version is recorded and names Save.
#[test]
fn before_the_first_save_and_with_no_store_the_panel_says_no_version_is_recorded_and_names_save() {
    for (name, store) in [("ac7-first", Store::Kept), ("ac7-none", Store::None)] {
        let mut s = Session::open_as(name, store, (1440.0, 900.0));
        s.show_versions();
        let words = rail_words(&s);
        assert!(
            words.contains("No version is recorded") && words.contains("Save"),
            "{name}: the empty state does not say no version is recorded and name Save: {words}"
        );
        assert!(
            s.cells().is_empty(),
            "{name}: the panel drew rows with nothing recorded"
        );
    }
}

/// **AC7, a store that cannot be opened** says why.
#[test]
fn a_store_that_cannot_be_opened_says_why() {
    let mut s = Session::open_as("ac7-blocked", Store::Unopenable, (1440.0, 900.0));
    s.show_versions();
    let words = rail_words(&s);
    assert!(
        words.contains("The versions cannot be listed") && words.contains("history folder"),
        "the panel does not say the store cannot be opened and why: {words}"
    );
    assert!(
        matches!(
            s.app.chart_doc().versions().listing(),
            Listing::Failed(why) if why.contains("history folder")
        ),
        "the listing is not a failure that names the folder"
    );
}

// ---------------------------------------------------------------------------
// The listing is read when it should be, and not otherwise
// ---------------------------------------------------------------------------

/// A Save while the panel is on screen lists the version it recorded on the
/// next frame, and a store written beside the window is listed when the panel
/// is shown again: the listing is read on a Save and on a show, and the rows are
/// not a copy of the first read.
#[test]
fn a_save_lists_its_version_and_a_show_after_a_hide_reads_the_store_again() {
    let t = recorded_texts();
    let mut s = Session::open("reread");
    s.show_versions();
    assert!(s.cells().is_empty(), "rows before any version is recorded");

    s.save();
    assert_eq!(
        s.cells().len(),
        1,
        "a Save made while the panel is on screen did not list its version: {:?}",
        s.cells()
    );

    // Away to the Log, a version recorded beside the window, and back.
    let log = s
        .app
        .rail_name_rect(LEDGER_RAIL, 0)
        .expect("the strip's Log name")
        .center();
    s.click(log);
    s.settle();
    s.seed(&[(false, &t.x_moved)]);
    s.show_versions();
    assert_eq!(
        s.cells().len(),
        2,
        "the panel shown again did not read the version recorded while it was hidden: {:?}",
        s.cells()
    );
}

// ---------------------------------------------------------------------------
// AC8 — the panel, as pixels
// ---------------------------------------------------------------------------

/// The window the baselines are drawn in, the size the dashboard baselines use.
const BASELINE_WINDOW: (f32, f32) = (1440.0, 900.0);

/// The height the baselines draw the rail at: the frame's. The rail opens at
/// 180, so the baseline's window is dragged to it.
const BASELINE_RAIL: f32 = 236.0;

/// The words the four versions of the baseline's history read, newest first:
/// a text edited outside, a scale switch, a column put on x, and the first
/// version kept.
const BASELINE_CHANGES: [&str; 4] = [
    "edited outside brightfield \u{b7} 3 lines",
    "population \u{b7} x scale: linear \u{2192} log",
    "Map \u{b7} projection type: equirectangular removed \u{b7} \
     x axis: longitude \u{2192} median_income",
    "the first version kept",
];

/// **The panel over a history of four versions, as pixels at 236 high.** The
/// history holds a save, a scale switch, a column put on x and a version
/// edited outside.
///
/// A probe window is driven through the gestures that open the panel and drag
/// the rail to its height, and every frame it ran is replayed in the capture's
/// window over the same files, so the picture is of what the probe was read to
/// hold. The probe is read first: `UPDATE_SNAPSHOTS=1` writes whatever it is
/// handed, so a regeneration over a click that missed would commit a picture
/// of some other pane under this one's name.
fn capture_the_versions_panel(mode: Mode, name: &str) -> image::RgbaImage {
    let t = recorded_texts();
    let outside = format!("{}# a note\n# another\n# a third\n", t.scale_switched);
    let mut probe = Session::open_for_capture("baseline", BASELINE_WINDOW);
    probe.seed(&[
        (true, &t.base),
        (false, &t.x_moved),
        (false, &t.scale_switched),
        (true, &outside),
    ]);
    probe.pin_clock(SAME_DAY);
    probe.show_versions();
    probe.drag_ledger_to(BASELINE_RAIL);

    let cells = probe.cells();
    assert_eq!(
        cells.iter().map(|r| r[2].as_str()).collect::<Vec<_>>(),
        BASELINE_CHANGES,
        "the probe's panel does not list the history the baseline is of"
    );
    assert_eq!(
        cells.iter().map(|r| r[1].as_str()).collect::<Vec<_>>(),
        ["saved", "before a write", "before a write", "saved"],
        "the probe's rows are not the kinds the baseline is of"
    );
    assert!(
        cells.iter().all(|r| r[0] == "today 14:02"),
        "a row's time does not read the pinned clock: {cells:?}"
    );
    assert_eq!(
        probe.app.rail_pane_title(LEDGER_RAIL).as_deref(),
        Some("Versions")
    );

    let clock = probe.clock_reading(SAME_DAY);
    let home = probe.root.0.clone();
    let store = HistoryStore::At(probe.history_root.clone());
    let data = probe.folder.join(HOUSING_FILE);
    let boot = Boot::data_file(data.to_str().expect("utf-8 path"))
        .unwrap_or_else(|e| panic!("open {}: {e}", data.display()));
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.capture.png"));
    std::fs::create_dir_all(out.parent().expect("a parent")).expect("the capture's folder");
    let (w, h) = brightfield_shell::capture::capture_png_prepared(
        boot,
        brightfield_shell::startup::default_layout(),
        mode,
        1.0,
        BASELINE_WINDOW,
        &out,
        probe.frames.clone(),
        |app| {
            app.set_history(Some(store));
            app.set_versions_env(clock, Some(home));
        },
    )
    .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8()
}

/// **AC8, light.** The Versions panel over four versions matches its baseline
/// with the rail 236 high.
#[test]
fn the_versions_panel_light_baseline() {
    let image = capture_the_versions_panel(Mode::Light, "versions_panel_light");
    egui_kittest::image_snapshot(&image, "versions_panel_light");
}

/// **AC8, dark:** the same frame and the same script, the ink moved.
#[test]
fn the_versions_panel_dark_baseline() {
    let image = capture_the_versions_panel(Mode::Dark, "versions_panel_dark");
    egui_kittest::image_snapshot(&image, "versions_panel_dark");
}
