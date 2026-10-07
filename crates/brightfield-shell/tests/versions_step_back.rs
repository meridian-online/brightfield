//! **Enter on a version steps the chart back to it as an unsaved edit, and
//! Save writes it.**
//!
//! The Versions panel listed a chart's saved versions and had no key that drew
//! one or stepped the chart back to one. With the panel holding the keys, `j`
//! and `k` move a cursor over its rows and the chart is drawn as the version
//! under it; `Esc` draws the chart as it was; `Enter`, or the Step back control
//! on the cursor's row, steps the chart back to the version as an unsaved edit
//! that Save writes, and `u` takes the step back back.
//!
//! Each test drives a window by the gestures a person has — keys, clicks, the
//! palette's Save — and reads what it drew, the chart file's bytes and the
//! store. The versions are made by real Saves, so their texts are the texts a
//! person's gestures write. **No test writes under the home directory**: each
//! window is given a store inside its own temporary folder.

use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use brightfield_protocol::chart_history::LocalHistory;
use brightfield_protocol::HistoryStore;
use brightfield_shell::app::VERSION_REFUSED;
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::SpineRole;
use brightfield_shell::shelf::ShelfChannels;
use brightfield_shell::text_ink;
use brightfield_shell::versions::{Clock, Listing, STEP_BACK};
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK, VERSION_SHOWN_STATUS_ID};
use brightfield_spec::layout::ScaleType;
use brightfield_spec::{parse_spec, Format};
use brightfield_workbench::arrangement::LEDGER_RAIL;
use brightfield_workbench::channel::ShelfChannel;

const HOUSING_FILE: &str = "california_housing_sample.csv";

/// The chart file's name: the Protocol's name, which is the data file's stem.
const CHART_FILE: &str = "california_housing_sample.yaml";

/// Where the ledger strip's Versions name is: the fifth.
const VERSIONS_AT: usize = 4;

/// The local time the newest version is pinned to read, 14:02:50 on
/// 2026-10-05: late in its minute, so a version a few seconds older reads 14:02
/// too.
const NEWEST_READS: i64 = 20_731 * 86_400 + 14 * 3_600 + 2 * 60 + 50;

/// The local time the clock is pinned to read: three minutes on, the same day.
const NOW_READS: i64 = NEWEST_READS + 3 * 60;

/// The ledger rail's height in these windows: the design frame's. At the
/// rail's default 180 the second row sits under the status band.
const RAIL_HEIGHT: f32 = 236.0;

/// The words the status band says while the version saved at 14:02 is drawn.
const SHOWING: &str = "showing the version saved at 14:02 \u{b7} Enter steps back to it as an \
                       unsaved edit \u{b7} Esc returns to now";

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
            "bf-versions-step-back-{name}-{}-{}",
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

/// A window over the housing file copied into a folder of its own, with a
/// store at `<root>/.arcform/history`. It keeps one `egui::Context` for its
/// whole life, because a click is resolved against the widget id a previous
/// frame registered.
struct Session {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    folder: PathBuf,
    history_root: PathBuf,
    /// The text the frame before drew.
    texts: Vec<text_ink::DrawnText>,
    /// Every frame's events, in order, so a capture can replay them.
    frames: Vec<Vec<egui::Event>>,
    /// Whether the last frame sent the window a request to cancel a close.
    cancelled_close: bool,
    /// egui's input clock, a sixtieth of a second a frame: what a toast's
    /// lifetime is counted on.
    time: f64,
    root: TempDir,
}

impl Session {
    fn open(name: &str) -> Self {
        let root = TempDir::new(name);
        let folder = root.0.join("data");
        std::fs::create_dir_all(&folder).expect("the data file's folder");
        let data = folder.join(HOUSING_FILE);
        std::fs::copy(housing(), &data).expect("the housing fixture copies");
        let boot = Boot::data_file(data.to_str().expect("utf-8 path"))
            .unwrap_or_else(|e| panic!("open {}: {e}", data.display()));
        let history_root = root.0.join(".arcform").join("history");
        let app = MeridianApp::headless(boot, Mode::Light)
            .keeping_history(Some(HistoryStore::At(history_root.clone())));
        let mut session = Self {
            app,
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
            folder,
            history_root,
            texts: Vec::new(),
            frames: Vec::new(),
            cancelled_close: false,
            time: 0.0,
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

    fn chart_bytes(&self) -> Vec<u8> {
        std::fs::read(self.chart_file()).expect("the chart file reads")
    }

    fn chart_text(&self) -> String {
        std::fs::read_to_string(self.chart_file()).expect("the chart file reads")
    }

    fn history(&self) -> LocalHistory {
        LocalHistory::at_root(&self.history_root)
    }

    fn marked(&self) -> bool {
        self.app.title().contains(UNSAVED_MARK)
    }

    fn frame(&mut self, events: Vec<egui::Event>, close: bool) {
        self.frames.push(events.clone());
        self.time += 1.0 / 60.0;
        let mut raw = egui::RawInput {
            screen_rect: Some(self.screen),
            time: Some(self.time),
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
        self.cancelled_close = out
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|root| {
                root.commands
                    .iter()
                    .any(|c| matches!(c, egui::ViewportCommand::CancelClose))
            });
        self.texts = texts;
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        self.frame(events, false);
    }

    /// Let `secs` pass on egui's clock, and settle.
    fn wait(&mut self, secs: f64) {
        self.time += secs;
        self.settle();
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

    /// Save through the chart palette, as a person does.
    fn save(&mut self) {
        self.key(egui::Key::Space);
        assert_eq!(self.app.open_overlay(), Some("palette"));
        self.settle();
        self.run(vec![egui::Event::Text("save-spec".to_owned())]);
        self.run(Vec::new());
        self.key(egui::Key::Enter);
        assert_eq!(
            self.app.open_overlay(),
            None,
            "save-spec did not close the palette"
        );
        self.settle();
    }

    /// Pin the clock so the newest version reads 14:02 and *now* is the same day.
    fn pin_clock(&mut self) {
        let clock = self.clock();
        self.app.set_versions_env(clock, Some(self.root.0.clone()));
        self.app.chart_doc_mut().versions_mut().invalidate();
        self.settle();
    }

    fn clock(&self) -> Clock {
        let newest = self
            .history()
            .entries_for_file(&self.chart_file())
            .expect("the chart file's history lists")
            .last()
            .expect("a version is recorded")
            .at;
        let at = i64::try_from(newest.duration_since(UNIX_EPOCH).unwrap().as_secs()).unwrap();
        Clock::Fixed {
            now: newest + Duration::from_secs(u64::try_from(NOW_READS - NEWEST_READS).unwrap()),
            offset_secs: i32::try_from(NEWEST_READS - at).unwrap(),
        }
    }

    /// Open the ledger on Versions by its strip name, then give the panel the
    /// keys with a press in it, below its rows.
    fn hold_versions(&mut self) {
        let name = self
            .app
            .rail_name_rect(LEDGER_RAIL, VERSIONS_AT)
            .expect("the ledger strip drew a fifth name")
            .center();
        self.click(name);
        self.settle();
        self.drag_ledger_to(RAIL_HEIGHT);
        self.focus_versions();
    }

    /// Drag the ledger rail's top edge until the rail is `height` high.
    fn drag_ledger_to(&mut self, height: f32) {
        let rail = self
            .app
            .region_rect(LEDGER_RAIL)
            .expect("the ledger rail drew");
        let grab = egui::pos2(rail.center().x, rail.top());
        let to = egui::pos2(grab.x, grab.y - (height - rail.height()));
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.run(vec![egui::Event::PointerMoved(grab)]);
        self.run(vec![egui::Event::PointerMoved(grab), press(grab, true)]);
        self.run(vec![egui::Event::PointerMoved(to)]);
        self.run(vec![egui::Event::PointerMoved(to)]);
        self.run(vec![egui::Event::PointerMoved(to), press(to, false)]);
        self.settle();
        let now = self.app.region_rect(LEDGER_RAIL).unwrap().height();
        assert!(
            (now - height).abs() < 0.5,
            "the rail is {now} high, not {height}"
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
    }

    /// Give the open Versions panel the keys, with a press on its first row.
    fn focus_versions(&mut self) {
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

    /// The listed rows' ids, newest first.
    fn row_ids(&self) -> Vec<String> {
        match self.app.chart_doc().versions().listing() {
            Listing::Listed { rows, .. } => rows.iter().map(|r| r.id.clone()).collect(),
            other => panic!("the panel lists no rows: {other:?}"),
        }
    }

    /// The version under the cursor's place among the rows.
    fn cursor_at(&self) -> Option<usize> {
        self.app
            .chart_doc()
            .versions()
            .cursor_row()
            .map(|(at, _)| at)
    }

    /// The hero's channels on the page drawn: the columns its x, y and colour
    /// carry. The window lays the page out at its own widths, so the spec the
    /// page was loaded from differs from a version's text in a width; the
    /// columns are what a version carries and the window does not rewrite.
    fn drawn(&self) -> String {
        hero(
            self.app.chart_doc().live_spec().expect("a live page"),
            &self.hero_path(),
        )
    }

    /// The hero's plot path, as the page drawn composes it.
    fn hero_path(&self) -> String {
        self.app.chart_doc().composed.plots[0].path.clone()
    }

    /// The hero's channels in the chart `text` parses to.
    fn version_hero(&self, text: &str) -> String {
        let spec = parse_spec(text, Format::Yaml).expect("a chart text").spec;
        hero(&spec, &self.hero_path())
    }

    /// The text the store recorded for `id`.
    fn version_text(&self, id: &str) -> String {
        self.history()
            .read_for_file(&self.chart_file(), id)
            .expect("the version reads")
    }

    fn drew(&self, text: &str) -> bool {
        self.texts.iter().any(|t| t.text == text)
    }
}

/// The channels of the plot at `path` in `spec`: its x, y and colour columns.
fn hero(spec: &brightfield_spec::ast::Spec, path: &str) -> String {
    let plot = brightfield_spec::edit::plot_at_path(spec, path).expect("the hero's plot");
    let c = ShelfChannels::of_plot(plot).expect("the hero's channels");
    format!(
        "x {:?} \u{b7} y {:?} \u{b7} colour {:?}",
        c.x, c.y, c.colour
    )
}

/// A window with two versions: the chart as generated, then `median_income`
/// put on x, each saved. The file holds the second.
fn two_versions(name: &str) -> Session {
    let mut s = Session::open(name);
    s.save();
    s.put("median_income", ShelfChannel::X);
    s.save();
    assert!(!s.marked());
    s.pin_clock();
    s
}

// ---------------------------------------------------------------------------
// AC1 — the cursor moves a row, stops at the ends, and draws the version
// ---------------------------------------------------------------------------

/// **AC1.** `j` and `k` and the arrows move the cursor a row and stop at the
/// first and last rows; the hero is drawn as the version under it, its header
/// ends `as saved 14:02`, and the status band says which version is shown and
/// names `Enter` and `Esc`.
#[test]
fn j_and_k_move_the_cursor_and_draw_the_chart_as_the_version_under_it() {
    let mut s = two_versions("ac1");
    let now = s.drawn();
    s.hold_versions();
    let ids = s.row_ids();
    assert_eq!(ids.len(), 2, "two Saves did not list two versions");
    assert_eq!(s.cursor_at(), None, "the cursor is on a row before a key");

    s.key(egui::Key::J);
    assert_eq!(s.cursor_at(), Some(0));
    s.key(egui::Key::J);
    assert_eq!(s.cursor_at(), Some(1));
    assert_eq!(
        s.drawn(),
        s.version_hero(&s.version_text(&ids[1])),
        "not drawn as the oldest"
    );
    s.key(egui::Key::ArrowDown);
    assert_eq!(s.cursor_at(), Some(1), "the cursor went past the last row");
    assert_ne!(s.drawn(), now, "the oldest version drew as the chart now");
    assert!(
        s.texts
            .iter()
            .any(|t| t.text.starts_with("Map") && t.text.ends_with(" \u{b7} as saved 14:02")),
        "the hero's header does not end `as saved 14:02`"
    );
    assert!(s.app.rail().drawn.contains(&VERSION_SHOWN_STATUS_ID));
    assert!(
        s.drew(SHOWING),
        "the status band does not say which version is shown"
    );

    s.key(egui::Key::K);
    assert_eq!(s.cursor_at(), Some(0));
    assert_eq!(s.drawn(), s.version_hero(&s.version_text(&ids[0])));
    s.key(egui::Key::ArrowUp);
    assert_eq!(s.cursor_at(), Some(0), "the cursor went past the first row");
    s.key(egui::Key::ArrowDown);
    assert_eq!(
        s.cursor_at(),
        Some(1),
        "the down arrow did not move the cursor"
    );
    assert!(!s.marked(), "drawing a version marked the window unsaved");
}

// ---------------------------------------------------------------------------
// AC2 — Esc draws the chart as it was, unsaved edit and mark included
// ---------------------------------------------------------------------------

/// **AC2.** `Esc` draws the chart as it was before the cursor moved, with the
/// unsaved edit it held, and the window's unsaved mark as it was.
#[test]
fn esc_draws_the_chart_as_it_was_with_its_unsaved_edit_and_mark() {
    let mut s = two_versions("ac2");
    s.put("median_house_value", ShelfChannel::Y);
    let now = s.drawn();
    let held = s.app.chart_doc().unsaved_edits().to_vec();
    s.hold_versions();
    s.key(egui::Key::J);
    s.key(egui::Key::J);
    assert_ne!(s.drawn(), now, "the cursor drew no version");
    assert!(s.marked(), "drawing a version took the unsaved mark away");

    s.key(egui::Key::Escape);
    assert_eq!(s.drawn(), now, "Esc did not draw the chart as it was");
    assert_eq!(s.app.chart_doc().unsaved_edits(), held.as_slice());
    assert!(s.marked(), "Esc took the unsaved mark away");
    assert_eq!(s.cursor_at(), None, "Esc left the cursor on a row");
    assert!(!s.app.rail().drawn.contains(&VERSION_SHOWN_STATUS_ID));
}

// ---------------------------------------------------------------------------
// AC3 and AC4 — Enter steps back as an unsaved edit; Save writes it
// ---------------------------------------------------------------------------

/// **AC3 and AC4.** `Enter` on an older version leaves it drawn, marks the
/// window unsaved and leaves the chart file byte-identical; a Save then writes
/// the version's recorded bytes, clears the mark, and lists a newest row with
/// the version stepped back to and the one newest before it still listed.
#[test]
fn enter_steps_back_as_an_unsaved_edit_and_save_writes_the_versions_bytes() {
    let mut s = two_versions("ac3");
    let before = s.chart_bytes();
    s.hold_versions();
    let ids = s.row_ids();
    s.key(egui::Key::J);
    s.key(egui::Key::J);
    s.key(egui::Key::Enter);
    let version = s.version_text(&ids[1]);
    assert_eq!(
        s.drawn(),
        s.version_hero(&version),
        "Enter did not leave the version drawn"
    );
    assert!(s.marked(), "Enter did not mark the window unsaved");
    assert_eq!(s.chart_bytes(), before, "Enter wrote the chart file");

    s.save();
    assert_eq!(
        s.chart_text(),
        version,
        "Save did not write the version's bytes"
    );
    assert!(!s.marked(), "Save left the unsaved mark");
    let after = s.row_ids();
    assert_eq!(
        after.len(),
        3,
        "the Save did not list a newest row: {after:?}"
    );
    assert!(!ids.contains(&after[0]), "the newest row is not the Save's");
    assert_eq!(
        &after[1..],
        ids.as_slice(),
        "the earlier versions are not still listed"
    );
}

/// **AC3, the control.** A click on the Step back control the cursor's row
/// drew does what `Enter` does, and the control carries its name.
#[test]
fn a_click_on_the_step_back_control_steps_back() {
    let mut s = two_versions("ac3-click");
    let before = s.chart_bytes();
    s.hold_versions();
    let ids = s.row_ids();
    s.key(egui::Key::J);
    s.key(egui::Key::J);
    let at = s
        .app
        .chart_doc()
        .versions()
        .step_back_drawn()
        .expect("the cursor's row drew no Step back control");
    assert!(
        s.app
            .named_controls()
            .iter()
            .any(|c| c.name == STEP_BACK && c.rect == at),
        "the Step back control carries no name"
    );
    // The Save's confirmation toast stands over the foot of the window until
    // it expires.
    s.wait(10.0);
    s.click(at.center());
    s.settle();
    assert!(s.marked(), "the click did not step back");
    assert_eq!(s.drawn(), s.version_hero(&s.version_text(&ids[1])));
    assert_eq!(s.chart_bytes(), before, "the click wrote the chart file");
}

// ---------------------------------------------------------------------------
// AC5 — an edit after the step back goes on its own line of the version
// ---------------------------------------------------------------------------

/// **AC5.** A switch thrown after a step back and before Save is written into
/// the version's text on its own line, every other line byte-identical.
#[test]
fn an_edit_after_a_step_back_is_written_into_the_versions_text_on_its_own_line() {
    let mut s = two_versions("ac5");
    s.hold_versions();
    let ids = s.row_ids();
    s.key(egui::Key::J);
    s.key(egui::Key::J);
    s.key(egui::Key::Enter);
    s.transpose();
    s.throw("population", ScaleType::Log);
    s.save();
    let version = s.version_text(&ids[1]);
    let written = s.chart_text();
    let a: Vec<&str> = version.split_inclusive('\n').collect();
    let b: Vec<&str> = written.split_inclusive('\n').collect();
    assert_eq!(b.len(), a.len() + 1, "the switch did not add one line");
    let at = (0..a.len()).find(|&i| a[i] != b[i]).unwrap_or(a.len());
    assert!(
        b[at].contains("Scale: log"),
        "the line added reads {:?}",
        b[at]
    );
    let rest: Vec<&str> = b[..at].iter().chain(&b[at + 1..]).copied().collect();
    assert_eq!(
        rest, a,
        "a line other than the switch's differs from the version's"
    );
}

// ---------------------------------------------------------------------------
// AC6 — u after a step back draws the unsaved edit it went over
// ---------------------------------------------------------------------------

/// **AC6.** With an unsaved edit held, `Enter` on an earlier version and then
/// `u` draws the chart with that edit again, and holds it for Save.
#[test]
fn enter_on_an_earlier_version_then_u_draws_the_unsaved_edit_again() {
    let mut s = two_versions("ac6");
    s.put("median_house_value", ShelfChannel::Y);
    let edited = s.drawn();
    s.hold_versions();
    s.key(egui::Key::J);
    s.key(egui::Key::J);
    s.key(egui::Key::Enter);
    assert_ne!(s.drawn(), edited);

    s.key(egui::Key::U);
    assert_eq!(s.drawn(), edited, "u did not draw the unsaved edit again");
    assert!(s.marked());
    assert_eq!(
        s.app.chart_doc().stepped_back_to(),
        None,
        "u left the step back held"
    );
    s.save();
    let written = s.chart_text();
    assert!(
        written.contains("median_house_value") && written.contains("median_income"),
        "the Save after u does not write the edit the step back went over"
    );
}

// ---------------------------------------------------------------------------
// AC7 — a close after a step back asks
// ---------------------------------------------------------------------------

/// **AC7.** A close request after a step back and before Save asks the close
/// question, and the window is not closed.
#[test]
fn a_close_after_a_step_back_asks_the_close_question() {
    let mut s = two_versions("ac7");
    s.hold_versions();
    s.key(egui::Key::J);
    s.key(egui::Key::J);
    s.key(egui::Key::Enter);
    assert!(
        s.app.chart_doc().unsaved_edits().is_empty(),
        "the step back held an edit"
    );
    s.frame(Vec::new(), true);
    assert!(s.cancelled_close, "the close request went through");
    assert_eq!(s.app.open_overlay(), Some("close-question"));
}

// ---------------------------------------------------------------------------
// AC8 — a version that does not load is not drawn
// ---------------------------------------------------------------------------

/// **AC8.** A version whose text does not load as a chart is not drawn: the
/// chart stays as it was, the window says why, and `Enter` does not step back.
#[test]
fn a_version_that_does_not_load_is_not_drawn_and_the_window_says_why() {
    let mut s = two_versions("ac8");
    s.history()
        .record_checkpoint_for_file(&s.chart_file(), "{ this is not: a chart\n")
        .expect("the bad version records");
    s.pin_clock();
    let now = s.drawn();
    s.hold_versions();
    s.key(egui::Key::J);
    assert_eq!(s.cursor_at(), Some(0));
    assert_eq!(s.drawn(), now, "a version that does not load was drawn");
    let fault = s
        .app
        .chart_doc()
        .chart_fault()
        .expect("the window says nothing");
    assert_eq!(fault.title, VERSION_REFUSED);
    assert!(
        fault.detail.contains("does not read as a chart"),
        "{}",
        fault.detail
    );
    assert!(s.drew(VERSION_REFUSED), "the window drew no word of why");

    s.key(egui::Key::Enter);
    assert!(
        !s.marked(),
        "Enter stepped back to a version that does not load"
    );
    assert_eq!(s.drawn(), now);
}

// ---------------------------------------------------------------------------
// AC9 — the panel with the cursor on a version, as pixels
// ---------------------------------------------------------------------------

/// **The panel with the cursor on the older of two versions**, the Step back
/// control on its row and the chart drawn as it, as pixels. A probe window is
/// driven there and every frame it ran is replayed in the capture's window over
/// the same files and store, so the picture is of what the probe was read to
/// hold.
fn capture_the_cursor(mode: Mode, name: &str) -> image::RgbaImage {
    // The probe draws the shelf band, whose Outline chips put the column its
    // second Save writes; the frames replayed after the Saves touch only the
    // ledger rail, which the band does not move.
    let mut probe = Session::open("baseline");
    probe.save();
    probe.put("median_income", ShelfChannel::X);
    probe.save();
    // The capture's window opens on the chart file those Saves wrote and the
    // store they recorded into, so it replays only the frames after them and
    // records no third version.
    let frames_before = probe.frames.len();
    probe.pin_clock();
    probe.hold_versions();
    probe.key(egui::Key::J);
    probe.key(egui::Key::J);
    probe.point(egui::pos2(1.0, 1.0));
    assert_eq!(
        probe.cursor_at(),
        Some(1),
        "the probe's cursor is not on the older row"
    );
    assert!(probe.app.chart_doc().versions().step_back_drawn().is_some());
    assert!(probe.drew(SHOWING));

    let clock = probe.clock();
    let home = probe.root.0.clone();
    let store = HistoryStore::At(probe.history_root.clone());
    let data = probe.folder.join(HOUSING_FILE);
    let boot = Boot::data_file(data.to_str().expect("utf-8 path")).expect("the data file opens");
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.capture.png"));
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    // The probe's Save is in its first frames; the capture's window opens on
    // the chart file that Save wrote, so it replays only the frames after it.
    let frames = probe.frames[frames_before..].to_vec();
    let (w, h) = brightfield_shell::capture::capture_png_prepared(
        boot,
        brightfield_shell::startup::default_layout(),
        mode,
        1.0,
        (1440.0, 900.0),
        &out,
        frames,
        |app| {
            app.set_history(Some(store));
            app.set_versions_env(clock, Some(home));
        },
    )
    .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    image::open(&out).unwrap().to_rgba8()
}

/// **AC9, light.**
#[test]
fn the_versions_cursor_light_baseline() {
    let image = capture_the_cursor(Mode::Light, "versions_step_back_light");
    egui_kittest::image_snapshot(&image, "versions_step_back_light");
}

/// **AC9, dark:** the same frames, the ink moved.
#[test]
fn the_versions_cursor_dark_baseline() {
    let image = capture_the_cursor(Mode::Dark, "versions_step_back_dark");
    egui_kittest::image_snapshot(&image, "versions_step_back_dark");
}
