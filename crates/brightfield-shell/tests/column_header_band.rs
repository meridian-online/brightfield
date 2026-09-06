//! **The grid pane's column header band**, read off laid-out frames at both
//! densities and checked against the fixture's own CSV.
//!
//! Three claims live here. That the band draws at the full density where the
//! grid is the canvas's view of the node, with the rows the compact density
//! does not have room for. That the extents differ by exactly the rows the
//! contract adds. And that the numbers the band states are the file's numbers
//! — mean, median, deviation, distinct count and bounds computed here, over
//! the CSV, by arithmetic that never goes near DuckDB.
//!
//! The compact half is `tests/canvas_pane_group.rs`'s, where the pane group is
//! already settled and the sideways-scroll driver already exists.
//!
//! # Why the oracle is the CSV and not a second query
//!
//! Every figure the band draws comes through the engine's profile pass. A test
//! that checked those against a second query would be checking DuckDB against
//! DuckDB and would stay green through a profile pass that read the wrong
//! column. [`fixture_stats`] parses every cell of the committed sample and does
//! the arithmetic in Rust, so the two answers have no common cause.
//!
//! **DuckDB's `median` interpolates**, so the oracle takes the mean of the
//! middle pair on an even row count rather than either of them — 240 rows, so
//! this is the case every column here exercises. The deviation is the SAMPLE
//! deviation, `stddev_samp`, over `n - 1`.

use brightfield_shell::column_header::GridDensity;
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::NodeView;
use brightfield_shell::window::{Boot, MeridianApp};

/// The committed table every window here opens over.
fn housing() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv")
}

/// The window the grid view's contract was measured in.
const SCREEN: (f32, f32) = (1440.0, 900.0);

/// A window that keeps its own `egui::Context` for its whole life, because a
/// click resolves against the widget id a previous frame registered.
struct Live {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Live {
    fn open() -> Self {
        let path = housing();
        let chosen = path.to_str().expect("utf-8 fixture path");
        let boot =
            Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        let mut live = Self {
            app: MeridianApp::headless(boot, Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SCREEN.0, SCREEN.1)),
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

    fn settle(&mut self) {
        self.run(vec![Vec::new(), Vec::new(), Vec::new()]);
    }

    /// One more frame, handing back every shape it painted.
    fn shapes(&mut self) -> Vec<egui::epaint::ClippedShape> {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            ..Default::default()
        };
        self.ctx.run_ui(raw, |ui| self.app.draw(ui)).shapes
    }

    /// Click where the last frame drew the navigator rail's row labelled
    /// `label` — the gesture that puts a node's view on the canvas.
    fn click_row(&mut self, label: &str) {
        let rows = self.app.spine_rows().to_vec();
        let row = rows
            .iter()
            .find(|row| row.label == label)
            .unwrap_or_else(|| {
                let drawn: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
                panic!("the rail drew no row labelled {label:?}; it drew {drawn:?}")
            });
        let at = row.rect.center();
        let mut events = vec![egui::Event::PointerMoved(at)];
        for pressed in [true, false] {
            events.push(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
        self.run(vec![events, Vec::new(), Vec::new()]);
    }

    /// What the grid's table laid out on the last frame.
    fn drawn(&self) -> brightfield_shell::data_grid::TableDrawn {
        self.app
            .chart_doc()
            .grid_drawn
            .clone()
            .expect("the grid pane laid a table out")
    }
}

/// Every text galley the frame painted, with the point it was painted at.
fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(egui::Pos2, String)> {
    fn walk(shape: &egui::epaint::Shape, into: &mut Vec<(egui::Pos2, String)>) {
        match shape {
            egui::epaint::Shape::Text(t) => into.push((t.pos, t.galley.text().to_string())),
            egui::epaint::Shape::Vec(shapes) => {
                for s in shapes {
                    walk(s, into);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

/// What the CSV says about one column — the independent oracle.
struct Stats {
    min: f64,
    max: f64,
    mean: f64,
    median: f64,
    sd: f64,
    distinct: usize,
    rows: usize,
    nulls: usize,
}

/// The fixture's columns, in file order, each with the arithmetic done here.
fn fixture_stats() -> std::collections::BTreeMap<String, Stats> {
    let text = std::fs::read_to_string(housing()).expect("the fixture reads");
    let mut lines = text.lines();
    let header: Vec<String> = lines
        .next()
        .expect("a header")
        .split(',')
        .map(str::to_string)
        .collect();
    let mut columns: Vec<Vec<f64>> = vec![Vec::new(); header.len()];
    let mut nulls = vec![0usize; header.len()];
    for line in lines.filter(|l| !l.trim().is_empty()) {
        for (i, cell) in line.split(',').enumerate() {
            if cell.trim().is_empty() {
                nulls[i] += 1;
                continue;
            }
            columns[i].push(
                cell.parse::<f64>()
                    .unwrap_or_else(|e| panic!("the fixture's {} cell {cell:?}: {e}", header[i])),
            );
        }
    }
    header
        .into_iter()
        .zip(columns)
        .zip(nulls)
        .map(|((name, mut values), nulls)| {
            let rows = values.len() + nulls;
            values.sort_by(f64::total_cmp);
            let n = values.len();
            let mean = values.iter().sum::<f64>() / n as f64;
            // DuckDB's median interpolates: on an even count it is the mean of
            // the middle pair, not either of them.
            let median = if n % 2 == 0 {
                (values[n / 2 - 1] + values[n / 2]) / 2.0
            } else {
                values[n / 2]
            };
            // The SAMPLE deviation — `stddev_samp`, over `n - 1`.
            let sd =
                (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0)).sqrt();
            let mut distinct: Vec<u64> = values.iter().map(|v| v.to_bits()).collect();
            distinct.dedup();
            let stats = Stats {
                min: values[0],
                max: values[n - 1],
                mean,
                median,
                sd,
                distinct: distinct.len(),
                rows,
                nulls,
            };
            (name, stats)
        })
        .collect()
}

/// The tolerance a `f64` the engine computed is compared to the same `f64`
/// computed here at: two sums over 240 doubles in different orders differ in
/// the last bits and in nothing else.
const EPSILON: f64 = 1e-9;

// ---------------------------------------------------------------------------
// AC2 — the band at the full density, where the grid is the canvas's view.
// ---------------------------------------------------------------------------

/// **AC2 — with the grid as the canvas's view of the table, the band draws at
/// the full density.**
///
/// Every row the contract adds over the compact density, read off the drawn
/// record: the finetype leaf and the storage type, the bar distribution, the
/// range, and the three caption rows. Then the extents, compared with the
/// compact band the same fixture draws beneath the hero — the difference is
/// stated as the rows that make it up rather than as one number, so a row that
/// changed height fails here naming itself.
#[test]
fn the_grid_as_the_canvas_view_draws_the_full_band() {
    let mut win = Live::open();
    let compact = win.drawn();
    assert!(
        !compact.band.is_empty(),
        "the rows pane beneath the hero drew no band at all, so nothing below \
         is a comparison between two densities"
    );

    win.click_row("grid");
    assert_eq!(
        win.app.canvas_holds().view(),
        Some(NodeView::Grid),
        "clicking the grid row puts the grid on the canvas"
    );
    let full = win.drawn();

    assert_eq!(
        full.band.len(),
        full.header_cells.len(),
        "the band drew a cell for each header cell the table laid out — \
         {} bands against {} header cells means the record is carrying \
         duplicates or dropping columns",
        full.band.len(),
        full.header_cells.len()
    );
    let mut seen: Vec<usize> = full.band.iter().map(|cell| cell.column).collect();
    let before = seen.len();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(
        seen.len(),
        before,
        "the band's record has more than one entry for some column: \
         `egui_table` offers a header cell once per scrolling region and once \
         more on its sizing pass, and the reduction that keeps one of them has \
         stopped"
    );

    for cell in &full.band {
        assert_eq!(cell.density, GridDensity::Full);
        assert!(
            cell.leaf.is_some() && cell.storage.is_some(),
            "the full band states the finetype leaf and the storage type: {cell:?}"
        );
        assert!(
            !cell.bars.is_empty(),
            "the full band draws a bar distribution: {cell:?}"
        );
        assert!(
            cell.rug.is_none(),
            "the bar distribution replaces the rug at this density: {cell:?}"
        );
        assert!(
            cell.range.is_some(),
            "the full band states the range: {cell:?}"
        );
        let stats = cell
            .stats
            .as_ref()
            .unwrap_or_else(|| panic!("the full band states the statistics: {cell:?}"));
        assert!(stats.mean_text.starts_with("mean "));
        assert!(stats.nulls_text.starts_with("nulls "));
        assert!(stats.median_text.starts_with("median "));
        assert!(stats.sd_text.starts_with("sd "));
        assert!(stats.distinct_text.ends_with(" distinct"));
    }

    // The extents, and the rows that separate them. The band's own record
    // carries the height it was drawn at, and `TableDrawn::header_height` is
    // what the widget was told — two answers that have to be the one number.
    let compact_extent = compact.band[0].extent;
    let full_extent = full.band[0].extent;
    assert!(
        (compact.header_height - compact_extent).abs() < f32::EPSILON
            && (full.header_height - full_extent).abs() < f32::EPSILON,
        "the height the table handed `egui_table` ({} compact, {} full) is not \
         the extent the band drew at ({compact_extent} and {full_extent})",
        compact.header_height,
        full.header_height
    );
    // The leaf-and-storage row (13), the bar chart over the rug (28 less 12),
    // two more points of range row (11 less 9), and three caption rows (13
    // each) — less the one row the compact density carries and the full one
    // does not, its own solo distinct-count row (13). The frames the contract
    // came from carry the two totals as 70 and 127, and 127 less 70 is this
    // sum.
    let added = 13.0 + (28.0 - 12.0) + (11.0 - 9.0) + 3.0 * 13.0 - 13.0;
    assert!(
        (full_extent - compact_extent - added).abs() < f32::EPSILON,
        "the full band is {full_extent} points and the compact one \
         {compact_extent}, a difference of {}, where the rows the contract \
         adds come to {added}",
        full_extent - compact_extent
    );

    // …and the rows are on screen, not merely in the record. Three of the
    // strings the full density adds, painted inside the band's own cells.
    let cells: Vec<egui::Rect> = full.band.iter().map(|cell| cell.cell).collect();
    let painted: Vec<String> = texts(&win.shapes())
        .into_iter()
        .filter(|(pos, _)| cells.iter().any(|r| r.contains(*pos)))
        .map(|(_, text)| text)
        .collect();
    for wanted in [
        full.band[0]
            .stats
            .as_ref()
            .expect("stats")
            .mean_text
            .clone(),
        full.band[0]
            .stats
            .as_ref()
            .expect("stats")
            .median_text
            .clone(),
        full.band[0]
            .stats
            .as_ref()
            .expect("stats")
            .distinct_text
            .clone(),
    ] {
        assert!(
            painted.contains(&wanted),
            "the record says the band drew {wanted:?} and no galley inside the \
             band's cells carries it. Painted there: {painted:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// AC3 — the values agree with the file.
// ---------------------------------------------------------------------------

/// **AC3 — the band's numbers are the file's numbers.**
///
/// `median_income` is the column the contract names, and it has 240 distinct
/// values in 240 rows — so it exercises the binned branch of the distribution
/// rather than the per-value one. `house_age` has 52, which is under the
/// per-value limit, so the two together cover both branches. Every figure is
/// compared against [`fixture_stats`], which reads the CSV.
#[test]
fn the_bands_values_are_the_values_the_file_holds() {
    let mut win = Live::open();
    win.click_row("grid");
    let drawn = win.drawn();
    let oracle = fixture_stats();

    for (name, bars) in [("median_income", 24usize), ("house_age", 52usize)] {
        let cell = drawn
            .band_named(name)
            .unwrap_or_else(|| panic!("the band drew no cell for {name}"));
        let want = oracle
            .get(name)
            .unwrap_or_else(|| panic!("the fixture has no column {name}"));
        let stats = cell
            .stats
            .as_ref()
            .unwrap_or_else(|| panic!("{name} drew no statistics"));

        assert_eq!(
            cell.missing as usize, want.nulls,
            "{name}: the band says {} rows are missing and the file has {}",
            cell.missing, want.nulls
        );
        assert_eq!(
            cell.valid as usize,
            want.rows - want.nulls,
            "{name}: the valid count is the rows less the missing ones"
        );
        assert_eq!(
            stats.nulls as usize, want.nulls,
            "{name}: the nulls caption"
        );
        assert_eq!(
            stats.distinct as usize, want.distinct,
            "{name}: the band says {} distinct values and the file has {}",
            stats.distinct, want.distinct
        );
        assert!(
            (stats.mean - want.mean).abs() < EPSILON,
            "{name}: mean {} against the file's {}",
            stats.mean,
            want.mean
        );
        assert!(
            (stats.median - want.median).abs() < EPSILON,
            "{name}: median {} against the file's {} — DuckDB's median \
             interpolates, so an even row count gives the mean of the middle \
             pair",
            stats.median,
            want.median
        );
        let sd = stats
            .sd
            .unwrap_or_else(|| panic!("{name}: 240 rows have a sample deviation"));
        assert!(
            (sd - want.sd).abs() < EPSILON,
            "{name}: sd {sd} against the file's {} — the sample deviation, \
             over n-1",
            want.sd
        );

        let (min, max) = cell
            .range
            .as_ref()
            .unwrap_or_else(|| panic!("{name} drew no range"));
        let parsed = |t: &str| {
            t.parse::<f64>()
                .unwrap_or_else(|e| panic!("{name}: the range text {t:?}: {e}"))
        };
        assert!(
            (parsed(min) - want.min).abs() < EPSILON,
            "{name}: the band's minimum {min} against the file's {}",
            want.min
        );
        assert!(
            (parsed(max) - want.max).abs() < EPSILON,
            "{name}: the band's maximum {max} against the file's {}",
            want.max
        );

        assert_eq!(
            cell.bars.len(),
            bars,
            "{name}: {} distinct values should draw {bars} bars — one per \
             value at 64 or fewer, and 24 bins above that",
            want.distinct
        );
    }

    // The two branches really are two branches, and not one number twice.
    assert_ne!(
        drawn.band_named("median_income").expect("drawn").bars.len(),
        drawn.band_named("house_age").expect("drawn").bars.len(),
        "both columns drew the same number of bars, so this test would hold \
         with the per-value branch deleted"
    );
}

// ---------------------------------------------------------------------------
// The two densities are told apart by where the pane is, not by what is in it.
// ---------------------------------------------------------------------------

/// **The density follows the pane's place.**
///
/// The same document, the same table and the same session draw the compact
/// band beneath the hero and the full band as the canvas's view. Nothing about
/// the data differs between the two frames, so this pins the one thing that
/// does: which branch of the canvas drew, and what it wrote on the document
/// before it did.
#[test]
fn the_same_table_draws_two_densities_by_where_its_pane_is() {
    let mut win = Live::open();
    let compact = win.drawn();
    assert!(
        compact
            .band
            .iter()
            .all(|c| c.density == GridDensity::Compact
                && c.leaf.is_none()
                && c.stats.is_none()
                && c.bars.is_empty()
                && c.rug.is_some()),
        "beneath the hero the band draws the compact rows and no others: {:?}",
        compact.band
    );

    win.click_row("grid");
    let full = win.drawn();
    assert!(
        full.band.iter().all(|c| c.density == GridDensity::Full),
        "as the canvas's view the band draws at the full density"
    );

    let names = |drawn: &brightfield_shell::data_grid::TableDrawn| -> Vec<String> {
        let mut out: Vec<String> = drawn.band.iter().map(|c| c.name.clone()).collect();
        out.sort();
        out
    };
    assert!(
        !names(&compact).is_empty() && !names(&full).is_empty(),
        "one of the two frames drew no columns"
    );
}

// ---------------------------------------------------------------------------
// AC2 — the compact band's own row states the distinct count.
// ---------------------------------------------------------------------------

/// **AC2 — the compact band draws the distinct count, off the file's own
/// numbers.**
///
/// `house_age` has 52 distinct values across 240 rows with no nulls, so a
/// regression that counted rows instead of distinct values reads 240 here,
/// not 52 — the two are asserted unequal first, so this pins nothing if a
/// future fixture swap makes them coincide. Checked against
/// [`fixture_stats`], which reads the CSV directly rather than through a
/// second query.
#[test]
fn the_compact_bands_own_row_states_the_distinct_count() {
    let mut win = Live::open();
    let drawn = win.drawn();
    let oracle = fixture_stats();
    let want = oracle.get("house_age").expect("the fixture has house_age");
    assert_ne!(
        want.distinct, want.rows,
        "this test pins nothing if house_age's distinct count and its row \
         count happen to be the same number"
    );

    let cell = drawn
        .band_named("house_age")
        .unwrap_or_else(|| panic!("the compact band drew no cell for house_age"));
    assert_eq!(cell.density, GridDensity::Compact);
    #[allow(clippy::cast_possible_truncation)]
    let want_distinct = want.distinct as u64;
    assert_eq!(
        cell.distinct,
        Some(want_distinct),
        "the compact band's distinct count is {:?} and the file has {}",
        cell.distinct,
        want.distinct
    );
    let text = cell
        .distinct_text
        .clone()
        .unwrap_or_else(|| panic!("house_age drew no distinct-count text: {cell:?}"));
    assert_eq!(text, format!("{} distinct", want.distinct));

    // …and it is on screen, not merely in the record.
    let painted: Vec<String> = texts(&win.shapes())
        .into_iter()
        .filter(|(pos, _)| cell.cell.contains(*pos))
        .map(|(_, text)| text)
        .collect();
    assert!(
        painted.contains(&text),
        "the record says the compact band drew {text:?} and no galley inside \
         the cell carries it. Painted there: {painted:?}"
    );
}

// ---------------------------------------------------------------------------
// The grid view says how much of the table is across, like the rows pane.
// ---------------------------------------------------------------------------

/// **The grid as the canvas's view carries the `N of M columns` readout too.**
///
/// It did not, and the reason its doc gave was that a grid with the whole
/// canvas usually fits the table. The full density's floor moves that premise:
/// nine columns at 128 points is 1152 before the scrollbar, and at 1440 by 900
/// the pane's content rect is not that wide once the rails and the pane inset
/// are taken out. This measures the pane rather than restating a number, and
/// then reads the note off the frame.
#[test]
fn the_grid_view_says_how_many_of_the_tables_columns_are_across() {
    let mut win = Live::open();
    win.click_row("grid");
    let panes = win.app.canvas_panes().clone();
    let grid = panes
        .panes
        .iter()
        .find(|p| p.name == "grid")
        .expect("the grid pane drew");
    let drawn = win.drawn();
    assert!(
        drawn.on_screen() < drawn.columns,
        "all {} columns fit the grid pane's {} points at {SCREEN:?}, so there \
         is no readout due and this test would hold with it deleted",
        drawn.columns,
        grid.body.width()
    );
    assert!(
        drawn.on_screen() > 0,
        "no column drew whole, so the record the readout is built from says \
         the grid put nothing on screen"
    );
    let (rect, note) = panes
        .rows_note
        .clone()
        .expect("the grid pane drew its readout");
    assert_eq!(
        note,
        format!("{} of {} columns", drawn.on_screen(), drawn.columns),
        "the readout does not say what the frame laid out"
    );
    assert!(
        grid.header.contains_rect(rect),
        "the readout at {rect:?} is not inside the grid pane's own header band \
         {:?}",
        grid.header
    );
}
