//! Every open goes through the window's one queue, and opening data makes a Protocol of it in the
//! Protocols folder: a lone CSV becomes a Protocol of one source, read where it is, and a folder of
//! tables becomes a Protocol of several. A Protocol directory, including one an earlier Brightfield
//! saved beside its CSV, opens where it is and makes nothing.
//!
//! Every window here is headless, with the `brightfield-shell` binary cargo built as its runner.
//! The config directory, the Protocols folder and arc's directories point at a scratch folder of
//! this file's before anything reads them, so nothing is written under the home directory.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use brightfield_shell::data_file;
use brightfield_shell::design::Mode;
use brightfield_shell::open_queue::{route, OpenRequest, Route};
use brightfield_shell::run::Runner;
use brightfield_shell::window::{Boot, MeridianApp};

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-open-protocol-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "protocols", "input"] {
            std::fs::create_dir_all(root.join(sub)).expect("a scratch folder");
        }
        // Set once, before any test in this file reads them, and never changed.
        std::env::set_var(
            brightfield_shell::startup::CONFIG_DIR_VAR,
            root.join("config"),
        );
        std::env::set_var(
            brightfield_shell::protocols_dir::PROTOCOLS_DIR_VAR,
            root.join("protocols"),
        );
        std::env::set_var("ARCFORM_HISTORY_DIR", root.join("history"));
        std::env::set_var("ARCFORM_DB_DIR", root.join("db"));
        root
    })
}

fn protocols() -> PathBuf {
    scratch().join("protocols")
}

/// A folder of this test's own under the scratch input folder.
fn input(name: &str) -> PathBuf {
    let dir = scratch().join("input").join(name);
    std::fs::create_dir_all(&dir).expect("an input folder");
    dir
}

/// The committed California Housing sample, which the tests copy under a name of their choosing.
fn housing() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv")
}

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// What a folder holds, by name, with each entry's size and modification time, so a file
/// rewritten in place shows as a change too.
fn listing(dir: &Path) -> BTreeSet<(String, u64, std::time::SystemTime)> {
    std::fs::read_dir(dir)
        .expect("the folder reads")
        .flatten()
        .map(|e| {
            let meta = e.metadata().expect("an entry's metadata");
            (
                e.file_name().to_string_lossy().into_owned(),
                meta.len(),
                meta.modified().expect("a modification time"),
            )
        })
        .collect()
}

/// The Protocols folder's entries whose name starts with `stem`.
fn protocols_named(stem: &str) -> Vec<String> {
    std::fs::read_dir(protocols())
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with(stem))
                .collect()
        })
        .unwrap_or_default()
}

/// The load steps' SQL in the Protocol at `dir`, one string per step.
fn load_steps(dir: &Path) -> Vec<String> {
    let manifest = std::fs::read_to_string(dir.join("arcform.yaml")).expect("a manifest");
    manifest
        .lines()
        .filter_map(|l| l.trim().strip_prefix("sql: "))
        .filter(|file| file.contains("_load_"))
        .map(|file| std::fs::read_to_string(dir.join(file)).expect("the load step's SQL"))
        .collect()
}

/// One headless window, its frames run through one context.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        scratch();
        let mut win = Self {
            app: MeridianApp::headless(Boot::empty(), Mode::Light).running_with(Some(runner())),
            ctx: egui::Context::default(),
        };
        win.frame();
        win
    }

    fn frame(&mut self) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440.0, 900.0),
            )),
            ..Default::default()
        };
        let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
    }

    /// Ask the window's queue to open `path`, and run the frames that take it.
    fn open(&mut self, path: &Path) {
        self.app
            .open_queue()
            .push(OpenRequest::Path(path.to_path_buf()));
        self.frame();
        self.frame();
    }

    /// Say what the window raised, so a failed open's own words reach the assertion.
    fn said(&self) -> String {
        self.app
            .notifications()
            .iter()
            .map(|n| format!("{}: {}", n.title, n.body.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[test]
fn a_csv_opens_as_a_protocol_in_the_protocols_folder() {
    let folder = input("housing");
    let csv = folder.join("california_housing.csv");
    std::fs::copy(housing(), &csv).expect("the housing sample copies");
    let before = listing(&folder);

    let mut win = Window::new();
    win.open(&csv);

    let dir = protocols().join("california_housing");
    assert!(
        dir.join("arcform.yaml").is_file(),
        "the Protocol is made in the Protocols folder, named after its source: {}",
        win.said()
    );
    let loads = load_steps(&dir);
    assert_eq!(loads.len(), 1, "one source, one load step: {loads:?}");
    let absolute = std::path::absolute(&csv).expect("an absolute path");
    assert!(
        loads[0].contains(&*absolute.to_string_lossy()),
        "the load reads the CSV where it is: {}",
        loads[0]
    );
    assert_eq!(
        win.app.own_protocol().map(|p| p.dir.clone()),
        Some(dir.clone()),
        "the window holds the Protocol it made"
    );
    assert!(!win.app.front_door_is_live(), "the open replaced the door");
    assert!(win.app.model_pane().is_none(), "one table has no model");
    assert!(win.app.chart_doc().is_live(), "the dashboard is drawn");
    assert!(
        win.app.title().contains("california_housing"),
        "{}",
        win.app.title()
    );
    assert_eq!(
        listing(&folder),
        before,
        "nothing is written in the CSV's folder"
    );
}

#[test]
fn a_csv_with_spaces_and_accents_in_a_read_only_folder_opens() {
    use std::os::unix::fs::PermissionsExt as _;
    let folder = input("accents").join("Données brutes");
    std::fs::create_dir_all(&folder).expect("the folder");
    let csv = folder.join("houses 2024.csv");
    std::fs::copy(housing(), &csv).expect("the housing sample copies");
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555))
        .expect("the folder made read-only");
    let before = listing(&folder);

    let mut win = Window::new();
    win.open(&csv);
    let after = listing(&folder);
    // Writable again, so the scratch folder can be removed by the next run.
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755))
        .expect("the folder made writable");

    let dir = protocols().join("houses_2024");
    assert!(dir.join("arcform.yaml").is_file(), "{}", win.said());
    let loads = load_steps(&dir);
    assert_eq!(loads.len(), 1, "{loads:?}");
    assert!(
        loads[0].contains("Données brutes/houses 2024.csv"),
        "{}",
        loads[0]
    );
    assert!(win.app.chart_doc().is_live(), "{}", win.said());
    assert_eq!(after, before, "the read-only folder is unchanged");
}

#[test]
fn a_folder_of_two_tables_opens_the_model() {
    let folder = input("shop");
    std::fs::write(folder.join("customers.csv"), "id,name\n1,Ada\n2,Bo\n3,Cy\n").unwrap();
    std::fs::write(folder.join("orders.csv"), "id,customer_id\n10,1\n11,2\n").unwrap();

    let mut win = Window::new();
    win.open(&folder);

    let pane = win
        .app
        .model_pane()
        .unwrap_or_else(|| panic!("two tables open the model: {}", win.said()));
    assert_eq!(pane.protocol.sources.tables.len(), 2);
    assert_eq!(
        pane.protocol.dir,
        protocols().join("shop"),
        "the Protocol is made in the Protocols folder"
    );
    assert!(win.app.own_protocol().is_none(), "the model pane holds it");
}

#[test]
fn an_existing_protocol_opens_where_it_is() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/shipped-protocol");
    let dir = input("existing").join("shipped_sample");
    for file in [
        "arcform.yaml",
        "models/measures.sql",
        "data/measures.csv",
        "panels/overview.yaml",
    ] {
        let to = dir.join(file);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(fixture.join(file), &to).expect("the fixture copies");
    }
    assert_eq!(route(&dir), Route::Existing(dir.clone()));

    let mut win = Window::new();
    win.open(&dir);

    assert_eq!(
        win.app.panel_tiles().map(|t| t.dir().to_path_buf()),
        Some(dir.clone()),
        "the Protocol opens where it is, as its panels: {}",
        win.said()
    );
    assert!(protocols_named("shipped").is_empty(), "no Protocol is made");
    assert!(win.app.own_protocol().is_none());
}

#[test]
fn a_protocol_saved_beside_its_csv_by_an_earlier_brightfield_still_opens() {
    // What an earlier Brightfield left on Save: the manifest and its model beside the CSV, and the
    // chart under `panels/`, here with a line in it the generator never writes.
    let folder = input("saved");
    let csv = folder.join("housing_saved.csv");
    std::fs::copy(housing(), &csv).expect("the housing sample copies");
    let opened = data_file::open(&csv.to_string_lossy()).expect("the CSV opens");
    opened
        .protocol
        .save_to(&folder)
        .expect("the Protocol saves");
    let chart = std::fs::read_to_string(opened.spec_file.as_ref().expect("a generated spec"))
        .expect("the spec reads");
    let panel = folder.join("panels").join("housing_saved.yaml");
    std::fs::create_dir_all(panel.parent().unwrap()).unwrap();
    std::fs::write(&panel, format!("# kept by an earlier build\n{chart}")).unwrap();
    assert_eq!(route(&csv), Route::Existing(folder.clone()));

    let mut win = Window::new();
    win.open(&csv);

    assert_eq!(
        win.app.chart_doc().spec_path.as_deref(),
        Some(std::path::absolute(&panel).unwrap().as_path()),
        "the chart its panels hold is drawn: {}",
        win.said()
    );
    assert!(
        win.app.title().contains("housing_saved"),
        "{}",
        win.app.title()
    );
    assert!(
        protocols_named("housing_saved").is_empty(),
        "no Protocol is made"
    );
}

/// The rows of the table the grid holds, as the profile counted them.
fn rows_shown(win: &Window) -> u64 {
    win.app
        .protocol_model()
        .source()
        .and_then(|s| s.columns.first())
        .map_or(0, |c| c.rows)
}

/// The housing sample cut to its header and first `rows` rows.
fn housing_rows(rows: usize) -> String {
    let text = std::fs::read_to_string(housing()).expect("the sample reads");
    text.lines()
        .take(rows + 1)
        .map(|l| format!("{l}\n"))
        .collect()
}

#[test]
fn a_one_source_protocol_opens_again_from_its_folder_and_its_manifest() {
    let folder = input("reopened");
    let csv = folder.join("reopened.csv");
    std::fs::copy(housing(), &csv).expect("the housing sample copies");
    Window::new().open(&csv);
    let dir = protocols().join("reopened");
    assert!(dir.join("arcform.yaml").is_file());

    for path in [dir.clone(), dir.join("arcform.yaml")] {
        let mut win = Window::new();
        win.open(&path);
        assert_eq!(
            win.app.own_protocol().map(|p| p.dir.clone()),
            Some(dir.clone()),
            "{}: the Protocol opens again on its table: {}",
            path.display(),
            win.said()
        );
        assert!(win.app.chart_doc().is_live(), "{}", win.said());
        assert_eq!(rows_shown(&win), 240);
        assert!(
            win.app.notifications().iter().next().is_none(),
            "{}",
            win.said()
        );
    }
    assert_eq!(
        protocols_named("reopened"),
        ["reopened"],
        "opening it again makes no other Protocol"
    );
}

#[test]
fn reloading_the_data_rereads_the_analysts_file() {
    let folder = input("reloaded");
    let csv = folder.join("reloaded.csv");
    std::fs::copy(housing(), &csv).expect("the housing sample copies");
    let mut win = Window::new();
    win.open(&csv);
    assert_eq!(rows_shown(&win), 240, "{}", win.said());

    std::fs::write(&csv, housing_rows(5)).expect("the CSV rewritten");
    let ctx = win.ctx.clone();
    win.app.reload_data_file(&ctx);
    win.frame();

    assert_eq!(
        rows_shown(&win),
        5,
        "the reload reads the file as it now is: {}",
        win.said()
    );
    assert!(win.app.own_protocol().is_some());
}

#[test]
fn saving_after_a_new_open_writes_into_the_protocol_and_not_its_grid_copy() {
    let folder = input("kept");
    let csv = folder.join("kept.csv");
    std::fs::copy(housing(), &csv).expect("the housing sample copies");
    let mut win = Window::new();
    win.open(&csv);
    let dir = protocols().join("kept");

    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some("house_age > 28".to_string()),
            order: None,
        });
    win.frame();
    let ctx = win.ctx.clone();
    let saved = win
        .app
        .save_protocol(&ctx)
        .expect("the window has a Protocol to save");
    win.frame();
    let saved = saved.unwrap_or_else(|e| panic!("the Save failed: {e}; {}", win.said()));

    assert_eq!(saved, dir.join("arcform.yaml"));
    let manifest = std::fs::read_to_string(dir.join("arcform.yaml")).unwrap();
    assert!(
        manifest.contains("filtered_by_house_age"),
        "the Protocol's manifest carries the filter: {manifest}"
    );
    assert!(
        manifest.contains("load_kept"),
        "its own load is kept: {manifest}"
    );
    let grid: Vec<String> = std::fs::read_dir(dir.join(".grid"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        grid.iter().all(|n| n.ends_with(".parquet")),
        "nothing but the copy is under .grid/: {grid:?}"
    );
    let recent = win
        .app
        .layout()
        .recents
        .iter()
        .find(|r| r.id.contains("kept"))
        .map(|r| PathBuf::from(&r.id))
        .expect("the Save is remembered");
    assert_eq!(
        recent.parent(),
        Some(dir.as_path()),
        "Recents names the Protocol folder: {}",
        recent.display()
    );
}

#[test]
fn a_queued_open_that_fetches_an_extension_draws_the_notice_first_and_loses_no_request() {
    scratch();
    let mut win = Window {
        app: MeridianApp::headless(Boot::empty(), Mode::Light)
            .running_with(Some(runner()))
            .knowing_extensions(Vec::new()),
        ctx: egui::Context::default(),
    };
    win.frame();
    let first = "/tmp/bf-no-such-first-book.xlsx";
    let second = "/tmp/bf-no-such-second-book.xlsx";
    let queue = win.app.open_queue();
    queue.push(OpenRequest::Path(first.into()));
    queue.push(OpenRequest::Path(second.into()));

    win.frame();
    assert_eq!(
        win.app.pending_open(),
        Some(first),
        "the first open waits, and the frame that took it draws the notice: {}",
        win.said()
    );
    assert!(
        win.said().contains("Fetching an extension"),
        "{}",
        win.said()
    );
    assert!(!win.said().contains("Could not open"), "{}", win.said());

    win.frame();
    assert!(
        win.said().contains("bf-no-such-first-book"),
        "the next frame ran the first open: {}",
        win.said()
    );
    assert_eq!(
        win.app.pending_open(),
        Some(second),
        "the second request waited in the queue"
    );

    win.frame();
    assert_eq!(win.app.pending_open(), None);
    assert!(
        win.said().contains("bf-no-such-second-book"),
        "{}",
        win.said()
    );
}

/// The names of the columns the grid's table holds.
fn columns_shown(win: &Window) -> Vec<String> {
    win.app
        .protocol_model()
        .source()
        .map(|s| s.columns.iter().map(|c| c.column.clone()).collect())
        .unwrap_or_default()
}

/// The housing sample with each line passed through `edit`, the header as line 0.
fn housing_edited(edit: impl Fn(usize, &str) -> String) -> String {
    let text = std::fs::read_to_string(housing()).expect("the sample reads");
    text.lines()
        .enumerate()
        .map(|(i, l)| format!("{}\n", edit(i, l)))
        .collect()
}

#[test]
fn reloading_after_the_file_gains_a_column_shows_it() {
    let folder = input("gains");
    let csv = folder.join("gains.csv");
    std::fs::copy(housing(), &csv).expect("the housing sample copies");
    let mut win = Window::new();
    win.open(&csv);
    assert!(!columns_shown(&win).contains(&"rating".to_string()));

    std::fs::write(
        &csv,
        housing_edited(|i, l| {
            if i == 0 {
                format!("{l},rating")
            } else {
                format!("{l},{}", i % 5)
            }
        }),
    )
    .expect("the CSV rewritten");
    let ctx = win.ctx.clone();
    win.app.reload_data_file(&ctx);
    win.frame();

    assert!(
        columns_shown(&win).contains(&"rating".to_string()),
        "the new column is listed: {:?}; {}",
        columns_shown(&win),
        win.said()
    );
    assert_eq!(rows_shown(&win), 240);
}

#[test]
fn reloading_after_a_column_is_renamed_shows_the_new_name() {
    let folder = input("renames");
    let csv = folder.join("renames.csv");
    std::fs::copy(housing(), &csv).expect("the housing sample copies");
    let mut win = Window::new();
    win.open(&csv);
    assert!(columns_shown(&win).contains(&"house_age".to_string()));

    std::fs::write(
        &csv,
        housing_edited(|i, l| {
            if i == 0 {
                l.replace("house_age", "building_age")
            } else {
                l.to_string()
            }
        }),
    )
    .expect("the CSV rewritten");
    let ctx = win.ctx.clone();
    win.app.reload_data_file(&ctx);
    win.frame();

    let columns = columns_shown(&win);
    assert!(
        columns.contains(&"building_age".to_string())
            && !columns.contains(&"house_age".to_string()),
        "the grid shows the new name: {columns:?}; {}",
        win.said()
    );
    assert!(
        win.app.notifications().iter().next().is_none(),
        "{}",
        win.said()
    );
}

#[test]
fn opening_a_one_source_protocol_again_does_not_rewrite_it() {
    let folder = input("untouched");
    let csv = folder.join("untouched.csv");
    std::fs::copy(housing(), &csv).expect("the housing sample copies");
    Window::new().open(&csv);
    let dir = protocols().join("untouched");
    let manifest = std::fs::read_to_string(dir.join("arcform.yaml")).unwrap();
    let models = listing(&dir.join("models"));

    let mut win = Window::new();
    win.open(&dir);
    assert!(win.app.own_protocol().is_some(), "{}", win.said());
    assert_eq!(
        std::fs::read_to_string(dir.join("arcform.yaml")).unwrap(),
        manifest
    );
    assert_eq!(listing(&dir.join("models")), models);
}

#[test]
fn reloading_keeps_a_recorded_view_step() {
    let folder = input("keeps");
    let csv = folder.join("keeps.csv");
    std::fs::copy(housing(), &csv).expect("the housing sample copies");
    let mut win = Window::new();
    win.open(&csv);
    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some("house_age > 28".to_string()),
            order: None,
        });
    win.frame();
    let ctx = win.ctx.clone();
    win.app
        .save_protocol(&ctx)
        .expect("a Protocol to save")
        .unwrap_or_else(|e| panic!("the Save failed: {e}"));
    win.frame();

    std::fs::write(
        &csv,
        housing_edited(|i, l| {
            if i == 0 {
                format!("{l},rating")
            } else {
                format!("{l},{}", i % 5)
            }
        }),
    )
    .expect("the CSV rewritten");
    win.app.reload_data_file(&ctx);
    win.frame();

    let dir = protocols().join("keeps");
    let manifest = std::fs::read_to_string(dir.join("arcform.yaml")).unwrap();
    let order: Vec<&str> = manifest
        .lines()
        .filter_map(|l| l.trim().strip_prefix("- name: "))
        .collect();
    let at = |prefix: &str| {
        order
            .iter()
            .position(|n| n.trim_matches('\'').trim_matches('"').starts_with(prefix))
            .unwrap_or_else(|| panic!("{prefix} in {order:?}"))
    };
    assert!(
        at("silver_") < at("keeps_filtered_by_house_age"),
        "the silver step stays ahead of the view that reads it: {order:?}"
    );
    assert!(
        columns_shown(&win).contains(&"rating".to_string()),
        "{}",
        win.said()
    );
}
