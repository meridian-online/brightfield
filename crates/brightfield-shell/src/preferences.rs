//! The Preferences sheet: the Protocols folder, shown as a path, with Choose… to pick another with
//! the system's folder picker, and, once one is chosen, the offer to move the old folder's
//! Protocols into it.
//!
//! The sheet draws in the window's theme tokens inside the window's modal layer, as every sheet
//! does. What its controls ask for is answered by the window after the frame
//! ([`PreferencesAction`]), since the picker blocks the thread and a move reopens the Protocol.

use std::path::{Path, PathBuf};

use brightfield_workbench::chrome;

/// The Preferences sheet's state: the offer to move, while one is waiting for an answer.
#[derive(Debug, Default)]
pub struct PreferencesSheet {
    /// The offer made when the Protocols folder was changed, until it is answered.
    pub offer: Option<MoveOffer>,
}

/// The offer, made once when the Protocols folder changes, to move what the old one holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveOffer {
    /// The Protocols folder before the change.
    pub from: PathBuf,
    /// The Protocols folder now.
    pub to: PathBuf,
    /// How many Protocols the old folder holds, as the door's tree reads it.
    pub count: usize,
}

/// What a control on the sheet asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferencesAction {
    /// Choose another Protocols folder with the system's folder picker.
    Choose,
    /// Move the old folder's Protocols into the new one.
    Move,
    /// Leave them where they are.
    Leave,
}

/// Draw the sheet's body: the Protocols folder `folder`, and the offer while one waits.
/// `overridden` is the variable that sets the folder for this launch, when it is set.
pub fn body(
    ui: &mut egui::Ui,
    sheet: &PreferencesSheet,
    folder: &Path,
    overridden: Option<&str>,
) -> Option<PreferencesAction> {
    use meridian_egui::MeridianUi as _;
    let gap = ui.tokens().section_gap;
    let sem = meridian_design::semantic(ui.visuals().dark_mode);
    let mut action = None;
    ui.label(egui::RichText::new("Protocols folder").strong());
    ui.label(
        egui::RichText::new("New Protocols are made here, and the front door lists what it holds.")
            .color(chrome::colour(sem.text.secondary)),
    );
    ui.add_space(gap);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(folder.display().to_string()).monospace());
        if ui.button("Choose\u{2026}").clicked() {
            action = Some(PreferencesAction::Choose);
        }
    });
    if let Some(var) = overridden {
        ui.label(
            egui::RichText::new(format!(
                "{var} sets the folder for this launch, so a folder chosen here is used once it \
                 is unset."
            ))
            .color(chrome::colour(sem.text.secondary)),
        );
    }
    if let Some(offer) = &sheet.offer {
        ui.add_space(gap);
        let what = if offer.count == 1 {
            "the Protocol".to_string()
        } else {
            format!("the {} Protocols", offer.count)
        };
        ui.label(format!(
            "Move {what} in {} to {}? Each moves as Location moves one, and the move stops at the \
             first that cannot.",
            offer.from.display(),
            offer.to.display()
        ));
        ui.horizontal(|ui| {
            if ui.button("Move them").clicked() {
                action = Some(PreferencesAction::Move);
            }
            if ui.button("Leave them").clicked() {
                action = Some(PreferencesAction::Leave);
            }
        });
    }
    action
}
