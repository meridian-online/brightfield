//! **Reopening a saved Protocol draws the chart file Save wrote beside it.**
//!
//! Opening a saved Protocol read the manifest to find the data file, threw it
//! away and generated the dashboard again, so the chart in `panels/` was on
//! disk and unread: a scale switch saved yesterday was linear again today. The
//! Protocol still opens as its data file — that is what gives the window its
//! rails and the Protocol Save writes again — and the chart file, when it is
//! there, now replaces the generated dashboard.
//!
//! Each test drives the whole loop a person has: switches thrown by click,
//! Save through the chart palette (`tests/chart_save_writes_chart.rs` says why
//! a direct call proves less), the window closed by dropping it, and the
//! Protocol opened again through [`Boot::open`], which is the route a command
//! line and a front-door row both reach. What is read back is what the new
//! window draws — the switch's `active` state is taken off the plot's own
//! composed scale, not off the spec — and the bytes on disk.
//!
//! The relative-path half of the loop moves the working directory, which is
//! process-wide, so it lives in `tests/chart_save_reopens_relative.rs`.

use std::path::{Path, PathBuf};

use brightfield_protocol::layout::Flow;
use brightfield_shell::app::GridLayout;
use brightfield_shell::design::Mode;
use brightfield_shell::editor::EDITOR;
use brightfield_shell::watch::WatchRole;
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_spec::layout::ScaleType;
use brightfield_workbench::arrangement::LEDGER_RAIL;
use brightfield_workbench::PaneKey;

const HOUSING_FILE: &str = "california_housing_sample.csv";

/// The chart file's name: the Protocol's name, which is the data file's stem.
const CHART_FILE: &str = "california_housing_sample.yaml";

/// A comment the analyst types into the chart file, above its `meta:` block.
/// The generator never writes it, so it is in the buffer the editor draws only
/// if the buffer is the chart file's.
const NOTE: &str = "# the income tile is the one I trust";

/// The committed table the first window is opened over.
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
            "bf-chart-reopen-{name}-{}-{}",
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

/// A window over the housing file copied into a folder of its own, so Save has
/// somewhere to write that is not the repository, and so the same folder can be
/// opened again as a Protocol.
///
/// One `egui::Context` for the window's whole life, because a click is resolved
/// against the widget id a previous frame registered. [`Session::reopen`] drops
/// the window and its context together and starts a new pair.
struct Session {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    /// The data file's folder: where `arcform.yaml` and `panels/` land.
    folder: PathBuf,
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
        Self::around(boot, folder, root)
    }

    fn around(boot: Boot, folder: PathBuf, root: TempDir) -> Self {
        let mut session = Self {
            app: MeridianApp::headless(boot, Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
            folder,
            _root: root,
        };
        session.settle();
        session
    }

    /// The saved Protocol's manifest, as a front-door row spells it.
    fn manifest(&self) -> String {
        self.folder
            .join("arcform.yaml")
            .to_str()
            .expect("utf-8 path")
            .to_string()
    }

    /// Close this window and open the Protocol it saved, into a new one.
    fn reopen(self) -> Self {
        let manifest = self.manifest();
        let Self {
            folder,
            _root: root,
            ..
        } = self;
        let boot = Boot::open(&manifest, Flow::Vertical, None)
            .unwrap_or_else(|e| panic!("reopen {manifest}: {e}"));
        Self::around(boot, folder, root)
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

    fn marked(&self) -> bool {
        self.app.title().contains(UNSAVED_MARK)
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

    /// Put the grid in its columns layout, where each histogram tile is on
    /// screen with its switch.
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

    /// The state `column`'s tile is drawn on, read off the composed plot.
    fn active(&self, column: &str) -> ScaleType {
        self.app
            .chart_doc()
            .scale_switches
            .iter()
            .find(|s| s.column == column)
            .unwrap_or_else(|| panic!("no scale switch for {column:?}"))
            .active
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

    /// **Save, through the gesture a person has**: the chart palette on
    /// `space`, the verb typed, confirmed with enter.
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

/// The absolute spelling of `path`, without resolving links.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// A window over the housing file, `columns`' tiles thrown to log and Saved,
/// and left open — the state a Protocol is in the moment before its window is
/// closed.
fn saved_with_log(name: &str, columns: &[&str]) -> Session {
    let mut session = Session::open(name);
    session.transpose();
    for column in columns {
        session.throw(column, ScaleType::Log);
    }
    session.save();
    assert!(
        session.chart_file().is_file(),
        "Save wrote no chart file: {:?}",
        session.files()
    );
    session
}

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

/// The one line that separates `after` from `before`, asserting that it is the
/// only thing that does: a line `after` has inserted, or a line it has
/// replaced, with every other line the same.
fn the_one_changed_line(before: &str, after: &str) -> String {
    let b: Vec<&str> = before.lines().collect();
    let a: Vec<&str> = after.lines().collect();
    let first = b.iter().zip(&a).take_while(|(x, y)| x == y).count();
    if a.len() == b.len() + 1 {
        assert_eq!(
            b[first..],
            a[first + 1..],
            "more than one line changed: a line was inserted at {first} and the rest differs"
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

// ---------------------------------------------------------------------------
// AC1 — the saved scale comes back
// ---------------------------------------------------------------------------

/// **AC1.** A tile thrown to log and Saved is drawn on a log x scale when the
/// Protocol is opened again, and the tile beside it is still on the scale the
/// generator drew. Opening it is not an edit, so the new window is not marked.
#[test]
fn a_reopened_protocol_draws_the_tile_on_the_scale_its_save_wrote() {
    let saved = saved_with_log("ac1", &["population"]);
    assert!(
        saved.chart_text().contains("xScale: log"),
        "Save did not write the scale the test then looks for"
    );

    let mut reopened = saved.reopen();
    reopened.transpose();

    assert_eq!(
        reopened.active("population"),
        ScaleType::Log,
        "the reopened window drew the tile linear: the chart file was not read"
    );
    assert_eq!(
        reopened.active("house_age"),
        ScaleType::Linear,
        "a tile the analyst did not switch is not on the generator's scale"
    );
    assert!(!reopened.marked(), "opening a Protocol marked it unsaved");
}

/// **AC1, the front door.** A window that is already open takes a saved
/// Protocol in through `open_protocol_path`, the route a front-door row for it
/// takes, and draws the saved chart there too.
#[test]
fn a_front_door_row_for_a_saved_protocol_draws_the_saved_chart() {
    let saved = saved_with_log("ac1-door", &["population"]);
    let manifest = saved.manifest();
    let Session {
        folder,
        _root: root,
        ..
    } = saved;

    let start = Boot::data_file(housing().to_str().expect("utf-8 path")).expect("open housing");
    let mut window = Session::around(start, folder, root);
    let ctx = window.ctx.clone();
    window.app.open_protocol_path(&ctx, &manifest);
    window.settle();
    window.transpose();

    assert_eq!(
        window.active("population"),
        ScaleType::Log,
        "the front-door route drew the generated dashboard over the saved chart"
    );
    assert_eq!(
        absolute(
            &window
                .app
                .chart_doc()
                .spec_path
                .clone()
                .expect("the document names a spec")
        ),
        absolute(&window.chart_file())
    );
}

// ---------------------------------------------------------------------------
// AC2 — the document names the chart file and the editor shows its text
// ---------------------------------------------------------------------------

/// **AC2.** The reopened window's chart document names the chart file as its
/// spec, and the editor pane draws that file's text — including a comment the
/// analyst typed into it, which the generator never wrote.
#[test]
fn the_reopened_document_names_the_chart_file_and_the_editor_shows_its_text() {
    let saved = saved_with_log("ac2", &["population"]);
    let typed = type_a_note_into(&saved);
    assert!(typed.contains(NOTE));

    let mut reopened = saved.reopen();
    reopened.transpose();

    let named = reopened
        .app
        .chart_doc()
        .spec_path
        .clone()
        .expect("the reopened document names a spec");
    assert_eq!(
        absolute(&named),
        absolute(&reopened.chart_file()),
        "the document names {} as its spec, not the chart file",
        named.display()
    );

    // Reach the editor the way a person does: its tab on the ledger rail.
    let tab = reopened
        .app
        .rail_name_rect(LEDGER_RAIL, 3)
        .expect("the ledger rail drew its editor tab")
        .center();
    reopened.click(tab);
    assert!(reopened.app.focus_pane(PaneKey::new(EDITOR)));
    reopened.settle();
    let rail = reopened
        .app
        .region_rect(LEDGER_RAIL)
        .expect("the ledger rail drew open");

    let drawn = reopened.drawn_text_in(rail).join("\n");
    assert!(
        drawn.contains(NOTE),
        "the editor pane does not draw the comment typed into the chart file: {drawn:?}"
    );
    assert!(
        drawn.contains("xScale: log"),
        "the editor pane does not draw the line Save wrote: {drawn:?}"
    );
}

// ---------------------------------------------------------------------------
// AC3 — a switch thrown after the reopen is one line of the same file
// ---------------------------------------------------------------------------

/// **AC3.** A switch thrown on the reopened window and Saved changes one line
/// of the chart file the window was opened from: the comment typed into it
/// stays, the line saved before stays, the folder gains no file, and the
/// window's unsaved mark clears.
#[test]
fn a_switch_thrown_after_the_reopen_changes_one_line_of_the_same_file() {
    let saved = saved_with_log("ac3", &["population"]);
    type_a_note_into(&saved);

    let mut reopened = saved.reopen();
    reopened.transpose();
    assert_eq!(
        reopened.active("population"),
        ScaleType::Log,
        "the reopened window is not drawing the saved chart, so a Save from it is not the one asked about"
    );
    let before = reopened.chart_text();
    let files = reopened.files();

    reopened.throw("house_age", ScaleType::Log);
    reopened.save();

    let after = reopened.chart_text();
    assert_eq!(
        the_one_changed_line(&before, &after).trim(),
        "xScale: log",
        "Save changed more than the one line the switch owns"
    );
    assert!(after.contains(NOTE), "Save lost the analyst's comment");
    assert_eq!(after.matches("xScale: log").count(), 2);
    assert_eq!(
        reopened.files(),
        files,
        "Save wrote a file the folder did not have"
    );
    assert!(!reopened.marked(), "the Save left the window marked");
}

/// **AC3, the watch.** The reopened window watches the chart file, so an edit
/// made to it from outside is noticed as a change to the spec — the watch does
/// not sit on the scratch file the generator wrote.
#[test]
fn an_edit_made_to_the_reopened_chart_file_from_outside_is_noticed() {
    let saved = saved_with_log("watch", &["population"]);
    let mut reopened = saved.reopen();
    assert!(
        reopened.app.chart_doc().watch.changes().is_empty(),
        "the watch reported a change before anything outside touched a file"
    );

    type_a_note_into(&reopened);
    reopened.app.chart_doc_mut().watch.poll_now();

    let chart = absolute(&reopened.chart_file());
    let noticed = reopened
        .app
        .chart_doc()
        .watch
        .changes()
        .iter()
        .any(|c| c.role == WatchRole::Spec && absolute(&c.path) == chart);
    assert!(
        noticed,
        "an outside edit to {} raised no spec change: {:?}",
        chart.display(),
        reopened.app.chart_doc().watch.changes()
    );
}

// ---------------------------------------------------------------------------
// AC4 — a Protocol with no chart file opens as it did
// ---------------------------------------------------------------------------

/// **AC4.** A saved Protocol with no chart file beside it — one saved before
/// Save wrote charts, or whose chart was deleted — opens with a generated
/// dashboard: every tile on the generator's scale, the document naming a
/// generated spec that holds no scale key, and nothing written into the folder
/// by opening it.
#[test]
fn a_saved_protocol_with_no_chart_file_opens_with_a_generated_dashboard() {
    let saved = saved_with_log("ac4", &["population"]);
    std::fs::remove_dir_all(saved.folder.join("panels")).expect("remove the chart");
    let files = saved.files();
    assert!(!files.iter().any(|f| f.starts_with("panels")));

    let mut reopened = saved.reopen();
    reopened.transpose();

    assert_eq!(reopened.active("population"), ScaleType::Linear);
    assert_eq!(reopened.active("house_age"), ScaleType::Linear);
    let spec = reopened
        .app
        .chart_doc()
        .spec_path
        .clone()
        .expect("a generated dashboard names the spec it was composed from");
    assert!(
        !absolute(&spec).starts_with(absolute(&reopened.folder)),
        "the document names {} inside the Protocol's folder, where no chart is",
        spec.display()
    );
    let text = std::fs::read_to_string(&spec).expect("the generated spec is on disk");
    assert!(
        !text.contains("xScale"),
        "the generated spec carries a scale the generator does not write"
    );
    assert_eq!(reopened.files(), files, "opening wrote into the folder");
}

// ---------------------------------------------------------------------------
// AC5 — a chart that will not parse is refused and left as it is
// ---------------------------------------------------------------------------

/// **AC5.** A chart file that does not parse is refused with the message the
/// same file gives when it is opened by name, its bytes are as they were, the
/// folder gains nothing, and no dashboard is generated in its place — opening
/// returns the refusal and no window.
#[test]
fn a_chart_file_that_does_not_parse_is_reported_and_left_as_it_is() {
    let saved = saved_with_log("ac5", &["population"]);
    let broken = "data: [unclosed\n";
    std::fs::write(saved.chart_file(), broken).expect("break the chart file");
    let files = saved.files();

    let by_protocol = Boot::open(&saved.manifest(), Flow::Vertical, None)
        .err()
        .expect("a Protocol whose chart will not parse opened anyway");
    let by_name = Boot::open(
        saved.chart_file().to_str().expect("utf-8 path"),
        Flow::Vertical,
        None,
    )
    .err()
    .expect("a chart file that will not parse opened by name anyway");

    assert!(
        by_name.starts_with("parse error"),
        "the by-name refusal is not the parse error this test compares against: {by_name:?}"
    );
    assert_eq!(
        by_protocol, by_name,
        "the Protocol's refusal is not the one the chart file gives by name"
    );
    assert_eq!(
        std::fs::read_to_string(saved.chart_file()).expect("the chart file is still there"),
        broken,
        "opening changed the chart file's bytes"
    );
    assert_eq!(saved.files(), files, "opening wrote into the folder");
}
