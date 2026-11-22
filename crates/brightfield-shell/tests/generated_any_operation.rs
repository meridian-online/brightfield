//! A folder holding a data-file load and steps arc generated for operations
//! other than a filter or a sort: it opens as its chain, each step reading
//! the tables its header lists, and each step's grid previews by running the
//! model arc wrote.
//!
//! The steps are recorded through arc itself (the `brightfield-shell` binary
//! cargo built for this suite is the runner), so the reader is held to what
//! arc writes.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use brightfield_engine::{preview_first_page, FileReader, RecordBatch};
use brightfield_protocol::layout::Flow;
use brightfield_shell::data_file;
use brightfield_shell::data_grid::{use_own_preview_gate_on_this_thread, PREVIEW_LABEL};
use brightfield_shell::design::Mode;
use brightfield_shell::generated::{
    generated_protocol_in, preview_chain, GeneratedProtocol, StepEffect,
};
use brightfield_shell::operation::{self, StepRequest};
use brightfield_shell::protocol::SpineRole;
use brightfield_shell::run::Runner;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};

/// The table the sample's load makes.
const ROOT: &str = "california_housing_sample";

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-generated-any-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
        .collect()
}

fn record(
    dir: &Path,
    name: &str,
    long_name: &str,
    on: &str,
    other: &[&str],
    args: &[(&str, &str)],
) {
    let request = StepRequest {
        long_name: long_name.to_string(),
        on: Some(on.to_string()),
        reads: other
            .iter()
            .map(|t| ("other".to_string(), (*t).to_string()))
            .collect(),
        args: pairs(args),
    };
    operation::record_step(&runner(), dir, name, &request)
        .unwrap_or_else(|e| panic!("arc records {name}: {e}"));
}

/// A copy of the housing sample saved once, so its load is written, and then
/// five steps recorded through arc:
///
/// - `h` hides `house_age` on the load's table;
/// - `r` renames `median_income` to `income` on `h`;
/// - `s` counts the load's rows by `house_age`;
/// - `j` joins the load's table to `s` on `house_age`;
/// - `a` appends `r`'s rows to `h`'s.
fn folder(name: &str) -> TempDir {
    let dir = TempDir::new(name);
    let data = dir.path().join("california_housing_sample.csv");
    std::fs::write(&data, include_bytes!("data/california_housing_sample.csv"))
        .expect("the sample writes");
    let opened = data_file::open(&data.to_string_lossy()).expect("the sample opens");
    opened
        .protocol
        .save_to(dir.path())
        .expect("the Protocol saves");
    let d = dir.path();
    record(d, "h", "hide-column", ROOT, &[], &[("column", "house_age")]);
    record(
        d,
        "r",
        "rename-column",
        "h",
        &[],
        &[("column", "median_income"), ("to", "income")],
    );
    record(
        d,
        "s",
        "summarise-rows",
        ROOT,
        &[],
        &[("aggregates", "count(*) AS n"), ("group_by", "house_age")],
    );
    record(
        d,
        "j",
        "join-tables",
        ROOT,
        &["s"],
        &[("using", "house_age")],
    );
    record(d, "a", "append-rows", "h", &["r"], &[]);
    dir
}

fn manifest_text(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("arcform.yaml")).expect("the manifest reads")
}

fn read(dir: &Path) -> GeneratedProtocol {
    generated_protocol_in(&manifest_text(dir), dir)
        .expect("the folder's steps read")
        .expect("the folder is a load and generated steps")
}

fn model_path(dir: &Path, name: &str) -> PathBuf {
    let parsed =
        brightfield_protocol::parse_manifest_str(&manifest_text(dir)).expect("the manifest parses");
    let step = parsed
        .steps
        .iter()
        .find(|s| s.name == name)
        .expect("the step is in the manifest");
    dir.join(step.sql.as_ref().expect("a sql step"))
}

fn columns(batches: &[RecordBatch]) -> Vec<String> {
    batches.first().map_or_else(Vec::new, |batch| {
        batch
            .schema()
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect()
    })
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn a_folder_of_any_generated_steps_opens_as_its_chain() {
    let dir = folder("chain");
    let protocol = read(dir.path());
    assert_eq!(protocol.root_table, ROOT);
    let got: Vec<(&str, StepEffect, Vec<String>)> = protocol
        .steps
        .iter()
        .map(|s| (s.name.as_str(), s.effect.clone(), s.reads.clone()))
        .collect();
    let generated = |long: &str| StepEffect::Generated(long.to_string());
    assert_eq!(
        got,
        [
            ("h", generated("hide-column"), strings(&[ROOT])),
            ("r", generated("rename-column"), strings(&["h"])),
            ("s", generated("summarise-rows"), strings(&[ROOT])),
            ("j", generated("join-tables"), strings(&[ROOT, "s"])),
            ("a", generated("append-rows"), strings(&["h", "r"])),
        ]
    );
    assert_eq!(protocol.steps[3].on, ROOT);
    // The argument is the model as arc wrote it.
    let model = std::fs::read_to_string(model_path(dir.path(), "r")).expect("r's model");
    assert_eq!(protocol.steps[1].argument, model);
}

#[test]
fn each_generated_step_previews_through_what_it_reads() {
    let dir = folder("previews");
    let protocol = read(dir.path());
    let page = |step: &str| {
        let chain = preview_chain(&protocol, step).expect("the step is in the folder");
        let names: Vec<String> = chain.iter().map(|(name, _)| name.clone()).collect();
        let page = preview_first_page(&protocol.data, FileReader::Csv, ROOT, &chain, 10)
            .unwrap_or_else(|e| panic!("{step} previews: {e}"));
        (names, columns(&page))
    };
    // `a` reads `h` and `r`, and not `s` or `j`.
    let (names, _) = page("a");
    assert_eq!(names, ["h", "r", "a"]);

    let (names, renamed) = page("r");
    assert_eq!(names, ["h", "r"]);
    assert!(renamed.contains(&"income".to_string()), "{renamed:?}");
    assert!(
        !renamed.contains(&"median_income".to_string()),
        "{renamed:?}"
    );
    assert!(!renamed.contains(&"house_age".to_string()), "{renamed:?}");

    let (names, joined) = page("j");
    assert_eq!(names, ["s", "j"]);
    assert!(joined.contains(&"s_n".to_string()), "{joined:?}");
}

fn collect_text(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, out);
            }
        }
        _ => {}
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

/// A headless window.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    texts: Vec<String>,
}

impl Window {
    fn over(boot: Boot) -> Self {
        use_own_preview_gate_on_this_thread();
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light)
                .running_with(Some(runner())),
            ctx: egui::Context::default(),
            texts: Vec::new(),
        };
        win.settle();
        win
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440.0, 900.0),
            )),
            events,
            ..Default::default()
        };
        let out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        let mut texts = Vec::new();
        for clipped in &out.shapes {
            collect_text(&clipped.shape, &mut texts);
        }
        self.texts = texts;
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    /// Click the `grid` row under the asset row labelled `node`, and wait for
    /// the preview it shows.
    fn click_grid_under(&mut self, node: &str) {
        let rows = self.app.spine_rows().to_vec();
        let at = rows
            .iter()
            .position(|row| row.role == SpineRole::Asset && row.label == node)
            .unwrap_or_else(|| panic!("the rail drew no asset row labelled {node:?}"));
        let grid = rows
            .get(at + 1)
            .filter(|row| row.role == SpineRole::View && row.label == "grid")
            .unwrap_or_else(|| panic!("no grid row stands under {node:?}"));
        let centre = grid.rect.center();
        self.run(vec![egui::Event::PointerMoved(centre)]);
        self.run(vec![button(centre, true), button(centre, false)]);
        self.settle();
        let deadline = Instant::now() + Duration::from_secs(60);
        while self
            .app
            .chart_doc()
            .grid_preview()
            .is_some_and(|p| p.is_reading())
        {
            assert!(Instant::now() < deadline, "the preview never arrived");
            std::thread::sleep(Duration::from_millis(5));
            self.run(Vec::new());
        }
        self.settle();
    }
}

#[test]
fn the_window_lays_the_steps_out_not_run() {
    let dir = folder("window");
    let manifest = dir.path().join("arcform.yaml");
    let boot =
        Boot::open(&manifest.to_string_lossy(), Flow::Vertical, None).expect("the folder opens");
    let mut win = Window::over(boot);

    let assets: Vec<String> = win
        .app
        .spine_rows()
        .iter()
        .filter(|row| row.role == SpineRole::Asset)
        .map(|row| row.label.clone())
        .collect();
    for step in ["h", "r", "s", "j", "a"] {
        assert!(assets.contains(&step.to_string()), "{step} in {assets:?}");
        assert!(
            !win.app.protocol_model().step_ran(step),
            "{step} has not run"
        );
    }
    assert!(!dir.path().join("build").exists(), "opening ran nothing");

    win.click_grid_under("r");
    assert!(
        win.texts.iter().any(|t| t.contains(PREVIEW_LABEL)),
        "the grid is labelled a preview: {:?}",
        win.texts
    );
    let page = win
        .app
        .chart_doc()
        .grid_preview()
        .and_then(|p| p.page())
        .expect("the grid holds a preview page");
    let names: Vec<&str> = page.columns.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"income"), "{names:?}");
    assert!(!names.contains(&"median_income"), "{names:?}");
}

#[test]
fn a_generated_model_edited_away_from_its_step_is_refused_naming_it() {
    let dir = folder("edited");
    let path = model_path(dir.path(), "r");
    let model = std::fs::read_to_string(&path).expect("r's model");
    let edited = model.replace(
        "CREATE OR REPLACE TABLE \"r\" AS",
        "CREATE OR REPLACE TABLE \"other\" AS",
    );
    assert_ne!(edited, model);
    std::fs::write(&path, edited).expect("the edit writes");
    let err = generated_protocol_in(&manifest_text(dir.path()), dir.path())
        .expect_err("r's model no longer makes r");
    assert!(err.contains("step \"r\""), "{err}");
    assert!(err.contains("does not make \"r\""), "{err}");
}
