//! The ribbon: its tabs in the title band, the selected tab's commands in a band of their own
//! under it, and the sentence a greyed command says, whether the pointer is over it or its key was
//! pressed.
//!
//! The ribbon's taxonomy names only icons the design system carries, so no command on a stage's
//! tab or a contextual tab is drawn without its glyph. The operation forms' icons are checked
//! where the forms join the ribbon, in `brightfield_shell::ribbon`'s own tests.

#[path = "support/scenes.rs"]
mod scenes;

use brightfield_keys::ribbon::{context_groups, stage_groups, Context, Stage};
use brightfield_keys::{registry, Availability, BindingContext, VerbStatus};
use brightfield_shell::ribbon::RibbonPick;
use brightfield_shell::window::RUN_PROTOCOL;
use brightfield_workbench::arrangement::{RIBBON_BAND, RIBBON_BAND_HEIGHT, TITLE_BAND};
use meridian_egui::Icon;
use scenes::Window;

#[test]
fn every_ribbon_command_names_an_icon_the_design_system_carries() {
    let groups = Stage::ALL
        .into_iter()
        .flat_map(|stage| stage_groups(stage).iter())
        .chain(
            Context::ALL
                .into_iter()
                .flat_map(|context| context_groups(context).iter()),
        );
    let mut checked = 0;
    let mut missing = Vec::new();
    for group in groups {
        for entry in group.iter() {
            checked += 1;
            if Icon::by_name(entry.icon).is_none() {
                missing.push(format!("{} names {:?}", entry.longname, entry.icon));
            }
        }
    }
    assert!(checked > 0, "the taxonomy lists no commands to check");
    assert!(
        missing.is_empty(),
        "the design system's icon manifest carries none of these: {missing:?}"
    );
}

/// Where the last frame drew the tab `pick`.
fn tab(win: &Window, pick: RibbonPick) -> egui::Rect {
    win.app
        .ribbon_tab_rects()
        .iter()
        .find(|(p, _)| *p == pick)
        .map(|(_, rect)| *rect)
        .unwrap_or_else(|| panic!("the title band drew no {pick:?} tab"))
}

/// Where the last frame drew the command `longname` on the ribbon's row.
fn command(win: &Window, longname: &str) -> Option<egui::Rect> {
    win.app
        .ribbon_command_rects()
        .iter()
        .find(|(name, _)| *name == longname)
        .map(|(_, rect)| *rect)
}

/// Whether the last frame drew `text` anywhere.
fn on_screen(win: &Window, text: &str) -> bool {
    win.drawn.iter().any(|(_, drawn)| drawn.contains(text))
}

/// The sentence `longname` says in this window, which the test expects it cannot act in.
fn missing(win: &Window, longname: &str) -> String {
    match win.app.availability_of(longname) {
        Some(Availability::Missing(sentence)) => sentence,
        other => panic!("{longname} was expected greyed here and reads {other:?}"),
    }
}

#[test]
fn the_command_row_draws_under_the_title_band_at_its_declared_height() {
    let win = scenes::data_file();
    let title = win.app.region_rect(TITLE_BAND).expect("the title band");
    let row = win.app.region_rect(RIBBON_BAND).expect("the ribbon band");
    assert!(
        (row.height() - RIBBON_BAND_HEIGHT).abs() < 1e-3,
        "the ribbon band drew {}pt tall against the {RIBBON_BAND_HEIGHT}pt declared",
        row.height()
    );
    assert!(
        (row.top() - title.bottom()).abs() < 1e-3,
        "the ribbon band starts at {} and the title band ends at {}",
        row.top(),
        title.bottom()
    );
    assert!(
        !win.app.ribbon_command_rects().is_empty(),
        "the Shape tab listed no command"
    );
    for (longname, rect) in win.app.ribbon_command_rects() {
        assert!(
            row.contains_rect(*rect),
            "{longname} at {rect:?} left {row:?}"
        );
    }
}

#[test]
fn the_front_door_draws_no_ribbon() {
    let win = scenes::door();
    assert_eq!(win.app.region_rect(RIBBON_BAND), None);
    assert!(win.app.ribbon_tab_rects().is_empty());
}

#[test]
fn clicking_a_stage_tab_shows_its_commands() {
    let mut win = scenes::data_file();
    assert_eq!(command(&win, "copy-chart-picture"), None);
    win.click(tab(&win, RibbonPick::Stage(Stage::Publish)).center());
    assert_eq!(win.app.ribbon().selected, RibbonPick::Stage(Stage::Publish));
    assert!(
        command(&win, "copy-chart-picture").is_some(),
        "the Publish tab drew {:?}",
        win.app.ribbon_command_rects()
    );
}

#[test]
fn clicking_a_tab_while_the_ribbon_is_put_away_brings_it_back() {
    let mut win = scenes::data_file();
    win.press("alt-r");
    assert!(win.app.ribbon().collapsed);
    assert_eq!(
        win.app.region_rect(RIBBON_BAND),
        None,
        "the band is put away"
    );
    win.click(tab(&win, RibbonPick::Stage(Stage::Publish)).center());
    assert!(
        !win.app.ribbon().collapsed,
        "the click left the ribbon away"
    );
    assert_eq!(win.app.ribbon().selected, RibbonPick::Stage(Stage::Publish));
    assert!(win.app.region_rect(RIBBON_BAND).is_some());
    assert!(command(&win, "copy-chart-picture").is_some());
}

#[test]
fn a_tab_the_narrow_band_cuts_away_is_no_control_and_no_anchor() {
    let mut win = scenes::data_file();
    win.screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(560.0, 820.0));
    win.settle();
    let band = win.app.region_rect(TITLE_BAND).expect("the title band");
    let run = win.app.run_control_rect().expect("Run");
    let tabs = win.app.ribbon_tab_rects().to_vec();
    assert!(tabs.len() < 5, "the narrow band cut no tab: {tabs:?}");
    for (pick, rect) in &tabs {
        assert!(
            band.contains_rect(*rect),
            "{pick:?} at {rect:?} left the band"
        );
        assert!(
            rect.right() <= run.left(),
            "{pick:?} at {rect:?} reaches Run"
        );
    }
}

#[test]
fn clicking_a_command_performs_it() {
    let mut win = scenes::data_file();
    win.click(tab(&win, RibbonPick::Stage(Stage::Publish)).center());
    let copy = command(&win, "copy-chart-picture").expect("Publish lists Copy picture");
    win.applied.clear();
    win.click(copy.center());
    assert!(
        win.applied.contains(&"copy-chart-picture"),
        "the click applied {:?}",
        win.applied
    );
    assert!(
        win.commands
            .iter()
            .any(|c| matches!(c, egui::OutputCommand::CopyImage(_))),
        "the chart's picture did not reach the clipboard: {}",
        win.said()
    );
}

#[test]
fn the_contextual_tab_is_named_for_the_activity_and_the_node() {
    let mut win = scenes::data_file();
    win.press("cmd-alt-g");
    let node = win
        .app
        .header_crumbs()
        .last()
        .cloned()
        .expect("the grid's header names its node");
    let want = format!("Grid \u{b7} {node}");
    win.click(tab(&win, RibbonPick::Context).center());
    assert_eq!(win.app.ribbon().selected, RibbonPick::Context);
    let context = tab(&win, RibbonPick::Context);
    assert!(
        win.app
            .named_controls()
            .iter()
            .any(|c| c.name == want && c.rect == context),
        "no tab is named {want:?}"
    );
    assert!(
        command(&win, "filter-rows-at-cursor").is_some(),
        "the grid's tab leads with the forms its foot row asks"
    );
}

#[test]
fn pointing_at_a_greyed_command_opens_its_reason() {
    let mut win = scenes::data_file();
    win.click(tab(&win, RibbonPick::Stage(Stage::Model)).center());
    let sentence = missing(&win, "relate");
    let relate = command(&win, "relate").expect("Model lists Relate");
    assert_eq!(win.app.ribbon_reason_rect(), None);
    win.run(vec![vec![egui::Event::PointerMoved(relate.center())]]);
    win.settle();
    let shown = win
        .app
        .ribbon_reason_rect()
        .expect("the reason popover opened");
    assert!(shown.top() >= relate.bottom(), "{shown:?} under {relate:?}");
    assert!(on_screen(&win, &sentence), "{sentence:?} is not on screen");
}

#[test]
fn pressing_a_greyed_commands_key_shows_the_same_sentence() {
    let mut win = scenes::live_chart("crossfilter.yaml");
    let sentence = missing(&win, RUN_PROTOCOL);
    let run = win.app.run_control_rect().expect("the title band drew Run");
    win.press("cmd-enter");
    assert!(
        !win.answered.contains(&RUN_PROTOCOL),
        "a refused key is not answered"
    );
    assert_eq!(
        win.app.ribbon_reason().map(|r| r.longname),
        Some(RUN_PROTOCOL)
    );
    let shown = win.app.ribbon_reason_rect().expect("the sentence is up");
    assert!(
        shown.top() >= run.bottom(),
        "{shown:?} under Run at {run:?}"
    );
    assert!(on_screen(&win, &sentence), "{sentence:?} is not on screen");
}

#[test]
fn a_greyed_commands_key_still_says_why_when_its_tab_is_not_open() {
    let mut win = scenes::data_file();
    assert_eq!(win.app.ribbon().selected, RibbonPick::Stage(Stage::Shape));
    let sentence = missing(&win, "relate");
    win.press("cmd-alt-m");
    assert!(!win.answered.contains(&"relate"));
    let model = tab(&win, RibbonPick::Stage(Stage::Model));
    let shown = win.app.ribbon_reason_rect().expect("the sentence is up");
    assert!(
        shown.top() >= model.bottom() && (shown.left() - model.left()).abs() < 1.0,
        "{shown:?} is not under the Model tab at {model:?}"
    );
    assert!(on_screen(&win, &sentence), "{sentence:?} is not on screen");
}

#[test]
fn a_refused_keys_sentence_comes_down_after_a_while() {
    let mut win = scenes::data_file();
    win.press("cmd-alt-m");
    assert!(win.app.ribbon_reason().is_some());
    // Five seconds of frames at sixty a second.
    win.run(vec![Vec::new(); 300]);
    assert_eq!(win.app.ribbon_reason(), None);
    assert_eq!(win.app.ribbon_reason_rect(), None);
}

#[test]
fn every_global_key_of_a_greyed_command_says_its_sentence() {
    let mut swept = 0;
    for verb in registry()
        .iter()
        .filter(|v| v.status == VerbStatus::Built && !v.is_reserved())
    {
        for binding in verb
            .binding_specs
            .iter()
            .filter(|b| b.context == BindingContext::Global)
        {
            let mut win = scenes::door();
            let Some(Availability::Missing(sentence)) = win.app.availability_of(verb.longname)
            else {
                continue;
            };
            swept += 1;
            win.press(binding.keystrokes);
            assert!(
                !win.answered.contains(&verb.longname),
                "{} performed {} on the front door, where it cannot act",
                binding.keystrokes,
                verb.longname
            );
            assert!(
                on_screen(&win, &sentence),
                "{} for {} left {sentence:?} off the screen: {}",
                binding.keystrokes,
                verb.longname,
                win.said()
            );
        }
    }
    assert!(swept > 0, "no Global key is greyed on the front door");
}

#[test]
fn at_the_narrow_window_the_title_band_keeps_run_and_every_tab_whole() {
    let mut win = scenes::data_file();
    win.screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0));
    win.settle();
    let band = win.app.region_rect(TITLE_BAND).expect("the title band");
    let mark = win.app.mark_rect().expect("the mark");
    let run = win.app.run_control_rect().expect("Run");
    let tabs = win.app.ribbon_tab_rects().to_vec();
    assert_eq!(
        tabs.len(),
        5,
        "four stages and the contextual tab: {tabs:?}"
    );
    let mut drawn = vec![("the mark".to_owned(), mark), ("Run".to_owned(), run)];
    drawn.extend(tabs.iter().map(|(pick, rect)| (format!("{pick:?}"), *rect)));
    for (i, (a, ra)) in drawn.iter().enumerate() {
        assert!(
            band.contains_rect(*ra),
            "{a} at {ra:?} left the band {band:?}"
        );
        for (b, rb) in &drawn[i + 1..] {
            // Tabs abut, so a shared edge is not an overlap.
            let shared = ra.intersect(*rb);
            assert!(
                shared.width() <= 0.0 || shared.height() <= 0.0,
                "{a} at {ra:?} and {b} at {rb:?} overlap"
            );
        }
    }
}
