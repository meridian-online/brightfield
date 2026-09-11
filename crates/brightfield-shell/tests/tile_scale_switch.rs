//! **Each histogram tile on the generated dashboard carries its own scale
//! switch**, and throwing it re-bins that tile's column in the new space.
//!
//! Every assertion here reads a **laid-out frame**. The control's geometry
//! comes off [`ChartDoc::scale_switches`], which the chart pane writes as it
//! draws, and the tile boxes it is held inside come off
//! [`MeridianApp::composed_plot_rects`] — two surfaces, so a control drawn
//! somewhere the tile is not fails here rather than needing an eye on a
//! screenshot. The words come off the shapes the frame painted.
//!
//! # What is covered and what is not
//!
//! Covered: which tiles offer a switch and which do not, where the control
//! sits, what it says, what a click writes into the canonical spec, what that
//! does to the tile's bins and ticks, what it leaves alone on the other tiles,
//! and that a brush swept afterwards narrows a log tile without moving its
//! scale. Not covered here: the pixels. `tests/dashboard_baseline.rs` is that
//! half — the two re-photographed baselines carry the control at rest.

use brightfield_protocol::layout::Flow;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::layout::ScaleType;

/// The committed table these windows are opened over — the same fixture
/// `tests/navigator_spine.rs` and `tests/dashboard_baseline.rs` use.
fn housing() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv")
}

/// A boot over [`housing`], as the front door's picker builds it.
fn housing_boot() -> Boot {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
}

/// A window that keeps its own `egui::Context` for its whole life, because a
/// click is resolved against the widget id a *previous* frame registered —
/// `tests/navigator_spine.rs`'s harness, over this file's own assertions.
struct Live {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Live {
    /// A window over `boot` at the size that boot asks for — the dashboard
    /// baseline's window.
    fn open(boot: Boot) -> Self {
        let size = boot.window_size();
        let ctx = egui::Context::default();
        Self {
            app: MeridianApp::headless(boot, Mode::Light),
            ctx,
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1)),
        }
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

    /// One more frame with no events, handing back every shape it painted.
    fn shapes(&mut self) -> Vec<egui::epaint::ClippedShape> {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            ..Default::default()
        };
        self.ctx.run_ui(raw, |ui| self.app.draw(ui)).shapes
    }

    /// The frame the pointer has been resting at `pos` for, handing back what
    /// it painted.
    ///
    /// Four frames, not one. A tooltip is decided from the hover a *previous*
    /// frame's widget rect resolved, and egui's own delay is measured off the
    /// input clock a scripted frame advances by one predicted step at a time —
    /// three frames of rest is where the text first appears, which a probe
    /// over a bare `on_hover_text` in an empty context reproduces outside this
    /// window. Two frames is not enough and this is the test that says so.
    fn hover_shapes(&mut self, pos: egui::Pos2) -> Vec<egui::epaint::ClippedShape> {
        // The delay is zeroed HERE and not at construction. The window
        // installs the design system's whole `Style` on its first draw —
        // `meridian_egui`'s `set_style_of` replaces the struct rather than
        // editing it — so a delay set before that frame is already gone when
        // the first pointer comes to rest.
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

    /// The switch the tile for `column` drew, panicking with the list of what
    /// was drawn instead — a dropped control fails with a sentence.
    fn switch(&self, column: &str) -> brightfield_shell::app::ScaleSwitchDrawn {
        let drawn = self.doc().scale_switches.clone();
        drawn
            .iter()
            .find(|s| s.column == column)
            .cloned()
            .unwrap_or_else(|| {
                let names: Vec<&str> = drawn.iter().map(|s| s.column.as_str()).collect();
                panic!("no scale switch for {column:?}; the page drew {names:?}")
            })
    }

    /// Press and release the primary button over `pos`, as three frames: egui
    /// resolves a click against the widget id the previous frame registered,
    /// and the canvas reads its own press edge across frames.
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
}

impl Live {
    /// A point `fraction` of the way across plot `plot`'s box, at its middle
    /// height — clear of the switch at its head, which sits in the top corner.
    ///
    /// Read off [`MeridianApp::composed_plot_rects`], which resolves the two
    /// origins the page is painted at, so a tile in the scrolled column is
    /// aimed at where it is on screen.
    fn at(&self, plot: usize, fraction: f32) -> egui::Pos2 {
        let rect = self.app.composed_plot_rects()[plot];
        egui::pos2(rect.left() + rect.width() * fraction, rect.center().y)
    }

    /// Sweep a brush across plot `plot`, from one fraction of its width to
    /// another, and release — the press, the move and the release are each a
    /// frame, because the canvas reads its own press edge across frames.
    fn brush(&mut self, plot: usize, from: f32, to: f32) {
        let start = self.at(plot, from);
        let end = self.at(plot, to);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.run(vec![
            vec![egui::Event::PointerMoved(start)],
            vec![egui::Event::PointerMoved(start), button(start, true)],
            vec![egui::Event::PointerMoved(end)],
            vec![egui::Event::PointerMoved(end), button(end, false)],
            Vec::new(),
            Vec::new(),
        ]);
    }

    /// Throw `column`'s switch to `kind` and let the page settle.
    fn switch_to(&mut self, column: &str, kind: ScaleType) {
        let at = self
            .switch(column)
            .states
            .iter()
            .find(|(state, _)| *state == kind)
            .unwrap_or_else(|| panic!("{column}'s switch offers no {kind:?}"))
            .1
            .center();
        self.click(at);
    }
}

/// Every text the frame painted, with its box and the font its first section
/// was set in — `tests/navigator_spine.rs`'s reader, so a label drawn in the
/// wrong face is a fact this file can state.
fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Rect, egui::FontId)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect, egui::FontId)>) {
        match shape {
            egui::Shape::Text(text) => {
                let font = text
                    .galley
                    .job
                    .sections
                    .first()
                    .map(|section| section.format.font_id.clone())
                    .unwrap_or_else(egui::FontId::default);
                out.push((
                    text.galley.text().to_string(),
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                    font,
                ));
            }
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

/// The columns the generator gives a histogram tile on this fixture, in the
/// order the composition places their plots. The hero point map is not among
/// them: it is the joint tile over the two coordinate columns.
const HISTOGRAM_COLUMNS: [&str; 7] = [
    "median_income",
    "house_age",
    "avg_rooms",
    "avg_bedrooms",
    "population",
    "avg_occupancy",
    "median_house_value",
];

// ---------------------------------------------------------------------------
// AC3 — the control is on every histogram tile, in its own box, and says so.
// ---------------------------------------------------------------------------

/// Seven tiles, seven switches, each **inside the box its own tile occupies**.
///
/// The containment is the assertion: the control's rect comes off the chart
/// pane's own record and the tile's off [`MeridianApp::composed_plot_rects`],
/// which resolves a plot's two possible origins independently. A control
/// placed from the wrong origin — the map pane's, on a tile the column pane
/// scrolled — lands outside and fails here.
#[test]
fn every_histogram_tile_carries_a_scale_switch_inside_its_own_box() {
    let mut live = Live::open(housing_boot());
    live.settle();

    let drawn: Vec<String> = live
        .doc()
        .scale_switches
        .iter()
        .map(|s| s.column.clone())
        .collect();
    assert_eq!(
        drawn, HISTOGRAM_COLUMNS,
        "one switch per histogram tile, in plot order"
    );

    let tiles = live.app.composed_plot_rects();
    for switch in &live.doc().scale_switches {
        let tile = tiles[switch.plot];
        assert!(
            tile.contains_rect(switch.rect),
            "{}'s switch at {:?} is not inside its tile at {tile:?}",
            switch.column,
            switch.rect
        );
        for (state, seg) in &switch.states {
            assert!(
                switch.rect.contains_rect(*seg),
                "{}'s {state:?} segment escapes the control's own box",
                switch.column
            );
        }
    }
}

/// The hero draws none, and neither does anything outside the tiles.
///
/// Held as a rect test rather than as a count so it reddens on a control
/// drawn over the hero from a *different* code path as loudly as on the
/// histogram rule growing an eighth entry.
#[test]
fn the_hero_point_map_draws_no_scale_switch() {
    let mut live = Live::open(housing_boot());
    live.settle();

    let hero = live
        .doc()
        .tile_columns()
        .first()
        .expect("the dashboard named a first tile")
        .clone();
    assert_eq!(
        hero.tile.as_deref(),
        Some("point-map"),
        "the hero on this fixture is the coordinate pair's point map"
    );
    let tiles = live.app.composed_plot_rects();
    let hero_rect = tiles[0];
    for switch in &live.doc().scale_switches {
        assert_ne!(switch.plot, 0, "the hero plot drew a switch");
        assert!(
            !hero_rect.intersects(switch.rect),
            "{}'s switch at {:?} reaches into the hero's box at {hero_rect:?}",
            switch.column,
            switch.rect
        );
    }
}

/// Three states, named as the spec names them, drawn in the chart-label face
/// — one step below UI body, which is the small face a tile's own ink is set
/// in.
#[test]
fn the_switch_offers_linear_log_and_symlog_in_the_small_face() {
    let mut live = Live::open(housing_boot());
    live.settle();

    for switch in &live.doc().scale_switches {
        let offered: Vec<ScaleType> = switch.states.iter().map(|(s, _)| *s).collect();
        assert_eq!(
            offered,
            vec![ScaleType::Linear, ScaleType::Log, ScaleType::Symlog],
            "{} offers the three continuous transforms in order",
            switch.column
        );
    }

    let shapes = live.shapes();
    let painted = texts(&shapes);
    let population = live.switch("population");
    for (state, seg) in &population.states {
        let word = state.wire_name();
        let found = painted
            .iter()
            .find(|(text, rect, _)| text == word && seg.contains_rect(*rect))
            .unwrap_or_else(|| {
                panic!("the frame painted no {word:?} inside population's {word} segment")
            });
        assert!(
            (found.2.size - meridian_design::typography::CHART_LABEL_SIZE).abs() < f32::EPSILON,
            "{word} is set at {} and not the chart-label size",
            found.2.size
        );
    }
}

/// The control names the tile it acts on, in its hover text — seven controls,
/// seven different strings, which is what makes the readback able to tell a
/// control aimed at the wrong plot from one aimed at the right one.
#[test]
fn each_switch_names_its_own_column_in_its_hover_text() {
    let mut live = Live::open(housing_boot());
    live.settle();

    let said: Vec<String> = live
        .doc()
        .scale_switches
        .iter()
        .map(|s| s.hover.clone())
        .collect();
    let want: Vec<String> = HISTOGRAM_COLUMNS
        .iter()
        .map(|c| format!("scale: {c}"))
        .collect();
    assert_eq!(said, want);

    // And the words the tooltip actually paints when the pointer rests on one
    // — the record and the paint are one `String`, and this is the half that
    // says the paint happens.
    let at = live.switch("population").rect.center();
    let shapes = live.hover_shapes(at);
    let painted: Vec<String> = texts(&shapes).into_iter().map(|(t, _, _)| t).collect();
    assert!(
        painted.iter().any(|t| t == "scale: population"),
        "resting on population's switch painted no hover text; it painted {painted:?}"
    );
}

/// **An authored spec draws no switch even when it draws a binned histogram.**
///
/// The fixture is `examples/rect-bin-count.yaml` — one `rectY` over a binned
/// column, the same device a generated tile emits — so the assertion is about
/// where the offer comes from rather than about a page with no binned axis to
/// offer it on. The switch is the generated dashboard's: it exists because a
/// generated
/// tile has no author standing by to rewrite its spec, and a spec somebody
/// wrote has one. A build that decided switchability from the marks on the
/// page rather than from the generator's tile list draws one here and fails.
#[test]
fn an_authored_binned_histogram_draws_no_scale_switch() {
    let spec = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/rect-bin-count.yaml");
    let spec = spec.to_str().expect("utf-8 example path");
    let boot = Boot::open(spec, Flow::Vertical, None).expect("the example opens");
    let mut live = Live::open(boot);
    live.settle();
    assert!(
        live.doc().tile_columns().is_empty(),
        "an authored spec names no tiles"
    );
    assert!(
        live.doc()
            .composed
            .plots
            .iter()
            .any(|p| p.marks.contains(&brightfield_spec::vocab::MarkKind::RectY)),
        "the fixture draws no binned rect, so there is no offer to withhold"
    );
    assert!(
        live.doc().scale_switches.is_empty(),
        "it drew {:?}",
        live.doc().scale_switches
    );
}

// ---------------------------------------------------------------------------
// What the tiles actually drew — read off the live session, not off a second
// derivation of the same arithmetic.
// ---------------------------------------------------------------------------

/// One mark's bins, as `(bin start, count)` pairs in the order the engine
/// returned them.
///
/// This is what "the picture" means for a binned mark: the rows the session
/// returns after the composition has run, so a bin count read here is the
/// number of bars the tile has to draw. Counting anything the composer wrote
/// down would be asking the code under test to confirm its own intention.
fn mark_bins(doc: &mut ChartDoc, mark: usize, column: &str) -> Vec<(f64, f64)> {
    use arrow::array::Float64Array;
    use arrow::compute::cast;
    use arrow::datatypes::DataType;
    let batches = doc
        .live_coordinator()
        .expect("a live document")
        .chart_rows(mark)
        .expect("the mark queries");
    let mut out = Vec::new();
    for batch in &batches {
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
    out
}

/// Which flat mark indices bin `column`, in index order.
///
/// A histogram tile emits two marks over one column — the unfiltered ghost and
/// the layer the selection narrows — so this answers with both, and the pair
/// is what makes "the filtered layer moved and the ghost did not" a thing a
/// test can say. Panics naming every mark's schema when none binds the column,
/// so a renamed bin output fails with a list rather than with an empty vector
/// that reads like a passing zero.
fn marks_binning(doc: &mut ChartDoc, column: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut schemas = Vec::new();
    for mark in 0..64 {
        let Some(coordinator) = doc.live_coordinator() else {
            break;
        };
        let Ok(batches) = coordinator.chart_rows(mark) else {
            break;
        };
        let Some(batch) = batches.first() else {
            continue;
        };
        let names: Vec<String> = batch
            .schema()
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect();
        if names.iter().any(|n| n == column) && names.iter().any(|n| n == "__bf_count") {
            found.push(mark);
        }
        schemas.push(format!("{mark}: {names:?}"));
    }
    assert!(
        !found.is_empty(),
        "no mark binned {column:?} into counted bins; the marks read {schemas:?}"
    );
    found
}

/// The x-axis tick labels the plot's own scale produces, at the density the
/// axis renderer asks for.
fn x_tick_labels(doc: &ChartDoc, plot: usize) -> Vec<String> {
    let scale = doc.composed.plots[plot]
        .scales
        .get(brightfield_render::channel::Channel::X)
        .expect("the plot has an x scale")
        .clone();
    brightfield_render::axis::compute_ticks(&scale, 10)
        .into_iter()
        .map(|t| t.label)
        .collect()
}

/// What a plot draws against, as one comparable value: the scale on each
/// positional axis and the box it was placed in.
fn plot_frame(doc: &ChartDoc, plot: usize) -> String {
    use brightfield_render::channel::Channel;
    let handle = &doc.composed.plots[plot];
    // Rendered rather than compared field by field: `Scale` carries a domain,
    // a pixel range and — on a band — its categories, and the debug form
    // holds each of the three, so a domain that widened by a pixel shows up
    // here. `Scale` derives no `PartialEq` to compare instead.
    format!(
        "x={:?} y={:?} at={:?}",
        handle.scales.get(Channel::X),
        handle.scales.get(Channel::Y),
        handle.rect
    )
}

// ---------------------------------------------------------------------------
// AC4 — a click rewrites one key, and the page re-queries around it.
// ---------------------------------------------------------------------------

/// Clicking `log` on `population` writes `xScale: log` into that plot's node
/// in the canonical spec and **changes nothing else in it**.
///
/// Held by walking every plot node in the spec before and after and comparing
/// them: the items of all eight, and the attribute maps of the seven the click
/// did not name. An edit that wrote to the focused plot *and* somewhere else —
/// or to the wrong plot — fails on a named path rather than on a byte count.
#[test]
fn a_click_writes_one_key_into_the_canonical_spec() {
    use brightfield_spec::layout::collect_plot_nodes;

    let mut live = Live::open(housing_boot());
    live.settle();
    let before = live
        .doc()
        .live_dashboard()
        .expect("a live session")
        .spec()
        .clone();

    let switch = live.switch("population");
    let log = switch
        .states
        .iter()
        .find(|(state, _)| *state == ScaleType::Log)
        .expect("the switch offers log")
        .1;
    live.click(log.center());

    let after = live
        .doc()
        .live_dashboard()
        .expect("a live session")
        .spec()
        .clone();
    let was = collect_plot_nodes(&before);
    let now = collect_plot_nodes(&after);
    assert_eq!(
        was.len(),
        now.len(),
        "the edit is count-stable: {} plots before, {} after",
        was.len(),
        now.len()
    );

    let target = &live.doc().composed.plots[switch.plot].path;
    let mut touched = Vec::new();
    for ((path_a, plot_a), (path_b, plot_b)) in was.iter().zip(now.iter()) {
        assert_eq!(path_a, path_b, "the plot order moved");
        assert_eq!(plot_a.items, plot_b.items, "{path_a}'s marks moved");
        if plot_a.attributes == plot_b.attributes {
            continue;
        }
        touched.push(path_a.clone());
        assert_eq!(
            path_a, target,
            "the edit landed on {path_a} and not on the tile that was clicked"
        );
        assert_eq!(
            plot_b.attributes.len(),
            plot_a.attributes.len() + 1,
            "one attribute added and nothing else: {:?} -> {:?}",
            plot_a.attributes,
            plot_b.attributes
        );
        assert_eq!(
            plot_b.attributes.get("xScale"),
            Some(&brightfield_spec::ast::SpecValue::String("log".to_string()))
        );
    }
    assert_eq!(
        touched,
        vec![target.clone()],
        "exactly one plot node changed"
    );
}

/// The tile re-queries: its bins are cut in log space, more of them are
/// occupied than the linear cut left, its x axis is ticked in decades — and
/// the other six tiles' frames and rows are where they were.
///
/// The bin count is read off the **session**, one row per occupied bin, so it
/// is the number of bars the tile has to draw rather than a number the
/// composer wrote down. The other six are read as their scales, their placed
/// boxes and their own rows: an edit that re-cut the whole page would move at
/// least one of the three.
#[test]
fn the_log_tile_re_bins_and_the_other_six_stand_still() {
    let mut live = Live::open(housing_boot());
    live.settle();

    let switch = live.switch("population");
    let others: Vec<usize> = live
        .doc()
        .scale_switches
        .iter()
        .map(|s| s.plot)
        .filter(|p| *p != switch.plot)
        .collect();
    assert_eq!(others.len(), 6, "six tiles besides population");

    let marks = marks_binning(live.app.chart_doc_mut(), "population");
    let linear_bins = mark_bins(live.app.chart_doc_mut(), marks[0], "population");
    let frames_before: Vec<_> = others.iter().map(|p| plot_frame(live.doc(), *p)).collect();
    let rows_before: Vec<Vec<(f64, f64)>> = live
        .doc()
        .scale_switches
        .iter()
        .filter(|s| s.plot != switch.plot)
        .map(|s| (s.plot, s.column.clone()))
        .collect::<Vec<_>>()
        .into_iter()
        .map(|(_, column)| {
            let mark = marks_binning(live.app.chart_doc_mut(), &column)[0];
            mark_bins(live.app.chart_doc_mut(), mark, &column)
        })
        .collect();
    let page_before = (live.doc().composed.width, live.doc().composed.height);
    assert!(
        matches!(
            live.doc().composed.plots[switch.plot]
                .scales
                .get(brightfield_render::channel::Channel::X),
            Some(brightfield_render::scale::Scale::Linear { .. })
        ),
        "population starts linear"
    );

    let log = switch
        .states
        .iter()
        .find(|(state, _)| *state == ScaleType::Log)
        .expect("the switch offers log")
        .1;
    live.click(log.center());

    // The picture is now a log picture, and the switch says so.
    assert!(
        matches!(
            live.doc().composed.plots[switch.plot]
                .scales
                .get(brightfield_render::channel::Channel::X),
            Some(brightfield_render::scale::Scale::Log { .. })
        ),
        "population's x scale after the click: {:?}",
        live.doc().composed.plots[switch.plot]
            .scales
            .get(brightfield_render::channel::Channel::X)
    );
    assert_eq!(live.switch("population").active, ScaleType::Log);

    // Ticked in decades: each label is ten times the last, which `nice_step`'s
    // 1/2/5 decimal ladder cannot produce.
    let labels = x_tick_labels(live.doc(), switch.plot);
    let values: Vec<f64> = labels
        .iter()
        .filter_map(|l| l.parse::<f64>().ok())
        .collect();
    assert!(
        values.len() >= 3,
        "a log axis over population is ticked at least three times; it drew {labels:?}"
    );
    for pair in values.windows(2) {
        let ratio = pair[1] / pair[0];
        assert!(
            (ratio - 10.0).abs() < 0.01,
            "the ticks step by {ratio} and not by a decade: {labels:?}"
        );
    }

    // More bins carry rows than the linear cut left — the whole point of the
    // switch on a long-tailed column.
    let marks = marks_binning(live.app.chart_doc_mut(), "population");
    let log_bins = mark_bins(live.app.chart_doc_mut(), marks[0], "population");
    assert!(
        log_bins.len() > linear_bins.len(),
        "log occupies {} bins and linear occupied {}",
        log_bins.len(),
        linear_bins.len()
    );
    let total = |bins: &[(f64, f64)]| bins.iter().map(|(_, c)| *c).sum::<f64>();
    assert!(
        (total(&log_bins) - total(&linear_bins)).abs() < f64::EPSILON,
        "the same rows are drawn either way: {} against {}",
        total(&log_bins),
        total(&linear_bins)
    );

    // And nothing else on the page moved.
    assert_eq!(
        (live.doc().composed.width, live.doc().composed.height),
        page_before,
        "the page was re-laid out"
    );
    for (i, plot) in others.iter().enumerate() {
        assert_eq!(
            plot_frame(live.doc(), *plot),
            frames_before[i],
            "plot {plot}'s scales or box moved"
        );
    }
    let columns: Vec<String> = live
        .doc()
        .scale_switches
        .iter()
        .filter(|s| s.plot != switch.plot)
        .map(|s| s.column.clone())
        .collect();
    for (i, column) in columns.iter().enumerate() {
        let mark = marks_binning(live.app.chart_doc_mut(), column)[0];
        assert_eq!(
            mark_bins(live.app.chart_doc_mut(), mark, column),
            rows_before[i],
            "{column}'s rows moved"
        );
    }
}

/// **A press that lands on the control is not a press on the canvas.**
///
/// The canvas selects a tile on its press edge, and it reads the pointer out
/// of the context rather than through an `egui::Response` — so egui's own
/// paint-order precedence, which does suppress the widget under a widget,
/// does not reach it. Without the gate in `drive_gestures` a click on `log`
/// also moved the window's selected tile to the tile under the control, so
/// throwing one tile's switch renamed what the inspector was describing.
///
/// The selection is set on another tile first, because a gate that were
/// missing would be invisible against no selection at all — which is the
/// state the page opens in.
#[test]
fn a_press_on_the_switch_is_not_a_press_on_the_canvas() {
    let mut live = Live::open(housing_boot());
    live.settle();

    let income = live.switch("median_income").plot;
    let at = live.at(income, 0.5);
    live.click(at);
    assert_eq!(
        live.doc().selected_column().map(|c| c.column.clone()),
        Some("median_income".to_string()),
        "the press on the tile body did not select it, so the gate below is          being measured against nothing"
    );

    let switch = live.switch("population");
    live.click(switch.states[1].1.center());
    assert_eq!(
        live.doc().selected_column().map(|c| c.column.clone()),
        Some("median_income".to_string()),
        "the press on population's switch moved the window's selected tile"
    );
    assert!(
        live.doc().selection_sql().is_none(),
        "the press on the switch committed {:?}",
        live.doc().selection_sql()
    );
}

// ---------------------------------------------------------------------------
// AC5 — a brush elsewhere narrows the log tile without moving its scale.
// ---------------------------------------------------------------------------

/// With `population` on log, a brush swept on `median_income` narrows
/// population's **filtered layer on the log bins** and leaves the tile's
/// scale, its bins and its ticks exactly where they were.
///
/// The two layers are the assertion. A histogram tile emits the unfiltered
/// ghost and the layer the selection narrows over one column; after the sweep
/// the ghost's rows are byte-equal to what they were and the filtered layer's
/// total has dropped, with every bin it still holds standing on an edge the
/// ghost also holds. A build that re-cut the tile on the *filtered* rows would
/// pass a test that only counted, and fails the edge check here.
#[test]
fn a_brush_on_another_tile_narrows_the_log_tile_on_its_own_bins() {
    let mut live = Live::open(housing_boot());
    live.settle();
    live.switch_to("population", ScaleType::Log);
    assert_eq!(live.switch("population").active, ScaleType::Log);

    let population = live.switch("population").plot;
    let marks = marks_binning(live.app.chart_doc_mut(), "population");
    assert_eq!(
        marks.len(),
        2,
        "a histogram tile draws a ghost and a filtered layer; it drew {marks:?}"
    );
    let (ghost, filtered) = (marks[0], marks[1]);
    let ghost_before = mark_bins(live.app.chart_doc_mut(), ghost, "population");
    let filtered_before = mark_bins(live.app.chart_doc_mut(), filtered, "population");
    let frame_before = plot_frame(live.doc(), population);
    let ticks_before = x_tick_labels(live.doc(), population);
    let total = |bins: &[(f64, f64)]| bins.iter().map(|(_, c)| *c).sum::<f64>();
    assert!(
        (total(&ghost_before) - total(&filtered_before)).abs() < f64::EPSILON,
        "nothing is selected yet, so the two layers hold the same rows"
    );

    // The sweep lands on the first tile in the column — `median_income`, the
    // file's own first column — and not on population's own tile.
    let income = live.switch("median_income").plot;
    assert_ne!(income, population);
    live.brush(income, 0.15, 0.45);
    assert!(
        live.doc().selection_sql().is_some(),
        "the sweep committed no selection"
    );

    let ghost_after = mark_bins(live.app.chart_doc_mut(), ghost, "population");
    let filtered_after = mark_bins(live.app.chart_doc_mut(), filtered, "population");
    assert_eq!(
        ghost_after, ghost_before,
        "the unfiltered layer is what the tile keeps behind the selection"
    );
    assert!(
        total(&filtered_after) < total(&filtered_before),
        "the filtered layer holds {} rows and held {}",
        total(&filtered_after),
        total(&filtered_before)
    );
    assert!(
        total(&filtered_after) > 0.0,
        "the sweep left the tile empty, which measures nothing about its bins"
    );

    // On the SAME bins: every edge the narrowed layer still stands on is an
    // edge the log cut produced, not a fresh cut over the filtered rows.
    let edges: Vec<f64> = ghost_before.iter().map(|(bin, _)| *bin).collect();
    for (bin, _) in &filtered_after {
        assert!(
            edges.iter().any(|e| (e - bin).abs() < f64::EPSILON),
            "the narrowed layer stands on {bin}, which is not one of the log \
             edges {edges:?}"
        );
    }

    // And the tile's own frame is untouched: same scale, same box, same ticks.
    assert_eq!(
        plot_frame(live.doc(), population),
        frame_before,
        "the brush moved population's scale or its box"
    );
    assert_eq!(
        x_tick_labels(live.doc(), population),
        ticks_before,
        "the brush moved population's ticks"
    );
    assert_eq!(
        live.switch("population").active,
        ScaleType::Log,
        "the switch stopped saying log"
    );
}
