//! **Quick open**: ⌘P finds any table or view of the Protocol by its DuckDB
//! name, from whatever holds the canvas, and puts it in focus in the activity
//! the canvas is showing.

#[path = "support/scenes.rs"]
mod scenes;

use brightfield_keys::Availability;
use brightfield_protocol::graph::AssetId;
use brightfield_shell::protocol::NodeView;
use brightfield_shell::window::CanvasHolds;

const QUICK_OPEN: &str = "quick-open";

/// The Protocol's tables and views as the outline lists them: each asset row
/// whose kind holds rows, with its id and its label.
fn tables(win: &scenes::Window) -> Vec<(AssetId, String)> {
    win.app
        .protocol_model()
        .outline()
        .into_iter()
        .filter(|row| row.depth == 0 && row.kind.is_tabular())
        .map(|row| (row.id, row.label))
        .collect()
}

/// Open quick open by its key and type `name` into its query line.
fn find(win: &mut scenes::Window, name: &str) {
    win.press("cmd-p");
    assert_eq!(win.answered, [QUICK_OPEN]);
    assert_eq!(win.app.open_overlay(), Some(QUICK_OPEN));
    win.type_text(name);
}

/// The first row quick open lists, by its label.
fn first_row(win: &scenes::Window) -> Option<String> {
    win.app
        .quick_open_rows()
        .into_iter()
        .next()
        .map(|(label, _)| label)
}

#[test]
fn cmd_p_finds_a_table_by_its_duckdb_name_from_any_focus() {
    // Over the graph.
    let mut win = scenes::crosswalk();
    let listed = tables(&win);
    assert!(listed.len() > 1, "the crosswalk lists tables: {listed:?}");
    let (_, name) = listed.last().cloned().expect("a table");
    find(&mut win, &name);
    assert_eq!(first_row(&win).as_deref(), Some(name.as_str()));
    let (_, detail) = win.app.quick_open_rows().remove(0);
    assert!(
        detail.is_some_and(|step| !step.is_empty()),
        "a table's row names the step that makes it"
    );

    // Over a chart, and over a grid.
    let mut win = scenes::data_file();
    let table = win.table.clone().expect("the data file's table");
    assert!(matches!(
        win.app.canvas_holds(),
        CanvasHolds::Dashboard { .. }
    ));
    find(&mut win, &table);
    assert_eq!(first_row(&win).as_deref(), Some(table.as_str()));
    win.key(egui::Key::Escape);
    assert_eq!(win.app.open_overlay(), None);

    win.press("cmd-alt-g");
    assert_eq!(win.app.canvas_holds().view(), Some(NodeView::Grid));
    find(&mut win, &table);
    assert_eq!(first_row(&win).as_deref(), Some(table.as_str()));
}

#[test]
fn choosing_a_table_focuses_it_in_the_current_activity() {
    // A grid stays a grid.
    let mut win = scenes::data_file();
    win.press("cmd-alt-g");
    let held = match win.app.canvas_holds() {
        CanvasHolds::View { node, .. } => node.clone(),
        other => panic!("the grid did not take the canvas: {other:?}"),
    };
    let (other, name) = tables(&win)
        .into_iter()
        .find(|(id, _)| *id != held)
        .expect("a table beside the held one");
    find(&mut win, &name);
    win.key(egui::Key::Enter);
    assert_eq!(win.app.open_overlay(), None);
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::View {
            node: other.clone(),
            view: NodeView::Grid,
        }
    );
    assert_eq!(win.app.protocol_model().selected(), Some(&other));

    // The graph stays the graph, with the table selected on it.
    let mut win = scenes::crosswalk();
    let (id, name) = tables(&win).last().cloned().expect("a table");
    find(&mut win, &name);
    win.key(egui::Key::Enter);
    assert_eq!(win.app.canvas_holds(), &CanvasHolds::Graph);
    assert_eq!(win.app.protocol_model().selected(), Some(&id));
}

#[test]
fn over_a_chart_the_chart_stays_and_a_table_no_chart_reads_opens_as_its_grid() {
    let mut win = scenes::data_file();
    let (chart, held) = match win.app.canvas_holds() {
        CanvasHolds::Dashboard { node, table } => (node.clone(), table.clone()),
        other => panic!("a data file opens on its chart: {other:?}"),
    };
    let listed = tables(&win);

    // The table the chart reads: the chart stays.
    let (_, held_name) = listed
        .iter()
        .find(|(id, _)| *id == held)
        .cloned()
        .expect("the chart's table is listed");
    find(&mut win, &held_name);
    win.key(egui::Key::Enter);
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::Dashboard {
            node: chart,
            table: held.clone(),
        }
    );

    // A table no chart reads: its grid takes the canvas.
    let (other, name) = listed
        .into_iter()
        .find(|(id, _)| *id != held)
        .expect("a table beside the chart's");
    find(&mut win, &name);
    win.key(egui::Key::Enter);
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::View {
            node: other.clone(),
            view: NodeView::Grid,
        }
    );
    assert_eq!(win.app.protocol_model().selected(), Some(&other));
}

#[test]
fn quick_open_is_offered_off_the_graph_too() {
    let mut win = scenes::data_file();
    assert!(!win.app.graph_on_canvas(), "a data file opens on its chart");
    assert_eq!(
        win.app.availability_of(QUICK_OPEN),
        Some(Availability::Available)
    );
    let row = win
        .app
        .menu_rows()
        .into_iter()
        .find(|row| row.longname == QUICK_OPEN)
        .expect("quick open is on a menu");
    assert_eq!(row.missing, None);

    win.choose(QUICK_OPEN);
    assert_eq!(win.applied, [QUICK_OPEN]);
    assert_eq!(win.app.open_overlay(), Some(QUICK_OPEN));
    let table = win.table.clone().expect("the data file's table");
    assert!(
        win.app
            .quick_open_rows()
            .iter()
            .any(|(label, _)| *label == table),
        "the empty query lists every table"
    );
}

#[test]
fn the_palettes_shift_cmd_p_still_opens_the_palette() {
    let mut win = scenes::data_file();
    win.press("cmd-shift-p");
    assert_eq!(win.app.open_overlay(), Some("palette"));
}

#[test]
fn quick_open_is_greyed_on_the_front_door() {
    let win = scenes::door();
    assert!(matches!(
        win.app.availability_of(QUICK_OPEN),
        Some(Availability::Missing(_))
    ));
}
