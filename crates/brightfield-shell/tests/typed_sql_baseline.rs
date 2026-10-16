//! **Typed SQL as pixels, in both themes.**
//!
//! The palette pair is the statement form over the housing file: the card
//! with no scrim, standing over the map pane, and the grid previewing the
//! statement's first page under the preview label. The spine pair is the same
//! statement kept with Cmd+Enter: its step and table at the foot of the
//! Protocol with the not-recorded dot, and its table in the grid.
//!
//! The statement is read on a worker, so the capture waits after each frame
//! until the read has answered, so Cmd+Enter is pressed after the answer
//! (`capture::capture_png_awaiting`). The statement ends in an `ORDER BY`, so
//! its first page is the same on every read.
//!
//! Regenerate with `UPDATE_SNAPSHOTS=1 cargo test -p brightfield-shell --test
//! typed_sql_baseline`.

use std::path::PathBuf;
use std::time::Duration;

use brightfield_shell::design::Mode;
use brightfield_shell::protocol::SpineMarker;
use brightfield_shell::starts;
use brightfield_shell::window::{Boot, MeridianApp};

/// The window's size: the accepted frame's.
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// The statement, typed a line at a time with Enter between the lines.
const LINES: [&str; 4] = [
    "select house_age, count(*) as n",
    "from california_housing",
    "group by house_age",
    "order by house_age",
];

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bf-typed-sql-baseline-{name}-{}",
            std::process::id()
        ));
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

fn press(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// `space`, then the statement typed with Enter between its lines, and, when
/// `keep`, Cmd+Enter.
fn script(keep: bool) -> Vec<Vec<egui::Event>> {
    let mut frames = vec![vec![press(egui::Key::Space, egui::Modifiers::NONE)]];
    for (i, line) in LINES.iter().enumerate() {
        if i > 0 {
            frames.push(vec![press(egui::Key::Enter, egui::Modifiers::NONE)]);
        }
        frames.push(vec![egui::Event::Text((*line).to_owned())]);
    }
    if keep {
        frames.push(vec![press(egui::Key::Enter, egui::Modifiers::COMMAND)]);
    }
    frames
}

/// Whether the window has nothing on a worker that the picture waits for:
/// the statement on the form has been answered, each kept statement's rows
/// have been counted, and the grid's preview has been read.
fn settled(app: &MeridianApp) -> bool {
    if app.typed_statement().is_some() {
        return app.typed_said().is_some();
    }
    app.chart_doc()
        .typed_steps()
        .iter()
        .all(|s| s.rows.is_some())
        && app
            .chart_doc()
            .grid_preview()
            .is_none_or(|p| !p.is_reading())
}

/// What the last frame drew, noted for the guards.
#[derive(Default)]
struct Seen {
    card: Option<egui::Rect>,
    grid: Option<egui::Rect>,
    unrecorded: bool,
    statement: Option<String>,
}

/// Capture the window over the housing file after [`script`] in `mode`.
fn capture(mode: Mode, name: &str, keep: bool) -> (image::RgbaImage, Seen) {
    let root = TempDir::new(name);
    let data = root.0.join(starts::HOUSING_FILE);
    std::fs::write(&data, starts::HOUSING_PARQUET).expect("the housing file writes");
    let boot = Boot::data_file(data.to_str().expect("utf-8 path"))
        .unwrap_or_else(|e| panic!("open {}: {e}", data.display()));

    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.capture.png"));
    std::fs::create_dir_all(out.parent().expect("a parent")).expect("the capture's folder");
    let mut seen = Seen::default();
    let (w, h) = brightfield_shell::capture::capture_png_awaiting(
        boot,
        brightfield_shell::startup::default_layout(),
        mode,
        1.0,
        WINDOW,
        &out,
        script(keep),
        |app| app.set_typed_pause(Duration::ZERO),
        |app| {
            seen = Seen {
                card: app.typed_card_rect(),
                grid: app.canvas_panes().pane("grid").map(|p| p.rect),
                unrecorded: app
                    .spine_rows()
                    .iter()
                    .any(|r| r.marker == SpineMarker::Unrecorded),
                statement: app.typed_statement().map(str::to_owned),
            };
            settled(app)
        },
    )
    .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();
    (image, seen)
}

/// The statement form, guarded before its snapshot: under
/// `UPDATE_SNAPSHOTS=1` a snapshot writes whatever it is handed, so a guard
/// behind it would author a golden of the wrong window.
fn palette_surface(mode: Mode, name: &str) {
    let (image, seen) = capture(mode, name, false);
    assert_eq!(
        seen.statement.as_deref(),
        Some(LINES.join("\n").as_str()),
        "{name}: the statement form does not hold the statement"
    );
    let card = seen.card.expect("the statement form drew its card");
    let grid = seen.grid.expect("the canvas drew a grid pane");
    assert!(
        !card.intersects(grid),
        "{name}: the card {card:?} stands over the grid {grid:?}"
    );
    egui_kittest::image_snapshot(&image, name);
}

/// The kept statement, guarded before its snapshot as [`palette_surface`] is.
fn spine_surface(mode: Mode, name: &str) {
    let (image, seen) = capture(mode, name, true);
    assert_eq!(seen.statement, None, "{name}: the statement was not kept");
    assert!(
        seen.unrecorded,
        "{name}: no spine row carries the not-recorded mark"
    );
    egui_kittest::image_snapshot(&image, name);
}

#[test]
fn typed_sql_palette_light_surface() {
    palette_surface(Mode::Light, "typed_sql_palette_light");
}

#[test]
fn typed_sql_palette_dark_surface() {
    palette_surface(Mode::Dark, "typed_sql_palette_dark");
}

#[test]
fn typed_sql_spine_light_surface() {
    spine_surface(Mode::Light, "typed_sql_spine_light");
}

#[test]
fn typed_sql_spine_dark_surface() {
    spine_surface(Mode::Dark, "typed_sql_spine_dark");
}
