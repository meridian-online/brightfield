//! **`Tab` on colour's list turns it to its settings, whose head rows are scheme
//! and reverse: each reads *auto* or the value the plot sets, the scheme paints
//! a ramp and offers the four it can be set to, and `h`, `l` and a click write.**
//!
//! The list is drawn into a `Ui` a test gives it, so the claims are read off the
//! frame it painted and the reports it answers with, which are what the window
//! writes to the plot. The window's side, that the write reaches the file and
//! the chart redraws, is `shelf_settings_colour_window.rs`. Three kinds of
//! reading, each at the altitude its claim lives at:
//!
//! - **Rows** are read off `ChannelSettings` for the plot a test writes, with the
//!   scales the plot was drawn against and the marks it drew handed in as the
//!   window hands them.
//! - **Ink** is read off the frame: the mesh painted for each ramp, its colours
//!   strip by strip; the stroke painted for the ring; the galley of each name.
//! - **Pixels** are baselines, one for each theme: colour's list with the cursor
//!   on the scheme row, on the scale row of a diverging plot, on the scale row
//!   with its pivot field open, and on the scheme row of a diverging plot with
//!   its strip. Regenerate them with
//!   `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell --test
//!   shelf_settings_colour`, and read what moved before committing it.

use brightfield_engine::{ColumnMoments, Distribution};
use brightfield_render::channel::Channel;
use brightfield_render::scale::{ramp_at, Scale, ScaleSet, SequentialScheme};
use brightfield_shell::design::{self, Mode};
use brightfield_shell::shelf::{
    diverging_strip, foot_sentence, scheme_strip, Binding, ChannelSettings, ColourScaleWrite,
    ColumnList, ColumnListRequest, Ends, ListColumn, ListDrawn, ListReport, ListTab, Number,
    RowEdit, SchemeDrawn, SettingRow, SettingRowDrawn, SettingValue, ShelfChannels, DOMAIN_ROW,
    LEGEND_ROW, OFF, ON, PIVOT_ROW, REVERSE_ROW, SCALE_ROW, SCHEME_ROW, STEPS_ROW,
};
use brightfield_shell::text_ink::{self, DrawnText};
use brightfield_spec::edit::plot_at_path;
use brightfield_spec::parse::{parse_spec, Format};
use brightfield_spec::vocab::MarkKind;
use brightfield_workbench::channel::ShelfChannel;
use brightfield_workbench::chrome;
use egui::epaint::{ClippedShape, Shape};
use egui_kittest::{Harness, SnapshotOptions};
use meridian_design::{semantic, viz};

/// The Outline rail's default width.
const WIDTH: f32 = 240.0;

/// The narrowest the list's rail goes.
const NARROWEST: f32 = 160.0;

/// Where the list's top left sits in the test's window, off the origin so no
/// reading assumes the list starts at zero.
const ORIGIN: egui::Pos2 = egui::pos2(12.0, 9.0);

/// Colour's slot in the design system's categorical order, blue, gold, teal,
/// red, violet, orange, plum, green counted from zero: orange. Read from the
/// system's order and not from the function that assigns a channel its hue.
const ORANGE: usize = 5;

/// The four schemes the strip offers, in the order it draws them.
const OFFERED: [&str; 4] = ["viridis", "blues", "turbo", "meridian"];

// ---------------------------------------------------------------------------
// The fixture.
// ---------------------------------------------------------------------------

fn columns() -> Vec<ListColumn> {
    [
        ("longitude", "DOUBLE"),
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

fn channels() -> ShelfChannels {
    ShelfChannels {
        mark: "dot".to_string(),
        x: Binding::Column("longitude".to_string()),
        y: Binding::Column("longitude".to_string()),
        colour: Binding::Column("median_income".to_string()),
    }
}

/// A fill scale of a number column: a ramp.
fn ramp_scales() -> ScaleSet {
    let mut drawn = ScaleSet::new();
    drawn.insert(
        Channel::Fill,
        Scale::Sequential {
            domain_min: 0.0,
            domain_max: 10.0,
            stops: SequentialScheme::Viridis.stops(),
        },
    );
    drawn
}

/// A fill scale of a column of names: a categorical set.
fn names_scales() -> ScaleSet {
    let mut drawn = ScaleSet::new();
    drawn.insert(
        Channel::Fill,
        Scale::Colour {
            categories: vec!["inland".to_string(), "coast".to_string()],
            palette: vec![[0.2, 0.4, 0.8, 1.0], [0.9, 0.5, 0.1, 1.0]],
        },
    );
    drawn
}

/// What colour's settings read from a plot whose attributes are `attrs`, drawn
/// against `drawn` and, where `marks` is given, with those marks.
fn settings_over(attrs: &str, drawn: &ScaleSet, marks: Option<&[MarkKind]>) -> ChannelSettings {
    let source = format!(
        "data:\n  t:\n    - {{ a: 1 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: a\n    y: a\nwidth: 600\nheight: 300\n{attrs}\n"
    );
    let spec = parse_spec(&source, Format::Yaml)
        .expect("the spec parses")
        .spec;
    let plot = plot_at_path(&spec, "root").expect("the spec's root is its plot");
    match marks {
        Some(marks) => {
            ChannelSettings::of_plot_marked(&spec, plot, "root", &channels(), drawn, marks)
        }
        None => ChannelSettings::of_plot_drawn(&spec, plot, "root", &channels(), drawn),
    }
}

/// The row of colour's settings named `name`.
fn row_of(attrs: &str, name: &str) -> SettingRow {
    settings_over(attrs, &ramp_scales(), None)
        .rows(ShelfChannel::Colour)
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("colour has no {name} row"))
        .clone()
}

/// Colour's list, handed `settings`, not yet turned.
fn list_over(settings: ChannelSettings) -> ColumnList {
    let mut list = ColumnList::new(ColumnListRequest {
        tile: "hero".to_string(),
        channel: ShelfChannel::Colour,
        channels: channels(),
        columns: columns(),
    });
    list.set_settings(settings);
    list
}

/// Colour's list over a plot with `attrs`, turned to its settings: the cursor on
/// the scheme row.
fn turned(attrs: &str) -> ColumnList {
    turned_over(settings_over(attrs, &ramp_scales(), None))
}

fn turned_over(settings: ChannelSettings) -> ColumnList {
    let mut list = list_over(settings);
    list.feed_events(&[key_event(egui::Key::Tab)]);
    assert_eq!(list.tab(), ListTab::Settings, "Tab turned colour's list");
    list
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

/// What a keystroke that types a character brings in one frame.
fn typed(key: egui::Key, text: &str) -> Vec<egui::Event> {
    vec![key_event(key), egui::Event::Text(text.to_string())]
}

fn h() -> Vec<egui::Event> {
    typed(egui::Key::H, "h")
}

fn l() -> Vec<egui::Event> {
    typed(egui::Key::L, "l")
}

fn j() -> Vec<egui::Event> {
    typed(egui::Key::J, "j")
}

fn backspace() -> Vec<egui::Event> {
    vec![key_event(egui::Key::Backspace)]
}

fn set(row: &'static str, value: SettingValue) -> ListReport {
    ListReport::Set(RowEdit {
        channel: ShelfChannel::Colour,
        row,
        value,
    })
}

fn word(name: &str) -> SettingValue {
    SettingValue::Word(name.to_string())
}

// ---------------------------------------------------------------------------
// The stage: a list drawn into a `Ui` a test gives it, and what the frame held.
// ---------------------------------------------------------------------------

struct Stage {
    ctx: egui::Context,
    mode: Mode,
    width: f32,
}

struct Frame {
    drawn: ListDrawn,
    shapes: Vec<ClippedShape>,
    texts: Vec<DrawnText>,
    collisions: Option<String>,
}

impl Stage {
    fn new(mode: Mode) -> Self {
        Self::at(mode, WIDTH)
    }

    /// A stage `width` wide with the theme applied and the faces loaded: the
    /// fonts `apply` installs take effect on the pass after it, so two frames
    /// run before anything is measured.
    fn at(mode: Mode, width: f32) -> Self {
        let ctx = egui::Context::default();
        design::apply(&ctx, mode);
        let stage = Self { ctx, mode, width };
        let mut warm = turned("");
        stage.draw(&mut warm);
        stage.draw(&mut warm);
        stage
    }

    fn frame(&self, list: &mut ColumnList, events: Vec<egui::Event>) -> Frame {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(self.width + 2.0 * ORIGIN.x, 600.0),
            )),
            events,
            ..Default::default()
        };
        let mut drawn = None;
        let mut texts = Vec::new();
        let mut collisions = None;
        let output = self.ctx.run_ui(raw, |ui| {
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                    ORIGIN,
                    egui::vec2(self.width, 560.0),
                )),
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

    fn draw(&self, list: &mut ColumnList) -> Frame {
        self.frame(list, Vec::new())
    }

    /// A click at `at`: the pointer to it, a settled frame, a press, a release.
    /// The release's frame, and the reports the frames answered with.
    fn click(&self, list: &mut ColumnList, at: egui::Pos2) -> (Frame, Vec<ListReport>) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut reports = Vec::new();
        reports.extend(
            self.frame(list, vec![egui::Event::PointerMoved(at)])
                .drawn
                .reports,
        );
        reports.extend(self.draw(list).drawn.reports);
        reports.extend(self.frame(list, vec![button(true)]).drawn.reports);
        let last = self.frame(list, vec![button(false)]);
        reports.extend(last.drawn.reports.clone());
        (last, reports)
    }
}

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

fn near(a: egui::Rect, b: egui::Rect) -> bool {
    (a.min - b.min).length() < 0.01 && (a.max - b.max).length() < 0.01
}

/// The colour of each flat strip of the ramp painted into `ramp`, left to right.
fn ramp_ink(frame: &Frame, ramp: egui::Rect) -> Vec<egui::Color32> {
    let meshes: Vec<_> = leaves(frame)
        .into_iter()
        .filter_map(|s| match s {
            Shape::Mesh(mesh) if near(mesh.calc_bounds(), ramp) => Some(mesh),
            _ => None,
        })
        .collect();
    assert_eq!(
        meshes.len(),
        1,
        "exactly one mesh is painted into the ramp {ramp:?}"
    );
    meshes[0]
        .vertices
        .chunks(4)
        .map(|quad| quad[0].color)
        .collect()
}

/// A colour as the chrome's one boundary quantises it, worked here from the
/// numbers and not through the library's own conversion.
fn quantised(c: [f32; 4]) -> egui::Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgba_unmultiplied(q(c[0]), q(c[1]), q(c[2]), q(c[3]))
}

/// The ramp `scheme` is painted as: twenty-eight strips along its stops.
fn expected_ramp(scheme: SequentialScheme) -> Vec<egui::Color32> {
    let stops = scheme.stops();
    (0..28)
        .map(|i| quantised(ramp_at(&stops, f64::from(i) / 27.0)))
        .collect()
}

fn hue(mode: Mode) -> egui::Color32 {
    chrome::colour(if mode.is_dark() {
        viz::CATEGORICAL_DARK[ORANGE]
    } else {
        viz::CATEGORICAL_LIGHT[ORANGE]
    })
}

fn drawn_row<'a>(frame: &'a Frame, name: &str) -> &'a SettingRowDrawn {
    frame
        .drawn
        .settings
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no row named {name} was drawn"))
}

fn drawn_scheme(row: &SettingRowDrawn, scheme: SequentialScheme) -> &SchemeDrawn {
    row.schemes
        .iter()
        .find(|s| s.scheme == Some(scheme))
        .unwrap_or_else(|| panic!("the strip offers no {scheme:?}"))
}

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

/// The words of the foot's sentence, as one string.
fn sentence(frame: &Frame) -> String {
    let at = frame.drawn.sentence.expect("the foot drew a sentence");
    texts_in(frame, at).join(" ")
}

// ---------------------------------------------------------------------------
// The rows.
// ---------------------------------------------------------------------------

/// **AC1.** The head rows are scheme, scale and reverse, in that order, and the
/// pivot and steps rows are found by name behind them.
#[test]
fn colours_settings_have_the_head_rows_scheme_scale_legend_and_reverse_in_that_order() {
    for attrs in ["", "colorScheme: blues\ncolorReverse: true"] {
        let settings = settings_over(attrs, &ramp_scales(), None);
        let rows = settings.rows(ShelfChannel::Colour);
        assert_eq!(
            rows.iter()
                .filter(|r| !r.by_name)
                .map(|r| r.name)
                .collect::<Vec<_>>(),
            [SCHEME_ROW, SCALE_ROW, LEGEND_ROW, REVERSE_ROW],
            "`{attrs}`"
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r.by_name)
                .map(|r| r.name)
                .collect::<Vec<_>>(),
            [PIVOT_ROW, STEPS_ROW, DOMAIN_ROW],
            "`{attrs}`: found by name"
        );
        let mut list = turned(attrs);
        let stage = Stage::new(Mode::Light);
        let frame = stage.draw(&mut list);
        assert_eq!(
            frame
                .drawn
                .settings
                .iter()
                .map(|r| r.name)
                .collect::<Vec<_>>(),
            [SCHEME_ROW, SCALE_ROW, LEGEND_ROW, REVERSE_ROW],
            "the list draws the head rows and no more"
        );
        assert_eq!(list.setting_cursor().map(|r| r.name), Some(SCHEME_ROW));
    }
}

/// **AC1.** Each row reads *auto* or the value the plot sets, by value and not by
/// whether the file wrote it: `colorScheme: viridis` is written and reads auto.
#[test]
fn each_row_reads_auto_or_its_set_value() {
    // (the plot's attributes, scheme, scheme set, reverse, reverse set)
    let cases = [
        ("", "viridis", false, OFF, false),
        ("colorScheme: viridis", "viridis", false, OFF, false),
        ("colorScheme: blues", "blues", true, OFF, false),
        ("colorScheme: turbo", "turbo", true, OFF, false),
        ("colorScheme: meridian", "meridian", true, OFF, false),
        ("colorScheme: rdbu", "rdbu", true, OFF, false),
        // A name this build cannot draw is drawn as the default, and so read.
        ("colorScheme: magma", "viridis", false, OFF, false),
        ("colorReverse: true", "viridis", false, ON, true),
        ("colorReverse: false", "viridis", false, OFF, false),
        (
            "colorScheme: blues\ncolorReverse: true",
            "blues",
            true,
            ON,
            true,
        ),
    ];
    for (attrs, scheme, scheme_set, reverse, reverse_set) in cases {
        let scheme_row = row_of(attrs, SCHEME_ROW);
        assert_eq!(
            (scheme_row.value.as_str(), scheme_row.set),
            (scheme, scheme_set),
            "scheme of `{attrs}`"
        );
        let reverse_row = row_of(attrs, REVERSE_ROW);
        assert_eq!(
            (reverse_row.value.as_str(), reverse_row.set),
            (reverse, reverse_set),
            "reverse of `{attrs}`"
        );
        assert!(
            scheme_row.reason.is_none() && reverse_row.reason.is_none(),
            "`{attrs}`: a ramp drawn under dots mutes neither row"
        );
    }
}

/// **AC1.** A row at brightfield's own value is drawn with the word *auto* and a
/// hollow ring, one that is set with a filled dot and no *auto*, as an axis's
/// rows are.
#[test]
fn a_row_at_its_own_value_is_drawn_auto_and_a_row_set_is_drawn_set() {
    let stage = Stage::new(Mode::Light);
    let mut auto = turned("");
    let frame = stage.draw(&mut auto);
    for name in [SCHEME_ROW, REVERSE_ROW] {
        assert!(
            drawn_row(&frame, name).auto_rect.is_some(),
            "{name} at its own value says auto"
        );
    }
    assert_eq!(
        texts_in(&frame, drawn_row(&frame, SCHEME_ROW).value_rect),
        ["viridis"],
        "auto reads viridis"
    );
    let mut set = turned("colorScheme: blues\ncolorReverse: true");
    let frame = stage.draw(&mut set);
    for name in [SCHEME_ROW, REVERSE_ROW] {
        assert!(
            drawn_row(&frame, name).auto_rect.is_none(),
            "{name} set does not say auto"
        );
    }
    assert_eq!(
        texts_in(&frame, drawn_row(&frame, SCHEME_ROW).value_rect),
        ["blues"]
    );
    assert_eq!(
        texts_in(&frame, drawn_row(&frame, REVERSE_ROW).value_rect),
        [ON]
    );
}

/// **AC1.** The foot prints each row's sentence under the cursor, and a word of
/// `auto` in it names the rule for the default.
#[test]
fn the_foot_prints_the_sentence_of_the_row_under_the_cursor() {
    let stage = Stage::new(Mode::Light);
    let mut list = turned("");
    let frame = stage.draw(&mut list);
    let scheme = row_of("", SCHEME_ROW);
    assert_eq!(sentence(&frame), foot_sentence(&scheme));
    assert!(
        scheme.says.contains("auto") && scheme.says.contains("viridis"),
        "the scheme's sentence names the default: {:?}",
        scheme.says
    );

    list.feed_events(&j());
    let frame = stage.draw(&mut list);
    let scale = row_of("", SCALE_ROW);
    assert_eq!(sentence(&frame), foot_sentence(&scale));
    assert!(scale.says.contains("Auto"), "{:?}", scale.says);

    list.feed_events(&j());
    let frame = stage.draw(&mut list);
    let legend = row_of("", LEGEND_ROW);
    assert_eq!(sentence(&frame), foot_sentence(&legend));
    assert!(legend.says.contains("Auto"), "{:?}", legend.says);

    list.feed_events(&j());
    let frame = stage.draw(&mut list);
    let reverse = row_of("", REVERSE_ROW);
    assert_eq!(sentence(&frame), foot_sentence(&reverse));
    assert_ne!(reverse.says, scheme.says, "each row says its own");
    assert!(reverse.says.contains("auto"), "{:?}", reverse.says);
}

// ---------------------------------------------------------------------------
// The scheme row's ramp and strip.
// ---------------------------------------------------------------------------

/// **AC2.** The strip offers the renderer's schemes but the diverging one, in
/// the order the renderer visits them: viridis, blues, turbo and meridian.
#[test]
fn the_strip_is_the_renderers_schemes_but_the_diverging_one() {
    let offered = scheme_strip();
    assert_eq!(
        offered.iter().map(|s| s.wire_name()).collect::<Vec<_>>(),
        OFFERED,
        "the four, in order"
    );
    let all_but_rdbu: Vec<_> = SequentialScheme::ALL
        .into_iter()
        .filter(|s| s.wire_name() != "rdbu")
        .collect();
    assert_eq!(
        offered, all_but_rdbu,
        "the strip is the renderer's list, not a second one"
    );
}

/// **AC2.** The scheme row paints a 28 by 10 ramp of its value before the name,
/// strip by strip along the scheme's stops.
#[test]
fn the_scheme_row_paints_a_28_by_10_ramp_of_its_value_before_the_name() {
    let stage = Stage::new(Mode::Light);
    for (attrs, scheme) in [
        ("", SequentialScheme::Viridis),
        ("colorScheme: blues", SequentialScheme::Blues),
        ("colorScheme: turbo", SequentialScheme::Turbo),
        ("colorScheme: meridian", SequentialScheme::Meridian),
        ("colorScheme: rdbu", SequentialScheme::Rdbu),
    ] {
        let mut list = turned(attrs);
        let frame = stage.draw(&mut list);
        let row = drawn_row(&frame, SCHEME_ROW);
        let ramp = row.ramp.unwrap_or_else(|| panic!("`{attrs}`: no ramp"));
        assert_eq!(
            (ramp.width(), ramp.height()),
            (28.0, 10.0),
            "`{attrs}`: the ramp is 28 by 10"
        );
        assert!(
            ramp.right() <= row.value_rect.left(),
            "`{attrs}`: the ramp stands before the name: {ramp:?} {:?}",
            row.value_rect
        );
        assert!(
            (ramp.center().y - row.value_rect.center().y).abs() < 1.0,
            "`{attrs}`: the ramp and the name share a line"
        );
        assert_eq!(
            ramp_ink(&frame, ramp),
            expected_ramp(scheme),
            "`{attrs}`: the ramp is {scheme:?}'s stops"
        );
        assert_eq!(texts_in(&frame, row.value_rect), [scheme.wire_name()]);
    }
    let mut list = turned("");
    let frame = stage.draw(&mut list);
    assert!(
        drawn_row(&frame, REVERSE_ROW).ramp.is_none(),
        "the reverse row paints no ramp"
    );
}

/// **AC2.** Under the cursor a second line shows viridis, blues, turbo and
/// meridian, each a ramp and its name, the current one ringed in colour's hue.
#[test]
fn under_the_cursor_a_second_line_offers_four_ramps_and_rings_the_current_one() {
    for mode in [Mode::Light, Mode::Dark] {
        let stage = Stage::new(mode);
        for (attrs, current) in [
            ("", SequentialScheme::Viridis),
            ("colorScheme: blues", SequentialScheme::Blues),
            ("colorScheme: turbo", SequentialScheme::Turbo),
            ("colorScheme: meridian", SequentialScheme::Meridian),
        ] {
            let mut list = turned(attrs);
            let frame = stage.draw(&mut list);
            let row = drawn_row(&frame, SCHEME_ROW);
            assert_eq!(
                row.schemes.iter().map(|s| s.word).collect::<Vec<_>>(),
                OFFERED,
                "{mode:?} `{attrs}`: the strip's four, in order"
            );
            for item in &row.schemes {
                assert_eq!(
                    (item.ramp.width(), item.ramp.height()),
                    (28.0, 10.0),
                    "{:?}: a ramp",
                    item.scheme
                );
                assert_eq!(
                    ramp_ink(&frame, item.ramp),
                    expected_ramp(item.scheme.expect("a scheme")),
                    "{mode:?}: {:?}'s ramp is its own stops",
                    item.scheme
                );
                assert_eq!(
                    texts_in(&frame, item.name),
                    [item.word],
                    "{mode:?}: the name beside the ramp"
                );
                assert!(
                    item.ramp.right() <= item.name.left(),
                    "{mode:?}: the name follows its ramp"
                );
                assert!(
                    item.ramp.top() >= row.value_rect.bottom(),
                    "{mode:?}: the strip stands under the line the value is on"
                );
                assert!(
                    row.rect.contains_rect(item.ramp.union(item.name)),
                    "{mode:?}: the strip is part of the row"
                );
            }
            let ringed: Vec<_> = row
                .schemes
                .iter()
                .filter(|s| s.ring.is_some())
                .filter_map(|s| s.scheme)
                .collect();
            assert_eq!(
                ringed,
                [current],
                "{mode:?} `{attrs}`: only the current is ringed"
            );
            let ring = drawn_scheme(row, current).ring.expect("ringed");
            let strokes: Vec<_> = leaves(&frame)
                .into_iter()
                .filter_map(|s| match s {
                    Shape::Rect(r) if r.stroke.width > 0.0 && near(r.rect, ring) => {
                        Some(r.stroke.color)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                strokes,
                [hue(mode)],
                "{mode:?} `{attrs}`: the ring is stroked once, in colour's hue"
            );
            assert!(
                ring.contains_rect(drawn_scheme(row, current).ramp),
                "{mode:?}: the ring goes round the ramp"
            );
        }
    }
}

/// **AC2.** A scheme off the strip (rdbu, which a file may name) rings none, and
/// the strip is only there while the cursor is on the scheme row.
#[test]
fn a_scheme_off_the_strip_rings_none_and_the_strip_goes_with_the_cursor() {
    let stage = Stage::new(Mode::Light);
    let mut list = turned("colorScheme: rdbu");
    let frame = stage.draw(&mut list);
    let row = drawn_row(&frame, SCHEME_ROW);
    assert_eq!(row.schemes.len(), 4);
    assert!(
        row.schemes.iter().all(|s| s.ring.is_none()),
        "rdbu is not among the four, so none is ringed"
    );

    let mut list = turned("colorScheme: blues");
    let on = stage.draw(&mut list);
    let tall = drawn_row(&on, SCHEME_ROW).rect.height();
    list.feed_events(&j());
    list.feed_events(&j());
    list.feed_events(&j());
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(REVERSE_ROW));
    let off = stage.draw(&mut list);
    let scheme = drawn_row(&off, SCHEME_ROW);
    assert!(scheme.schemes.is_empty(), "no strip off the row");
    assert!(
        scheme.ramp.is_some(),
        "the row keeps its ramp when the cursor leaves"
    );
    assert!(
        scheme.rect.height() < tall,
        "the row is one line high again: {} against {tall}",
        scheme.rect.height()
    );
    assert_eq!(
        scheme.rect.height(),
        drawn_row(&off, REVERSE_ROW).rect.height(),
        "and as high as a row of one line"
    );
}

/// **AC2.** A narrow rail wraps the strip onto more lines: every ramp and name
/// stays inside the list, no two collide, and the narrower rail takes more lines
/// than the default one.
#[test]
fn a_narrow_rail_wraps_the_strip_onto_more_lines_and_nothing_leaves_the_list() {
    let lines = |frame: &Frame| {
        let mut tops: Vec<i32> = drawn_row(frame, SCHEME_ROW)
            .schemes
            .iter()
            .map(|s| s.ramp.top().round() as i32)
            .collect();
        tops.dedup();
        tops.len()
    };
    let wide = Stage::new(Mode::Light);
    let narrow = Stage::at(Mode::Light, NARROWEST);
    let mut list = turned("colorScheme: turbo");
    let at_default = wide.draw(&mut list);
    let mut list = turned("colorScheme: turbo");
    let at_least = narrow.draw(&mut list);
    assert!(
        lines(&at_least) > lines(&at_default) && lines(&at_least) >= 2,
        "{} lines at {NARROWEST}, {} at {WIDTH}",
        lines(&at_least),
        lines(&at_default)
    );
    for (frame, name) in [(&at_default, "default"), (&at_least, "narrowest")] {
        let row = drawn_row(frame, SCHEME_ROW);
        for item in &row.schemes {
            let whole = item.ramp.union(item.name);
            assert!(
                frame.drawn.rect.contains_rect(whole),
                "{name}: {:?} stays inside the list {:?}: {whole:?}",
                item.scheme,
                frame.drawn.rect
            );
        }
        for (i, a) in row.schemes.iter().enumerate() {
            for b in &row.schemes[i + 1..] {
                assert!(
                    !a.ramp.union(a.name).intersects(b.ramp.union(b.name)),
                    "{name}: {:?} and {:?} overlap",
                    a.scheme,
                    b.scheme
                );
            }
        }
        assert_eq!(frame.collisions, None, "{name}: no two texts collide");
    }
    let mut list = turned("colorScheme: turbo");
    let frame = narrow.draw(&mut list);
    assert_eq!(
        drawn_row(&frame, SCHEME_ROW).schemes.len(),
        4,
        "all four are offered at the narrowest"
    );
}

// ---------------------------------------------------------------------------
// The keys and the pointer.
// ---------------------------------------------------------------------------

/// One case of the scheme row's steps: the plot's attributes, the key pressed,
/// and the scheme the step writes, where it writes one.
type StepCase = (&'static str, fn() -> Vec<egui::Event>, Option<&'static str>);

/// **AC2.** `h` and `l` step the scheme along the strip and stop at its ends.
/// A step back to viridis writes viridis by name, not `auto`.
#[test]
fn h_and_l_step_the_scheme_along_the_strip_and_stop_at_its_ends() {
    let cases: [StepCase; 12] = [
        ("", l, Some("blues")),
        ("", h, None),
        ("colorScheme: viridis", l, Some("blues")),
        ("colorScheme: blues", l, Some("turbo")),
        ("colorScheme: blues", h, Some("viridis")),
        ("colorScheme: turbo", l, Some("meridian")),
        ("colorScheme: turbo", h, Some("blues")),
        ("colorScheme: meridian", l, None),
        ("colorScheme: meridian", h, Some("turbo")),
        // rdbu stands past the end of the order the renderer visits.
        ("colorScheme: rdbu", h, Some("meridian")),
        ("colorScheme: rdbu", l, None),
        ("colorScheme: magma", l, Some("blues")),
    ];
    for (attrs, key, writes) in cases {
        let mut list = turned(attrs);
        let reports = list.feed_events(&key());
        let expected: Vec<ListReport> = writes
            .map(|name| set(SCHEME_ROW, word(name)))
            .into_iter()
            .collect();
        assert_eq!(reports, expected, "`{attrs}`");
    }
}

/// **AC2.** `⌫` on the scheme row reports a step back to auto, which the window
/// answers with viridis or no edit.
#[test]
fn backspace_on_the_scheme_row_reports_auto() {
    for attrs in ["", "colorScheme: blues"] {
        let mut list = turned(attrs);
        assert_eq!(
            list.feed_events(&backspace()),
            [set(SCHEME_ROW, SettingValue::Auto)],
            "`{attrs}`"
        );
    }
}

/// **AC2.** A click on a ramp, or on its name, writes that scheme; the ringed one
/// included, whose write the window leaves as no edit.
#[test]
fn a_click_on_a_ramp_or_its_name_writes_that_scheme() {
    let stage = Stage::new(Mode::Light);
    for current in ["", "colorScheme: meridian"] {
        for target in [
            SequentialScheme::Viridis,
            SequentialScheme::Blues,
            SequentialScheme::Turbo,
            SequentialScheme::Meridian,
        ] {
            for on_the in ["ramp", "name"] {
                let mut list = turned(current);
                let frame = stage.draw(&mut list);
                let item = drawn_scheme(drawn_row(&frame, SCHEME_ROW), target);
                let at = if on_the == "ramp" {
                    item.ramp.center()
                } else {
                    item.name.center()
                };
                let (_, reports) = stage.click(&mut list, at);
                assert_eq!(
                    reports,
                    [set(SCHEME_ROW, word(target.wire_name()))],
                    "a click on {target:?}'s {on_the} with `{current}`"
                );
                assert_eq!(
                    list.setting_cursor().map(|r| r.name),
                    Some(SCHEME_ROW),
                    "the cursor stays on the row"
                );
            }
        }
    }
}

/// **AC2.** A click on the scheme row's value, off the strip, steps it as `l`
/// does, as a click on an axis's value does.
#[test]
fn a_click_on_the_scheme_rows_value_steps_it_forward() {
    let stage = Stage::new(Mode::Light);
    let mut list = turned("colorScheme: blues");
    let frame = stage.draw(&mut list);
    let at = drawn_row(&frame, SCHEME_ROW).value_rect.center();
    let (_, reports) = stage.click(&mut list, at);
    assert_eq!(reports, [set(SCHEME_ROW, word("turbo"))]);
}

/// **AC3.** The reverse row switches by `h`, `l` and a click, and `⌫` puts it
/// back to auto.
#[test]
fn the_reverse_row_switches_by_h_l_and_a_click() {
    let stage = Stage::new(Mode::Light);
    for (attrs, key) in [
        ("", "l"),
        ("", "h"),
        ("colorReverse: true", "l"),
        ("colorReverse: true", "h"),
    ] {
        let mut list = turned(attrs);
        list.feed_events(&j());
        list.feed_events(&j());
        list.feed_events(&j());
        assert_eq!(list.setting_cursor().map(|r| r.name), Some(REVERSE_ROW));
        let on = attrs.contains("true");
        let reports = list.feed_events(&if key == "l" { l() } else { h() });
        assert_eq!(
            reports,
            [set(REVERSE_ROW, SettingValue::Switch(!on))],
            "`{attrs}` {key}: a switch turns over"
        );
    }
    for (attrs, on) in [("", false), ("colorReverse: true", true)] {
        let mut list = turned(attrs);
        list.feed_events(&j());
        list.feed_events(&j());
        list.feed_events(&j());
        let frame = stage.draw(&mut list);
        let at = drawn_row(&frame, REVERSE_ROW).value_zone.center();
        let (_, reports) = stage.click(&mut list, at);
        assert_eq!(
            reports,
            [set(REVERSE_ROW, SettingValue::Switch(!on))],
            "a click on the value of `{attrs}`"
        );
        assert_eq!(
            list.feed_events(&backspace()),
            [set(REVERSE_ROW, SettingValue::Auto)]
        );
    }
}

/// The reverse row draws no strip and no ramp, and the key table of colour holds
/// the two plot attributes the rows write.
#[test]
fn the_rows_write_the_plot_attributes_mosaic_spells() {
    use brightfield_shell::shelf::{row_default, row_key};
    assert_eq!(
        row_key(ShelfChannel::Colour, SCHEME_ROW),
        Some("colorScheme")
    );
    assert_eq!(
        row_key(ShelfChannel::Colour, REVERSE_ROW),
        Some("colorReverse")
    );
    assert_eq!(row_key(ShelfChannel::Colour, "scale"), None, "no scale row");
    assert_eq!(
        row_key(ShelfChannel::Colour, "legend"),
        None,
        "no legend row"
    );
    assert_eq!(row_key(ShelfChannel::Mark, SCHEME_ROW), None);
    assert_eq!(row_key(ShelfChannel::X, SCHEME_ROW), None);
    assert_eq!(
        row_default(REVERSE_ROW),
        Some(brightfield_spec::ast::SpecValue::Bool(false)),
        "reverse's own is off"
    );
}

// ---------------------------------------------------------------------------
// Where a row does not apply.
// ---------------------------------------------------------------------------

/// **AC4.** Over a colour that paints names, the scheme row is drawn muted with
/// its reason and does nothing, and reverse acts.
#[test]
fn on_a_colour_of_names_the_scheme_row_is_muted_with_its_reason_and_reverse_acts() {
    let stage = Stage::new(Mode::Light);
    let settings = settings_over("colorScheme: blues", &names_scales(), None);
    let scheme = settings.rows(ShelfChannel::Colour)[0].clone();
    let reverse = settings
        .rows(ShelfChannel::Colour)
        .iter()
        .find(|r| r.name == REVERSE_ROW)
        .expect("colour has a reverse row")
        .clone();
    let reason = scheme.reason.clone().expect("the scheme row has a reason");
    assert!(
        reason.contains("names") && reason.contains("categorical"),
        "the reason says why: {reason:?}"
    );
    assert_eq!(reverse.reason, None, "reverse applies to names");
    assert!(!scheme.steps() && reverse.steps());

    let mut list = turned_over(settings.clone());
    let frame = stage.draw(&mut list);
    let row = drawn_row(&frame, SCHEME_ROW);
    let said = row.reason_rect.expect("the reason is drawn under the row");
    assert_eq!(texts_in(&frame, said), [reason.as_str()]);
    assert!(
        row.ramp.is_none(),
        "a ramp that is not in force is not painted"
    );
    assert!(row.schemes.is_empty(), "and no strip is offered");
    let muted = chrome::colour(semantic(false).text.muted);
    assert_eq!(ink_at(&frame, row.name_rect), muted, "the name is muted");
    assert_eq!(
        ink_at(&frame, row.value_rect),
        muted,
        "the value is muted, set or not"
    );
    assert_eq!(sentence(&frame), foot_sentence(&scheme));
    assert!(
        sentence(&frame).starts_with("A column of names"),
        "{:?}",
        sentence(&frame)
    );
    let primary = chrome::colour(semantic(false).text.primary);
    let reverse_row = drawn_row(&frame, REVERSE_ROW);
    assert_eq!(
        ink_at(&frame, reverse_row.name_rect),
        primary,
        "reverse is not muted"
    );

    // Nothing steps, resets or picks on the muted row.
    for events in [l(), h(), backspace()] {
        assert!(
            list.feed_events(&events).is_empty(),
            "a muted row reports nothing"
        );
    }
    let at = row.value_zone.center();
    let (_, reports) = stage.click(&mut list, at);
    assert!(
        reports.is_empty(),
        "a click on a muted row's value writes nothing"
    );

    // Reverse acts.
    list.feed_events(&j());
    list.feed_events(&j());
    list.feed_events(&j());
    assert_eq!(
        list.feed_events(&l()),
        [set(REVERSE_ROW, SettingValue::Switch(true))]
    );

    // Control: the same plot over a ramp mutes nothing.
    let ramp = settings_over("colorScheme: blues", &ramp_scales(), None);
    assert_eq!(ramp.rows(ShelfChannel::Colour)[0].reason, None);
}

/// A colour no column is bound to draws no scale; its rows are not muted for it.
#[test]
fn a_colour_that_draws_no_scale_mutes_neither_row() {
    let settings = settings_over("", &ScaleSet::new(), Some(&[MarkKind::Dot]));
    // The pivot and steps rows are muted by the scale the plot names, not by
    // the want of one drawn.
    for row in settings
        .rows(ShelfChannel::Colour)
        .iter()
        .filter(|r| !r.by_name)
    {
        assert_eq!(
            row.reason, None,
            "{} is not muted for want of a scale",
            row.name
        );
    }
}

/// **AC3.** Reverse is muted where the render crate's judge says no dot is among
/// the marks, and applies where one is or where none has been handed in.
#[test]
fn reverse_is_muted_only_where_the_judge_says_no_dot_is_drawn() {
    let reverse = |marks: Option<&[MarkKind]>| {
        settings_over("colorReverse: true", &ramp_scales(), marks)
            .rows(ShelfChannel::Colour)
            .iter()
            .find(|r| r.name == REVERSE_ROW)
            .expect("colour has a reverse row")
            .clone()
    };
    assert_eq!(reverse(Some(&[MarkKind::Dot])).reason, None);
    assert_eq!(reverse(Some(&[MarkKind::Cell, MarkKind::Dot])).reason, None);
    assert_eq!(reverse(None).reason, None, "no judge speaks without marks");
    let muted = reverse(Some(&[MarkKind::Cell]));
    let reason = muted.reason.clone().expect("a chart of cells has a reason");
    assert!(reason.contains("dot"), "{reason:?}");
    assert!(muted.set, "the row is still the analyst's");
    assert!(!muted.steps());
    let stage = Stage::new(Mode::Light);
    let mut list = turned_over(settings_over(
        "colorReverse: true",
        &ramp_scales(),
        Some(&[MarkKind::Cell]),
    ));
    list.feed_events(&j());
    list.feed_events(&j());
    list.feed_events(&j());
    for events in [l(), h(), backspace()] {
        assert!(list.feed_events(&events).is_empty());
    }
    let frame = stage.draw(&mut list);
    assert_eq!(
        texts_in(
            &frame,
            drawn_row(&frame, REVERSE_ROW)
                .reason_rect
                .expect("the reason is drawn")
        ),
        [reason.as_str()]
    );
}

// ---------------------------------------------------------------------------
// The baselines.
// ---------------------------------------------------------------------------

fn options() -> SnapshotOptions {
    SnapshotOptions::default()
}

/// Colour's list over a plot that sets `colorScheme: blues`, the cursor on the
/// scheme row, drawn through the wgpu renderer and compared with the committed
/// baseline `name`.
fn baseline(name: &str, mode: Mode) {
    let list = turned("colorScheme: blues");
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(SCHEME_ROW));
    baseline_of(list, name, mode);
}

/// `list` drawn through the wgpu renderer and compared with the committed
/// baseline `name`.
fn baseline_of(mut list: ColumnList, name: &str, mode: Mode) {
    let size = egui::vec2(WIDTH, 420.0);
    let mut harness = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(2.0)
        .wgpu()
        .build_ui(move |ui| {
            design::apply(ui.ctx(), mode);
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                |ui| {
                    list.show(ui, mode);
                },
            );
        });
    harness.run();
    harness.snapshot_options(name, &options());
}

/// The scheme row with the cursor on it: the ramp of blues before the name, and
/// under the row the strip of four ramps with blues ringed in colour's hue.
#[test]
fn colours_scheme_row_with_its_strip_light_matches_its_baseline() {
    baseline("shelf_settings_colour_scheme_light", Mode::Light);
}

#[test]
fn colours_scheme_row_with_its_strip_dark_matches_its_baseline() {
    baseline("shelf_settings_colour_scheme_dark", Mode::Dark);
}

/// Moments of a column of 0.5 to 15 whose median is 3.47: no zero crossing,
/// so the auto pivot is the median.
fn income_moments() -> ColumnMoments {
    ColumnMoments {
        mean: 3.87,
        median: 3.47,
        sd: Some(1.9),
        distinct: 12_000,
        min: 0.5,
        max: 15.0,
        distribution: Distribution::Bins(vec![0; 480]),
    }
}

fn colour_row(attrs: &str, name: &str, drawn: &ScaleSet) -> SettingRow {
    let source = format!(
        "data:\n  t:\n    - {{ a: 1 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: a\n    y: a\nwidth: 600\nheight: 300\n{attrs}\n"
    );
    let spec = parse_spec(&source, Format::Yaml)
        .expect("the spec parses")
        .spec;
    let plot = plot_at_path(&spec, "root").expect("the root is its plot");
    let moments = income_moments();
    ChannelSettings::of_plot_profiled(
        &spec,
        plot,
        "root",
        &channels(),
        drawn,
        &[MarkKind::Dot],
        Some(&moments),
    )
    .rows(ShelfChannel::Colour)
    .iter()
    .find(|r| r.name == name)
    .unwrap_or_else(|| panic!("colour has no {name} row"))
    .clone()
}

/// **The legend row reads where the file puts the legend, steps through right,
/// below and none, and carries the dot for the two that are not auto.**
#[test]
fn the_legend_row_steps_right_below_none_and_dots_the_two_that_are_not_auto() {
    let none = row_of("", LEGEND_ROW);
    assert_eq!(
        none.value, "none",
        "a plot with no legend node has no legend"
    );
    assert!(none.set, "none carries the dot");
    assert!(none.steps() && !none.by_name);
    assert_eq!(none.stepped(1), None, "none is the last step");
    assert_eq!(
        none.stepped(-1),
        Some(SettingValue::Word("below".to_string()))
    );
    let below = SettingRow {
        value: "below".to_string(),
        ..none.clone()
    };
    assert_eq!(
        below.stepped(1),
        Some(SettingValue::Word("none".to_string()))
    );
    assert_eq!(
        below.stepped(-1),
        Some(SettingValue::Word("right".to_string()))
    );
    let right = SettingRow {
        value: "right".to_string(),
        set: false,
        ..none
    };
    assert_eq!(right.stepped(-1), None, "right is the first step");
    assert_eq!(
        right.stepped(1),
        Some(SettingValue::Word("below".to_string()))
    );
}

#[test]
fn colours_head_rows_are_scheme_scale_legend_and_reverse() {
    let settings = settings_over("", &ramp_scales(), Some(&[MarkKind::Dot]));
    let head: Vec<&str> = settings
        .rows(ShelfChannel::Colour)
        .iter()
        .filter(|r| !r.by_name)
        .map(|r| r.name)
        .collect();
    assert_eq!(head, ["scheme", "scale", "legend", "reverse"]);
}

#[test]
fn the_scale_row_reads_its_value_and_carries_its_parameter() {
    let linear = colour_row("", SCALE_ROW, &ramp_scales());
    assert_eq!((linear.value.as_str(), linear.set), ("linear", false));

    let auto = colour_row(
        "colorScale: diverging\ncolorPivot: 3.47",
        SCALE_ROW,
        &ramp_scales(),
    );
    assert_eq!(auto.value, "diverging");
    assert_eq!(auto.colour.as_ref().and_then(|c| c.pivot), Some(3.47));
    assert!(auto.set, "a scale off linear is set");
    assert!(
        !auto.parameter_set(),
        "the pivot equal to the auto pivot reads auto"
    );

    let typed = colour_row(
        "colorScale: diverging\ncolorPivot: 2",
        SCALE_ROW,
        &ramp_scales(),
    );
    assert!(typed.parameter_set(), "a pivot off the auto pivot is set");

    let steps = colour_row("colorScale: quantize\ncolorN: 5", SCALE_ROW, &ramp_scales());
    assert_eq!(steps.value, "steps");
    assert_eq!(steps.colour.as_ref().and_then(|c| c.count), Some(5));
    assert!(!steps.parameter_set());
}

/// A file written by hand with diverging and no pivot reads the pivot the
/// chart drew, muted: nothing in the file is the analyst's beyond the scale.
#[test]
fn a_diverging_scale_with_no_pivot_written_reads_the_drawn_pivot_as_auto() {
    let mut drawn = ScaleSet::new();
    drawn.insert(
        Channel::Fill,
        Scale::Diverging {
            domain_min: 0.0,
            domain_max: 7.0,
            pivot: 3.5,
            stops: vec![[0.0; 4]; 3],
        },
    );
    // The profile says 3.47 and the chart drew 3.5: a file with no pivot of its
    // own is the analyst's for the scale alone.
    let row = colour_row("colorScale: diverging", SCALE_ROW, &drawn);
    assert_eq!(row.colour.as_ref().and_then(|c| c.pivot), Some(3.5));
    assert!(!row.parameter_set());
}

#[test]
fn the_scale_row_is_muted_on_a_colour_of_names_and_does_not_step() {
    let row = colour_row("", SCALE_ROW, &names_scales());
    assert!(row.reason.is_some());
    assert_eq!(row.stepped(1), None);
}

#[test]
fn stepping_to_diverging_carries_the_auto_pivot() {
    let row = colour_row("", SCALE_ROW, &ramp_scales());
    assert_eq!(
        row.stepped(1),
        Some(SettingValue::ColourScale(ColourScaleWrite::Diverging {
            pivot: Number::new(3.47),
        })),
    );
    let diverging = colour_row(
        "colorScale: diverging\ncolorPivot: 3.47",
        SCALE_ROW,
        &ramp_scales(),
    );
    assert_eq!(
        diverging.stepped(1),
        Some(SettingValue::ColourScale(ColourScaleWrite::Steps {
            count: 5
        })),
    );
    assert_eq!(
        diverging.stepped(-1),
        Some(SettingValue::ColourScale(ColourScaleWrite::Linear)),
    );
}

#[test]
fn pivot_and_steps_are_found_by_name_and_muted_off_their_scale() {
    let pivot = colour_row("", PIVOT_ROW, &ramp_scales());
    assert!(pivot.by_name);
    assert_eq!(
        pivot.reason.as_deref(),
        Some("pivot applies to a diverging scale")
    );
    let steps = colour_row("", STEPS_ROW, &ramp_scales());
    assert_eq!(
        steps.reason.as_deref(),
        Some("steps applies to a stepped scale")
    );
    let on = colour_row("colorScale: quantize\ncolorN: 7", STEPS_ROW, &ramp_scales());
    assert_eq!(
        (on.value.as_str(), on.set, on.reason.clone()),
        ("7", true, None)
    );
}

// ---------------------------------------------------------------------------
// The scale row's parameter.
// ---------------------------------------------------------------------------

fn enter() -> Vec<egui::Event> {
    vec![key_event(egui::Key::Enter)]
}

fn text(s: &str) -> Vec<egui::Event> {
    vec![egui::Event::Text(s.to_string())]
}

/// Colour's list over a plot with `attrs`, turned to its settings with the
/// cursor on the scale row, the profile of the column handed in.
fn on_scale_row(attrs: &str) -> ColumnList {
    let mut list = turned_over(profiled_settings(attrs, &ramp_scales()));
    list.feed_events(&j());
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(SCALE_ROW));
    list
}

fn profiled_settings(attrs: &str, drawn: &ScaleSet) -> ChannelSettings {
    let source = format!(
        "data:\n  t:\n    - {{ a: 1 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: a\n    y: a\nwidth: 600\nheight: 300\n{attrs}\n"
    );
    let spec = parse_spec(&source, Format::Yaml)
        .expect("the spec parses")
        .spec;
    let plot = plot_at_path(&spec, "root").expect("the root is its plot");
    ChannelSettings::of_plot_profiled(
        &spec,
        plot,
        "root",
        &channels(),
        drawn,
        &[MarkKind::Dot],
        Some(&income_moments()),
    )
}

#[test]
fn enter_on_diverging_opens_the_pivot_and_keeps_a_typed_one() {
    let mut list = on_scale_row("colorScale: diverging\ncolorPivot: 3.47");
    let reports = list.feed_events(&enter());
    assert!(reports.is_empty());
    assert_eq!(list.field().map(|f| f.text.as_str()), Some("3.47"));
    list.feed_events(&text("2"));
    let reports = list.feed_events(&enter());
    assert!(reports.contains(&set(
        PIVOT_ROW,
        SettingValue::Number(Number::new(2.0).unwrap())
    )));
}

#[test]
fn enter_on_steps_opens_the_count_and_keeps_a_typed_one() {
    let mut list = on_scale_row("colorScale: quantize\ncolorN: 5");
    list.feed_events(&enter());
    assert_eq!(list.field().map(|f| f.text.as_str()), Some("5"));
    list.feed_events(&text("7"));
    let reports = list.feed_events(&enter());
    assert!(reports.contains(&set(STEPS_ROW, SettingValue::Count(7))));
}

#[test]
fn a_pivot_or_count_the_row_cannot_take_is_refused_and_the_field_stays() {
    for (attrs, typed) in [
        ("colorScale: diverging\ncolorPivot: 1", "abc"),
        ("colorScale: diverging\ncolorPivot: 1", "1e400"),
        ("colorScale: quantize\ncolorN: 5", "1"),
        ("colorScale: quantize\ncolorN: 5", "0"),
        ("colorScale: quantize\ncolorN: 5", "300"),
    ] {
        let mut list = on_scale_row(attrs);
        list.feed_events(&enter());
        list.feed_events(&text(typed));
        let reports = list.feed_events(&enter());
        assert!(
            !reports.iter().any(|r| matches!(r, ListReport::Set(_))),
            "{typed} on {attrs} writes nothing",
        );
        assert!(
            list.field().and_then(|f| f.refusal.clone()).is_some(),
            "{typed} is refused in words"
        );
    }
}

#[test]
fn enter_on_linear_opens_no_field() {
    let mut list = on_scale_row("");
    list.feed_events(&enter());
    assert!(list.field().is_none());
}

#[test]
fn a_pivot_differing_from_auto_beyond_the_displayed_precision_reads_as_not_set() {
    // The auto pivot is the median, 3.47; the row shows four figures.
    let beyond = colour_row(
        "colorScale: diverging\ncolorPivot: 3.470004",
        SCALE_ROW,
        &ramp_scales(),
    );
    assert!(!beyond.parameter_set(), "off by less than the row shows");
    let within = colour_row(
        "colorScale: diverging\ncolorPivot: 3.48",
        SCALE_ROW,
        &ramp_scales(),
    );
    assert!(within.parameter_set(), "off by a digit the row shows");
}

/// The ink the text `text` was painted in on the row, read off the frame.
fn ink_of_text(frame: &Frame, row: &SettingRowDrawn, text: &str) -> egui::Color32 {
    leaves(frame)
        .into_iter()
        .find_map(|s| match s {
            Shape::Text(t)
                if row.rect.contains(t.pos + t.galley.size() / 2.0)
                    && t.galley.job.text.contains(text) =>
            {
                t.galley.job.sections.first().map(|sec| sec.format.color)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no text containing {text:?} was drawn on the row"))
}

fn row_text(frame: &Frame, row: &SettingRowDrawn) -> String {
    let mut parts: Vec<_> = frame
        .texts
        .iter()
        .filter(|t| row.rect.contains(t.ink.center()) && t.ink.min.x > row.name_rect.max.x)
        .collect();
    parts.sort_by(|a, b| a.ink.min.x.total_cmp(&b.ink.min.x));
    parts
        .iter()
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Colour's list over a plot with `attrs`, turned to its settings with the
/// cursor left on the scheme row, so the scale row draws no chips.
fn off_scale_row(attrs: &str) -> ColumnList {
    turned_over(profiled_settings(attrs, &ramp_scales()))
}

#[test]
fn the_scale_row_draws_its_parameter_muted_while_auto() {
    let muted = chrome::colour(semantic(false).text.muted);
    let primary = chrome::colour(semantic(false).text.primary);
    let stage = Stage::new(Mode::Light);

    let mut list = off_scale_row("colorScale: diverging\ncolorPivot: 3.47");
    let frame = stage.draw(&mut list);
    let row = drawn_row(&frame, SCALE_ROW);
    assert_eq!(row_text(&frame, row), "diverging about 3.47");
    assert_eq!(ink_of_text(&frame, row, "3.47"), muted);
    assert_eq!(ink_of_text(&frame, row, "diverging"), primary);

    let mut list = off_scale_row("colorScale: diverging\ncolorPivot: 2");
    let frame = stage.draw(&mut list);
    let row = drawn_row(&frame, SCALE_ROW);
    assert_eq!(ink_of_text(&frame, row, "about 2"), primary);

    let mut list = off_scale_row("colorScale: quantize\ncolorN: 5");
    let frame = stage.draw(&mut list);
    let row = drawn_row(&frame, SCALE_ROW);
    assert_eq!(row_text(&frame, row), "steps \u{b7} 5");
    assert_eq!(ink_of_text(&frame, row, "\u{b7} 5"), muted);

    let mut list = off_scale_row("colorScale: quantize\ncolorN: 7");
    let frame = stage.draw(&mut list);
    assert_eq!(
        ink_of_text(&frame, drawn_row(&frame, SCALE_ROW), "\u{b7} 7"),
        primary
    );
}

/// Under the cursor the row wears its chips, and the pivot gives way to the bare
/// number to leave them room; it is still muted while auto.
#[test]
fn under_the_cursor_the_pivot_is_the_bare_number_beside_the_chips() {
    let muted = chrome::colour(semantic(false).text.muted);
    let stage = Stage::new(Mode::Light);
    let mut list = on_scale_row("colorScale: diverging\ncolorPivot: 3.47");
    let frame = stage.draw(&mut list);
    let row = drawn_row(&frame, SCALE_ROW);
    assert!(row.chips.is_some());
    assert_eq!(row_text(&frame, row), "diverging 3.47 \u{2190} \u{2192}");
    assert_eq!(ink_of_text(&frame, row, "3.47"), muted);
}

#[test]
fn the_open_field_takes_the_parameters_place_behind_the_word() {
    let stage = Stage::new(Mode::Light);
    let mut list = on_scale_row("colorScale: diverging\ncolorPivot: 3.47");
    list.feed_events(&enter());
    let frame = stage.draw(&mut list);
    let row = drawn_row(&frame, SCALE_ROW);
    let field = row.field.expect("the field is drawn on the scale row");
    assert!(field.left() > row.value_rect.right(), "after the word");
    let said = row_text(&frame, row);
    assert!(said.starts_with("diverging 3.47"), "{said}");
    assert!(!said.contains("about"), "the field replaces the parameter");
}

// ---------------------------------------------------------------------------
// The scheme row on a diverging scale.
// ---------------------------------------------------------------------------

/// brightfield's diverging pair as a file writes it: a YAML array of three
/// `#rrggbb` strings, each channel rounded to the byte.
fn pair_text() -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let items: Vec<String> = brightfield_render::scale::own_diverging_range()
        .iter()
        .map(|c| {
            format!(
                "\"#{:02x}{:02x}{:02x}\"",
                byte(c[0]),
                byte(c[1]),
                byte(c[2])
            )
        })
        .collect();
    format!("[{}]", items.join(", "))
}

#[test]
fn on_diverging_the_scheme_row_offers_the_pair_and_rdbu() {
    let pair = format!(
        "colorScale: diverging\ncolorPivot: 1\ncolorRange: {}",
        pair_text()
    );
    let row = colour_row(&pair, SCHEME_ROW, &ramp_scales());
    assert_eq!((row.value.as_str(), row.set), ("blue-red", false));
    assert_eq!(row.stepped(1), Some(word("rdbu")));
    let rdbu = colour_row(
        "colorScale: diverging\ncolorPivot: 1\ncolorScheme: rdbu",
        SCHEME_ROW,
        &ramp_scales(),
    );
    assert_eq!((rdbu.value.as_str(), rdbu.set), ("rdbu", true));
    assert_eq!(rdbu.stepped(-1), Some(SettingValue::Auto));
    assert_eq!(diverging_strip(), ["blue-red", "rdbu"]);
}

#[test]
fn off_diverging_the_scheme_row_offers_the_four_sequential_schemes_as_before() {
    let row = colour_row("colorScheme: viridis", SCHEME_ROW, &ramp_scales());
    assert_eq!(row.stepped(1), Some(word("blues")));
    assert_eq!(scheme_strip().len(), 4);
}

#[test]
fn on_diverging_the_strip_draws_the_pair_and_rdbu_and_rings_the_current_one() {
    let stage = Stage::new(Mode::Light);
    let pair = format!(
        "colorScale: diverging\ncolorPivot: 1\ncolorRange: {}",
        pair_text()
    );
    let mut list = off_scale_row(&pair);
    let frame = stage.draw(&mut list);
    let row = drawn_row(&frame, SCHEME_ROW);
    assert_eq!(
        row.schemes.iter().map(|s| s.word).collect::<Vec<_>>(),
        ["blue-red", "rdbu"]
    );
    let ringed: Vec<_> = row.schemes.iter().filter(|s| s.ring.is_some()).collect();
    assert_eq!(ringed.len(), 1);
    assert_eq!(ringed[0].word, "blue-red");
    let ink = brightfield_render::ink::ChartInk::for_mode(false);
    let stops = brightfield_render::scale::diverging_stops(None, &ink);
    let expected: Vec<_> = (0..28)
        .map(|i| quantised(ramp_at(&stops, f64::from(i) / 27.0)))
        .collect();
    assert_eq!(ramp_ink(&frame, row.schemes[0].ramp), expected);
    assert_eq!(
        ramp_ink(&frame, row.schemes[1].ramp),
        expected_ramp(SequentialScheme::Rdbu)
    );
}

// ---------------------------------------------------------------------------
// The baselines of the scale row and the diverging strip.
// ---------------------------------------------------------------------------

/// The scale row on a diverging scale with the cursor on it: the word, the pivot
/// muted beside it while it is the auto one, and the chips.
fn scale_diverging() -> ColumnList {
    on_scale_row("colorScale: diverging\ncolorPivot: 3.47")
}

/// The same with `Enter` pressed and a `2` typed: the field takes the pivot's
/// place behind the word.
fn scale_pivot_field() -> ColumnList {
    let mut list = on_scale_row("colorScale: diverging\ncolorPivot: 3.47");
    list.feed_events(&enter());
    list.feed_events(&text("2"));
    list
}

/// The scheme row of a diverging plot with the cursor on it: the pair and `rdbu`
/// under the row, the pair ringed.
fn diverging_strip_list() -> ColumnList {
    off_scale_row(&format!(
        "colorScale: diverging\ncolorPivot: 3.47\ncolorRange: {}",
        pair_text()
    ))
}

#[test]
fn colours_scale_row_diverging_light_matches_its_baseline() {
    baseline_of(
        scale_diverging(),
        "shelf_settings_colour_scale_diverging_light",
        Mode::Light,
    );
}

#[test]
fn colours_scale_row_diverging_dark_matches_its_baseline() {
    baseline_of(
        scale_diverging(),
        "shelf_settings_colour_scale_diverging_dark",
        Mode::Dark,
    );
}

#[test]
fn colours_scale_row_pivot_field_light_matches_its_baseline() {
    baseline_of(
        scale_pivot_field(),
        "shelf_settings_colour_scale_pivot_field_light",
        Mode::Light,
    );
}

#[test]
fn colours_scale_row_pivot_field_dark_matches_its_baseline() {
    baseline_of(
        scale_pivot_field(),
        "shelf_settings_colour_scale_pivot_field_dark",
        Mode::Dark,
    );
}

#[test]
fn colours_scheme_row_diverging_strip_light_matches_its_baseline() {
    baseline_of(
        diverging_strip_list(),
        "shelf_settings_colour_scheme_diverging_strip_light",
        Mode::Light,
    );
}

#[test]
fn colours_scheme_row_diverging_strip_dark_matches_its_baseline() {
    baseline_of(
        diverging_strip_list(),
        "shelf_settings_colour_scheme_diverging_strip_dark",
        Mode::Dark,
    );
}

// ---------------------------------------------------------------------------
// The pivot and steps rows found by name, and the diverging scheme's edges.
// ---------------------------------------------------------------------------

#[test]
fn a_pivot_off_auto_beyond_the_displayed_precision_reads_the_same_on_both_rows() {
    let attrs = "colorScale: diverging\ncolorPivot: 3.470004";
    let scale = colour_row(attrs, SCALE_ROW, &ramp_scales());
    let pivot = colour_row(attrs, PIVOT_ROW, &ramp_scales());
    assert_eq!(pivot.value, "3.47");
    assert!(!pivot.set && !scale.parameter_set(), "muted: it is auto");
    let stage = Stage::new(Mode::Light);
    let mut list = off_scale_row(attrs);
    let frame = stage.draw(&mut list);
    let row = drawn_row(&frame, SCALE_ROW);
    assert_eq!(row_text(&frame, row), "diverging about 3.47");
    assert_eq!(
        ink_of_text(&frame, row, "3.47"),
        chrome::colour(semantic(false).text.muted)
    );
}

/// A list turned to the settings with the cursor on the row found by name.
fn on_named_row(attrs: &str, name: &'static str) -> ColumnList {
    let mut list = turned_over(profiled_settings(attrs, &ramp_scales()));
    assert!(list.open_on_row(name));
    list
}

#[test]
fn typing_into_the_pivot_row_found_by_name_keeps_a_pivot_and_not_a_tick_count() {
    let mut list = on_named_row("colorScale: diverging\ncolorPivot: 3.47", PIVOT_ROW);
    list.feed_events(&enter());
    list.feed_events(&text("2.5"));
    let reports = list.feed_events(&enter());
    assert!(reports.contains(&set(
        PIVOT_ROW,
        SettingValue::Number(Number::new(2.5).unwrap())
    )));
}

#[test]
fn typing_into_the_steps_row_found_by_name_keeps_a_count_of_steps() {
    let mut list = on_named_row("colorScale: quantize\ncolorN: 5", STEPS_ROW);
    list.feed_events(&enter());
    list.feed_events(&text("9"));
    let reports = list.feed_events(&enter());
    assert!(reports.contains(&set(STEPS_ROW, SettingValue::Count(9))));
    let mut list = on_named_row("colorScale: quantize\ncolorN: 5", STEPS_ROW);
    list.feed_events(&enter());
    list.feed_events(&text("1"));
    let reports = list.feed_events(&enter());
    assert!(!reports.iter().any(|r| matches!(r, ListReport::Set(_))));
}

#[test]
fn a_scheme_off_the_diverging_strip_steps_to_rdbu_and_back_to_the_pair() {
    let row = colour_row(
        "colorScale: diverging\ncolorPivot: 1\ncolorScheme: blues",
        SCHEME_ROW,
        &ramp_scales(),
    );
    assert_eq!(row.stepped(1), Some(word("rdbu")));
    assert_eq!(row.stepped(-1), Some(SettingValue::Auto));
}

#[test]
fn resetting_a_diverging_scheme_leaves_an_analysts_colours_and_writes_the_pair_elsewhere() {
    use brightfield_shell::shelf_edit::put_diverging_scheme;
    use brightfield_spec::analysis::ComponentPath;
    let spec_of = |attrs: &str| {
        let source = format!(
            "data:\n  t:\n    - {{ a: 1 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: a\n    y: a\nwidth: 600\nheight: 300\n{attrs}\n"
        );
        parse_spec(&source, Format::Yaml).unwrap().spec
    };
    let root = ComponentPath("root".to_string());
    let keys = |spec: &brightfield_spec::ast::Spec| {
        let plot = plot_at_path(spec, "root").unwrap();
        (
            plot.attributes.contains_key("colorScheme"),
            plot.attributes.get("colorRange").cloned(),
        )
    };
    let mut own = spec_of("colorScale: diverging\ncolorScheme: rdbu\ncolorRange: [\"#111111\", \"#222222\", \"#333333\"]");
    let before = keys(&own).1;
    put_diverging_scheme(&mut own, &root, false).unwrap();
    assert_eq!(keys(&own), (false, before), "the analyst's colours stay");

    let mut none = spec_of("colorScale: diverging\ncolorScheme: rdbu");
    put_diverging_scheme(&mut none, &root, false).unwrap();
    let (named, range) = keys(&none);
    assert!(
        !named && range.is_some(),
        "the pair is written where none is"
    );
}

#[test]
fn the_domain_row_reads_auto_with_the_drawn_ends_fixed_or_two_ends() {
    let auto = colour_row("", DOMAIN_ROW, &ramp_scales());
    assert_eq!((auto.value.as_str(), auto.set), ("auto", false));
    assert_eq!(
        auto.range.as_ref().and_then(|r| r.drawn),
        Ends::new(0.0, 10.0)
    );
    let fixed = colour_row("colorDomain: Fixed", DOMAIN_ROW, &ramp_scales());
    assert_eq!((fixed.value.as_str(), fixed.set), ("fixed", true));
    let ends = colour_row("colorDomain: [0, 4]", DOMAIN_ROW, &ramp_scales());
    assert_eq!((ends.value.as_str(), ends.set), ("0 \u{2013} 4", true));
    assert!(ends.by_name);
}

#[test]
fn the_domain_row_steps_between_auto_and_fixed() {
    let auto = colour_row("", DOMAIN_ROW, &ramp_scales());
    assert_eq!(auto.stepped(1), Some(SettingValue::Word("fixed".into())));
    let fixed = colour_row("colorDomain: Fixed", DOMAIN_ROW, &ramp_scales());
    assert_eq!(fixed.stepped(-1), Some(SettingValue::Auto));
}

#[test]
fn the_domain_row_is_muted_on_a_colour_of_names() {
    assert!(colour_row("", DOMAIN_ROW, &names_scales()).reason.is_some());
}

/// The domain row found by name with the cursor on it, reading auto: the ends
/// the colours were drawn over follow it, muted.
fn domain_row_list() -> ColumnList {
    let mut list = turned_over(profiled_settings("", &ramp_scales()));
    list.feed_events(&typed(egui::Key::Slash, "/"));
    for (key, letter) in [
        (egui::Key::D, "d"),
        (egui::Key::O, "o"),
        (egui::Key::M, "m"),
    ] {
        list.feed_events(&typed(key, letter));
    }
    list.feed_events(&enter());
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(DOMAIN_ROW));
    list
}

#[test]
fn colours_domain_row_light_matches_its_baseline() {
    baseline_of(
        domain_row_list(),
        "shelf_settings_colour_domain_row_light",
        Mode::Light,
    );
}

#[test]
fn colours_domain_row_dark_matches_its_baseline() {
    baseline_of(
        domain_row_list(),
        "shelf_settings_colour_domain_row_dark",
        Mode::Dark,
    );
}
