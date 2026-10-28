//! **The chart under the pointer, handed over as a picture.**
//!
//! `copy-chart-picture` puts the plot the pointer is on onto the clipboard and
//! `export-chart-picture` saves it as a PNG, each from its key and from the
//! palette. What is read back here is what a person receives: the image the
//! frame handed the clipboard, and the file written in place of the save
//! dialog's answer.

use std::path::PathBuf;

use brightfield_shell::chart_picture::{clipboard_image, plot_extent, render_plot, PICTURE_SCALE};
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::live_spec;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};

struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    commands: Vec<egui::OutputCommand>,
}

impl Window {
    /// A chart over `fixture` (a name under `examples/`), live, saving its
    /// exported pictures into `exports` when one is given.
    fn chart(fixture: &str, exports: Option<PathBuf>) -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(fixture);
        let (live, composed) = live_spec(path.to_str().expect("utf-8 fixture path"))
            .unwrap_or_else(|e| panic!("{fixture} loads live: {e}"));
        let mut boot = Boot::charts(composed);
        boot.live = Some(live);
        boot.spec_path = Some(path);
        let mut app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
        if let Some(dir) = exports {
            app = app.exporting_pictures_to(dir);
        }
        let mut win = Self {
            app,
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
            commands: Vec::new(),
        };
        win.run(vec![Vec::new(), Vec::new()]);
        win
    }

    fn run(&mut self, frames: Vec<Vec<egui::Event>>) {
        for events in frames {
            let raw = egui::RawInput {
                screen_rect: Some(self.screen),
                events,
                ..Default::default()
            };
            let out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
            self.commands.extend(out.platform_output.commands);
        }
    }

    /// Rest the pointer at `at` for a few frames, long enough for the hover
    /// readout to draw.
    fn hover(&mut self, at: egui::Pos2) {
        self.run(vec![vec![egui::Event::PointerMoved(at)]]);
        for _ in 0..4 {
            std::thread::sleep(std::time::Duration::from_millis(60));
            self.run(vec![Vec::new()]);
        }
    }

    /// Press ⌘⇧ and `key`.
    fn chord(&mut self, key: egui::Key) {
        self.run(vec![
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            }],
            Vec::new(),
        ]);
    }

    /// The pictures the frames put on the clipboard, oldest first.
    fn copied(&self) -> Vec<egui::ColorImage> {
        self.commands
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::CopyImage(image) => Some(image.clone()),
                _ => None,
            })
            .collect()
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let dir = std::env::temp_dir().join(format!("bf-{name}-{}-{nanos}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temp directory");
    dir
}

/// The picture is the tile's own drawing, and the hover readout over it is not
/// in it: the picture copied with the pointer resting on a mark is the picture
/// copied with the pointer away from the chart.
#[test]
fn the_copied_picture_is_the_chart_without_its_hover_chrome() {
    let mut win = Window::chart("scatter.yaml", None);
    win.chord(egui::Key::C);
    let rect = *win
        .app
        .composed_plot_rects()
        .first()
        .expect("scatter draws one plot");
    win.hover(rect.center());
    win.chord(egui::Key::C);
    let copied = win.copied();
    assert_eq!(copied.len(), 2, "each ⌘⇧C copies one picture");
    assert_eq!(
        copied[0], copied[1],
        "the picture copied under a resting pointer differs from the one copied without it"
    );
    let composed = &win.app.chart_doc().composed;
    let extent = plot_extent(composed, 0).expect("the plot has an extent");
    let want = [
        (extent.width * f64::from(PICTURE_SCALE)).round() as usize,
        (extent.height * f64::from(PICTURE_SCALE)).round() as usize,
    ];
    assert_eq!(
        copied[0].size, want,
        "the picture is the plot and its legend, at 2x"
    );
}

/// On a page of two plots, the picture is of the plot the pointer is on.
#[test]
fn the_picture_is_of_the_plot_under_the_pointer() {
    let mut win = Window::chart("crossfilter.yaml", None);
    let rects = win.app.composed_plot_rects();
    assert_eq!(rects.len(), 2, "crossfilter draws two plots");
    win.hover(rects[1].center());
    win.chord(egui::Key::C);
    let copied = win.copied();
    let composed = &win.app.chart_doc().composed;
    let second = clipboard_image(&render_plot(composed, 1, PICTURE_SCALE).expect("draws"));
    let first = clipboard_image(&render_plot(composed, 0, PICTURE_SCALE).expect("draws"));
    assert_eq!(
        copied.last(),
        Some(&second),
        "the picture is not the second plot"
    );
    assert_ne!(
        first, second,
        "the fixture's two plots draw alike, so this proves nothing"
    );
}

/// ⌘⇧E saves the plot under the pointer under the spec's name, numbered by its
/// place on the page when the page has more than one.
#[test]
fn the_export_key_saves_the_plot_under_the_pointer_as_a_png() {
    let dir = temp_dir("chart-picture");
    let mut win = Window::chart("crossfilter.yaml", Some(dir.clone()));
    let rects = win.app.composed_plot_rects();
    win.hover(rects[1].center());
    win.chord(egui::Key::E);
    let written = dir.join("crossfilter-2.png");
    let saved = image::open(&written)
        .unwrap_or_else(|e| panic!("⌘⇧E wrote no {}: {e}", written.display()))
        .to_rgba8();
    let want = render_plot(&win.app.chart_doc().composed, 1, PICTURE_SCALE).expect("draws");
    assert_eq!(saved.dimensions(), want.dimensions());
    assert!(
        saved == want,
        "the saved file is not the picture of the second plot"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
