//! **The open places above the spine**: each place the canvas shows is listed
//! once, a click brings it back, ⌘W closes the current one but never the last,
//! and ⇧⌘] and ⇧⌘[ walk the list.

#[path = "support/scenes.rs"]
mod scenes;

use brightfield_keys::places::{CLOSE_PLACE, NEXT_PLACE, PREV_PLACE};
use brightfield_shell::protocol::NodeView;
use brightfield_shell::spine::Place;
use brightfield_shell::window::CanvasHolds;

/// The data file's table and the chart that reads it, off the canvas it opens
/// on.
fn table_and_chart(win: &scenes::Window) -> (String, String) {
    match win.app.canvas_holds() {
        CanvasHolds::Dashboard { node, table } => (table.clone(), node.clone()),
        other => panic!("a data file opens on its dashboard, not {other:?}"),
    }
}

fn places(win: &scenes::Window) -> Vec<Place> {
    win.app.open_places().places().to_vec()
}

/// The data file with three places open: its chart, its grid and the graph,
/// the graph current.
fn three_places() -> (scenes::Window, String, String) {
    let mut win = scenes::data_file();
    let (table, chart) = table_and_chart(&win);
    win.press("cmd-alt-g");
    assert!(
        win.graph_to_canvas(),
        "the graph chip put no graph on the canvas"
    );
    win.settle();
    assert_eq!(
        places(&win),
        [
            Place::Chart {
                node: chart.clone(),
                table: table.clone()
            },
            Place::Grid(table.clone()),
            Place::Graph,
        ]
    );
    (win, table, chart)
}

#[test]
fn focusing_a_node_adds_its_place_once() {
    let mut win = scenes::data_file();
    let (table, chart) = table_and_chart(&win);
    let on_chart = Place::Chart {
        node: chart,
        table: table.clone(),
    };
    assert_eq!(places(&win), std::slice::from_ref(&on_chart));
    win.press("cmd-alt-g");
    win.press("cmd-alt-c");
    win.press("cmd-alt-g");
    assert_eq!(places(&win), [on_chart, Place::Grid(table)]);
    assert_eq!(win.app.open_places().current(), Some(1));
    let drawn = win.app.places_drawn();
    assert_eq!(drawn.len(), 2, "both places are drawn: {drawn:?}");
    assert!(drawn[0].label.ends_with(" · Chart"), "{drawn:?}");
    assert!(drawn[1].label.ends_with(" · Grid"), "{drawn:?}");
    assert!(drawn[1].current && !drawn[0].current, "{drawn:?}");
    assert!(
        drawn[0].rect.bottom() <= drawn[1].rect.top(),
        "the places stack: {drawn:?}"
    );
    let first_row = win
        .app
        .spine_rows()
        .first()
        .map(|row| row.rect)
        .expect("the spine is drawn");
    assert!(
        drawn[1].rect.bottom() <= first_row.top(),
        "the places stand above the spine: {drawn:?}, {first_row:?}"
    );
}

#[test]
fn clicking_a_place_brings_its_node_and_view_back() {
    let (mut win, table, chart) = three_places();
    let grid = win.app.places_drawn()[1].rect.left_center() + egui::vec2(24.0, 0.0);
    win.click(grid);
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::View {
            node: table.clone(),
            view: NodeView::Grid,
        }
    );
    assert_eq!(win.app.open_places().current(), Some(1));
    let chart_at = win.app.places_drawn()[0].rect.left_center() + egui::vec2(24.0, 0.0);
    win.click(chart_at);
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::Dashboard {
            node: chart,
            table: table.clone()
        }
    );
    let graph_at = win.app.places_drawn()[2].rect.left_center() + egui::vec2(24.0, 0.0);
    win.click(graph_at);
    assert!(
        win.app.graph_on_canvas(),
        "the graph's place brings it back"
    );
    // The graph chip still leads back to where the graph was reached from.
    assert!(win.graph_to_canvas());
    let chip = win
        .app
        .spine_rows()
        .iter()
        .find_map(|r| r.chip)
        .expect("the spine's head draws the chip");
    win.click(chip.rect.center());
    assert!(matches!(
        win.app.canvas_holds(),
        CanvasHolds::Dashboard { .. }
    ));
    assert_eq!(places(&win).len(), 3, "no place was added twice");
}

#[test]
fn closing_the_current_place_moves_to_the_one_above() {
    let (mut win, table, chart) = three_places();
    win.press("cmd-w");
    assert_eq!(win.answered, [CLOSE_PLACE]);
    assert_eq!(
        places(&win),
        [
            Place::Chart {
                node: chart.clone(),
                table: table.clone()
            },
            Place::Grid(table.clone()),
        ]
    );
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::View {
            node: table.clone(),
            view: NodeView::Grid,
        }
    );
    // The close control on the current place does the same.
    let row = win.app.places_drawn()[1].rect;
    let close = egui::pos2(
        row.right() - meridian_design::spacing::SPACE_2 - meridian_design::control::HEIGHT_XS / 2.0,
        row.center().y,
    );
    win.click(close);
    assert_eq!(places(&win), [Place::Chart { node: chart, table }]);
    assert!(matches!(
        win.app.canvas_holds(),
        CanvasHolds::Dashboard { .. }
    ));
}

#[test]
fn the_last_place_cannot_be_closed_out_of_the_window() {
    let mut win = scenes::data_file();
    let before = places(&win);
    assert_eq!(before.len(), 1);
    win.press("cmd-w");
    assert_eq!(win.answered, [CLOSE_PLACE]);
    assert_eq!(places(&win), before);
    assert!(
        !win.app.front_door_is_live(),
        "the window stays on the Protocol"
    );
    assert!(matches!(
        win.app.canvas_holds(),
        CanvasHolds::Dashboard { .. }
    ));
}

#[test]
fn next_and_previous_place_walk_the_list() {
    let (mut win, table, _) = three_places();
    win.press("cmd-shift-]");
    assert_eq!(win.answered, [NEXT_PLACE]);
    assert_eq!(
        win.app.open_places().current(),
        Some(0),
        "the top after the bottom"
    );
    assert!(matches!(
        win.app.canvas_holds(),
        CanvasHolds::Dashboard { .. }
    ));
    win.press("cmd-shift-]");
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::View {
            node: table,
            view: NodeView::Grid,
        }
    );
    win.press("cmd-shift-[");
    assert_eq!(win.answered, [PREV_PLACE]);
    assert_eq!(win.app.open_places().current(), Some(0));
    // The Window menu's items walk it the same way.
    win.choose(PREV_PLACE);
    assert_eq!(win.applied, [PREV_PLACE]);
    assert!(win.app.graph_on_canvas(), "the bottom before the top");
    win.choose(NEXT_PLACE);
    assert_eq!(win.app.open_places().current(), Some(0));
}

#[test]
fn the_front_door_closes_every_place_and_offers_no_place_verb() {
    let (mut win, _, _) = three_places();
    win.choose("open-home");
    assert!(win.app.front_door_is_live());
    assert!(places(&win).is_empty());
    assert!(win.app.places_drawn().is_empty());
    // The key is refused with its sentence, and taken, so the door stays.
    win.press("cmd-w");
    assert!(win.answered.is_empty());
    assert!(win.app.menu_said().is_some(), "⌘W on the door says why");
    assert!(win.app.front_door_is_live(), "⌘W on the door leaves it");
}

/// **Opening another file closes the places of the one before.** The places
/// are one Protocol's: a file opened over the data file, with no visit to the
/// front door between, lists its own chart alone, not the graph and grid the
/// file before it had open.
#[test]
fn opening_another_file_closes_the_places_of_the_one_before() {
    let (mut win, table, _) = three_places();
    let other = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv");
    win.open_path(&other);
    win.settle();
    let (other_table, other_chart) = table_and_chart(&win);
    assert_ne!(
        other_table, table,
        "the second file opened the first's table"
    );
    assert_eq!(
        places(&win),
        [Place::Chart {
            node: other_chart,
            table: other_table
        }]
    );
}
