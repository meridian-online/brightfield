//! **A plot whose navigated extent holds none of its data stays placed** —
//! panning or zooming the map pane past the cloud does not drop the hero out
//! of the composition.
//!
//! Before this card, a plot whose every mark queried clean and drew zero rows
//! under its navigated extent was silently `continue`d out of
//! `Composed::plots` and its scene out of `placements`
//! (`compose_from_results`, `crates/brightfield-shell/src/pipeline.rs`). On a
//! generated dashboard that is not "the picture goes blank" — it is a plot
//! COUNT that drops by one while `Dashboard::tile_columns()` (set once, at
//! file open, and never resized) keeps its own count, so every plot AFTER the
//! dropped one reads one index low against the tile it is supposed to be. The
//! map pane's count overlay reads `composed.plots.first()` for its own
//! position (`crate::window::hero_data_area`) and finds the wrong plot there
//! entirely, so it draws nowhere at all — measured as `canvas_panes().count`
//! going `None`. And a press on the column's own top tile resolves through
//! `composed.plots`' shifted index into the WRONG entry of
//! `ChartDoc::tile_columns()`.
//!
//! Driven through the real shell, as `tests/canvas_pane_group.rs` and
//! `tests/equal_aspect_resize.rs` are: [`MeridianApp::headless`] over
//! `california_housing_sample.csv`, a real secondary-button drag for each
//! pan (the shape `tests/navigation_extent.rs`'s
//! `a_secondary_button_drag_pans_and_queries_on_release` already drives), and
//! a real primary-button click for the tile-select assertion.

use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp};

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
/// axes drawn and its count at zero, the column's top tile still selects its
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

    // AC2 — the count chip reads zero rather than disappearing, and the
    // column's top tile still selects its own column.
    let count = panes
        .count
        .expect("the count overlay did not draw over an empty-but-placed hero");
    assert!(
        map.body.contains_rect(count),
        "the count at {count:?} is not inside the map pane {:?}",
        map.body
    );
    let count_text = panes
        .count_text
        .clone()
        .expect("the count overlay drew a rect but no text — the two come off one paint");
    assert!(
        count_text.starts_with("0 points"),
        "the count chip read {count_text:?} — an empty-under-navigation hero \
         should say zero points, not the file's own static total"
    );

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
    let restored_panes = app.canvas_panes();
    assert!(
        restored_panes.count.is_some(),
        "the count overlay stayed absent after the pan back put the map \
         over real data again"
    );
    let restored_text = restored_panes
        .count_text
        .clone()
        .expect("the restored count overlay drew a rect but no text");
    assert!(
        !restored_text.starts_with("0 points"),
        "the count chip still read {restored_text:?} after the pan back put \
         real rows under the hero again"
    );
}
