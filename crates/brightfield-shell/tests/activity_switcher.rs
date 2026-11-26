//! **The activities of the node in focus**: Grid and Chart over a table or a
//! chart, Model over the model step, each switched to by its key or its menu
//! command, and each greyed with the sentence its verb says.

#[path = "support/scenes.rs"]
mod scenes;

use brightfield_keys::activity::{RELATE, SHOW_CHART, SHOW_GRID};
use brightfield_keys::{Availability, Need};
use brightfield_shell::node_activity::{offers, FocusedNode, NodeActivity};
use brightfield_shell::protocol::{NodeView, SpineRole};
use brightfield_shell::window::CanvasHolds;
use brightfield_workbench::arrangement;

/// The data file's table and the chart that reads it, off the canvas it opens
/// on.
fn table_and_chart(win: &scenes::Window) -> (String, String) {
    match win.app.canvas_holds() {
        CanvasHolds::Dashboard { node, table } => (table.clone(), node.clone()),
        other => panic!("a data file opens on its dashboard, not {other:?}"),
    }
}

#[test]
fn switching_from_chart_to_grid_and_back_keeps_the_node_and_the_selection() {
    let mut win = scenes::data_file();
    let (table, chart) = table_and_chart(&win);
    assert!(
        scenes::arrange(&mut win, Need::Column(brightfield_keys::ColumnKind::Any)),
        "a column could not be selected"
    );
    win.settle();
    let column = win
        .app
        .chart_doc()
        .selected_column_name()
        .map(str::to_owned);
    assert!(column.is_some(), "the scene selected no column");
    let selected = win.app.protocol_model().selected().cloned();

    win.press("cmd-alt-g");
    assert_eq!(win.answered, [SHOW_GRID]);
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::View {
            node: table.clone(),
            view: NodeView::Grid,
        }
    );
    assert_eq!(
        win.app.node_in_focus(),
        Some(FocusedNode::Table(table.clone()))
    );
    assert_eq!(
        win.app
            .chart_doc()
            .selected_column_name()
            .map(str::to_owned),
        column
    );
    assert_eq!(win.app.protocol_model().selected().cloned(), selected);

    win.press("cmd-alt-c");
    assert_eq!(win.answered, [SHOW_CHART]);
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::Dashboard { node: chart, table }
    );
    assert_eq!(
        win.app
            .chart_doc()
            .selected_column_name()
            .map(str::to_owned),
        column
    );
    assert_eq!(win.app.protocol_model().selected().cloned(), selected);
}

#[test]
fn chart_opens_the_one_chart_that_reads_the_node() {
    let mut win = scenes::data_file();
    let (table, chart) = table_and_chart(&win);
    let model = win.app.protocol_model();
    assert_eq!(model.charts_reading(&table), vec![chart.clone()]);
    assert_eq!(model.table_read_by(&chart), Some(&table));
    assert_eq!(model.table_read_by(&table), None, "a table is no chart");
    win.choose(SHOW_GRID);
    assert_eq!(win.applied, [SHOW_GRID]);
    assert_eq!(win.app.canvas_holds().view(), Some(NodeView::Grid));

    win.choose(SHOW_CHART);
    assert_eq!(win.applied, [SHOW_CHART]);
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::Dashboard { node: chart, table }
    );
}

#[test]
fn a_table_and_a_chart_each_offer_grid_then_chart_and_both_can_be_taken() {
    let mut win = scenes::data_file();
    let (table, chart) = table_and_chart(&win);
    let on_chart = FocusedNode::Chart {
        node: chart,
        table: table.clone(),
    };
    assert_eq!(win.app.node_in_focus(), Some(on_chart));
    for step in [None, Some("cmd-alt-g")] {
        if let Some(keys) = step {
            win.press(keys);
            assert_eq!(
                win.app.node_in_focus(),
                Some(FocusedNode::Table(table.clone()))
            );
        }
        let offered = offers(&win.app);
        assert_eq!(
            offered.iter().map(|o| o.activity).collect::<Vec<_>>(),
            [NodeActivity::Grid, NodeActivity::Chart]
        );
        assert_eq!(
            offered.iter().map(|o| o.verb).collect::<Vec<_>>(),
            [SHOW_GRID, SHOW_CHART]
        );
        assert!(
            offered.iter().all(|o| o.unavailable.is_none()),
            "{offered:?}"
        );
    }
    assert_eq!(win.app.canvas_holds().view(), Some(NodeView::Grid));
}

/// The menu row `longname` draws, with the sentence it is greyed with.
fn menu_missing(win: &scenes::Window, longname: &str) -> Option<String> {
    win.app
        .menu_rows()
        .into_iter()
        .find(|row| row.longname == longname)
        .unwrap_or_else(|| panic!("{longname} is on no menu"))
        .missing
}

#[test]
fn over_the_graph_with_nothing_selected_grid_and_chart_are_greyed_and_do_nothing() {
    let mut win = scenes::data_file();
    assert!(win.graph_to_canvas(), "the graph did not take the canvas");
    assert_eq!(win.app.protocol_model().selected(), None);
    assert_eq!(win.app.node_in_focus(), None);
    assert!(offers(&win.app).is_empty());
    assert_eq!(
        menu_missing(&win, SHOW_GRID).as_deref(),
        Some("Grid needs a table or view in focus.")
    );
    assert_eq!(
        menu_missing(&win, SHOW_CHART).as_deref(),
        Some("Chart needs a table or view in focus.")
    );
    win.press("cmd-alt-g");
    assert_eq!(win.answered, [SHOW_GRID]);
    assert_eq!(
        win.app.menu_said(),
        Some("Grid needs a table or view in focus.")
    );
    assert_eq!(win.app.canvas_holds(), &CanvasHolds::Graph);
}

#[test]
fn chart_over_a_node_no_chart_reads_is_greyed_on_the_menu_and_by_its_offer() {
    let mut win = scenes::data_file();
    let (table, _) = table_and_chart(&win);
    assert!(win.graph_to_canvas(), "the graph did not take the canvas");
    let assets: Vec<_> = win
        .app
        .spine_rows()
        .iter()
        .filter(|r| r.role == SpineRole::Asset)
        .map(|r| r.rect.center())
        .collect();
    let mut found = false;
    for at in assets {
        win.click(at);
        if !win.app.graph_on_canvas() {
            assert!(win.graph_to_canvas());
        }
        match win.app.node_in_focus() {
            Some(FocusedNode::Table(node)) if node != table => {
                found = true;
                break;
            }
            _ => {}
        }
    }
    assert!(found, "no asset beside the table could be put in focus");
    let sentence = "There is no chart to show for this node yet.";
    assert_eq!(menu_missing(&win, SHOW_CHART).as_deref(), Some(sentence));
    assert_eq!(menu_missing(&win, SHOW_GRID), None);
    let offered = offers(&win.app);
    let chart = offered
        .iter()
        .find(|o| o.activity == NodeActivity::Chart)
        .expect("a table is offered Chart");
    assert_eq!(chart.unavailable.as_deref(), Some(sentence));
    win.press("cmd-alt-c");
    assert_eq!(win.app.menu_said(), Some(sentence));
    assert_eq!(win.app.canvas_holds(), &CanvasHolds::Graph);
}

#[test]
fn relate_needs_two_sources() {
    let mut win = scenes::data_file();
    let sentence = "Relate needs two sources.";
    assert_eq!(
        win.app.availability_of(RELATE),
        Some(Availability::Missing(sentence.to_string()))
    );
    let toasts = win.app.toasts().len();
    let holds = win.app.canvas_holds().clone();
    win.press("cmd-alt-m");
    assert_eq!(win.answered, [RELATE]);
    assert_eq!(win.app.menu_said(), Some(sentence));
    assert!(win.app.toasts().len() > toasts, "nothing said why");
    assert_eq!(win.app.canvas_holds(), &holds, "the canvas moved");
}

#[test]
fn the_model_step_offers_model_and_relate_is_taken_there() {
    let mut win = scenes::own_files();
    assert_eq!(win.app.node_in_focus(), Some(FocusedNode::Model));
    let offered = offers(&win.app);
    assert_eq!(offered.len(), 1);
    assert_eq!(offered[0].activity, NodeActivity::Model);
    assert_eq!(offered[0].unavailable, None);
    win.choose(RELATE);
    assert_eq!(win.applied, [RELATE]);
    assert!(win.app.model_pane().is_some());
    assert!(
        matches!(
            win.app.availability_of(SHOW_GRID),
            Some(Availability::Missing(_))
        ),
        "the model holds no table's rows, so Grid is greyed"
    );
}

#[test]
fn relate_brings_the_model_back_from_behind_a_tables_grid() {
    use brightfield_render::model_scene::Selection;
    let mut win = scenes::own_files();
    let dataset = win
        .app
        .model_pane_mut()
        .and_then(|pane| pane.layout().cards.first().map(|card| card.dataset.clone()))
        .expect("the model lists a table");
    let ctx = win.ctx.clone();
    assert!(win
        .app
        .open_model_in_grid(&ctx, &Selection::Dataset(dataset)));
    win.settle();
    assert!(win.app.is_model_behind_grid(), "the model is not waiting");
    assert!(
        matches!(
            win.app.node_in_focus(),
            Some(FocusedNode::Table(_) | FocusedNode::Chart { .. })
        ),
        "the table's rows are in focus: {:?}",
        win.app.node_in_focus()
    );
    assert_eq!(
        win.app.availability_of(RELATE),
        Some(Availability::Available)
    );

    win.press("cmd-alt-m");
    assert_eq!(win.answered, [RELATE]);
    assert!(
        !win.app.is_model_behind_grid(),
        "the model is still behind the grid"
    );
    assert_eq!(win.app.node_in_focus(), Some(FocusedNode::Model));
}

#[test]
fn the_chart_chord_as_the_mac_delivers_it_shows_the_chart_and_the_picture_chord_does_not() {
    let mut win = scenes::data_file();
    let (table, chart) = table_and_chart(&win);
    win.press("cmd-alt-g");
    assert_eq!(win.app.canvas_holds().view(), Some(NodeView::Grid));

    // The host makes a copy of the chord while Shift is held as well: that is
    // the chart picture's chord, not the Chart activity's.
    win.answered.clear();
    let picture = egui::Modifiers::COMMAND | egui::Modifiers::ALT | egui::Modifiers::SHIFT;
    win.frame(picture, vec![egui::Event::Copy]);
    win.settle();
    assert!(
        !win.answered.contains(&SHOW_CHART),
        "the picture chord showed the chart"
    );
    assert_eq!(win.app.canvas_holds().view(), Some(NodeView::Grid));

    // The host makes a copy of ⌘⌥C in place of its key event.
    win.answered.clear();
    win.frame(
        egui::Modifiers::COMMAND | egui::Modifiers::ALT,
        vec![egui::Event::Copy],
    );
    win.settle();
    assert_eq!(win.answered, [SHOW_CHART]);
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::Dashboard { node: chart, table }
    );
}

// ---------------------------------------------------------------------------
// The pane header: the node and the step that made it, the switcher, and the
// node's kind and size
// ---------------------------------------------------------------------------

/// The table's label, as the spine names it.
fn label_of(win: &scenes::Window, node: &str) -> String {
    win.app
        .protocol_model()
        .label_of(&node.to_string())
        .unwrap_or(node)
        .to_string()
}

#[test]
fn the_pane_header_names_the_node_and_the_step_that_made_it() {
    let mut win = scenes::data_file();
    let (table, _) = table_and_chart(&win);
    win.press("cmd-alt-g");
    let crumbs = win.app.header_crumbs();
    assert_eq!(
        crumbs.len(),
        2,
        "the header names a step and a node: {crumbs:?}"
    );
    assert_eq!(crumbs[1], label_of(&win, &table), "{crumbs:?}");
    let drawn = win.app.pane_header().expect("the grid drew a pane header");
    let labels: Vec<&str> = drawn.crumbs.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(labels, crumbs, "the header drew other crumbs than it names");
    assert!(
        drawn.crumbs[0].1.right() < drawn.crumbs[1].1.left(),
        "the step is not drawn before the node: {:?}",
        drawn.crumbs
    );
}

#[test]
fn the_header_offers_grid_and_chart_with_their_keys() {
    let mut win = scenes::data_file();
    let (choices, selected) = win
        .app
        .canvas_header()
        .activities
        .expect("the dashboard's header offers its activities");
    let platform = brightfield_keys::KeyPlatform::host();
    assert_eq!(
        choices
            .iter()
            .map(|c| (c.label, c.keystroke.clone()))
            .collect::<Vec<_>>(),
        [
            (
                "Grid",
                Some(brightfield_keys::keys_label("cmd-alt-g", platform))
            ),
            (
                "Chart",
                Some(brightfield_keys::keys_label("cmd-alt-c", platform))
            ),
        ]
    );
    assert_eq!(
        choices[selected].label, "Chart",
        "the dashboard shows Chart"
    );

    // A click on Grid, the switcher's first option, performs Grid's command.
    let switcher = win
        .app
        .pane_header()
        .and_then(|h| h.switcher)
        .expect("the header drew the switcher");
    let header = win.app.pane_header().cloned().unwrap();
    let after_crumbs = header.crumbs.last().map(|(_, r)| r.right()).unwrap();
    assert!(
        switcher.left() > after_crumbs,
        "the switcher is not after the breadcrumb"
    );
    win.applied.clear();
    win.click(egui::pos2(
        switcher.left() + switcher.width() / 4.0,
        switcher.center().y,
    ));
    assert_eq!(win.applied, [SHOW_GRID]);
    assert_eq!(win.app.canvas_holds().view(), Some(NodeView::Grid));
    let (_, selected) = win.app.canvas_header().activities.unwrap();
    assert_eq!(selected, 0, "the grid's header does not show Grid");

    // …and Chart, its second, brings the chart back.
    let switcher = win.app.pane_header().and_then(|h| h.switcher).unwrap();
    win.applied.clear();
    win.click(egui::pos2(
        switcher.right() - switcher.width() / 4.0,
        switcher.center().y,
    ));
    assert_eq!(win.applied, [SHOW_CHART]);
    assert!(matches!(
        win.app.canvas_holds(),
        CanvasHolds::Dashboard { .. }
    ));
}

#[test]
fn an_activity_the_node_cannot_take_is_greyed_with_its_reason() {
    let mut win = scenes::data_file();
    let (table, _) = table_and_chart(&win);
    assert!(win.graph_to_canvas(), "the graph did not take the canvas");
    let assets: Vec<_> = win
        .app
        .spine_rows()
        .iter()
        .filter(|r| r.role == SpineRole::Asset)
        .map(|r| r.rect.center())
        .collect();
    let mut other = None;
    for at in assets {
        win.click(at);
        if !win.app.graph_on_canvas() {
            assert!(win.graph_to_canvas());
        }
        if let Some(FocusedNode::Table(node)) = win.app.node_in_focus() {
            if node != table {
                other = Some(node);
                break;
            }
        }
    }
    let other = other.expect("no asset beside the table could be put in focus");
    win.press("cmd-alt-g");
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::View {
            node: other,
            view: NodeView::Grid,
        }
    );
    let sentence = "There is no chart to show for this node yet.";
    let (choices, _) = win
        .app
        .canvas_header()
        .activities
        .expect("the grid's header offers its activities");
    let chart = choices
        .iter()
        .find(|c| c.label == "Chart")
        .expect("a table is offered Chart");
    assert_eq!(chart.unavailable.as_deref(), Some(sentence));

    // A click on the greyed option performs nothing.
    let holds = win.app.canvas_holds().clone();
    let switcher = win.app.pane_header().and_then(|h| h.switcher).unwrap();
    win.applied.clear();
    win.click(egui::pos2(
        switcher.right() - switcher.width() / 4.0,
        switcher.center().y,
    ));
    assert!(win.applied.is_empty(), "{:?}", win.applied);
    assert_eq!(win.app.canvas_holds(), &holds, "the canvas moved");
}

#[test]
fn the_headers_right_end_says_the_nodes_kind_and_size() {
    let mut win = scenes::data_file();
    win.press("cmd-alt-g");
    let rows = win
        .app
        .chart_doc()
        .composed
        .rows
        .map(|rows| rows.total)
        .expect("the data file's rows are counted");
    let note = win
        .app
        .canvas_header()
        .size_note
        .expect("the grid's header says the table's size");
    assert_eq!(
        note,
        format!(
            "table \u{b7} {} rows",
            brightfield_shell::column_header::thousands(rows)
        )
    );
    let drawn = win.app.pane_header().cloned().expect("the pane header");
    let at = drawn.size_note.expect("the note was drawn");
    let switcher = drawn.switcher.expect("the switcher was drawn");
    assert!(
        at.left() > switcher.right(),
        "the note is not at the right end"
    );
    let pane = win.app.canvas_panes().panes[0];
    assert!(
        (pane.header.right() - at.right()) < 24.0,
        "the note is not at the header's right end: {at:?} in {:?}",
        pane.header
    );
}

#[test]
fn the_open_header_is_two_rows() {
    let win = scenes::data_file();
    let title = win
        .app
        .region_rect(arrangement::TITLE_BAND)
        .expect("the title band drew");
    // Nothing is drawn between the title band and the canvas, the rails and
    // the panes: the band that carried the breadcrumb is gone, and the pane
    // header carries it.
    for region in [
        arrangement::CANVAS,
        arrangement::NAVIGATOR_RAIL,
        arrangement::INSPECTOR_RAIL,
    ] {
        let rect = win.app.region_rect(region).expect("the region drew");
        assert!(
            (rect.top() - title.bottom()).abs() < 1.0,
            "{region:?} starts at {} below a title band ending at {}",
            rect.top(),
            title.bottom()
        );
    }
    assert!(
        win.app
            .drawn_regions()
            .iter()
            .all(|(id, _)| id.as_str() != "locator-band"),
        "a locator band was drawn"
    );
}
