//! **The shelf band names each channel of a chart's mark in its hue, with the
//! key that reaches it.**
//!
//! The band is drawn into a `Ui` a test gives it, so the claims are read off the
//! frame the band painted and off the state it answers a key or a click with.
//! Four kinds of reading, each at the altitude its claim lives at:
//!
//! - **Geometry and words** are read off the laid-out frame: the rectangles the
//!   band painted and the galleys it put in them (`text_ink::frame_text`), not
//!   off the numbers the band reports about itself.
//! - **Hue** is read off the painted fills, against colours this file computes
//!   from the design system's categorical palette and its tint strength, by
//!   slot number. They are not read back from the function that assigns a hue to
//!   a channel, which a wrong assignment would make agree with itself.
//! - **Keys** go in as `egui` key events through `ShelfBand::feed_events`, which
//!   resolves them through the registry's dispatch table, and a click goes in as
//!   pointer events over a frame the band has already laid out. The state read
//!   back is the open cell.
//! - **Pixels** are two baselines per mode: the band at rest, and with the x
//!   cell open and previewing a column. They are drawn through `egui_kittest`'s
//!   wgpu renderer and compared under `kittest.toml`'s thresholds. Regenerate
//!   them with `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell
//!   --test shelf_band`, and read what moved before committing it.
//!
//! The channels the fixture's generated map takes are pinned once, in
//! `the_band_reads_the_generated_map_as_a_dot_of_longitude_by_latitude_with_no_colour`,
//! and the baselines and the geometry tests draw that same set directly, so a
//! change to what the generator writes reddens one named test and not all of
//! them.

use std::path::PathBuf;

use brightfield_engine::{ColumnProfile, ProfileOutcome};
use brightfield_keys::registry::{registry, BindingContext};
use brightfield_shell::data_file::{self, OpenedFile};
use brightfield_shell::design::{self, Mode};
use brightfield_shell::shelf::{
    cell_key, BandDrawn, Binding, ChannelSettings, ShelfBand, ShelfChannels, ADD_A_COLUMN,
    AN_EXPRESSION, PREVIEW,
};
use brightfield_shell::shelf_edit::{put_colour, put_column};
use brightfield_shell::text_ink::{self, DrawnText};
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::ast::{Component, Spec, SpecValue, ValueOrParamRef};
use brightfield_spec::edit::{self, plot_at_path, ChartEdit};
use brightfield_spec::layout::PlotAxis;
use brightfield_spec::parse::{parse_spec, Format};
use brightfield_spec::vocab::is_colour_literal;
use brightfield_workbench::channel::ShelfChannel;
use brightfield_workbench::chrome;
use egui::epaint::{ClippedShape, Shape};
use egui_kittest::{Harness, SnapshotOptions};
use meridian_design::{control, semantic, viz};

/// The width the band is drawn at: the generated map's plot at the window size
/// the shelf was drawn for.
const WIDTH: f32 = 578.0;

/// The band's height.
const HEIGHT: f32 = 44.0;

/// Where the band's top left sits in the test's window, off the origin so no
/// reading assumes the band starts at zero.
const ORIGIN: egui::Pos2 = egui::pos2(12.0, 9.0);

/// The room a cell keeps between its edge and its contents.
const PAD: f32 = 8.0;

/// The mark, x, y and colour in the design system's categorical order, which is
/// blue, gold, teal, red, violet, orange, plum, green, counted from zero.
const PLUM: usize = 6;
const TEAL: usize = 2;
const VIOLET: usize = 4;
const ORANGE: usize = 5;

/// The slot each channel takes, in the order the band draws them.
const SLOTS: [(ShelfChannel, usize); 4] = [
    (ShelfChannel::Mark, PLUM),
    (ShelfChannel::X, TEAL),
    (ShelfChannel::Y, VIOLET),
    (ShelfChannel::Colour, ORANGE),
];

// ---------------------------------------------------------------------------
// The fixture: the generated map, as a data file opens it.
// ---------------------------------------------------------------------------

fn housing() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv")
}

struct Opened {
    file: OpenedFile,
    table: Vec<ColumnProfile>,
    spec: Spec,
    hero: ComponentPath,
}

fn open() -> Opened {
    let path = housing();
    let mut file = data_file::open(path.to_str().expect("utf-8 fixture path"))
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let table = file
        .live
        .coordinator()
        .session()
        .profile_sources()
        .into_iter()
        .find(|p| p.name == data_file::SOURCE)
        .map(|p| match p.outcome {
            ProfileOutcome::Profiled { columns, .. } => columns,
            other => panic!("the table did not profile: {other:?}"),
        })
        .expect("the opened file has a source to profile");
    let spec = file.live.spec().clone();
    let hero = ComponentPath(file.composed.plots[0].path.clone());
    Opened {
        file,
        table,
        spec,
        hero,
    }
}

/// What the generator's map takes: a dot, longitude on x, latitude on y and no
/// colour. Drawn directly by every test that is not about reading the map.
fn map_channels() -> ShelfChannels {
    ShelfChannels {
        mark: "dot".to_string(),
        x: Binding::Column("longitude".to_string()),
        y: Binding::Column("latitude".to_string()),
        colour: Binding::Unset,
    }
}

fn channels_of(spec: &Spec, path: &ComponentPath) -> ShelfChannels {
    let plot = plot_at_path(spec, &path.0).expect("the path names a plot");
    ShelfChannels::of_plot(plot).expect("the plot has a mark")
}

// ---------------------------------------------------------------------------
// The stage: a band drawn into a `Ui` a test gives it, and what the frame held.
// ---------------------------------------------------------------------------

struct Stage {
    ctx: egui::Context,
    mode: Mode,
    width: f32,
}

/// One frame of the band: what it reported, what it painted and what text it
/// put on the screen.
struct Frame {
    drawn: BandDrawn,
    shapes: Vec<ClippedShape>,
    texts: Vec<DrawnText>,
    collisions: Option<String>,
}

impl Stage {
    /// A stage `width` wide, with the theme applied and the faces loaded: the
    /// fonts `apply` installs take effect on the pass after it, so two frames
    /// run before anything is measured.
    fn new(mode: Mode, width: f32) -> Self {
        let ctx = egui::Context::default();
        design::apply(&ctx, mode);
        let stage = Self { ctx, mode, width };
        let mut warm = ShelfBand::new(map_channels());
        stage.frame(&mut warm, Vec::new());
        stage.frame(&mut warm, Vec::new());
        stage
    }

    fn frame(&self, band: &mut ShelfBand, events: Vec<egui::Event>) -> Frame {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(self.width + 2.0 * ORIGIN.x, 120.0),
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
                    egui::vec2(self.width, HEIGHT),
                )),
                |ui| drawn = Some(band.show(ui, self.mode)),
            );
            texts = text_ink::frame_text(ui.ctx());
            collisions = text_ink::collision_report(ui.ctx(), "the band");
        });
        Frame {
            drawn: drawn.expect("the band drew"),
            shapes: output.shapes,
            texts,
            collisions,
        }
    }

    /// Draw the band once it has settled, with no input.
    fn draw(&self, band: &mut ShelfBand) -> Frame {
        self.frame(band, Vec::new())
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

/// The bounding box of each stroked path the frame painted: the chevrons.
fn paths(frame: &Frame) -> Vec<egui::Rect> {
    leaves(frame)
        .into_iter()
        .filter_map(|s| match s {
            Shape::Path(p) => Some(egui::Rect::from_points(&p.points)),
            _ => None,
        })
        .collect()
}

/// The colour each galley of text was laid out in, with what it reads.
fn text_colours(frame: &Frame) -> Vec<(String, egui::Color32)> {
    leaves(frame)
        .into_iter()
        .filter_map(|s| match s {
            Shape::Text(t) => Some((
                t.galley.text().to_string(),
                t.galley
                    .job
                    .sections
                    .first()
                    .map_or(egui::Color32::PLACEHOLDER, |sec| sec.format.color),
            )),
            _ => None,
        })
        .collect()
}

/// The text painted inside `cell`, by where its ink is centred.
fn texts_in(frame: &Frame, cell: egui::Rect) -> Vec<&DrawnText> {
    frame
        .texts
        .iter()
        .filter(|t| cell.contains(t.ink.center()))
        .collect()
}

/// The one galley in `cell` that reads `text`.
fn text_in<'a>(frame: &'a Frame, channel: ShelfChannel, text: &str) -> &'a DrawnText {
    let cell = frame.drawn.cells[channel.index()];
    let found: Vec<&DrawnText> = texts_in(frame, cell)
        .into_iter()
        .filter(|t| t.text == text)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "{channel:?}'s cell holds {} galleys reading {text:?}; it holds {:?}",
        found.len(),
        texts_in(frame, cell)
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
    );
    found[0]
}

fn has_text(frame: &Frame, channel: ShelfChannel, text: &str) -> bool {
    let cell = frame.drawn.cells[channel.index()];
    texts_in(frame, cell).iter().any(|t| t.text == text)
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

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

/// A design-system token as the colour the band paints it in.
fn paint(token: meridian_design::Rgba) -> egui::Color32 {
    chrome::colour(token)
}

fn categorical(mode: Mode, slot: usize) -> meridian_design::Rgba {
    if mode.is_dark() {
        viz::CATEGORICAL_DARK[slot]
    } else {
        viz::CATEGORICAL_LIGHT[slot]
    }
}

// ---------------------------------------------------------------------------
// What the band reads out of a plot.
// ---------------------------------------------------------------------------

#[test]
fn the_band_reads_the_generated_map_as_a_dot_of_longitude_by_latitude_with_no_colour() {
    let opened = open();
    assert_eq!(channels_of(&opened.spec, &opened.hero), map_channels());
}

#[test]
fn the_band_reads_the_same_over_the_map_and_over_the_dot_plot_it_becomes() {
    let mut opened = open();
    let before = channels_of(&opened.spec, &opened.hero);

    put_column(
        &mut opened.spec,
        &opened.hero,
        PlotAxis::X,
        "median_income",
        &opened.table,
    )
    .expect("a column goes on the map's x");
    let after = channels_of(&opened.spec, &opened.hero);

    // The same four channels, with x holding the column that took the map off
    // its coordinate pair.
    assert_eq!(after.mark, before.mark);
    assert_eq!(after.x, Binding::Column("median_income".to_string()));
    assert_eq!(after.y, before.y);
    assert_eq!(after.colour, Binding::Unset);

    let stage = Stage::new(Mode::Light, WIDTH);
    let mut band = ShelfBand::new(after);
    let frame = stage.draw(&mut band);
    for channel in ShelfChannel::ALL {
        assert!(
            has_text(&frame, channel, channel.word()),
            "the dot plot's band has no {:?} cell",
            channel.word()
        );
    }
    text_in(&frame, ShelfChannel::X, "median_income");
}

#[test]
fn a_channel_bound_to_an_aggregate_reads_as_an_expression_and_not_as_empty() {
    let opened = open();
    // The generator's histograms count rows in y.
    let histogram = ComponentPath(opened.file.composed.plots[1].path.clone());
    let read = channels_of(&opened.spec, &histogram);
    assert_eq!(read.y, Binding::Expression, "{read:?}");
    assert_ne!(read.x, Binding::Unset, "{read:?}");

    let stage = Stage::new(Mode::Light, WIDTH);
    let frame = stage.draw(&mut ShelfBand::new(read));
    text_in(&frame, ShelfChannel::Y, AN_EXPRESSION);
    assert!(!has_text(&frame, ShelfChannel::Y, ADD_A_COLUMN));
}

#[test]
fn colour_is_read_from_the_layer_the_shelf_writes_it_on() {
    let mut opened = open();
    // The ghost layer is the map's first mark and it binds `fill`, to the ghost
    // ink as a literal. A reader that took the first mark's fill would read
    // that, so this case tells the layers apart.
    let plot = plot_at_path(&opened.spec, &opened.hero.0).expect("the hero");
    let ghost_fill = match &plot.items[0] {
        Component::Mark(m) => m.options.get("fill").cloned(),
        other => panic!("the first item of the map is not a mark: {other:?}"),
    };
    assert!(
        matches!(
            &ghost_fill,
            Some(ValueOrParamRef::Value(SpecValue::String(ink))) if is_colour_literal(ink)
        ),
        "the ghost layer does not bind a colour literal, so this case does not \
         distinguish the layers: {ghost_fill:?}"
    );

    put_colour(
        &mut opened.spec,
        &opened.hero,
        "median_house_value",
        &opened.table,
    )
    .expect("a column goes on the map's colour");
    assert_eq!(
        channels_of(&opened.spec, &opened.hero).colour,
        Binding::Column("median_house_value".to_string())
    );
}

#[test]
fn a_colour_written_as_a_literal_is_no_column_and_reads_as_empty() {
    let mut opened = open();
    // `fill: steelblue` on the highlighted layer is the mark's constant ink.
    let literal = ChartEdit::SetChannel {
        plot: opened.hero.clone(),
        mark_ordinal: 1,
        channel: "fill".to_string(),
        column: "steelblue".to_string(),
    };
    edit::apply_for_fresh_load(&mut opened.spec, &literal).expect("the literal is set");
    assert_eq!(
        channels_of(&opened.spec, &opened.hero).colour,
        Binding::Unset
    );

    // A string that is not a colour is a column.
    let column = ChartEdit::SetChannel {
        plot: opened.hero.clone(),
        mark_ordinal: 1,
        channel: "fill".to_string(),
        column: "weather".to_string(),
    };
    edit::apply_for_fresh_load(&mut opened.spec, &column).expect("the column is set");
    assert_eq!(
        channels_of(&opened.spec, &opened.hero).colour,
        Binding::Column("weather".to_string())
    );
}

// ---------------------------------------------------------------------------
// AC1: the band's extents.
// ---------------------------------------------------------------------------

#[test]
fn drawn_578_wide_over_the_generated_map_the_band_is_44_high_with_four_cells_in_order() {
    let opened = open();
    let stage = Stage::new(Mode::Light, WIDTH);
    let mut band = ShelfBand::new(channels_of(&opened.spec, &opened.hero));
    let frame = stage.draw(&mut band);
    let cells = frame.drawn.cells;

    // The band itself, as the ground it painted.
    let ground = paint(semantic(false).surfaces.header);
    // The keycaps take the same surface, so the ground is the one fill of it
    // wider than a cell.
    let painted: Vec<egui::Rect> = fills(&frame)
        .into_iter()
        .filter(|(rect, fill)| *fill == ground && rect.width() > 200.0)
        .map(|(rect, _)| rect)
        .collect();
    assert_eq!(painted.len(), 1, "the band paints one ground: {painted:?}");
    assert!(near(painted[0].width(), 578.0), "{:?}", painted[0]);
    assert!(near(painted[0].height(), 44.0), "{:?}", painted[0]);
    assert_eq!(painted[0].min, ORIGIN);
    assert_eq!(frame.drawn.rect, painted[0]);

    // Four cells side by side across it, the mark's 88 wide and the other three
    // a third of the rest.
    assert!(near(cells[0].width(), 88.0), "{cells:?}");
    for cell in &cells[1..] {
        assert!(near(cell.width(), (578.0 - 88.0) / 3.0), "{cells:?}");
    }
    assert!(near(cells[0].left(), painted[0].left()));
    for pair in cells.windows(2) {
        assert!(near(pair[0].right(), pair[1].left()), "{cells:?}");
    }
    assert!(near(cells[3].right(), painted[0].right()));
    for cell in &cells {
        assert!(near(cell.height(), 44.0), "{cells:?}");
    }

    // In the order mark, x axis, y axis, colour: the word each cell says.
    let said: Vec<&str> = ["mark", "x axis", "y axis", "colour"].to_vec();
    for (cell, word) in cells.iter().zip(said) {
        assert!(
            texts_in(&frame, *cell).iter().any(|t| t.text == word),
            "the cell at {cell:?} does not say {word:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// AC2: what a cell says.
// ---------------------------------------------------------------------------

/// The key the registry binds, in its Shelf context, to the verb named.
fn registry_key(verb: &str) -> &'static str {
    registry()
        .into_iter()
        .find(|v| v.longname == verb)
        .and_then(|v| {
            v.binding_specs
                .iter()
                .find(|b| b.context == BindingContext::Shelf)
                .map(|b| b.keystrokes)
        })
        .unwrap_or_else(|| panic!("the registry binds {verb} in no Shelf context"))
}

#[test]
fn the_cell_keys_printed_on_the_band_are_the_registrys() {
    let expected = [
        (ShelfChannel::Mark, "go-to-mark-cell", "m"),
        (ShelfChannel::X, "go-to-x-cell", "x"),
        (ShelfChannel::Y, "go-to-y-cell", "y"),
        (ShelfChannel::Colour, "go-to-colour-cell", "c"),
    ];
    let stage = Stage::new(Mode::Light, WIDTH);
    let frame = stage.draw(&mut ShelfBand::new(map_channels()));
    let sunken = paint(semantic(false).surfaces.sunken);
    for (channel, verb, key) in expected {
        // The registry holds the key the prototype named, and the band prints
        // the registry's.
        assert_eq!(registry_key(verb), key, "{verb}");
        assert_eq!(cell_key(channel), key, "{channel:?}");

        // Drawn as a keycap: the glyph sits inside a sunken fill that is a chip
        // and not the cell, and that is the leftmost thing in the cell.
        let cell = frame.drawn.cells[channel.index()];
        let glyph = text_in(&frame, channel, key);
        let caps: Vec<egui::Rect> = fills(&frame)
            .into_iter()
            .filter(|(rect, fill)| {
                *fill == sunken && rect.contains_rect(glyph.ink) && cell.contains_rect(*rect)
            })
            .map(|(rect, _)| rect)
            .collect();
        assert_eq!(
            caps.len(),
            1,
            "{channel:?}: no keycap round {key:?}: {caps:?}"
        );
        assert!(caps[0].width() < 40.0, "{channel:?}: {:?}", caps[0]);
        assert!(
            (caps[0].left() - (cell.left() + PAD)).abs() < 0.5,
            "{channel:?}: the keycap is at {:.4}, not a pad in from {:.4}",
            caps[0].left(),
            cell.left()
        );
        for other in texts_in(&frame, cell) {
            assert!(
                other.ink.left() >= glyph.ink.left() - 0.01,
                "{channel:?}: {:?} stands left of the key",
                other.text
            );
        }
    }
}

#[test]
fn a_cell_says_the_channels_word_over_the_columns_name_with_a_chevron_on_the_words_line() {
    let stage = Stage::new(Mode::Light, WIDTH);
    let frame = stage.draw(&mut ShelfBand::new(map_channels()));
    let names = [
        (ShelfChannel::Mark, "dot"),
        (ShelfChannel::X, "longitude"),
        (ShelfChannel::Y, "latitude"),
        (ShelfChannel::Colour, ADD_A_COLUMN),
    ];
    let chevrons = paths(&frame);
    for (channel, name) in names {
        let cell = frame.drawn.cells[channel.index()];
        let word = text_in(&frame, channel, channel.word());
        let value = text_in(&frame, channel, name);
        assert!(
            word.ink.bottom() <= value.ink.top() + 0.5,
            "{channel:?}: the word {:?} is not over {name:?}",
            word.ink
        );
        assert!(
            near(word.ink.left(), value.ink.left()),
            "{channel:?}: the word and the name do not share a left edge"
        );

        // One chevron in the cell, at its trailing edge, vertically on the
        // word's line and clear of the word.
        let here: Vec<&egui::Rect> = chevrons
            .iter()
            .filter(|r| cell.contains(r.center()))
            .collect();
        assert_eq!(here.len(), 1, "{channel:?}: chevrons in the cell: {here:?}");
        let chevron = here[0];
        assert!(
            chevron.center().y >= word.ink.top() && chevron.center().y <= word.ink.bottom(),
            "{channel:?}: the chevron {chevron:?} is not on the word's line {:?}",
            word.ink
        );
        assert!(chevron.left() > word.ink.right(), "{channel:?}");
        assert!(chevron.right() <= cell.right() - PAD + 0.01, "{channel:?}");
        assert!(chevron.width() <= control::ICON_SM, "{channel:?}");
    }
    assert_eq!(chevrons.len(), 4, "four cells, four chevrons: {chevrons:?}");
}

#[test]
fn the_word_preview_stands_beside_a_column_being_previewed_and_nowhere_else() {
    let stage = Stage::new(Mode::Light, WIDTH);
    let mut band = ShelfBand::new(map_channels());

    let rest = stage.draw(&mut band);
    for channel in ShelfChannel::ALL {
        assert!(!has_text(&rest, channel, PREVIEW), "{channel:?} at rest");
    }

    band.activate(ShelfChannel::X);
    band.set_preview(ShelfChannel::X, "median_income");
    let frame = stage.draw(&mut band);
    let word = text_in(&frame, ShelfChannel::X, "x axis");
    let preview = text_in(&frame, ShelfChannel::X, PREVIEW);
    let name = text_in(&frame, ShelfChannel::X, "median_income");
    assert!(
        preview.ink.left() > word.ink.right(),
        "preview {:?} is not beside the word {:?}",
        preview.ink,
        word.ink
    );
    // The same line: the ink boxes of `preview` and `x axis` differ by a
    // descender, so their centres are compared to a point and not to a hair.
    assert!(
        (preview.ink.center().y - word.ink.center().y).abs() < 2.0,
        "preview {:?} is not on the word's line {:?}",
        preview.ink,
        word.ink
    );
    assert!(name.ink.top() >= word.ink.bottom() - 0.5);
    // The column it replaced is not drawn, and no other cell says preview.
    assert!(!has_text(&frame, ShelfChannel::X, "longitude"));
    for channel in [ShelfChannel::Mark, ShelfChannel::Y, ShelfChannel::Colour] {
        assert!(!has_text(&frame, channel, PREVIEW), "{channel:?}");
    }

    // Dropping it puts the column back and takes the word away.
    band.clear_preview();
    let after = stage.draw(&mut band);
    text_in(&after, ShelfChannel::X, "longitude");
    assert!(!has_text(&after, ShelfChannel::X, PREVIEW));
}

#[test]
fn colour_with_no_column_reads_add_a_column_and_with_one_reads_its_name() {
    let stage = Stage::new(Mode::Light, WIDTH);
    let frame = stage.draw(&mut ShelfBand::new(map_channels()));
    text_in(&frame, ShelfChannel::Colour, ADD_A_COLUMN);
    for channel in [ShelfChannel::Mark, ShelfChannel::X, ShelfChannel::Y] {
        assert!(!has_text(&frame, channel, ADD_A_COLUMN), "{channel:?}");
    }

    let mut painted = map_channels();
    painted.colour = Binding::Column("median_house_value".to_string());
    let frame = stage.draw(&mut ShelfBand::new(painted));
    text_in(&frame, ShelfChannel::Colour, "median_house_value");
    assert!(!has_text(&frame, ShelfChannel::Colour, ADD_A_COLUMN));
}

// ---------------------------------------------------------------------------
// AC3: each channel's hue.
// ---------------------------------------------------------------------------

#[test]
fn each_cell_is_tinted_in_its_channels_hue_at_the_design_systems_strength() {
    for mode in [Mode::Light, Mode::Dark] {
        let stage = Stage::new(mode, WIDTH);
        let frame = stage.draw(&mut ShelfBand::new(map_channels()));
        let painted = fills(&frame);
        let mut seen = Vec::new();
        for (channel, slot) in SLOTS {
            let expected = paint(viz::chrome_tint(categorical(mode, slot), mode.is_dark()));
            let cell = frame.drawn.cells[channel.index()];
            assert!(
                painted.contains(&(cell, expected)),
                "{mode:?} {channel:?}: no fill of {expected:?} over {cell:?}"
            );
            assert!(
                !seen.contains(&expected),
                "{mode:?}: two cells share a tint"
            );
            seen.push(expected);
        }
    }
}

#[test]
fn the_open_cell_adds_a_bar_in_its_hue_along_its_foot_and_no_other_cell_does() {
    for mode in [Mode::Light, Mode::Dark] {
        let stage = Stage::new(mode, WIDTH);
        for (open, slot) in SLOTS {
            let mut band = ShelfBand::new(map_channels());
            band.activate(open);
            let frame = stage.draw(&mut band);
            let painted = fills(&frame);

            // The hue at its own strength, which only the bar takes: the tint is
            // the same hue at a fraction of it.
            let hue = paint(categorical(mode, slot));
            let cell = frame.drawn.cells[open.index()];
            let bars: Vec<egui::Rect> = painted
                .iter()
                .filter(|(_, fill)| *fill == hue)
                .map(|(rect, _)| *rect)
                .collect();
            assert_eq!(bars.len(), 1, "{mode:?} {open:?}: bars {bars:?}");
            let bar = bars[0];
            assert!(near(bar.left(), cell.left()), "{mode:?} {open:?}: {bar:?}");
            assert!(
                near(bar.right(), cell.right()),
                "{mode:?} {open:?}: {bar:?}"
            );
            assert!(
                near(bar.bottom(), cell.bottom()),
                "{mode:?} {open:?}: {bar:?}"
            );
            assert!(
                near(bar.height(), control::TAB_BAR_WIDTH),
                "{mode:?} {open:?}: {bar:?}"
            );

            // It adds to the tint and does not replace it.
            let tint = paint(viz::chrome_tint(categorical(mode, slot), mode.is_dark()));
            assert!(painted.contains(&(cell, tint)), "{mode:?} {open:?}");

            // No other channel's hue is drawn at full strength.
            for (other, other_slot) in SLOTS {
                if other != open {
                    let other_hue = paint(categorical(mode, other_slot));
                    assert!(
                        !painted.iter().any(|(_, fill)| *fill == other_hue),
                        "{mode:?} {open:?} open: {other:?}'s hue is drawn as a bar"
                    );
                }
            }
        }

        // With no cell open there is no bar.
        let frame = stage.draw(&mut ShelfBand::new(map_channels()));
        for (channel, slot) in SLOTS {
            let hue = paint(categorical(mode, slot));
            assert!(
                !fills(&frame).iter().any(|(_, fill)| *fill == hue),
                "{mode:?}: {channel:?}'s hue is drawn with no cell open"
            );
        }
    }
}

#[test]
fn every_label_is_in_a_text_ink_and_none_is_in_a_hue() {
    for mode in [Mode::Light, Mode::Dark] {
        let sem = semantic(mode.is_dark());
        let inks = [
            paint(sem.text.primary),
            paint(sem.text.secondary),
            paint(sem.text.muted),
        ];
        let hues: Vec<egui::Color32> = (0..8).map(|slot| paint(categorical(mode, slot))).collect();
        let stage = Stage::new(mode, WIDTH);
        let mut band = ShelfBand::new(map_channels());
        band.activate(ShelfChannel::X);
        band.set_preview(ShelfChannel::X, "median_income");
        let frame = stage.draw(&mut band);

        let coloured = text_colours(&frame);
        assert!(coloured.len() >= 12, "{mode:?}: {coloured:?}");
        for (text, colour) in &coloured {
            assert!(
                !hues.contains(colour),
                "{mode:?}: {text:?} is lettered in a hue of the palette"
            );
            assert!(
                inks.contains(colour),
                "{mode:?}: {text:?} is lettered in {colour:?}, which is no text ink"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// AC4: a cell keeps its width, and a long name is cut with an ellipsis.
// ---------------------------------------------------------------------------

#[test]
fn a_cell_keeps_its_width_when_its_column_changes() {
    let stage = Stage::new(Mode::Light, WIDTH);
    let mut band = ShelfBand::new(map_channels());
    let short = stage.draw(&mut band).drawn.cells;

    let mut long = map_channels();
    long.x = Binding::Column("an_extraordinarily_long_column_name_for_the_x_axis".to_string());
    long.colour = Binding::Column("median_house_value".to_string());
    band.set_channels(long);
    let replaced = stage.draw(&mut band);
    // The columns did change on the screen, or the widths below compare a band
    // with itself: x's name is cut where it was whole, and colour holds a column.
    assert!(
        texts_in(&replaced, replaced.drawn.cells[ShelfChannel::X.index()])
            .iter()
            .any(|t| t.text.starts_with("an_extra") && t.elided),
        "x still reads its first column"
    );
    text_in(&replaced, ShelfChannel::Colour, "median_house_value");
    assert_eq!(short, replaced.drawn.cells);

    band.activate(ShelfChannel::X);
    band.set_preview(
        ShelfChannel::X,
        "another_column_of_a_length_no_cell_can_hold",
    );
    let changed = stage.draw(&mut band).drawn.cells;
    assert_eq!(short, changed);

    // The width follows the band's and nothing the cells hold.
    let wide = Stage::new(Mode::Light, 800.0);
    let widened = wide.draw(&mut band).drawn.cells;
    assert!(widened[1].width() > short[1].width(), "{widened:?}");
    assert!(near(widened[0].width(), 88.0));
}

#[test]
fn a_name_longer_than_its_cell_ends_in_an_ellipsis_and_one_that_fits_does_not() {
    let stage = Stage::new(Mode::Light, WIDTH);

    let mut channels = map_channels();
    let long = "an_extraordinarily_long_column_name_for_the_x_axis";
    channels.x = Binding::Column(long.to_string());
    channels.y = Binding::Column("median_house_value".to_string());
    let frame = stage.draw(&mut ShelfBand::new(channels));

    let cell = frame.drawn.cells[ShelfChannel::X.index()];
    let cut: Vec<&DrawnText> = texts_in(&frame, cell)
        .into_iter()
        .filter(|t| t.text.starts_with("an_extra"))
        .collect();
    assert_eq!(cut.len(), 1, "{cut:?}");
    assert!(cut[0].elided, "{:?} is not marked elided", cut[0].text);
    assert!(cut[0].text.ends_with('\u{2026}'), "{:?}", cut[0].text);
    assert!(cut[0].text.len() < long.len());
    assert!(
        cut[0].ink.right() <= cell.right() - PAD + 0.5,
        "the cut name reaches {:?}, past the cell's padding {:?}",
        cut[0].ink,
        cell
    );

    // `median_house_value` fits the cell at this width with a little to spare,
    // so it is drawn whole and not cut to be safe.
    let whole = text_in(&frame, ShelfChannel::Y, "median_house_value");
    assert!(!whole.elided);
    assert!(!whole.text.ends_with('\u{2026}'));

    // And no two texts of the band land in one place, with the name cut and
    // the keycap, the word and the chevron beside it.
    assert_eq!(frame.collisions, None);
}

#[test]
fn a_previewed_long_name_leaves_the_word_preview_clear_of_the_channels_word() {
    let stage = Stage::new(Mode::Light, WIDTH);
    let mut band = ShelfBand::new(map_channels());
    band.activate(ShelfChannel::Y);
    band.set_preview(
        ShelfChannel::Y,
        "an_extraordinarily_long_column_name_for_the_y_axis",
    );
    let frame = stage.draw(&mut band);
    assert!(has_text(&frame, ShelfChannel::Y, PREVIEW));
    assert_eq!(frame.collisions, None);
}

// ---------------------------------------------------------------------------
// AC5: the keys, and a click.
// ---------------------------------------------------------------------------

#[test]
fn in_the_shelf_context_the_keys_open_the_cell_they_name_and_move_to_the_one_beside() {
    use egui::Key;
    let mut band = ShelfBand::new(map_channels());
    let press = |band: &mut ShelfBand, key: Key| band.feed_events(&[key_event(key)]);

    assert_eq!(band.active(), None);
    // `l` and `h` with no cell open open none.
    press(&mut band, Key::L);
    press(&mut band, Key::H);
    assert_eq!(band.active(), None);

    assert!(press(&mut band, Key::X));
    assert_eq!(band.active(), Some(ShelfChannel::X));
    assert!(press(&mut band, Key::L));
    assert_eq!(band.active(), Some(ShelfChannel::Y));
    assert!(press(&mut band, Key::H));
    assert_eq!(band.active(), Some(ShelfChannel::X));
    assert!(press(&mut band, Key::H));
    assert_eq!(band.active(), Some(ShelfChannel::Mark));

    // `h` at the mark's cell leaves it open, and the band still answers it.
    assert!(press(&mut band, Key::H));
    assert_eq!(band.active(), Some(ShelfChannel::Mark));

    // Right, through to colour, where it stops.
    for expected in [ShelfChannel::X, ShelfChannel::Y, ShelfChannel::Colour] {
        press(&mut band, Key::L);
        assert_eq!(band.active(), Some(expected));
    }
    assert!(press(&mut band, Key::L));
    assert_eq!(band.active(), Some(ShelfChannel::Colour));

    // The other three go to a cell from any cell, and the arrows are the
    // letters' twins.
    press(&mut band, Key::M);
    assert_eq!(band.active(), Some(ShelfChannel::Mark));
    press(&mut band, Key::C);
    assert_eq!(band.active(), Some(ShelfChannel::Colour));
    press(&mut band, Key::Y);
    assert_eq!(band.active(), Some(ShelfChannel::Y));
    press(&mut band, Key::ArrowLeft);
    assert_eq!(band.active(), Some(ShelfChannel::X));
    press(&mut band, Key::ArrowRight);
    assert_eq!(band.active(), Some(ShelfChannel::Y));

    // `Esc` leaves no cell open.
    assert!(press(&mut band, Key::Escape));
    assert_eq!(band.active(), None);
}

#[test]
fn a_key_with_a_modifier_and_the_lists_keys_are_not_the_bands() {
    use egui::Key;
    let mut band = ShelfBand::new(map_channels());
    band.activate(ShelfChannel::X);

    for modifiers in [
        egui::Modifiers::COMMAND,
        egui::Modifiers::CTRL,
        egui::Modifiers::ALT,
        egui::Modifiers::SHIFT,
    ] {
        assert!(!band.press(Key::Y, modifiers), "{modifiers:?}");
        assert_eq!(band.active(), Some(ShelfChannel::X), "{modifiers:?}");
    }
    // The list's keys belong to the list.
    for key in [Key::J, Key::K, Key::Enter, Key::Tab, Key::Slash, Key::U] {
        assert!(!band.feed_events(&[key_event(key)]), "{key:?}");
        assert_eq!(band.active(), Some(ShelfChannel::X), "{key:?}");
    }
    // A key released, and a key that is text and not a key, are not presses.
    let release = egui::Event::Key {
        key: Key::Y,
        physical_key: Some(Key::Y),
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    assert!(!band.feed_events(&[release, egui::Event::Text("y".to_string())]));
    assert_eq!(band.active(), Some(ShelfChannel::X));
}

#[test]
fn a_preview_belongs_to_its_cell_and_goes_when_the_cell_does() {
    use egui::Key;
    let mut band = ShelfBand::new(map_channels());
    band.activate(ShelfChannel::X);
    band.set_preview(ShelfChannel::X, "median_income");
    assert_eq!(band.preview(), Some((ShelfChannel::X, "median_income")));

    // Reopening the open cell keeps it, and moving to another drops it.
    band.activate(ShelfChannel::X);
    assert_eq!(band.preview(), Some((ShelfChannel::X, "median_income")));
    band.feed_events(&[key_event(Key::L)]);
    assert_eq!(band.active(), Some(ShelfChannel::Y));
    assert_eq!(band.preview(), None);

    band.set_preview(ShelfChannel::Y, "median_income");
    band.feed_events(&[key_event(Key::Escape)]);
    assert_eq!((band.active(), band.preview()), (None, None));
}

#[test]
fn a_click_on_a_cell_opens_it() {
    let stage = Stage::new(Mode::Light, WIDTH);
    let mut band = ShelfBand::new(map_channels());
    let cells = stage.draw(&mut band).drawn.cells;

    for channel in [
        ShelfChannel::Y,
        ShelfChannel::Mark,
        ShelfChannel::Colour,
        ShelfChannel::X,
    ] {
        let at = cells[channel.index()].center();
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        stage.frame(&mut band, vec![egui::Event::PointerMoved(at)]);
        stage.frame(&mut band, vec![button(true)]);
        let released = stage.frame(&mut band, vec![button(false)]);
        assert_eq!(released.drawn.clicked, Some(channel), "{channel:?}");
        assert_eq!(band.active(), Some(channel), "{channel:?}");

        // The bar is under the cell that was clicked.
        let hue = paint(categorical(Mode::Light, SLOTS[channel.index()].1));
        let bar = fills(&released)
            .into_iter()
            .find(|(_, fill)| *fill == hue)
            .map(|(rect, _)| rect)
            .expect("the open cell's bar");
        assert!(
            near(bar.left(), cells[channel.index()].left()),
            "{channel:?}"
        );
    }

    // A frame with no click reports none.
    assert_eq!(stage.draw(&mut band).drawn.clicked, None);
}

// ---------------------------------------------------------------------------
// AC6: the baselines.
// ---------------------------------------------------------------------------

/// `kittest.toml`'s thresholds, unloosened.
fn options() -> SnapshotOptions {
    SnapshotOptions::default()
}

/// Draw the band over the generated map's channels through the wgpu renderer
/// and compare it with the committed baseline `name`.
fn baseline(name: &str, mode: Mode, open: Option<(ShelfChannel, &'static str)>) {
    let mut band = ShelfBand::new(map_channels());
    if let Some((channel, preview)) = open {
        band.activate(channel);
        band.set_preview(channel, preview);
    }
    let mut harness = Harness::builder()
        .with_size(egui::vec2(WIDTH, HEIGHT))
        .with_pixels_per_point(2.0)
        .wgpu()
        .build_ui(move |ui| {
            design::apply(ui.ctx(), mode);
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(WIDTH, HEIGHT),
                )),
                |ui| {
                    band.show(ui, mode);
                },
            );
        });
    harness.run();
    harness.snapshot_options(name, &options());
}

#[test]
fn the_band_at_rest_light_matches_its_baseline() {
    baseline("shelf_band_rest_light", Mode::Light, None);
}

#[test]
fn the_band_at_rest_dark_matches_its_baseline() {
    baseline("shelf_band_rest_dark", Mode::Dark, None);
}

#[test]
fn the_x_cell_open_and_previewing_a_column_light_matches_its_baseline() {
    baseline(
        "shelf_band_preview_light",
        Mode::Light,
        Some((ShelfChannel::X, "median_income")),
    );
}

#[test]
fn the_x_cell_open_and_previewing_a_column_dark_matches_its_baseline() {
    baseline(
        "shelf_band_preview_dark",
        Mode::Dark,
        Some((ShelfChannel::X, "median_income")),
    );
}

// ---------------------------------------------------------------------------
// What a cell marks: the dot of a value set, and the scale's name.
// ---------------------------------------------------------------------------

/// What the map's axes read when the plot's attributes are `attrs`, a block of
/// top-level lines of a one-plot spec, as the window reads them off the live
/// plot and hands them to the band.
fn settings_of(attrs: &str) -> ChannelSettings {
    let source = format!(
        "data:\n  t:\n    - {{ a: 1 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: a\n    y: a\nwidth: 600\nheight: 300\n{attrs}\n"
    );
    let spec = parse_spec(&source, Format::Yaml)
        .expect("the spec parses")
        .spec;
    let plot = plot_at_path(&spec, "root").expect("the spec's root is its plot");
    ChannelSettings::of_plot(&spec, plot, &map_channels())
}

/// The band over the generated map's channels, handed what `attrs` set.
fn band_with(attrs: &str) -> ShelfBand {
    let mut band = ShelfBand::new(map_channels());
    band.set_settings(settings_of(attrs));
    band
}

/// Every filled circle the frame painted inside `cell`: its centre, its radius
/// and its fill.
fn dots_in(frame: &Frame, cell: egui::Rect) -> Vec<(egui::Pos2, f32, egui::Color32)> {
    leaves(frame)
        .into_iter()
        .filter_map(|s| match s {
            Shape::Circle(c) if cell.contains(c.center) && c.fill != egui::Color32::TRANSPARENT => {
                Some((c.center, c.radius, c.fill))
            }
            _ => None,
        })
        .collect()
}

/// What a frame holds that a mark on a cell would change: the text and where
/// its ink sits, the filled rectangles, the chevrons and the filled circles.
#[allow(clippy::type_complexity)]
fn drawing_of(
    frame: &Frame,
) -> (
    Vec<(String, egui::Rect)>,
    Vec<(egui::Rect, egui::Color32)>,
    Vec<egui::Rect>,
    Vec<(egui::Pos2, f32, egui::Color32)>,
) {
    let whole = egui::Rect::EVERYTHING;
    (
        frame
            .texts
            .iter()
            .map(|t| (t.text.clone(), t.ink))
            .collect(),
        fills(frame),
        paths(frame),
        dots_in(frame, whole),
    )
}

/// One attribute that sets each of an axis's rows, head and by name, spelled for
/// the axis's letter.
const ROW_KEYS: [&str; 7] = [
    "Label: Residents",
    "Scale: log",
    "TickFormat: .2s",
    "Ticks: 8",
    "Grid: false",
    "Zero: true",
    "Reverse: true",
];

/// **A cell carries a dot while any one of its channel's rows is set, whichever
/// row it is**, the three head rows and the four found by name, and only on the
/// cell of the channel whose key was written. The dot is a filled circle in the
/// text ink on the word's line, to the right of the word.
#[test]
fn a_cell_carries_a_dot_while_any_one_of_its_channels_rows_is_set_and_no_other_cell_does() {
    let stage = Stage::new(Mode::Light, WIDTH);
    let ink = chrome::colour(semantic(false).text.primary);
    for channel in [ShelfChannel::X, ShelfChannel::Y] {
        let letter = if channel == ShelfChannel::X { "x" } else { "y" };
        for key in ROW_KEYS {
            let attrs = format!("{letter}{key}");
            let mut band = band_with(&attrs);
            let frame = stage.draw(&mut band);
            for other in ShelfChannel::ALL {
                let dots = dots_in(&frame, frame.drawn.cells[other.index()]);
                if other != channel {
                    assert!(
                        dots.is_empty(),
                        "`{attrs}` marks {other:?}'s cell: {dots:?}"
                    );
                    continue;
                }
                assert_eq!(dots.len(), 1, "`{attrs}` marks {channel:?}'s cell once");
                let (centre, _, fill) = dots[0];
                assert_eq!(fill, ink, "the dot is in the text ink");
                let word = frame.texts.iter().find(|t| {
                    frame.drawn.cells[channel.index()].contains(t.ink.center())
                        && t.text.starts_with(channel.word())
                });
                let word = word.expect("the cell's word");
                assert!(
                    centre.x > word.ink.right(),
                    "the dot ({centre:?}) follows the word ({:?})",
                    word.ink
                );
                assert!(
                    (centre.y - word.ink.center().y).abs() < 2.0,
                    "the dot stands on the word's line"
                );
            }
        }
    }
}

/// **A plot that sets nothing draws the band as a band handed no settings does**,
/// and so does one that writes brightfield's own value of every key: a cell is
/// marked by a value that differs, not by a key that is written.
#[test]
fn a_plot_with_nothing_set_draws_the_band_as_it_does_with_no_settings() {
    let stage = Stage::new(Mode::Light, WIDTH);
    let bare = drawing_of(&stage.draw(&mut ShelfBand::new(map_channels())));
    assert!(
        bare.3.is_empty(),
        "a band with no settings paints no dot: {:?}",
        bare.3
    );
    let written = "xScale: linear\nxTicks: 5\nxGrid: true\nxZero: false\nxReverse: false";
    for attrs in ["", written] {
        let drawn = drawing_of(&stage.draw(&mut band_with(attrs)));
        assert_eq!(drawn, bare, "`{attrs}` draws as a band with no settings");
    }
}

/// **The scale's name follows the channel's word while the scale is not linear**,
/// and nothing follows a linear one: `x axis · log`, `y axis · symlog`, `x axis`.
#[test]
fn the_scale_name_follows_the_word_while_the_scale_is_not_linear() {
    let stage = Stage::new(Mode::Light, WIDTH);
    let (x, y) = (ShelfChannel::X.word(), ShelfChannel::Y.word());
    let cases = [
        ("xScale: log", ShelfChannel::X, format!("{x} · log")),
        ("xScale: symlog", ShelfChannel::X, format!("{x} · symlog")),
        ("yScale: log", ShelfChannel::Y, format!("{y} · log")),
        ("xScale: linear", ShelfChannel::X, x.to_string()),
        ("xGrid: false", ShelfChannel::X, x.to_string()),
        ("", ShelfChannel::X, x.to_string()),
    ];
    for (attrs, channel, said) in cases {
        let frame = stage.draw(&mut band_with(attrs));
        assert!(
            has_text(&frame, channel, &said),
            "`{attrs}` reads {said:?} in {channel:?}'s cell: {:?}",
            texts_in(&frame, frame.drawn.cells[channel.index()])
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>()
        );
    }
    // The other cells keep their words: an axis's scale marks its own cell.
    let frame = stage.draw(&mut band_with("xScale: log"));
    assert!(has_text(&frame, ShelfChannel::Y, y));
    assert!(has_text(
        &frame,
        ShelfChannel::Mark,
        ShelfChannel::Mark.word()
    ));
}

/// **The dot and the scale's name leave the word *preview* its room**: a
/// previewed column on a cell with both marks reads the word, the scale, the dot
/// and *preview* in that order, none over another.
#[test]
fn a_previewed_cell_with_a_dot_and_a_scale_reads_word_dot_and_preview_in_order() {
    // Wide enough that the cell has the room for all three.
    let stage = Stage::new(Mode::Light, 1100.0);
    let mut band = band_with("xScale: log");
    band.activate(ShelfChannel::X);
    band.set_preview(ShelfChannel::X, "median_income");
    let frame = stage.draw(&mut band);
    assert!(frame.collisions.is_none(), "{:?}", frame.collisions);
    let cell = frame.drawn.cells[ShelfChannel::X.index()];
    let word = text_in(
        &frame,
        ShelfChannel::X,
        &format!("{} · log", ShelfChannel::X.word()),
    );
    let preview = text_in(&frame, ShelfChannel::X, PREVIEW);
    let dots = dots_in(&frame, cell);
    assert_eq!(dots.len(), 1);
    let (centre, radius, _) = dots[0];
    assert!(
        word.ink.right() < centre.x - radius,
        "the dot follows the word"
    );
    assert!(
        centre.x + radius < preview.ink.left(),
        "the word preview follows the dot"
    );
}

/// **The word *preview* stands clear of the dot at each width the cell takes**:
/// the preview is drawn where the word and the dot leave it the room, and left
/// out where they do not, so that it is never drawn over the dot. The widths are
/// scanned in steps finer than the dot and its gap, and the scan has to meet
/// both a width that holds the preview and one that does not.
#[test]
fn a_previewed_cell_keeps_the_word_preview_clear_of_the_dot_at_each_width() {
    let (mut drawn_at, mut left_out_at) = (0, 0);
    for width in (400..=1100).step_by(6) {
        let stage = Stage::new(Mode::Light, width as f32);
        let mut band = band_with("xScale: log");
        band.activate(ShelfChannel::X);
        band.set_preview(ShelfChannel::X, "median_income");
        let frame = stage.draw(&mut band);
        let cell = frame.drawn.cells[ShelfChannel::X.index()];
        let Some(preview) = texts_in(&frame, cell)
            .into_iter()
            .find(|t| t.text == PREVIEW)
        else {
            left_out_at += 1;
            continue;
        };
        drawn_at += 1;
        for (centre, radius, _) in dots_in(&frame, cell) {
            assert!(
                centre.x + radius < preview.ink.left(),
                "at {width} the word preview ({:?}) is over the dot ({centre:?})",
                preview.ink
            );
        }
    }
    assert!(
        drawn_at > 0 && left_out_at > 0,
        "the scan met {drawn_at} widths with the preview and {left_out_at} without"
    );
}

/// The band over the generated map with a scale and a value set, through the wgpu
/// renderer, compared with the committed baseline `name`.
fn marked_baseline(name: &str, mode: Mode) {
    let mut band = band_with("xScale: log\nyGrid: false");
    let mut harness = Harness::builder()
        .with_size(egui::vec2(WIDTH, HEIGHT))
        .with_pixels_per_point(2.0)
        .wgpu()
        .build_ui(move |ui| {
            design::apply(ui.ctx(), mode);
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(WIDTH, HEIGHT),
                )),
                |ui| {
                    band.show(ui, mode);
                },
            );
        });
    harness.run();
    harness.snapshot_options(name, &options());
}

#[test]
fn the_band_with_a_scale_and_a_value_set_light_matches_its_baseline() {
    marked_baseline("shelf_band_marked_light", Mode::Light);
}

#[test]
fn the_band_with_a_scale_and_a_value_set_dark_matches_its_baseline() {
    marked_baseline("shelf_band_marked_dark", Mode::Dark);
}
