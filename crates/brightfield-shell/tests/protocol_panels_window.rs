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
use brightfield_shell::versions::{Listing, BEFORE_NOT_HERE, NOT_ON_THIS_MACHINE};
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::arrangement::LEDGER_RAIL;
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
        assert!(self.app.open_panel_with_grid(at));
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

    /// Open the ledger rail on its Versions panel, the strip's fifth name.
    fn show_versions(&mut self) {
        let at = self
            .app
            .rail_name_rect(LEDGER_RAIL, 4)
            .expect("the ledger strip drew a fifth name")
            .center();
        self.click(at);
        assert_eq!(
            self.app.rail_pane_title(LEDGER_RAIL).as_deref(),
            Some("Versions")
        );
    }

    /// The Versions panel's head line and its rows' who, what changed and
    /// whether each is on this machine, newest first.
    fn versions(&self) -> (String, Vec<(String, String, bool)>) {
        match self.app.chart_doc().versions().listing() {
            Listing::Listed { head, rows } => (
                head.clone(),
                rows.iter()
                    .map(|r| (r.who.clone(), r.changed.clone(), r.on_this_machine))
                    .collect(),
            ),
            other => panic!("the Versions panel lists nothing: {other:?}"),
        }
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
fn the_line_tile_says_its_colours_repeat_and_the_bar_tile_does_not() {
    let ran = ran_fixture(CO2);
    let win = Window::over(open(&ran.dir));
    let note = "9 series share 8 colours, so colours repeat";
    assert!(win.drew(note), "no note in {:?}", win.texts);
    assert_eq!(
        win.texts
            .iter()
            .filter(|t| t.contains("colours repeat"))
            .count(),
        1,
        "the bar tile draws no note: {:?}",
        win.texts
    );
    let drawn = win.app.panel_tiles().expect("tiles").drawn();
    let line = drawn
        .iter()
        .find(|t| t.file != Path::new(CHANGE))
        .expect("the line tile");
    let at = win.text_at(
        win.texts
            .iter()
            .find(|t| t.contains("colours repeat"))
            .expect("the note"),
    );
    assert!(
        at.top() >= line.body.top() - 1.0 && at.bottom() <= line.body.top() + 40.0,
        "the note stands at the top of the tile's body: {at:?} in {:?}",
        line.body
    );
    // Beneath the note, not over the chart: the composed chart leaves its band.
    let composed = win.panels().tiles[win.tile_index(&line.file.to_string_lossy())]
        .outcome
        .as_ref()
        .expect("the line tile draws");
    assert!(
        line.body.top() + composed.height as f32 + 30.0 <= line.body.bottom() + 1.0,
        "the chart fits under the note"
    );
}

/// A tile says what its panel's load had to say, once per sentence, and the
/// lines are the ones `export-png` prints for the same panel: one list, so the
/// window and the exports never disagree about a panel.
#[test]
fn a_tile_shows_the_notes_the_export_prints_once_each() {
    let ran = ran_fixture(CO2);
    std::fs::write(
        ran.dir.join("panels/labelled.yaml"),
        "plot:\n\
         - mark: text\n  data: { from: co2_per_capita_change }\n  \
         x: change_pct\n  y: country\n  text: country\n  dy: -10\n\
         - mark: text\n  data: { from: co2_per_capita_change }\n  \
         x: change_pct\n  y: country\n  text: change_pct\n  dy: 10\n",
    )
    .expect("the panel writes");
    let win = Window::over(open(&ran.dir));
    let note = "mark `text` sets `dy`, which nothing in the render path reads — it has no effect";
    assert_eq!(
        win.texts.iter().filter(|t| t.as_str() == note).count(),
        1,
        "the tile says the note once though two marks set `dy`: {:?}",
        win.texts
    );
    let drawn = win.app.panel_tiles().expect("tiles").drawn();
    let tile = drawn
        .iter()
        .find(|t| t.file == Path::new("panels/labelled.yaml"))
        .expect("the labelled tile");
    let at = win.text_at(note);
    assert!(
        at.top() >= tile.body.top() - 1.0
            && at.bottom()
                <= tile.body.top() + brightfield_shell::protocol_panels::NOTE_HEIGHT as f32,
        "the note stands in the band at the top of the tile: {at:?} in {:?}",
        tile.body
    );

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_brightfield-shell"))
        .args(["export-png", &ran.dir.to_string_lossy(), "labelled"])
        .output()
        .expect("the binary runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let printed: Vec<String> = String::from_utf8_lossy(&out.stderr)
        .lines()
        .map(|l| {
            l.strip_prefix("panels/labelled.yaml: ")
                .unwrap_or_else(|| panic!("an export line names the panel: {l}"))
                .to_string()
        })
        .collect();
    assert_eq!(printed, [note], "the export prints what the tile shows");
}

/// A panel that sets more unread options than a tile has lines for shows the
/// first notes and a line counting the rest, and its chart stays inside the
/// tile under them rather than being pushed out of it.
#[test]
fn a_tile_with_many_notes_caps_them_and_keeps_its_chart_inside() {
    use brightfield_shell::protocol_panels::{MAX_NOTE_LINES, NOTE_HEIGHT};
    let ran = ran_fixture(CO2);
    std::fs::write(
        ran.dir.join("panels/busy.yaml"),
        "plot:\n\
         - mark: text\n  data: { from: co2_per_capita_change }\n  \
         x: change_pct\n  y: country\n  text: country\n  \
         dx: 4\n  dy: -10\n  fontSize: 8\n  opacity: 0.5\n  \
         fillOpacity: 0.5\n  strokeWidth: 2\n  rotate: 45\n",
    )
    .expect("the panel writes");
    let win = Window::over(open(&ran.dir));
    let unread = win
        .texts
        .iter()
        .filter(|t| t.contains("nothing in the render path reads"))
        .count();
    assert_eq!(unread, MAX_NOTE_LINES - 1, "{:?}", win.texts);
    assert!(
        win.drew("…and 5 more notes about this panel"),
        "the last line counts the rest: {:?}",
        win.texts
    );
    let drawn = win.app.panel_tiles().expect("tiles").drawn();
    let tile = drawn
        .iter()
        .find(|t| t.file == Path::new("panels/busy.yaml"))
        .expect("the busy tile");
    let chart = tile.chart.expect("the tile drew its chart");
    assert!(
        (chart.top() - (tile.body.top() + (NOTE_HEIGHT * MAX_NOTE_LINES as f64) as f32)).abs()
            < 1.0,
        "the chart starts under the capped band: {chart:?} in {:?}",
        tile.body
    );
    assert!(
        chart.bottom() <= tile.body.bottom() + 1.0 && chart.right() <= tile.body.right() + 1.0,
        "the chart stays inside the tile: {chart:?} in {:?}",
        tile.body
    );
}

/// In the narrowest window the note stays within its band, whatever it has to
/// give up to do so, and the chart starts below it.
#[test]
fn the_note_stays_in_its_band_in_a_narrow_window() {
    let ran = ran_fixture(CO2);
    let mut win = Window::over(open(&ran.dir));
    win.screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(560.0, 820.0));
    win.settle();
    let line = win
        .app
        .panel_tiles()
        .expect("tiles")
        .drawn()
        .iter()
        .find(|t| t.file != Path::new(CHANGE))
        .expect("the line tile")
        .body;
    let (_, at) = win
        .text_rects
        .iter()
        .find(|(t, _)| t.contains("colours repeat") || t.contains("series share"))
        .unwrap_or_else(|| panic!("no note drawn: {:?}", win.texts));
    assert!(
        at.bottom() <= line.top() + brightfield_shell::protocol_panels::NOTE_HEIGHT as f32,
        "the note overruns its band: {at:?} in {line:?}"
    );
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
    std::fs::write(&data, include_bytes!("data/california_housing.parquet")).unwrap();
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

    assert!(
        win.app.open_panel_with_grid(at),
        "the tile opens as the chart"
    );
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

/// The Versions panel lists the panel open from its tile, the file a Save
/// writes: first the agent's version from the Protocol's log, not on this
/// machine, and after a Save from the tile the Save's own rows above it.
#[test]
fn the_versions_panel_lists_the_open_tiles_panel_and_its_save() {
    let ran = ran_fixture(CO2);
    let history = tempfile::tempdir().expect("a temp store");
    let mut win = Window::over(open(&ran.dir));
    win.app
        .set_history(Some(brightfield_protocol::HistoryStore::At(
            history.path().join("history"),
        )));
    win.open_bar_tile();
    win.show_versions();

    let (head, rows) = win.versions();
    assert!(head.starts_with(CHANGE), "{head}");
    assert_eq!(
        rows,
        [(
            "arc (command line)".to_string(),
            NOT_ON_THIS_MACHINE.to_string(),
            false
        )]
    );

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
    win.settle();

    let (head, rows) = win.versions();
    assert!(head.starts_with(CHANGE), "{head}");
    let who: Vec<(&str, bool)> = rows.iter().map(|r| (r.0.as_str(), r.2)).collect();
    assert_eq!(
        who,
        [
            ("Brightfield", true),
            ("Brightfield", true),
            ("arc (command line)", false)
        ],
        "the Save and the text it replaced, above the agent's version: {rows:?}"
    );
    assert!(rows[0].1.contains("percent change since 1990"), "{rows:?}");
    assert_eq!(
        rows[1].1, BEFORE_NOT_HERE,
        "the text the Save replaced has no version before it on this machine"
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

    // The agent takes every panel away: the tiles say there are none.
    let panels_dir = copy.dir.join("panels");
    let aside = copy.dir.join("panels-aside");
    std::fs::rename(&panels_dir, &aside).unwrap();
    frames_until(
        &mut win,
        "the tiles never said there were no panels",
        &|w| matches!(problem(w), Some(OpenProblem::NoPanels { .. })),
    );
    assert_eq!(win.app.protocol_model().last_run_word(), "not run");

    // A run made outside the window: the tiles still have no panels, and the
    // rail reads the run.
    run_protocol(&copy.dir);
    frames_until(&mut win, "the rail never read the run", &|w| {
        w.app.protocol_model().last_run_word() != "not run"
    });
    assert!(matches!(problem(&win), Some(OpenProblem::NoPanels { .. })));

    // The agent writes its panels one at a time: the first opens the tiles
    // from the run, and the second joins them.
    std::fs::create_dir(&panels_dir).unwrap();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&aside)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    let place = |from: &Path| {
        std::fs::copy(from, panels_dir.join(from.file_name().unwrap())).unwrap();
    };
    place(&files[0]);
    frames_until(&mut win, "the first panel never opened as a tile", &|w| {
        w.app.panel_tiles().and_then(|t| t.panels()).is_some()
    });
    assert_eq!(win.panels().tiles.len(), 1);
    place(&files[1]);
    frames_until(&mut win, "the second panel never joined the tiles", &|w| {
        w.panels().tiles.len() == 2
    });
    win.settle();
    assert!(win.panels().tiles.iter().all(|t| t.outcome.is_ok()));
    assert_eq!(win.app.panel_tiles().unwrap().drawn().len(), 2);
}

/// Open tiles see a chart the agent adds, and one it rewrites, without a run.
#[test]
fn a_panel_added_or_rewritten_reloads_the_open_tiles() {
    let ran = ran_fixture(CO2);
    let mut win = Window::over(open(&ran.dir));
    assert_eq!(win.panels().tiles.len(), 2);
    let run = win.panels().run_id.clone();

    let added = "panels/co2_per_capita_change_pct_copy.yaml";
    std::fs::copy(ran.dir.join(CHANGE), ran.dir.join(added)).unwrap();
    frames_until(&mut win, "the added panel never showed", &|w| {
        w.panels().tiles.len() == 3
    });
    assert!(win
        .panels()
        .tiles
        .iter()
        .any(|t| t.file == Path::new(added)));

    let title = "Per-capita CO2, rewritten";
    edit(
        &ran.dir.join(CHANGE),
        "title: Change in per-capita CO2, 1990 to 2024 (percent)",
        &format!("title: {title}"),
    );
    frames_until(&mut win, "the rewritten title never showed", &|w| {
        w.panels()
            .tiles
            .iter()
            .any(|t| t.file == Path::new(CHANGE) && t.title.as_deref() == Some(title))
    });
    win.settle();
    assert_eq!(
        win.panels().run_id,
        run,
        "the tiles still draw the same run"
    );
    assert!(win.panels().tiles.iter().all(|t| t.outcome.is_ok()));
    assert_eq!(win.app.panel_tiles().unwrap().drawn().len(), 3);
}

/// Draw frames, on the tiles' own check cadence, until `done` holds.
fn frames_until(win: &mut Window, what: &str, done: &dyn Fn(&Window) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done(win) {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(50));
        win.run(Vec::new());
    }
}

/// Going back to every tile would drop an edit no file holds, so the window
/// stays on the open tile and says why.
#[test]
fn going_back_is_refused_while_the_open_tile_has_unsaved_edits() {
    let ran = ran_fixture(CO2);
    let mut win = Window::over(open(&ran.dir));
    let at = win.tile_index(CHANGE);
    assert!(win.app.open_panel_with_grid(at));
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
    assert!(win.app.open_panel_with_grid(at));
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

/// The rows the grid drew on the last frame, and its column count.
fn grid_drew(win: &Window) -> (u64, usize) {
    let drawn = win
        .app
        .chart_doc()
        .grid_drawn()
        .expect("the grid drew a table");
    (drawn.rows, drawn.columns)
}

/// A Protocol's panel opens as the chart document with the Data pane beside
/// it, as a data file's dashboard does, and the grid lists the table the
/// panel's marks read: the bar panel reads the one-row-per-country change
/// table (nine countries, nine columns), not the per-year table it is built
/// from (hundreds of rows, seven columns).
#[test]
fn a_protocol_panel_opens_beside_the_grid_of_the_table_it_reads() {
    let ran = ran_fixture(CO2);
    let mut win = Window::over(open(&ran.dir));
    assert_eq!(win.tile_index(CHANGE), 0, "the bar panel is the first tile");
    let countries = bar_categories(win.panels()).len();
    assert_eq!(countries, 9, "the change table holds one row per country");

    assert!(win.app.open_panel_with_grid(0), "the panel opens");
    win.settle();
    assert_eq!(win.selected(), Some(0));

    let panes = win.app.canvas_panes();
    assert!(
        panes.pane("map").is_some() && panes.pane("grid").is_some(),
        "the canvas did not draw the panel beside its grid"
    );
    assert_eq!(
        grid_drew(&win),
        (countries as u64, 9),
        "the grid lists a table other than the one the panel reads"
    );

    // A panel has no column of tiles to read as rows, so the grid stays a
    // table when the window's grid layout is the transposed one.
    win.app
        .set_grid_layout(brightfield_shell::app::GridLayout::Columns);
    win.settle();
    assert_eq!(
        grid_drew(&win),
        (countries as u64, 9),
        "under the transposed layout the panel's grid drew no table"
    );
    // …and its foot row takes a filter there, as it does in the rows layout:
    // an ask opened under the transposed layout outlives the frame.
    win.app.chart_doc_mut().foot_offer(
        brightfield_shell::grid_foot::FootField::Where,
        "grouping = 'G7'".into(),
    );
    win.settle();
    assert!(
        win.app.chart_doc().foot_asking().is_some(),
        "the frame put back the filter being typed in the panel's grid"
    );
    assert!(
        win.app.chart_doc().grid_layout_switch.is_none(),
        "a panel with no column to read as rows drew the layout switch"
    );

    // The committed panel declares no crossfilter selection: a typed filter
    // narrows its grid, leaves its chart whole, and raises no fault.
    win.app.chart_doc_mut().foot_cancel();
    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some("grouping = 'G7'".into()),
            ..Default::default()
        });
    win.settle();
    assert_eq!(grid_drew(&win).0, 7, "the grid did not take the filter");
    assert!(
        win.app.chart_doc().chart_fault().is_none(),
        "a filter on a panel with no crossfilter raised a fault: {:?}",
        win.app
            .chart_doc()
            .chart_fault()
            .map(|f| (f.title, f.detail))
    );
}

/// A filter typed in a Protocol panel's grid narrows the panel's charts when
/// the panel declares a crossfilter selection, through the same path as a
/// data file's dashboard. The committed panels declare none, so the copy's
/// bar panel is given one.
#[test]
fn a_typed_grid_filter_narrows_a_protocol_panels_charts() {
    let ran = ran_fixture(CO2);
    let path = ran.dir.join(CHANGE);
    edit(
        &path,
        "data: { from: co2_per_capita_change }",
        "data: { from: co2_per_capita_change, filterBy: $sel }",
    );
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        format!("params:\n  sel: {{ select: crossfilter }}\n{text}"),
    )
    .unwrap();
    let mut win = Window::over(open(&ran.dir));
    assert!(win.app.open_panel_with_grid(0), "the panel opens");
    win.settle();
    let bars = |win: &Window| match win.app.chart_doc().composed.plots[0].scales.get(Channel::Y) {
        Some(Scale::Band { categories, .. }) => categories.len(),
        other => panic!("the bar panel's y scale is not a band: {other:?}"),
    };
    assert_eq!(bars(&win), 9, "the panel opens whole");

    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some("grouping = 'G7'".into()),
            ..Default::default()
        });
    win.settle();

    assert_eq!(grid_drew(&win).0, 7, "the grid did not take the filter");
    assert_eq!(
        bars(&win),
        7,
        "the chart still draws the rows the grid filtered out"
    );
    assert!(
        win.app.chart_doc().chart_fault().is_none(),
        "a filter over the panel's own table raised a fault: {:?}",
        win.app
            .chart_doc()
            .chart_fault()
            .map(|f| (f.title, f.detail))
    );
}
