//! The Preferences sheet: the Protocols folder, shown as a path, with Choose… to pick another with
//! the system's folder picker, and, once one is chosen, the offer to move the old folder's
//! Protocols into it. Below it, on a Mac, the button that asks macOS to make Brightfield the
//! default app for CSV, Parquet and DuckDB files ([`crate::default_app`]). Below that, once a
//! folder has been trusted with the Protocols beneath it, the trusted folders, each with Remove.
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
    /// Stop trusting the folder at this place in the list of trusted folders.
    Untrust(usize),
    /// Ask macOS to make Brightfield the default app for CSV, Parquet and DuckDB files.
    MakeDefault,
}

/// The button that asks macOS to make Brightfield the default app, on the first launch's question
/// and in Preferences.
pub const MAKE_DEFAULT: &str = "Make Brightfield the default";

/// Width kept beside the path for the Choose… button and the gap before it.
const CHOOSE_ROOM: f32 = 110.0;

/// `path` shortened in the middle to fit `room` points in `font`: the start and as many of the
/// last folders as fit, joined by an ellipsis, such as `/Users/…/Documents/Protocols`. The final
/// folder is always kept, even where it alone is wider than `room`.
pub(crate) fn elide_path(ui: &egui::Ui, path: &Path, font: &egui::FontId, room: f32) -> String {
    let full = path.display().to_string();
    let width = |text: &str| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE)
            .size()
            .x
    };
    if width(&full) <= room {
        return full;
    }
    let parts: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    // The root and the first folder lead; the root's own text is the separator.
    let head = parts.len().min(2);
    let join = |part: &[String]| part.join("/").replacen("//", "/", 1);
    let mut best = String::new();
    for keep in (1..=parts.len().saturating_sub(head)).rev() {
        let tail = &parts[parts.len() - keep..];
        let candidate = format!("{}/\u{2026}/{}", join(&parts[..head]), tail.join("/"));
        best = candidate;
        if width(&best) <= room {
            break;
        }
    }
    if best.is_empty() {
        full
    } else {
        best
    }
}

/// Draw the sheet's body: the Protocols folder `folder`, the offer while one waits, the default-app
/// button where `default_app` says there is one (`Err` with why it cannot be pressed in this
/// launch), and the folders in `trusted` when there are any. `overridden` is the variable that
/// sets the folder for this launch, when it is set.
pub fn body(
    ui: &mut egui::Ui,
    sheet: &PreferencesSheet,
    folder: &Path,
    overridden: Option<&str>,
    default_app: Option<&Result<(), String>>,
    trusted: &[PathBuf],
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
        // Room for Choose… beside the path, so a long path gives up its middle rather than
        // widening the card past its rung.
        let room = if overridden.is_none() {
            (ui.available_width() - CHOOSE_ROOM).max(0.0)
        } else {
            ui.available_width()
        };
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        ui.label(egui::RichText::new(elide_path(ui, folder, &font, room)).monospace())
            .on_hover_text(folder.display().to_string());
        // The variable is for tests and portable installs, and wins over the setting, so the
        // sheet does not offer to change a setting that would not be used.
        if overridden.is_none() && ui.button("Choose\u{2026}").clicked() {
            action = Some(PreferencesAction::Choose);
        }
    });
    if let Some(var) = overridden {
        ui.label(
            egui::RichText::new(format!(
                "{var} sets the Protocols folder for this launch, so it cannot be changed here."
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
        let font = egui::TextStyle::Body.resolve(ui.style());
        let room = ui.available_width() * 0.4;
        ui.label(format!(
            "Move {what} in {} to {}? Each moves as Location moves one, and the move stops at the \
             first that cannot.",
            elide_path(ui, &offer.from, &font, room),
            elide_path(ui, &offer.to, &font, room)
        ))
        .on_hover_text(format!(
            "{} to {}",
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
    if let Some(can) = default_app {
        ui.add_space(gap);
        ui.label(egui::RichText::new("Default app").strong());
        ui.label(
            egui::RichText::new(
                "Make Brightfield the default app for CSV, Parquet and DuckDB files, so they open \
                 here when you double-click them. macOS asks you to confirm each kind of file.",
            )
            .color(chrome::colour(sem.text.secondary)),
        );
        let button = ui.add_enabled(can.is_ok(), egui::Button::new(MAKE_DEFAULT));
        let button = match can {
            Ok(()) => button.on_hover_text("macOS shows its own confirmation for each kind"),
            Err(why) => button.on_disabled_hover_text(why.as_str()),
        };
        if button.clicked() {
            action = Some(PreferencesAction::MakeDefault);
        }
    }
    if !trusted.is_empty() {
        ui.add_space(gap);
        ui.label(egui::RichText::new("Trusted folders").strong());
        ui.label(
            egui::RichText::new(
                "Protocols in these folders run when they open, including ones added later.",
            )
            .color(chrome::colour(sem.text.secondary)),
        );
        for (index, trusted) in trusted.iter().enumerate() {
            ui.horizontal(|ui| {
                let font = egui::TextStyle::Monospace.resolve(ui.style());
                let room = (ui.available_width() - CHOOSE_ROOM).max(0.0);
                ui.label(egui::RichText::new(elide_path(ui, trusted, &font, room)).monospace())
                    .on_hover_text(trusted.display().to_string());
                if ui.button("Remove").clicked() {
                    action = Some(PreferencesAction::Untrust(index));
                }
            });
        }
    }
    action
}
