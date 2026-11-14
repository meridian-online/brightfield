//! The organise sheet drawn: a heading, the shared join canvas over the Protocol's joins, and a
//! foot naming the keys. Its state and keys are [`crate::organise_joins`]'s.

use meridian_design::semantic;
use meridian_egui::MeridianUi as _;

use crate::design::{to_color32, Mode};
use crate::organise_joins::{Effect, Organise};
use crate::sources_browser_view::{draw_join_canvas, token};

/// **Draw the organise sheet into `ui`**, in the sources browser's frame and the theme of `mode`,
/// and answer what its keys and clicks asked of the window this frame.
///
/// The keys are read before the sheet is drawn and taken out of the frame's input, so a `j`
/// moves the sheet's cursor and not the graph's behind it, and `Esc` closes the sheet rather
/// than a layer under it. A key with a modifier held is left in the input for the window.
pub fn show(ui: &mut egui::Ui, sheet: &mut Organise, mode: Mode) -> Effect {
    let effect = keys(ui, sheet);
    if effect != Effect::None {
        return effect;
    }
    let sem = semantic(mode.is_dark());
    // A row is the click target, so the words on it are not selectable text that would take the
    // click from the row under them.
    ui.style_mut().interaction.selectable_labels = false;
    let mut clicked = None;
    crate::sources_browser_view::frame(ui).show(ui, |ui| {
        let t = ui.tokens();
        ui.label(
            egui::RichText::new("Organise joins")
                .strong()
                .color(to_color32(sem.text.primary)),
        );
        ui.add_space(t.section_gap);
        if sheet.rows.is_empty() {
            ui.label(
                egui::RichText::new("The model holds no joins between its tables yet.")
                    .color(to_color32(sem.text.muted)),
            );
        } else {
            let joins = sheet.canvas_joins();
            clicked = draw_join_canvas(ui, &joins, sheet.cursor, true, "kept").clicked;
        }
        ui.add_space(t.section_gap);
        ui.label(
            egui::RichText::new("Space ticks or unticks \u{b7} Enter applies \u{b7} Esc closes")
                .small()
                .color(to_color32(sem.text.muted)),
        );
    });
    if let Some(at) = clicked {
        sheet.toggle(at);
    }
    // The sheet answers `Space` itself, so egui's own focus is kept off its rows, where `Space`
    // would click one as well.
    if let Some(id) = ui.ctx().memory(egui::Memory::focused) {
        ui.ctx().memory_mut(|m| m.surrender_focus(id));
    }
    Effect::None
}

/// Answer the frame's keystrokes, in the order they were pressed, and take each one answered out
/// of the input. The first that asks something of the window ends the frame's keys.
fn keys(ui: &mut egui::Ui, sheet: &mut Organise) -> Effect {
    let pressed: Vec<(usize, &'static str)> = ui.input(|i| {
        i.events
            .iter()
            .enumerate()
            .filter_map(|(at, e)| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.is_none() => token(*key).map(|t| (at, t)),
                _ => None,
            })
            .collect()
    });
    if pressed.is_empty() {
        return Effect::None;
    }
    let mut effect = Effect::None;
    let mut taken = Vec::new();
    for (at, keystroke) in pressed {
        taken.push(at);
        effect = sheet.press(keystroke);
        if effect != Effect::None {
            break;
        }
    }
    let mut index = 0;
    ui.input_mut(|i| {
        i.events.retain(|_| {
            let keep = !taken.contains(&index);
            index += 1;
            keep
        });
    });
    ui.ctx().request_repaint();
    effect
}
