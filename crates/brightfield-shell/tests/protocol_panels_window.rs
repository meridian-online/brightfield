//! An agent-written Protocol opens in the window as tiles, one per panel: each
//! tile draws its chart or says, in its own words, why it cannot.
//!
//! The Protocols are real runs, as in `tests/protocol_panels.rs`: the CO2
//! fixture copied into a directory of the test's own and run by `arc` through
//! the `brightfield-shell` binary cargo built (`support/agent_protocol.rs` says
//! how the fetch step is kept off the network).

#[path = "support/agent_protocol.rs"]
mod agent_protocol;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use agent_protocol::{copied_fixture, copied_fixture_in, ran_fixture, run_protocol};
use brightfield_protocol::layout::Flow;
use brightfield_render::channel::Channel;
use brightfield_render::Scale;
use brightfield_shell::capture::capture_png_awaiting;
use brightfield_shell::design::Mode;
use brightfield_shell::protocol_panels::{
    strip_protocol_tables, with_protocol_tables, PanelProblem, ProtocolPanels,
};
use brightfield_shell::run::Runner;
use brightfield_shell::shelf::{RowEdit, SettingValue, TITLE_ROW};
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::channel::ShelfChannel;

const CO2: &str = "co2-per-capita";
const CHANGE: &str = "panels/co2_per_capita_change_pct.yaml";

/// The window the live binary opens when nothing was saved: the size these
/// tiles have to be readable at.
const DEFAULT_WINDOW: (f32, f32) = (1280.0, 820.0);

fn manifest(dir: &Path) -> String {
    dir.join("arcform.yaml").to_string_lossy().into_owned()
}

fn open(dir: &Path) -> Boot {
    Boot::open(&manifest(dir), Flow::Vertical, None)
        .unwrap_or_else(|e| panic!("{} did not open: {e}", dir.display()))
}

fn panels(boot: &Boot) -> &ProtocolPanels {
    boot.panels
        .as_ref()
        .expect("the Protocol opened by the agent-panels route")
}

fn edit(path: &Path, from: &str, to: &str) {
    let text = std::fs::read_to_string(path).expect("the file reads");
    assert!(text.contains(from), "{} has no {from:?}", path.display());
    std::fs::write(path, text.replace(from, to)).expect("the file writes");
}

fn collect_text(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
    match shape {
        egui::Shape::Text(text) => out.push((
            text.galley.text().to_owned(),
            text.galley.rect.translate(text.pos.to_vec2()),
        )),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, out);
            }
        }
        _ => {}
    }
}

/// A device-free window that keeps its context across frames, with the text
/// each frame painted.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    texts: Vec<String>,
    text_rects: Vec<(String, egui::Rect)>,
}

impl Window {
    fn over(boot: Boot) -> Self {
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light)
                .running_with(Some(Runner::at(env!("CARGO_BIN_EXE_brightfield-shell")))),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(DEFAULT_WINDOW.0, DEFAULT_WINDOW.1),
            ),
            texts: Vec::new(),
            text_rects: Vec::new(),
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
        let mut texts = Vec::new();
        for clipped in &out.shapes {
            collect_text(&clipped.shape, &mut texts);
        }
        self.texts = texts.iter().map(|(t, _)| t.clone()).collect();
        self.text_rects = texts;
    }

    /// Press and release the primary button at `at`, and settle.
    fn click(&mut self, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.run(vec![button(true), button(false)]);
        self.settle();
    }

    fn key(&mut self, key: egui::Key) {
        self.run(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        self.settle();
    }

    /// Where the last frame painted `text`.
    fn text_at(&self, text: &str) -> egui::Rect {
        self.text_rects
            .iter()
            .find(|(t, _)| t == text)
            .map(|(_, r)| *r)
            .unwrap_or_else(|| panic!("no {text:?} drawn: {:?}", self.texts))
    }

    fn selected(&self) -> Option<usize> {
        self.app.panel_tiles().and_then(|t| t.selected())
    }

    /// The CO2 bar tile, open as the chart.
    fn open_bar_tile(&mut self) -> usize {
        let at = self.tile_index(CHANGE);
        assert!(self.app.open_panel_tile(at));
        self.settle();
        assert_eq!(self.selected(), Some(at));
        at
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    fn drew(&self, text: &str) -> bool {
        self.texts.iter().any(|t| t.contains(text))
    }

    fn panels(&self) -> &ProtocolPanels {
        self.app
            .panel_tiles()
            .and_then(|tiles| tiles.panels())
            .expect("the window holds the Protocol's panels")
    }

    fn tile_index(&self, file: &str) -> usize {
        self.panels()
            .tiles
            .iter()
            .position(|t| t.file == Path::new(file))
            .unwrap_or_else(|| panic!("no tile for {file}"))
    }
}

/// The y categories of the bar tile's plot: one per bar.
fn bar_categories(panels: &ProtocolPanels) -> Vec<String> {
    let tile = panels
        .tiles
        .iter()
        .find(|t| t.file == Path::new(CHANGE))
        .expect("the bar tile");
    let Ok(composed) = &tile.outcome else {
        panic!("the bar tile did not draw");
    };
    match composed.plots[0].scales.get(Channel::Y) {
        Some(Scale::Band { categories, .. }) => categories.clone(),
        other => panic!("the bar tile's y scale is not a band: {other:?}"),
    }
}

#[test]
fn the_co2_protocol_opens_as_two_tiles() {
    let ran = ran_fixture(CO2);
    let boot = open(&ran.dir);
    {
        let panels = panels(&boot);
        assert_eq!(panels.tiles.len(), 2);
        for tile in &panels.tiles {
            assert!(tile.outcome.is_ok(), "{} did not draw", tile.file.display());
        }
    }
    assert!(
        !boot.has_chart(),
        "no generated dashboard stands in for the tiles"
    );

    let out = std::env::temp_dir().join(format!(
        "bf-protocol-panels-window-{}.png",
        std::process::id()
    ));
    let mut bodies: Vec<egui::Rect> = Vec::new();
    let (w, h) = capture_png_awaiting(
        boot,
        default_layout(),
        Mode::Light,
        1.0,
        DEFAULT_WINDOW,
        &out,
        Vec::new(),
        |_| {},
        |app| {
            bodies = app
                .panel_tiles()
                .map(|tiles| tiles.drawn().iter().map(|t| t.body).collect())
                .unwrap_or_default();
            true
        },
    )
    .expect("the window captures");
    let image = image::open(&out).expect("the capture reads").to_rgba8();
    let _ = std::fs::remove_file(&out);
    assert_eq!((image.width(), image.height()), (w, h));
    assert_eq!(bodies.len(), 2, "two tiles were drawn");
    assert!(
        !bodies[0].intersects(bodies[1]),
        "the tiles overlap: {bodies:?}"
    );
    for body in &bodies {
        assert!(body.width() >= 300.0 && body.height() >= 200.0, "{body:?}");
        // A tile with a chart in it holds many colours; a blank one, its
        // surface and little else.
        let mut colours = std::collections::BTreeSet::new();
        for y in (body.top() as u32..body.bottom() as u32).step_by(2) {
            for x in (body.left() as u32..body.right() as u32).step_by(2) {
                if x < image.width() && y < image.height() {
                    colours.insert(image.get_pixel(x, y).0);
                }
            }
        }
        assert!(
            colours.len() > 20,
            "the tile at {body:?} is blank: {} colours",
            colours.len()
        );
    }
}

#[test]
fn a_broken_tile_shows_its_message() {
    let ran = ran_fixture(CO2);
    edit(&ran.dir.join(CHANGE), "x: change_pct", "x: change_percent");
    let boot = open(&ran.dir);
    let message = {
        let tile = panels(&boot)
            .tiles
            .iter()
            .find(|t| t.file == Path::new(CHANGE))
            .expect("the broken tile");
        let Err(problems) = &tile.outcome else {
            panic!("the broken panel drew");
        };
        problems[0].message()
    };
    let win = Window::over(boot);

    assert!(win.drew(&message), "no {message:?} in {:?}", win.texts);
    assert!(
        win.texts.iter().any(|t| t == "co2_per_capita_change_pct"),
        "the tile names its panel: {:?}",
        win.texts
    );
    assert!(
        !win.texts.iter().any(|t| t.ends_with(".yaml")),
        "a tile's header spells its file's extension: {:?}",
        win.texts
    );
    let tiles = win.app.panel_tiles().expect("tiles");
    assert_eq!(tiles.drawn().len(), 2);
    assert!(tiles.drawn().iter().any(|t| t.drew), "the other tile draws");
}

#[test]
fn a_one_step_protocol_with_agent_panels_opens_by_the_new_route() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("tiny");
    std::fs::create_dir_all(dir.join("models")).unwrap();
    std::fs::create_dir_all(dir.join("panels")).unwrap();
    std::fs::write(dir.join("tiny.csv"), "country,value\nA,1\nB,3\nC,2\n").unwrap();
    std::fs::write(
        dir.join("arcform.yaml"),
        "name: tiny\nengine: duckdb\nsteps:\n  - name: load\n    sql: models/load.sql\n    \
         depends_on:\n      - tiny.csv\n    produces:\n      - tiny\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("models/load.sql"),
        "CREATE OR REPLACE TABLE tiny AS SELECT * FROM read_csv('tiny.csv');\n",
    )
    .unwrap();
    for (file, mark) in [("bars.yaml", "barX"), ("dots.yaml", "dot")] {
        std::fs::write(
            dir.join("panels").join(file),
            format!("plot:\n  - mark: {mark}\n    data: {{ from: tiny }}\n    x: value\n    y: country\n"),
        )
        .unwrap();
    }
    run_protocol(&dir);

    let boot = open(&dir);
    let panels = panels(&boot);
    assert_eq!(panels.tiles.len(), 2);
    for tile in &panels.tiles {
        assert!(tile.outcome.is_ok(), "{} did not draw", tile.file.display());
    }
    assert!(
        !boot.has_chart(),
        "a generated dashboard was opened instead"
    );
}

/// A folder made the way an analyst makes one: a window over the housing file
/// applies a view and Saves, which records it through arc and writes the chart
/// file beside it. Its `panels/` holds Brightfield's own file and nothing else.
#[test]
fn a_brightfield_protocol_opens_as_before() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = tmp.path().join("data");
    std::fs::create_dir_all(&folder).unwrap();
    let data = folder.join(starts::HOUSING_FILE);
    std::fs::write(&data, starts::HOUSING_PARQUET).unwrap();
    {
        let boot = Boot::data_file(&data.to_string_lossy()).expect("the housing file opens");
        let mut win = Window::over(boot);
        win.app
            .chart_doc_mut()
            .apply_grid_view(brightfield_engine::RowsView {
                condition: Some("house_age > 28".to_owned()),
                order: None,
            });
        win.settle();
        win.app
            .save_protocol(&win.ctx)
            .expect("the window has a Protocol to save")
            .expect("the Save records");
    }
    let panels_dir: Vec<PathBuf> = std::fs::read_dir(folder.join("panels"))
        .map(|entries| entries.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default();
    assert_eq!(
        panels_dir.len(),
        1,
        "Save wrote its own chart file: {panels_dir:?}"
    );

    let boot = open(&folder);
    assert!(boot.panels.is_none(), "the agent-panels route took it");
    assert!(boot.has_chart(), "the generated dashboard did not open");
    let win = Window::over(boot);
    assert!(win.app.panel_tiles().is_none());
    let steps: Vec<String> = win
        .app
        .protocol_model()
        .sheet()
        .rows()
        .iter()
        .map(|r| r.name.clone())
        .collect();
    assert_eq!(steps.len(), 2, "{steps:?}");
}

#[test]
fn saving_a_tile_writes_the_panel_without_injected_tables() {
    let ran = ran_fixture(CO2);
    let path = ran.dir.join(CHANGE);
    let before = std::fs::read_to_string(&path).unwrap();
    let mut win = Window::over(open(&ran.dir));
    let tables = win.panels().tables_dir().to_path_buf();
    let at = win.tile_index(CHANGE);

    assert!(win.app.open_panel_tile(at), "the tile opens as the chart");
    win.settle();
    let title = RowEdit {
        channel: ShelfChannel::X,
        row: TITLE_ROW,
        value: SettingValue::Text("percent change since 1990".to_owned()),
    };
    assert!(win.app.chart_doc_mut().set_axis_row(0, &title));
    win.settle();
    win.app
        .save_protocol(&win.ctx)
        .expect("a tile's Save writes its panel")
        .expect("the Save writes");

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("percent change since 1990"), "{after}");
    assert!(
        !after.contains(&*tables.to_string_lossy()) && !after.contains(".parquet"),
        "an injected table reached the file:\n{after}"
    );
    let changed: Vec<(&str, &str)> = before
        .lines()
        .zip(after.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(before.lines().count(), after.lines().count(), "{after}");
    assert_eq!(changed.len(), 1, "{changed:?}");
    assert!(changed[0].0.starts_with("xLabel:"), "{changed:?}");
    assert_eq!(
        before.ends_with('\n'),
        after.ends_with('\n'),
        "the file's last line ending changed"
    );
}

#[test]
fn strip_protocol_tables_is_the_inverse_of_with_protocol_tables() {
    let panel = "title: t\nplot:\n  - mark: dot\n    data: { from: tiny }\n    x: a\n    y: b\n";
    let dir = Path::new("/tmp/bf-tables");
    let injected = with_protocol_tables(panel, dir).unwrap();
    assert!(injected.contains("tiny.parquet"));
    let stripped = strip_protocol_tables(&injected, &["tiny".to_owned()]);
    assert!(!stripped.contains(".parquet"), "{stripped}");
    let a: serde_yaml::Value = serde_yaml::from_str(panel).unwrap();
    let b: serde_yaml::Value = serde_yaml::from_str(&stripped).unwrap();
    assert_eq!(a, b);
    // A text with no injected entry is handed back as it is.
    assert_eq!(strip_protocol_tables(panel, &["tiny".to_owned()]), panel);
}

#[test]
fn a_new_run_reloads_the_tiles() {
    let ran = copied_fixture_in(CO2, CO2);
    run_protocol(&ran.dir);
    let mut win = Window::over(open(&ran.dir));
    let first_run = win.panels().run_id.clone();
    assert_eq!(bar_categories(win.panels()).len(), 9);

    let model = ran.dir.join("models/03_co2_per_capita_change.sql");
    let text = std::fs::read_to_string(&model).unwrap();
    std::fs::write(&model, format!("{} LIMIT 3\n", text.trim_end())).unwrap();
    run_protocol(&ran.dir);

    let deadline = Instant::now() + Duration::from_secs(30);
    while win.panels().run_id == first_run {
        assert!(Instant::now() < deadline, "the tiles never reloaded");
        std::thread::sleep(Duration::from_millis(50));
        win.run(Vec::new());
    }
    assert_eq!(bar_categories(win.panels()).len(), 3);
    for tile in &win.panels().tiles {
        if let Err(problems) = &tile.outcome {
            let said: Vec<String> = problems.iter().map(PanelProblem::message).collect();
            panic!(
                "{} stopped drawing after the reload: {said:?}",
                tile.file.display()
            );
        }
    }
}

/// A Protocol that has not run opens saying so, and its Run, taken through the
/// window's own run path, lands as the tiles of the run it made.
#[test]
fn an_unrun_protocols_run_reloads_the_tiles() {
    let copy = copied_fixture(CO2);
    let boot = open(&copy.dir);
    assert!(
        boot.panels.is_none(),
        "an unrun Protocol has no panels to load"
    );
    let mut win = Window::over(boot);
    let tiles = win
        .app
        .panel_tiles()
        .expect("the agent-panels route took it");
    assert!(
        matches!(
            tiles.problem(),
            Some(brightfield_shell::protocol_panels::OpenProblem::NotRun { .. })
        ),
        "{:?}",
        tiles.problem()
    );
    assert!(win.drew("has not run"), "{:?}", win.texts);

    assert!(win.app.run_protocol(&win.ctx), "the run started");
    let deadline = Instant::now() + Duration::from_secs(180);
    while win.app.panel_tiles().and_then(|t| t.panels()).is_none() {
        assert!(Instant::now() < deadline, "the run never landed as tiles");
        std::thread::sleep(Duration::from_millis(50));
        win.run(Vec::new());
    }
    win.settle();
    assert_eq!(win.panels().tiles.len(), 2);
    assert!(win.panels().tiles.iter().all(|t| t.outcome.is_ok()));
    assert_eq!(win.app.panel_tiles().unwrap().drawn().len(), 2);
}

/// Tiles that cannot open for a reason that outlasts a reload are opened
/// again when the folder changes under them: a run made outside the window
/// reaches the rail, and the agent's first panel file opens the tiles.
#[test]
fn panels_added_to_a_protocol_without_any_open_as_tiles() {
    use brightfield_shell::protocol_panels::OpenProblem;
    let copy = copied_fixture(CO2);
    let mut win = Window::over(open(&copy.dir));
    let problem = |win: &Window| win.app.panel_tiles().and_then(|t| t.problem()).cloned();
    assert!(matches!(problem(&win), Some(OpenProblem::NotRun { .. })));

    // Until the change lands, frames are drawn on the tiles' own check cadence.
    let until = |win: &mut Window, what: &str, done: &dyn Fn(&Window) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !done(win) {
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(50));
            win.run(Vec::new());
        }
    };

    // The agent takes every panel away: the tiles say there are none.
    let panels_dir = copy.dir.join("panels");
    let aside = copy.dir.join("panels-aside");
    std::fs::rename(&panels_dir, &aside).unwrap();
    until(
        &mut win,
        "the tiles never said there were no panels",
        &|w| matches!(problem(w), Some(OpenProblem::NoPanels { .. })),
    );
    assert_eq!(win.app.protocol_model().last_run_word(), "not run");

    // A run made outside the window: the tiles still have no panels, and the
    // rail reads the run.
    run_protocol(&copy.dir);
    until(&mut win, "the rail never read the run", &|w| {
        w.app.protocol_model().last_run_word() != "not run"
    });
    assert!(matches!(problem(&win), Some(OpenProblem::NoPanels { .. })));

    // The agent writes its panels: the tiles open from the run.
    std::fs::create_dir(&panels_dir).unwrap();
    for entry in std::fs::read_dir(&aside).unwrap() {
        let from = entry.unwrap().path();
        std::fs::copy(&from, panels_dir.join(from.file_name().unwrap())).unwrap();
    }
    until(&mut win, "the panels never opened as tiles", &|w| {
        w.app.panel_tiles().and_then(|t| t.panels()).is_some()
    });
    win.settle();
    assert_eq!(win.panels().tiles.len(), 2);
    assert!(win.panels().tiles.iter().all(|t| t.outcome.is_ok()));
    assert_eq!(win.app.panel_tiles().unwrap().drawn().len(), 2);
}

/// Going back to every tile would drop an edit no file holds, so the window
/// stays on the open tile and says why.
#[test]
fn going_back_is_refused_while_the_open_tile_has_unsaved_edits() {
    let ran = ran_fixture(CO2);
    let mut win = Window::over(open(&ran.dir));
    let at = win.tile_index(CHANGE);
    assert!(win.app.open_panel_tile(at));
    win.settle();
    let title = RowEdit {
        channel: ShelfChannel::X,
        row: TITLE_ROW,
        value: SettingValue::Text("an unsaved title".to_owned()),
    };
    assert!(win.app.chart_doc_mut().set_axis_row(0, &title));
    win.settle();

    assert!(!win.app.back_to_panel_tiles(), "it went back over the edit");
    win.settle();
    assert_eq!(win.app.panel_tiles().unwrap().selected(), Some(at));
    assert!(win.app.chart_doc().has_unsaved_work());
    assert!(win.drew("All panels"), "the way back is still drawn");
}

/// A run that lands while a tile is open waits: the open tile's session reads
/// the snapshot a reload would replace. Going back draws the newer run.
#[test]
fn a_reload_waits_for_the_way_back_from_an_open_tile() {
    let ran = copied_fixture_in(CO2, CO2);
    run_protocol(&ran.dir);
    let mut win = Window::over(open(&ran.dir));
    let first_run = win.panels().run_id.clone();
    let at = win.tile_index(CHANGE);
    assert!(win.app.open_panel_tile(at));
    win.settle();

    let model = ran.dir.join("models/03_co2_per_capita_change.sql");
    let text = std::fs::read_to_string(&model).unwrap();
    std::fs::write(&model, format!("{} LIMIT 3\n", text.trim_end())).unwrap();
    run_protocol(&ran.dir);
    // Past several checks' worth of frames.
    let until = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
        win.run(Vec::new());
    }
    assert_eq!(
        win.panels().run_id,
        first_run,
        "it reloaded under the open tile"
    );
    assert_eq!(win.app.panel_tiles().unwrap().selected(), Some(at));

    assert!(win.app.back_to_panel_tiles());
    win.settle();
    assert_ne!(win.panels().run_id, first_run);
    assert_eq!(bar_categories(win.panels()).len(), 3);
}

/// A tile's header names its panel without `.yaml`; its tooltip says where the
/// file is and what it is.
#[test]
fn a_panels_name_drops_its_extension_and_its_tooltip_names_the_file() {
    use brightfield_shell::panel_tiles::{panel_about, panel_name};
    let file = Path::new(CHANGE);
    assert_eq!(panel_name(file), "co2_per_capita_change_pct");
    assert_eq!(
        panel_about(file),
        "panels/co2_per_capita_change_pct.yaml \u{b7} Mosaic spec"
    );
}

/// The Protocol rail lists the agent's steps and the tables they make, read
/// from the steps: the Protocol declares no `assets:`.
#[test]
fn the_protocol_rail_lists_the_steps_and_the_tables_they_make() {
    let ran = ran_fixture(CO2);
    // The committed manifest, fetch step and all: the run's copy dropped it
    // only to keep the run off the network.
    std::fs::copy(
        agent_protocol::fixture_dir(CO2).join("arcform.yaml"),
        ran.dir.join("arcform.yaml"),
    )
    .unwrap();
    let mut win = Window::over(open(&ran.dir));
    let steps: Vec<String> = win
        .app
        .protocol_model()
        .sheet()
        .rows()
        .iter()
        .map(|r| r.name.clone())
        .collect();
    assert_eq!(
        steps,
        [
            "fetch_owid_co2",
            "raw_owid_co2",
            "clean_co2_per_capita",
            "co2_per_capita_change",
            "china_overtakes_g7"
        ]
    );
    // Open the rail, which the tiles open shut, and read what it drew.
    let rail = brightfield_workbench::arrangement::NAVIGATOR_RAIL;
    assert!(win.app.rail_is_collapsed(rail));
    let toggle = win.app.rail_collapse_rect(rail).expect("the rail's toggle");
    win.click(toggle.center());
    assert!(!win.app.rail_is_collapsed(rail), "the rail did not open");
    assert!(!win.drew("No assets yet"), "{:?}", win.texts);
    let rows: Vec<String> = win
        .app
        .spine_rows()
        .iter()
        .map(|r| r.label.clone())
        .collect();
    for table in [
        "raw_owid_co2",
        "clean_co2_per_capita",
        "co2_per_capita_change",
        "china_overtakes_g7",
    ] {
        assert!(rows.iter().any(|r| r == table), "no {table} in {rows:?}");
    }
    // The tiles still hold the canvas.
    assert_eq!(win.app.panel_tiles().unwrap().drawn().len(), 2);
}

#[test]
fn the_all_panels_control_goes_back_to_the_tiles() {
    let ran = ran_fixture(CO2);
    let mut win = Window::over(open(&ran.dir));
    win.open_bar_tile();
    let control = win.text_at("\u{2190} All panels");
    win.click(control.center());
    assert_eq!(win.selected(), None);
    assert_eq!(win.app.panel_tiles().unwrap().drawn().len(), 2);
}

#[test]
fn the_protocols_crumb_goes_back_to_the_tiles() {
    let ran = ran_fixture(CO2);
    let mut win = Window::over(open(&ran.dir));
    win.open_bar_tile();
    assert_eq!(
        win.app.locator_crumbs(),
        [
            "Protocol \u{b7} co2-per-capita",
            "co2_per_capita_change_pct"
        ]
    );
    let crumb = win
        .app
        .crumb_link_rect()
        .expect("the Protocol's crumb is a link");
    win.click(crumb.center());
    assert_eq!(win.selected(), None);
    assert_eq!(win.app.locator_crumbs(), ["Protocol \u{b7} co2-per-capita"]);
}

#[test]
fn escape_goes_back_to_the_tiles() {
    let ran = ran_fixture(CO2);
    let mut win = Window::over(open(&ran.dir));
    win.open_bar_tile();
    win.key(egui::Key::Escape);
    assert_eq!(win.selected(), None);
}

/// Each way back stands down while the open panel holds an edit no file does.
#[test]
fn every_way_back_is_refused_over_an_unsaved_edit() {
    let ran = ran_fixture(CO2);
    let mut win = Window::over(open(&ran.dir));
    let at = win.open_bar_tile();
    let title = RowEdit {
        channel: ShelfChannel::X,
        row: TITLE_ROW,
        value: SettingValue::Text("an unsaved title".to_owned()),
    };
    assert!(win.app.chart_doc_mut().set_axis_row(0, &title));
    win.settle();

    win.key(egui::Key::Escape);
    assert_eq!(win.selected(), Some(at), "Esc went back over the edit");
    let crumb = win.app.crumb_link_rect().expect("the crumb");
    win.click(crumb.center());
    assert_eq!(
        win.selected(),
        Some(at),
        "the crumb went back over the edit"
    );
    let control = win.text_at("\u{2190} All panels");
    win.click(control.center());
    assert_eq!(
        win.selected(),
        Some(at),
        "the control went back over the edit"
    );
    assert!(win.app.chart_doc().has_unsaved_work());
}

/// The rail reads the steps' outcomes from the Protocol's newest run, as a
/// saved Protocol's rail does: a Protocol that ran does not say "not run".
/// (The run's own manifest, without the fetch step the runner drops to stay
/// off the network, so the run's steps are the manifest's.)
#[test]
fn the_rail_reads_the_protocols_newest_run() {
    let ran = ran_fixture(CO2);
    let win = Window::over(open(&ran.dir));
    assert_ne!(
        win.app.protocol_model().last_run_word(),
        "not run",
        "the rail does not read the Protocol's run"
    );
    assert!(!win.drew("not run"), "{:?}", win.texts);
}

/// Esc that takes a text field's keyboard away is the field's: the open panel
/// stays. The next Esc, with no field holding the keyboard, goes back.
#[test]
fn escape_leaves_a_text_field_before_it_leaves_the_panel() {
    let ran = ran_fixture(CO2);
    let mut win = Window::over(open(&ran.dir));
    let at = win.open_bar_tile();
    // The spec editor, in the ledger, holds the panel's text: click into it.
    let tab = win.text_at("Editor");
    win.click(tab.center());
    let line = win
        .text_rects
        .iter()
        .find(|(t, _)| t.contains("xLabel"))
        .map(|(_, r)| *r)
        .unwrap_or_else(|| panic!("the editor drew no panel text: {:?}", win.texts));
    win.click(line.center());
    assert!(
        win.ctx.memory(|m| m.focused().is_some()),
        "the editor did not take the keyboard"
    );

    win.key(egui::Key::Escape);
    assert!(
        win.ctx.memory(|m| m.focused().is_none()),
        "Esc left the field focused"
    );
    assert_eq!(
        win.selected(),
        Some(at),
        "Esc left the panel with the field"
    );

    win.key(egui::Key::Escape);
    assert_eq!(win.selected(), None, "the second Esc did not go back");
}

/// With a panel open the window is titled by the locator's crumbs, and the
/// locator's right edge names the panel as its tile does.
#[test]
fn an_open_panel_titles_the_window_and_the_locator_by_its_name() {
    let ran = ran_fixture(CO2);
    let mut win = Window::over(open(&ran.dir));
    win.open_bar_tile();
    let crumbs = "Protocol \u{b7} co2-per-capita \u{203a} co2_per_capita_change_pct";
    assert_eq!(win.app.title(), crumbs);
    assert!(win.texts.iter().any(|t| t == crumbs), "{:?}", win.texts);
    assert!(
        !win.texts.iter().any(|t| t.ends_with(".yaml")),
        "the window spells a panel's extension: {:?}",
        win.texts
    );
}
