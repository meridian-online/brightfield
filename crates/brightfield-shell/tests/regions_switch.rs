//! **A plot whose regions can be drawn carries a switch that turns them off and
//! on, and the choice is saved with the chart as `regions: off`.**
//!
//! The switch sits at the plot's top-right, drawn with the tile controls
//! beside it, and only on a plot whose regions can be drawn: a dot layer over
//! two plain columns, on linear axes. Turning it off writes `regions: off` on
//! the plot through the same edit the scale switch makes, so Save puts it in
//! the file; turning it on takes the key out again, so a spec that never kept
//! the choice stays plain Mosaic, byte for byte.
//!
//! The switch's geometry comes off [`ChartDoc::regions_switches`], which the
//! chart pane writes as it draws, and the regions off the document's region
//! worker, so the tests need no GPU.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use brightfield_protocol::layout::Flow;
use brightfield_shell::app::{ChartDoc, GridLayout, RegionsSwitchDrawn};
use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::ast::SpecValue;
use brightfield_spec::layout::collect_plot_nodes;

/// Two square clusters of a hundred points on a dot plot, beside a bar chart
/// of the same rows: the first plot's regions can be drawn, the second's
/// cannot.
const PANEL: &str = r"data:
  t:
    query: |
      SELECT 20 + i % 10 AS x, 20 + (i // 10) % 10 AS y FROM range(100) r(i)
      UNION ALL SELECT 75 + i % 10, 75 + (i // 10) % 10 FROM range(100) r(i)
hconcat:
  - plot:
      - mark: dot
        data: { from: t }
        x: x
        y: y
    xDomain: [0, 100]
    yDomain: [0, 100]
    width: 360
    height: 300
  - plot:
      - mark: barY
        data: { from: t }
        x: x
        y: y
    width: 360
    height: 300
";

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-regions-switch-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }

    /// `text` written as `panel.yaml` in this directory.
    fn panel(&self, text: &str) -> PathBuf {
        let path = self.0.join("panel.yaml");
        std::fs::write(&path, text).expect("the panel is written");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A window that keeps its own `egui::Context` for its whole life, because a
/// click is resolved against the widget id a previous frame registered.
struct Live {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Live {
    fn open(path: &Path) -> Self {
        let boot = Boot::open(path.to_str().expect("utf-8 path"), Flow::Vertical, None)
            .expect("the panel opens");
        Self::boot(boot)
    }

    /// The window the committed housing table opens as a generated dashboard,
    /// which profiles the table at open.
    fn housing() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/california_housing_sample.csv");
        let boot = Boot::data_file(path.to_str().expect("utf-8 path"))
            .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        Self::boot(boot)
    }

    fn boot(boot: Boot) -> Self {
        let size = boot.window_size();
        let mut live = Self {
            app: MeridianApp::headless(boot, Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1)),
        };
        live.settle();
        live
    }

    fn run(&mut self, frames: Vec<Vec<egui::Event>>) {
        for events in frames {
            let raw = egui::RawInput {
                screen_rect: Some(self.screen),
                events,
                ..Default::default()
            };
            let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        }
    }

    /// Frames until the layout has settled and no region run is in flight,
    /// then one more so the page drawn is the one with the answers on it.
    fn settle(&mut self) {
        self.run(vec![Vec::new(), Vec::new(), Vec::new()]);
        let deadline = Instant::now() + Duration::from_secs(60);
        while self.doc().regions_pending() {
            assert!(Instant::now() < deadline, "the region run did not finish");
            std::thread::sleep(Duration::from_millis(5));
            self.run(vec![Vec::new()]);
        }
        self.run(vec![Vec::new(), Vec::new()]);
    }

    fn doc(&self) -> &ChartDoc {
        self.app.chart_doc()
    }

    fn switches(&self) -> Vec<RegionsSwitchDrawn> {
        self.doc().regions_switches.clone()
    }

    /// The switch drawn on plot `plot`, panicking with what was drawn instead.
    fn switch(&self, plot: usize) -> RegionsSwitchDrawn {
        let drawn = self.switches();
        drawn
            .iter()
            .find(|s| s.plot == plot)
            .cloned()
            .unwrap_or_else(|| panic!("no regions switch on plot {plot}; drawn: {drawn:?}"))
    }

    /// Press and release the primary button over `pos`, as five frames.
    fn click(&mut self, pos: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.run(vec![
            vec![egui::Event::PointerMoved(pos)],
            vec![egui::Event::PointerMoved(pos), button(true)],
            vec![egui::Event::PointerMoved(pos), button(false)],
            Vec::new(),
            Vec::new(),
        ]);
    }

    /// Click the state of plot `plot`'s switch that turns its regions `on` or
    /// off, and let the page settle.
    fn throw(&mut self, plot: usize, on: bool) {
        let switch = self.switch(plot);
        let (_, rect) = switch
            .hits()
            .find(|(state, _)| *state == on)
            .unwrap_or_else(|| panic!("the switch offers {on}: {switch:?}"));
        self.click(rect.center());
        self.settle();
    }

    /// The `regions` key the live spec holds on plot `plot`.
    fn regions_key(&self, plot: usize) -> Option<SpecValue> {
        let path = &self.doc().composed.plots[plot].path;
        let live = self.doc().live_dashboard().expect("a live document");
        collect_plot_nodes(live.spec())
            .into_iter()
            .find(|(p, _)| p == path)
            .and_then(|(_, node)| node.attributes.get("regions").cloned())
    }

    /// Whether plot `plot` has regions drawn on it now.
    fn draws_regions(&self, plot: usize) -> bool {
        let live = self.doc().live_dashboard().expect("a live document");
        live.regions()
            .drawn_on(&self.doc().composed.plots[plot])
            .is_some_and(|r| !r.regions.is_empty())
    }

    /// Throw the grid pane's layout switch to columns, the layout in which a
    /// press on a tile body selects its column, as `tests/tile_scale_switch.rs`
    /// does before its own press test.
    fn transpose(&mut self) {
        let at = self
            .doc()
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

    fn save(&mut self, panel: &Path) {
        self.app
            .chart_doc_mut()
            .save_chart_at(panel, None)
            .expect("the chart saves");
    }
}

#[test]
fn the_switch_shows_only_on_qualifying_plots() {
    let dir = TempDir::new("qualifying");
    let live = Live::open(&dir.panel(PANEL));
    let plots: Vec<usize> = live.switches().iter().map(|s| s.plot).collect();
    assert_eq!(plots, vec![0], "the dot plot carries one, the bars none");

    let switch = live.switch(0);
    let tile = live.app.composed_plot_rects()[0];
    assert!(
        tile.contains_rect(switch.rect),
        "the switch {:?} sits inside its plot {tile:?}",
        switch.rect
    );
    assert!(
        switch.rect.right() > tile.center().x && switch.rect.top() < tile.center().y,
        "at the plot's top-right: {:?} in {tile:?}",
        switch.rect
    );
    assert!(switch.active, "the regions are on until turned off");
    assert!(live.draws_regions(0), "and they are drawn");

    // A dot plot on a log axis cannot have its regions drawn, so no switch.
    let logged = PANEL.replacen("xDomain: [0, 100]", "xScale: log\n    xDomain: [1, 100]", 1);
    let dir = TempDir::new("log");
    let live = Live::open(&dir.panel(&logged));
    assert!(
        live.switches().is_empty(),
        "a log axis offers no regions switch: {:?}",
        live.switches()
    );
}

#[test]
fn turning_regions_off_writes_the_attribute_and_hides_them() {
    let dir = TempDir::new("off");
    let mut live = Live::open(&dir.panel(PANEL));
    assert!(live.draws_regions(0));

    live.throw(0, false);
    assert_eq!(
        live.regions_key(0),
        Some(SpecValue::String("off".to_string())),
        "the plot now says `regions: off`"
    );
    assert!(!live.draws_regions(0), "and draws no regions");
    let path = live.doc().composed.plots[0].path.clone();
    let worker = live.doc().live_dashboard().expect("live").regions();
    assert!(worker.regions(&path).is_none(), "no answer is held for it");
    assert_eq!(worker.started(), 0, "and no run starts for it");
    let switch = live.switch(0);
    assert!(!switch.active, "the switch stays, showing off");
}

#[test]
fn turning_them_on_removes_the_attribute() {
    let dir = TempDir::new("on");
    let panel = dir.panel(PANEL);
    let mut live = Live::open(&panel);
    live.throw(0, false);
    live.throw(0, true);
    assert_eq!(live.regions_key(0), None, "the key is gone, not set to on");
    assert!(live.switch(0).active);
    assert!(live.draws_regions(0), "the regions are drawn again");
    live.save(&panel);
    assert_eq!(
        std::fs::read_to_string(&panel).expect("the panel reads"),
        PANEL,
        "off and on again saves the file as it was"
    );

    // A file that kept `regions: off` loses the line when turned on.
    let kept = PANEL.replacen("    width: 360", "    regions: off\n    width: 360", 1);
    let dir = TempDir::new("kept");
    let panel = dir.panel(&kept);
    let mut live = Live::open(&panel);
    assert!(!live.switch(0).active);
    live.throw(0, true);
    live.save(&panel);
    assert_eq!(std::fs::read_to_string(&panel).expect("reads"), PANEL);
}

#[test]
fn a_spec_saved_with_regions_off_reopens_with_them_off() {
    let dir = TempDir::new("reopen");
    let panel = dir.panel(PANEL);
    let mut live = Live::open(&panel);
    live.throw(0, false);
    live.save(&panel);
    let saved = std::fs::read_to_string(&panel).expect("the panel reads");
    assert!(saved.contains("regions: off"), "the file says so:\n{saved}");

    let reopened = Live::open(&panel);
    assert_eq!(
        reopened.regions_key(0),
        Some(SpecValue::String("off".to_string()))
    );
    assert!(!reopened.switch(0).active, "the switch opens showing off");
    assert!(!reopened.draws_regions(0), "and the plot opens with none");
}

#[test]
fn regions_under_plot_defaults_leave_the_switch_on_and_save_works() {
    let defaulted = format!("plotDefaults:\n  regions: off\n{PANEL}");
    let dir = TempDir::new("defaults");
    let panel = dir.panel(&defaulted);
    let mut live = Live::open(&panel);
    assert!(
        live.switch(0).active,
        "`regions` under `plotDefaults:` is not handed to the plot"
    );
    assert!(live.draws_regions(0), "so its regions draw");

    live.throw(0, false);
    live.save(&panel);
    let saved = std::fs::read_to_string(&panel).expect("the panel reads");
    assert!(
        saved.contains("    regions: off\n"),
        "the plot's own `regions: off` is saved:\n{saved}"
    );

    live.throw(0, true);
    assert!(live.draws_regions(0));
    live.save(&panel);
    assert_eq!(
        std::fs::read_to_string(&panel).expect("the panel reads"),
        defaulted,
        "turning them on takes the plot's own line out and leaves the defaults"
    );
}

/// The plot of the housing dashboard whose regions can be drawn: its hero
/// point map.
fn regions_plot(live: &Live) -> usize {
    (0..live.doc().composed.plots.len())
        .find(|&i| live.doc().region_job(i).is_some())
        .expect("the housing dashboard has a plot with regions")
}

#[test]
fn a_tile_switch_keeps_the_profiles_the_region_names_read() {
    let mut live = Live::housing();
    let plot = regions_plot(&live);
    let read = |live: &Live| {
        let job = live.doc().region_job(plot).expect("a region job");
        (job.profiles, job.representations)
    };
    let opened = read(&live);
    assert!(
        opened.0.as_ref().is_some_and(|p| !p.is_empty()),
        "the open profiled the table, so a job carries its profiles"
    );

    let scale = live.doc().scale_switches[0].clone();
    assert!(
        live.app.chart_doc_mut().set_plot_scale(
            scale.plot,
            scale.axis,
            brightfield_spec::layout::ScaleType::Log
        ),
        "the scale switch rebuilds the page"
    );
    live.settle();
    assert_eq!(read(&live), opened, "a scale switch keeps them");

    live.throw(plot, false);
    live.throw(plot, true);
    assert_eq!(read(&live), opened, "the regions switch keeps them");
}

#[test]
fn a_press_on_the_regions_switch_is_not_a_press_on_the_canvas() {
    let mut live = Live::housing();
    live.transpose();
    let plot = regions_plot(&live);
    let other = live.doc().scale_switches[0].plot;
    let rect = live.app.composed_plot_rects()[other];
    live.click(egui::pos2(rect.center().x, rect.center().y));
    let selected = live.doc().selected_column().map(|c| c.column.clone());
    assert!(
        selected.is_some(),
        "the press on a tile body selected it, so the check below measures something"
    );

    let switch = live.switch(plot);
    live.click(switch.states[0].1.center());
    live.settle();
    assert_eq!(
        live.doc().selected_column().map(|c| c.column.clone()),
        selected,
        "the press on the regions switch moved the window's selected tile"
    );
    assert!(
        live.doc().selection_sql().is_none(),
        "the press on the switch committed {:?}",
        live.doc().selection_sql()
    );
}
