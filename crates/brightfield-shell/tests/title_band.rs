//! The unified title band: it leaves the window's own controls their corner on macOS, where the
//! window draws under a hidden title bar; there its empty stretch moves the window when dragged and
//! zooms it when double-clicked, as a title bar does, and elsewhere it leaves both to the window's
//! own title bar; it keeps the open Protocol's name and the folder it is kept in; and it carries
//! Run, with its key and, where it cannot act, the sentence that says why, and the activity area,
//! which says what the Protocol's runs have done. A run under way, and the runs that land, are
//! driven in `run_control.rs`, which has a runner to give a window.

#[path = "support/scenes.rs"]
mod scenes;

use brightfield_keys::{keys_label, Availability, KeyPlatform};
use brightfield_shell::menu_bar::MenuHost;
use brightfield_shell::window::{ACTIVITY_CONTROL_NAME, RUN_LABEL, RUN_PROTOCOL};
use brightfield_workbench::arrangement::{LEDGER_RAIL, TITLE_BAND};
use scenes::Window;

/// A point on the title band's empty stretch: left of the Protocol's name field, clear of the
/// name the band leads with.
fn on_the_stretch(win: &Window) -> egui::Pos2 {
    let band = win
        .app
        .region_rect(TITLE_BAND)
        .expect("the title band was drawn");
    let place = win
        .app
        .title_place_rects()
        .expect("the title band drew the Protocol's place");
    let name = place.name.expect("the name fits a wide window");
    egui::pos2(name.left() - 24.0, band.center().y)
}

fn button(at: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn started_a_drag(win: &Window) -> bool {
    win.viewport_commands
        .iter()
        .any(|c| matches!(c, egui::ViewportCommand::StartDrag))
}

fn zoomed(win: &Window) -> bool {
    win.viewport_commands
        .iter()
        .any(|c| matches!(c, egui::ViewportCommand::Maximized(true)))
}

#[cfg(target_os = "macos")]
#[test]
fn the_title_band_leaves_the_window_controls_their_corner() {
    use brightfield_workbench::arrangement::WINDOW_CONTROLS_INSET;
    let mut win = scenes::data_file();
    // The mark is drawn off the door on either host; on a host with no menu bar it opens the
    // menus, and it stands in the same place.
    let home = win.app.mark_rect().expect("the title band drew the mark");
    assert!(
        home.left() >= WINDOW_CONTROLS_INSET,
        "the mark at {home:?} sits under the window's controls, which end at {WINDOW_CONTROLS_INSET}"
    );
    win.app.set_menu_host(MenuHost::Mark);
    win.settle();
    let mark = win.app.mark_rect().expect("the title band drew the mark");
    assert!(mark.left() >= WINDOW_CONTROLS_INSET, "{mark:?}");
}

#[cfg(not(target_os = "macos"))]
#[test]
fn where_the_window_keeps_its_title_bar_the_band_moves_nothing() {
    let mut win = scenes::data_file();
    let at = on_the_stretch(&win);
    win.viewport_commands.clear();
    win.run(vec![
        vec![egui::Event::PointerMoved(at), button(at, true)],
        vec![egui::Event::PointerMoved(at + egui::vec2(40.0, 4.0))],
        vec![button(at + egui::vec2(40.0, 4.0), false)],
        vec![button(at + egui::vec2(40.0, 4.0), true)],
        vec![button(at + egui::vec2(40.0, 4.0), false)],
    ]);
    assert!(
        !started_a_drag(&win) && !zoomed(&win),
        "{:?}",
        win.viewport_commands
    );
}

#[test]
fn the_inset_is_the_window_controls_corner_on_macos_and_nothing_elsewhere() {
    let inset = brightfield_shell::title_band::controls_inset();
    if cfg!(target_os = "macos") {
        // The close, minimise and zoom buttons end 69 points in.
        assert!(inset > 69.0, "{inset}");
    } else {
        assert_eq!(inset, 0.0);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn a_drag_on_the_title_bands_empty_stretch_moves_the_window() {
    let mut win = scenes::data_file();
    let at = on_the_stretch(&win);
    win.viewport_commands.clear();
    win.run(vec![
        vec![egui::Event::PointerMoved(at), button(at, true)],
        vec![egui::Event::PointerMoved(at + egui::vec2(12.0, 0.0))],
        vec![egui::Event::PointerMoved(at + egui::vec2(40.0, 4.0))],
        vec![button(at + egui::vec2(40.0, 4.0), false)],
    ]);
    assert!(
        started_a_drag(&win),
        "a drag on the band asked the window to move: {:?}",
        win.viewport_commands
    );
    assert!(!zoomed(&win), "a drag is not a zoom");
}

#[cfg(target_os = "macos")]
#[test]
fn a_double_click_on_the_title_bands_empty_stretch_zooms_the_window() {
    let mut win = scenes::data_file();
    let at = on_the_stretch(&win);
    win.viewport_commands.clear();
    win.run(vec![
        vec![egui::Event::PointerMoved(at), button(at, true)],
        vec![button(at, false)],
        vec![button(at, true)],
        vec![button(at, false)],
    ]);
    assert!(
        zoomed(&win),
        "a double-click on the band zoomed the window: {:?}",
        win.viewport_commands
    );
    assert!(
        !started_a_drag(&win),
        "a click that does not move is no drag"
    );
}

#[test]
fn a_press_on_the_mark_or_the_name_field_moves_nothing() {
    let mut win = scenes::data_file();
    let mark = win.app.mark_rect().expect("the mark");
    let name = win
        .app
        .title_place_rects()
        .and_then(|p| p.name)
        .expect("the name field");
    for at in [mark.center(), name.right_center() - egui::vec2(4.0, 0.0)] {
        win.viewport_commands.clear();
        win.run(vec![
            vec![egui::Event::PointerMoved(at), button(at, true)],
            vec![egui::Event::PointerMoved(at + egui::vec2(30.0, 0.0))],
            vec![button(at + egui::vec2(30.0, 0.0), false)],
        ]);
        assert!(
            !started_a_drag(&win),
            "a press at {at:?} on a control moved the window"
        );
        win.key(egui::Key::Escape);
    }
}

#[test]
fn the_protocols_name_and_place_are_kept_in_the_title_band() {
    let win = scenes::data_file();
    let band = win.app.region_rect(TITLE_BAND).expect("the title band");
    let place = win
        .app
        .title_place_rects()
        .expect("the title band drew the Protocol's place");
    for (what, rect) in [("name", place.name), ("location", place.location)] {
        let rect = rect.unwrap_or_else(|| panic!("the {what} field fits a wide window"));
        assert!(
            band.contains_rect(rect),
            "the {what} field at {rect:?} lies outside the title band {band:?}"
        );
    }
    let (name, location) = (place.name.unwrap(), place.location.unwrap());
    assert!(
        location.left() >= name.right(),
        "the name reads before the folder it is kept in: {place:?}"
    );
}

#[test]
fn the_front_door_draws_no_protocols_place() {
    let win = scenes::door();
    assert!(win.app.front_door_is_live());
    assert_eq!(win.app.title_place_rects(), None);
}

#[test]
fn the_pane_header_keeps_the_crumbs_below_the_place() {
    let win = scenes::data_file();
    // A Protocol made from a data file opens on its dashboard, which its pane
    // header names after the table it reads.
    let crumbs = win.app.header_crumbs();
    assert!(crumbs.len() > 1, "the pane header kept a trail: {crumbs:?}");
    assert_eq!(
        crumbs.last().map(String::as_str),
        Some("dashboard"),
        "{crumbs:?}"
    );
    let header = win
        .app
        .pane_header()
        .expect("the canvas drew a pane header");
    let title = win.app.region_rect(TITLE_BAND).expect("the title band");
    for (crumb, rect) in &header.crumbs {
        assert!(
            !title.intersects(*rect),
            "the crumb {crumb:?} at {rect:?} is in the title band {title:?}"
        );
    }
    let place = win.app.title_place_rects().expect("the place");
    assert!(
        [place.name, place.location]
            .into_iter()
            .flatten()
            .all(|rect| title.contains_rect(rect)),
        "the place fields left the title band"
    );
}

#[test]
fn on_a_narrow_window_the_place_fields_keep_off_the_mark() {
    let mut win = scenes::data_file();
    win.screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(560.0, 820.0));
    win.settle();
    let mark = win.app.mark_rect().expect("the mark");
    // Open the Location field, the widest the band draws, if it is still drawn; otherwise the
    // name, through F2.
    match win.app.title_place_rects().and_then(|p| p.location) {
        Some(location) => win.click(location.right_center() - egui::vec2(4.0, 0.0)),
        None => win.key(egui::Key::F2),
    }
    let place = win.app.title_place_rects().expect("the place");
    let drawn: Vec<egui::Rect> = [place.name, place.location, place.choose]
        .into_iter()
        .flatten()
        .collect();
    assert!(!drawn.is_empty(), "an open field is drawn: {place:?}");
    for rect in drawn {
        assert!(
            rect.left() > mark.right(),
            "a place field at {rect:?} covers the mark at {mark:?}"
        );
    }
    // The mark still takes its click: it goes home.
    win.key(egui::Key::Escape);
    win.click(mark.center());
    assert!(win.app.front_door_is_live(), "the mark took the click home");
}

/// Every text the next frame draws inside `rect`, with the pointer at `pointer` when one is given.
fn text_in(win: &mut Window, rect: egui::Rect, pointer: Option<egui::Pos2>) -> Vec<String> {
    win.run(vec![pointer
        .map(egui::Event::PointerMoved)
        .into_iter()
        .collect()]);
    win.drawn
        .iter()
        .filter(|(r, _)| rect.expand(1.0).contains_rect(*r))
        .map(|(_, t)| t.clone())
        .collect()
}

/// **A Protocol's title band offers Run at its right end, with the key that runs it.**
///
/// The key is the one the registry binds, as the platform writes it, so a rebinding cannot leave
/// the control naming a key that no longer runs.
#[test]
fn the_title_band_offers_run_with_its_key() {
    let mut win = scenes::data_file();
    let band = win.app.region_rect(TITLE_BAND).expect("the title band");
    let run = win.app.run_control_rect().expect("the title band drew Run");
    assert!(
        band.contains_rect(run),
        "Run at {run:?} lies outside the band {band:?}"
    );
    let place = win.app.title_place_rects().expect("the place");
    for field in [place.name, place.location].into_iter().flatten() {
        assert!(
            field.right() <= run.left(),
            "the place field at {field:?} runs into Run at {run:?}"
        );
    }
    assert!(
        win.app
            .named_controls()
            .iter()
            .any(|c| c.name == RUN_LABEL && c.rect == run),
        "Run is not among the band's named controls"
    );
    let words = text_in(&mut win, run, None);
    assert!(words.iter().any(|w| w == RUN_LABEL), "Run reads {words:?}");
    let key = keys_label("cmd-enter", KeyPlatform::host());
    assert!(words.contains(&key), "Run shows no {key}: {words:?}");
    assert_eq!(
        win.app.availability_of(RUN_PROTOCOL),
        Some(Availability::Available)
    );
}

/// **The front door draws neither Run nor the activity area**: it holds no Protocol to run.
#[test]
fn the_front_door_draws_no_run_and_no_activity() {
    let win = scenes::door();
    assert_eq!(win.app.run_control_rect(), None);
    assert_eq!(win.app.activity_area_rect(), None);
}

/// **Over a shipped start, Run is greyed, and pointing at it says the sentence its needs
/// declare**, the same sentence the palette and the menus give.
#[test]
fn on_a_shipped_start_run_is_greyed_with_its_declared_sentence() {
    let mut win = scenes::crosswalk();
    let Some(Availability::Missing(sentence)) = win.app.availability_of(RUN_PROTOCOL) else {
        panic!("Run can act over a shipped start");
    };
    let run = win
        .app
        .run_control_rect()
        .expect("the band drew Run, greyed");
    win.run(vec![vec![egui::Event::PointerMoved(run.center())]]);
    let screen = win.screen;
    let words = text_in(&mut win, screen, Some(run.center()));
    assert!(
        words.iter().any(|w| w.contains(&sentence)),
        "pointing at greyed Run did not say {sentence:?}: {words:?}"
    );
    // Taking it runs nothing.
    win.click(run.center());
    assert!(!win.app.run_in_progress(), "a greyed Run started a run");
}

/// **The activity area sits between the Protocol's name and Run, and says when the Protocol last
/// ran**: a data file's Protocol runs as it opens, so it reads today's date and "just now".
#[test]
fn the_activity_area_says_when_the_protocol_last_ran() {
    let mut win = scenes::data_file();
    let area = win
        .app
        .activity_area_rect()
        .expect("the band drew the activity area");
    let band = win.app.region_rect(TITLE_BAND).expect("the title band");
    assert!(
        band.contains_rect(area),
        "the area at {area:?} lies outside the band {band:?}"
    );
    let run = win.app.run_control_rect().expect("Run");
    assert!(
        area.right() <= run.left(),
        "the area at {area:?} runs into Run at {run:?}"
    );
    let words = text_in(&mut win, area, None);
    assert_eq!(
        words,
        vec![
            chrono::Local::now().format("ran %-d %b").to_string(),
            "just now".to_string()
        ]
    );
}

/// **A click on the activity area opens the ledger on its Log**, where the runs it tells of are
/// written out.
#[test]
fn clicking_the_activity_area_opens_the_run_log() {
    let mut win = scenes::data_file();
    // Off Log first, which is where the ledger opens, so the click has somewhere to move it.
    let other = win
        .app
        .rail_name_rect(LEDGER_RAIL, 1)
        .expect("the ledger names a second pane");
    win.click(other.center());
    let before = win.app.rail_pane_title(LEDGER_RAIL);
    assert!(
        before
            .as_deref()
            .is_some_and(|t| !t.to_lowercase().contains("log")),
        "the ledger is still on {before:?}"
    );
    let collapse = win
        .app
        .rail_collapse_rect(LEDGER_RAIL)
        .expect("the ledger drew its collapse control");
    win.click(collapse.center());
    assert!(
        win.app.rail_is_collapsed(LEDGER_RAIL),
        "the ledger did not close"
    );
    let area = win.app.activity_area_rect().expect("the activity area");
    assert!(
        win.app
            .named_controls()
            .iter()
            .any(|c| c.name == ACTIVITY_CONTROL_NAME && c.rect == area),
        "the activity area is not among the band's named controls"
    );
    win.click(area.center());
    assert!(
        !win.app.rail_is_collapsed(LEDGER_RAIL),
        "the click left the ledger closed"
    );
    let title = win.app.rail_pane_title(LEDGER_RAIL);
    assert!(
        title
            .as_deref()
            .is_some_and(|t| t.to_lowercase().contains("log")),
        "the ledger opened on {title:?}"
    );
}

/// **On a narrow window the activity area gives way first: Run and the Protocol's place stay
/// whole in the band, and the area comes back when the window is wide again.**
#[test]
fn on_a_narrow_window_the_activity_area_goes_first_and_run_stays() {
    let mut win = scenes::data_file();
    assert!(
        win.app.activity_area_rect().is_some(),
        "a wide band draws the area"
    );
    let wide = win.screen;
    win.screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(560.0, 820.0));
    win.settle();
    let band = win.app.region_rect(TITLE_BAND).expect("the title band");
    let run = win
        .app
        .run_control_rect()
        .expect("Run is drawn at any width");
    assert!(
        band.contains_rect(run),
        "Run at {run:?} is cut by the band {band:?}"
    );
    let place = win.app.title_place_rects().expect("the place");
    assert!(
        place.name.is_some(),
        "the name field went before the activity area"
    );
    assert_eq!(
        win.app.activity_area_rect(),
        None,
        "the activity area stayed on a band too narrow for it"
    );
    win.screen = wide;
    win.settle();
    assert!(
        win.app.activity_area_rect().is_some(),
        "the activity area did not come back on a wide band"
    );
}

/// The spec file a chart was composed from is offered on the hover of the
/// title band's name, by its file name alone.
#[test]
fn the_title_bands_name_offers_the_spec_file_on_hover() {
    let win = scenes::data_file();
    let spec = win
        .app
        .chart_doc()
        .spec_path
        .clone()
        .expect("a Protocol made from a data file composes a spec file");
    let name = spec.file_name().expect("a file name").to_string_lossy();
    assert_eq!(win.app.document_source_about().as_deref(), Some(&*name));
}
