//! The palette's history carries from one window to the next: a command run
//! from the palette is written to the config folder, and a window made after
//! it ranks that command higher under an empty query than a window made
//! before it did.
//!
//! One test in its own binary, because it points `BRIGHTFIELD_CONFIG_DIR` at a
//! scratch folder, and that variable is the whole process's: a test beside it
//! in the same binary would read the history this one writes.

use std::path::PathBuf;

use brightfield_model::palette_history_file::{load_palette_history, palette_history_path};
use brightfield_protocol::layout::Flow;
use brightfield_shell::design::Mode;
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};

/// A headless window over the shipped crosswalk, with its context, as
/// `overlay_wiring.rs` drives one.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
}

impl Window {
    fn crosswalk() -> Self {
        let boot = Boot::start(starts::CROSSWALK, Flow::Vertical).expect("the crosswalk ships");
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
        };
        win.frames(vec![Vec::new(), Vec::new()]);
        win
    }

    fn frames(&mut self, frames: Vec<Vec<egui::Event>>) {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0));
        for events in frames {
            let raw = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        }
    }

    fn key(&mut self, key: egui::Key) {
        let press = egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        };
        self.frames(vec![vec![press], Vec::new()]);
    }

    /// Where `longname` sits in the palette's empty-query list, opened and
    /// closed again.
    fn empty_query_position(&mut self, longname: &str) -> usize {
        self.key(egui::Key::Space);
        self.frames(vec![Vec::new()]);
        let rows = self.app.open_palette_rows();
        self.key(egui::Key::Escape);
        rows.iter()
            .position(|row| row == longname)
            .unwrap_or_else(|| panic!("{longname} is not in the palette: {rows:?}"))
    }
}

#[test]
fn a_palette_pick_ranks_higher_in_the_next_window() {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("bf-palette-history-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch config folder");
    std::env::set_var(brightfield_shell::startup::CONFIG_DIR_VAR, &dir);

    const PICKED: &str = "open-steps-sheet";
    let mut first = Window::crosswalk();
    let before = first.empty_query_position(PICKED);
    assert!(
        before > 0,
        "{PICKED} already leads the palette, so a pick could not lift it"
    );

    first.key(egui::Key::Space);
    first.frames(vec![Vec::new()]);
    first.frames(vec![
        vec![egui::Event::Text("steps".to_owned())],
        Vec::new(),
    ]);
    first.key(egui::Key::Enter);
    assert!(
        first.app.protocol_model().show_sheet(),
        "the palette did not run {PICKED}"
    );

    assert!(
        palette_history_path(&dir).exists(),
        "the pick was not saved"
    );
    assert!(
        load_palette_history(&dir).weight(PICKED, brightfield_keys::unix_now_secs()) > 0,
        "the saved history does not hold {PICKED}"
    );

    let mut second = Window::crosswalk();
    let after = second.empty_query_position(PICKED);
    assert!(
        after < before,
        "{PICKED} sat at {before} before the pick and at {after} in the next window"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
