//! **In the window, `Tab` on an axis's shelf entry turns its list to the
//! settings, from the Outline or, with the rail shut, from the card hung from the
//! cell, and no widget takes the keyboard with it.**
//!
//! `shelf_settings.rs` drives the list alone. What the list cannot say by itself is
//! this file's: that the key reaches the list, that the chart is drawn as
//! it was kept once the list is on its settings, that the settings read the chart
//! the list was opened on, and that `Tab`, which `egui` reads at the head of a
//! pass as a request to move focus to the next widget that takes it, does not
//! also give a button the keyboard, so that the next `j` goes to the list.
//!
//! Every assertion reads a frame the window drew or what the window holds after
//! it: the open list, the band's active cell, the context's keyboard focus, the
//! card's drawn record and the chart's preview.

use brightfield_shell::app::CHART;
use brightfield_shell::design::Mode;
use brightfield_shell::shelf::{
    CardDrawn, ListTab, FORMAT_ROW, GRID_ROW, REVERSE_ROW, SCALE_ROW, TICKS_ROW, TITLE_ROW,
    ZERO_ROW,
};
use brightfield_shell::startup::default_layout;
use brightfield_shell::text_ink;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::arrangement;
use brightfield_workbench::channel::ShelfChannel;
use brightfield_workbench::PaneKey;

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

/// One headless window over the housing sample, the dashboard on the canvas.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    fn open() -> Self {
        let boot =
            Boot::data_file(housing().to_str().expect("utf-8 path")).expect("the sample opens");
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
        };
        win.settle();
        assert!(
            win.app.focus_pane(PaneKey::new(CHART)),
            "the dashboard's pane takes focus"
        );
        win.settle();
        win
    }

    /// A window whose navigator rail is shut, as a click on its collapse control
    /// shuts it.
    fn rail_shut() -> Self {
        let mut win = Self::open();
        let at = win
            .app
            .rail_collapse_rect(arrangement::NAVIGATOR_RAIL)
            .expect("the navigator rail drew a collapse control")
            .center();
        win.click(at);
        assert!(
            win.app.rail_is_collapsed(arrangement::NAVIGATOR_RAIL),
            "a click on the collapse control did not shut the navigator rail"
        );
        win
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            events,
            ..Default::default()
        };
        let _ = self.ctx.run_ui(raw, |ui| {
            self.app.draw(ui);
            let _ = text_ink::frame_text(ui.ctx());
        });
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

    /// Press a key that makes no text, and let the frame after run.
    fn press(&mut self, key: egui::Key) {
        self.run(vec![key_down(key)]);
        self.run(Vec::new());
    }

    fn click(&mut self, at: egui::Pos2) {
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.settle();
        self.run(vec![button(at, true)]);
        self.run(vec![button(at, false)]);
        self.settle();
    }

    /// `e` and then the channel's letter: the keys that open a cell.
    fn open_cell(&mut self, key: egui::Key, letter: &str) {
        self.type_letter(egui::Key::E, "e");
        self.type_letter(key, letter);
    }

    fn list_tab(&self) -> Option<ListTab> {
        self.app.protocol_model().column_list().map(|l| l.tab())
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

    fn setting_cursor(&self) -> Option<&'static str> {
        self.app
            .protocol_model()
            .column_list()
            .and_then(|l| l.setting_cursor().map(|r| r.name))
    }

    fn active(&self) -> Option<ShelfChannel> {
        self.app.shelf_band().and_then(|b| b.active())
    }

    /// The card the last frame drew.
    fn card(&self) -> &CardDrawn {
        self.app
            .shelf_card_drawn()
            .expect("the last frame hung a card")
    }

    /// Whether any widget holds the keyboard: what the window asks before it
    /// hands a key to the shelf, and so whether the next `j` reaches the list.
    fn keyboard_taken(&self) -> bool {
        self.ctx.egui_wants_keyboard_input() || self.ctx.memory(|m| m.focused()).is_some()
    }

    /// The value the open list's settings row `name` reads.
    fn row_value(&self, name: &str) -> String {
        self.app
            .protocol_model()
            .column_list()
            .expect("a list is open")
            .settings()
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("the list has no {name} row"))
            .value
            .clone()
    }
}

/// The state a window is left in after a list is backed out of, in the terms the
/// window holds it: the open cell, the open list, whether the band keeps the
/// keys, whether a card is hung and whether the hero is drawn with a column it
/// has not kept.
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
// AC1: in the Outline.
// ---------------------------------------------------------------------------

/// **`Tab`, `Tab`, `j` ends on the columns with the cursor moved, so no button
/// has taken the keyboard.** `egui` reads a bare `Tab` at the head of a pass as a
/// request to move focus, and the first widget that takes it would hold the
/// keyboard from then on, so that the `j` went to no list. The window withdraws
/// the request when the shelf owns the key; this test is what reddens when it
/// does not.
#[test]
fn tab_tab_j_ends_on_the_columns_with_the_cursor_moved_and_no_widget_holding_the_keyboard() {
    let mut win = Window::open();
    win.open_cell(egui::Key::X, "x");
    assert_eq!(win.list_channel(), Some(ShelfChannel::X));
    assert_eq!(win.list_tab(), Some(ListTab::Columns));
    assert!(
        win.app.shelf_card_drawn().is_none(),
        "the rail is open, so the Outline draws the list and no card is hung"
    );
    let held = win.cursor().expect("the cursor opens on x's column");
    assert!(
        !win.keyboard_taken(),
        "no widget holds the keyboard to begin with"
    );

    win.press(egui::Key::Tab);
    assert_eq!(
        win.list_tab(),
        Some(ListTab::Settings),
        "Tab turned the list"
    );
    assert_eq!(win.list_channel(), Some(ShelfChannel::X));
    assert_eq!(
        win.active(),
        Some(ShelfChannel::X),
        "the band's cell stays open"
    );
    assert!(
        !win.keyboard_taken(),
        "after the first Tab a widget holds the keyboard"
    );

    win.press(egui::Key::Tab);
    assert_eq!(
        win.list_tab(),
        Some(ListTab::Columns),
        "Tab turned the list back"
    );
    assert_eq!(win.cursor().as_deref(), Some(held.as_str()));
    assert!(
        !win.keyboard_taken(),
        "after the second Tab a widget holds the keyboard"
    );

    win.type_letter(egui::Key::J, "j");
    let moved = win.cursor().expect("the list has a cursor");
    assert_ne!(
        moved, held,
        "j moved the cursor off {held}: it reached the list"
    );
    assert_eq!(win.list_tab(), Some(ListTab::Columns));
    assert!(!win.keyboard_taken(), "after j a widget holds the keyboard");
}

/// **`Tab` turns x's list to settings that read the chart as it is kept, and
/// `Esc` returns to the band's cell as it does from the columns.**
#[test]
fn the_settings_read_the_chart_as_kept_and_esc_leaves_as_it_does_from_the_columns() {
    // Keep the next column on x, then open the list again: the title row reads
    // the column the chart now draws.
    let mut win = Window::open();
    win.open_cell(egui::Key::X, "x");
    let before = win.cursor().expect("a cursor");
    win.type_letter(egui::Key::J, "j");
    let next = win.cursor().expect("a cursor");
    assert_ne!(next, before, "j moved the cursor");
    win.press(egui::Key::Enter);
    assert_eq!(
        win.list_channel(),
        None,
        "Enter kept the column and closed the list"
    );
    win.open_cell(egui::Key::X, "x");
    win.press(egui::Key::Tab);
    assert_eq!(win.list_tab(), Some(ListTab::Settings));
    assert_eq!(
        win.row_value(TITLE_ROW),
        next,
        "the title row names the column the chart was kept with"
    );
    assert_ne!(win.row_value(TITLE_ROW), before);

    // Esc from the settings and Esc from the columns leave the same state.
    let mut from_settings = Window::open();
    from_settings.open_cell(egui::Key::X, "x");
    from_settings.press(egui::Key::Tab);
    assert_eq!(from_settings.list_tab(), Some(ListTab::Settings));
    from_settings.press(egui::Key::Escape);
    let mut from_columns = Window::open();
    from_columns.open_cell(egui::Key::X, "x");
    from_columns.press(egui::Key::Escape);
    assert_eq!(
        left_after_backing_out(&from_settings),
        left_after_backing_out(&from_columns),
        "Esc from x's settings leaves what Esc from x's columns leaves"
    );
    assert_eq!(from_settings.list_channel(), None, "the list is closed");
    assert!(!from_settings.keyboard_taken());
}

/// **From x's settings `y` opens y's settings, and `Tab` on colour's cell leaves
/// the list on its columns.**
#[test]
fn from_the_x_settings_y_opens_the_y_settings_and_tab_on_colour_stays_on_the_columns() {
    let mut win = Window::open();
    win.open_cell(egui::Key::X, "x");
    win.press(egui::Key::Tab);
    assert_eq!(win.setting_cursor(), Some(TITLE_ROW));
    let x_title = win.row_value(TITLE_ROW);

    win.type_letter(egui::Key::Y, "y");
    assert_eq!(
        win.active(),
        Some(ShelfChannel::Y),
        "the band's cell is y's"
    );
    assert_eq!(win.list_channel(), Some(ShelfChannel::Y));
    assert_eq!(win.list_tab(), Some(ListTab::Settings), "the tab is kept");
    assert_ne!(
        win.row_value(TITLE_ROW),
        x_title,
        "the rows are y's, which names another column"
    );
    assert!(!win.keyboard_taken());

    let mut win = Window::open();
    win.open_cell(egui::Key::C, "c");
    assert_eq!(win.list_channel(), Some(ShelfChannel::Colour));
    let held = win.cursor();
    win.press(egui::Key::Tab);
    assert_eq!(
        win.list_tab(),
        Some(ListTab::Columns),
        "colour's list has no settings to turn to"
    );
    assert_eq!(win.cursor(), held, "the cursor is where it was");
    assert!(
        !win.keyboard_taken(),
        "Tab on colour's cell gave a widget the keyboard"
    );
    win.type_letter(egui::Key::J, "j");
    assert_ne!(win.cursor(), held, "j still reaches colour's list");
}

/// **A column the cursor is previewing belongs to the columns: turned to the
/// settings, the chart is drawn as it was kept.**
#[test]
fn turning_to_the_settings_backs_a_previewed_column_out() {
    let mut win = Window::open();
    win.open_cell(egui::Key::X, "x");
    assert!(win.app.chart_doc().shelf_preview().is_none());
    win.type_letter(egui::Key::J, "j");
    assert!(
        win.app.chart_doc().shelf_preview().is_some(),
        "j previews the column under the cursor"
    );
    win.press(egui::Key::Tab);
    assert_eq!(win.list_tab(), Some(ListTab::Settings));
    assert!(
        win.app.chart_doc().shelf_preview().is_none(),
        "the chart is drawn as it was kept while the list is on its settings"
    );
}

// ---------------------------------------------------------------------------
// AC4: the card, with the rail shut.
// ---------------------------------------------------------------------------

/// **With the rail shut, `Tab` turns the card hung from the cell to the same
/// settings list, and its keys act on it as they do in the Outline.**
#[test]
fn with_the_rail_shut_tab_turns_the_hung_card_to_the_settings_and_its_keys_act_on_it() {
    let mut win = Window::rail_shut();
    win.open_cell(egui::Key::X, "x");
    let columns = win.card().list.clone();
    assert_eq!(columns.tab, ListTab::Columns);
    assert!(
        columns.tabs.is_some(),
        "the card draws the strip over its columns"
    );
    assert!(!columns.rows.is_empty());

    win.press(egui::Key::Tab);
    assert_eq!(win.list_tab(), Some(ListTab::Settings));
    assert!(
        !win.keyboard_taken(),
        "Tab on the card gave a widget the keyboard"
    );
    let list = win.card().list.clone();
    assert_eq!(list.tab, ListTab::Settings, "the card drew the settings");
    assert_eq!(
        list.settings.iter().map(|r| r.name).collect::<Vec<_>>(),
        [TITLE_ROW, SCALE_ROW, FORMAT_ROW],
        "the card's rows are the Outline's"
    );
    assert!(list.rule.is_some() && list.sentence.is_some());
    assert!(
        list.rows.is_empty(),
        "no column row is drawn on the settings"
    );

    // The keys act on the card as they do in the Outline: j moves the settings
    // cursor, and Tab and Esc turn and leave it.
    assert_eq!(win.setting_cursor(), Some(TITLE_ROW));
    win.type_letter(egui::Key::J, "j");
    assert_eq!(win.setting_cursor(), Some(SCALE_ROW));
    win.press(egui::Key::Tab);
    assert_eq!(win.list_tab(), Some(ListTab::Columns));
    assert_eq!(
        win.card().list.tab,
        ListTab::Columns,
        "the card is back on its columns"
    );
    win.type_letter(egui::Key::J, "j");
    assert!(!win.keyboard_taken());

    win.press(egui::Key::Tab);
    assert_eq!(win.list_tab(), Some(ListTab::Settings));
    win.press(egui::Key::Escape);
    assert_eq!(win.list_channel(), None, "Esc closed the list");
    assert!(win.app.shelf_card_drawn().is_none(), "and the card with it");
}

// ---------------------------------------------------------------------------
// The rows found by name are judged against the scales the hero was drawn with.
// ---------------------------------------------------------------------------

/// **The window hands the settings the scales the hero was drawn against, so a
/// row the render crate's judge says does not apply to the axis carries its
/// reason.** The settings are built from the plot's attributes and the scales
/// beside them; a window that built them from the attributes alone would offer
/// ticks, grid, zero and reverse on a map as it offers them on a scatter. The
/// oracle is the judges themselves, asked of the scales the document holds.
#[test]
fn the_window_judges_the_by_name_rows_against_the_scales_the_hero_was_drawn_with() {
    let mut win = Window::open();
    let scales = win.app.chart_doc().composed.plots[0].scales.clone();
    assert!(
        !brightfield_render::scene::axis_keys_apply(&scales),
        "the generated hero is a map, whose x and y are its projection's"
    );
    for channel in [egui::Key::X, egui::Key::Y] {
        win.open_cell(channel, if channel == egui::Key::X { "x" } else { "y" });
        win.press(egui::Key::Tab);
        assert_eq!(win.list_tab(), Some(ListTab::Settings));
        let rows = win
            .app
            .protocol_model()
            .column_list()
            .expect("a list is open")
            .settings()
            .to_vec();
        for name in [TICKS_ROW, GRID_ROW, ZERO_ROW, REVERSE_ROW] {
            let row = rows.iter().find(|r| r.name == name).expect("the row");
            assert!(
                row.reason.is_some(),
                "{name} on {channel:?} of a map carries the reason the judge gives"
            );
        }
        for name in [TITLE_ROW, SCALE_ROW, FORMAT_ROW] {
            let row = rows.iter().find(|r| r.name == name).expect("the row");
            assert!(row.reason.is_none(), "{name} applies to every axis");
        }
        win.press(egui::Key::Escape);
        win.press(egui::Key::Escape);
    }
}
