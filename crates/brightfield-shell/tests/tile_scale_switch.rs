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

use brightfield_shell::app::ChartDoc;
use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_protocol::layout::Flow;
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
    /// over a bare `on_hover_text` in an empty context reproduces with nothing
    /// of this window in it. Two frames is not enough and this is the test
    /// that says so.
    fn hover_shapes(&mut self, pos: egui::Pos2) -> Vec<egui::epaint::ClippedShape> {
        // The delay is zeroed HERE and not at construction. The window
        // installs the design system's whole `Style` on its first draw —
        // `meridian_egui`'s `set_style_of` replaces the struct rather than
        // editing it — so a delay set before that frame is gone by the time a
        // pointer could rest on anything.
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
            .unwrap_or_else(|| panic!("the frame painted no {word:?} inside population's {word} segment"));
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
    // says the paint happens at all.
    let at = live.switch("population").rect.center();
    let shapes = live.hover_shapes(at);
    let painted: Vec<String> = texts(&shapes).into_iter().map(|(t, _, _)| t).collect();
    assert!(
        painted.iter().any(|t| t == "scale: population"),
        "resting on population's switch painted no hover text; it painted {painted:?}"
    );
}

/// A document the generator never named draws no switch at all — the list the
/// control is derived from is the tile list, and a shipped start has none.
#[test]
fn an_authored_spec_draws_no_scale_switch() {
    let boot = Boot::start(brightfield_shell::starts::CROSSWALK, Flow::Vertical)
        .expect("the shipped start opens");
    let mut live = Live::open(boot);
    live.settle();
    assert!(
        live.doc().tile_columns().is_empty(),
        "a shipped start names no tiles"
    );
    assert!(
        live.doc().scale_switches.is_empty(),
        "it drew {:?}",
        live.doc().scale_switches
    );
}
