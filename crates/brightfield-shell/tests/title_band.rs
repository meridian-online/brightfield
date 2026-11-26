//! The unified title band: it leaves the window's own controls their corner on macOS, where the
//! window draws under a hidden title bar; there its empty stretch moves the window when dragged and
//! zooms it when double-clicked, as a title bar does, and elsewhere it leaves both to the window's
//! own title bar; and it keeps the open Protocol's name and the
//! folder it is kept in.

#[path = "support/scenes.rs"]
mod scenes;

use brightfield_shell::menu_bar::MenuHost;
use brightfield_workbench::arrangement::TITLE_BAND;
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
