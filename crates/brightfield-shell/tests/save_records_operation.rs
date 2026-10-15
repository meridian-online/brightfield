//! Save records each applied grid view through arc as a generated step, named
//! by its effect: the condition first, then the order on the condition's table.
//! Save and Run leave alone the steps they did not write.
//!
//! The window is given the `brightfield-shell` binary cargo built for this
//! suite as its runner, so every recording is arc's own `operation record`, and
//! what the assertions read back is the folder arc wrote.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use brightfield_engine::RowsView;
use brightfield_shell::design::Mode;
use brightfield_shell::grid_foot::FootField;
use brightfield_shell::overlays::CloseAnswer;
use brightfield_shell::run::{self, Runner};
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp, GRID_FOOT_STATUS_ID};
use brightfield_workbench::arrangement::LEDGER_RAIL;

/// The table the housing file's load makes.
const ROOT: &str = "california_housing";
const FILTER_STEP: &str = "california_housing_filtered_by_house_age";
const SORT_STEP: &str = "california_housing_sorted_by_median_income";

/// How long a run may take before the test calls it hung.
const PATIENCE: Duration = Duration::from_secs(180);

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
            "bf-save-records-{name}-{}-{}",
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

/// Puts directories' permissions back when dropped, so a test that made them
/// read-only leaves a folder its `TempDir` can remove even when it fails.
#[cfg(unix)]
struct Restore(Vec<PathBuf>);

#[cfg(unix)]
impl Drop for Restore {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        for dir in &self.0 {
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
        }
    }
}

/// Every file under `dir`, path relative to it, with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
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

/// A window over the housing start's bytes in a folder of its own.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    folder: PathBuf,
    sent_close: bool,
    _root: TempDir,
}

impl Window {
    fn open(name: &str) -> Self {
        Self::open_with(name, runner())
    }

    fn open_with(name: &str, runner: Runner) -> Self {
        let root = TempDir::new(name);
        let folder = root.0.join("data");
        std::fs::create_dir_all(&folder).expect("the data file's folder");
        let data = folder.join(starts::HOUSING_FILE);
        std::fs::write(&data, starts::HOUSING_PARQUET).expect("the housing file writes");
        let boot = Boot::data_file(&data.to_string_lossy()).expect("the housing file opens");
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light)
                .running_with(Some(runner))
                .keeping_history(Some(brightfield_protocol::HistoryStore::At(
                    root.0.join("history"),
                ))),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
            folder,
            sent_close: false,
            _root: root,
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
        let out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        if let Some(root) = out.viewport_output.get(&egui::ViewportId::ROOT) {
            if root
                .commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::Close))
            {
                self.sent_close = true;
            }
        }
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
        self.settle();
    }

    fn apply(&mut self, condition: Option<&str>, order: Option<&str>) {
        self.app.chart_doc_mut().apply_grid_view(RowsView {
            condition: condition.map(str::to_owned),
            order: order.map(str::to_owned),
        });
        self.settle();
    }

    fn save(&mut self) -> Result<PathBuf, String> {
        let saved = self
            .app
            .save_protocol(&self.ctx)
            .expect("the window has a Protocol to save");
        self.settle();
        saved
    }

    fn manifest(&self) -> String {
        std::fs::read_to_string(self.folder.join("arcform.yaml")).expect("the manifest reads")
    }

    /// The names of the manifest's steps, in order.
    fn steps(&self) -> Vec<String> {
        brightfield_protocol::parse_manifest_str(&self.manifest())
            .expect("the manifest parses")
            .steps
            .into_iter()
            .map(|s| s.name)
            .collect()
    }

    /// The model of the step `name`.
    fn model(&self, name: &str) -> String {
        let manifest = brightfield_protocol::parse_manifest_str(&self.manifest())
            .expect("the manifest parses");
        let step = manifest
            .steps
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no step {name} in {:?}", self.steps()));
        std::fs::read_to_string(self.folder.join(step.sql.as_ref().expect("a sql step")))
            .expect("the model reads")
    }

    /// The headline and body of every banner the window is showing.
    fn banners(&self) -> Vec<(String, String)> {
        self.app
            .notifications()
            .iter()
            .map(|n| (n.title.clone(), n.body.clone().unwrap_or_default()))
            .collect()
    }

    /// The status band's line for the band, as the frame drew it.
    fn foot_status(&self) -> Option<String> {
        if !self.app.rail().drawn.contains(&GRID_FOOT_STATUS_ID) {
            return None;
        }
        self.app.chart_doc().foot_said()
    }

    /// The data sources the chart is drawn from, each with what it reads.
    fn chart_sources(&self) -> Vec<(String, String)> {
        self.app
            .chart_doc()
            .live_spec()
            .expect("a live chart")
            .data
            .iter()
            .map(|(name, source)| (name.clone(), format!("{:?}", source.kind)))
            .collect()
    }
}

/// A runner that, while the file `refuse` stands in `dir`, refuses every
/// `sort-rows` recording in arc's way (`error: ` and a line on stderr), and
/// hands everything else to the real one.
#[cfg(unix)]
fn refusing_sorts(dir: &Path) -> Runner {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("refusing-sorts.sh");
    let flag = dir.join("refuse");
    std::fs::write(&flag, "").expect("the flag writes");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nif [ -e '{}' ]; then\n  case \"$*\" in\n    *sort-rows*) echo 'error: sorts are refused here' >&2; exit 2;;\n  esac\nfi\nexec '{}' \"$@\"\n",
            flag.display(),
            env!("CARGO_BIN_EXE_brightfield-shell")
        ),
    )
    .expect("the script writes");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("the script is executable");
    Runner::at(script)
}

/// Let [`refusing_sorts`]'s runner in `dir` take sorts again.
#[cfg(unix)]
fn take_sorts(dir: &Path) {
    std::fs::remove_file(dir.join("refuse")).expect("the flag is removed");
}

#[test]
fn save_records_the_filter_as_a_named_generated_step() {
    let mut win = Window::open("filter");
    win.apply(Some("house_age > 28"), None);
    assert!(win.app.carries_unsaved_mark());

    win.save().expect("the Save records");

    assert_eq!(
        win.steps(),
        vec!["load".to_string(), FILTER_STEP.to_string()]
    );
    assert!(
        win.model(FILTER_STEP)
            .starts_with("-- generated: filter-rows on california_housing"),
        "{}",
        win.model(FILTER_STEP)
    );
    assert!(!win.app.carries_unsaved_mark());
}

#[test]
fn save_records_filter_then_sort_on_the_filters_table() {
    let mut win = Window::open("filter-sort");
    win.apply(Some("house_age > 28"), Some("median_income DESC"));

    win.save().expect("the Save records");

    assert_eq!(
        win.steps(),
        vec![
            "load".to_string(),
            FILTER_STEP.to_string(),
            SORT_STEP.to_string()
        ]
    );
    assert!(
        win.model(SORT_STEP)
            .starts_with(&format!("-- generated: sort-rows on {FILTER_STEP}")),
        "{}",
        win.model(SORT_STEP)
    );
    assert!(!win.app.carries_unsaved_mark());
}

#[test]
fn a_second_save_records_nothing_and_erases_nothing() {
    let mut win = Window::open("twice");
    win.apply(Some("house_age > 28"), Some("median_income DESC"));
    win.save().expect("the first Save records");
    let first = snapshot(&win.folder);
    assert_eq!(win.steps().len(), 3);

    win.save().expect("the second Save");

    assert_eq!(
        snapshot(&win.folder),
        first,
        "the second Save changed a file"
    );
}

#[test]
fn run_keeps_the_generated_steps() {
    let mut win = Window::open("run");
    win.apply(Some("house_age > 28"), Some("median_income DESC"));
    win.save().expect("the Save records");

    assert!(win.app.run_protocol(&win.ctx), "the run did not start");
    let deadline = Instant::now() + PATIENCE;
    while win.app.run_outstanding() {
        assert!(Instant::now() < deadline, "the run did not land");
        win.run(Vec::new());
        std::thread::sleep(Duration::from_millis(5));
    }
    win.settle();

    let record = run::records_newest_first(&win.folder, ROOT)
        .next()
        .expect("the run wrote a record");
    assert_eq!(
        record.run.outcome,
        brightfield_protocol::contract::Outcome::Success,
        "the run did not succeed"
    );
    assert_eq!(
        win.steps(),
        vec![
            "load".to_string(),
            FILTER_STEP.to_string(),
            SORT_STEP.to_string()
        ]
    );
    for step in [FILTER_STEP, SORT_STEP] {
        assert!(win.model(step).starts_with("-- generated:"), "{step}");
    }
}

#[cfg(unix)]
#[test]
fn a_refusal_is_shown_in_arcs_words_and_keeps_the_mark() {
    use std::os::unix::fs::PermissionsExt;
    let mut win = Window::open("refused");
    win.save().expect("the first Save writes the folder");
    let models = win.folder.join("models");
    let _restore = Restore(vec![win.folder.clone(), models.clone()]);
    for dir in [&win.folder, &models] {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555))
            .expect("the folder is made read-only");
    }
    if std::fs::write(win.folder.join(".probe"), "").is_ok() {
        eprintln!("this process ignores directory permissions; the write cannot be refused");
        return;
    }
    win.apply(Some("house_age > 28"), None);

    let err = win.save().expect_err("arc refuses to record");

    let (_, body) = win
        .banners()
        .into_iter()
        .find(|(_, body)| *body == err)
        .unwrap_or_else(|| panic!("no banner carries arc's words {err:?}: {:?}", win.banners()));
    assert!(!body.is_empty() && !body.starts_with("error:"), "{body}");
    assert_eq!(
        win.app.chart_doc().grid_view().condition.as_deref(),
        Some("house_age > 28")
    );
    assert!(win.app.chart_doc().grid_view_unrecorded());
    assert!(win.app.carries_unsaved_mark());
    assert_eq!(win.steps(), vec!["load".to_string()]);
}

#[test]
#[ignore = "the window adopts the saved Protocol through the open path for a folder of generated steps, which is not built yet"]
fn after_save_the_window_shows_the_protocols_steps() {
    let mut win = Window::open("adopt");
    win.apply(Some("house_age > 28"), None);
    win.save().expect("the Save records");

    assert!(
        !win.app.rail_is_collapsed(LEDGER_RAIL),
        "the ledger rail is shut"
    );
    let model = win.app.protocol_model();
    assert_eq!(model.sheet().len(), 2);
    assert!(
        model.selected().is_some_and(|id| id.contains(FILTER_STEP)),
        "the newest step is not selected: {:?}",
        model.selected()
    );
}

#[test]
fn a_view_leaves_charts_on_the_load() {
    let mut win = Window::open("charts");
    let before = win.chart_sources();
    assert!(
        before.len() == 1 && before[0].1.contains(starts::HOUSING_FILE),
        "the chart does not read the load's file: {before:?}"
    );
    win.apply(Some("house_age > 28"), Some("median_income DESC"));

    win.save().expect("the Save records");

    assert_eq!(win.chart_sources(), before);
    let chart = std::fs::read_to_string(win.folder.join("panels").join(format!("{ROOT}.yaml")))
        .expect("the chart file was written");
    assert!(chart.contains(ROOT), "{chart}");
    assert!(
        !chart.contains(FILTER_STEP) && !chart.contains(SORT_STEP),
        "the chart reads a generated step: {chart}"
    );
}

#[cfg(unix)]
#[test]
fn a_partial_save_keeps_the_grid_and_records_the_rest_on_the_filters_table() {
    let scripts = TempDir::new("partial-runner");
    let mut win = Window::open_with("partial", refusing_sorts(&scripts.0));
    win.apply(Some("house_age > 28"), Some("median_income DESC"));

    let err = win.save().expect_err("the sort is refused");
    assert_eq!(err, "sorts are refused here");
    assert_eq!(
        win.steps(),
        vec!["load".to_string(), FILTER_STEP.to_string()]
    );

    // The grid is what the analyst saw: the condition reads as recorded, the
    // order is still applied and not.
    let doc = win.app.chart_doc();
    assert_eq!(doc.grid_view().condition.as_deref(), Some("house_age > 28"));
    assert_eq!(doc.grid_view().order.as_deref(), Some("median_income DESC"));
    let band = doc.foot_drawn().expect("the band drew");
    assert!(
        !band.where_dot,
        "the recorded condition still carries the dot"
    );
    assert!(band.order_dot, "the unrecorded order lost its dot");
    assert!(win.app.carries_unsaved_mark());

    // The next Save, with a runner that takes sorts, records only the rest,
    // on the filter's table.
    take_sorts(&scripts.0);
    win.save().expect("the rest records");
    assert_eq!(
        win.steps(),
        vec![
            "load".to_string(),
            FILTER_STEP.to_string(),
            SORT_STEP.to_string()
        ]
    );
    assert!(
        win.model(SORT_STEP)
            .starts_with(&format!("-- generated: sort-rows on {FILTER_STEP}")),
        "{}",
        win.model(SORT_STEP)
    );
    assert!(!win.app.carries_unsaved_mark());
}

#[cfg(unix)]
#[test]
fn after_a_partial_save_the_recorded_condition_cannot_be_edited() {
    let scripts = TempDir::new("held-runner");
    let mut win = Window::open_with("held", refusing_sorts(&scripts.0));
    win.apply(Some("house_age > 28"), Some("median_income DESC"));
    win.save().expect_err("the sort is refused");

    // Neither a key nor an offer opens the condition while it is held.
    win.app
        .chart_doc_mut()
        .foot_offer(FootField::Where, "house_age > 5".to_owned());
    win.settle();
    assert_eq!(win.app.chart_doc().foot_asking(), None);
    assert!(!win.app.chart_doc_mut().ask_where_at_cursor());
    let said = win.foot_status().expect("the status band says why");
    assert!(said.contains("recorded"), "{said}");
    assert!(said.contains(FILTER_STEP), "{said}");

    // Emptying the order leaves nothing unrecorded, and the window adopts the
    // Protocol the folder now holds.
    win.app
        .chart_doc_mut()
        .foot_offer(FootField::OrderBy, String::new());
    win.settle();
    win.key(egui::Key::Enter);
    assert!(!win.app.carries_unsaved_mark());
    assert_eq!(win.app.chart_doc().grid_view(), &RowsView::default());
    assert_eq!(win.app.protocol_model().sheet().len(), 2);
}

#[cfg(unix)]
#[test]
fn closing_after_a_refused_recording_keeps_the_window() {
    let scripts = TempDir::new("close-runner");
    let mut win = Window::open_with("close", refusing_sorts(&scripts.0));
    win.apply(None, Some("median_income DESC"));

    win.app
        .answer_close_question(&win.ctx.clone(), CloseAnswer::SaveAndClose);
    win.settle();

    assert!(!win.sent_close, "the window closed over an unrecorded view");
    assert!(win.app.carries_unsaved_mark());

    // The same answer once arc records closes the window, so the first half
    // cannot pass by the close never reaching the frame.
    take_sorts(&scripts.0);
    win.app
        .answer_close_question(&win.ctx.clone(), CloseAnswer::SaveAndClose);
    win.settle();
    assert!(
        win.sent_close,
        "the window stayed open after the view was recorded"
    );
}
