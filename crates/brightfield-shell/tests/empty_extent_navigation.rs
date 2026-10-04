//! **A plot whose navigated extent has no data beneath it stays placed** —
//! panning or zooming the map pane past the cloud does not drop the hero out
//! of the composition.
//!
//! Before this card, a plot whose marks each queried clean and drew zero
//! rows under its navigated extent was silently `continue`d out of
//! `Composed::plots` and its scene out of `placements`
//! (`compose_from_results`, `crates/brightfield-shell/src/pipeline.rs`). On a
//! generated dashboard that is not "the picture goes blank" — it is a plot
//! COUNT that drops by one while `Dashboard::tile_columns()` (set once, at
//! file open, and left at that size from then on) keeps its own count, so
//! each plot after the dropped one reads one index low against the tile it
//! is supposed to be: a press on the column's own top tile resolves through
//! `composed.plots`' shifted index into the WRONG entry of
//! `ChartDoc::tile_columns()`.
//!
//! Driven through the real shell, as `tests/canvas_pane_group.rs` and
//! `tests/equal_aspect_resize.rs` are: [`MeridianApp::headless`] over
//! `california_housing_sample.csv`, a real secondary-button drag for each
//! pan (the shape `tests/navigation_extent.rs`'s
//! `a_secondary_button_drag_pans_and_queries_on_release` already drives), and
//! a real primary-button click for the tile-select assertion.

use brightfield_engine::ProfileOutcome;
use brightfield_shell::app::GridLayout;
use brightfield_shell::data_file;
use brightfield_shell::design::Mode;
use brightfield_shell::legend::{band_width, blocks, LegendSpec};
use brightfield_shell::pipeline::LiveDashboard;
use brightfield_shell::shelf_edit::put_colour;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::analysis::ComponentPath;

/// The window this card's own evidence was measured at.
const SCREEN: egui::Rect = egui::Rect {
    min: egui::Pos2::ZERO,
    max: egui::pos2(1440.0, 900.0),
};

fn fixture() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv")
}

/// A settled window over the fixture, as it opens — the ledger rail closed to
/// its strip, same as `tests/canvas_pane_group.rs::settled`.
fn settled() -> (MeridianApp, egui::Context) {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    settle(boot)
}

/// `boot` as a settled window.
fn settle(boot: Boot) -> (MeridianApp, egui::Context) {
    let mut app = MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(SCREEN),
        ..Default::default()
    };
    for _ in 0..3 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    (app, ctx)
}

fn frame(app: &mut MeridianApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput {
        screen_rect: Some(SCREEN),
        events,
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| app.draw(ui));
}

fn button(pos: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// **Throw the layout switch on the grid pane's header band to its columns
/// state**, and settle.
///
/// A tiled column's own picture is on screen only with the grid transposed:
/// untransposed the grid pane draws a table and each column's distribution is
/// a rug in its header band, so a press aimed at a tile has no tile to land
/// on. `tests/navigator_spine.rs` and `tests/hover_readout.rs` carry the same
/// helper, each in the shape its own file uses.
///
/// Aimed at the rect the frame recorded for the control, and the state is read
/// back: a miss would leave the press below landing on the table instead.
fn transpose_the_grid(app: &mut MeridianApp, ctx: &egui::Context) {
    let at = app
        .chart_doc()
        .grid_layout_switch
        .as_ref()
        .expect("the grid pane's header band drew a layout switch")
        .states
        .iter()
        .find(|(state, _)| *state == GridLayout::Columns)
        .expect("the switch offers a columns state")
        .1
        .center();
    frame(app, ctx, vec![egui::Event::PointerMoved(at)]);
    frame(
        app,
        ctx,
        vec![button(at, egui::PointerButton::Primary, true)],
    );
    frame(
        app,
        ctx,
        vec![button(at, egui::PointerButton::Primary, false)],
    );
    for _ in 0..3 {
        frame(app, ctx, Vec::new());
    }
    assert_eq!(
        app.grid_layout(),
        GridLayout::Columns,
        "the click at {at:?} did not throw the switch, so the tile the press \
         below aims at is not on screen"
    );
}

/// A point inside plot `index`'s own DATA area, at `fx` and `fy` of its
/// width and height — the frame the axes bound, not the plot's outer rect, so
/// a click or a drag origin lands inside the picture rather than on a margin.
/// [`tests/canvas_pane_group.rs::hero_data_point`] is this at `index = 0`.
fn plot_data_point(app: &MeridianApp, index: usize, fx: f64, fy: f64) -> egui::Pos2 {
    let drawn = app.composed_plot_rects()[index];
    let doc = app.chart_doc();
    let l = &doc.composed.plots[index].layout;
    #[allow(clippy::cast_possible_truncation)]
    egui::pos2(
        drawn.left() + (l.plot_x_start() + (l.plot_x_end() - l.plot_x_start()) * fx) as f32,
        drawn.top() + (l.plot_y_start() + (l.plot_y_end() - l.plot_y_start()) * fy) as f32,
    )
}

/// One settled secondary-button pan across the hero's own data area, from
/// `(0.15, 0.15)` of it to `(0.85, 0.85)` — the drag this card's own evidence
/// was measured with. Four intermediate steps and a release, so the gesture
/// settles (queries) rather than staying a live drag.
fn pan_the_map(app: &mut MeridianApp, ctx: &egui::Context) {
    let from = plot_data_point(app, 0, 0.15, 0.15);
    let to = plot_data_point(app, 0, 0.85, 0.85);
    frame(
        app,
        ctx,
        vec![
            egui::Event::PointerMoved(from),
            button(from, egui::PointerButton::Secondary, true),
        ],
    );
    for step in 1..=4 {
        #[allow(clippy::cast_precision_loss)]
        let t = step as f32 / 4.0;
        let at = from + (to - from) * t;
        frame(app, ctx, vec![egui::Event::PointerMoved(at)]);
    }
    frame(
        app,
        ctx,
        vec![button(to, egui::PointerButton::Secondary, false)],
    );
    // Settle: a released pan queries on the frame after the release, same
    // shape as `tests/navigation_extent.rs`'s own secondary-drag test.
    frame(app, ctx, Vec::new());
}

/// **AC1, AC2, AC3** — two pans past the data keep the hero placed with its
/// axes drawn and read as empty, the column's top tile still selects its
/// own column, and a pan back over the data restores the points with no
/// reset.
#[test]
fn a_navigated_map_with_no_data_beneath_it_stays_placed() {
    let (mut app, ctx) = settled();

    let before_plots = app.chart_doc().composed.plots.len();
    let before_tiles = app.chart_doc().tile_columns().len();
    assert_eq!(
        before_plots, before_tiles,
        "the fixture's own plots and tile columns start at one index apiece"
    );
    let top_tile_column = app.chart_doc().tile_columns()[1].column.clone();

    // Two settled pans, each carrying the map further off the data — the
    // reproduction this card's evidence names.
    pan_the_map(&mut app, &ctx);
    pan_the_map(&mut app, &ctx);

    let doc = app.chart_doc();
    assert!(
        doc.navigated(),
        "two settled pans left no navigation in force — this gate needs a \
         gesture the window actually applied"
    );

    // AC1 — the hero is still placed, in the map pane, axes drawn, header
    // unchanged, and `composed.plots` keeps its count.
    assert_eq!(
        doc.composed.plots.len(),
        before_plots,
        "the navigated-empty hero was dropped instead of staying placed — \
         `composed.plots` no longer keeps one index per `tile_columns()` entry"
    );
    let hero = &doc.composed.plots[0];
    assert!(
        hero.navigated_empty,
        "the hero drew real marks after two pans meant to carry it past \
         every row — this gate needs a gesture that actually empties it"
    );
    let panes = app.canvas_panes();
    let map = panes.pane("map").expect("the map pane drew");
    let hero_rect = app.composed_plot_rects()[0];
    assert!(
        map.body.contains_rect(hero_rect),
        "the hero's placed rect {hero_rect:?} is not inside the map pane's \
         content rect {:?}",
        map.body
    );
    use brightfield_render::channel::Channel;
    assert!(
        hero.scales.get(Channel::X).is_some() && hero.scales.get(Channel::Y).is_some(),
        "the empty hero drew no axes — `scales` carries neither a continuous \
         X nor a continuous Y to draw ticks from"
    );

    // AC2 — the column's top tile still selects its own column.
    //
    // Thrown here rather than at the open, so the pans above happen in the
    // layout the hero is read in and only the press below needs the tile.
    transpose_the_grid(&mut app, &ctx);
    let top_tile = plot_data_point(&app, 1, 0.5, 0.5);
    frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::PointerMoved(top_tile),
            button(top_tile, egui::PointerButton::Primary, true),
        ],
    );
    frame(
        &mut app,
        &ctx,
        vec![button(top_tile, egui::PointerButton::Primary, false)],
    );
    let selected = app.chart_doc().selected_column().map(|f| f.column.clone());
    assert_eq!(
        selected.as_deref(),
        Some(top_tile_column.as_str()),
        "a press on the column's own top tile selected {selected:?} instead \
         of its own column {top_tile_column:?} — `composed.plots` and \
         `tile_columns()` have shifted apart by an index"
    );

    // AC3 — a pan back over the data restores the points, with no reset.
    let reset_before = app.chart_doc().navigated();
    let back_from = plot_data_point(&app, 0, 0.85, 0.85);
    let back_to = plot_data_point(&app, 0, 0.15, 0.15);
    frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::PointerMoved(back_from),
            button(back_from, egui::PointerButton::Secondary, true),
        ],
    );
    for step in 1..=4 {
        #[allow(clippy::cast_precision_loss)]
        let t = step as f32 / 4.0;
        let at = back_from + (back_to - back_from) * t;
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    }
    frame(
        &mut app,
        &ctx,
        vec![button(back_to, egui::PointerButton::Secondary, false)],
    );
    frame(&mut app, &ctx, Vec::new());

    assert!(
        reset_before,
        "the gate above already asserted `navigated()`; restated so a \
         reordering of this test still catches a false pass below"
    );
    let restored = app.chart_doc();
    assert!(
        !restored.composed.plots[0].navigated_empty,
        "panning back over the data left the hero reading empty — no reset \
         was pressed, so this has to be the pan-back putting rows under it \
         again"
    );
}

/// The number column the hero is coloured by.
const COLOUR: &str = "median_house_value";

/// The fixture's hero map coloured by [`COLOUR`] through the shelf's own
/// `put_colour`, which writes the legend item beside the scheme, as a live
/// window settled over it.
fn settled_coloured() -> (MeridianApp, egui::Context) {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let mut file =
        data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let table = file
        .live
        .coordinator()
        .session()
        .profile_sources()
        .into_iter()
        .find(|p| p.name == data_file::SOURCE)
        .map(|p| match p.outcome {
            ProfileOutcome::Profiled { columns, .. } => columns,
            other => panic!("the table did not profile: {other:?}"),
        })
        .expect("the opened file has a source to profile");
    let hero = ComponentPath(file.composed.plots[0].path.clone());
    let mut spec = file.live.spec().clone();
    put_colour(&mut spec, &hero, COLOUR, &table).expect("the table has the column");
    let base = file.live.base_dir().map(std::path::Path::to_path_buf);
    let mut live = LiveDashboard::load(spec, base.as_deref()).expect("the coloured spec loads");
    let composed = live.present().expect("the coloured page presents");
    let mut boot = Boot::charts(composed);
    boot.live = Some(live);
    settle(boot)
}

/// **A coloured map navigated to an empty extent draws no legend, and the plot
/// keeps the name of the column it is coloured by.** Measured at `505c0fd` and
/// again at the head this lands on: the pans leave the hero placed with its
/// axes (`navigated_empty`) and no colour scale to read a legend from, so
/// [`LegendSpec::of_plot`] answers `None` and the page reserves no band for one.
/// The fill column is still recorded on that path (`compose_from_results`'s
/// synthetic-batch fallback), so a colour scale that ever reaches it draws its
/// legend with its name rather than without.
///
/// Fails when the lines that record the fill column on that path are removed
/// (the handle names none), and when a legend starts to draw on it (a block, or
/// a band reserved for one).
#[test]
fn a_coloured_map_with_no_data_beneath_it_draws_no_legend_and_keeps_its_fill_column() {
    let (mut app, ctx) = settled_coloured();

    // Before the pans the legend is drawn and names the column, so the colouring
    // took and an empty answer after them is a change and not an absence.
    let composed = &app.chart_doc().composed;
    assert_eq!(
        composed.plots[0].fill_column.as_deref(),
        Some(COLOUR),
        "the coloured hero's handle names its fill column"
    );
    assert!(
        matches!(
            LegendSpec::of_plot(&composed.plots[0]),
            Some(LegendSpec::Sequential { .. })
        ),
        "the coloured hero draws no legend before it is panned"
    );
    assert_eq!(blocks(composed).len(), 1, "the legend's block");
    assert!(band_width(composed) > 0.0, "the legend's band");

    // The reproduction the first test in this file names: two settled pans, each
    // carrying the map further off the data.
    pan_the_map(&mut app, &ctx);
    pan_the_map(&mut app, &ctx);

    let composed = &app.chart_doc().composed;
    let hero = &composed.plots[0];
    assert!(
        hero.navigated_empty,
        "the hero drew real marks after two pans meant to carry it past every row"
    );
    assert_eq!(
        hero.fill_column.as_deref(),
        Some(COLOUR),
        "the navigated-empty hero lost the name of its fill column"
    );
    assert_eq!(
        LegendSpec::of_plot(hero),
        None,
        "a legend started to draw for a plot navigated to an empty extent"
    );
    assert!(
        blocks(composed).is_empty(),
        "a legend block is drawn for a plot navigated to an empty extent"
    );
    assert_eq!(
        band_width(composed),
        0.0,
        "a band is reserved for a legend a plot navigated to an empty extent does not draw"
    );
}
