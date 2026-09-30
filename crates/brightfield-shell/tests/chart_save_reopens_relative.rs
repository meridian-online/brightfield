//! **A Protocol saved from a data file opened by a relative path reopens and
//! draws the saved chart over that data file.**
//!
//! Save spells the chart's `file:` as the caller spelled the data file, so a
//! file opened as `data/housing.csv` leaves `file: 'data/housing.csv'` in a
//! chart that sits one folder below it, in `panels/`. That spelling is relative
//! to the working directory of the launch that saved it. A reopen has to
//! resolve it to the data file the Protocol names, from the same working
//! directory and from another one — a front-door row reopens a Protocol by its
//! absolute path in a later launch, wherever that launch was started.
//!
//! Each test that reopens from another directory leaves a different file of the
//! same name at the relative path the chart spells, in the directory the
//! reopen is made from: a `file:` resolved against the working directory rather
//! than against the Protocol opens that file, or refuses, and the assertions
//! on the resolved data file and the tile's columns say which.
//!
//! # The working directory is process-wide
//!
//! `std::env::set_current_dir` changes it for every thread in this binary, so
//! every test takes [`CWD`] before it moves and [`Cwd`] puts the directory back
//! on drop. They live in a binary of their own for the reason
//! `tests/saved_protocol_working_directory.rs` gives: a test elsewhere that
//! reads a relative path would race them.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use brightfield_protocol::layout::Flow;
use brightfield_shell::app::GridLayout;
use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_spec::layout::ScaleType;

const HOUSING_FILE: &str = "california_housing_sample.csv";

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
            "bf-chart-reopen-relative-{name}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        // Resolved, so a comparison against a path the window reports is not
        // between two spellings of a temp directory that is itself a link.
        Self(std::fs::canonicalize(&dir).expect("the temp directory resolves"))
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

/// A different file under the same name, left where the relative path the chart
/// spells would land if it were resolved against the working directory.
const DECOY_CSV: &str = "decoy_left,decoy_right\n\
                         1,2\n\
                         3,4\n\
                         5,6\n\
                         7,8\n";

/// The committed table the fixture copies.
fn housing() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv")
}

/// The file `path` names, with every link and `..` resolved — from the working
/// directory the caller is standing in when it asks.
fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|e| panic!("resolve {}: {e}", path.display()))
}

/// A window under test: one `egui::Context` for its whole life.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    fn new(boot: Boot) -> Self {
        let mut win = Self {
            app: MeridianApp::headless(boot, Mode::Light),
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

    fn switch_of(&self, column: &str) -> &brightfield_shell::app::ScaleSwitchDrawn {
        self.app
            .chart_doc()
            .scale_switches
            .iter()
            .find(|s| s.column == column)
            .unwrap_or_else(|| panic!("no scale switch for {column:?}"))
    }

    /// The state `column`'s tile is drawn on, read off the composed plot.
    fn active(&self, column: &str) -> ScaleType {
        self.switch_of(column).active
    }

    /// Throw the scale switch on `column`'s tile to `kind`, by the gesture.
    fn throw(&mut self, column: &str, kind: ScaleType) {
        let at = self
            .switch_of(column)
            .states
            .iter()
            .find(|(state, _)| *state == kind)
            .unwrap_or_else(|| panic!("{column}'s switch offers no {kind:?}"))
            .1
            .center();
        self.click(at);
        assert!(
            self.app.title().contains(UNSAVED_MARK),
            "the click at {at:?} did not throw {column}'s switch"
        );
    }

    /// Save, through the chart palette on `space`.
    fn save(&mut self) {
        assert!(self.app.has_protocol_to_save());
        self.key(egui::Key::Space);
        assert_eq!(self.app.open_overlay(), Some("palette"));
        self.settle();
        self.run(vec![egui::Event::Text("save-spec".to_owned())]);
        self.run(Vec::new());
        self.key(egui::Key::Enter);
        assert_eq!(self.app.open_overlay(), None);
        self.settle();
    }

    /// The data file the chart on screen reads, resolved against the base the
    /// reopened dashboard carries — the file the picture was drawn over.
    fn charted_data_file(&self) -> PathBuf {
        let live = self
            .app
            .chart_doc()
            .live_dashboard()
            .expect("the window holds a live dashboard");
        live.data_files(live.base_dir())
            .into_iter()
            .next()
            .expect("the chart reads a data file")
    }

    /// The column names the navigator rail lists.
    fn columns(&self) -> Vec<String> {
        self.app
            .protocol_model()
            .columns()
            .iter()
            .map(|c| c.column.clone())
            .collect()
    }
}

/// The data file `folder` holds, copied from the housing fixture.
fn housing_in(folder: &Path) -> PathBuf {
    let data = folder.join(HOUSING_FILE);
    std::fs::copy(housing(), &data).expect("the housing fixture copies");
    data
}

/// Open `data` as the caller spelled it, throw `population` to log and Save.
/// Returns the chart file's text, read before the window closes.
fn open_and_save(data: &str) -> String {
    let boot = Boot::data_file(data).unwrap_or_else(|e| panic!("open {data}: {e}"));
    let mut window = Window::new(boot);
    window.transpose();
    window.throw("population", ScaleType::Log);
    window.save();
    // The chart sits under `panels/` beside the Protocol, which is beside the
    // data file as the caller spelled it.
    let chart = Path::new(data)
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
        .join("panels")
        .join("california_housing_sample.yaml");
    std::fs::read_to_string(&chart).unwrap_or_else(|e| panic!("read {}: {e}", chart.display()))
}

/// Reopen the Protocol at `manifest` and check it draws the saved chart over
/// `expected`, the data file the Protocol names.
fn assert_reopens_over(manifest: &str, expected: &Path) {
    let boot = Boot::open_sampled(manifest, Flow::Vertical, None, None)
        .unwrap_or_else(|e| panic!("reopen {manifest}: {e}"));
    let mut window = Window::new(boot);
    window.transpose();

    assert_eq!(
        window.active("population"),
        ScaleType::Log,
        "the reopened window did not draw the saved scale"
    );
    let charted = window.charted_data_file();
    assert_eq!(
        resolved(&charted),
        resolved(expected),
        "the chart was drawn over {} and not the data file the Protocol names",
        charted.display()
    );
    let header = std::fs::read_to_string(housing()).expect("the fixture reads");
    let header: Vec<String> = header
        .lines()
        .next()
        .expect("the fixture has a header")
        .split(',')
        .map(str::to_string)
        .collect();
    assert_eq!(
        window.columns(),
        header,
        "the rails list another file's columns"
    );
}

// ---------------------------------------------------------------------------
// AC6
// ---------------------------------------------------------------------------

/// **AC6, the same working directory.** Opened as `data/<file>` from `<root>`,
/// Saved, and the Protocol opened again as `data/arcform.yaml` from `<root>`:
/// the chart's relative `file:` is spelled against `<root>`, and the reopen
/// draws over the file it names.
///
/// The layout on disk:
///
/// ```text
/// <root>/data/arcform.yaml
/// <root>/data/california_housing_sample.csv
/// <root>/data/panels/california_housing_sample.yaml   file: 'data/california_housing_sample.csv'
/// ```
#[test]
fn a_protocol_saved_from_a_relative_path_reopens_from_the_same_directory() {
    let cwd = Cwd::hold();
    let root = TempDir::new("same");
    let folder = root.dir("data");
    let data = housing_in(&folder);
    cwd.enter(&root.0);

    let chart = open_and_save(&format!("data/{HOUSING_FILE}"));
    assert!(
        chart.contains(&format!("file: 'data/{HOUSING_FILE}'")),
        "the fixture is not the relative case: the chart spells its file as\n{chart}"
    );

    assert_reopens_over("data/arcform.yaml", &data);
}

/// **AC6, another working directory.** The same Protocol opened by its absolute
/// path — the spelling a front-door row is remembered under — from a directory
/// with a different file at the relative path the chart spells. The chart's
/// `data/<file>` is resolved against where it was written from, which the two
/// spellings of the data file give, and not against this working directory.
///
/// ```text
/// <root>/data/...                                     the Protocol, saved from <root>
/// <root>/elsewhere/data/california_housing_sample.csv a decoy, the reopen's cwd
/// ```
#[test]
fn a_protocol_saved_from_a_relative_path_reopens_from_another_directory() {
    let cwd = Cwd::hold();
    let root = TempDir::new("elsewhere");
    let folder = root.dir("data");
    let data = housing_in(&folder);
    let decoys = root.dir("elsewhere/data");
    std::fs::write(decoys.join(HOUSING_FILE), DECOY_CSV).expect("the decoy");

    cwd.enter(&root.0);
    let chart = open_and_save(&format!("data/{HOUSING_FILE}"));
    assert!(chart.contains(&format!("file: 'data/{HOUSING_FILE}'")));

    cwd.enter(&root.0.join("elsewhere"));
    let manifest = folder.join("arcform.yaml");
    assert_reopens_over(manifest.to_str().expect("utf-8 path"), &data);
}

/// **AC6, opened from inside the data file's folder.** The chart spells the
/// bare file name, and the reopen from another directory — with a decoy of that
/// name in it — still draws over the file beside the Protocol.
#[test]
fn a_protocol_saved_from_beside_its_data_file_reopens_from_another_directory() {
    let cwd = Cwd::hold();
    let root = TempDir::new("beside");
    let folder = root.dir("data");
    let data = housing_in(&folder);
    let elsewhere = root.dir("elsewhere");
    std::fs::write(elsewhere.join(HOUSING_FILE), DECOY_CSV).expect("the decoy");

    cwd.enter(&folder);
    let chart = open_and_save(HOUSING_FILE);
    assert!(
        chart.contains(&format!("file: '{HOUSING_FILE}'")),
        "the fixture is not the bare-name case: the chart spells its file as\n{chart}"
    );

    cwd.enter(&elsewhere);
    let manifest = folder.join("arcform.yaml");
    assert_reopens_over(manifest.to_str().expect("utf-8 path"), &data);
}

/// **AC6, a `..` in the chart's `file:`.** Saved from `<root>/launch` over
/// `../data/<file>`, which leaves `file: '../data/<file>'` in the chart, and
/// reopened after `launch` has been removed, from a directory whose own
/// `../data/<file>` is a decoy. The `..` has to be resolved through directories
/// that exist, and to land on the file beside the Protocol.
///
/// ```text
/// <root>/launch/                                      cwd 1, removed after the Save
/// <root>/data/                                        the Protocol and its file
/// <root>/later/on/                                    cwd 2
/// <root>/later/data/california_housing_sample.csv     a decoy: cwd 2's ../data/…
/// ```
#[test]
fn a_protocol_saved_over_a_dotdot_path_reopens_after_that_directory_is_gone() {
    let cwd = Cwd::hold();
    let root = TempDir::new("dotdot");
    let launch = root.dir("launch");
    let folder = root.dir("data");
    let data = housing_in(&folder);
    let later = root.dir("later/on");
    std::fs::write(root.dir("later/data").join(HOUSING_FILE), DECOY_CSV).expect("the decoy");

    cwd.enter(&launch);
    let chart = open_and_save(&format!("../data/{HOUSING_FILE}"));
    assert!(
        chart.contains(&format!("file: '../data/{HOUSING_FILE}'")),
        "the fixture is not the dotdot case: the chart spells its file as\n{chart}"
    );

    cwd.enter(&later);
    std::fs::remove_dir_all(&launch).expect("the Save-time directory is removed");
    let manifest = folder.join("arcform.yaml");
    assert_reopens_over(manifest.to_str().expect("utf-8 path"), &data);
}
