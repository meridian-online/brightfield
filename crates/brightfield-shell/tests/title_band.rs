//! The unified title band: it leaves the window's own controls their corner on macOS, where the
//! window draws under a hidden title bar; its empty stretch moves the window when dragged and zooms
//! it when double-clicked, as a title bar does; and it keeps the open Protocol's name and the
//! folder it is kept in, which used to sit in the locator band below it.

#[path = "support/scenes.rs"]
mod scenes;

use brightfield_shell::menu_bar::MenuHost;
use brightfield_workbench::arrangement::{LOCATOR_BAND, TITLE_BAND};
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
    egui::pos2(place.name.left() - 24.0, band.center().y)
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
    let name = win.app.title_place_rects().expect("the place").name;
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
        assert!(
            band.contains_rect(rect),
            "the {what} field at {rect:?} lies outside the title band {band:?}"
        );
    }
    assert!(
        place.location.left() >= place.name.right(),
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
fn the_locator_band_keeps_the_crumbs_and_counts() {
    let win = scenes::data_file();
    let locator = win.app.region_rect(LOCATOR_BAND).expect("the locator band");
    // A Protocol made from a data file reads its spine as the trail: the raw table, the steps
    // that refine it, and the dashboard last.
    let crumbs = win.app.locator_crumbs();
    assert!(
        crumbs.len() > 1,
        "the locator band kept its trail: {crumbs:?}"
    );
    assert_eq!(
        crumbs.last().map(String::as_str),
        Some("dashboard"),
        "{crumbs:?}"
    );
    let place = win.app.title_place_rects().expect("the place");
    assert!(
        !locator.intersects(place.name) && !locator.intersects(place.location),
        "the place fields left the locator band"
    );
}
