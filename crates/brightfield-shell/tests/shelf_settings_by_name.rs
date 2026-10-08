//! **An axis's settings list finds ticks, grid, zero and reverse by name, and a
//! row the render crate's judges say does not apply is drawn muted with the
//! reason.**
//!
//! `shelf_settings.rs` holds the head rows, title, scale and format, and the
//! list's keys. This file holds the four rows the list does not show until a
//! query names them. The band's cell, which carries the dot and the scale's
//! name, is `shelf_band.rs`; the window handing the list the scales its chart
//! was drawn against is `shelf_settings_window.rs`.
//!
//! Three kinds of reading, each at the altitude its claim lives at:
//!
//! - **Which rows are listed** is read off the frame a list drew, by the names of
//!   the rows it laid out, for a list with no query and for one with each
//!   word typed.
//! - **Where a row does not apply** is asked of the render crate's own judges,
//!   over every kind of scale a plot draws, and the rows are read against that
//!   answer. The test does not restate what each judge says: a judge changed in
//!   the render crate moves the oracle and the row with it, and a row that stops
//!   asking the judge does not.
//! - **How a row looks** is read off the shapes the frame painted: the ink each
//!   galley was laid out in and the marker at each row's trailing end. The
//!   pixels are baselines in `outline_list.rs`, beside the head rows' own, since
//!   a target that draws through the wgpu renderer has to be one of the
//!   workflow's serial targets and this one is not.

use brightfield_render::axis::tick_count_applies;
use brightfield_render::channel::Channel;
use brightfield_render::mark::Projection;
use brightfield_render::scale::{Scale, ScaleSet};
use brightfield_render::scene::{axis_ends_apply, axis_keys_apply, axis_reverse_applies};
use brightfield_shell::design::{self, Mode};
use brightfield_shell::shelf::{
    Binding, ChannelSettings, ColumnList, ColumnListRequest, ListColumn, ListDrawn, ListTab,
    SettingRow, ShelfChannels, AUTO, FORMAT_ROW, GRID_ROW, OFF, ON, REVERSE_ROW, SCALE_ROW,
    TICKS_ROW, TITLE_ROW, ZERO_ROW,
};
use brightfield_shell::text_ink::{self, DrawnText};
use brightfield_spec::edit::plot_at_path;
use brightfield_spec::layout::DEFAULT_TICK_COUNT;
use brightfield_spec::parse::{parse_spec, Format};
use brightfield_workbench::channel::ShelfChannel;
use brightfield_workbench::chrome;
use egui::epaint::{ClippedShape, Shape};
use meridian_design::semantic;

/// The Outline rail's default width.
const WIDTH: f32 = 240.0;

/// Where the list's top left sits in the test's window, off the origin so no
/// reading assumes the list starts at zero.
const ORIGIN: egui::Pos2 = egui::pos2(12.0, 9.0);

/// The four rows found by name, in the order the list keeps them.
const BY_NAME: [&str; 4] = [TICKS_ROW, GRID_ROW, ZERO_ROW, REVERSE_ROW];

// ---------------------------------------------------------------------------
// The fixture: a table, the channels the tile takes, and the scales it drew.
// ---------------------------------------------------------------------------

/// A table with no data behind it: the list is offered names and types.
fn columns() -> Vec<ListColumn> {
    [
        ("longitude", "DOUBLE"),
        ("latitude", "DOUBLE"),
        ("population", "BIGINT"),
    ]
    .into_iter()
    .map(|(name, kind)| ListColumn {
        name: name.to_string(),
        kind: kind.to_string(),
        moments: None,
    })
    .collect()
}

fn channels() -> ShelfChannels {
    ShelfChannels {
        mark: "dot".to_string(),
        x: Binding::Column("population".to_string()),
        y: Binding::Column("latitude".to_string()),
        colour: Binding::Unset,
    }
}

fn linear() -> Scale {
    Scale::Linear {
        domain_min: 0.0,
        domain_max: 10.0,
        range_start: 0.0,
        range_end: 100.0,
    }
}

fn log() -> Scale {
    Scale::Log {
        domain_min: 1.0,
        domain_max: 1000.0,
        range_start: 0.0,
        range_end: 100.0,
    }
}

fn symlog() -> Scale {
    Scale::Symlog {
        domain_min: -10.0,
        domain_max: 1000.0,
        range_start: 0.0,
        range_end: 100.0,
    }
}

fn time() -> Scale {
    Scale::Time {
        domain_min_us: 0,
        domain_max_us: 86_400_000_000,
        range_start: 0.0,
        range_end: 100.0,
    }
}

fn names() -> Scale {
    Scale::Band {
        categories: vec!["north".to_string(), "south".to_string()],
        range_start: 0.0,
        range_end: 100.0,
        padding: 0.1,
    }
}

fn days() -> Scale {
    Scale::Band {
        categories: vec!["2026-10-01".to_string(), "2026-10-02".to_string()],
        range_start: 0.0,
        range_end: 100.0,
        padding: 0.1,
    }
}

/// Every kind of scale a positional axis draws, with a word for the failure
/// message.
fn every_scale() -> Vec<(&'static str, Scale)> {
    vec![
        ("linear", linear()),
        ("log", log()),
        ("symlog", symlog()),
        ("time", time()),
        ("names", names()),
        ("days", days()),
    ]
}

/// What a plot draws when both its axes are `scale`.
fn drawn_with(scale: &Scale) -> ScaleSet {
    let mut set = ScaleSet::new();
    set.insert(Channel::X, scale.clone());
    set.insert(Channel::Y, scale.clone());
    set
}

/// What a plot with a map projection draws: linear axes in the projection's
/// planar units, and the projection recorded on the set.
fn projected() -> ScaleSet {
    let mut set = drawn_with(&linear());
    set.set_projection(Projection::Mercator, None);
    set
}

/// The plot attributes' reading as the window hands it to the list: a one-plot
/// spec whose top-level lines after `height` are `attrs`, read against the
/// scales `drawn`.
fn settings_of(attrs: &str, drawn: &ScaleSet) -> ChannelSettings {
    let source = format!(
        "data:\n  t:\n    - {{ a: 1 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: a\n    y: a\nwidth: 600\nheight: 300\n{attrs}\n"
    );
    let spec = parse_spec(&source, Format::Yaml)
        .expect("the spec parses")
        .spec;
    let plot = plot_at_path(&spec, "root").expect("the spec's root is its plot");
    ChannelSettings::of_plot_drawn(&spec, plot, &channels(), drawn)
}

/// A list on `channel`, handed what the plot with `attrs` reads against `drawn`.
fn list_with(channel: ShelfChannel, attrs: &str, drawn: &ScaleSet) -> ColumnList {
    let mut list = ColumnList::new(ColumnListRequest {
        tile: "hero".to_string(),
        channel,
        channels: channels(),
        columns: columns(),
    });
    list.set_settings(settings_of(attrs, drawn));
    list
}

/// The row of `channel`'s settings named `name`.
fn row_of(attrs: &str, drawn: &ScaleSet, channel: ShelfChannel, name: &str) -> SettingRow {
    settings_of(attrs, drawn)
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

struct Frame {
    drawn: ListDrawn,
    shapes: Vec<ClippedShape>,
    texts: Vec<DrawnText>,
}

impl Stage {
    /// A stage with the theme applied and the faces loaded: the fonts `apply`
    /// installs take effect on the pass after it, so two frames run before
    /// anything is measured.
    fn new(mode: Mode) -> Self {
        let ctx = egui::Context::default();
        design::apply(&ctx, mode);
        let stage = Self { ctx, mode };
        let mut warm = list_with(ShelfChannel::X, "", &ScaleSet::new());
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
        let output = self.ctx.run_ui(raw, |ui| {
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(egui::Rect::from_min_size(ORIGIN, egui::vec2(WIDTH, 560.0))),
                |ui| drawn = Some(list.show(ui, self.mode)),
            );
            texts = text_ink::frame_text(ui.ctx());
        });
        Frame {
            drawn: drawn.expect("the list drew"),
            shapes: output.shapes,
            texts,
        }
    }

    /// `list` turned to its settings, with `word` typed, and drawn.
    fn found(&self, list: &mut ColumnList, word: &str) -> Frame {
        search(list, word);
        list.feed_events(&[key_event(egui::Key::Tab)]);
        assert_eq!(list.tab(), ListTab::Settings, "Tab turned the list");
        assert_eq!(list.query(), word, "the query is kept across the turn");
        self.draw(list)
    }

    /// `list` turned to its settings with no query, and drawn.
    fn rest(&self, list: &mut ColumnList) -> Frame {
        list.feed_events(&[key_event(egui::Key::Tab)]);
        assert_eq!(list.tab(), ListTab::Settings, "Tab turned the list");
        self.draw(list)
    }

    fn muted(&self) -> egui::Color32 {
        chrome::colour(semantic(self.mode.is_dark()).text.muted)
    }

    fn primary(&self) -> egui::Color32 {
        chrome::colour(semantic(self.mode.is_dark()).text.primary)
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
/// `Some(false)` a hollow ring, `None` where no circle was painted, with the ink.
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

/// Press `/` and type `word`, a letter a frame, as a reader does.
fn search(list: &mut ColumnList, word: &str) {
    list.feed_events(&typed(egui::Key::Slash, "/"));
    for ch in word.chars() {
        let key = egui::Key::from_name(&ch.to_ascii_uppercase().to_string())
            .unwrap_or_else(|| panic!("no key for {ch:?}"));
        list.feed_events(&typed(key, &ch.to_string()));
    }
}

/// The names of the settings rows a frame drew, top to bottom.
fn drawn_rows(frame: &Frame) -> Vec<&str> {
    frame.drawn.settings.iter().map(|r| r.name).collect()
}

// ---------------------------------------------------------------------------
// AC1: found by name, and each reading auto or its set value.
// ---------------------------------------------------------------------------

/// **With the query empty an axis's list shows its three head rows and none of
/// the four found by name**, on either axis, whatever the axis draws, so a row
/// that does not apply is not offered either. The four are there to be found:
/// each axis holds them behind the head rows, flagged by name.
#[test]
fn with_the_query_empty_the_list_shows_its_head_rows_and_no_by_name_row() {
    let stage = Stage::new(Mode::Light);
    let mut sets = vec![("nothing drawn", ScaleSet::new())];
    sets.extend(
        every_scale()
            .into_iter()
            .map(|(word, scale)| (word, drawn_with(&scale))),
    );
    sets.push(("projected", projected()));
    for (word, drawn) in &sets {
        for channel in [ShelfChannel::X, ShelfChannel::Y] {
            let mut list = list_with(channel, "", drawn);
            let frame = stage.rest(&mut list);
            assert_eq!(
                drawn_rows(&frame),
                [TITLE_ROW, SCALE_ROW, FORMAT_ROW],
                "{channel:?} over {word} lists its head rows alone"
            );
            let rows = settings_of("", drawn).rows(channel).to_vec();
            let found: Vec<&str> = rows.iter().filter(|r| r.by_name).map(|r| r.name).collect();
            assert_eq!(found, BY_NAME, "{channel:?} over {word} holds the four");
            assert!(
                rows.iter()
                    .filter(|r| !r.by_name)
                    .all(|r| [TITLE_ROW, SCALE_ROW, FORMAT_ROW].contains(&r.name)),
                "no head row is flagged as found by name"
            );
        }
    }
}

/// **Typing a row's name lists it, and each reads auto while the plot sets
/// nothing**: the word *auto*, a hollow ring and muted ink, as the head rows read.
/// `tick` finds `ticks` alone, and `grid`, `zero` and `reverse` find their own.
#[test]
fn typing_a_name_lists_its_row_and_each_reads_auto_while_the_plot_sets_nothing() {
    let stage = Stage::new(Mode::Light);
    let drawn = drawn_with(&linear());
    let wanted = [
        ("tick", TICKS_ROW, DEFAULT_TICK_COUNT.to_string()),
        ("grid", GRID_ROW, ON.to_string()),
        ("zero", ZERO_ROW, OFF.to_string()),
        ("reverse", REVERSE_ROW, OFF.to_string()),
    ];
    for (word, name, value) in wanted {
        for channel in [ShelfChannel::X, ShelfChannel::Y] {
            let mut list = list_with(channel, "", &drawn);
            let frame = stage.found(&mut list, word);
            assert_eq!(drawn_rows(&frame), [name], "{word} on {channel:?}");
            let row = &frame.drawn.settings[0];
            assert!(
                !settings_of("", &drawn)
                    .rows(channel)
                    .iter()
                    .find(|r| r.name == name)
                    .expect("the row")
                    .set,
                "{name} reads as brightfield's own"
            );
            let auto = row.auto_rect.expect("an auto row carries the word auto");
            assert_eq!(texts_in(&frame, auto), [AUTO], "{name} says auto");
            assert_eq!(
                texts_in(&frame, row.value_rect),
                [value.as_str()],
                "{name} reads brightfield's own value"
            );
            let (filled, ring) = marker_filled(&frame, row.marker).expect("a marker");
            assert!(!filled, "{name}'s marker is a hollow ring");
            assert_ne!(ring, stage.primary());
            assert_eq!(ink_at(&frame, row.value_rect), stage.muted());
            assert!(row.reason_rect.is_none(), "{name} applies to a linear axis");
        }
    }
}

/// **A row the plot sets reads its value, in the text ink with a filled dot and
/// no word *auto*, on the axis the key names and on that axis alone**: `xTicks`
/// is x's row and y's reads auto.
#[test]
fn a_row_the_plot_sets_reads_its_value_and_a_key_names_its_own_axis_alone() {
    let stage = Stage::new(Mode::Light);
    let drawn = drawn_with(&linear());
    let x_set = "xTicks: 8\nxGrid: false\nxZero: true\nxReverse: true";
    let wanted = [
        ("tick", TICKS_ROW, "8"),
        ("grid", GRID_ROW, OFF),
        ("zero", ZERO_ROW, ON),
        ("reverse", REVERSE_ROW, ON),
    ];
    for (word, name, value) in wanted {
        let mut list = list_with(ShelfChannel::X, x_set, &drawn);
        let frame = stage.found(&mut list, word);
        assert_eq!(drawn_rows(&frame), [name]);
        let row = &frame.drawn.settings[0];
        assert!(row.auto_rect.is_none(), "{name} set is no auto row");
        assert_eq!(texts_in(&frame, row.value_rect), [value], "{name}'s value");
        let (filled, dot) = marker_filled(&frame, row.marker).expect("a marker");
        assert!(filled, "{name}'s marker is a filled dot");
        assert_eq!(dot, stage.primary());
        assert_eq!(ink_at(&frame, row.value_rect), stage.primary());

        // The same plot's y axis sets none of them.
        let mut list = list_with(ShelfChannel::Y, x_set, &drawn);
        let frame = stage.found(&mut list, word);
        let row = &frame.drawn.settings[0];
        assert!(row.auto_rect.is_some(), "{name} on y reads auto");
        assert!(!row_of(x_set, &drawn, ShelfChannel::Y, name).set);

        // And y's own key sets y's row.
        let y_set = x_set.replace('x', "y");
        let mut list = list_with(ShelfChannel::Y, &y_set, &drawn);
        let frame = stage.found(&mut list, word);
        let row = &frame.drawn.settings[0];
        assert_eq!(texts_in(&frame, row.value_rect), [value], "y's {name}");
        assert!(row.auto_rect.is_none());
    }
}

/// **A value is the analyst's when it differs from brightfield's own, however the
/// file wrote it**: a written default reads auto, as `yScale: linear` does.
#[test]
fn a_value_equal_to_brightfields_own_reads_auto_however_the_file_wrote_it() {
    let drawn = drawn_with(&linear());
    let written =
        format!("xTicks: {DEFAULT_TICK_COUNT}\nxGrid: true\nxZero: false\nxReverse: false");
    for name in BY_NAME {
        let row = row_of(&written, &drawn, ShelfChannel::X, name);
        assert!(!row.set, "{name} written as its default reads auto");
    }
    let differing = format!(
        "xTicks: {}\nxGrid: false\nxZero: true\nxReverse: true",
        DEFAULT_TICK_COUNT + 1
    );
    for name in BY_NAME {
        let row = row_of(&differing, &drawn, ShelfChannel::X, name);
        assert!(row.set, "{name} written as another value reads set");
    }
}

// ---------------------------------------------------------------------------
// AC2: a row that does not apply is muted, with the reason the judge gives.
// ---------------------------------------------------------------------------

/// **A row carries a reason exactly where the render crate's judge says the key
/// does not apply**, over the kinds of scale a positional axis draws: ticks
/// where `tick_count_applies` is false, zero where `axis_ends_apply` is, and,
/// under a map projection, the four where `axis_keys_apply` and
/// `axis_reverse_applies` are. The grid row has no judge of its own and carries
/// a reason under a projection alone.
#[test]
fn a_row_carries_a_reason_exactly_where_the_render_judge_says_it_does_not_apply() {
    for (word, scale) in every_scale() {
        let drawn = drawn_with(&scale);
        for channel in [ShelfChannel::X, ShelfChannel::Y] {
            let reasons = |name: &str| row_of("", &drawn, channel, name).reason;
            assert_eq!(
                reasons(TICKS_ROW).is_some(),
                !tick_count_applies(&scale),
                "ticks on {channel:?} over {word}"
            );
            assert_eq!(
                reasons(ZERO_ROW).is_some(),
                !axis_ends_apply(&scale),
                "zero on {channel:?} over {word}"
            );
            assert_eq!(
                reasons(REVERSE_ROW).is_some(),
                !axis_reverse_applies(&drawn),
                "reverse on {channel:?} over {word}"
            );
            assert!(reasons(GRID_ROW).is_none(), "grid over {word} applies");
            for name in [TITLE_ROW, SCALE_ROW, FORMAT_ROW] {
                assert!(reasons(name).is_none(), "{name} over {word} applies");
            }
        }
    }

    let drawn = projected();
    assert!(!axis_keys_apply(&drawn) && !axis_reverse_applies(&drawn));
    for channel in [ShelfChannel::X, ShelfChannel::Y] {
        let reasons: Vec<Option<String>> = BY_NAME
            .iter()
            .map(|name| row_of("", &drawn, channel, name).reason)
            .collect();
        assert!(
            reasons.iter().all(Option::is_some),
            "every row found by name does not apply to a map's {channel:?}: {reasons:?}"
        );
        assert!(
            reasons.windows(2).all(|w| w[0] == w[1]),
            "the four share the one reason a projection gives: {reasons:?}"
        );
        assert!(row_of("", &drawn, channel, TITLE_ROW).reason.is_none());
    }
}

/// **Typed, a row that does not apply is listed, in muted ink with its reason
/// under it**: zero on a log axis, ticks on an axis of names and reverse on a
/// projected plot. The reason is the row's own, laid out below the row's line
/// and inside the row, and the same row on an axis it applies to has neither.
#[test]
fn a_typed_row_that_does_not_apply_is_listed_muted_with_its_reason() {
    for mode in [Mode::Light, Mode::Dark] {
        let stage = Stage::new(mode);
        let cases = [
            ("zero", ZERO_ROW, drawn_with(&log()), drawn_with(&linear())),
            (
                "tick",
                TICKS_ROW,
                drawn_with(&names()),
                drawn_with(&linear()),
            ),
            ("reverse", REVERSE_ROW, projected(), drawn_with(&linear())),
        ];
        for (word, name, off, on) in cases {
            let mut list = list_with(ShelfChannel::X, "", &off);
            let frame = stage.found(&mut list, word);
            assert_eq!(drawn_rows(&frame), [name], "{word} finds {name}");
            let row = &frame.drawn.settings[0];
            let reason = row_of("", &off, ShelfChannel::X, name)
                .reason
                .expect("the judge says it does not apply");
            let at = row.reason_rect.expect("the reason is laid out");
            assert_eq!(texts_in(&frame, at), [reason.as_str()], "{name}'s reason");
            assert!(
                at.top() >= row.name_rect.bottom() - 0.01 && row.rect.contains_rect(at),
                "the reason sits under the line, inside the row"
            );
            assert_eq!(ink_at(&frame, at), stage.muted(), "the reason is muted");
            assert_eq!(
                ink_at(&frame, row.name_rect),
                stage.muted(),
                "{name}'s name is muted"
            );
            assert_eq!(ink_at(&frame, row.value_rect), stage.muted());
            let (_, ring) = marker_filled(&frame, row.marker).expect("a marker");
            assert_ne!(ring, stage.primary());

            // The same row where it applies is in the text ink and says no more.
            let mut list = list_with(ShelfChannel::X, "", &on);
            let frame = stage.found(&mut list, word);
            let row = &frame.drawn.settings[0];
            assert!(row.reason_rect.is_none());
            assert_eq!(ink_at(&frame, row.name_rect), stage.primary());
        }
    }
}

/// **A value the plot sets on an axis it does not apply to is the analyst's
/// still, and is drawn muted with its reason**: `xZero: true` on a log axis keeps
/// its dot, in muted ink, and says why it does nothing.
#[test]
fn a_set_value_on_an_axis_it_does_not_apply_to_keeps_its_dot_and_says_why() {
    let stage = Stage::new(Mode::Light);
    let drawn = drawn_with(&log());
    let row = row_of("xZero: true", &drawn, ShelfChannel::X, ZERO_ROW);
    assert!(row.set && row.reason.is_some());

    let mut list = list_with(ShelfChannel::X, "xZero: true", &drawn);
    let frame = stage.found(&mut list, "zero");
    let row = &frame.drawn.settings[0];
    assert!(row.auto_rect.is_none());
    assert!(row.reason_rect.is_some());
    let (filled, ink) = marker_filled(&frame, row.marker).expect("a marker");
    assert!(filled, "the value is the analyst's, so the marker is a dot");
    assert_eq!(ink, stage.muted(), "and a dot in muted ink");
    assert_eq!(texts_in(&frame, row.value_rect), [ON]);
}

/// **A query narrows the by-name rows with the head rows**: a letter found in
/// several names lists each, the names that begin with it first.
#[test]
fn a_letter_lists_the_head_and_by_name_rows_that_hold_it_beginning_with_it_first() {
    let stage = Stage::new(Mode::Light);
    let mut list = list_with(ShelfChannel::X, "", &drawn_with(&linear()));
    let frame = stage.found(&mut list, "t");
    assert_eq!(
        drawn_rows(&frame),
        [TITLE_ROW, TICKS_ROW, FORMAT_ROW],
        "`title` and `ticks` begin with t and `format` holds it"
    );
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(TITLE_ROW));
    list.feed_events(&[key_event(egui::Key::Escape)]);
    let frame = stage.draw(&mut list);
    assert_eq!(
        drawn_rows(&frame),
        [TITLE_ROW, SCALE_ROW, FORMAT_ROW],
        "Esc clears the query and the by-name rows go with it"
    );
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(TITLE_ROW));
}

/// **The values stand in one column whichever rows a query leaves**: the names
/// share a column as wide as the longest of the channel's rows, the four found
/// by name included, so a value is where it was when the query is typed and
/// another row is left on the screen.
#[test]
fn the_values_stand_in_one_column_whichever_rows_a_query_leaves() {
    let stage = Stage::new(Mode::Light);
    let drawn = drawn_with(&linear());
    let mut list = list_with(ShelfChannel::X, "", &drawn);
    let rest = stage.rest(&mut list);
    let column = rest.drawn.settings[0].value_rect.left();
    assert!(
        rest.drawn
            .settings
            .iter()
            .all(|r| near(r.value_rect.left(), column)),
        "the head rows' values share a column"
    );
    for word in ["tick", "grid", "zero", "reverse", "o"] {
        let mut list = list_with(ShelfChannel::X, "", &drawn);
        let frame = stage.found(&mut list, word);
        assert!(!frame.drawn.settings.is_empty(), "{word} leaves rows");
        for row in &frame.drawn.settings {
            assert!(
                near(row.value_rect.left(), column),
                "{word} leaves {} at {}, the column is at {column}",
                row.name,
                row.value_rect.left()
            );
        }
    }
    // The longest name is the one found by name, and its value clears it.
    let mut list = list_with(ShelfChannel::X, "", &drawn);
    let frame = stage.found(&mut list, "reverse");
    let row = &frame.drawn.settings[0];
    assert!(
        row.value_rect.left() > row.name_rect.right(),
        "reverse's value follows its name"
    );
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}
