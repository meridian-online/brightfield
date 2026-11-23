//! The unified title band's window behaviour: the corner it leaves for the window's own
//! controls, and the empty stretch that moves and zooms the window.
//!
//! On macOS the window draws its content under a hidden title bar (`main.rs` builds the viewport
//! that way), so the title band is the window's top edge. That takes over two things the title bar
//! did: a drag on it moves the window, and a double-click zooms it. Both are answered here, on
//! whatever part of the band no control took, so a press on the mark, the name or a field still
//! goes to that control.

use brightfield_workbench::arrangement::WINDOW_CONTROLS_INSET;

/// Whether the title band stands in for the window's title bar: on macOS, where `main.rs` hides
/// the title bar and draws the window's content under it.
#[must_use]
pub fn replaces_the_title_bar() -> bool {
    cfg!(target_os = "macos")
}

/// How far the band's first control stands from the window's left edge: clear of the close,
/// minimise and zoom buttons on macOS, and nothing on a platform that draws them outside the
/// window's content.
#[must_use]
pub fn controls_inset() -> f32 {
    if replaces_the_title_bar() {
        WINDOW_CONTROLS_INSET
    } else {
        0.0
    }
}

/// Make `rect`, a stretch of the band no control occupies, move the window when dragged and zoom
/// it when double-clicked, as a title bar does.
///
/// Call it after the band's controls are drawn, with a rect that overlaps none of them: egui gives
/// a press to the last widget that senses the point under it. `salt` tells apart the band's
/// stretches.
///
/// Only where the band replaces the title bar, which is macOS: elsewhere the window keeps its
/// native title bar, which moves and zooms it, and the band leaves those to it.
pub fn drag_and_zoom(ui: &egui::Ui, salt: &str, rect: egui::Rect) {
    if !replaces_the_title_bar() || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let response = ui.interact(
        rect,
        ui.id().with(("title-band-stretch", salt)),
        egui::Sense::click_and_drag(),
    );
    if response.double_clicked() {
        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    } else if response.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}
