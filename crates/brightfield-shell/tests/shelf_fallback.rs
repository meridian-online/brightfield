//! **With the navigator rail shut, a shelf cell hangs its column list from
//! itself.**
//!
//! The list of columns a shelf cell opens is the Outline's, and the Outline
//! lives in the navigator rail. With the rail shut nothing draws the Outline,
//! so the list is drawn as a card: 320 wide, from the open cell down over the
//! plot, in the floating card's frame. Its keys are the list's own, and a click
//! anywhere off the card backs out as `Esc` does. With the rail open the
//! Outline draws the list and no card is hung.
//!
//! Every assertion reads a frame the window drew: the card's drawn record, the
//! band's cells, the text and the shapes the frame painted, and the state the
//! window holds after the keys and clicks. The card's pixels are two baselines,
//! one a mode, drawn through `egui_kittest`'s wgpu renderer and compared under
//! `kittest.toml`'s thresholds. Regenerate them with `UPDATE_SNAPSHOTS=1 cargo
//! +1.95.0 test -p brightfield-shell --test shelf_fallback`, and read what
//! moved before committing it.

use brightfield_render::channel::Channel;
use brightfield_shell::app::CHART;
use brightfield_shell::design::{self, Mode};
use brightfield_shell::one_step::ColumnFacts;
use brightfield_shell::protocol::SpineRole;
use brightfield_shell::shelf::{
    Binding, CardDrawn, ChannelSettings, ColumnList, ColumnListRequest, ListColumn, ListTab,
    ShelfChannels, QUERY_PLACEHOLDER,
};
use brightfield_shell::startup::default_layout;
use brightfield_shell::text_ink::{self, DrawnText};
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::edit::plot_at_path;
use brightfield_spec::parse::{parse_spec, Format};
use brightfield_workbench::arrangement;
use brightfield_workbench::channel::{self, ShelfChannel};
use brightfield_workbench::chrome;
use brightfield_workbench::PaneKey;
use egui::epaint::{ClippedShape, RectShape, Shape};
use egui_kittest::{Harness, SnapshotOptions};
use meridian_design::{semantic, spacing, Elevation};

/// How wide the design puts the card, in points.
const CARD_WIDTH: f32 = 320.0;

/// A window narrow enough that the colour cell, the band's last, has less than a
/// card's width to its right.
const NARROW: f32 = 450.0;

/// How near two edges must be to be the same edge: egui lays a card out on the
/// pixel grid, so an edge can sit half a point off the cell it hangs from.
const EDGE: f32 = 0.51;

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

fn button(at: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= EDGE
}

/// Every rectangle `shapes` paint, the nested ones included.
fn rects(shapes: &[ClippedShape]) -> Vec<RectShape> {
    fn walk(shape: &Shape, out: &mut Vec<RectShape>) {
        match shape {
            Shape::Rect(rect) => out.push(rect.clone()),
            Shape::Vec(inner) => inner.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

/// One headless window over the housing sample, the hero pane focused.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    /// The text the frame before drew.
    texts: Vec<DrawnText>,
    /// The shapes the frame before painted.
    shapes: Vec<ClippedShape>,
}

impl Window {
    fn open(mode: Mode) -> Self {
        Self::open_at(mode, 900.0)
    }

    /// [`Self::open`] in a window `height` points high.
    fn open_at(mode: Mode, height: f32) -> Self {
        Self::open_sized(mode, 1440.0, height)
    }

    /// [`Self::open`] in a window `width` across and `height` high.
    fn open_sized(mode: Mode, width: f32, height: f32) -> Self {
        let path = housing();
        let boot = Boot::data_file(path.to_str().expect("utf-8 path")).expect("the sample opens");
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), mode),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height)),
            texts: Vec::new(),
            shapes: Vec::new(),
        };
        win.settle();
        assert!(
            win.app.focus_pane(PaneKey::new(CHART)),
            "the dashboard's pane takes focus"
        );
        win.settle();
        win
    }

    /// A window whose navigator rail is shut, as a click on its collapse
    /// control shuts it.
    fn rail_shut(mode: Mode) -> Self {
        let mut win = Self::open(mode);
        win.shut_the_rail();
        win
    }

    /// [`Self::rail_shut`] in a window `width` across and `height` high.
    fn rail_shut_sized(mode: Mode, width: f32, height: f32) -> Self {
        let mut win = Self::open_sized(mode, width, height);
        win.shut_the_rail();
        win
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            events,
            ..Default::default()
        };
        let mut texts = Vec::new();
        let output = self.ctx.run_ui(raw, |ui| {
            self.app.draw(ui);
            texts = text_ink::frame_text(ui.ctx());
        });
        self.texts = texts;
        self.shapes = output.shapes;
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    /// Press a letter, as a keyboard brings it, and let the frame after run.
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

    /// Click the navigator rail's collapse control, which shuts it open and
    /// opens it shut.
    fn shut_the_rail(&mut self) {
        assert!(
            !self.app.rail_is_collapsed(arrangement::NAVIGATOR_RAIL),
            "the navigator rail starts open"
        );
        let at = self
            .app
            .rail_collapse_rect(arrangement::NAVIGATOR_RAIL)
            .expect("the navigator rail drew a collapse control")
            .center();
        self.click(at);
        assert!(
            self.app.rail_is_collapsed(arrangement::NAVIGATOR_RAIL),
            "a click on the collapse control did not shut the navigator rail"
        );
    }

    /// `e` and then the channel's letter: the keys that open a cell.
    fn open_cell(&mut self, key: egui::Key, letter: &str) {
        self.type_letter(egui::Key::E, "e");
        self.type_letter(key, letter);
    }

    fn list_channel(&self) -> Option<ShelfChannel> {
        self.app.protocol_model().column_list().map(|l| l.channel())
    }

    fn cursor(&self) -> Option<String> {
        self.app
            .protocol_model()
            .column_list()
            .and_then(|l| l.cursor().map(str::to_owned))
    }

    fn active(&self) -> Option<ShelfChannel> {
        self.app.shelf_band().and_then(|b| b.active())
    }

    /// The card the last frame drew.
    fn card(&self) -> CardDrawn {
        self.app
            .shelf_card_drawn()
            .expect("the last frame hung a card")
            .clone()
    }

    /// The band's cell for `channel`, as the last frame drew it.
    fn band_cell(&self, channel: ShelfChannel) -> egui::Rect {
        self.app.shelf_drawn().expect("the band drew").cells[channel.index()]
    }

    /// The hero's plot as the frame drew it, in window space.
    fn hero_plot(&self) -> egui::Rect {
        *self
            .app
            .composed_plot_rects()
            .first()
            .expect("the dashboard drew its hero's plot")
    }

    /// The text the last frame drew inside `rect`.
    fn text_in(&self, rect: egui::Rect) -> Vec<String> {
        self.texts
            .iter()
            .filter(|t| rect.contains_rect(t.visible) && t.visible.is_positive())
            .map(|t| t.text.clone())
            .collect()
    }

    /// The point in the hero's plot farthest from the card, where a click lands
    /// on nothing the card covers.
    fn far_from_the_card(&self) -> egui::Pos2 {
        let plot = self.hero_plot();
        let card = self.card().rect;
        assert!(
            !card.contains(plot.right_bottom() - egui::vec2(20.0, 20.0)),
            "the plot's far corner is under the card"
        );
        plot.right_bottom() - egui::vec2(20.0, 20.0)
    }
}

/// The state a window is left in after a list is backed out of, in the terms
/// the window holds it: the open cell, the open list, whether the band keeps
/// the keys, whether a card is hung and whether the hero is drawn with a
/// column it has not kept.
fn left_after_backing_out(
    win: &Window,
) -> (Option<ShelfChannel>, Option<ShelfChannel>, bool, bool, bool) {
    (
        win.active(),
        win.list_channel(),
        win.app.shelf_holds_keys(),
        win.app.shelf_card_drawn().is_some(),
        win.app.chart_doc().shelf_preview().is_some(),
    )
}

// ---------------------------------------------------------------------------
// AC1: the list, hung from the cell.
// ---------------------------------------------------------------------------

/// **The control.** With the rail open the Outline draws the list and no card
/// is hung, so the list is on screen once and not twice.
#[test]
fn with_the_rail_open_the_outline_draws_the_list_and_no_card_is_hung() {
    let mut win = Window::open(Mode::Light);
    assert!(!win.app.rail_is_collapsed(arrangement::NAVIGATOR_RAIL));
    win.open_cell(egui::Key::X, "x");
    assert_eq!(
        win.list_channel(),
        Some(ShelfChannel::X),
        "x's list is open"
    );
    assert!(
        win.app.shelf_card_drawn().is_none(),
        "a card is hung while the Outline is open to draw the list"
    );
    assert!(
        win.app
            .spine_rows()
            .iter()
            .any(|r| r.label.starts_with("OUTLINE") && r.label.contains("x axis of")),
        "the Outline drew the list's heading"
    );
}

/// **AC1.** `e x` with the rail shut hangs the list as a card of the designed
/// width, 320 points, whose top left is the x cell's bottom left, over the
/// hero's plot.
#[test]
fn with_the_rail_shut_e_x_hangs_the_list_from_the_x_cell_over_the_plot_as_a_card_of_the_designed_width(
) {
    let mut win = Window::rail_shut(Mode::Light);
    assert!(
        win.app.shelf_card_drawn().is_none(),
        "no cell is open, so no card is hung"
    );

    win.open_cell(egui::Key::X, "x");
    assert_eq!(win.active(), Some(ShelfChannel::X), "`x` opens the x cell");
    assert_eq!(win.list_channel(), Some(ShelfChannel::X));

    let card = win.card();
    let cell = win.band_cell(ShelfChannel::X);
    assert_eq!(card.cell, cell, "the card says which cell it hangs from");
    assert!(
        near(card.rect.width(), CARD_WIDTH),
        "the card is {} wide, not {CARD_WIDTH}",
        card.rect.width()
    );
    assert!(
        near(card.rect.left(), cell.left()) && near(card.rect.top(), cell.bottom()),
        "the card's top left is {:?}, not the x cell's bottom left {:?}",
        card.rect.left_top(),
        cell.left_bottom()
    );
    assert!(
        card.rect.intersects(win.hero_plot()),
        "the card {:?} lies clear of the hero's plot {:?}",
        card.rect,
        win.hero_plot()
    );
    assert!(
        win.screen.contains_rect(card.rect),
        "the card runs off the window"
    );
}

/// **AC1, each cell.** The card hangs from the cell that is open: `l` carries
/// the list to y and the card with it. In a window narrow enough that a card
/// 320 wide hung from the colour cell, the band's last, would run off the right
/// edge, the card is kept inside the window.
#[test]
fn the_card_hangs_from_whichever_cell_is_open_and_stays_inside_the_window() {
    let mut win = Window::rail_shut(Mode::Light);
    win.open_cell(egui::Key::X, "x");
    assert!(near(
        win.card().rect.left(),
        win.band_cell(ShelfChannel::X).left()
    ));

    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.list_channel(), Some(ShelfChannel::Y), "`l` goes to y");
    let card = win.card();
    assert_eq!(card.cell, win.band_cell(ShelfChannel::Y));
    assert!(
        near(card.rect.left(), win.band_cell(ShelfChannel::Y).left()),
        "the card did not follow the list to the y cell"
    );

    let mut narrow = Window::rail_shut_sized(Mode::Light, NARROW, 900.0);
    narrow.open_cell(egui::Key::C, "c");
    assert_eq!(narrow.list_channel(), Some(ShelfChannel::Colour));
    let colour = narrow.band_cell(ShelfChannel::Colour);
    assert!(
        colour.left() + CARD_WIDTH > narrow.screen.right(),
        "the colour cell at {colour:?} leaves room for the card in a window {NARROW} across, so \
         this window does not test the card staying inside it"
    );
    let card = narrow.card();
    assert!(near(card.rect.width(), CARD_WIDTH));
    assert!(
        narrow.screen.contains_rect(card.rect),
        "the card hung from the colour cell runs off a window {NARROW} across: {:?}",
        card.rect
    );
}

/// **AC1, what is in the card.** The list the Outline would draw: the same
/// heading, the same columns in the same order, a query line, and the keys the
/// list answers. Read against a second window whose rail is open, which draws
/// the list in the Outline.
#[test]
fn the_card_holds_the_list_the_outline_would_draw() {
    let mut open = Window::open(Mode::Light);
    open.open_cell(egui::Key::X, "x");
    let outline: Vec<(String, String)> = open
        .app
        .spine_rows()
        .iter()
        .filter(|r| r.label.starts_with("OUTLINE") || r.role == SpineRole::Column)
        .map(|r| (r.label.clone(), r.kind.clone()))
        .collect();
    assert!(
        outline.len() > 2,
        "the Outline drew the heading and columns"
    );

    let mut shut = Window::rail_shut(Mode::Light);
    shut.open_cell(egui::Key::X, "x");
    let card = shut.card();
    let mut hung: Vec<(String, String)> = vec![(card.list.heading_text.clone(), String::new())];
    hung.extend(
        card.list
            .rows
            .iter()
            .map(|r| (r.column.clone(), r.kind.clone().unwrap_or_default())),
    );
    assert_eq!(
        hung, outline,
        "the card's heading and rows are not the Outline's"
    );

    let words = shut.text_in(card.rect);
    assert!(
        card.rect.contains_rect(card.list.query),
        "the query line is outside the card"
    );
    for want in [
        QUERY_PLACEHOLDER,
        "search",
        "move",
        "channel",
        "keep",
        "back",
    ] {
        assert!(
            words.iter().any(|w| w == want),
            "the card drew no {want:?}: it drew {words:?}"
        );
    }
}

/// **AC1, the keys.** The list's keys act on the card as they do in the
/// Outline: `j` moves the cursor, `/` types into the query and narrows the
/// rows, `Esc` clears the query and then backs out, and `Enter` keeps the
/// column under the cursor on the cell's channel.
#[test]
fn the_lists_keys_act_on_the_card_as_they_do_in_the_outline() {
    let mut win = Window::rail_shut(Mode::Light);
    win.open_cell(egui::Key::X, "x");
    let before = win.cursor().expect("the list has a cursor");
    win.type_letter(egui::Key::J, "j");
    let after = win.cursor().expect("the cursor is still on a column");
    assert_ne!(before, after, "`j` did not move the list's cursor");
    let barred: Vec<String> = win
        .card()
        .list
        .rows
        .iter()
        .filter(|r| r.bar.is_some())
        .map(|r| r.column.clone())
        .collect();
    assert_eq!(
        barred,
        vec![after.clone()],
        "the card does not wear its cursor's bar on the row the cursor is on"
    );

    win.type_letter(egui::Key::Slash, "/");
    win.run(vec![egui::Event::Text("lat".to_owned())]);
    win.run(Vec::new());
    let card = win.card();
    assert_eq!(
        card.list.rows.first().map(|r| r.column.as_str()),
        Some("latitude"),
        "the query did not bring the match to the top of the card"
    );
    assert!(
        card.list.divider.is_some(),
        "the card draws no divider between the matches and the rest"
    );

    win.press(egui::Key::Escape);
    assert!(
        win.app.shelf_card_drawn().is_some(),
        "the first `Esc` cleared the query and left the card"
    );
    assert_eq!(
        win.app
            .protocol_model()
            .column_list()
            .expect("open")
            .query(),
        ""
    );

    // With the query cleared the cursor is back on the column x holds; `j`
    // moves it off, and the hero draws the column it lands on as a preview,
    // which is not an edit until `Enter` keeps it.
    win.type_letter(egui::Key::J, "j");
    let chosen = win.cursor().expect("the cursor is on a column");
    assert!(
        !win.app.chart_doc().has_unsaved_edit(),
        "a column under the cursor is a preview and not an edit"
    );
    win.press(egui::Key::Enter);
    assert!(
        win.app.shelf_card_drawn().is_none(),
        "keeping a column left the card hung"
    );
    assert_eq!(win.list_channel(), None, "keeping a column closes the list");
    assert_eq!(
        win.app.shelf_band().expect("band").channels().x,
        Binding::Column(chosen.clone()),
        "`Enter` did not keep {chosen} on x"
    );
    assert!(
        win.app.chart_doc().has_unsaved_edit(),
        "`Enter` kept {chosen} and left no edit for Save to write"
    );
}

/// **AC1, `Esc`.** With the query empty `Esc` backs out of the card and the
/// band keeps the keys, as it does out of the Outline's list.
#[test]
fn esc_backs_out_of_the_card() {
    let mut win = Window::rail_shut(Mode::Light);
    win.open_cell(egui::Key::Y, "y");
    assert!(win.app.shelf_card_drawn().is_some());
    win.press(egui::Key::Escape);
    assert!(win.app.shelf_card_drawn().is_none(), "`Esc` left the card");
    assert_eq!(win.list_channel(), None);
    assert_eq!(win.active(), None);
    assert!(win.app.shelf_holds_keys(), "the band keeps the keys");
}

/// **AC1, the pointer.** The pointer moves the card's cursor and a click on a
/// row keeps the column, which is what the same two gestures do in the Outline.
#[test]
fn the_pointer_moves_the_cards_cursor_and_a_click_on_a_row_keeps_the_column() {
    let mut win = Window::rail_shut(Mode::Light);
    win.open_cell(egui::Key::X, "x");
    let held = win.cursor().expect("the list opens on a column");
    let row = win
        .card()
        .list
        .rows
        .iter()
        .find(|r| r.column != held)
        .expect("the table has a second column")
        .clone();

    win.point(row.name_rect.center());
    assert_eq!(
        win.cursor().as_deref(),
        Some(row.column.as_str()),
        "the pointer over a row did not move the cursor to it"
    );
    assert!(
        win.app.shelf_card_drawn().is_some(),
        "moving over a row closed the card"
    );
    assert!(
        !win.app.chart_doc().has_unsaved_edit(),
        "moving over a row kept it: a column under the pointer is a preview"
    );

    win.click(row.name_rect.center());
    assert!(
        win.app.shelf_card_drawn().is_none(),
        "a click on a row left the card hung"
    );
    assert_eq!(
        win.app.shelf_band().expect("band").channels().x,
        Binding::Column(row.column.clone()),
        "a click on {} did not keep it on x",
        row.column
    );
    assert!(
        win.app.chart_doc().has_unsaved_edit(),
        "a click on {} left no edit for Save to write: it is still a preview",
        row.column
    );
}

/// **A cell that lists no columns hangs no card.** The mark's cell lists
/// marks, which the list does not draw.
#[test]
fn the_marks_cell_hangs_no_card() {
    let mut win = Window::rail_shut(Mode::Light);
    win.open_cell(egui::Key::M, "m");
    assert_eq!(win.active(), Some(ShelfChannel::Mark));
    assert!(win.app.shelf_card_drawn().is_none());
}

/// **A card that is taller than the room under its cell stops at the window.**
/// A short window leaves the list less room than its rows take, and the rows
/// scroll inside the card.
#[test]
fn a_card_taller_than_the_room_under_its_cell_stops_at_the_window_and_scrolls() {
    let tall = {
        let mut win = Window::rail_shut(Mode::Light);
        win.open_cell(egui::Key::X, "x");
        win.card().rect.height()
    };
    let mut win = Window::open_at(Mode::Light, 420.0);
    win.shut_the_rail();
    win.open_cell(egui::Key::X, "x");
    let card = win.card();
    assert!(
        card.rect.height() < tall,
        "the card is as tall in a window 420 high ({}) as in one 900 high ({tall})",
        card.rect.height()
    );
    assert!(
        card.rect.bottom() <= win.screen.bottom(),
        "the card runs off the foot of the window: {:?}",
        card.rect
    );
}

// ---------------------------------------------------------------------------
// AC2: the floating card's frame.
// ---------------------------------------------------------------------------

/// **AC2.** The card is drawn as the design system's floating card: a rule one
/// pixel wide in the default border ink round the card, and under it the
/// overlay elevation's shadow, offset as the token says and with no blur. Read
/// off the shapes the frame painted, against the tokens, in both modes.
#[test]
fn the_card_is_drawn_with_a_one_pixel_rule_and_a_shadow_with_no_blur() {
    for mode in [Mode::Light, Mode::Dark] {
        let dark = mode.is_dark();
        let mut win = Window::rail_shut(mode);
        win.open_cell(egui::Key::X, "x");
        let card = win.card().rect;
        let painted = rects(&win.shapes);

        let rule = chrome::colour(semantic(dark).borders.default_);
        let ruled: Vec<&RectShape> = painted
            .iter()
            .filter(|r| {
                near(r.rect.left(), card.left())
                    && near(r.rect.top(), card.top())
                    && near(r.rect.right(), card.right())
                    && near(r.rect.bottom(), card.bottom())
                    && r.stroke.width > 0.0
            })
            .collect();
        assert!(
            ruled
                .iter()
                .any(|r| r.stroke.width == 1.0 && r.stroke.color == rule),
            "{mode:?}: no rectangle round the card {card:?} is ruled one pixel wide in the \
             default border ink; the ruled ones are {:?}",
            ruled.iter().map(|r| r.stroke).collect::<Vec<_>>()
        );

        let shadow = Elevation::Overlay
            .shadow(dark)
            .expect("an overlay casts a shadow");
        let offset = egui::vec2(shadow.x, shadow.y);
        let cast = painted.iter().find(|r| {
            r.fill == chrome::colour(shadow.colour)
                && near(r.rect.left(), card.left() + offset.x)
                && near(r.rect.top(), card.top() + offset.y)
                && near(r.rect.width(), card.width())
        });
        let cast = cast.unwrap_or_else(|| {
            panic!(
                "{mode:?}: no shadow of the overlay elevation is painted under the card {card:?}"
            )
        });
        assert_eq!(
            cast.blur_width, 0.0,
            "{mode:?}: the card's shadow is blurred"
        );
    }
}

// ---------------------------------------------------------------------------
// AC3: a click outside backs out.
// ---------------------------------------------------------------------------

/// **AC3.** A click on the plot, off the card, backs out of it as `Esc` does:
/// the same open cell, list, keys, card and preview are left as an `Esc` leaves.
#[test]
fn a_click_outside_the_card_backs_out_as_esc_does() {
    let mut escaped = Window::rail_shut(Mode::Light);
    escaped.open_cell(egui::Key::X, "x");
    escaped.press(egui::Key::Escape);
    let by_esc = left_after_backing_out(&escaped);
    assert_eq!(by_esc, (None, None, true, false, false));

    let mut win = Window::rail_shut(Mode::Light);
    win.open_cell(egui::Key::X, "x");
    assert!(win.app.shelf_card_drawn().is_some());
    let away = win.far_from_the_card();
    win.click(away);
    assert_eq!(
        left_after_backing_out(&win),
        by_esc,
        "a click off the card did not leave what `Esc` leaves"
    );
}

/// **AC3, the preview.** A column the cursor has drawn on the hero as a
/// preview is dropped by the click, as `Esc` drops it.
#[test]
fn a_click_outside_the_card_drops_the_preview_the_cursor_drew() {
    let mut win = Window::rail_shut(Mode::Light);
    win.open_cell(egui::Key::X, "x");
    win.type_letter(egui::Key::J, "j");
    assert!(
        win.app.chart_doc().shelf_preview().is_some(),
        "moving the cursor drew no preview to drop"
    );
    let away = win.far_from_the_card();
    win.click(away);
    assert!(win.app.shelf_card_drawn().is_none());
    assert!(
        win.app.chart_doc().shelf_preview().is_none(),
        "the card went and its preview stayed on the hero"
    );
}

/// **AC3, with a query typed.** The click backs out of the list at once; it
/// does not stop at clearing the query as the first `Esc` does.
#[test]
fn a_click_outside_a_card_with_a_query_typed_leaves_the_list_at_once() {
    let mut win = Window::rail_shut(Mode::Light);
    win.open_cell(egui::Key::X, "x");
    win.type_letter(egui::Key::Slash, "/");
    win.run(vec![egui::Event::Text("lat".to_owned())]);
    win.run(Vec::new());
    assert!(win.app.shelf_card_drawn().is_some());
    let away = win.far_from_the_card();
    win.click(away);
    assert!(win.app.shelf_card_drawn().is_none());
    assert_eq!(win.list_channel(), None);
}

/// **AC3, the other side of the line.** A click on the card, on its heading
/// where there is no row, is not outside it and leaves the card hung.
#[test]
fn a_click_on_the_card_is_not_a_click_outside_it() {
    let mut win = Window::rail_shut(Mode::Light);
    win.open_cell(egui::Key::X, "x");
    let heading = win.card().list.heading;
    win.click(heading.center());
    assert!(
        win.app.shelf_card_drawn().is_some(),
        "a click on the card's heading backed out of it"
    );
    assert_eq!(win.list_channel(), Some(ShelfChannel::X));
}

/// **AC3, the cells.** The cell the card hangs from is outside it: a click
/// there backs out. A click on another cell is the band's, and the card is
/// hung from that cell instead.
#[test]
fn a_click_on_the_open_cell_backs_out_and_a_click_on_another_cell_moves_the_card() {
    let mut win = Window::rail_shut(Mode::Light);
    win.open_cell(egui::Key::X, "x");

    let y = win.band_cell(ShelfChannel::Y).center();
    win.click(y);
    assert_eq!(win.list_channel(), Some(ShelfChannel::Y), "the click on y");
    let card = win.card();
    assert!(
        near(card.rect.left(), win.band_cell(ShelfChannel::Y).left()),
        "the card did not move to the y cell"
    );

    let y = win.band_cell(ShelfChannel::Y).center();
    win.click(y);
    assert!(
        win.app.shelf_card_drawn().is_none(),
        "a click on the cell the card hangs from left it hung"
    );
    assert_eq!(win.list_channel(), None);
}

// ---------------------------------------------------------------------------
// The card, against the pointer, the window's foot, the hover readout and the
// repaint it asks for.
// ---------------------------------------------------------------------------

/// **A press on the card that is let go off it is not a click outside.** The
/// pointer pressed on a row, moved off the card and released over the plot: the
/// press and the release are not one click, so the list stays open and the
/// cursor stays where the press found it. A card that backed out on any
/// release outside it would close here.
#[test]
fn a_press_on_the_card_let_go_off_it_leaves_the_list_open() {
    let mut win = Window::rail_shut(Mode::Light);
    win.open_cell(egui::Key::X, "x");
    let held = win.cursor().expect("the list opens on a column");
    let row = win
        .card()
        .list
        .rows
        .iter()
        .find(|r| r.column != held)
        .expect("the table has a second column")
        .clone();
    let on = row.name_rect.center();

    win.point(on);
    let pressed_on = win.cursor();
    assert_eq!(
        pressed_on.as_deref(),
        Some(row.column.as_str()),
        "the pointer over {} did not take the cursor there, so the press below is not on its row",
        row.column
    );

    win.run(vec![button(on, true)]);
    let away = win.far_from_the_card();
    win.run(vec![egui::Event::PointerMoved(away)]);
    win.run(vec![button(away, false)]);
    win.settle();

    assert!(
        win.app.shelf_card_drawn().is_some(),
        "a press on a row let go off the card closed it"
    );
    assert_eq!(
        win.list_channel(),
        Some(ShelfChannel::X),
        "a press on a row let go off the card left x's list"
    );
    assert_eq!(
        win.cursor(),
        pressed_on,
        "a press on a row let go off the card moved the cursor"
    );
}

/// The card's rule, in points: the frame adds it to the scroll area's height on
/// the card's top and again on its foot.
const RULE: f32 = 1.0;

/// **The card's foot keeps the design system's gap from the window's.** A window
/// too short for the list leaves the card the room under its cell less
/// `SPACE_4`, and the frame's rule on top of that room is two points more, so
/// the foot stands `SPACE_4` less the rule's two sides off the window's. With no
/// gap the card would run to the window's edge.
#[test]
fn a_card_taller_than_the_room_keeps_a_space_4_gap_from_the_windows_foot() {
    let tall = {
        let mut win = Window::rail_shut(Mode::Light);
        win.open_cell(egui::Key::X, "x");
        win.card().rect.height()
    };
    let mut win = Window::open_at(Mode::Light, 420.0);
    win.shut_the_rail();
    win.open_cell(egui::Key::X, "x");
    let card = win.card();
    assert!(
        card.rect.height() < tall,
        "the list fits the room in a window 420 high ({}), so no gap is being asked of the card",
        card.rect.height()
    );
    let gap = win.screen.bottom() - card.rect.bottom();
    let designed = spacing::SPACE_4 - 2.0 * RULE;
    assert!(
        near(gap, designed),
        "the card's foot stands {gap} from the window's, not the {designed} the design \
         system's `SPACE_4` ({}) less the rule's two sides leaves",
        spacing::SPACE_4
    );
}

/// The `(longitude, latitude)` of every row of the sample.
fn sample_marks() -> Vec<(f64, f64)> {
    let text = std::fs::read_to_string(housing()).expect("the sample reads");
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().expect("a header row").split(',').collect();
    let column = |name: &str| {
        header
            .iter()
            .position(|c| *c == name)
            .unwrap_or_else(|| panic!("the sample has no {name} column"))
    };
    let (lon, lat) = (column("longitude"), column("latitude"));
    lines
        .map(|line| {
            let cells: Vec<f64> = line
                .split(',')
                .map(|c| c.parse().expect("a numeric cell"))
                .collect();
            (cells[lon], cells[lat])
        })
        .collect()
}

/// Where the hero's plot draws the mark farthest to its right, in window space:
/// a point the card hung from the band's left does not cover, with a mark under
/// it for the pointer to come to rest on.
fn rightmost_mark(win: &Window) -> egui::Pos2 {
    let plot = win.hero_plot();
    let scales = &win.app.chart_doc().composed.plots[0].scales;
    let x = scales.get(Channel::X).expect("an x scale");
    let y = scales.get(Channel::Y).expect("a y scale");
    #[allow(clippy::cast_possible_truncation)]
    sample_marks()
        .into_iter()
        .map(|(lon, lat)| {
            egui::pos2(
                plot.min.x + x.map_f64(lon) as f32,
                plot.min.y + y.map_f64(lat) as f32,
            )
        })
        .max_by(|a, b| a.x.total_cmp(&b.x))
        .expect("the sample has rows")
}

/// **The card is drawn over the hover readout.** The pointer rests on a mark
/// and the hero's readout is up; `e x` then hangs the card. The card's layer is
/// the top one of the order the readout's is in, read from egui's layer order,
/// so the readout does not draw over a list the analyst is choosing from.
#[test]
fn the_card_draws_over_the_hover_readout() {
    let mut win = Window::rail_shut(Mode::Light);
    let mark = rightmost_mark(&win);
    win.point(mark);
    assert!(
        win.app.chart_doc().hover_readout.is_some(),
        "the pointer at rest on the mark at {mark:?} raised no readout, so there is nothing for \
         the card to be drawn over"
    );

    win.open_cell(egui::Key::X, "x");
    let card = win.card();
    assert!(
        !card.rect.contains(mark),
        "the card {:?} covers the pointer at {mark:?}",
        card.rect
    );
    assert!(
        win.app.chart_doc().hover_readout.is_some(),
        "hanging the card took the readout down, so the two are not on screen together"
    );

    // Each layer is found by the id its `Area` is drawn under, in whatever order
    // it is drawn in: the order is what is read, so it is not typed here.
    let (card_layer, readout_layer) = win.ctx.memory(|m| {
        let on_screen = m.areas().visible_layer_ids();
        let layer = |id: &str| {
            on_screen
                .iter()
                .copied()
                .find(|l| l.id == egui::Id::new(id))
        };
        (layer("shelf-column-card"), layer("chart-hover-readout"))
    });
    let card_layer = card_layer.expect("the card's layer is on screen");
    let readout_layer = readout_layer.expect("the readout's layer is on screen");
    assert!(
        card_layer.order >= readout_layer.order,
        "the card is drawn in the {:?} order, under the hover readout's {:?}",
        card_layer.order,
        readout_layer.order
    );
    win.ctx.memory(|m| {
        assert_eq!(
            m.areas().top_layer_id(card_layer.order),
            Some(card_layer),
            "the top layer of the card's order is not the card's, so the hover readout draws \
             over it"
        );
    });
}

/// The files that asked for a repaint in the pass before the last: egui
/// records each request with the file it was made from.
fn repaint_files(win: &Window) -> Vec<&'static str> {
    win.ctx.repaint_causes().iter().map(|c| c.file).collect()
}

/// **A click off the card asks for a repaint.** Backing out is held for the next
/// frame's feed, and the card is gone only on that frame; with no request the
/// window waits on the next input event with the card still up. egui records
/// each request with the file that made it, and other code asks on the same
/// frame, so the read is the request the column list's own file makes: it is
/// absent from the frame the press landed in and present in the frame the
/// release made the click.
#[test]
fn a_click_off_the_card_asks_the_context_for_a_repaint() {
    let mut win = Window::rail_shut(Mode::Light);
    win.open_cell(egui::Key::X, "x");
    let away = win.far_from_the_card();
    win.point(away);
    win.run(vec![button(away, true)]);
    win.run(Vec::new());
    assert!(
        !repaint_files(&win)
            .iter()
            .any(|f| f.ends_with("protocol.rs")),
        "a press off the card, which is not yet a click, asked for a repaint from the column \
         list's file: {:?}",
        win.ctx.repaint_causes()
    );

    win.run(vec![button(away, false)]);
    win.run(Vec::new());
    assert!(
        repaint_files(&win)
            .iter()
            .any(|f| f.ends_with("protocol.rs")),
        "a click off the card asked for no repaint from the column list's file: {:?}",
        win.ctx.repaint_causes()
    );
}

/// **A cold click on x's cell hangs x's list.** The rail is shut and no cell is
/// open, no key has been pressed: the click alone opens the cell, opens its
/// list and hangs the card from it.
#[test]
fn a_click_on_x_with_the_rail_shut_and_no_cell_open_hangs_x_as_a_card() {
    let mut win = Window::rail_shut(Mode::Light);
    assert_eq!(win.active(), None, "a cell is open before the click");
    assert_eq!(win.list_channel(), None, "a list is open before the click");
    assert!(
        win.app.shelf_card_drawn().is_none(),
        "a card is hung before the click"
    );

    let x = win.band_cell(ShelfChannel::X);
    win.click(x.center());
    assert_eq!(
        win.active(),
        Some(ShelfChannel::X),
        "the click opened no cell"
    );
    assert_eq!(
        win.list_channel(),
        Some(ShelfChannel::X),
        "the click opened no list"
    );
    let card = win.card();
    assert_eq!(card.cell, x, "the card says which cell it hangs from");
    assert!(
        near(card.rect.left(), x.left()) && near(card.rect.top(), x.bottom()),
        "the card's top left is {:?}, not the x cell's bottom left {:?}",
        card.rect.left_top(),
        x.left_bottom()
    );
}

// ---------------------------------------------------------------------------
// The card's pixels.
// ---------------------------------------------------------------------------

/// The table's columns, as the list is offered them.
fn columns() -> Vec<ListColumn> {
    let path = housing();
    let facts: Vec<ColumnFacts> = brightfield_shell::data_file::open(path.to_str().expect("utf-8"))
        .expect("the sample opens")
        .protocol
        .inputs()
        .expect("the opened file's protocol")
        .columns;
    facts
        .iter()
        .map(|c| ListColumn {
            name: c.column.clone(),
            kind: c.leaf.clone(),
            moments: c.moments.clone(),
        })
        .collect()
}

/// What the hero's axes read when the plot's attributes are `attrs`, a block of
/// top-level lines of the spec, as the window reads them off the live plot and
/// hands them to the list.
fn settings_of(attrs: &str, channels: &ShelfChannels) -> ChannelSettings {
    let source = format!(
        "data:\n  t:\n    - {{ a: 1 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: a\n    y: a\nwidth: 600\nheight: 300\n{attrs}\n"
    );
    let spec = parse_spec(&source, Format::Yaml)
        .expect("the spec parses")
        .spec;
    let plot = plot_at_path(&spec, "root").expect("the spec's root is its plot");
    ChannelSettings::of_plot(&spec, plot, channels)
}

/// The card's list over the housing columns, as the window hands it: x's, with
/// `attrs` the plot's attributes the axes' settings read.
fn card_list(attrs: &str) -> ColumnList {
    let channels = ShelfChannels {
        mark: "dot".to_string(),
        x: Binding::Column("population".to_string()),
        y: Binding::Column("latitude".to_string()),
        colour: Binding::Column("median_income".to_string()),
    };
    let settings = settings_of(attrs, &channels);
    let mut list = ColumnList::new(ColumnListRequest {
        tile: "hero".to_string(),
        channel: ShelfChannel::X,
        channels,
        columns: columns(),
    });
    list.set_settings(settings);
    list
}

/// Draw the card over a stand-in for the band through the wgpu renderer and
/// compare it with the committed baseline `name`.
fn baseline(name: &str, mode: Mode) {
    baseline_of(name, mode, card_list(""));
}

/// [`baseline`] over `list`, which a test opens on the tab it names.
fn baseline_of(name: &str, mode: Mode, mut list: ColumnList) {
    let size = egui::vec2(420.0, 520.0);
    let cell = egui::Rect::from_min_size(egui::pos2(40.0, 24.0), egui::vec2(120.0, 44.0));
    let mut harness = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(2.0)
        .wgpu()
        .build_ui(move |ui| {
            design::apply(ui.ctx(), mode);
            let sem = semantic(mode.is_dark());
            ui.painter().rect_filled(
                egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                0.0,
                chrome::colour(sem.surfaces.sunken),
            );
            ui.painter()
                .rect_filled(cell, 0.0, chrome::colour(sem.surfaces.header));
            ui.painter().rect_filled(
                cell,
                0.0,
                chrome::colour(channel::tint(ShelfChannel::X, mode)),
            );
            list.show_card(ui.ctx(), cell, mode);
        });
    harness.run();
    harness.snapshot_options(name, &SnapshotOptions::default());
}

#[test]
fn the_card_light_matches_its_baseline() {
    baseline("shelf_fallback_card_light", Mode::Light);
}

#[test]
fn the_card_dark_matches_its_baseline() {
    baseline("shelf_fallback_card_dark", Mode::Dark);
}

/// The card's list turned to x's settings, with a scale and a format the file
/// sets and a title it leaves to the column's name, so the card draws a row of
/// each state.
fn settings_card() -> ColumnList {
    let mut list = card_list("xScale: log\nxTickFormat: ',d'");
    list.feed_events(&[key_down(egui::Key::Tab)]);
    assert_eq!(list.tab(), ListTab::Settings, "Tab turned the card's list");
    list
}

#[test]
fn the_settings_card_light_matches_its_baseline() {
    baseline_of(
        "shelf_fallback_settings_card_light",
        Mode::Light,
        settings_card(),
    );
}

#[test]
fn the_settings_card_dark_matches_its_baseline() {
    baseline_of(
        "shelf_fallback_settings_card_dark",
        Mode::Dark,
        settings_card(),
    );
}
