//! **`Tab` turns an axis's list from its columns to its settings, and each row
//! reads *auto* or the value the file sets.**
//!
//! The list is drawn into a `Ui` a test gives it, so the claims are read off the
//! frame it painted and off the state it answers a key with, as
//! `outline_list.rs` reads the columns. Three kinds of reading, each at the
//! altitude its claim lives at:
//!
//! - **Keys** go in as `egui` events through `ColumnList::feed_events`, as the
//!   window hands them. What is read back is the `ListReport`s, which are what
//!   the window acts on, and the tab and the channel the list is left on.
//! - **Geometry, words and ink** are read off the laid-out frame: the rectangles
//!   the list painted, the galleys put in them, the colour each galley was laid
//!   out in and the shape drawn at each row's marker. They are not read back from
//!   the numbers the list reports about itself.
//! - **Values** are read twice, off the rows the plot resolves to
//!   (`ChannelSettings::of_plot`) and off the frame a list handed those rows
//!   draws, because a row that read right and drew wrong, or the other way, is
//!   a different defect from either.
//!
//! The window's own claims, that `Tab` reaches the list and takes no
//! widget's focus with it, are `shelf_settings_window.rs`. The settings list's
//! pixels are baselines in `outline_list.rs` and `shelf_fallback.rs`, beside the
//! columns state they sit behind.

use brightfield_shell::design::{self, Mode};
use brightfield_shell::shelf::{
    Binding, ChannelSettings, ColumnList, ColumnListRequest, ListColumn, ListDrawn, ListReport,
    ListTab, SettingRow, SettingRowDrawn, ShelfChannels, AUTO, FORMAT_ROW, NO_TITLE, RANGE_ROW,
    SCALE_ROW, TICKS_ROW, TITLE_ROW,
};
use brightfield_shell::text_ink::{self, DrawnText};
use brightfield_spec::edit::plot_at_path;
use brightfield_spec::layout::resolve_tick_formats;
use brightfield_spec::parse::{parse_spec, Format};
use brightfield_workbench::channel::ShelfChannel;
use brightfield_workbench::chrome;
use egui::epaint::{ClippedShape, Shape};
use meridian_design::{semantic, viz};

/// The Outline rail's default width.
const WIDTH: f32 = 240.0;

/// Where the list's top left sits in the test's window, off the origin so no
/// reading assumes the list starts at zero.
const ORIGIN: egui::Pos2 = egui::pos2(12.0, 9.0);

/// The slot each channel takes in the design system's categorical order, which
/// is blue, gold, teal, red, violet, orange, plum, green, counted from zero.
const TEAL: usize = 2;
const VIOLET: usize = 4;

/// What the heading reads over x's list in the tile the fixture names.
const X_HEADING: &str = "OUTLINE   \u{b7}   x axis of hero";

// ---------------------------------------------------------------------------
// The fixture: a table, the channels the tile takes, and what the axes read.
// ---------------------------------------------------------------------------

/// A table with no data behind it: the list is offered names and types.
fn columns() -> Vec<ListColumn> {
    [
        ("longitude", "DOUBLE"),
        ("latitude", "DOUBLE"),
        ("population", "BIGINT"),
        ("median_income", "DOUBLE"),
        ("ocean_proximity", "VARCHAR"),
    ]
    .into_iter()
    .map(|(name, kind)| ListColumn {
        name: name.to_string(),
        kind: kind.to_string(),
        moments: None,
    })
    .collect()
}

/// What the hero takes: population on x, latitude on y, median income on
/// colour. None is the table's first column.
fn channels() -> ShelfChannels {
    ShelfChannels {
        mark: "dot".to_string(),
        x: Binding::Column("population".to_string()),
        y: Binding::Column("latitude".to_string()),
        colour: Binding::Column("median_income".to_string()),
    }
}

/// The plot attributes' reading as the window hands it to the list: a one-plot
/// spec whose top-level lines after `height` are `attrs`.
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

/// A list on `channel`, handed what the plot with `attrs` reads.
fn list_with(channel: ShelfChannel, attrs: &str) -> ColumnList {
    let mut list = ColumnList::new(ColumnListRequest {
        tile: "hero".to_string(),
        channel,
        channels: channels(),
        columns: columns(),
    });
    list.set_settings(settings_of(attrs, &channels()));
    list
}

/// A list on `channel` over a plot that sets nothing.
fn list(channel: ShelfChannel) -> ColumnList {
    list_with(channel, "")
}

/// The row of `channel`'s settings named `name`, as the plot with `attrs`
/// resolves it.
fn row_of(attrs: &str, channel: ShelfChannel, name: &str) -> SettingRow {
    settings_of(attrs, &channels())
        .rows(channel)
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("{channel:?} has no {name} row"))
        .clone()
}

// ---------------------------------------------------------------------------
// The stage: a list drawn into a `Ui` a test gives it, and what the frame held.
// ---------------------------------------------------------------------------

struct Stage {
    ctx: egui::Context,
    mode: Mode,
}

/// One frame of the list: what it drew, what it painted and what text it put on
/// the screen.
struct Frame {
    drawn: ListDrawn,
    shapes: Vec<ClippedShape>,
    texts: Vec<DrawnText>,
    collisions: Option<String>,
}

impl Stage {
    /// A stage with the theme applied and the faces loaded: the fonts `apply`
    /// installs take effect on the pass after it, so two frames run before
    /// anything is measured.
    fn new(mode: Mode) -> Self {
        let ctx = egui::Context::default();
        design::apply(&ctx, mode);
        let stage = Self { ctx, mode };
        let mut warm = list(ShelfChannel::X);
        stage.draw(&mut warm);
        stage.draw(&mut warm);
        stage
    }

    fn draw(&self, list: &mut ColumnList) -> Frame {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(WIDTH + 2.0 * ORIGIN.x, 600.0),
            )),
            ..Default::default()
        };
        let mut drawn = None;
        let mut texts = Vec::new();
        let mut collisions = None;
        let output = self.ctx.run_ui(raw, |ui| {
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(egui::Rect::from_min_size(ORIGIN, egui::vec2(WIDTH, 560.0))),
                |ui| drawn = Some(list.show(ui, self.mode)),
            );
            texts = text_ink::frame_text(ui.ctx());
            collisions = text_ink::collision_report(ui.ctx(), "the list");
        });
        Frame {
            drawn: drawn.expect("the list drew"),
            shapes: output.shapes,
            texts,
            collisions,
        }
    }

    /// `list` turned to its settings and drawn.
    fn settings(&self, list: &mut ColumnList) -> Frame {
        list.feed_events(&[key_event(egui::Key::Tab)]);
        assert_eq!(list.tab(), ListTab::Settings, "Tab turned the list");
        self.draw(list)
    }
}

/// The leaves of each shape the frame painted, with the `Vec`s opened.
fn leaves(frame: &Frame) -> Vec<&Shape> {
    fn open<'a>(shape: &'a Shape, out: &mut Vec<&'a Shape>) {
        match shape {
            Shape::Vec(inner) => inner.iter().for_each(|s| open(s, out)),
            other => out.push(other),
        }
    }
    let mut out = Vec::new();
    for clipped in &frame.shapes {
        open(&clipped.shape, &mut out);
    }
    out
}

/// Every filled rectangle the frame painted, with its fill.
fn fills(frame: &Frame) -> Vec<(egui::Rect, egui::Color32)> {
    leaves(frame)
        .into_iter()
        .filter_map(|s| match s {
            Shape::Rect(r) if r.fill != egui::Color32::TRANSPARENT => Some((r.rect, r.fill)),
            _ => None,
        })
        .collect()
}

/// The text painted inside `region`, by where its ink is centred.
fn texts_in(frame: &Frame, region: egui::Rect) -> Vec<&str> {
    frame
        .texts
        .iter()
        .filter(|t| region.contains(t.ink.center()))
        .map(|t| t.text.as_str())
        .collect()
}

/// The colour the galley painted at `at`'s top left was laid out in.
fn ink_at(frame: &Frame, at: egui::Rect) -> egui::Color32 {
    leaves(frame)
        .into_iter()
        .find_map(|s| match s {
            Shape::Text(t) if (t.pos - at.min).length() < 0.5 => {
                t.galley.job.sections.first().map(|sec| sec.format.color)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no text was painted at {at:?}"))
}

/// What the marker at `marker`'s centre is: `Some(true)` a filled dot,
/// `Some(false)` a hollow ring, `None` where no circle was painted.
fn marker_filled(frame: &Frame, marker: egui::Rect) -> Option<(bool, egui::Color32)> {
    leaves(frame).into_iter().find_map(|s| match s {
        Shape::Circle(c) if (c.center - marker.center()).length() < 0.5 => {
            if c.fill == egui::Color32::TRANSPARENT {
                Some((false, c.stroke.color))
            } else {
                Some((true, c.fill))
            }
        }
        _ => None,
    })
}

fn key_event(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// What a keystroke that types a character brings in one frame: the key press,
/// then the text it makes.
fn typed(key: egui::Key, text: &str) -> Vec<egui::Event> {
    vec![key_event(key), egui::Event::Text(text.to_string())]
}

/// `Tab`, as a frame brings it: a key press and no text.
fn tab() -> Vec<egui::Event> {
    vec![key_event(egui::Key::Tab)]
}

/// Press `/` and type `word`, a letter a frame, as a reader does.
fn search(list: &mut ColumnList, word: &str) {
    list.feed_events(&typed(egui::Key::Slash, "/"));
    for ch in word.chars() {
        let key = match ch {
            's' => egui::Key::S,
            'c' => egui::Key::C,
            't' => egui::Key::T,
            'z' => egui::Key::Z,
            other => panic!("the test has no key for {other:?}"),
        };
        list.feed_events(&typed(key, &ch.to_string()));
    }
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

fn categorical(mode: Mode, slot: usize) -> egui::Color32 {
    chrome::colour(if mode.is_dark() {
        viz::CATEGORICAL_DARK[slot]
    } else {
        viz::CATEGORICAL_LIGHT[slot]
    })
}

/// The names of the settings rows a frame drew, top to bottom.
fn drawn_rows(frame: &Frame) -> Vec<&str> {
    frame.drawn.settings.iter().map(|r| r.name).collect()
}

/// The head rows of an axis's settings, top to bottom. The one test that pins
/// their order reads them as a literal; every other test reaches a row by its
/// name, so a row added to the head moves none of them.
const HEAD_ROWS: [&str; 4] = [TITLE_ROW, SCALE_ROW, RANGE_ROW, FORMAT_ROW];

/// Step the cursor down with `j` until it reads `name`.
fn cursor_down_to(list: &mut ColumnList, name: &str) {
    for _ in 0..HEAD_ROWS.len() {
        if list.setting_cursor().map(|r| r.name) == Some(name) {
            return;
        }
        list.feed_events(&typed(egui::Key::J, "j"));
    }
    assert_eq!(
        list.setting_cursor().map(|r| r.name),
        Some(name),
        "`j` did not reach the row"
    );
}

/// The row a frame drew under `name`.
fn drawn_row<'a>(frame: &'a Frame, name: &str) -> &'a SettingRowDrawn {
    frame
        .drawn
        .settings
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no row named {name} was drawn"))
}

// ---------------------------------------------------------------------------
// AC1: Tab turns the list, and the list's head says what it is.
// ---------------------------------------------------------------------------

/// **`Tab` turns x's list to its settings and `Tab` again turns it back.** The
/// turn to the settings reports it, so the window can back out a preview; the
/// turn back puts the cursor on the column the channel holds.
#[test]
fn tab_turns_the_x_list_to_its_settings_and_tab_again_turns_it_back() {
    let mut list = list(ShelfChannel::X);
    assert_eq!(
        list.tab(),
        ListTab::Columns,
        "the list opens on its columns"
    );

    let reports = list.feed_events(&tab());
    assert_eq!(reports, [ListReport::Turned(ListTab::Settings)]);
    assert_eq!(list.tab(), ListTab::Settings);
    assert_eq!(list.channel(), ShelfChannel::X, "the channel is kept");

    // Move off the first row, so the way back is not the way it came.
    list.feed_events(&typed(egui::Key::J, "j"));
    let reports = list.feed_events(&tab());
    assert_eq!(
        reports,
        [ListReport::Turned(ListTab::Columns)],
        "turned back to the column the channel holds, which is already the cursor's"
    );
    assert_eq!(list.tab(), ListTab::Columns);
    assert_eq!(
        list.cursor(),
        Some("population"),
        "the cursor is on the column x holds"
    );
}

/// **The list's head names the channel and the tile, and a strip under it reads
/// *columns · settings*, the open tab underlined in the channel's hue.**
#[test]
fn the_settings_list_is_headed_with_the_channel_and_the_tile_and_a_strip_of_two_tabs() {
    for mode in [Mode::Light, Mode::Dark] {
        let stage = Stage::new(mode);
        let hue = categorical(mode, TEAL);

        let mut list = list(ShelfChannel::X);
        let columns = stage.draw(&mut list);
        let strip = columns.drawn.tabs.as_ref().expect("x's list draws a strip");
        assert_eq!(
            texts_in(&columns, strip.rect),
            ["columns", "\u{b7}", "settings"],
            "{mode:?}: the strip's words"
        );
        let open: Vec<ListTab> = strip
            .tabs
            .iter()
            .filter(|t| t.bar.is_some())
            .map(|t| t.tab)
            .collect();
        assert_eq!(
            open,
            [ListTab::Columns],
            "{mode:?}: the open tab is columns"
        );

        let settings = stage.settings(&mut list);
        assert_eq!(settings.drawn.heading_text, X_HEADING);
        assert_eq!(
            texts_in(&settings, settings.drawn.heading),
            [X_HEADING],
            "{mode:?}: the heading row holds the heading and nothing else"
        );
        let strip = settings
            .drawn
            .tabs
            .as_ref()
            .expect("the settings tab draws the strip");
        assert_eq!(
            texts_in(&settings, strip.rect),
            ["columns", "\u{b7}", "settings"],
            "{mode:?}: the strip's words are the same on either tab"
        );
        let under: Vec<_> = strip.tabs.iter().filter(|t| t.bar.is_some()).collect();
        assert_eq!(under.len(), 1, "{mode:?}: one tab is underlined");
        let word = under[0];
        assert_eq!(word.tab, ListTab::Settings, "{mode:?}: it is settings");
        let bar = word.bar.expect("filtered on it");
        assert!(
            fills(&settings).contains(&(bar, hue)),
            "{mode:?}: the bar is painted in x's hue {hue:?}"
        );
        assert!(
            bar.left() < word.word.left()
                && bar.right() > word.word.right()
                && near(bar.bottom(), strip.rect.bottom()),
            "{mode:?}: the bar runs under the word *settings*, from {:?} to {:?}",
            bar.left(),
            bar.right()
        );
        assert!(
            strip.rect.top() >= settings.drawn.heading.bottom() - 0.01,
            "{mode:?}: the strip is under the heading"
        );
    }
}

/// **The head rows are title, scale, range and format, in that order, and a rule
/// follows them.** No column row is drawn on the settings tab.
#[test]
fn the_head_rows_are_title_scale_range_and_format_in_that_order_and_a_rule_follows_them() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);
    let frame = stage.settings(&mut list);
    assert_eq!(
        drawn_rows(&frame),
        ["title", "scale", "range", "format"],
        "the rows, top to bottom, in the words an analyst reads"
    );
    assert_eq!(
        HEAD_ROWS,
        ["title", "scale", "range", "format"],
        "the names the other tests reach the rows by are the same words"
    );
    let tops: Vec<f32> = frame.drawn.settings.iter().map(|r| r.rect.top()).collect();
    assert!(
        tops.windows(2).all(|w| w[0] < w[1]),
        "each row is below the one before: {tops:?}"
    );
    assert!(frame.drawn.rows.is_empty(), "no column row is drawn");
    let rule = frame.drawn.rule.expect("a rule follows the head rows");
    let last = frame.drawn.settings.last().expect("rows were drawn");
    assert!(
        rule.top() >= last.rect.bottom() - 0.01,
        "the rule is under the last row"
    );
    assert!(
        frame.drawn.foot.top() >= rule.bottom() - 0.01,
        "the foot is under the rule"
    );
}

/// **Text typed before `Tab` still narrows the rows**, the rows whose name
/// begins with it first and those that merely hold it after, and a rule no longer
/// follows what is left.
#[test]
fn text_typed_before_tab_narrows_the_settings_rows_and_the_rule_goes() {
    let stage = Stage::new(Mode::Light);

    let mut list = list(ShelfChannel::X);
    search(&mut list, "sc");
    list.feed_events(&tab());
    assert_eq!(list.query(), "sc", "the query is kept across the turn");
    let frame = stage.draw(&mut list);
    assert_eq!(drawn_rows(&frame), [SCALE_ROW]);
    assert!(
        frame.drawn.rule.is_none(),
        "a query typed takes the rule away"
    );

    // `title` and `ticks`, which a query finds by name, begin with the letter
    // and `format` only holds it: `scale` holds none.
    let mut list = self::list(ShelfChannel::X);
    search(&mut list, "t");
    list.feed_events(&tab());
    let frame = stage.draw(&mut list);
    assert_eq!(drawn_rows(&frame), [TITLE_ROW, TICKS_ROW, FORMAT_ROW]);
    assert_eq!(
        list.setting_cursor().map(|r| r.name),
        Some(TITLE_ROW),
        "the cursor is on the first row left"
    );

    // A query no row matches says so, in the channel's word.
    let mut list = self::list(ShelfChannel::X);
    search(&mut list, "zz");
    list.feed_events(&tab());
    let frame = stage.draw(&mut list);
    assert!(drawn_rows(&frame).is_empty());
    assert!(
        frame
            .texts
            .iter()
            .any(|t| t.text == "no setting has \"zz\" in its name"),
        "the list says no row is left: {:?}",
        frame.texts.iter().map(|t| &t.text).collect::<Vec<_>>()
    );
}

/// **`Esc` leaves the settings as it leaves the columns**: it clears a query
/// first, and with an empty query reports backing out, which the window answers by
/// returning to the band's cell.
#[test]
fn esc_clears_the_query_and_then_backs_out_of_the_settings_as_of_the_columns() {
    let mut list = list(ShelfChannel::X);
    list.feed_events(&tab());
    search(&mut list, "sc");
    let reports = list.feed_events(&[key_event(egui::Key::Escape)]);
    assert!(reports.is_empty(), "the first Esc only clears: {reports:?}");
    assert_eq!(list.query(), "");
    assert_eq!(list.tab(), ListTab::Settings, "the list stays on its tab");
    let reports = list.feed_events(&[key_event(egui::Key::Escape)]);
    assert_eq!(reports, [ListReport::BackedOut]);
}

/// **From x's settings `y` opens y's settings, and `c` names colour, whose list
/// has no settings and so goes to its columns.** The rows are the new channel's,
/// drawn in its hue.
#[test]
fn from_the_x_settings_y_opens_the_y_settings_and_c_leaves_for_the_colour_columns() {
    for mode in [Mode::Light, Mode::Dark] {
        let stage = Stage::new(mode);
        // y's title is its own column's name, so the rows tell the channels
        // apart.
        let mut list = list(ShelfChannel::X);
        let x = stage.settings(&mut list);
        assert_eq!(texts_in(&x, x.drawn.settings[0].value_rect), ["population"]);

        let reports = list.feed_events(&typed(egui::Key::Y, "y"));
        assert_eq!(reports, [ListReport::GoTo(ShelfChannel::Y)]);
        assert_eq!(list.channel(), ShelfChannel::Y);
        assert_eq!(list.tab(), ListTab::Settings, "{mode:?}: the tab is kept");
        let y = stage.draw(&mut list);
        assert_eq!(drawn_rows(&y), HEAD_ROWS);
        assert_eq!(
            texts_in(&y, y.drawn.settings[0].value_rect),
            ["latitude"],
            "{mode:?}: the rows are y's"
        );
        let strip = y.drawn.tabs.as_ref().expect("y's list draws a strip");
        let bar = strip
            .tabs
            .iter()
            .find_map(|t| t.bar)
            .expect("an open tab is underlined");
        assert!(
            fills(&y).contains(&(bar, categorical(mode, VIOLET))),
            "{mode:?}: the bar is in y's hue"
        );

        let reports = list.feed_events(&typed(egui::Key::C, "c"));
        assert_eq!(reports, [ListReport::GoTo(ShelfChannel::Colour)]);
        assert_eq!(list.channel(), ShelfChannel::Colour);
        assert_eq!(
            list.tab(),
            ListTab::Columns,
            "{mode:?}: colour has no settings to stay on"
        );
        assert_eq!(list.cursor(), Some("median_income"));
        let colour = stage.draw(&mut list);
        assert!(
            colour.drawn.tabs.is_none(),
            "{mode:?}: no strip names a tab `Tab` cannot reach"
        );
    }
}

/// **`Tab` on colour's or the mark's cell leaves the list on its columns**, and
/// draws no strip, as there is no second tab for a strip to name.
#[test]
fn tab_on_colours_or_the_marks_list_leaves_it_on_its_columns() {
    let stage = Stage::new(Mode::Light);
    for channel in [ShelfChannel::Colour, ShelfChannel::Mark] {
        let mut list = list(channel);
        let reports = list.feed_events(&tab());
        assert!(
            reports.is_empty(),
            "{channel:?}: Tab reports nothing: {reports:?}"
        );
        assert_eq!(list.tab(), ListTab::Columns, "{channel:?}");
        let frame = stage.draw(&mut list);
        assert!(frame.drawn.tabs.is_none(), "{channel:?}: no strip is drawn");
        assert!(
            frame.drawn.settings.is_empty() && !frame.drawn.rows.is_empty(),
            "{channel:?}: the columns are still listed"
        );
    }
}

/// **`h`, `l` and the arrows beside them are inert on the title row, and `Enter`
/// is inert on the scale row**, which takes no typed text; `j` and `k` move the
/// cursor and report no preview, because a settings row has no column to
/// preview. `Enter` on a row that does take typed text opens its field, which
/// `shelf_settings_typed.rs` reads.
#[test]
fn h_l_and_enter_do_nothing_on_a_settings_row_and_j_k_move_the_cursor_silently() {
    let mut list = list(ShelfChannel::X);
    list.feed_events(&tab());
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(TITLE_ROW));
    let before = (list.channel(), list.tab());

    for events in [
        typed(egui::Key::H, "h"),
        typed(egui::Key::L, "l"),
        vec![key_event(egui::Key::ArrowLeft)],
        vec![key_event(egui::Key::ArrowRight)],
    ] {
        let reports = list.feed_events(&events);
        assert!(reports.is_empty(), "{events:?} reported {reports:?}");
        assert_eq!((list.channel(), list.tab()), before, "{events:?}");
        assert_eq!(list.setting_cursor().map(|r| r.name), Some(TITLE_ROW));
    }

    let reports = list.feed_events(&typed(egui::Key::J, "j"));
    assert!(reports.is_empty(), "j reported {reports:?}");
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(SCALE_ROW));
    let reports = list.feed_events(&[key_event(egui::Key::Enter)]);
    assert!(
        reports.is_empty(),
        "Enter on the scale row reported {reports:?}"
    );
    assert!(
        list.field().is_none(),
        "Enter opened a field on the scale row"
    );
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(SCALE_ROW));
    for _ in 0..HEAD_ROWS.len() * 2 {
        list.feed_events(&typed(egui::Key::J, "j"));
    }
    assert_eq!(
        list.setting_cursor().map(|r| r.name),
        Some(FORMAT_ROW),
        "the cursor stops at the last row"
    );
    let reports = list.feed_events(&typed(egui::Key::K, "k"));
    assert!(reports.is_empty(), "k reported {reports:?}");
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(RANGE_ROW));
}

// ---------------------------------------------------------------------------
// AC2: a row reads auto where the value is brightfield's own, else what is set.
// ---------------------------------------------------------------------------

/// **A row is *set* by comparing values, and each reading is the one the
/// resolvers give.** `yScale: linear` is written and is brightfield's own, so it
/// reads *auto*; `yScale: log` is not; `yScale: sqrt` is a name this build draws
/// as linear, so it reads the linear it is drawn as, auto; `xLabel: null` takes
/// the title away, which is not the column's name, so the title row is set and
/// reads no text.
#[test]
fn a_row_is_set_by_value_so_a_written_linear_reads_auto_and_a_log_reads_set() {
    // (attributes, channel, row, the value it reads, whether it is set)
    let cases = [
        ("", ShelfChannel::Y, SCALE_ROW, "linear", false),
        (
            "yScale: linear",
            ShelfChannel::Y,
            SCALE_ROW,
            "linear",
            false,
        ),
        ("yScale: log", ShelfChannel::Y, SCALE_ROW, "log", true),
        ("yScale: symlog", ShelfChannel::Y, SCALE_ROW, "symlog", true),
        ("yScale: sqrt", ShelfChannel::Y, SCALE_ROW, "linear", false),
        // The scale of the other axis is its own.
        ("yScale: log", ShelfChannel::X, SCALE_ROW, "linear", false),
        ("xLabel: null", ShelfChannel::X, TITLE_ROW, NO_TITLE, true),
        // A title the file writes that is the column's own name is brightfield's
        // own, and one that is not is the analyst's.
        ("", ShelfChannel::X, TITLE_ROW, "population", false),
        (
            "xLabel: population",
            ShelfChannel::X,
            TITLE_ROW,
            "population",
            false,
        ),
        (
            "xLabel: Residents",
            ShelfChannel::X,
            TITLE_ROW,
            "Residents",
            true,
        ),
        ("", ShelfChannel::X, FORMAT_ROW, AUTO, false),
        (
            "xTickFormat: ',d'",
            ShelfChannel::X,
            FORMAT_ROW,
            "custom",
            true,
        ),
        (
            "xTickFormat: ',f'",
            ShelfChannel::X,
            FORMAT_ROW,
            "number",
            true,
        ),
        (
            "xTickFormat: '~s'",
            ShelfChannel::X,
            FORMAT_ROW,
            "short",
            true,
        ),
        (
            "yTickFormat: '%'",
            ShelfChannel::Y,
            FORMAT_ROW,
            "percent",
            true,
        ),
        (
            "yTickFormat: '$,f'",
            ShelfChannel::Y,
            FORMAT_ROW,
            "currency",
            true,
        ),
    ];
    for (attrs, channel, name, value, set) in cases {
        let row = row_of(attrs, channel, name);
        assert_eq!(
            (row.value.as_str(), row.set),
            (value, set),
            "{channel:?} {name} with `{attrs}`"
        );
    }
}

/// **A row that reads auto draws its value in muted ink with the word *auto* and
/// a hollow ring; a row that is set draws it in full ink with a filled dot and
/// no *auto*.** Read off the painted frame in both themes: the galley's ink, the
/// shape at the marker and the word's place.
#[test]
fn an_auto_row_draws_muted_ink_and_a_ring_and_a_set_row_draws_full_ink_and_a_dot() {
    for mode in [Mode::Light, Mode::Dark] {
        let stage = Stage::new(mode);
        let sem = semantic(mode.is_dark());
        let muted = chrome::colour(sem.text.muted);
        let primary = chrome::colour(sem.text.primary);
        let ring = chrome::colour(sem.borders.default_);

        // Title and scale are the analyst's, the range and the format are
        // brightfield's own. Nothing is drawn, so the range has no ends to show
        // and the word *auto* stands alone.
        let mut list = list_with(ShelfChannel::X, "xLabel: Residents\nxScale: log");
        let frame = stage.settings(&mut list);
        for (row, set, value) in [
            (drawn_row(&frame, TITLE_ROW), true, &["Residents"][..]),
            (drawn_row(&frame, SCALE_ROW), true, &["log"][..]),
            (drawn_row(&frame, RANGE_ROW), false, &[][..]),
            (drawn_row(&frame, FORMAT_ROW), false, &[AUTO][..]),
        ] {
            let name = row.name;
            assert_eq!(
                texts_in(&frame, row.value_rect),
                value,
                "{mode:?} {name}: the value read"
            );
            assert_eq!(
                ink_at(&frame, row.value_rect),
                if set { primary } else { muted },
                "{mode:?} {name}: the value's ink"
            );
            let (filled, colour) = marker_filled(&frame, row.marker).unwrap_or_else(|| {
                panic!("{mode:?} {name}: nothing round was painted at the marker")
            });
            assert_eq!(
                filled, set,
                "{mode:?} {name}: a dot where set, a ring where auto"
            );
            assert_eq!(
                colour,
                if set { primary } else { ring },
                "{mode:?} {name}: the marker's colour"
            );
            match row.auto_rect {
                Some(auto) => {
                    assert!(!set, "{mode:?} {name}: a set row says auto");
                    assert_eq!(texts_in(&frame, auto), [AUTO], "{mode:?} {name}");
                    assert_eq!(
                        ink_at(&frame, auto),
                        muted,
                        "{mode:?} {name}: auto is muted"
                    );
                }
                None => assert!(set, "{mode:?} {name}: an auto row does not say auto"),
            }
        }
    }
}

/// **`xLabel: null` draws the title row as set, with the word for no text.**
#[test]
fn a_title_the_file_takes_away_draws_as_set_with_no_text() {
    let stage = Stage::new(Mode::Light);
    let mut list = list_with(ShelfChannel::X, "xLabel: null");
    let frame = stage.settings(&mut list);
    let title = &frame.drawn.settings[0];
    assert_eq!(texts_in(&frame, title.value_rect), [NO_TITLE]);
    assert!(title.auto_rect.is_none(), "a set row does not say auto");
    assert_eq!(
        marker_filled(&frame, title.marker).map(|(filled, _)| filled),
        Some(true)
    );
}

/// **The format row reads a format as the axis reader does.** The row and
/// `resolve_tick_formats` judge one attribute; where the reader takes a format
/// the row is set and reads it, and where the reader refuses one the axis draws
/// its own tick text and the row reads *auto*. A row that took `~~` for a format
/// would claim a setting no axis draws.
#[test]
fn a_format_row_agrees_with_the_reader_over_what_is_a_format() {
    for written in [
        "'.2s'", "',d'", "'+.1f'", "s", "'%'", "'~~'", "'+f'", "5", "true", "''", "null",
    ] {
        for (channel, key) in [
            (ShelfChannel::X, "xTickFormat"),
            (ShelfChannel::Y, "yTickFormat"),
        ] {
            let attrs = format!("{key}: {written}");
            let source = format!(
                "data:\n  t:\n    - {{ a: 1 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: a\n    y: a\nwidth: 600\nheight: 300\n{attrs}\n"
            );
            let spec = parse_spec(&source, Format::Yaml).expect("parses").spec;
            let plot = plot_at_path(&spec, "root").expect("one plot");
            let read = resolve_tick_formats(plot);
            let reader_takes = match channel {
                ShelfChannel::X => read.x.is_some(),
                _ => read.y.is_some(),
            };
            let row = row_of(&attrs, channel, FORMAT_ROW);
            assert_eq!(
                row.set,
                reader_takes,
                "`{attrs}`: the row is {}, the reader {}",
                if row.set { "set" } else { "auto" },
                if reader_takes {
                    "takes it"
                } else {
                    "refuses it"
                }
            );
            if !reader_takes {
                assert_eq!(row.value, AUTO, "`{attrs}`: a refused format reads auto");
            }
            // The other axis's row is untouched by this key.
            let other = if channel == ShelfChannel::X {
                ShelfChannel::Y
            } else {
                ShelfChannel::X
            };
            assert!(
                !row_of(&attrs, other, FORMAT_ROW).set,
                "`{attrs}` moved the other axis's format row"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// AC3: the foot's sentence for the row under the cursor.
// ---------------------------------------------------------------------------

/// **The foot carries one sentence for the row under the cursor: what it does
/// and the rule for its default.** It follows the cursor, and the columns state
/// has none.
#[test]
fn the_foot_carries_one_sentence_for_the_row_under_the_cursor() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);
    assert!(
        stage.draw(&mut list).drawn.sentence.is_none(),
        "the columns state has no row to speak of"
    );
    stage.settings(&mut list);

    let sentences = [
        (
            TITLE_ROW,
            "The words along the axis, which auto takes from the column's name.",
        ),
        (
            SCALE_ROW,
            "How values are spaced along the axis, which auto draws linear.",
        ),
        (
            RANGE_ROW,
            "The two numbers the axis runs between, which auto leaves to the rows.",
        ),
        (
            FORMAT_ROW,
            "How a tick's number or date is written, which auto leaves to the axis's own tick text. Takes: auto, number, short, percent, currency, custom.",
        ),
    ];
    for (row, said) in sentences {
        cursor_down_to(&mut list, row);
        let frame = stage.draw(&mut list);
        let sentence = frame.drawn.sentence.expect("a row is under the cursor");
        assert_eq!(texts_in(&frame, sentence), [said], "{row} row");
        assert!(
            said.contains(AUTO),
            "{row} row: the sentence says what auto is"
        );
        assert!(
            sentence.top() >= frame.drawn.foot.top() - 0.01
                && sentence.bottom() <= frame.drawn.foot.bottom() + 0.01,
            "{row} row: the sentence stands in the foot"
        );
    }
}

// ---------------------------------------------------------------------------
// The settings list draws no text into another's place.
// ---------------------------------------------------------------------------

/// **No two texts of the settings list are laid into one place**, in both
/// themes, with every row set and a title too long for the room the row leaves.
#[test]
fn no_two_texts_in_the_settings_list_are_drawn_into_one_place() {
    let long = "xLabel: 'A title so long that it can never fit in the room the row leaves it'";
    for mode in [Mode::Light, Mode::Dark] {
        let stage = Stage::new(mode);
        for attrs in [
            String::new(),
            format!("{long}\nxScale: log\nxTickFormat: ',d'"),
        ] {
            let mut list = list_with(ShelfChannel::X, &attrs);
            let frame = stage.settings(&mut list);
            assert_eq!(frame.collisions, None, "{mode:?} with `{attrs}`");
        }
    }
    // The long title is cut to its room and says so.
    let stage = Stage::new(Mode::Light);
    let mut list = list_with(ShelfChannel::X, long);
    let frame = stage.settings(&mut list);
    let title = &frame.drawn.settings[0];
    let drawn: Vec<&DrawnText> = frame
        .texts
        .iter()
        .filter(|t| title.value_rect.contains(t.ink.center()))
        .collect();
    assert_eq!(drawn.len(), 1);
    assert!(
        drawn[0].elided,
        "the title is cut to the room the row leaves"
    );
    assert!(
        drawn[0].ink.right() <= title.marker.left(),
        "and it stops before the marker"
    );
}
