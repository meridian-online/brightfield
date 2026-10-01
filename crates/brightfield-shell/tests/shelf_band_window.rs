//! **`e` gives the hero pane's keys to a band carved under its header.**
//!
//! With the dashboard's hero pane focused, `e` hands the keys to the shelf
//! band; the band's own keys open a cell, a cell opens the Outline's list of
//! that channel's columns, and the list's keys move its cursor. `Esc` steps
//! back one level a press. With the pane holding the keys `x` is still the axis
//! lock's.
//!
//! The band is 44 high and the pane's width, directly under the pane's header,
//! and the hero's plot gives up that height while the grid pane is laid out as
//! before. Every assertion reads a frame the window drew — the band's drawn
//! record, the panes' rects, the model the Outline reads its list from — and
//! not a field a key wrote.

use brightfield_shell::app::CHART;
use brightfield_shell::capture::capture_png;
use brightfield_shell::data_grid::DATA;
use brightfield_shell::design::Mode;
use brightfield_shell::navigation::AxisLock;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, CanvasPane, MeridianApp};
use brightfield_workbench::channel::{ShelfChannel, BAND_HEIGHT};
use brightfield_workbench::PaneKey;

/// The housing sample every window criterion opens.
fn housing() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv")
}

fn key_down(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// A letter as a keyboard brings it: the key press, then the text it makes.
fn letter(key: egui::Key, text: &str) -> Vec<egui::Event> {
    vec![key_down(key), egui::Event::Text(text.to_owned())]
}

fn button(at: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// One headless window over the housing sample.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    /// A window with the hero pane focused, the band drawn or switched off.
    fn open(band: bool) -> Self {
        let path = housing();
        let boot = Boot::data_file(path.to_str().expect("utf-8 path")).expect("the sample opens");
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
        };
        win.app.set_shelf_band_drawn(band);
        win.settle();
        assert!(
            win.app.focus_pane(PaneKey::new(CHART)),
            "the dashboard's pane takes focus"
        );
        win.settle();
        win
    }

    fn housing() -> Self {
        Self::open(true)
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
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

    /// Press a letter, and let the frame after it run.
    fn type_letter(&mut self, key: egui::Key, text: &str) {
        self.run(letter(key, text));
        self.run(Vec::new());
    }

    fn press(&mut self, key: egui::Key) {
        self.run(vec![key_down(key)]);
        self.run(Vec::new());
    }

    fn active(&self) -> Option<ShelfChannel> {
        self.app.shelf_band().and_then(|b| b.active())
    }

    /// The channel the Outline's list is open on, if one is open.
    fn list_channel(&self) -> Option<ShelfChannel> {
        self.app.protocol_model().column_list().map(|l| l.channel())
    }

    fn map_pane(&self) -> CanvasPane {
        *self
            .app
            .canvas_panes()
            .pane("map")
            .expect("the canvas drew the hero pane")
    }
}

/// **`e` is the registry's key for the verb that hands over the keys**, which
/// the window reads off the registry rather than spelling itself. A key moved
/// there that the window does not map opens nothing, so this reddens first.
#[test]
fn e_is_the_registrys_key_for_set_channel() {
    let key = brightfield_keys::registry()
        .iter()
        .find(|v| v.longname == "set-channel")
        .and_then(brightfield_keys::VerbEntry::primary_key);
    assert_eq!(key, Some("e"));
}

#[test]
fn the_dashboard_draws_the_band_and_a_pane_holds_the_keys_to_begin_with() {
    let win = Window::housing();
    assert!(
        win.app.shelf_drawn().is_some(),
        "a generated dashboard's hero pane draws a band"
    );
    assert!(!win.app.shelf_holds_keys());
    assert_eq!(win.active(), None);
    assert_eq!(win.list_channel(), None);
}

/// **AC1.** `e`, then `x`: the band holds the keys, the x cell is the active
/// one, x's list is open in the Outline, and the list's keys move its cursor.
/// The axis lock is not cycled by the `x`.
#[test]
fn e_gives_the_band_the_keys_and_x_then_opens_the_x_list_whose_keys_move_its_cursor() {
    let mut win = Window::housing();
    let lock = win.app.chart_doc().axis_lock;

    win.type_letter(egui::Key::E, "e");
    assert!(win.app.shelf_holds_keys(), "`e` gives the band the keys");
    assert_eq!(
        win.active(),
        None,
        "no cell is open until a channel is named"
    );
    assert_eq!(win.list_channel(), None);

    win.type_letter(egui::Key::X, "x");
    assert_eq!(win.active(), Some(ShelfChannel::X), "`x` opens the x cell");
    assert_eq!(
        win.list_channel(),
        Some(ShelfChannel::X),
        "and opens x's list in the Outline"
    );
    assert_eq!(
        win.app.chart_doc().axis_lock,
        lock,
        "an `x` the band took does not cycle the axis lock"
    );

    let list = win.app.protocol_model().column_list().expect("list open");
    assert!(list.display().len() > 1, "the housing sample has columns");
    let before = list.cursor().map(str::to_owned);
    win.type_letter(egui::Key::J, "j");
    let after = win
        .app
        .protocol_model()
        .column_list()
        .expect("the list stays open")
        .cursor()
        .map(str::to_owned);
    assert_ne!(before, after, "`j` moves the list's cursor");
    assert_eq!(
        win.app.chart_doc().axis_lock,
        lock,
        "and the keys the list took moved nothing else"
    );
}

/// **AC1, the other half.** With the band not holding the keys `x` is the
/// axis lock's, as it was; and once the keys have been handed back it is
/// again.
#[test]
fn with_the_band_not_holding_the_keys_x_still_cycles_the_axis_lock() {
    let mut win = Window::housing();
    let start = win.app.chart_doc().axis_lock;
    assert_eq!(start, AxisLock::default());

    win.type_letter(egui::Key::X, "x");
    let cycled = win.app.chart_doc().axis_lock;
    assert_ne!(cycled, start, "`x` cycles the axis lock");
    assert_eq!(win.active(), None, "and opens no cell");
    assert_eq!(win.list_channel(), None);

    win.type_letter(egui::Key::E, "e");
    win.press(egui::Key::Escape);
    assert!(!win.app.shelf_holds_keys(), "`Esc` hands the keys back");
    win.type_letter(egui::Key::X, "x");
    assert_ne!(
        win.app.chart_doc().axis_lock,
        cycled,
        "with the keys handed back `x` is the axis lock's again"
    );
}

/// **AC2.** `Esc` steps back through the list and the band to the pane, one
/// level a press: the query, the list with its cell, the band.
#[test]
fn esc_steps_back_through_the_list_and_the_band_to_the_pane_one_level_a_press() {
    let mut win = Window::housing();
    win.type_letter(egui::Key::E, "e");
    win.type_letter(egui::Key::X, "x");
    assert_eq!(win.list_channel(), Some(ShelfChannel::X));

    // Level one: the query. `/` gives it the keys, and what is typed is text.
    win.type_letter(egui::Key::Slash, "/");
    assert!(win
        .app
        .protocol_model()
        .column_list()
        .expect("list open")
        .querying());
    win.run(vec![egui::Event::Text("lat".to_owned())]);
    win.run(Vec::new());
    assert_eq!(
        win.app
            .protocol_model()
            .column_list()
            .expect("open")
            .query(),
        "lat"
    );
    win.press(egui::Key::Escape);
    let list = win.app.protocol_model().column_list().expect("still open");
    assert_eq!(list.query(), "", "the first `Esc` clears the query");
    assert!(win.app.shelf_holds_keys());

    // Level two: the list, and the cell it was opened from.
    win.press(egui::Key::Escape);
    assert_eq!(win.list_channel(), None, "the next `Esc` closes the list");
    assert_eq!(win.active(), None, "and leaves the cell");
    assert!(
        win.app.shelf_holds_keys(),
        "but the band still holds the keys"
    );

    // Level three: the band, back to the pane.
    win.press(egui::Key::Escape);
    assert!(
        !win.app.shelf_holds_keys(),
        "the last `Esc` gives the pane the keys"
    );
    assert_eq!(win.active(), None);
}

/// **A click on a cell is the same way in as the key.** The cell opens, the
/// band holds the keys and the list opens on that channel.
#[test]
fn a_click_on_a_cell_opens_it_and_its_list() {
    let mut win = Window::housing();
    let drawn = win.app.shelf_drawn().expect("the band drew").clone();
    let at = drawn.cells[ShelfChannel::Y.index()].center();

    win.run(vec![egui::Event::PointerMoved(at)]);
    win.run(vec![button(at, true)]);
    win.run(vec![button(at, false)]);
    win.settle();

    assert_eq!(win.active(), Some(ShelfChannel::Y));
    assert!(
        win.app.shelf_holds_keys(),
        "the click gave the band the keys"
    );
    assert_eq!(win.list_channel(), Some(ShelfChannel::Y));
}

/// **The shelf lets go where the pane does.** Another pane taking focus closes
/// the list and hands the keys back.
#[test]
fn another_pane_taking_focus_gives_the_keys_back() {
    let mut win = Window::housing();
    win.type_letter(egui::Key::E, "e");
    win.type_letter(egui::Key::X, "x");
    assert_eq!(win.list_channel(), Some(ShelfChannel::X));

    assert!(
        win.app.focus_pane(PaneKey::new(DATA)),
        "the grid takes focus"
    );
    win.settle();
    assert!(!win.app.shelf_holds_keys());
    assert_eq!(win.list_channel(), None);
    assert_eq!(win.active(), None);
}

/// **AC3.** The band is 44 high and the pane's width, directly under the
/// hero's header, and the hero gives up that height. The pane's frame and
/// header, and the grid pane, are as they are with no band.
#[test]
fn the_band_is_44_high_under_the_heros_header_and_the_hero_gives_up_that_height() {
    let with = Window::open(true);
    let without = Window::open(false);
    assert!(without.app.shelf_drawn().is_none());
    let band = with.app.shelf_drawn().expect("the band drew").rect;
    let (a, b) = (with.map_pane(), without.map_pane());

    assert!((BAND_HEIGHT - 44.0).abs() < f32::EPSILON);
    assert_eq!(band.height(), 44.0, "the band is 44 high");
    assert_eq!(band.width(), b.body.width(), "and the pane's content width");
    assert_eq!(band.min, b.body.min, "directly under the pane's header");
    assert!(band.top() >= a.header.bottom(), "and under it, not in it");

    assert_eq!(a.rect, b.rect, "the pane's frame is the same");
    assert_eq!(a.header, b.header, "and its header is not taller");
    assert_eq!(
        a.body.top(),
        b.body.top() + 44.0,
        "the body starts below the band"
    );
    assert_eq!(a.body.bottom(), b.body.bottom());
    assert_eq!(
        a.body.height(),
        b.body.height() - 44.0,
        "the plot gives up 44"
    );

    let views = with.app.chart_doc().pane_views.as_ref().expect("views");
    assert_eq!(views.first, a.body, "the page is composed in what is left");

    let (ga, gb) = (
        with.app.canvas_panes().pane("grid").expect("grid pane"),
        without.app.canvas_panes().pane("grid").expect("grid pane"),
    );
    assert_eq!(ga, gb, "the grid pane is laid out as before");
}

/// Every filled rect a frame painted, flattened in paint order, with its fill.
fn collect_fills(shape: &egui::Shape, out: &mut Vec<(egui::Rect, egui::Color32)>) {
    match shape {
        egui::Shape::Rect(rect) if rect.fill.a() > 0 => out.push((rect.rect, rect.fill)),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_fills(shape, out);
            }
        }
        _ => {}
    }
}

/// `over` laid on `ground`, as the painter composites a premultiplied fill.
fn composite(over: egui::Color32, ground: egui::Color32) -> [u8; 3] {
    let keep = 1.0 - f32::from(over.a()) / 255.0;
    let mix = |o: u8, g: u8| {
        (f32::from(o) + f32::from(g) * keep)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    [
        mix(over.r(), ground.r()),
        mix(over.g(), ground.g()),
        mix(over.b(), ground.b()),
    ]
}

/// **The ground each of the band's cells is painted on**, as a live window
/// draws it: the cell's fill laid over the band's own, read off the shapes a
/// headless frame handed the painter.
fn cell_grounds() -> Vec<[u8; 3]> {
    let path = housing();
    let boot = Boot::data_file(path.to_str().expect("utf-8 path")).expect("the sample opens");
    let mut app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
    let ctx = egui::Context::default();
    let raw = |events| egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1440.0, 900.0),
        )),
        events,
        ..Default::default()
    };
    let mut shapes = Vec::new();
    for _ in 0..4 {
        let out = ctx.run_ui(raw(Vec::new()), |ui| app.draw(ui));
        shapes = out.shapes.into_iter().map(|c| c.shape).collect();
    }
    let drawn = app.shelf_drawn().expect("the live window draws the band");
    let mut fills = Vec::new();
    shapes.iter().for_each(|s| collect_fills(s, &mut fills));
    let at = |rect: egui::Rect| {
        fills
            .iter()
            .rev()
            .find(|(r, _)| *r == rect)
            .map(|(_, fill)| *fill)
            .expect("the band painted a fill over that rect")
    };
    let ground = at(drawn.rect);
    ShelfChannel::ALL
        .iter()
        .map(|c| composite(at(drawn.cells[c.index()]), ground))
        .collect()
}

/// How many pixels of `image` are within `slack` of `colour` on each channel.
fn pixels_of(image: &image::RgbaImage, colour: [u8; 3], slack: u8) -> usize {
    image
        .pixels()
        .filter(|p| (0..3).all(|i| p.0[i].abs_diff(colour[i]) <= slack))
        .count()
}

/// **AC4.** A headless render of the dashboard, as `brightfield-shot` makes it,
/// draws no band: no ground a live window gives the mark, x and y cells is on
/// the page. A cell is the width of a third of the pane or more and 44 high, so
/// a band would put thousands of pixels on each; the few that match by chance
/// are the allowance. The colour cell is left out of the count: its ground is
/// the warm off-white other surfaces of the window share.
#[test]
fn a_headless_capture_of_the_dashboard_draws_no_band() {
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let grounds = cell_grounds();
    let path = housing();
    let boot = Boot::data_file(path.to_str().expect("utf-8 path")).expect("the sample opens");
    let out =
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("shelf_band_window.capture.png");
    capture_png(boot, Mode::Light, 1.0, &out, Vec::new()).expect("the capture ran");
    let image = image::open(&out)
        .expect("the capture reads back")
        .to_rgba8();

    for channel in [ShelfChannel::Mark, ShelfChannel::X, ShelfChannel::Y] {
        let ground = grounds[channel.index()];
        let matched = pixels_of(&image, ground, 2);
        assert!(
            matched < 200,
            "{} pixels of the capture are the {} cell's ground {ground:?}: \
             the capture drew a band",
            matched,
            channel.word()
        );
    }
}
