//! **A click on a part of an axis opens that axis's settings on the row the part
//! belongs to.**
//!
//! The title goes to `title`, the tick labels to `format` and the axis line to
//! `range`, the way `x` or `y`, `Tab` and the `j`s down to the row would. The
//! parts are the rects the render crate reports from the axes it drew, carried
//! on the plot's handle; these tests click the middle of each in a headless
//! window over the housing sample and read the list the window then holds.
//!
//! Three things keep a click the canvas's. A press where a mark is under the
//! pointer is the mark's, even inside the axis line's reach into the data area.
//! A window that draws no shelf band has no axes that are targets. And a press
//! on no axis part is not heard by the settings at all.

use brightfield_render::channel::Channel;
use brightfield_shell::app::CHART;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::{live_spec, AxisPart, PlotHandle};
use brightfield_shell::shelf::{ColumnList, ListTab, FORMAT_ROW, RANGE_ROW, TITLE_ROW};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::arrangement;
use brightfield_workbench::channel::ShelfChannel;
use brightfield_workbench::PaneKey;

fn housing() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv")
}

/// The crossfilter example, whose hero is a dot plot with an x-range brush. A
/// window over it holds a chart and no data grid, so it carves no shelf band.
fn crossfilter_boot(live_dashboard: bool) -> Boot {
    let path = crossfilter_path();
    let (live, composed) = crossfilter_live();
    let mut boot = Boot::charts(composed);
    if live_dashboard {
        boot.live = Some(live);
        boot.spec_path = Some(path);
    }
    boot
}

fn crossfilter_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/crossfilter.yaml")
}

/// The crossfilter example loaded live: the session behind it and the composed
/// page.
fn crossfilter_live() -> (
    brightfield_shell::pipeline::LiveDashboard,
    brightfield_shell::pipeline::Composed,
) {
    live_spec(crossfilter_path().to_str().expect("utf-8 path")).expect("the example loads live")
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

fn button(at: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// One headless window over the housing sample, the hero's pane focused, and the
/// hero turned to a plot with axes by keeping `median_income` on x.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    fn open() -> Self {
        let boot =
            Boot::data_file(housing().to_str().expect("utf-8 path")).expect("the sample opens");
        let mut win = Self::over(boot);
        win.keep_income_on_x();
        win
    }

    /// One headless window over `boot`, settled, the hero's pane focused.
    fn over(boot: Boot) -> Self {
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
        };
        win.settle();
        assert!(
            win.app.focus_pane(PaneKey::new(CHART)),
            "the pane takes focus"
        );
        win.settle();
        win
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

    fn type_letter(&mut self, key: egui::Key, text: &str) {
        self.run(vec![key_down(key), egui::Event::Text(text.to_owned())]);
        self.run(Vec::new());
    }

    fn press(&mut self, key: egui::Key) {
        self.run(vec![key_down(key)]);
        self.run(Vec::new());
    }

    /// The pointer moved to `at` and the frames after it settled.
    fn point(&mut self, at: egui::Pos2) {
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.settle();
    }

    /// A click at `at`: the pointer moved there, pressed, released, and the
    /// frames after it settled.
    fn click(&mut self, at: egui::Pos2) {
        self.point(at);
        self.run(vec![button(at, true)]);
        self.run(vec![button(at, false)]);
        self.settle();
    }

    /// `e`, `x`, up to `median_income`, `Enter`: a plot with axes to click on.
    fn keep_income_on_x(&mut self) {
        self.type_letter(egui::Key::E, "e");
        self.type_letter(egui::Key::X, "x");
        for _ in 0..12 {
            if self
                .list()
                .is_some_and(|l| l.cursor() == Some("median_income"))
            {
                break;
            }
            self.type_letter(egui::Key::K, "k");
        }
        self.press(egui::Key::Enter);
        self.settle();
        assert!(self.list().is_none(), "keeping a column closes the list");
    }

    fn list(&self) -> Option<&ColumnList> {
        self.app.protocol_model().column_list()
    }

    fn hero(&self) -> &PlotHandle {
        &self.app.chart_doc().composed.plots[0]
    }

    /// The hero's placed tile in window space: its top-left corner, to which
    /// the tile-local rects the render crate reports are added.
    fn tile_origin(&self) -> egui::Pos2 {
        self.app
            .composed_plot_rects()
            .first()
            .expect("the hero drew")
            .min
    }

    /// A point in window space from one in the hero's tile.
    fn at(&self, local: kurbo::Point) -> egui::Pos2 {
        self.tile_origin() + egui::vec2(local.x as f32, local.y as f32)
    }

    /// The point to click for `part` of `channel`'s axis: the middle of the
    /// part's rect, except the axis line's, which is taken outside the data
    /// area, on the tick marks' side, where no mark can be under the pointer.
    fn part(&self, channel: Channel, part: AxisPart) -> egui::Pos2 {
        let axes = self.hero().axes;
        let axis = match channel {
            Channel::X => axes.x,
            _ => axes.y,
        }
        .expect("the hero drew that axis");
        let rect = match part {
            AxisPart::Title => axis.title.expect("the axis is titled"),
            AxisPart::Labels => axis.labels.expect("the axis is labelled"),
            AxisPart::Line => axis.line,
        };
        let mut at = rect.center();
        if part == AxisPart::Line {
            // x's line strip runs from 8 px inside the data area to 5 px below
            // it, and y's from 8 px left of it to 8 px right.
            match channel {
                Channel::X => at.y = rect.y1 - 2.0,
                _ => at.x = rect.x0 + 3.0,
            }
        }
        self.at(at)
    }

    fn open_row(&self) -> Option<(ShelfChannel, ListTab, &'static str)> {
        let list = self.list()?;
        Some((list.channel(), list.tab(), list.setting_cursor()?.name))
    }

    /// Two points three pixels inside the data area along x's line, found by
    /// resting the pointer along it: one where the hover reads a dot, and one
    /// where it reads none.
    fn dot_and_gap_on_x_line(&mut self) -> (egui::Pos2, egui::Pos2) {
        let axis = self.hero().axes.x.expect("x drew an axis");
        let y = axis.line.y1 - 5.0 - 3.0;
        let mut on_mark = None;
        let mut off_mark = None;
        let mut x = axis.line.x0 + 2.0;
        while x < axis.line.x1 && (on_mark.is_none() || off_mark.is_none()) {
            let at = self.at(kurbo::Point::new(x, y));
            self.point(at);
            let read = self.app.chart_doc().hover_readout.is_some();
            match (read, on_mark.is_some(), off_mark.is_some()) {
                (true, false, _) => on_mark = Some(at),
                (false, _, false) => off_mark = Some(at),
                _ => {}
            }
            x += 6.0;
        }
        (
            on_mark.expect("fixture check: a dot lies 3 px inside the data area at the x line"),
            off_mark.expect("fixture check: some of that strip has no dot under it"),
        )
    }

    /// A press at `press` and a drag into the plot, the button left down: the
    /// point dragged to, and the ink the canvas's brush records for the sweep,
    /// which is `None` when the press never reached the canvas.
    fn sweep_from(&mut self, press: egui::Pos2) -> (egui::Pos2, Option<egui::Rect>) {
        let area = self.hero().data_area();
        let tile = self.hero().rect;
        let to = self.tile_origin()
            + egui::vec2(
                (area.x - tile.x + area.width * 0.75) as f32,
                (area.y - tile.y + area.height * 0.5) as f32,
            );
        let midway = press + (to - press) * 0.5;
        self.run(vec![egui::Event::PointerMoved(press)]);
        self.run(vec![button(press, true)]);
        self.run(vec![egui::Event::PointerMoved(midway)]);
        self.run(vec![egui::Event::PointerMoved(to)]);
        // Read with the button still down: the ink is an uncommitted sweep's.
        (to, self.app.chart_doc().gesture_ink)
    }
}

/// A click on x's tick labels opens x's settings on format, on its title on
/// title, and on its axis line on range, with no list open before it.
#[test]
fn a_click_on_each_part_of_x_opens_its_row() {
    for (part, row) in [
        (AxisPart::Labels, FORMAT_ROW),
        (AxisPart::Title, TITLE_ROW),
        (AxisPart::Line, RANGE_ROW),
    ] {
        let mut win = Window::open();
        let at = win.part(Channel::X, part);
        win.click(at);
        assert_eq!(
            win.open_row(),
            Some((ShelfChannel::X, ListTab::Settings, row)),
            "a click on x's {part:?} at {at:?}"
        );
    }
}

/// The same for y, whose parts are laid out the other way round: its title is
/// rotated, its labels are right-aligned and its line's strip reaches right.
#[test]
fn a_click_on_each_part_of_y_opens_its_row() {
    for (part, row) in [
        (AxisPart::Labels, FORMAT_ROW),
        (AxisPart::Title, TITLE_ROW),
        (AxisPart::Line, RANGE_ROW),
    ] {
        let mut win = Window::open();
        let at = win.part(Channel::Y, part);
        win.click(at);
        assert_eq!(
            win.open_row(),
            Some((ShelfChannel::Y, ListTab::Settings, row)),
            "a click on y's {part:?} at {at:?}"
        );
    }
}

/// With the navigator rail shut the list is the card hung from the cell, and a
/// click on an axis part opens it the same way.
#[test]
fn with_the_rail_shut_the_hung_card_opens_on_the_same_row() {
    let mut win = Window::open();
    let collapse = win
        .app
        .rail_collapse_rect(arrangement::NAVIGATOR_RAIL)
        .expect("the navigator rail drew a collapse control")
        .center();
    win.click(collapse);
    assert!(
        win.app.rail_is_collapsed(arrangement::NAVIGATOR_RAIL),
        "fixture check: the rail is shut"
    );
    win.settle();

    let at = win.part(Channel::X, AxisPart::Labels);
    win.click(at);
    assert_eq!(
        win.open_row(),
        Some((ShelfChannel::X, ListTab::Settings, FORMAT_ROW))
    );
    assert!(
        win.app.shelf_card_drawn().is_some(),
        "with the rail shut the list is drawn as the card"
    );
}

/// Where the axis line's strip reaches into the data area, a mark under the
/// pointer takes the click: a pointer resting on a mark reads it out, and a click
/// there opens nothing, while the same strip where no mark is opens `range`.
#[test]
fn a_mark_in_the_axis_lines_reach_takes_the_click_and_the_settings_stay_shut() {
    let mut win = Window::open();
    let (on_mark, off_mark) = win.dot_and_gap_on_x_line();

    win.click(on_mark);
    assert_eq!(
        win.open_row(),
        None,
        "a click on a dot 3 px inside the data area at {on_mark:?} opened settings"
    );
    win.click(off_mark);
    assert_eq!(
        win.open_row(),
        Some((ShelfChannel::X, ListTab::Settings, RANGE_ROW)),
        "the control: the same strip with no dot under the pointer at {off_mark:?}"
    );
}

/// The other half of the same rule: a press on that dot is the canvas's, so the
/// canvas hears it. A drag from the dot records the canvas's brush ink, which an
/// axis press never does, and then the same drag from the strip where no dot is
/// records none.
#[test]
fn a_press_on_a_mark_in_the_axis_lines_reach_starts_the_canvas_brush() {
    let mut win = Window::open();
    let (on_mark, off_mark) = win.dot_and_gap_on_x_line();

    let (to, ink) = win.sweep_from(off_mark);
    assert_eq!(
        ink, None,
        "the control: a press on the strip where no dot is at {off_mark:?} is an axis press, which starts no brush"
    );
    win.run(vec![button(to, false)]);
    win.settle();

    let (to, ink) = win.sweep_from(on_mark);
    let ink = ink.expect("a press on a dot 3 px inside the data area started the canvas's brush");
    assert!(
        ink.width() > 1.0,
        "the drag from {on_mark:?} to {to:?} recorded {ink:?}, which does not follow the pointer"
    );
    win.run(vec![button(to, false)]);
}

/// A window that draws no shelf band has no axes that are targets, and a click
/// where the axis line's strip is opens nothing; draw the band and the same click
/// opens `range`.
#[test]
fn with_no_band_drawn_the_axes_are_not_targets() {
    let mut win = Window::open();
    let at = win.part(Channel::X, AxisPart::Line);

    win.app.set_shelf_band_drawn(false);
    win.settle();
    assert!(!win.app.chart_doc().axis_targets_live);
    win.click(at);
    assert_eq!(win.open_row(), None, "a click opened settings with no band");

    win.app.set_shelf_band_drawn(true);
    win.settle();
    assert!(win.app.chart_doc().axis_targets_live);
    win.click(at);
    assert_eq!(
        win.open_row(),
        Some((ShelfChannel::X, ListTab::Settings, RANGE_ROW)),
        "the control: the same click with the band drawn"
    );
}

/// A document with no live dashboard behind it, a picture shown as published,
/// has no band and no axes that are targets: a click on x's tick labels opens
/// nothing. The handle reports a part at the point clicked, so the click is aimed
/// at a real part and the absence of a band is what keeps it the canvas's.
#[test]
fn a_document_with_no_live_dashboard_has_no_axis_targets() {
    let mut win = Window::over(crossfilter_boot(false));
    assert!(
        win.app.chart_doc().live_dashboard().is_none(),
        "fixture check: nothing is live behind this document"
    );

    let hero = win.hero();
    let labels = hero.axes.x.and_then(|x| x.labels).expect("x drew labels");
    let page = kurbo::Point::new(
        hero.rect.x + labels.center().x,
        hero.rect.y + labels.center().y,
    );
    assert_eq!(
        hero.axis_part_at(page).map(|hit| (hit.channel, hit.part)),
        Some((Channel::X, AxisPart::Labels)),
        "fixture check: the point clicked is on x's labels"
    );

    let at = win.part(Channel::X, AxisPart::Labels);
    win.click(at);
    assert_eq!(win.open_row(), None, "a click opened settings at {at:?}");
    assert!(
        !win.app.chart_doc().axis_targets_live,
        "the axes are targets where a band is drawn, and none is drawn here"
    );
}

/// With no band drawn, a press on an axis part is the canvas's as it is today: a
/// press on x's tick labels and a drag into the plot sweeps the plot's x-range
/// brush. Were the axes targets without a band, the press would be an axis press,
/// which starts no brush, and the drag would record no ink.
#[test]
fn with_no_band_drawn_a_press_on_an_axis_part_starts_the_canvas_brush() {
    let mut win = Window::over(crossfilter_boot(true));
    win.app.set_shelf_band_drawn(false);
    win.settle();
    assert!(
        win.app.chart_doc().live_dashboard().is_some(),
        "fixture check: a live dashboard, so a press can start a brush"
    );
    assert!(!win.app.chart_doc().axis_targets_live);

    let press = win.part(Channel::X, AxisPart::Labels);
    let (to, ink) = win.sweep_from(press);
    let ink = ink.expect("a press on x's labels started the canvas's brush");
    assert!(
        ink.width() > 1.0,
        "the drag from {press:?} to {to:?} recorded {ink:?}, which does not follow the pointer"
    );
    win.run(vec![button(to, false)]);
    win.settle();
    assert_eq!(
        win.open_row(),
        None,
        "a sweep from an axis part opened settings"
    );
}

/// A window that drew the band and then takes a picture that draws none hands
/// the picture's axes back to the canvas. The band is carved by the pane-group
/// layouts only, so a frame of one picture never reaches the code that sets the
/// flag, and a flag left standing from the banded frame would have a press on
/// x's tick labels taken as an axis press the canvas never hears: no brush.
/// The document is swapped in the two calls a start opening makes.
#[test]
fn a_bandless_picture_opened_in_a_banded_window_keeps_the_canvas_brush() {
    let mut win = Window::open();
    assert!(
        win.app.chart_doc().axis_targets_live && win.app.shelf_drawn().is_some(),
        "fixture check: the window drew the band, so the axes were targets"
    );

    let (live, composed) = crossfilter_live();
    win.app.open_chart(composed);
    win.app.chart_doc_mut().attach_live(live);
    win.settle();
    assert!(
        win.app.shelf_drawn().is_none() && win.app.chart_doc().stacked_tiles().is_none(),
        "fixture check: the swapped-in picture is one pane and draws no band"
    );
    assert!(
        !win.app.chart_doc().axis_targets_live,
        "the axes were targets with no band drawn after the swap"
    );

    let press = win.part(Channel::X, AxisPart::Labels);
    let (to, ink) = win.sweep_from(press);
    let ink = ink.expect("a press on x's labels started the canvas's brush after the swap");
    assert!(
        ink.width() > 1.0,
        "the drag from {press:?} to {to:?} recorded {ink:?}, which does not follow the pointer"
    );
    win.run(vec![button(to, false)]);
    win.settle();
    assert_eq!(
        win.open_row(),
        None,
        "a sweep from an axis part opened settings"
    );
}

/// A click that lands on no axis part, in the middle of the plot's data area
/// where no part reaches, opens no settings.
#[test]
fn a_click_on_no_axis_part_opens_no_settings() {
    let mut win = Window::open();
    let area = win.hero().data_area();
    let centre = win.tile_origin()
        + egui::vec2(
            (area.x - win.hero().rect.x + area.width / 2.0) as f32,
            (area.y - win.hero().rect.y + area.height / 2.0) as f32,
        );
    win.click(centre);
    assert_eq!(win.open_row(), None);
}

/// The hit test on the handle: the parts are tried x's title, labels and line,
/// then y's, and a point on none answers none.
#[test]
fn the_handle_names_the_part_under_a_point() {
    let win = Window::open();
    let hero = win.hero();
    let origin = kurbo::Point::new(hero.rect.x, hero.rect.y);
    let axes = hero.axes;
    let x = axes.x.expect("x drew an axis");
    let y = axes.y.expect("y drew an axis");
    let page = |p: kurbo::Point| kurbo::Point::new(origin.x + p.x, origin.y + p.y);

    let hit = |p: kurbo::Point| hero.axis_part_at(page(p));
    let labels = hit(x.labels.expect("labels").center()).expect("a hit on x's labels");
    assert_eq!(
        (labels.channel, labels.part),
        (Channel::X, AxisPart::Labels)
    );
    assert!(!labels.in_data_area);
    let title = hit(x.title.expect("title").center()).expect("a hit on x's title");
    assert_eq!((title.channel, title.part), (Channel::X, AxisPart::Title));
    let top = hit(kurbo::Point::new(x.line.center().x, x.line.y0 + 1.0)).expect("x's reach");
    assert_eq!((top.channel, top.part), (Channel::X, AxisPart::Line));
    assert!(top.in_data_area, "the line's reach is inside the data area");
    let left = hit(y.labels.expect("labels").center()).expect("a hit on y's labels");
    assert_eq!((left.channel, left.part), (Channel::Y, AxisPart::Labels));
    let corner = hit(kurbo::Point::new(x.line.x0 + 1.0, x.line.y0 + 1.0)).expect("the corner");
    assert_eq!(
        (corner.channel, corner.part),
        (Channel::X, AxisPart::Line),
        "where the two lines' reaches overlap, x has it"
    );
    assert_eq!(hit(kurbo::Point::new(-5.0, -5.0)), None);
}
