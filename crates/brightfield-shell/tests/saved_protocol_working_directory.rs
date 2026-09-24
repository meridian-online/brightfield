//! **A saved Protocol that names its data file by a path relative to itself
//! reopens to that file from a working directory that is not its own.**
//!
//! A one-step Protocol spells its data file `./name`, relative to the
//! Protocol's directory — `one_step`'s module docs give the reasons. That
//! spelling names the right file when it is resolved against the Protocol's
//! directory, and two routes resolve it: [`Boot::open_sampled`], which a command line and a
//! front-door row both reach, and the window's Save, which remembers the
//! Protocol for the next launch to list. Each test below changes the working
//! directory between writing the Protocol and reopening it, and reads back the
//! data file the window opened — a directory resolved against the wrong base
//! either refuses or opens a different file, and a different file of the same
//! name is waiting in the working directory to be opened by mistake.
//!
//! # The working directory is process-wide
//!
//! `std::env::set_current_dir` changes it for every thread in this binary, and
//! cargo runs a binary's tests on several threads. So every test here takes
//! [`CWD`] before it moves, and [`Cwd`] puts the directory back on drop —
//! including on the panic of a failing assertion, which would otherwise leave
//! the next test in a directory that has just been deleted. They live in a
//! binary of their own for the same reason: a test elsewhere that reads a
//! relative path would race them.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use brightfield_protocol::layout::Flow;
use brightfield_shell::design::Mode;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::arrangement::LOCATOR_BAND;
use brightfield_workbench::{GridLayout, GridSpot, RunState, SavedLayout};

/// Held for the whole of any test that changes the working directory.
static CWD: Mutex<()> = Mutex::new(());

/// The working directory, moved by this test and restored when it drops.
struct Cwd {
    back: PathBuf,
    _serial: MutexGuard<'static, ()>,
}

impl Cwd {
    fn hold() -> Self {
        let serial = CWD.lock().unwrap_or_else(PoisonError::into_inner);
        Self {
            back: std::env::current_dir().expect("a working directory"),
            _serial: serial,
        }
    }

    fn enter(&self, dir: &Path) {
        std::env::set_current_dir(dir).unwrap_or_else(|e| panic!("cd {}: {e}", dir.display()));
    }
}

impl Drop for Cwd {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.back);
    }
}

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "bf-saved-protocol-{name}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }

    /// `rel` under this directory, created.
    fn dir(&self, rel: &str) -> PathBuf {
        let dir = self.0.join(rel);
        std::fs::create_dir_all(&dir).expect("a directory under the fixture");
        dir
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The file the Protocol names: three columns, one of them a category.
const READINGS_CSV: &str = "station,reading,depth\n\
                            north,12,4.5\n\
                            north,18,6.0\n\
                            south,31,2.5\n\
                            south,44,9.5\n\
                            east,7,1.0\n\
                            east,25,7.5\n\
                            west,52,3.0\n\
                            west,63,8.0\n";

/// A different file under the same name, left in the working directory the
/// Protocol is reopened from — what a relative name resolved against the
/// working directory rather than the Protocol's opens instead of refusing.
const DECOY_CSV: &str = "decoy_left,decoy_right\n\
                         1,2\n\
                         3,4\n\
                         5,6\n\
                         7,8\n";

/// A one-step Protocol in the shape brightfield writes, naming `data` relative
/// to the Protocol's own directory. Written by hand rather than by
/// `OneStepProtocol::save_to`: the open route recognises the shape, not the
/// writer — `one_step::data_file_named_by` says why — so a spec a person wrote
/// has to reopen the same way.
fn one_step_manifest(data: &str) -> String {
    format!(
        "name: readings\n\
         engine: duckdb\n\
         steps:\n  \
           - name: load\n    \
             sql: models/load.sql\n    \
             depends_on:\n      \
               - '{data}'\n    \
             produces:\n      \
               - readings\n"
    )
}

/// The file `path` names, with every link and `..` resolved — from the
/// working directory the caller is standing in when it asks.
fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|e| panic!("resolve {}: {e}", path.display()))
}

/// The data file `app`'s Protocol read, as the window holds it.
fn opened_data_file(app: &MeridianApp) -> PathBuf {
    app.protocol_model()
        .source()
        .expect("the window holds a one-step Protocol")
        .data
        .clone()
}

/// The column names `app`'s navigator rail lists — the opened file's own
/// header, read back through DuckDB's profile of it.
fn columns(app: &MeridianApp) -> Vec<String> {
    app.protocol_model()
        .columns()
        .iter()
        .map(|c| c.column.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// AC1 — the open route
// ---------------------------------------------------------------------------

/// **Opened by a path with a directory in it, from a working directory that is
/// not the Protocol's, a one-step Protocol opens the file beside itself.**
///
/// The layout on disk:
///
/// ```text
/// <root>/protocol/arcform.yaml     depends_on: './readings.csv'
/// <root>/protocol/readings.csv     the file it names
/// <root>/elsewhere/readings.csv    a decoy under the same name
/// ```
///
/// opened as `../protocol/arcform.yaml` from `<root>/elsewhere`. Resolving
/// `./readings.csv` against the working directory rather than the Protocol's
/// parent opens the decoy, whose path and columns are both wrong.
#[test]
fn a_protocol_opened_from_another_directory_reads_the_file_beside_it() {
    let cwd = Cwd::hold();
    let root = TempDir::new("open-route");
    let protocol = root.dir("protocol");
    let elsewhere = root.dir("elsewhere");
    std::fs::write(protocol.join("readings.csv"), READINGS_CSV).expect("the data file");
    std::fs::write(
        protocol.join("arcform.yaml"),
        one_step_manifest("./readings.csv"),
    )
    .expect("the Protocol");
    std::fs::write(elsewhere.join("readings.csv"), DECOY_CSV).expect("the decoy");

    cwd.enter(&elsewhere);
    let spec = "../protocol/arcform.yaml";
    let boot = Boot::open_sampled(spec, Flow::Vertical, None, None)
        .unwrap_or_else(|e| panic!("open {spec} from {}: {e}", elsewhere.display()));
    let app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);

    let opened = opened_data_file(&app);
    assert_eq!(
        opened,
        Path::new("../protocol").join("./readings.csv"),
        "the data file is the Protocol's directory joined with the name it spells"
    );
    assert_eq!(
        resolved(&opened),
        resolved(&protocol.join("readings.csv")),
        "the window opened {} — not the file beside the Protocol",
        opened.display()
    );
    assert_eq!(
        columns(&app),
        vec!["station", "reading", "depth"],
        "the rails list another file's columns"
    );
}

// ---------------------------------------------------------------------------
// AC2 — Save, then reopen
// ---------------------------------------------------------------------------

/// A window under test: one `egui::Context` for its whole life and one screen
/// rect — `tests/one_step_protocol.rs`'s arrangement, so the gestures below
/// land where that file's do.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    fn with_layout(boot: Boot, layout: brightfield_workbench::SavedLayout) -> Self {
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, layout, Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
        };
        win.settle();
        win
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

    fn click(&mut self, at: egui::Pos2) {
        self.run(vec![
            egui::Event::PointerMoved(at),
            button_at(at, true),
            button_at(at, false),
        ]);
        self.settle();
    }

    /// Press the grid pane's layout switch onto its columns state and read
    /// the state back — `tests/data_file.rs`'s `transpose_the_grid`, aimed at
    /// the rect the frame recorded so a miss cannot read as a saved layout of
    /// rows.
    fn transpose_the_grid(&mut self) {
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
        assert_eq!(
            self.app.grid_layout(),
            GridLayout::Columns,
            "the click at {at:?} did not throw the grid's layout switch"
        );
    }

    /// Press `spot` on the grid's own spot switch, wherever the grid drew it,
    /// and read the state back — `tests/grid_spot.rs`'s `click_spot_switch`.
    fn send_the_grid_to(&mut self, spot: GridSpot) {
        let at = self
            .app
            .chart_doc()
            .grid_spot_switch
            .as_ref()
            .expect("the grid's header band drew its spot switch")
            .states
            .iter()
            .find(|(s, _)| *s == spot)
            .expect("the switch offers that spot")
            .1
            .center();
        self.click(at);
        assert_eq!(
            self.app.grid_spot(),
            spot,
            "the click at {at:?} did not throw the grid's spot switch"
        );
    }

    /// **Save, through the gesture a person has**: the chart palette on
    /// `space`, the verb typed, confirmed with enter —
    /// `one_step_protocol.rs`'s `save_through_the_palette`, which says why a
    /// direct call to `save_protocol` proves the method and not the product.
    fn save_through_the_palette(&mut self) {
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

fn button_at(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// The housing fixture `tests/dashboard_baseline.rs` photographs.
fn housing() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv")
}

const HOUSING_FILE: &str = "california_housing_sample.csv";

/// The Protocol's name, which is the file's stem — what the front door's row
/// for it draws.
const HOUSING_PROTOCOL: &str = "california_housing_sample";

/// **Opened by a relative path, saved, and reopened from a third working
/// directory, the housing screen reopens to the file it was saved over.**
///
/// ```text
/// <root>/launch/                       working directory 1: opens ../data/…
/// <root>/data/california_housing_sample.csv
/// <root>/data/arcform.yaml             where Save writes, beside the file
/// <root>/later/on/                     working directory 3: the next launch
/// <root>/later/on/california_housing_sample.csv   a decoy
/// ```
///
/// The next launch is a second window over the layout the first one wrote,
/// opened on nothing — the front door — in the third directory, and the
/// reopen is a click on the door's row for the saved Protocol: the route an
/// analyst who saved, changed directory and relaunched has. The third
/// directory sits at a different depth from the first so that a path
/// remembered relative to the first cannot resolve by coincidence.
#[test]
fn a_protocol_saved_over_a_relative_path_reopens_from_a_third_directory() {
    let cwd = Cwd::hold();
    let root = TempDir::new("save-reopen");
    let launch = root.dir("launch");
    let data = root.dir("data");
    let later = root.dir("later/on");
    std::fs::copy(housing(), data.join(HOUSING_FILE)).expect("the housing fixture copies");
    std::fs::write(later.join(HOUSING_FILE), DECOY_CSV).expect("the decoy");

    // Working directory 1: open by a relative path, and Save.
    cwd.enter(&launch);
    let chosen = format!("../data/{HOUSING_FILE}");
    let boot = Boot::open_sampled(&chosen, Flow::Vertical, None, None)
        .unwrap_or_else(|e| panic!("open {chosen}: {e}"));
    let mut first = Window::with_layout(boot, default_layout());
    let housing_columns = columns(&first.app);
    first.save_through_the_palette();
    let saved = data.join("arcform.yaml");
    assert!(
        saved.is_file(),
        "Save wrote no Protocol beside the data file at {}",
        saved.display()
    );
    let manifest = std::fs::read_to_string(&saved).expect("the saved Protocol reads");
    assert!(
        manifest.contains(&format!("- './{HOUSING_FILE}'")),
        "Save names the data file relative to the Protocol, as `./{HOUSING_FILE}`:\n{manifest}"
    );
    let layout = first.app.layout().clone();
    drop(first);

    // Working directory 3: the next launch, and a click on the door's row.
    cwd.enter(&later);
    let mut next = Window::with_layout(Boot::empty(), layout);
    assert!(
        next.app.front_door_is_live(),
        "a launch on nothing shows the front door"
    );
    let row = next
        .app
        .front_door_rows()
        .iter()
        .find(|r| r.name == HOUSING_PROTOCOL)
        .unwrap_or_else(|| {
            panic!(
                "the front door, launched from {}, has no row for the saved Protocol — the \
                 layout remembers {:?}, which does not resolve from here",
                later.display(),
                next.app
                    .layout()
                    .recents
                    .iter()
                    .map(|r| r.id.clone())
                    .collect::<Vec<_>>()
            )
        })
        .clone();
    next.click(row.rect.center());
    assert!(
        !next.app.front_door_is_live(),
        "the click on {:?} left the door up — nothing was opened",
        row.id
    );

    let opened = opened_data_file(&next.app);
    assert_eq!(
        resolved(&opened),
        resolved(&data.join(HOUSING_FILE)),
        "the reopened window read {} — not the file the Protocol was saved over",
        opened.display()
    );
    assert_eq!(
        columns(&next.app),
        housing_columns,
        "the reopened window lists another file's columns"
    );

    // The locator band's file name, off the drawn frame: the crumb that
    // leads `file › load › table › dashboard`. The title band above draws the
    // same name as the window's title, so the read is kept to the band's own
    // rect; and the band paints its right-hand source note before its crumbs,
    // so the crumb is found by what follows it rather than by position.
    let band = next
        .app
        .region_rect(LOCATOR_BAND)
        .expect("the locator band drew");
    let drawn = next.drawn_text_in(band);
    assert!(
        drawn
            .windows(3)
            .any(|w| w[0] == HOUSING_FILE && w[1] == "\u{203a}" && w[2] == "load"),
        "the locator band's crumbs do not lead with the saved Protocol's data file, \
         {HOUSING_FILE}: {drawn:?}"
    );
}

// ---------------------------------------------------------------------------
// The id a saved Protocol is remembered under, with the directory it was
// saved from removed
// ---------------------------------------------------------------------------

/// **The housing screen opened over `../data/…` from `launch`, read the way a
/// person leaves it — the grid transposed and sent to the ledger — and saved.**
/// Returns the layout that Save wrote its row into.
///
/// ```text
/// <root>/launch/                       the working directory the Save is made from
/// <root>/data/california_housing_sample.csv
/// <root>/data/arcform.yaml             where Save writes, beside the file
/// ```
///
/// The grid's state is what a later launch has to find under the row, so it
/// is set to the two values a fresh open does not start in: rows in the
/// canvas is what an open that found nothing would draw, and this is neither.
fn save_the_housing_screen_from(cwd: &Cwd, launch: &Path, data: &Path) -> SavedLayout {
    std::fs::copy(housing(), data.join(HOUSING_FILE)).expect("the housing fixture copies");
    cwd.enter(launch);
    let chosen = format!("../data/{HOUSING_FILE}");
    let boot = Boot::open_sampled(&chosen, Flow::Vertical, None, None)
        .unwrap_or_else(|e| panic!("open {chosen}: {e}"));
    let mut first = Window::with_layout(boot, default_layout());
    first.transpose_the_grid();
    first.send_the_grid_to(GridSpot::Ledger);
    first.save_through_the_palette();
    assert!(
        data.join("arcform.yaml").is_file(),
        "Save wrote no Protocol beside the data file at {}",
        data.display()
    );
    first.app.layout().clone()
}

/// The front door's rows for the housing Protocol.
fn housing_rows(app: &MeridianApp) -> Vec<String> {
    app.front_door_rows()
        .iter()
        .filter(|r| r.name == HOUSING_PROTOCOL)
        .map(|r| r.id.clone())
        .collect()
}

/// **A Protocol saved over `../data/…` from a directory that is then removed
/// is still on the front door, and reopens with the grid where it was left.**
///
/// ```text
/// <root>/launch/       working directory 1: Save is made from here, then it is removed
/// <root>/data/         the Protocol and its data file, which stay
/// <root>/later/on/     working directory 3: the next launch
/// ```
///
/// The id Save remembers is the path it wrote, made absolute against
/// `launch/`. Left with the `..` in it — `<root>/launch/../data/arcform.yaml` —
/// it names a file only while `launch/` exists, so the door filters the row
/// out and the layout saved under it has no row to be read from.
#[test]
fn a_protocol_saved_from_a_directory_that_is_removed_still_reopens_with_its_layout() {
    let cwd = Cwd::hold();
    let root = TempDir::new("save-remove-reopen");
    let launch = root.dir("launch");
    let data = root.dir("data");
    let later = root.dir("later/on");

    let layout = save_the_housing_screen_from(&cwd, &launch, &data);
    cwd.enter(&later);
    std::fs::remove_dir_all(&launch).expect("the Save-time directory is removed");

    let mut next = Window::with_layout(Boot::empty(), layout);
    assert!(
        next.app.front_door_is_live(),
        "a launch on nothing shows the front door"
    );
    let row = next
        .app
        .front_door_rows()
        .iter()
        .find(|r| r.name == HOUSING_PROTOCOL)
        .unwrap_or_else(|| {
            panic!(
                "the front door has no row for the saved Protocol once {} is removed — the \
                 layout remembers {:?}",
                launch.display(),
                next.app
                    .layout()
                    .recents
                    .iter()
                    .map(|r| r.id.clone())
                    .collect::<Vec<_>>()
            )
        })
        .clone();
    next.click(row.rect.center());
    assert!(
        !next.app.front_door_is_live(),
        "the click on {:?} left the door up — nothing was opened",
        row.id
    );
    assert_eq!(
        resolved(&opened_data_file(&next.app)),
        resolved(&data.join(HOUSING_FILE)),
        "the reopened window read another file"
    );
    assert_eq!(
        (next.app.grid_layout(), next.app.grid_spot()),
        (GridLayout::Columns, GridSpot::Ledger),
        "the reopened Protocol lost the grid layout and spot it was saved with"
    );
}

/// **One Protocol reached by two relative spellings from two working
/// directories is one row with one layout.**
///
/// ```text
/// <root>/launch/       Save is made from here, over ../data/…
/// <root>/other/deep/   the second launch names it ../../data/arcform.yaml
/// <root>/later/on/     the launch after that reads the front door
/// ```
///
/// The second launch is the command line: it opens the manifest, and the
/// window it builds has to find the grid layout and spot the first launch
/// saved under the same document. It saves again, so the second spelling
/// reaches the layout's rows, and the door of a third launch is read for what
/// it holds.
#[test]
fn one_protocol_named_two_ways_is_one_row_and_the_command_line_reopen_finds_its_layout() {
    let cwd = Cwd::hold();
    let root = TempDir::new("two-spellings");
    let launch = root.dir("launch");
    let data = root.dir("data");
    let other = root.dir("other/deep");
    let later = root.dir("later/on");

    let layout = save_the_housing_screen_from(&cwd, &launch, &data);

    // Working directory 2: the command line, by another relative spelling.
    cwd.enter(&other);
    let spec = "../../data/arcform.yaml";
    let boot = Boot::open_sampled(spec, Flow::Vertical, None, None)
        .unwrap_or_else(|e| panic!("open {spec} from {}: {e}", other.display()));
    let mut second = Window::with_layout(boot, layout);
    let found = (second.app.grid_layout(), second.app.grid_spot());
    second.save_through_the_palette();
    let layout = second.app.layout().clone();
    drop(second);

    // Working directory 3: the next launch, on nothing.
    cwd.enter(&later);
    let mut door = Window::with_layout(Boot::empty(), layout);
    assert_eq!(
        housing_rows(&door.app),
        vec![resolved(&data.join("arcform.yaml"))
            .to_string_lossy()
            .into_owned()],
        "the front door holds one row for the Protocol, under the one absolute name"
    );
    // Read after the count so that each fails on its own: a lookup that misses
    // still saves a row, and the count is what says whether it saved a second.
    assert_eq!(
        found,
        (GridLayout::Columns, GridSpot::Ledger),
        "opened by {spec} from {}, the window did not find the grid layout and spot the \
         Protocol was saved with — the layout keys its row by the path Save wrote",
        other.display()
    );
    let row = door
        .app
        .front_door_rows()
        .iter()
        .find(|r| r.name == HOUSING_PROTOCOL)
        .expect("the row that was just counted")
        .clone();
    door.click(row.rect.center());
    assert_eq!(
        (door.app.grid_layout(), door.app.grid_spot()),
        (GridLayout::Columns, GridSpot::Ledger),
        "the reopen from the door lost the grid layout and spot"
    );
}

/// **A document named on the command line by a relative path finds the grid
/// layout and spot remembered under its absolute path, on three of the four
/// routes the command line has to a document: a data file, a one-step
/// Protocol and a chart spec.**
///
/// `Boot::open_sampled` classifies its argument four ways, and each sets the
/// id the window looks its layout up under. The layout here is seeded under
/// the absolute name, as a Save or a door click leaves it, and the file is
/// named `../…` from `launch/`; a route that keeps the spelling looks the
/// layout up under a name nothing wrote and opens on rows in the canvas.
///
/// The fourth route, a Protocol manifest with a run behind it, opens only
/// under an environment variable that is process-wide; no test in this binary
/// sets it, so that route is not driven here.
#[test]
fn the_command_line_finds_the_layout_remembered_under_the_absolute_name() {
    const CHART: &str = "data:\n  t:\n    - { x: 1, y: 1 }\n    - { x: 2, y: 2 }\n\
                         plot:\n  - mark: dot\n    data: { from: t }\n    x: x\n    y: y\n";
    let cwd = Cwd::hold();
    let root = TempDir::new("routes");
    let launch = root.dir("launch");
    let data = root.dir("data");
    std::fs::copy(housing(), data.join(HOUSING_FILE)).expect("the housing fixture copies");
    std::fs::write(data.join("readings.csv"), READINGS_CSV).expect("the data file");
    std::fs::write(
        data.join("one_step.yaml"),
        one_step_manifest("./readings.csv"),
    )
    .expect("the one-step Protocol");
    std::fs::write(data.join("chart.yaml"), CHART).expect("the chart spec");
    // From here the id is what `remembered_id` gives, and the cwd it reads is
    // the real path: a temporary directory can be reached through a link.
    let base = resolved(&root.0);

    for (route, file) in [
        ("a data file", HOUSING_FILE),
        ("a one-step Protocol", "one_step.yaml"),
        ("a chart spec", "chart.yaml"),
    ] {
        let mut layout = default_layout();
        layout.remember(
            &base.join("data").join(file).to_string_lossy(),
            "remembered",
            RunState::NeverRun,
            GridLayout::Columns,
            GridSpot::Ledger,
            1_000,
        );
        cwd.enter(&launch);
        let spec = format!("../data/{file}");
        let boot = Boot::open_sampled(&spec, Flow::Vertical, None, None)
            .unwrap_or_else(|e| panic!("open {spec}: {e}"));
        let app = MeridianApp::headless_with_layout(boot, layout, Mode::Light);
        assert_eq!(
            (app.grid_layout(), app.grid_spot()),
            (GridLayout::Columns, GridSpot::Ledger),
            "{route}, opened by {spec} from {}, did not find the layout remembered under {}",
            launch.display(),
            base.join("data").join(file).display()
        );
    }
}

/// **Three spellings of one manifest's path boot under one id.**
///
/// The one-step Protocol at `<root>/data/x.yaml`, named `../data/x.yaml` from
/// `<root>/launch`, `<root>/launch/../data/x.yaml` from `<root>`, and
/// `./data/x.yaml` from `<root>`; each boots with `<root>/data/x.yaml` as the
/// id it looks its layout up under.
#[test]
fn three_spellings_of_one_manifest_boot_under_one_id() {
    let cwd = Cwd::hold();
    let root = TempDir::new("spellings");
    let launch = root.dir("launch");
    let data = root.dir("data");
    std::fs::write(data.join("readings.csv"), READINGS_CSV).expect("the data file");
    std::fs::write(data.join("x.yaml"), one_step_manifest("./readings.csv")).expect("the Protocol");
    let base = resolved(&root.0);
    let wanted = base
        .join("data")
        .join("x.yaml")
        .to_string_lossy()
        .into_owned();

    let cases: [(&Path, String); 3] = [
        (launch.as_path(), "../data/x.yaml".to_owned()),
        (
            base.as_path(),
            format!("{}/launch/../data/x.yaml", base.display()),
        ),
        (base.as_path(), "./data/x.yaml".to_owned()),
    ];
    for (from, spec) in cases {
        cwd.enter(from);
        let boot = Boot::open_sampled(&spec, Flow::Vertical, None, None)
            .unwrap_or_else(|e| panic!("open {spec} from {}: {e}", from.display()));
        assert_eq!(
            boot.opened_id.as_deref(),
            Some(wanted.as_str()),
            "{spec}, opened from {}",
            from.display()
        );
    }
}
