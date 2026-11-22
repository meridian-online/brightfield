//! What the window holds, as a command's declared needs read it: the
//! situation each arrangement of the window reports, and the availability a
//! command reads from it.

#[path = "support/scenes.rs"]
mod scenes;

use brightfield_keys::{Availability, Condition, Selected, Situation};

fn holds(situation: &Situation, selected: &[Selected], conditions: &[Condition]) {
    for what in selected {
        assert!(
            situation.selected.contains(what),
            "{what:?} is not held: {situation:?}"
        );
    }
    for condition in conditions {
        assert!(
            situation.conditions.contains(condition),
            "{condition:?} does not hold: {situation:?}"
        );
    }
}

#[test]
fn the_front_door_holds_no_protocol() {
    let win = scenes::door();
    let situation = win.app.situation();
    assert_eq!(situation.selected, Vec::new(), "{situation:?}");
    assert_eq!(situation.sources, 0);
    assert_eq!(situation.column, None);
    for condition in [
        Condition::ProtocolToRun,
        Condition::Trusted,
        Condition::UnsavedWork,
        Condition::SelectionHeld,
    ] {
        assert!(
            !situation.conditions.contains(&condition),
            "{condition:?} holds on the front door: {situation:?}"
        );
    }
    assert_eq!(
        win.app.availability_of("open-home"),
        Some(Availability::Missing(
            "You are on the front door already.".to_owned()
        ))
    );
    assert_eq!(
        win.app.availability_of("open"),
        Some(Availability::Available)
    );
    assert_eq!(win.app.availability_of("no-such-command"), None);
}

#[test]
fn a_data_file_protocol_holds_a_chart_its_rows_and_its_source() {
    let win = scenes::data_file();
    let situation = win.app.situation();
    holds(
        &situation,
        &[Selected::Protocol, Selected::Chart, Selected::Rows],
        &[
            Condition::ProtocolToRun,
            Condition::Trusted,
            Condition::TakesSources,
            Condition::FileToReload,
            Condition::ShelfDrawn,
        ],
    );
    assert!(!situation.selected.contains(&Selected::Graph));
    assert!(!situation.selected.contains(&Selected::Model));
    assert_eq!(situation.sources, 1);
    for verb in ["run-protocol", "zoom-in", "reload-data", "open-home"] {
        assert_eq!(
            win.app.availability_of(verb),
            Some(Availability::Available),
            "{verb}"
        );
    }
    assert_eq!(
        win.app.availability_of("organise-joins"),
        Some(Availability::Missing(
            "Organising joins needs a Protocol of two sources or more.".to_owned()
        ))
    );
}

#[test]
fn the_crosswalk_puts_its_graph_on_the_canvas() {
    let win = scenes::crosswalk();
    let situation = win.app.situation();
    holds(&situation, &[Selected::Protocol, Selected::Graph], &[]);
    assert!(
        !situation.selected.contains(&Selected::Chart),
        "{situation:?}"
    );
    assert_eq!(
        win.app.availability_of("zoom-in"),
        Some(Availability::Missing(
            "Zoom in needs a chart on the canvas.".to_owned()
        ))
    );
    assert_eq!(
        win.app.availability_of("focus-jump"),
        Some(Availability::Available)
    );
    // A shipped start has no folder to stop trusting.
    assert_eq!(
        win.app.availability_of("untrust-protocol"),
        Some(Availability::Missing(
            "Stop trusting needs a Protocol opened from a folder.".to_owned()
        ))
    );
    assert_eq!(
        scenes::data_file().app.availability_of("untrust-protocol"),
        Some(Availability::Available)
    );
}

#[test]
fn an_own_files_protocol_of_two_tables_has_two_sources_and_the_model() {
    let win = scenes::own_files();
    let situation = win.app.situation();
    holds(
        &situation,
        &[Selected::Protocol, Selected::Model],
        &[Condition::ProtocolToRun, Condition::TakesSources],
    );
    assert_eq!(situation.sources, 2, "{situation:?}");
    assert!(
        !situation.selected.contains(&Selected::Chart),
        "the model pane holds the canvas: {situation:?}"
    );
    assert_eq!(
        win.app.availability_of("cycle-model-view"),
        Some(Availability::Available)
    );
    // Run acts on the model pane's Protocol, which has no data file's source.
    assert_eq!(
        win.app.availability_of("run-protocol"),
        Some(Availability::Available)
    );
}

/// A selection held on the chart is a condition Clear selection waits on. The
/// kept column Undo waits on is read where the shelf's own tests keep one
/// (`shelf_column_route.rs`'s `a_kept_column_is_a_condition`).
#[test]
fn a_held_selection_is_a_condition() {
    use brightfield_engine::coordinator::Interaction;
    use brightfield_engine::SqlPredicate;
    use brightfield_spec::analysis::ComponentPath;

    let mut win = scenes::live_chart("crossfilter.yaml");
    let before = win.app.situation();
    holds(&before, &[Selected::Protocol, Selected::Chart], &[]);
    assert!(!before.conditions.contains(&Condition::SelectionHeld));
    assert_eq!(
        win.app.availability_of("clear-selection"),
        Some(Availability::Missing(
            "Clear selection needs a selection on the chart.".to_owned()
        ))
    );
    // A chart spec has no Protocol for Save or Run to act on.
    assert!(!before.conditions.contains(&Condition::ProtocolToRun));

    assert!(
        win.app
            .chart_doc_mut()
            .apply_interaction(Interaction::Select {
                name: "brush".to_string(),
                contributor: ComponentPath("root/hconcat[0]".to_string()),
                predicate: SqlPredicate::Expr("temp > 5".to_string()),
            }),
        "the fixture accepted a selection"
    );
    assert!(
        win.app
            .situation()
            .conditions
            .contains(&Condition::SelectionHeld),
        "{:?}",
        win.app.situation()
    );
    assert_eq!(
        win.app.availability_of("clear-selection"),
        Some(Availability::Available)
    );
}
