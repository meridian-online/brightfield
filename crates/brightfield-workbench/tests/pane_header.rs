//! The pane header: the breadcrumb in place of the title, the activity
//! switcher after it, and the node's kind and size at the right end, asserted
//! by running real frames and reading back what the header reports.

use brightfield_keys::BindingContext;
use brightfield_workbench::chrome::{self, HeaderDrawn};
use brightfield_workbench::subject::{ActivityChoice, Crumb};
use brightfield_workbench::{Icon, Mode, Subject, Verb};

fn screen(width: f32) -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 200.0))
}

fn choices(chart_reason: Option<&str>) -> Vec<ActivityChoice> {
    vec![
        ActivityChoice {
            label: "Grid",
            verb: Verb::new("show-grid"),
            keystroke: Some("G".to_string()),
            unavailable: None,
        },
        ActivityChoice {
            label: "Chart",
            verb: Verb::new("show-chart"),
            keystroke: Some("C".to_string()),
            unavailable: chart_reason.map(str::to_string),
        },
    ]
}

fn node_subject(chart_reason: Option<&str>) -> Subject {
    Subject::new("Grid", Icon("table"), BindingContext::Workspace)
        .with_breadcrumb(vec![Crumb::new("rank"), Crumb::new("ranking")])
        .with_activities(choices(chart_reason), 0)
        .with_size_note("table \u{b7} 19,028 rows")
}

/// One context run over several frames, each frame's events in turn, handing
/// back what the header reported on each.
fn frames(width: f32, subject: &Subject, events: Vec<Vec<egui::Event>>) -> Vec<HeaderDrawn> {
    let ctx = egui::Context::default();
    let mut drawn = Vec::new();
    for (i, events) in events.into_iter().enumerate() {
        let input = egui::RawInput {
            screen_rect: Some(screen(width)),
            time: Some(i as f64 / 60.0),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| {
            drawn.push(chrome::pane_frame_drawn(ui, subject, true, Mode::Light).1);
        });
    }
    drawn
}

fn click_at(at: egui::Pos2) -> Vec<Vec<egui::Event>> {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    vec![
        vec![egui::Event::PointerMoved(at)],
        vec![button(true)],
        vec![button(false)],
    ]
}

#[test]
fn the_header_draws_the_crumbs_in_place_of_the_title_then_the_switcher_then_the_size() {
    let drawn = frames(600.0, &node_subject(None), vec![vec![]]);
    let header = &drawn[0];
    let labels: Vec<&str> = header.crumbs.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(labels, ["rank", "ranking"]);
    let switcher = header.switcher.expect("the switcher was drawn");
    let note = header.size_note.expect("the size note was drawn");
    assert!(header.crumbs[1].1.right() < switcher.left());
    assert!(switcher.right() < note.left());
    assert!(
        note.right() <= 600.0 && note.right() > 560.0,
        "the note is not at the right end: {note:?}"
    );
    assert!(
        switcher.height() <= chrome::header_band_height(),
        "the switcher is taller than the header it sits in"
    );
}

#[test]
fn a_header_with_no_crumbs_draws_its_title_and_no_switcher() {
    let subject = Subject::new("Rows", Icon("list"), BindingContext::Workspace);
    let drawn = frames(600.0, &subject, vec![vec![]]);
    assert!(drawn[0].crumbs.is_empty());
    assert_eq!(drawn[0].switcher, None);
    assert_eq!(drawn[0].size_note, None);
}

#[test]
fn a_click_on_an_activity_reports_its_verb() {
    let subject = node_subject(None);
    let switcher = frames(600.0, &subject, vec![vec![]])[0].switcher.unwrap();
    let chart = egui::pos2(switcher.right() - 4.0, switcher.center().y);
    let mut events = vec![vec![]];
    events.extend(click_at(chart));
    let drawn = frames(600.0, &subject, events);
    let activities: Vec<Option<Verb>> = drawn.iter().map(|d| d.activity).collect();
    assert!(
        activities.contains(&Some(Verb::new("show-chart"))),
        "a click on Chart reported {activities:?}"
    );
}

#[test]
fn a_click_on_an_unavailable_activity_reports_nothing() {
    let subject = node_subject(Some("There is no chart to show for this node yet."));
    let switcher = frames(600.0, &subject, vec![vec![]])[0].switcher.unwrap();
    let chart = egui::pos2(switcher.right() - 4.0, switcher.center().y);
    let mut events = vec![vec![]];
    events.extend(click_at(chart));
    let drawn = frames(600.0, &subject, events);
    assert!(
        drawn.iter().all(|d| d.activity.is_none()),
        "a greyed option reported a click"
    );
}

#[test]
fn a_linked_crumb_reports_its_click_and_a_plain_one_has_no_link() {
    let plain = node_subject(None);
    assert_eq!(frames(600.0, &plain, vec![vec![]])[0].crumb_link, None);

    let subject =
        Subject::new("Chart", Icon("chart"), BindingContext::Workspace).with_breadcrumb(vec![
            Crumb::new("Protocol \u{b7} co2").linked("Back to every panel"),
            Crumb::new("change"),
        ]);
    let (link, _) = frames(600.0, &subject, vec![vec![]])[0]
        .crumb_link
        .expect("the first crumb is a link");
    let mut events = vec![vec![]];
    events.extend(click_at(link.center()));
    let drawn = frames(600.0, &subject, events);
    assert!(
        drawn
            .iter()
            .any(|d| d.crumb_link.is_some_and(|(_, clicked)| clicked)),
        "the link's click was not reported"
    );
}

#[test]
fn a_narrow_header_drops_the_size_note_before_it_touches_the_switcher() {
    let drawn = frames(260.0, &node_subject(None), vec![vec![]]);
    let header = &drawn[0];
    let switcher = header.switcher.expect("the switcher keeps its place");
    if let Some(note) = header.size_note {
        assert!(
            note.left() > switcher.right(),
            "the note {note:?} overlaps the switcher {switcher:?}"
        );
    }
    assert_eq!(header.size_note, None, "a 260-point header has no room");
}

#[test]
fn the_switchers_options_tile_it_left_to_right() {
    let header = &frames(600.0, &node_subject(None), vec![vec![]])[0];
    let switcher = header.switcher.expect("the switcher was drawn");
    let labels: Vec<&str> = header.options.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels, ["Grid", "Chart"]);
    let (grid, chart) = (header.options[0].1, header.options[1].1);
    assert!(
        grid.right() <= chart.left() + 0.5,
        "{grid:?} runs into {chart:?}"
    );
    for rect in [grid, chart] {
        assert!(
            switcher.expand(0.5).contains_rect(rect),
            "the option {rect:?} is outside the switcher {switcher:?}"
        );
    }
}

#[test]
fn a_long_crumb_is_cut_short_before_the_switcher() {
    let subject = Subject::new("Map", Icon("map"), BindingContext::Workspace)
        .with_breadcrumb(vec![
            Crumb::new("california_housing_sample"),
            Crumb::new("dashboard"),
            Crumb::new(
                "Map \u{b7} latitude \u{d7} longitude \u{b7} equirectangular as saved 14:02",
            ),
        ])
        .with_activities(choices(None), 0);
    let header = &frames(420.0, &subject, vec![vec![]])[0];
    let switcher = header.switcher.expect("the switcher keeps its place");
    assert!(
        switcher.right() <= 420.0,
        "the switcher {switcher:?} runs off the header"
    );
    for (label, rect) in &header.crumbs {
        assert!(
            rect.right() < switcher.left(),
            "the crumb {label:?} at {rect:?} runs into the switcher {switcher:?}"
        );
    }
}

#[test]
fn the_general_crumbs_give_way_before_the_last_one() {
    let title = "Map \u{b7} latitude \u{d7} longitude \u{b7} as saved 14:02";
    let subject = Subject::new("Map", Icon("map"), BindingContext::Workspace)
        .with_breadcrumb(vec![
            Crumb::new("california_housing_sample"),
            Crumb::new("dashboard"),
            Crumb::new(title),
        ])
        .with_activities(choices(None), 0);
    let wide = &frames(1200.0, &subject, vec![vec![]])[0];
    let title_width = wide.crumbs[2].1.width();
    let narrow = &frames(560.0, &subject, vec![vec![]])[0];
    assert_eq!(narrow.crumbs.len(), 3, "a crumb was left out");
    assert!(
        narrow.crumbs[0].1.width() < wide.crumbs[0].1.width(),
        "the most general crumb kept its width"
    );
    assert!(
        (narrow.crumbs[2].1.width() - title_width).abs() < 0.5,
        "the last crumb was cut while a general one could still give way"
    );
}

#[test]
fn a_narrow_header_leaves_the_general_crumbs_out_before_it_cuts_the_last() {
    let title = "Map \u{b7} latitude \u{d7} longitude \u{b7} as closed unsaved 14:02";
    let subject = Subject::new("Map", Icon("map"), BindingContext::Workspace)
        .with_breadcrumb(vec![
            Crumb::new("california_housing_sample"),
            Crumb::new("dashboard"),
            Crumb::new(title),
        ])
        .with_activities(choices(None), 0);
    let wide = &frames(1200.0, &subject, vec![vec![]])[0];
    let title_width = wide.crumbs[2].1.width();
    let narrow = &frames(title_width + 180.0, &subject, vec![vec![]])[0];
    let last = narrow.crumbs.last().expect("the last crumb was drawn");
    assert_eq!(last.0, title);
    assert!(
        (last.1.width() - title_width).abs() < 0.5,
        "the last crumb was cut while a general one could be left out: {narrow:?}"
    );
    assert!(
        narrow.crumbs.len() < 3,
        "every general crumb was kept: {narrow:?}"
    );
}
