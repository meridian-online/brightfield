//! **A column's row in the Outline puts the column on a channel of the hero's
//! shelf: by `z` and the channel's letter, or by a chip on the row.**
//!
//! The band's own route is `e`, a channel's letter, a column in the list the
//! Outline draws for it, and `Enter`. This is the other route, from where the
//! column is listed: with a column's row under the Outline's cursor and the
//! Outline holding the keys, `z c` keeps the column on the hero's colour; under
//! the pointer the row trades its type for three chips, x, y and c, the pointer
//! on a chip draws the hero with the column on that channel as a preview, and a
//! click keeps it.
//!
//! Every assertion reads what a frame drew or what the window holds after it:
//! the hero's composition, the band's cells, the Outline's rows and chips, the
//! spec the page is drawn from and the window title.

use brightfield_keys::{registry, VerbStatus};
use brightfield_protocol::layout::Flow;
use brightfield_render::channel::Channel;
use brightfield_shell::app::CHART;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::PlotHandle;
use brightfield_shell::protocol::{OutlineChipDrawn, SpineRole, SpineRowDrawn, OUTLINE};
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::text_ink;
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_spec::ast::Spec;
use brightfield_workbench::channel::ShelfChannel;
use brightfield_workbench::PaneKey;

const HOUSING_FILE: &str = "california_housing_sample.csv";
const INCOME: &str = "median_income";
const VALUE: &str = "median_house_value";

/// The three verbs, as the registry names them.
const PUT_VERBS: [&str; 3] = ["put-column-on-x", "put-column-on-y", "put-column-on-colour"];

fn housing() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(HOUSING_FILE)
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

/// Frames the pointer is held still on a chip before a tooltip is read: egui
/// refuses a tooltip while the pointer's recent history still holds the jump it
/// arrived by — see `tile_scale_switch.rs`, which this is measured against.
const STILL_FRAMES: usize = 12;

/// One headless window over a data file.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    /// The text the frame before drew.
    texts: Vec<text_ink::DrawnText>,
}

impl Window {
    fn over(boot: Boot) -> Self {
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
            texts: Vec::new(),
        };
        win.settle();
        win
    }

    /// A window over the committed housing sample, the dashboard on the
    /// canvas. Two of them read one file, so their specs name the same path.
    fn fixture() -> Self {
        let boot =
            Boot::data_file(housing().to_str().expect("utf-8 path")).expect("the sample opens");
        let win = Self::over(boot);
        assert!(
            !win.app.graph_on_canvas(),
            "the housing sample opened with the graph on the canvas"
        );
        win
    }

    /// The shipped crosswalk, its graph on the canvas: a Protocol with
    /// families, siblings and a drill for the grammar to act on.
    fn crosswalk() -> Self {
        let boot = Boot::start(starts::CROSSWALK, Flow::Vertical).expect("the crosswalk ships");
        let win = Self::over(boot);
        assert!(
            win.app.graph_on_canvas(),
            "the crosswalk opened on no graph"
        );
        win
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            events,
            ..Default::default()
        };
        let mut texts = Vec::new();
        let _ = self.ctx.run_ui(raw, |ui| {
            self.app.draw(ui);
            texts = text_ink::frame_text(ui.ctx());
        });
        self.texts = texts;
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

    /// The Outline's row for `column`, as the last frame drew it: the plain
    /// row under the table, not a row of a shelf's list.
    fn column_row(&self, column: &str) -> SpineRowDrawn {
        self.app
            .spine_rows()
            .iter()
            .find(|r| r.role == SpineRole::Column && r.depth == 1 && r.label == column)
            .unwrap_or_else(|| panic!("the Outline drew no row for {column}"))
            .clone()
    }

    /// The spine row labelled `label`.
    fn spine_row(&self, label: &str) -> SpineRowDrawn {
        self.app
            .spine_rows()
            .iter()
            .find(|r| r.role == SpineRole::Asset && r.label == label)
            .unwrap_or_else(|| panic!("the spine drew no asset row {label}"))
            .clone()
    }

    /// The chips the Outline drew on `column`'s row on the last frame.
    fn chips(&self, column: &str) -> Vec<OutlineChipDrawn> {
        self.app
            .outline_chips()
            .iter()
            .filter(|c| c.column == column)
            .cloned()
            .collect()
    }

    /// The chip that puts `column` on `channel`, with the pointer brought onto
    /// the row first so the row draws it.
    fn chip(&mut self, column: &str, channel: ShelfChannel) -> OutlineChipDrawn {
        let row = self.column_row(column);
        self.point(row.name_rect.center());
        self.chips(column)
            .into_iter()
            .find(|c| c.channel == channel)
            .unwrap_or_else(|| panic!("{column}'s row under the pointer drew no {channel:?} chip"))
    }

    /// Select `column` in the Outline, as a click on its row selects it.
    fn select_in_outline(&mut self, column: &str) {
        let row = self.column_row(column);
        self.click(row.name_rect.center());
        assert_eq!(
            self.app.protocol_model().outline_column(),
            Some(column),
            "a click on {column}'s row left the Outline's cursor elsewhere"
        );
        assert_eq!(
            self.app.focused_pane(),
            Some(PaneKey::new(OUTLINE)),
            "a click on {column}'s row did not give the Outline the keys"
        );
    }

    /// `z` and then `letter`, as the keyboard brings them.
    fn chord(&mut self, key: egui::Key, letter: &str) {
        self.type_letter(egui::Key::Z, "z");
        self.type_letter(key, letter);
    }

    /// The hero's plot, as the page composed it.
    fn hero(&self) -> &PlotHandle {
        self.app
            .chart_doc()
            .composed
            .plots
            .first()
            .expect("the page composed its hero")
    }

    /// The column the hero's top layer draws on `channel`.
    fn hero_column(&self, channel: Channel) -> Option<String> {
        self.hero()
            .hover
            .as_ref()
            .and_then(|layer| layer.column(channel).map(str::to_owned))
    }

    fn marked_unsaved(&self) -> bool {
        self.app.title().contains(UNSAVED_MARK)
    }

    /// The spec the page is drawn from.
    fn spec(&self) -> Spec {
        self.app
            .chart_doc()
            .live_dashboard()
            .expect("a live dashboard")
            .spec()
            .clone()
    }

    /// The text the last frame drew inside `rect`.
    fn text_in(&self, rect: egui::Rect) -> Vec<String> {
        self.texts
            .iter()
            .filter(|t| rect.contains_rect(t.visible) && t.visible.is_positive())
            .map(|t| t.text.clone())
            .collect()
    }

    /// The text the last frame drew in `channel`'s cell of the band.
    fn cell_text(&self, channel: ShelfChannel) -> Vec<String> {
        let drawn = self.app.shelf_drawn().expect("the band drew");
        self.text_in(drawn.cells[channel.index()])
    }

    /// Whether `channel`'s cell drew `column` as its column: whole, or cut to
    /// the cell's room and ended in an ellipsis, as the band draws a name
    /// longer than its room.
    fn cell_names(&self, channel: ShelfChannel, column: &str) -> bool {
        let cell = self.app.shelf_drawn().expect("the band drew").cells[channel.index()];
        self.texts
            .iter()
            .filter(|t| cell.contains_rect(t.visible) && t.visible.is_positive())
            .any(|t| {
                t.text == column
                    || (t.elided && {
                        let kept = t.text.trim_end_matches('\u{2026}');
                        kept.len() > column.len() / 2 && column.starts_with(kept)
                    })
            })
    }

    /// Move the open list's cursor to `column`, down the rows by `j` and then
    /// up them by `k`, and keep it with `Enter`.
    fn keep_from_list(&mut self, column: &str) {
        let cursor = |w: &Self| {
            w.app
                .protocol_model()
                .column_list()
                .and_then(|l| l.cursor().map(str::to_owned))
        };
        for (key, text) in [(egui::Key::J, "j"), (egui::Key::K, "k")] {
            for _ in 0..12 {
                if cursor(self).as_deref() == Some(column) {
                    self.press(egui::Key::Enter);
                    return;
                }
                self.type_letter(key, text);
            }
        }
        panic!(
            "the list's cursor did not reach {column}: it is on {:?}",
            cursor(self)
        );
    }

    /// The text painted on the frame drawn with the pointer resting on `at`,
    /// tooltips included.
    fn resting_text(&mut self, at: egui::Pos2) -> Vec<String> {
        // Zeroed here and not at construction: the window installs the design
        // system's whole `Style` on its first draw, which replaces a delay set
        // before it.
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            self.ctx
                .style_mut_of(theme, |style| style.interaction.tooltip_delay = 0.0);
        }
        self.run(vec![egui::Event::PointerMoved(at)]);
        for _ in 0..STILL_FRAMES {
            self.run(Vec::new());
        }
        assert!(
            self.ctx.input(|i| i.pointer.is_still()),
            "the pointer is still moving by egui's reading after {STILL_FRAMES} frames"
        );
        self.run(Vec::new());
        self.texts.iter().map(|t| t.text.clone()).collect()
    }

    /// Open the palette with `space`, type `longname` and confirm it.
    fn confirm(&mut self, longname: &str) {
        self.press(egui::Key::Space);
        assert_eq!(
            self.app.open_overlay(),
            Some("palette"),
            "space opened no palette"
        );
        self.settle();
        self.run(vec![egui::Event::Text(longname.to_owned())]);
        self.run(Vec::new());
        self.press(egui::Key::Enter);
        assert_eq!(
            self.app.open_overlay(),
            None,
            "confirming {longname} left the palette open"
        );
    }

    /// The rows the palette lists when `space` opens it, closed again after.
    fn palette_rows(&mut self) -> Vec<String> {
        self.press(egui::Key::Space);
        assert_eq!(
            self.app.open_overlay(),
            Some("palette"),
            "space opened no palette"
        );
        self.settle();
        let rows = self.app.open_palette_rows();
        self.press(egui::Key::Escape);
        assert_eq!(
            self.app.open_overlay(),
            None,
            "escape left the palette open"
        );
        rows
    }

    /// Put the graph on the canvas by the chip in the spine's head.
    fn graph_to_canvas(&mut self) {
        let chip = self
            .app
            .spine_rows()
            .iter()
            .find_map(|r| r.chip)
            .expect("the spine's head drew its graph chip");
        self.click(chip.rect.center());
        assert!(
            self.app.graph_on_canvas(),
            "the graph chip put no graph on the canvas"
        );
    }

    /// Select the `index`th row of the node jump's list, as `/`, arrows and
    /// `Enter` select it.
    fn jump_to(&mut self, index: usize) {
        self.press(egui::Key::Slash);
        assert_eq!(self.app.open_overlay(), Some("jump"));
        for _ in 0..index {
            self.press(egui::Key::ArrowDown);
        }
        self.press(egui::Key::Enter);
        assert_eq!(self.app.open_overlay(), None);
    }
}

/// The open hero, before anything is put on it: no column paints it.
fn assert_unpainted(win: &Window, when: &str) {
    assert_eq!(
        win.hero_column(Channel::Fill),
        None,
        "{when}: a column already paints the hero"
    );
}

/// **AC1.** With `median_house_value` selected in the Outline by a click on its
/// row, the dashboard on the canvas and the Outline holding the keys, `z c`
/// keeps it on the hero's colour and the window is marked unsaved.
#[test]
fn z_c_on_a_selected_columns_row_keeps_it_on_the_heros_colour() {
    let mut win = Window::fixture();
    assert_unpainted(&win, "at open");
    assert!(
        !win.marked_unsaved(),
        "a window just opened is marked unsaved"
    );
    win.select_in_outline(VALUE);
    assert_unpainted(&win, "after the click that selected it");

    win.chord(egui::Key::C, "c");

    assert_eq!(
        win.hero_column(Channel::Fill).as_deref(),
        Some(VALUE),
        "z c did not paint the hero by median_house_value"
    );
    assert_eq!(
        win.app.chart_doc().shelf_preview(),
        None,
        "the column was left standing as a preview, not kept"
    );
    assert!(
        win.cell_names(ShelfChannel::Colour, VALUE),
        "the band's colour cell drew {:?}, not median_house_value",
        win.cell_text(ShelfChannel::Colour)
    );
    assert!(
        win.marked_unsaved(),
        "z c kept a column and left the title unmarked"
    );
}

/// **AC1, the keys taken.** The `x` after a `z` is the chord's: it puts the
/// column on x and does not also cycle the axis lock, which `x` is with the
/// keys elsewhere.
#[test]
fn z_x_puts_the_column_on_x_and_does_not_also_cycle_the_axis_lock() {
    let mut win = Window::fixture();
    let lock = win.app.chart_doc().axis_lock;
    win.select_in_outline(INCOME);

    win.chord(egui::Key::X, "x");

    assert_eq!(
        win.hero_column(Channel::X).as_deref(),
        Some(INCOME),
        "z x did not put median_income on the hero's x"
    );
    assert_eq!(
        win.app.chart_doc().axis_lock,
        lock,
        "the x of z x also cycled the axis lock"
    );
    assert!(win.marked_unsaved());
}

/// **AC2, the chips.** Under the pointer a column's row draws three chips,
/// x, y and c, left to right inside the row, in place of its type; a row the
/// pointer is not on keeps its type.
#[test]
fn a_columns_row_under_the_pointer_trades_its_type_for_three_chips() {
    let mut win = Window::fixture();
    let at_rest = win.column_row(VALUE);
    assert!(
        !at_rest.kind.is_empty() && at_rest.kind_rect.is_some(),
        "median_house_value's row drew no type at rest, so its giving one up proves nothing"
    );
    let type_text = at_rest.kind.clone();
    assert!(
        win.chips(VALUE).is_empty(),
        "the row drew chips with no pointer on it"
    );

    win.point(at_rest.name_rect.center());

    let chips = win.chips(VALUE);
    let channels: Vec<ShelfChannel> = chips.iter().map(|c| c.channel).collect();
    assert_eq!(
        channels,
        vec![ShelfChannel::X, ShelfChannel::Y, ShelfChannel::Colour],
        "the row under the pointer drew chips {channels:?}"
    );
    let row = win.column_row(VALUE);
    for pair in chips.windows(2) {
        assert!(
            pair[0].rect.right() <= pair[1].rect.left(),
            "the chips do not read left to right: {:?}",
            chips.iter().map(|c| c.rect).collect::<Vec<_>>()
        );
    }
    for chip in &chips {
        // Inside the row across it, and centred on its line: the design
        // system's key chip is a point taller than a dense row, so it overhangs
        // the row by half a point above and below rather than hanging from its
        // top.
        assert!(
            row.rect.left() <= chip.rect.left() && chip.rect.right() <= row.rect.right(),
            "the {:?} chip {:?} is outside its row {:?}",
            chip.channel,
            chip.rect,
            row.rect
        );
        assert!(
            (chip.rect.center().y - row.rect.center().y).abs() <= 0.5,
            "the {:?} chip {:?} is not centred on its row {:?}",
            chip.channel,
            chip.rect,
            row.rect
        );
        assert!(
            row.name_rect.right() <= chip.rect.left(),
            "the name {:?} runs under the {:?} chip {:?}",
            row.name_rect,
            chip.channel,
            chip.rect
        );
    }
    assert!(
        row.kind.is_empty() && row.kind_rect.is_none(),
        "the row under the pointer still drew its type {:?}",
        row.kind
    );
    let painted = win.text_in(row.rect);
    assert_eq!(
        painted
            .iter()
            .filter(|t| ["x", "y", "c"].contains(&t.as_str()))
            .count(),
        3,
        "the row drew {painted:?}, not the three chips' letters"
    );
    assert!(
        !painted.contains(&type_text),
        "the row under the pointer painted its type {type_text:?} as well: {painted:?}"
    );
    let other = win.column_row(INCOME);
    assert!(
        !other.kind.is_empty() && win.chips(INCOME).is_empty(),
        "a row the pointer is not on gave up its type"
    );
}

/// **AC2, the preview.** The pointer on the c chip draws the hero with the
/// column on colour and the band's colour cell says so; the window is not
/// marked unsaved. The pointer leaving the chip puts the kept chart back and
/// adds nothing to what Save writes.
#[test]
fn the_pointer_on_a_chip_previews_the_column_and_leaving_it_puts_the_kept_chart_back() {
    let mut win = Window::fixture();
    let kept = win.spec();
    let chip = win.chip(VALUE, ShelfChannel::Colour);

    win.point(chip.rect.center());

    assert_eq!(
        win.hero_column(Channel::Fill).as_deref(),
        Some(VALUE),
        "the pointer on the c chip did not draw the hero by median_house_value"
    );
    assert_eq!(
        win.app.chart_doc().shelf_preview(),
        Some((ShelfChannel::Colour, VALUE)),
        "the hero is drawn with the column but not as a preview"
    );
    let cell = win.cell_text(ShelfChannel::Colour);
    assert!(
        win.cell_names(ShelfChannel::Colour, VALUE) && cell.iter().any(|t| t == "preview"),
        "the colour cell drew {cell:?}, not the column as a preview"
    );
    assert!(!win.marked_unsaved(), "a preview marked the window unsaved");

    // Off the chip and onto the row's name: the row still draws its chips.
    let row = win.column_row(VALUE);
    win.point(row.name_rect.center());
    assert!(
        !win.chips(VALUE).is_empty(),
        "the row stopped drawing its chips"
    );

    assert_unpainted(&win, "after the pointer left the chip");
    assert_eq!(
        win.app.chart_doc().shelf_preview(),
        None,
        "the preview outlived the pointer"
    );
    assert!(
        !win.cell_text(ShelfChannel::Colour)
            .iter()
            .any(|t| t == "preview"),
        "the colour cell still says preview"
    );
    assert!(
        !win.marked_unsaved(),
        "backing out of a preview marked the window unsaved"
    );
    assert!(
        !win.app.chart_doc().has_unsaved_edit(),
        "the preview left an edit for Save to write"
    );
    assert_eq!(
        win.spec(),
        kept,
        "the kept chart is not the one drawn before the preview"
    );
}

/// **AC2, the click.** A click on a chip keeps the column on its channel, as
/// `z` and the channel does: the c chip on `median_house_value` leaves the
/// spec `z c` leaves, kept, and marks the window unsaved.
#[test]
fn a_click_on_the_c_chip_does_what_z_c_does() {
    let mut by_chip = Window::fixture();
    let chip = by_chip.chip(VALUE, ShelfChannel::Colour);
    by_chip.click(chip.rect.center());
    // The pointer goes away, so nothing the chip is still previewing can pass
    // for what the click kept.
    by_chip.point(egui::pos2(1.0, 1.0));

    let mut by_keys = Window::fixture();
    by_keys.select_in_outline(VALUE);
    by_keys.chord(egui::Key::C, "c");

    assert_eq!(
        by_chip.hero_column(Channel::Fill).as_deref(),
        Some(VALUE),
        "a click on the c chip did not keep median_house_value on colour"
    );
    assert_eq!(by_chip.app.chart_doc().shelf_preview(), None);
    assert!(
        by_chip.marked_unsaved(),
        "a click that kept a column left the title unmarked"
    );
    assert!(
        by_chip.spec() == by_keys.spec(),
        "the c chip left a different spec from z c"
    );
}

/// **AC2, the tooltip.** The pointer resting on a chip draws the design
/// system's action tooltip, naming the verb and its keys.
#[test]
fn a_chips_tooltip_names_the_verb_and_its_keys() {
    let mut win = Window::fixture();
    for (channel, verb, keys) in [
        (ShelfChannel::X, "put-column-on-x", "z x"),
        (ShelfChannel::Colour, "put-column-on-colour", "z c"),
    ] {
        let chip = win.chip(VALUE, channel);
        let painted = win.resting_text(chip.rect.center());
        assert!(
            painted.iter().any(|t| t == verb) && painted.iter().any(|t| t == keys),
            "resting on the {channel:?} chip painted {painted:?}, not {verb} and {keys}"
        );
    }
}

/// **AC3.** `z a` on a spine row still opens and closes its fold, through the
/// keys on the window: the crosswalk's family, picked by the jump.
#[test]
fn z_a_on_a_spine_row_still_folds_and_unfolds() {
    let mut win = Window::crosswalk();
    let family = win
        .app
        .protocol_model()
        .outline()
        .iter()
        .position(|r| r.id.starts_with("family."))
        .expect("the crosswalk lists a family");
    win.jump_to(family);
    assert!(!win.app.protocol_model().is_expanded());

    win.chord(egui::Key::A, "a");
    assert!(
        win.app.protocol_model().is_expanded(),
        "z a did not open the family"
    );
    win.chord(egui::Key::A, "a");
    assert!(
        !win.app.protocol_model().is_expanded(),
        "z a again did not close it"
    );
}

/// **AC3.** `z c` on a spine row changes nothing, with a column selected
/// before the spine row was: the column stays highlighted, the Outline's
/// cursor is on the spine row, and neither the chart nor the title moves.
#[test]
fn z_c_on_a_spine_row_changes_nothing() {
    let mut win = Window::fixture();
    win.select_in_outline(VALUE);
    let table = win
        .app
        .protocol_model()
        .table()
        .cloned()
        .expect("the sample's Protocol holds a table");
    let label = win
        .app
        .protocol_model()
        .outline()
        .into_iter()
        .find(|r| r.id == table)
        .expect("the outline lists the table")
        .label;
    let row = win.spine_row(&label);
    win.click(row.name_rect.center());
    assert_eq!(
        win.app.protocol_model().selected(),
        Some(&table),
        "the click did not select the table's spine row"
    );
    assert_eq!(
        win.app.protocol_model().selected_column(),
        Some(VALUE),
        "the column is no longer selected, so this proves nothing about a spine row"
    );
    assert_eq!(win.app.focused_pane(), Some(PaneKey::new(OUTLINE)));
    let kept = win.spec();

    win.chord(egui::Key::C, "c");

    assert_unpainted(&win, "after z c on a spine row");
    assert_eq!(win.spec(), kept, "z c on a spine row changed the spec");
    assert!(
        !win.marked_unsaved(),
        "z c on a spine row marked the window unsaved"
    );
}

/// **AC4.** A column put on a channel from its row leaves the same spec the
/// band's route leaves for the same column: colour by `z c` against `e c` and
/// `Enter` on `median_house_value`, and x by `z x` against `e x` and `Enter` on
/// `median_income`.
#[test]
fn a_column_put_from_its_row_leaves_the_spec_the_bands_route_leaves() {
    for (column, key, letter) in [(VALUE, egui::Key::C, "c"), (INCOME, egui::Key::X, "x")] {
        let mut by_band = Window::fixture();
        assert!(by_band.app.focus_pane(PaneKey::new(CHART)));
        by_band.settle();
        by_band.type_letter(egui::Key::E, "e");
        by_band.type_letter(key, letter);
        by_band.keep_from_list(column);
        assert!(
            by_band.marked_unsaved(),
            "the band's route kept nothing for {column}"
        );

        let mut by_row = Window::fixture();
        by_row.select_in_outline(column);
        by_row.chord(key, letter);
        assert!(
            by_row.marked_unsaved(),
            "z {letter} kept nothing for {column}"
        );

        let (row, band) = (by_row.spec(), by_band.spec());
        if row != band {
            let (r, b) = (format!("{row:#?}"), format!("{band:#?}"));
            let differ: Vec<(&str, &str)> = r
                .lines()
                .zip(b.lines())
                .filter(|(a, b)| a != b)
                .take(12)
                .collect();
            panic!("z {letter} on {column}'s row left a different spec from the band: {differ:#?}");
        }
    }
}

/// **AC5.** The Protocol palette opens over the graph, where a column put on a
/// channel is dropped, so with a column under the Outline's cursor it offers
/// none of the three put rows; and `z c` there changes nothing either.
#[test]
fn the_protocol_palette_offers_no_put_row_where_choosing_one_would_put_nothing() {
    let mut win = Window::fixture();
    win.select_in_outline(VALUE);
    let kept = win.spec();
    win.graph_to_canvas();
    let row = win.column_row(VALUE);
    win.click(row.name_rect.center());
    assert_eq!(win.app.protocol_model().outline_column(), Some(VALUE));

    let rows = win.palette_rows();
    assert!(
        rows.iter().any(|r| r == "toggle-fold"),
        "the palette lists no Protocol verb, so this is not the Protocol palette: {rows:?}"
    );
    for verb in PUT_VERBS {
        assert!(
            !rows.iter().any(|r| r == verb),
            "the Protocol palette offers {verb}, which puts nothing over the graph: {rows:?}"
        );
    }

    win.chord(egui::Key::C, "c");
    assert_eq!(
        win.spec(),
        kept,
        "z c over the graph changed the hero's spec"
    );
    assert!(
        !win.marked_unsaved(),
        "z c over the graph marked the window unsaved"
    );
}

/// **AC5.** Every row the Protocol palette offers as enabled does something
/// when chosen.
///
/// The rows are the palette's own, read off the window the key opened it over:
/// the housing sample with a column under the Outline's cursor and the graph on
/// the canvas, the state that would draw the put rows if any did. Each row is
/// then chosen through the palette on a window in a state where it has work,
/// and the one piece of state it is documented to change is read. **A row with
/// no arm here panics**: a verb that joins the Protocol palette later is not
/// passed over in silence, which is how the three put rows sat enabled and inert.
#[test]
fn every_row_the_protocol_palette_offers_as_enabled_does_something_when_chosen() {
    let mut win = Window::fixture();
    win.select_in_outline(VALUE);
    win.graph_to_canvas();
    let row = win.column_row(VALUE);
    win.click(row.name_rect.center());
    let reg = registry::registry();
    let enabled: Vec<String> = win
        .palette_rows()
        .into_iter()
        .filter(|r| {
            reg.iter()
                .find(|v| v.longname == r)
                .is_some_and(|v| v.status != VerbStatus::Reserved)
        })
        .collect();
    assert!(
        enabled.len() > 3,
        "the Protocol palette offers too little to sweep: {enabled:?}"
    );

    for verb in &enabled {
        let verb = verb.as_str();
        match verb {
            "protocol-producer"
            | "protocol-consumer"
            | "protocol-sibling-next"
            | "protocol-sibling-prev" => {
                // The first node of the crosswalk's outline the verb moves from.
                let mut moved = false;
                let nodes = Window::crosswalk().app.protocol_model().outline().len();
                for index in 0..nodes {
                    let mut at = Window::crosswalk();
                    at.jump_to(index);
                    let before = at.app.protocol_model().selected().cloned();
                    at.confirm(verb);
                    if at.app.protocol_model().selected().cloned() != before {
                        moved = true;
                        break;
                    }
                }
                assert!(
                    moved,
                    "{verb} moved the selection from no node of the crosswalk"
                );
            }
            "toggle-fold" => {
                let mut at = Window::crosswalk();
                let family = at
                    .app
                    .protocol_model()
                    .outline()
                    .iter()
                    .position(|r| r.id.starts_with("family."))
                    .expect("the crosswalk lists a family");
                at.jump_to(family);
                at.confirm(verb);
                assert!(
                    at.app.protocol_model().is_expanded(),
                    "{verb} opened no fold"
                );
            }
            "protocol-drill-in" => {
                let mut at = Window::crosswalk();
                at.jump_to(1);
                at.confirm(verb);
                assert!(
                    at.app.protocol_model().is_drilled(),
                    "{verb} drilled into nothing"
                );
            }
            "protocol-drill-out" => {
                let mut at = Window::crosswalk();
                at.jump_to(1);
                at.press(egui::Key::Enter);
                assert!(
                    at.app.protocol_model().is_drilled(),
                    "Enter drilled into nothing"
                );
                at.confirm(verb);
                assert!(
                    !at.app.protocol_model().is_drilled(),
                    "{verb} left the drill"
                );
            }
            "open-steps-sheet" => {
                let mut at = Window::crosswalk();
                at.confirm(verb);
                assert!(
                    at.app.protocol_model().show_sheet(),
                    "{verb} opened no sheet"
                );
            }
            "yank-address" => {
                let mut at = Window::crosswalk();
                at.jump_to(1);
                at.confirm(verb);
                assert!(
                    at.app.protocol_model().yank_flash().is_some(),
                    "{verb} yanked nothing"
                );
            }
            "toggle-outline-rail" => {
                let mut at = Window::crosswalk();
                assert_ne!(at.app.focused_pane(), Some(PaneKey::new(OUTLINE)));
                at.confirm(verb);
                assert_eq!(
                    at.app.focused_pane(),
                    Some(PaneKey::new(OUTLINE)),
                    "{verb} did not take focus to the Outline"
                );
            }
            "open-home" => {
                let mut at = Window::crosswalk();
                at.confirm(verb);
                assert!(
                    at.app.front_door_is_live(),
                    "{verb} did not go to the front door"
                );
            }
            other => panic!(
                "{other} is offered enabled on the Protocol palette with no proof here that \
                 choosing it does something"
            ),
        }
    }
}
