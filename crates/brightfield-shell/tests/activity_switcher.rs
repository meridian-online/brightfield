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
    assert_eq!(
        win.app.protocol_model().charts_reading(&table),
        vec![chart.clone()]
    );
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
    assert_eq!(win.app.node_in_focus(), Some(on_chart.clone()));
    for node in [on_chart, FocusedNode::Table(table)] {
        let offered = offers(&node, &win.app);
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
    win.press("cmd-alt-g");
    assert_eq!(win.app.canvas_holds().view(), Some(NodeView::Grid));
}

#[test]
fn chart_over_a_node_no_chart_reads_is_greyed_and_says_so() {
    let win = scenes::data_file();
    let (table, _) = table_and_chart(&win);
    let model = win.app.protocol_model();
    let other = model
        .spine()
        .into_iter()
        .filter(|row| row.role == SpineRole::Asset)
        .filter_map(|row| row.id)
        .find(|id| *id != table && model.charts_reading(id).is_empty())
        .expect("the data file's Protocol lists its source beside its table");
    let label = model.label_of(&other).unwrap_or(&other).to_owned();
    let offered = offers(&FocusedNode::Table(other), &win.app);
    let chart = offered
        .iter()
        .find(|o| o.activity == NodeActivity::Chart)
        .expect("a table is offered Chart");
    assert_eq!(
        chart.unavailable.as_deref(),
        Some(format!("No chart reads {label} yet.").as_str())
    );
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
    let offered = offers(&FocusedNode::Model, &win.app);
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
