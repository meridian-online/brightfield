//! **The grid pane draws its columns as rows, one switch away from the rows it
//! opens in.**
//!
//! Every claim here is read off a **laid-out frame**. The control's geometry
//! comes off [`ChartDoc::grid_layout_switch`], which the canvas writes as it
//! draws; the rows come off [`ChartDoc::transposed_rows`], which is the record
//! the band's own painter leaves; the pictures beside them come off
//! [`MeridianApp::composed_plot_rects`], which resolves the two origins one
//! page is painted at. Three surfaces, so a row whose numbers and whose
//! picture came apart fails here rather than needing an eye on a screenshot.
//!
//! No GPU: `MeridianApp::headless` reserves the same boxes and paints no
//! raster, so the layout, the pane split and the gesture routing are the ones
//! the window runs. The pixels are `tests/dashboard_baseline.rs`'s half.

use brightfield_shell::app::{ChartDoc, GridLayout, LayoutSwitchDrawn};
use brightfield_shell::column_header::ColumnBandDrawn;
use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp};

/// The committed table these windows are opened over — the sample
/// `tests/canvas_pane_group.rs` and `tests/tile_scale_switch.rs` use.
fn housing() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv")
}

/// The tiles the fixture's nine columns produce, in the order the generator
/// stacks them — the hero's coordinate pair first, then the seven the column
/// beside it draws and the transposed layout draws as rows.
const TILE_ORDER: [&str; 7] = [
    "median_income",
    "house_age",
    "avg_rooms",
    "avg_bedrooms",
    "population",
    "avg_occupancy",
    "median_house_value",
];

/// A window that keeps its own `egui::Context` for its whole life, because a
/// click is resolved against the widget id a *previous* frame registered.
struct Live {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Live {
    /// A window over the fixture at the size that boot asks for, settled.
    fn open() -> Self {
        Self::open_at(None)
    }

    /// [`Live::open`] in a window of a named size — the short window the
    /// scroll claims are read in, where the rows cannot all stand at once.
    fn open_at(screen: Option<egui::Rect>) -> Self {
        let path = housing();
        let chosen = path.to_str().expect("utf-8 fixture path");
        let boot =
            Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        let size = boot.window_size();
        let mut live = Self {
            app: MeridianApp::headless(boot, Mode::Light),
            ctx: egui::Context::default(),
            screen: screen.unwrap_or_else(|| {
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1))
            }),
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

    /// Three frames with no events — one more than the layout needs, for the
    /// reason `tests/arrangement.rs` runs three.
    fn settle(&mut self) {
        self.run(vec![Vec::new(), Vec::new(), Vec::new()]);
    }

    /// Press and release the primary button over `pos`, as frames: egui
    /// resolves a click against the widget id the previous frame registered.
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

    /// The frame the pointer has been resting at `pos` for, handing back what
    /// it painted — four frames, because a tooltip is decided from the hover a
    /// previous frame's widget rect resolved. `tests/tile_scale_switch.rs`
    /// measured where the text first appears.
    fn hover_shapes(&mut self, pos: egui::Pos2) -> Vec<egui::epaint::ClippedShape> {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            self.ctx
                .style_mut_of(theme, |style| style.interaction.tooltip_delay = 0.0);
        }
        let moved = || vec![egui::Event::PointerMoved(pos)];
        self.run(vec![moved(), moved(), moved()]);
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            events: moved(),
            ..Default::default()
        };
        self.ctx.run_ui(raw, |ui| self.app.draw(ui)).shapes
    }

    fn doc(&self) -> &ChartDoc {
        self.app.chart_doc()
    }

    /// The layout switch the last frame drew, panicking when the band drew
    /// none — a dropped control fails with a sentence rather than with an
    /// `unwrap`.
    fn switch(&self) -> LayoutSwitchDrawn {
        self.doc()
            .grid_layout_switch
            .clone()
            .expect("the grid pane's header band drew a layout switch")
    }

    /// Throw the switch to `state` by a click on the rect the frame recorded
    /// for it, and let the page settle.
    fn throw(&mut self, state: GridLayout) {
        let at = self
            .switch()
            .states
            .iter()
            .find(|(drawn, _)| *drawn == state)
            .unwrap_or_else(|| panic!("the switch offers no {state:?} state"))
            .1
            .center();
        self.click(at);
        self.settle();
    }

    /// The rows the transposed layout drew, in the order they were drawn.
    fn rows(&self) -> Vec<ColumnBandDrawn> {
        self.doc().transposed_rows.clone()
    }

    /// The row whose numbers name `column`, with the picture beside it — the
    /// pairing the whole layout is, read back off the two records that make
    /// it.
    fn row(&self, column: &str) -> (ColumnBandDrawn, egui::Rect) {
        let rows = self.rows();
        let cell = rows
            .iter()
            .find(|row| row.name == column)
            .unwrap_or_else(|| {
                let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
                panic!("no transposed row for {column:?}; the pane drew {names:?}")
            })
            .clone();
        let plot = self.app.composed_plot_rects()[cell.column];
        (cell, plot)
    }
}

// ---------------------------------------------------------------------------
// What the tiles actually drew — read off the live session through the plot's
// own marks, not off a second derivation of the same arithmetic.
// ---------------------------------------------------------------------------

/// The flat mark indices plot `plot` owns, as the engine numbers them.
///
/// The composition places plots in the spec's own depth-first order and each
/// [`PlotHandle::marks`] is that plot's marks in declaration order, so a
/// plot's first mark is the running sum of the mark counts before it. The
/// derivation is self-checking: [`plot_bins`] refuses a mark whose rows do not
/// carry the column it was asked about, so a numbering that drifted fails by
/// name rather than by reading a neighbour's bins.
fn plot_marks(doc: &ChartDoc, plot: usize) -> Vec<usize> {
    let before: usize = doc.composed.plots[..plot]
        .iter()
        .map(|p| p.marks.len())
        .sum();
    (before..before + doc.composed.plots[plot].marks.len()).collect()
}

/// **The bins plot `plot` draws of `column`**, as `(bin start, count)` pairs
/// in the order the engine returned them, taken from the plot's own marks.
///
/// This is what "the picture" means for a binned mark: the rows the session
/// returns after the composition has run, so a bin read here is a bar the tile
/// has to draw. Counting what the composer wrote down instead would be asking
/// the code under test to confirm its own intention.
///
/// Panics naming the schemas when no mark of that plot bins the column, which
/// is what makes [`plot_marks`]'s numbering an assertion rather than a hope.
fn plot_bins(doc: &mut ChartDoc, plot: usize, column: &str) -> Vec<(f64, f64)> {
    use arrow::array::Float64Array;
    use arrow::compute::cast;
    use arrow::datatypes::DataType;
    let marks = plot_marks(doc, plot);
    let mut out = Vec::new();
    let mut schemas = Vec::new();
    for mark in marks {
        let Some(coordinator) = doc.live_coordinator() else {
            break;
        };
        let Ok(batches) = coordinator.chart_rows(mark) else {
            continue;
        };
        for batch in &batches {
            let names: Vec<String> = batch
                .schema()
                .fields()
                .iter()
                .map(|f| f.name().clone())
                .collect();
            schemas.push(format!("{mark}: {names:?}"));
            let (Ok(bin), Ok(count)) = (
                batch.schema().index_of(column),
                batch.schema().index_of("__bf_count"),
            ) else {
                continue;
            };
            let bins = cast(batch.column(bin), &DataType::Float64).expect("numeric bins");
            let counts = cast(batch.column(count), &DataType::Float64).expect("numeric counts");
            let bins = bins
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("f64 bins");
            let counts = counts
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("f64 counts");
            for i in 0..bins.len() {
                out.push((bins.value(i), counts.value(i)));
            }
        }
    }
    assert!(
        !out.is_empty(),
        "plot {plot} drew no counted bins of {column:?}; its marks read {schemas:?}"
    );
    out
}

// ---------------------------------------------------------------------------
// AC1 — the rows are the tiles, in tile order, drawing their own columns.
// ---------------------------------------------------------------------------

/// **Thrown, the grid pane draws one row per tile column in tile order, and a
/// row's histogram is that column's own tile.**
///
/// Three claims, and the third is the one the layout exists for:
///
/// 1. the rows the pane drew are the seven tile columns, in the order the
///    column beside the hero stacks them, top to bottom;
/// 2. each row's numbers stand beside that row's own picture — the cell ends
///    where the plot begins, at the plot's own top and bottom;
/// 3. the picture beside `population`'s numbers bins `population`, at exactly
///    the bins the tile drew before the switch was thrown. Same edges, same
///    counts: the layout moved the plot, it did not re-query it.
#[test]
fn the_transposed_grid_draws_one_row_per_tile_column_in_tile_order() {
    let mut live = Live::open();

    // The column's own tile, before anything is thrown: which plot draws
    // `population`, and the bins it draws of it.
    let tile = live
        .doc()
        .tile_columns()
        .iter()
        .position(|c| c.column == "population")
        .expect("the fixture's tiles include population");
    let before = plot_bins(live.app.chart_doc_mut(), tile, "population");
    assert!(
        before.len() > 1,
        "population's tile drew {} bins, too few to tell one column's picture \
         from another's",
        before.len()
    );

    live.throw(GridLayout::Columns);

    let rows = live.rows();
    let drawn: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(
        drawn,
        TILE_ORDER.to_vec(),
        "the transposed pane drew {drawn:?} where the column stacks {TILE_ORDER:?}"
    );
    let tops: Vec<f32> = rows.iter().map(|row| row.cell.top()).collect();
    assert!(
        tops.windows(2).all(|pair| pair[0] < pair[1]),
        "the rows were drawn at {tops:?}, which is not top to bottom in tile order"
    );

    // Each row's numbers end where its own picture begins.
    for row in &rows {
        let plot = live.app.composed_plot_rects()[row.column];
        assert!(
            (row.cell.right() - plot.left()).abs() < 1.0,
            "{}'s numbers end at {} and its picture begins at {}",
            row.name,
            row.cell.right(),
            plot.left()
        );
        assert!(
            (row.cell.top() - plot.top()).abs() < 1.0
                && (row.cell.bottom() - plot.bottom()).abs() < 1.0,
            "{}'s numbers stand at {:?} beside a picture at {:?}",
            row.name,
            row.cell.y_range(),
            plot.y_range()
        );
    }

    // …and the picture beside `population`'s numbers is `population`'s tile.
    let (cell, _) = live.row("population");
    let column = live.doc().composed.plots[cell.column].x_column.clone();
    assert_eq!(
        column.as_deref(),
        Some("population"),
        "the picture beside population's numbers bins {column:?}"
    );
    let after = plot_bins(live.app.chart_doc_mut(), cell.column, "population");
    assert_eq!(
        after, before,
        "the row's histogram drew different bins from the tile it was laid out \
         from: {} bins against {}",
        after.len(),
        before.len()
    );
}

/// **Every transposed row states the numbers the full band states.**
///
/// The record is the band's own — the same painter at
/// `GridDensity::Row` — so this reads the fields that painter fills: the
/// glyph and the name, the leaf and the storage type, the validity band's
/// three segments and its count, the range, and the five statistics. And no
/// bars: the picture is the tile beside the cell, and a second distribution in
/// the cell is the duplication the layout exists to end.
#[test]
fn every_transposed_row_states_the_numbers_the_band_states() {
    let mut live = Live::open();
    live.throw(GridLayout::Columns);

    let rows = live.rows();
    assert_eq!(
        rows.len(),
        TILE_ORDER.len(),
        "the pane drew {} rows, so the loop below would state nothing",
        rows.len()
    );
    for row in rows {
        let name = row.name.clone();
        assert_eq!(row.glyph, "#", "{name} drew glyph {:?}", row.glyph);
        assert!(row.leaf.is_some(), "{name} stated no finetype leaf");
        assert!(row.storage.is_some(), "{name} stated no storage type");
        assert!(row.range.is_some(), "{name} stated no range");
        assert_eq!(
            row.valid + row.invalid + row.missing,
            240,
            "{name}'s validity counts do not sum to the sample's 240 rows"
        );
        assert!(
            row.count_text.ends_with("missing"),
            "{name}'s validity count reads {:?}",
            row.count_text
        );
        let stats = row.stats.as_ref().unwrap_or_else(|| {
            panic!("{name} stated no statistics beside its picture");
        });
        for text in [
            &stats.mean_text,
            &stats.median_text,
            &stats.sd_text,
            &stats.nulls_text,
            &stats.distinct_text,
        ] {
            assert!(!text.is_empty(), "{name} drew an empty statistic");
        }
        assert!(
            row.bars.is_empty() && row.rug.is_none(),
            "{name} drew a distribution of its own beside the tile that is \
             already its distribution"
        );
    }
}

// ---------------------------------------------------------------------------
// AC3 — the control: two words, hover text, a rect, and both ways back.
// ---------------------------------------------------------------------------

/// **The switch's two states are words on the band, its rect is recorded, and
/// two scripted clicks take the pane out to the columns and back.**
///
/// The words are read off the shapes a frame painted, not off the record, so a
/// control that recorded a state it never drew fails here. The layout is read
/// back after each click off what the pane drew: the rows record for the
/// transposed arrangement and the columns pane's own presence for the other.
#[test]
fn the_layout_switch_reads_its_two_states_and_takes_the_pane_both_ways() {
    let mut live = Live::open();

    // The fixture opens on its rows.
    assert_eq!(
        live.app.grid_layout(),
        GridLayout::Rows,
        "the file did not open on its rows"
    );
    assert!(
        live.rows().is_empty(),
        "a window that opened on its rows drew transposed rows"
    );

    let switch = live.switch();
    assert_eq!(switch.active, GridLayout::Rows);
    let words: Vec<&str> = switch
        .states
        .iter()
        .map(|(state, _)| state.word())
        .collect();
    assert_eq!(words, vec!["rows", "columns"]);
    for (state, rect) in &switch.states {
        assert!(
            switch.rect.contains_rect(*rect),
            "the {:?} state was drawn at {rect:?}, outside the control's own \
             box {:?}",
            state,
            switch.rect
        );
    }
    let band = live
        .app
        .canvas_panes()
        .pane("rows")
        .expect("the rows pane drew")
        .header;
    assert!(
        band.contains_rect(switch.rect),
        "the control was drawn at {:?}, off the grid pane's header band {band:?}",
        switch.rect
    );

    // The words a stranger reads, off the frame.
    let painted = live.hover_shapes(switch.rect.center());
    let texts: Vec<String> = shape_texts(&painted);
    for word in ["rows", "columns"] {
        assert!(
            texts.iter().any(|t| t == word),
            "the band painted no {word:?}; it painted {texts:?}"
        );
    }
    assert!(
        texts.iter().any(|t| *t == switch.hover),
        "resting on the control painted no hover text {:?}; the frame painted \
         {texts:?}",
        switch.hover
    );

    // Out to the columns…
    live.throw(GridLayout::Columns);
    assert_eq!(live.app.grid_layout(), GridLayout::Columns);
    assert_eq!(live.switch().active, GridLayout::Columns);
    assert_eq!(live.rows().len(), TILE_ORDER.len());

    // …and back, by a second click on the rect this frame recorded.
    live.throw(GridLayout::Rows);
    assert_eq!(live.app.grid_layout(), GridLayout::Rows);
    assert_eq!(live.switch().active, GridLayout::Rows);
    assert!(
        live.rows().is_empty(),
        "the pane came back to its rows still drawing transposed rows"
    );
}

/// Every text a frame painted, in paint order.
fn shape_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
            egui::Shape::Vec(shapes) => {
                for s in shapes {
                    walk(s, out);
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

// ---------------------------------------------------------------------------
// AC5 — the columns pane goes, the map takes its width, and both come back.
// ---------------------------------------------------------------------------

/// **While the grid is transposed the columns pane is not drawn and the map
/// spans the width it left; switched back, every pane rect is the one it was.**
#[test]
fn the_transposed_canvas_drops_the_columns_pane_and_gives_the_map_its_width() {
    let mut live = Live::open();
    let before: Vec<(String, egui::Rect)> = live
        .app
        .canvas_panes()
        .panes
        .iter()
        .map(|pane| (pane.name.to_string(), pane.rect))
        .collect();
    let names: Vec<&str> = before.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, vec!["map", "rows", "columns"]);
    let narrow_map = before[0].1;
    let columns = before[2].1;

    live.throw(GridLayout::Columns);

    let panes = live.app.canvas_panes();
    let drawn: Vec<&str> = panes.panes.iter().map(|pane| pane.name).collect();
    assert_eq!(
        drawn,
        vec!["map", "rows"],
        "the transposed canvas drew {drawn:?}"
    );
    let wide_map = panes.pane("map").expect("the map pane drew").rect;
    let wide_rows = panes.pane("rows").expect("the grid pane drew").rect;
    assert!(
        (wide_map.right() - columns.right()).abs() < 0.5,
        "the map ends at {} where the columns pane ended at {}",
        wide_map.right(),
        columns.right()
    );
    assert!(
        wide_map.width() > narrow_map.width(),
        "the map kept its narrow width {} when the pane beside it went",
        narrow_map.width()
    );
    assert!(
        (wide_rows.right() - wide_map.right()).abs() < 0.5
            && (wide_rows.left() - wide_map.left()).abs() < 0.5,
        "the grid pane at {:?} does not stand under the map at {:?}",
        wide_rows,
        wide_map
    );
    assert!(
        (wide_map.bottom() - narrow_map.bottom()).abs() < 0.5,
        "transposing moved the edge between the map and the grid from {} to {}",
        narrow_map.bottom(),
        wide_map.bottom()
    );

    live.throw(GridLayout::Rows);

    let after: Vec<(String, egui::Rect)> = live
        .app
        .canvas_panes()
        .panes
        .iter()
        .map(|pane| (pane.name.to_string(), pane.rect))
        .collect();
    assert_eq!(
        after, before,
        "the panes came back at {after:?} from {before:?}"
    );
}

// ---------------------------------------------------------------------------
// The floor, and the scroll that buys it.
// ---------------------------------------------------------------------------

/// **A row is tall enough for its bars and for its numbers, and the pane
/// scrolls rather than compressing past that.**
///
/// The constant is held to the two measurements it was chosen from rather than
/// to itself: the tile floor the column already keeps, and the height the row's
/// own numbers stack to. A face change that grows the summaries reddens here
/// instead of quietly overprinting a row.
#[test]
fn a_transposed_row_clears_its_own_floor() {
    use brightfield_shell::column_header::{column_header_frame, GridDensity};
    use brightfield_shell::dashboard::{MIN_COLUMN_TILE_HEIGHT, MIN_ROW_HEIGHT};

    let numbers = column_header_frame(GridDensity::Row, Mode::Light).extent();
    assert!(
        MIN_ROW_HEIGHT >= numbers,
        "a row's floor is {MIN_ROW_HEIGHT} points and its numbers stack to \
         {numbers}"
    );
    assert!(
        MIN_ROW_HEIGHT >= MIN_COLUMN_TILE_HEIGHT,
        "a row's floor is {MIN_ROW_HEIGHT} points and a tile's is \
         {MIN_COLUMN_TILE_HEIGHT}"
    );

    // A window short enough that seven rows at that floor do not fit the pane.
    let mut live = Live::open_at(Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(1440.0, 900.0),
    )));
    live.throw(GridLayout::Columns);

    let rows = live.rows();
    assert_eq!(rows.len(), TILE_ORDER.len());
    for row in &rows {
        let plot = live.app.composed_plot_rects()[row.column];
        assert!(
            plot.height() >= MIN_ROW_HEIGHT - 0.5,
            "{}'s row was drawn {} points tall, under the {MIN_ROW_HEIGHT}-point \
             floor",
            row.name,
            plot.height()
        );
    }
    let pane = live
        .app
        .canvas_panes()
        .pane("rows")
        .expect("the grid pane drew")
        .body;
    let stack = rows.len() as f32 * MIN_ROW_HEIGHT;
    assert!(
        stack > pane.height(),
        "the rows stack to {stack} points in a pane {} tall, so this window \
         makes no claim about scrolling",
        pane.height()
    );
}
