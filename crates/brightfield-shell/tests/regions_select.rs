//! **A click inside a region's outline selects the region's rows.**
//!
//! The click publishes to the plot's own `intervalXY` selection, in place of
//! the brush's clause, so the plots already following that selection follow
//! the region. The clause is the region's grid cells over the extent they were
//! found on. Clicking the same region again clears it, Escape clears it, and a
//! drag still draws the rectangle brush. A plot with no `intervalXY` draws its
//! regions and offers no click.
//!
//! The fixture is the earthquake start's panel over the start's own table, in
//! a headless window, so the clicks go through the chart pane's own gesture
//! code. No GPU.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use arrow::array::{Array, Float64Array, Int64Array};
use brightfield_engine::{RecordBatch, SqlPredicate};
use brightfield_protocol::layout::Flow;
use brightfield_render::channel::Channel;
use brightfield_render::regions::cell_bounds;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::analysis::ComponentPath;

/// The map's selection in the earthquake panel.
const SELECTION: &str = "sel";

/// The weekly bars' layer: the map's dot and hexbin layers come first. The
/// bars highlight by the selection, so their query counts the selected rows
/// of each week beside the week's own count.
const WEEKLY_BARS: usize = 2;

/// The column a highlighting aggregate counts its selected rows in.
const SELECTED_COUNT: &str = "__bf_selected_count";

fn start_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/starts/usgs-earthquakes-2025")
        .canonicalize()
        .expect("the earthquake start is in the repository")
}

/// The start's panel, unchanged, over its table: the start's own model step as
/// a `query:` source reading the start's Parquet, as `tests/regions_overlay.rs`
/// builds it.
fn earthquake_spec() -> String {
    let dir = start_dir();
    let panel = std::fs::read_to_string(dir.join("panels/dashboard.yaml")).expect("panel");
    let model = std::fs::read_to_string(dir.join("models/02_earthquakes.sql")).expect("model");
    let parquet = dir.join("data/earthquakes_2025.parquet");
    let select = model
        .trim()
        .trim_start_matches("CREATE OR REPLACE TABLE \"earthquakes\" AS")
        .replace(
            "FROM raw_earthquakes",
            &format!("FROM '{}'", parquet.display()),
        );
    let select: String = select
        .trim()
        .lines()
        .map(|line| format!("      {line}\n"))
        .collect();
    format!("data:\n  earthquakes:\n    query: |\n{select}{panel}")
}

/// The panel with the map's hexbin layer taken out, so the map's frame follows
/// a zoom. (The hexbin layer holds the drawn axes at its lattice's extent, as
/// `tests/regions_overlay.rs` notes.) The weekly bars are then the second
/// layer.
fn earthquake_spec_without_hexbin() -> String {
    let spec = earthquake_spec();
    let start = spec
        .find("      - mark: hexbin\n")
        .expect("the hexbin layer");
    let end = start + spec[start..].find("binWidth: 14\n").expect("its last line") + 13;
    let mut out = spec[..start].to_string();
    out.push_str(&spec[end..]);
    out
}

/// The panel's map alone, its dot layer over the start's Parquet, with the
/// given gesture lines (empty for none).
fn earthquake_scatter(select: &str) -> String {
    let parquet = start_dir().join("data/earthquakes_2025.parquet");
    format!(
        "data:\n  earthquakes:\n    file: \"{}\"\nplot:\n  - mark: dot\n    \
         data: {{ from: earthquakes }}\n    x: longitude\n    y: latitude\n\
         {select}width: 760\nheight: 400\n",
        parquet.display()
    )
}

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-regions-select-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }

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

/// A window that keeps its own `egui::Context` for its whole life, as
/// `tests/regions_switch.rs` drives one.
struct Live {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    _dir: TempDir,
}

impl Live {
    fn open(name: &str, spec: &str) -> Self {
        let dir = TempDir::new(name);
        let path = dir.panel(spec);
        Self::boot(&path, dir)
    }

    fn boot(path: &Path, dir: TempDir) -> Self {
        let boot = Boot::open(path.to_str().expect("utf-8 path"), Flow::Vertical, None)
            .expect("the panel opens");
        let size = boot.window_size();
        let mut live = Self {
            app: MeridianApp::headless(boot, Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1)),
            _dir: dir,
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

    /// Frames until no region run is in flight, then two more so the page
    /// drawn is the one with the answers on it.
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

    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    /// Press and release the primary button over `pos`.
    fn click(&mut self, pos: egui::Pos2) {
        self.run(vec![
            vec![egui::Event::PointerMoved(pos)],
            vec![egui::Event::PointerMoved(pos), Self::button(pos, true)],
            vec![egui::Event::PointerMoved(pos), Self::button(pos, false)],
            Vec::new(),
            Vec::new(),
        ]);
    }

    /// Press at `from`, move to `to` in steps, and release there.
    fn drag(&mut self, from: egui::Pos2, to: egui::Pos2) {
        let mut frames = vec![
            vec![egui::Event::PointerMoved(from)],
            vec![egui::Event::PointerMoved(from), Self::button(from, true)],
        ];
        for step in 1..=6 {
            let p = from + (to - from) * (step as f32 / 6.0);
            frames.push(vec![egui::Event::PointerMoved(p)]);
        }
        frames.push(vec![egui::Event::PointerMoved(to), Self::button(to, false)]);
        frames.push(Vec::new());
        frames.push(Vec::new());
        self.run(frames);
    }

    fn key(&mut self, key: egui::Key) {
        self.run(vec![
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            Vec::new(),
        ]);
    }

    /// Where on screen the centre of region `region`'s peak cell, on plot
    /// `plot`, is drawn.
    fn region_point(&self, plot: usize, region: usize) -> egui::Pos2 {
        let doc = self.doc();
        let handle = &doc.composed.plots[plot];
        let regions = doc
            .live_dashboard()
            .expect("a live document")
            .regions()
            .drawn_on(handle)
            .expect("the plot's regions are drawn");
        let ((x0, x1), (y0, y1)) = cell_bounds(&regions.grid, regions.regions[region].peak);
        let (sx, sy) = (
            handle.scales.get(Channel::X).expect("an x scale"),
            handle.scales.get(Channel::Y).expect("a y scale"),
        );
        let local = (
            sx.map_f64(f64::midpoint(x0, x1)),
            sy.map_f64(f64::midpoint(y0, y1)),
        );
        let rect = self.app.composed_plot_rects()[plot];
        egui::pos2(rect.min.x + local.0 as f32, rect.min.y + local.1 as f32)
    }

    /// The clause the map holds in its selection now.
    fn held(&self) -> Option<SqlPredicate> {
        let path = ComponentPath(self.doc().composed.plots[0].path.clone());
        self.doc().held_clause(SELECTION, &path)
    }

    /// The selected rows the layer `mark` counts across its groups, or `None`
    /// when its query has no selected count (nothing is selected).
    fn selected_total(&mut self, mark: usize) -> Option<f64> {
        let batches = self
            .app
            .chart_doc_mut()
            .live_coordinator()
            .expect("a live document")
            .session_mut()
            .execute_mark(mark)
            .expect("the layer runs");
        let mut total = None;
        for batch in &batches {
            let Some(column) = batch.column_by_name(SELECTED_COUNT) else {
                continue;
            };
            let column = column
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("a DOUBLE count");
            let sum: f64 = (0..column.len())
                .filter(|&i| !column.is_null(i))
                .map(|i| column.value(i))
                .sum();
            *total.get_or_insert(0.0) += sum;
        }
        total
    }

    /// How many of the map's dot rows `clause` keeps, counted by DuckDB.
    fn rows_kept_by(&mut self, clause: &SqlPredicate) -> i64 {
        let session = self
            .app
            .chart_doc_mut()
            .live_coordinator()
            .expect("a live document")
            .session_mut();
        let rows = session.region_rows_sql(0).expect("the dot layer's rows");
        let batches = session
            .execute_uncached(&format!(
                "SELECT count(*) FROM ({rows}) AS r WHERE {clause}"
            ))
            .expect("the count runs");
        count_of(&batches)
    }

    /// The region the map draws emphasised as the one held, if any.
    fn emphasised(&self) -> Option<usize> {
        let doc = self.doc();
        doc.live_dashboard()
            .expect("a live document")
            .held_region(&doc.composed.plots[0])
    }
}

fn count_of(batches: &[RecordBatch]) -> i64 {
    let batch = batches.iter().find(|b| b.num_rows() > 0).expect("a row");
    batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("a BIGINT count")
        .value(0)
}

#[test]
fn clicking_a_region_filters_the_other_plots_to_its_rows() {
    let mut live = Live::open("filters", &earthquake_spec());
    assert_eq!(live.held(), None, "nothing is selected at open");
    assert_eq!(live.selected_total(WEEKLY_BARS), None);

    live.click(live.region_point(0, 0));
    let held = live.held().expect("the click selected the region");
    assert!(
        matches!(held, SqlPredicate::Cells { .. }),
        "the map holds the region's cells: {held}"
    );
    let rows = live.rows_kept_by(&held);
    assert!(rows > 0, "the region holds rows");
    assert!(rows < 8_502, "and not every row of the table: {rows}");
    assert_eq!(
        live.selected_total(WEEKLY_BARS),
        Some(rows as f64),
        "the weekly bars count the region's rows as selected"
    );
    assert_eq!(live.emphasised(), Some(0), "the region is drawn as held");
    let readout = live.doc().selection_sql().expect("a selection is shown");
    assert!(
        readout.starts_with("$sel = ") && readout.contains(&held.to_string()),
        "the readout shows the clause that ran: {readout}"
    );
}

#[test]
fn clicking_it_again_clears_the_selection() {
    let mut live = Live::open("again", &earthquake_spec());
    let at = live.region_point(0, 0);
    live.click(at);
    assert!(live.held().is_some(), "the first click selects");
    live.click(at);
    assert_eq!(live.held(), None, "the second click clears");
    assert_eq!(live.selected_total(WEEKLY_BARS), None);
    assert_eq!(live.doc().selection_sql(), None);
    assert_eq!(live.emphasised(), None);

    // A click on another region, while one is held, replaces it.
    live.click(at);
    let first = live.held().expect("selected again");
    live.click(live.region_point(0, 1));
    let second = live.held().expect("the other region is held");
    assert_ne!(first, second, "the second region replaced the first");
    assert_eq!(live.emphasised(), Some(1));
}

#[test]
fn escape_clears_a_region_selection() {
    let mut live = Live::open("escape", &earthquake_spec());
    live.click(live.region_point(0, 0));
    assert!(live.held().is_some());
    live.key(egui::Key::Escape);
    assert_eq!(live.held(), None, "escape cleared the region");
    assert_eq!(live.selected_total(WEEKLY_BARS), None);
    assert_eq!(live.emphasised(), None);
}

#[test]
fn a_drag_still_draws_the_rectangle_brush() {
    let mut live = Live::open("drag", &earthquake_spec());
    let from = live.region_point(0, 0);
    live.drag(from, from + egui::vec2(60.0, 40.0));
    let held = live.held().expect("the drag committed a brush");
    assert!(
        matches!(held, SqlPredicate::And(ref parts)
            if parts.iter().all(|p| matches!(p, SqlPredicate::Interval { .. }))),
        "a drag from inside a region is a rectangle brush: {held}"
    );
    assert!(
        live.doc().composed.plots[0].committed_rect.is_some(),
        "and its rectangle is drawn"
    );
    assert_eq!(live.emphasised(), None, "no region is drawn as held");
}

#[test]
fn a_plot_without_an_xy_brush_offers_no_click() {
    for (name, select) in [
        ("none", ""),
        ("x-only", "  - select: intervalX\n    as: $sel\n"),
    ] {
        let mut live = Live::open(name, &earthquake_scatter(select));
        let regions_drawn = {
            let doc = live.doc();
            doc.live_dashboard()
                .expect("live")
                .regions()
                .drawn_on(&doc.composed.plots[0])
                .is_some_and(|r| !r.regions.is_empty())
        };
        assert!(regions_drawn, "{name}: the regions are drawn all the same");
        live.click(live.region_point(0, 0));
        assert_eq!(live.held(), None, "{name}: the click selected nothing");
        assert_eq!(live.doc().selection_sql(), None, "{name}");
        assert_eq!(live.emphasised(), None, "{name}");
    }
}

#[test]
fn turning_regions_off_drops_a_held_region_selection() {
    let mut live = Live::open("off", &earthquake_spec());
    live.click(live.region_point(0, 0));
    assert!(live.held().is_some());
    assert!(
        live.app.chart_doc_mut().set_plot_regions(0, false),
        "the switch rebuilds the page"
    );
    live.settle();
    assert_eq!(
        live.held(),
        None,
        "the rebuild dropped the region selection"
    );
    assert_eq!(live.selected_total(WEEKLY_BARS), None);
    assert_eq!(live.doc().selection_sql(), None);
    assert!(
        !live.doc().selection_active(),
        "the document holds no gesture that the session no longer has"
    );
    assert_eq!(live.emphasised(), None, "and no held outline is drawn");
}

#[test]
fn a_held_region_keeps_its_cells_after_a_zoom() {
    let mut live = Live::open("zoom", &earthquake_spec_without_hexbin());
    // Without the hexbin layer the weekly bars are the second layer.
    let bars = 1;
    live.click(live.region_point(0, 0));
    let held = live.held().expect("the click selected the region");
    let selected = live.selected_total(bars);
    assert!(selected.is_some_and(|n| n > 0.0));
    assert_eq!(live.emphasised(), Some(0));
    let before = live
        .doc()
        .live_dashboard()
        .expect("live")
        .regions()
        .drawn_on(&live.doc().composed.plots[0])
        .expect("drawn")
        .extent
        .clone();

    assert!(live.app.chart_doc_mut().zoom_view(4.0), "the zoom settles");
    live.settle();
    let after = live
        .doc()
        .live_dashboard()
        .expect("live")
        .regions()
        .drawn_on(&live.doc().composed.plots[0])
        .expect("the zoomed map draws its own regions")
        .extent
        .clone();
    assert_ne!(after, before, "the zoom moved the grid");
    assert_eq!(
        live.held(),
        Some(held),
        "the selection keeps the cells it was made with"
    );
    assert_eq!(
        live.selected_total(bars),
        selected,
        "so it keeps the same rows"
    );
    assert_eq!(
        live.emphasised(),
        None,
        "the regions at the new view are new regions, none of them held"
    );
}
