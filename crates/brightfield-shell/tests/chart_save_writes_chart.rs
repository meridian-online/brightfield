//! **Save writes the chart beside the Protocol, and each edit goes in as one
//! line of the chart file's own text.**
//!
//! Save wrote `arcform.yaml` and its model into the data file's folder and no
//! chart, so a scale or normalise switch was on the screen until the window
//! closed and in no file. It now also writes `panels/<name>.yaml` in that
//! folder: the text the generator wrote at open on a first Save, and the edits
//! made since the last Save placed into the text on disk on every Save after.
//!
//! The tests here drive Save through the gesture a person has — the chart palette
//! on `space`, `save-spec` typed, enter — because a direct call to
//! `save_protocol` proves the method and not the product
//! (`tests/saved_protocol_working_directory.rs` says why), and reads the
//! result off the disk and the window title rather than off the flag behind
//! them.
//!
//! # Which switches, and which gestures
//!
//! A scale switch a generated dashboard draws sits on its tile's binned
//! axis, which for the housing file is x, so the gesture writes `xScale`. A
//! `yScale` is written through `ChartDoc::set_plot_scale`, the entry point
//! `tests/chart_save_unsaved_mark.rs` drives for the same reason. No generated
//! tile carries a colour group, so the normalise control is not drawn in a
//! data-file window, and `stackOffset` is written through
//! `ChartDoc::set_plot_stack_offset` in the same way.
//!
//! # What is covered and what is not
//!
//! Covered: the chart file's place and its difference from the generated text
//! (one line), a comment typed into it surviving the next Save, the edits of
//! one Save not being replayed by the next, a Save with no edit on a folder
//! with and without a chart file, the unsaved mark clearing on a write and
//! staying on a failure with the window saying why, a chart file that no longer
//! holds the edit's plot being left alone, a window closed without a Save
//! leaving the folder as it found it, and the editor pane showing the saved
//! file. Not covered: reopening the saved chart, which is a later card's, and a
//! chart edit that is not a scale or normalise switch.

use std::path::{Path, PathBuf};

use brightfield_shell::app::GridLayout;
use brightfield_shell::design::Mode;
use brightfield_shell::editor::EDITOR;
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_spec::layout::{PlotAxis, ScaleType, StackOffset};
use brightfield_workbench::arrangement::LEDGER_RAIL;
use brightfield_workbench::PaneKey;

const HOUSING_FILE: &str = "california_housing_sample.csv";

/// The chart file's name: the Protocol's name, which is the data file's stem.
const CHART_FILE: &str = "california_housing_sample.yaml";

/// The banner's headline when a Save could not write the chart.
const BANNER: &str = "Could not save this chart";

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
            "bf-chart-save-{name}-{}-{}",
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

/// A window over the housing file copied into a folder of its own, so a Save
/// has somewhere to write that is not the repository.
///
/// It keeps one `egui::Context` for its whole life, because a click is
/// resolved against the widget id a *previous* frame registered.
struct Session {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    /// The data file's folder: where `arcform.yaml` and `panels/` land.
    folder: PathBuf,
    /// The text the generator wrote at open, read off the spec file the window
    /// opened the document from.
    generated: String,
    /// Held last so the folder outlives the window.
    _root: TempDir,
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
        let mut session = Self {
            app: MeridianApp::headless(boot, Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
            folder,
            generated: String::new(),
            _root: root,
        };
        session.settle();
        let spec = session
            .app
            .chart_doc()
            .spec_path
            .clone()
            .expect("a generated dashboard carries the spec file it was composed from");
        session.generated = std::fs::read_to_string(&spec)
            .unwrap_or_else(|e| panic!("read the generated spec {}: {e}", spec.display()));
        session
    }

    fn chart_file(&self) -> PathBuf {
        self.folder.join("panels").join(CHART_FILE)
    }

    fn chart_text(&self) -> String {
        std::fs::read_to_string(self.chart_file())
            .unwrap_or_else(|e| panic!("read the chart file {}: {e}", self.chart_file().display()))
    }

    /// Every file under the data file's folder, as a path relative to it.
    fn files(&self) -> Vec<String> {
        fn walk(dir: &Path, base: &Path, into: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("a readable folder").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, base, into);
                } else {
                    into.push(
                        path.strip_prefix(base)
                            .expect("under the folder")
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
            }
        }
        let mut found = Vec::new();
        walk(&self.folder, &self.folder, &mut found);
        found.sort();
        found
    }

    fn title(&self) -> String {
        self.app.title()
    }

    fn marked(&self) -> bool {
        self.title().contains(UNSAVED_MARK)
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            events,
            ..Default::default()
        };
        let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
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

    /// Put the grid in its columns layout, the one where each histogram tile
    /// is on screen with its switch — `tests/chart_save_unsaved_mark.rs`'s
    /// `transpose`.
    fn transpose(&mut self) {
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
            self.title()
        );
    }

    /// The plot the tile for `column` draws on.
    fn plot_of(&self, column: &str) -> usize {
        self.app
            .chart_doc()
            .scale_switches
            .iter()
            .find(|s| s.column == column)
            .unwrap_or_else(|| panic!("no scale switch for {column:?}"))
            .plot
    }

    /// Write `yScale: log` onto `column`'s tile through the document's entry
    /// point — no tile draws a y switch.
    fn y_log(&mut self, column: &str) {
        let plot = self.plot_of(column);
        assert!(self
            .app
            .chart_doc_mut()
            .set_plot_scale(plot, PlotAxis::Y, ScaleType::Log));
        self.settle();
    }

    /// Write `stackOffset: normalize` onto `column`'s tile through the
    /// document's entry point — no generated tile draws the control.
    fn normalise(&mut self, column: &str) {
        let plot = self.plot_of(column);
        assert!(self
            .app
            .chart_doc_mut()
            .set_plot_stack_offset(plot, StackOffset::Normalize));
        self.settle();
    }

    /// **Save, through the gesture a person has**: the chart palette on
    /// `space`, the verb typed, confirmed with enter —
    /// `tests/saved_protocol_working_directory.rs`'s `save_through_the_palette`.
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

    /// The headline and body of every banner the window is showing.
    fn banners(&self) -> Vec<(String, String)> {
        self.app
            .notifications()
            .iter()
            .map(|n| (n.title.clone(), n.body.clone().unwrap_or_default()))
            .collect()
    }

    /// The body of the banner that says the chart could not be saved, if the
    /// window is showing one.
    fn chart_banner(&self) -> Option<String> {
        self.banners()
            .into_iter()
            .find(|(title, _)| title == BANNER)
            .map(|(_, body)| body)
    }

    /// Every string the next frame hands the painter inside `rect`.
    fn drawn_text_in(&mut self, rect: egui::Rect) -> Vec<String> {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            ..Default::default()
        };
        let out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        let mut text = Vec::new();
        for clipped in &out.shapes {
            collect_text_in(&clipped.shape, rect, &mut text);
        }
        text
    }
}

fn collect_text_in(shape: &egui::epaint::Shape, rect: egui::Rect, into: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Text(t) if rect.contains(t.pos) => {
            into.push(t.galley.text().to_string());
        }
        egui::epaint::Shape::Vec(shapes) => {
            for s in shapes {
                collect_text_in(s, rect, into);
            }
        }
        _ => {}
    }
}

/// The one line that separates `after` from `before`, asserting that it is the
/// only thing that does: either a line `after` has inserted, or a line it has
/// replaced, with every other line and every byte of line ending the same.
fn the_one_changed_line(before: &str, after: &str) -> String {
    let b: Vec<&str> = before.lines().collect();
    let a: Vec<&str> = after.lines().collect();
    assert_eq!(
        before.ends_with('\n'),
        after.ends_with('\n'),
        "the two texts end differently"
    );
    let first = b.iter().zip(&a).take_while(|(x, y)| x == y).count();
    if a.len() == b.len() + 1 {
        assert_eq!(
            b[first..],
            a[first + 1..],
            "more than one line changed: a line was inserted at {first} and the rest differs"
        );
        assert_eq!(
            before.len() + a[first].len() + 1,
            after.len(),
            "the inserted line is not the only difference in bytes"
        );
    } else if a.len() == b.len() && first < a.len() {
        assert_eq!(
            b[first + 1..],
            a[first + 1..],
            "more than one line changed: line {first} was replaced and the rest differs"
        );
    } else {
        panic!(
            "the texts differ by more than one line: {} lines became {}",
            b.len(),
            a.len()
        );
    }
    a[first].to_string()
}

/// The generator's comment lines: the header and the one above each tile.
fn comment_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|l| l.trim_start().starts_with('#'))
        .collect()
}

/// A comment the analyst types into the chart file, put on its own line above
/// the `meta:` block.
const NOTE: &str = "# the income tile is the one I trust";

/// Type [`NOTE`] into the chart file on disk, as an editor's save would, and
/// give back the text as it now stands.
fn type_a_note_into(session: &Session) -> String {
    let saved = session.chart_text();
    let typed = saved.replacen("meta:\n", &format!("{NOTE}\nmeta:\n"), 1);
    assert_ne!(
        typed, saved,
        "the fixture has no `meta:` line to type above"
    );
    std::fs::write(session.chart_file(), &typed).expect("the analyst's edit is written");
    typed
}

// ---------------------------------------------------------------------------
// AC1 — the chart file is written, and differs from the generated text by the
// one line the edit changed
// ---------------------------------------------------------------------------

/// The shared body of the two AC1 tests: the chart is in no file until Save,
/// and after it is in `panels/` beside `arcform.yaml`, differing from the text
/// the generator wrote by the one line `expected`, with every comment kept.
fn assert_saved_as_the_generated_text_plus(session: &mut Session, expected: &str) {
    assert!(
        !session.chart_file().exists(),
        "a chart file is there before any Save"
    );
    session.save();

    assert!(
        session.folder.join("arcform.yaml").is_file(),
        "Save wrote no Protocol beside the data file: {:?}",
        session.files()
    );
    assert!(
        session.chart_file().is_file(),
        "Save wrote no chart file at {}: {:?}",
        session.chart_file().display(),
        session.files()
    );
    let chart = session.chart_text();
    assert_eq!(
        the_one_changed_line(&session.generated, &chart).trim(),
        expected
    );

    let comments = comment_lines(&session.generated);
    assert!(
        comments.len() > 1,
        "the generator wrote no header and tile comments to keep: {comments:?}"
    );
    for comment in comments {
        assert!(
            chart.lines().any(|l| l == comment),
            "the generator's comment {comment:?} is not in the chart file"
        );
    }
}

/// **AC1, through the gesture.** A tile's scale switch thrown to log and
/// Saved writes the chart beside the Protocol with `xScale: log` on its own
/// line — the key a generated tile's switch writes, its binned axis being x.
#[test]
fn a_switch_thrown_before_save_is_written_beside_the_protocol_as_one_line() {
    let mut session = Session::open("ac1-gesture");
    session.transpose();
    session.throw("population", ScaleType::Log);

    assert_saved_as_the_generated_text_plus(&mut session, "xScale: log");
}

/// **AC1, `yScale`.** The same, through the document's entry point, for the
/// key the card names and no generated tile's switch writes.
#[test]
fn a_y_scale_written_before_save_is_written_beside_the_protocol_as_one_line() {
    let mut session = Session::open("ac1-y");
    session.y_log("population");

    assert_saved_as_the_generated_text_plus(&mut session, "yScale: log");
}

// ---------------------------------------------------------------------------
// AC2, AC3 — a second Save keeps a comment typed into the file, and changes
// only the line the second edit is about
// ---------------------------------------------------------------------------

/// The shared body of AC2 and AC3: one edit and a Save; a note typed into the
/// chart file; a second edit on another tile and a Save. The second Save's
/// chart file is the file as it stood, plus the one line `expected`.
fn two_saves_with_a_note_between(
    session: &mut Session,
    first: impl FnOnce(&mut Session),
    second: impl FnOnce(&mut Session),
    expected: &str,
) {
    first(session);
    session.save();
    let typed = type_a_note_into(session);

    second(session);
    session.save();

    let after = session.chart_text();
    assert!(
        after.lines().any(|l| l == NOTE),
        "the note typed into the chart file is not in it after the Save"
    );
    assert_eq!(the_one_changed_line(&typed, &after).trim(), expected);
}

/// **AC2.** A comment typed into the chart file is in it after the next Save,
/// and every byte outside the line the second switch changed is the file as it
/// stood.
#[test]
fn a_note_typed_into_the_chart_file_survives_the_next_save() {
    let mut session = Session::open("ac2");
    session.transpose();
    two_saves_with_a_note_between(
        &mut session,
        |s| s.throw("population", ScaleType::Log),
        |s| s.throw("house_age", ScaleType::Log),
        "xScale: log",
    );
}

/// **AC3.** The same two steps with the normalise switch change the one line
/// that carries `stackOffset`.
#[test]
fn the_normalise_switch_changes_the_one_stack_offset_line_on_each_save() {
    let mut session = Session::open("ac3");
    two_saves_with_a_note_between(
        &mut session,
        |s| s.normalise("median_income"),
        |s| s.normalise("house_age"),
        "stackOffset: normalize",
    );
}

/// **The edits since the last Save, and no others.** After a Save, an analyst
/// who changes the line that Save wrote and throws a second switch has the
/// second edit written and their own change left standing: the first edit is
/// not placed again over it.
#[test]
fn a_save_places_only_the_edits_made_since_the_last_one() {
    let mut session = Session::open("since-last");
    session.transpose();
    session.throw("population", ScaleType::Log);
    session.save();
    let saved = session.chart_text();
    assert!(saved.contains("xScale: log"));

    // The analyst puts the population tile's scale back by hand.
    let by_hand = saved.replacen("xScale: log", "xScale: linear", 1);
    std::fs::write(session.chart_file(), &by_hand).expect("the analyst's edit is written");

    session.throw("house_age", ScaleType::Log);
    session.save();

    let after = session.chart_text();
    assert_eq!(
        the_one_changed_line(&by_hand, &after).trim(),
        "xScale: log",
        "the second Save changed something other than the second tile's line"
    );
    assert!(
        after.contains("xScale: linear"),
        "the first edit was placed again over the analyst's own change"
    );
}

// ---------------------------------------------------------------------------
// AC4 — a Save with no edit
// ---------------------------------------------------------------------------

/// **AC4, no chart file.** A Save with no edit writes the generator's text as
/// it was written at open.
#[test]
fn a_save_with_no_edit_writes_the_generators_text_when_no_chart_file_is_there() {
    let mut session = Session::open("ac4-none");
    assert!(!session.marked());
    session.save();

    assert_eq!(
        session.chart_text(),
        session.generated,
        "the chart file is not the text the generator wrote at open"
    );
}

/// **AC4, a chart file there.** A Save with no edit leaves a chart file that is
/// there byte-identical — and does not put the generator's text over it.
#[test]
fn a_save_with_no_edit_leaves_a_chart_file_that_is_there_byte_identical() {
    let mut session = Session::open("ac4-there");
    let theirs = format!("# a chart an analyst kept\n{}", session.generated);
    std::fs::create_dir_all(session.folder.join("panels")).expect("panels/");
    std::fs::write(session.chart_file(), &theirs).expect("the chart file");

    session.save();

    assert_eq!(
        session.chart_text(),
        theirs,
        "a Save with no edit changed a chart file that was there"
    );
}

// ---------------------------------------------------------------------------
// AC5 — the unsaved mark, and what the window says when it stays
// ---------------------------------------------------------------------------

/// **AC5, a write.** The title carries the mark after a switch and carries none
/// after the Save that wrote the chart.
#[test]
fn a_save_that_wrote_the_chart_clears_the_unsaved_mark() {
    let mut session = Session::open("ac5-clears");
    session.transpose();
    session.throw("population", ScaleType::Log);
    assert!(session.marked(), "the switch left the title unmarked");

    session.save();

    assert!(
        !session.marked(),
        "the title is {:?} after a Save that wrote the chart",
        session.title()
    );
    assert_eq!(session.chart_banner(), None);
}

/// **AC5, a failure to read.** A Save that could not write the chart leaves
/// the mark and says why, and the next Save that can clears both.
///
/// The failure is a regular file where the `panels/` folder belongs, so the
/// text the edit goes into cannot be read; the Protocol beside it is written
/// all the same, which is the point of keeping the two apart. The failure of
/// the write itself, after the text is read and the edit placed, is the next
/// test's.
#[test]
fn a_save_that_could_not_write_the_chart_keeps_the_mark_and_says_why() {
    let mut session = Session::open("ac5-keeps");
    session.transpose();
    session.throw("population", ScaleType::Log);
    let blocker = session.folder.join("panels");
    std::fs::write(&blocker, "not a folder").expect("a file where panels/ belongs");

    session.save();

    assert!(
        session.marked(),
        "the title is {:?} after a Save that could not write the chart",
        session.title()
    );
    let said = session.chart_banner().unwrap_or_else(|| {
        panic!(
            "no banner says the chart was not saved: {:?}",
            session.banners()
        )
    });
    assert!(
        said.contains("panels"),
        "the banner does not say where the write failed: {said:?}"
    );
    assert!(
        session.folder.join("arcform.yaml").is_file(),
        "the Protocol was not written beside a chart that could not be"
    );

    std::fs::remove_file(&blocker).expect("the blocker is removed");
    session.save();

    assert!(
        !session.marked(),
        "the title is {:?} after a Save that could",
        session.title()
    );
    assert_eq!(session.chart_banner(), None, "the banner stayed up");
    assert!(session.chart_text().contains("xScale: log"));
}

/// Puts a directory's permissions back when dropped, so a test that made one
/// read-only leaves a folder its `TempDir` can remove even when an assertion
/// fails.
#[cfg(unix)]
struct Restore(PathBuf);

#[cfg(unix)]
impl Drop for Restore {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}

/// **AC5, a failure to write.** The edit is read, placed and ready, and the
/// write into `panels/` is refused: the mark stays and the window says why.
///
/// The `panels/` folder is there and read-only, so no chart file is there to
/// read, the scratch spec is read instead, the edit goes into it, and creating
/// the file is what fails. A window that cleared the held edits before the
/// write landed would lose the mark here and lose the edit with it; the read
/// failure above cannot show that, because it returns before the write. A
/// process that ignores directory permissions cannot make this failure, so the
/// test says so and stands down there.
#[cfg(unix)]
#[test]
fn a_save_whose_write_is_refused_keeps_the_mark_and_says_why() {
    use std::os::unix::fs::PermissionsExt;

    let mut session = Session::open("ac5-write");
    session.transpose();
    session.throw("population", ScaleType::Log);
    let panels = session.folder.join("panels");
    std::fs::create_dir_all(&panels).expect("panels/");
    let _restore = Restore(panels.clone());
    std::fs::set_permissions(&panels, std::fs::Permissions::from_mode(0o555))
        .expect("panels/ is made read-only");
    let probe = panels.join(".probe");
    if std::fs::write(&probe, "").is_ok() {
        let _ = std::fs::remove_file(&probe);
        eprintln!("this process ignores directory permissions; the write cannot be refused");
        return;
    }

    session.save();

    assert!(
        session.marked(),
        "the title is {:?} after a write that was refused",
        session.title()
    );
    let said = session.chart_banner().unwrap_or_else(|| {
        panic!(
            "no banner says the chart was not saved: {:?}",
            session.banners()
        )
    });
    assert!(
        said.contains("panels"),
        "the banner does not say where the write failed: {said:?}"
    );
    assert!(!session.chart_file().exists());

    std::fs::set_permissions(&panels, std::fs::Permissions::from_mode(0o755))
        .expect("panels/ is made writable again");
    session.save();

    assert!(
        !session.marked(),
        "the edit was lost with the refused write: the title is {:?} after a Save that could",
        session.title()
    );
    assert!(
        session.chart_text().contains("xScale: log"),
        "the edit held across the refused write is not in the chart file"
    );
}

// ---------------------------------------------------------------------------
// AC6 — a chart file that no longer holds the plot
// ---------------------------------------------------------------------------

/// **AC6.** When the chart file on disk no longer holds the plot an edit is
/// about, Save leaves it byte-identical, keeps the mark, and names the edit it
/// could not place.
///
/// The analyst has replaced the saved chart with a single plot; the window's
/// picture still has every tile, and a switch thrown on one of them is an edit
/// the file has nowhere to put. The edit that could not be placed is the
/// second Save's sole edit, so the banner naming the `house_age` tile is the
/// edit named, not a leftover.
#[test]
fn a_chart_file_that_lost_the_plot_is_left_byte_identical_and_the_edit_is_named() {
    let mut session = Session::open("ac6");
    session.transpose();
    session.throw("population", ScaleType::Log);
    session.save();
    assert!(!session.marked());

    let one_plot = "\
data:
  opened:
    file: 'elsewhere.csv'
plot:
  - mark: dot
    data: { from: opened }
    x: 'a'
    y: 'b'
";
    std::fs::write(session.chart_file(), one_plot).expect("the analyst's rewrite");
    session.throw("house_age", ScaleType::Log);

    session.save();

    assert_eq!(
        session.chart_text(),
        one_plot,
        "a Save that could not place the edit changed the chart file"
    );
    assert!(
        session.marked(),
        "the mark cleared over an edit that was not written"
    );
    let said = session
        .chart_banner()
        .unwrap_or_else(|| panic!("no banner names the edit: {:?}", session.banners()));
    assert!(
        said.contains("house_age") && said.contains("xScale"),
        "the banner does not name the edit it could not place: {said:?}"
    );
    assert!(
        said.contains("no plot"),
        "the banner does not say why the edit could not be placed: {said:?}"
    );
}

// ---------------------------------------------------------------------------
// AC7 — nothing is written without a Save
// ---------------------------------------------------------------------------

/// **AC7.** Opening a data file, and even editing its chart, writes nothing
/// into the data file's folder until a Save: closing the window leaves the
/// folder with the data file alone.
#[test]
fn opening_a_data_file_and_closing_it_without_a_save_writes_nothing_beside_it() {
    let mut session = Session::open("ac7");
    assert_eq!(session.files(), vec![HOUSING_FILE.to_string()]);

    session.transpose();
    session.throw("population", ScaleType::Log);
    session.settle();

    assert_eq!(
        session.files(),
        vec![HOUSING_FILE.to_string()],
        "the window wrote into the data file's folder without a Save"
    );
}

// ---------------------------------------------------------------------------
// AC8 — the document names the saved file
// ---------------------------------------------------------------------------

/// The absolute spelling of `path`, without resolving links.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// **AC8.** After a Save the chart document names the saved chart file as its
/// spec, and the editor pane, opened on the ledger rail, shows that file: the
/// line the edit wrote is in the buffer it draws, which the scratch spec the
/// window was opened from does not hold.
#[test]
fn after_a_save_the_editor_pane_shows_the_saved_chart_file() {
    let mut session = Session::open("ac8");
    session.transpose();
    let scratch = session
        .app
        .chart_doc()
        .spec_path
        .clone()
        .expect("the document names a spec at open");
    session.throw("population", ScaleType::Log);
    session.save();

    let named = session
        .app
        .chart_doc()
        .spec_path
        .clone()
        .expect("the document names a spec after Save");
    assert_ne!(named, scratch, "the document still names the scratch spec");
    assert_eq!(absolute(&named), absolute(&session.chart_file()));

    // Reach the editor the way a person does: its tab on the ledger rail.
    let tab = session
        .app
        .rail_name_rect(LEDGER_RAIL, 3)
        .expect("the ledger rail drew its editor tab")
        .center();
    session.click(tab);
    assert!(session.app.focus_pane(PaneKey::new(EDITOR)));
    session.settle();
    let rail = session
        .app
        .region_rect(LEDGER_RAIL)
        .expect("the ledger rail drew open");

    let drawn = session.drawn_text_in(rail).join("\n");
    assert!(
        drawn.contains("xScale: log"),
        "the editor pane does not draw the line the Save wrote: {drawn:?}"
    );
}
