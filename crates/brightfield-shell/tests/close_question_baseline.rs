//! **The close question as pixels, in both themes.**
//!
//! The question over a window holding a column put on the Map's x, the two
//! edits the accepted frame draws: the file named, the edits counted and
//! listed, what closing without saving keeps, and the three answers with their
//! keys and no footer.
//!
//! A probe window is driven by clicks, and a capture replays its frames. A
//! close request is raised in the viewport's input and not as an event, so no
//! frame of a script carries one; the capture opens the question after the
//! script through `MeridianApp::ask_before_closing`, the entry a close request
//! over the unsaved mark reaches (`capture::capture_png_staged`). That a close
//! request reaches it is `close_question_keeps_unsaved.rs`'s to hold.
//!
//! Regenerate with `UPDATE_SNAPSHOTS=1 cargo test -p brightfield-shell --test
//! close_question_baseline`.

use std::path::PathBuf;

use brightfield_protocol::HistoryStore;
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::SpineRole;
use brightfield_shell::versions::Clock;
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_workbench::channel::ShelfChannel;

const HOUSING_FILE: &str = "california_housing_sample.csv";

/// The window's size: the accepted frame's.
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// What a column put on the Map's x reads as in the question's list.
const X_PUT: [&str; 2] = [
    "Map \u{b7} projection type: equirectangular removed",
    "Map \u{b7} x axis: longitude \u{2192} median_income",
];

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("bf-close-baseline-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A window and the frames it was driven through, for the capture to replay.
struct Probe {
    app: MeridianApp,
    ctx: egui::Context,
    frames: Vec<Vec<egui::Event>>,
}

impl Probe {
    fn run(&mut self, events: Vec<egui::Event>) {
        self.frames.push(events.clone());
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(WINDOW.0, WINDOW.1),
            )),
            events,
            ..Default::default()
        };
        let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    fn point(&mut self, at: egui::Pos2) {
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.settle();
    }

    fn click(&mut self, pos: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![egui::Event::PointerMoved(pos), button(true)],
            vec![egui::Event::PointerMoved(pos), button(false)],
            Vec::new(),
            Vec::new(),
        ] {
            self.run(events);
        }
    }

    /// Put `column` on `channel`, by the chip on its row in the Outline.
    fn put(&mut self, column: &str, channel: ShelfChannel) {
        let row = self
            .app
            .spine_rows()
            .iter()
            .find(|r| r.role == SpineRole::Column && r.depth == 1 && r.label == column)
            .unwrap_or_else(|| panic!("the Outline drew no row for {column}"))
            .clone();
        self.point(row.name_rect.center());
        let chip = self
            .app
            .outline_chips()
            .iter()
            .find(|c| c.column == column && c.channel == channel)
            .unwrap_or_else(|| panic!("{column}'s row drew no {channel:?} chip"))
            .clone();
        self.click(chip.rect.center());
        self.point(egui::pos2(1.0, 1.0));
    }
}

/// Capture the question over the window in `mode`.
fn capture_the_question(mode: Mode, name: &str) -> image::RgbaImage {
    let root = TempDir::new(name);
    let folder = root.0.join("data");
    std::fs::create_dir_all(&folder).expect("the data file's folder");
    let data = folder.join(HOUSING_FILE);
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(HOUSING_FILE),
        &data,
    )
    .expect("the housing fixture copies");
    let store = HistoryStore::At(root.0.join(".arcform").join("history"));
    let boot = || {
        Boot::data_file(data.to_str().expect("utf-8 path"))
            .unwrap_or_else(|e| panic!("open {}: {e}", data.display()))
    };

    let app = MeridianApp::headless(boot(), Mode::Light).keeping_history(Some(store.clone()));
    let mut probe = Probe {
        app,
        ctx: egui::Context::default(),
        frames: Vec::new(),
    };
    probe.settle();
    probe.put("median_income", ShelfChannel::X);
    assert!(
        probe.app.title().contains(UNSAVED_MARK),
        "the probe holds no unsaved edit"
    );
    assert!(probe.app.ask_before_closing(), "the probe did not ask");
    assert_eq!(
        probe.app.close_question().expect("a question").edits,
        X_PUT,
        "the probe's question does not list the edits the baseline is of"
    );

    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.capture.png"));
    std::fs::create_dir_all(out.parent().expect("a parent")).expect("the capture's folder");
    let home = root.0.clone();
    let mut asked = false;
    let (w, h) = brightfield_shell::capture::capture_png_staged(
        boot(),
        brightfield_shell::startup::default_layout(),
        mode,
        1.0,
        WINDOW,
        &out,
        probe.frames.clone(),
        |app| {
            // The shelf band is authoring chrome, which a capture leaves out;
            // the column is put by the Outline's chips, which it holds, and
            // the accepted frame draws it.
            app.set_shelf_band_drawn(true);
            app.set_history(Some(store));
            app.set_versions_env(Clock::System, Some(home));
        },
        |app| asked = app.ask_before_closing(),
    )
    .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(
        asked,
        "{name}: the captured window held no edit to ask about"
    );
    assert!(w > 0 && h > 0, "{name}: empty capture");
    image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8()
}

/// **AC7, light.** The question over a column put on x matches its baseline.
#[test]
fn the_close_question_light_baseline() {
    let image = capture_the_question(Mode::Light, "close_question_light");
    egui_kittest::image_snapshot(&image, "close_question_light");
}

/// **AC7, dark.** The same window and the same script, the ink moved.
#[test]
fn the_close_question_dark_baseline() {
    let image = capture_the_question(Mode::Dark, "close_question_dark");
    egui_kittest::image_snapshot(&image, "close_question_dark");
}
