//! The shelf band: one cell for the mark and one for each channel the mark
//! takes, naming the column on it and the key that reaches it.
//!
//! **The cells come from the channels the plot's mark takes** (mark, x, y and
//! fill), and not from the chart kind's slots, whose roles on the generated map
//! are `lon` and `lat`. So the band reads the same over the map and over the dot
//! plot the map becomes when x takes a column that is not a coordinate:
//! [`ShelfChannels::of_plot`] reads a plot, and
//! `the_band_reads_the_same_over_the_map_and_over_the_dot_plot_it_becomes`
//! holds both.
//!
//! **This module draws the band and answers it; it does not place it.** A
//! caller hands [`ShelfBand::show`] a `Ui` the band's width and gets back the
//! rectangles it drew and whether a cell was clicked. The window carves the
//! band's [`BAND_HEIGHT`] out of a pane and routes keys to
//! [`ShelfBand::feed_events`] while the shelf holds focus.
//!
//! # What a cell says
//!
//! Its key as a keycap, the channel's name as a word over the column's name,
//! and a chevron on the word's line. The word *preview* stands beside the word
//! while the column is a preview, and a channel with no column reads *add a
//! column*. A column's name longer than the room it has ends in an ellipsis,
//! and the cell keeps its width, which [`channel::cell_widths`] takes from the
//! band's width alone.
//!
//! **A cell marks what is set on its channel.** A dot follows the word while any
//! one of the channel's settings rows is the analyst's, and the scale's name
//! follows it (`x axis · log`) while the scale is not linear: log or symlog, or
//! the band and time that an axis of names or of dates draws, which leave no
//! dot since they are brightfield's own. The band is handed
//! the rows by [`ShelfBand::set_settings`]; a band handed none, and a plot that
//! sets nothing, draw the cell as it was drawn before there were marks.
//!
//! # Colour
//!
//! Each cell's ground is its channel's hue at the design system's tint
//! strength, and the open cell adds a bar in that hue along its foot. Every label
//! is in a text ink: a hue marks a kind of data, and a hue on the lettering
//! would be a second mark for the same thing. Which hue a channel takes is
//! [`channel::hue`]'s.
//!
//! # Keys
//!
//! The keys are the registry's, in its Shelf context: `m` `x` `y` `c` go to a
//! cell, `h` `l` (and the arrows) move to the cell beside and stop at the
//! mark's cell on the left and at colour on the right, `Tab` turns an axis's
//! list between its columns and its settings, and `Esc` leaves the open cell.
//! The keycap a cell prints is read from the registry's binding for
//! the verb that goes to it, so a key moved there moves on the band.
//! `the_cell_keys_printed_on_the_band_are_the_registrys` holds it. The list of
//! columns that opens under a cell, its query, `j` `k` and `Enter` belong to
//! the list, and the band returns `false` for them, as
//! `a_key_with_a_modifier_and_the_lists_keys_are_not_the_bands` holds.

use std::sync::OnceLock;

use brightfield_keys::dispatch::{resolution_table, DispatchContext, ResolutionTable};
use brightfield_keys::registry::{keymap_bindings, registry, BindingContext};
use brightfield_render::axis::{axis_kind, tick_count_applies, top_tick_text, AxisKind};
use brightfield_render::channel::Channel;
use brightfield_render::scale::{log_ends_refused, Scale, ScaleSet};
use brightfield_render::scene::{axis_ends_apply, axis_keys_apply, axis_reverse_applies};
use brightfield_spec::ast::{Mark, PlotNode, Spec, SpecValue, ValueOrParamRef};
use brightfield_spec::layout::{read_domains_in, DomainReading};
use brightfield_spec::layout::{
    read_tick_format, resolve_axis_ends, resolve_axis_reverse, resolve_axis_titles,
    resolve_grid_lines, resolve_plot_scales_in, resolve_tick_counts, tick_count_target, AxisTitle,
    ScaleType, TickFormatReading, DEFAULT_TICK_COUNT, MAX_TICK_COUNT,
};
use brightfield_spec::number_format::NumberFormat;
use brightfield_spec::vocab::is_colour_literal;
use brightfield_workbench::channel::{self, ShelfChannel, BAND_HEIGHT};
use brightfield_workbench::chrome;
use meridian_design::{control, radius, semantic, spacing, typography, Elevation};
use meridian_egui::{icons, key_chip};

use brightfield_engine::ColumnMoments;

use crate::column_header::{column_header_frame, draw_rug_in, GridDensity, RugDrawn};
use crate::design::Mode;
use crate::protocol::{caption, caption_font, ui_font};
use crate::shelf_edit::{column_of, marks_of, reads_selection};
use crate::text_ink::{self, TwoEndedRow};

/// What a channel with no column reads.
pub const ADD_A_COLUMN: &str = "add a column";

/// The word that stands beside a column being previewed.
pub const PREVIEW: &str = "preview";

/// What a channel bound to something other than a plain column reads: an
/// aggregate, a transform, an expression or a param.
pub const AN_EXPRESSION: &str = "an expression";

/// The channel a colour column is bound through, as a spec writes it.
const COLOUR_KEY: &str = "fill";

/// The room a cell keeps between its edge and its contents.
const PAD: f32 = spacing::SPACE_4;

/// The gap between a cell's keycap and its words.
const KEY_GAP: f32 = spacing::SPACE_3;

/// The gap between the channel's word and the one above the column's name.
const LINE_GAP: f32 = spacing::SPACE_1;

/// The inset of the hairline between two cells from the band's top and foot.
const RULE_INSET: f32 = spacing::SPACE_4;

/// What one channel of the plot's mark is bound to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Binding {
    /// The mark binds nothing to the channel.
    Unset,
    /// A plain column, by name.
    Column(String),
    /// Something that is not a plain column: an aggregate, a transform, an
    /// expression or a param.
    Expression,
}

/// What the plot's mark takes on each of the band's channels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShelfChannels {
    /// The mark's kind, as a spec writes it: `dot`.
    pub mark: String,
    /// What x is bound to.
    pub x: Binding,
    /// What y is bound to.
    pub y: Binding,
    /// What colour, written `fill`, is bound to.
    pub colour: Binding,
}

impl ShelfChannels {
    /// The channels `plot`'s marks take, or `None` when it has no mark.
    ///
    /// The mark is the first mark's kind. x and y are read from the first mark
    /// that binds them, as [`crate::shelf_edit::put_column`] moves each mark
    /// that binds the channel. Colour is read where
    /// [`crate::shelf_edit::put_colour`] writes it: the first mark that reads
    /// through a selection, or the first mark when no mark does, so the map's
    /// ghost layer, which takes a literal ink and no column, is not the layer
    /// the cell reads: `colour_is_read_from_the_layer_the_shelf_writes_it_on`.
    #[must_use]
    pub fn of_plot(plot: &PlotNode) -> Option<Self> {
        let marks = marks_of(plot);
        let first = marks.first()?;
        let bound = |channel: &str, among: &[&Mark]| {
            among
                .iter()
                .find(|m| m.options.contains_key(channel))
                .map_or(Binding::Unset, |m| binding(m, channel))
        };
        let highlighted: Vec<&Mark> = marks
            .iter()
            .copied()
            .filter(|m| reads_selection(m))
            .collect();
        let painted: &[&Mark] = if highlighted.is_empty() {
            &marks[..1]
        } else {
            &highlighted
        };
        Some(Self {
            mark: first.kind.wire_name().to_string(),
            x: bound("x", &marks),
            y: bound("y", &marks),
            colour: bound(COLOUR_KEY, painted),
        })
    }

    /// What `channel` is bound to. The mark's cell holds the mark's kind and
    /// not a column, so it has no binding.
    fn binding(&self, channel: ShelfChannel) -> Option<&Binding> {
        match channel {
            ShelfChannel::Mark => None,
            ShelfChannel::X => Some(&self.x),
            ShelfChannel::Y => Some(&self.y),
            ShelfChannel::Colour => Some(&self.colour),
        }
    }
}

/// How `mark` binds `channel`: a plain column name, or something else.
///
/// **A colour written as a literal is no column.** The spec language writes
/// `fill: steelblue` and `fill: weather` in one slot, and the string decides
/// which: the renderer binds a literal as the mark's constant ink and anything
/// else as a column. The map's ghost layer carries one, so reading it as a
/// column would put a hex code in the colour cell. The cell reads as empty, as
/// a channel with no column does.
fn binding(mark: &Mark, channel: &str) -> Binding {
    match column_of(mark, channel) {
        Some(name) if channel == COLOUR_KEY && is_colour_literal(name) => Binding::Unset,
        Some(name) => Binding::Column(name.to_string()),
        None => match mark.options.get(channel) {
            None | Some(ValueOrParamRef::Value(SpecValue::Null)) => Binding::Unset,
            Some(_) => Binding::Expression,
        },
    }
}

/// What a band drew in one frame.
#[derive(Clone, Debug)]
pub struct BandDrawn {
    /// The whole band.
    pub rect: egui::Rect,
    /// Each cell, in [`ShelfChannel::ALL`] order.
    pub cells: [egui::Rect; 4],
    /// The cell a click landed on this frame, which is also the cell now open.
    pub clicked: Option<ShelfChannel>,
}

/// The band's state: what the plot's mark takes, which cell is open and which
/// column is being previewed.
#[derive(Clone, Debug)]
pub struct ShelfBand {
    channels: ShelfChannels,
    active: Option<ShelfChannel>,
    preview: Option<(ShelfChannel, String)>,
    /// What the axes' settings read, which a cell marks: a dot while any of its
    /// channel's rows is set, and the scale's name while it is not linear.
    settings: ChannelSettings,
}

impl ShelfBand {
    /// A band over `channels` with no cell open.
    #[must_use]
    pub fn new(channels: ShelfChannels) -> Self {
        Self {
            channels,
            active: None,
            preview: None,
            settings: ChannelSettings::default(),
        }
    }

    /// Hand the band what the axes' settings read, as the plot's spec changes.
    /// A band handed none, or a plot that sets nothing, draws its cells with no
    /// mark on them.
    pub fn set_settings(&mut self, settings: ChannelSettings) {
        self.settings = settings;
    }

    /// What the plot's mark takes, as the band says it.
    #[must_use]
    pub fn channels(&self) -> &ShelfChannels {
        &self.channels
    }

    /// Replace what the plot's mark takes, as the plot's spec changes, and keep
    /// the open cell and the preview where they are.
    pub fn set_channels(&mut self, channels: ShelfChannels) {
        self.channels = channels;
    }

    /// The open cell, if any.
    #[must_use]
    pub fn active(&self) -> Option<ShelfChannel> {
        self.active
    }

    /// The column being previewed, and the channel it is previewed on.
    #[must_use]
    pub fn preview(&self) -> Option<(ShelfChannel, &str)> {
        self.preview.as_ref().map(|(c, name)| (*c, name.as_str()))
    }

    /// Open `channel`'s cell. A preview belongs to the cell it was made in, so
    /// opening another cell drops it.
    pub fn activate(&mut self, channel: ShelfChannel) {
        if self.active != Some(channel) {
            self.preview = None;
        }
        self.active = Some(channel);
    }

    /// Leave the open cell, and the preview made in it.
    pub fn leave(&mut self) {
        self.active = None;
        self.preview = None;
    }

    /// Show `column` on `channel`'s cell as a preview: its name, with the word
    /// *preview* beside it, until the preview is dropped or the cell is left.
    pub fn set_preview(&mut self, channel: ShelfChannel, column: impl Into<String>) {
        self.preview = Some((channel, column.into()));
    }

    /// Drop the preview and show what the plot's mark takes.
    pub fn clear_preview(&mut self) {
        self.preview = None;
    }

    /// Answer the registry's Shelf-context verb `verb`, and say whether the band
    /// answers it.
    ///
    /// The four `go-to-…-cell` verbs open a cell. `move-shelf-left` and
    /// `move-shelf-right` open the cell beside the open one, and stop at the
    /// mark's cell and at colour; with no cell open they open none.
    /// `back-out-of-shelf` leaves the open cell. The list's verbs are not the
    /// band's and return `false`.
    ///
    /// `Esc` here takes the band back in one step. The steps the list adds
    /// (a value not kept, then the query, then the list) are the list's to take
    /// before the key reaches the band.
    pub fn dispatch(&mut self, verb: &str) -> bool {
        match verb {
            "go-to-mark-cell" => self.activate(ShelfChannel::Mark),
            "go-to-x-cell" => self.activate(ShelfChannel::X),
            "go-to-y-cell" => self.activate(ShelfChannel::Y),
            "go-to-colour-cell" => self.activate(ShelfChannel::Colour),
            "move-shelf-left" => self.step(-1),
            "move-shelf-right" => self.step(1),
            "back-out-of-shelf" => self.leave(),
            _ => return false,
        }
        true
    }

    /// Open the cell `by` places from the open one, where there is one.
    fn step(&mut self, by: isize) {
        let Some(open) = self.active else { return };
        let to = open.index().checked_add_signed(by);
        if let Some(next) = to.and_then(|i| ShelfChannel::ALL.get(i)) {
            self.activate(*next);
        }
    }

    /// Answer a key press in the Shelf context, and say whether the band
    /// answers it.
    ///
    /// The key is resolved through the registry's dispatch table, in the
    /// context where a shelf holds focus, to the verbs bound to it there. A key
    /// with a modifier held, and a key the Shelf context does not bind to a verb
    /// the band answers, return `false`.
    pub fn press(&mut self, key: egui::Key, modifiers: egui::Modifiers) -> bool {
        if !modifiers.is_none() {
            return false;
        }
        let Some(token) = key_token(key) else {
            return false;
        };
        shelf_keys()
            .resolves(token, DispatchContext::ShelfFocused)
            .into_iter()
            .any(|verb| self.dispatch(verb))
    }

    /// [`ShelfBand::press`] for each key press among `events`, in order, and
    /// whether the band answered any.
    pub fn feed_events(&mut self, events: &[egui::Event]) -> bool {
        let mut answered = false;
        for event in events {
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            {
                answered |= self.press(*key, *modifiers);
            }
        }
        answered
    }

    /// What `channel`'s cell says: the word over the column, the column (or what
    /// stands for none), whether the column is a preview and whether the
    /// channel holds no column.
    fn words(&self, channel: ShelfChannel) -> CellWords {
        let word = channel.word();
        let rows = self.settings.rows(channel);
        let set = rows.iter().any(|row| row.set);
        let scale = rows
            .iter()
            .find(|row| row.name == SCALE_ROW && row.value != ScaleType::Linear.wire_name())
            .map(|row| row.value.clone());
        if let Some((on, column)) = &self.preview {
            if *on == channel {
                return CellWords {
                    word,
                    value: column.clone(),
                    previewed: true,
                    empty: false,
                    set,
                    scale,
                };
            }
        }
        let (value, empty) = match self.channels.binding(channel) {
            None => (self.channels.mark.clone(), false),
            Some(Binding::Column(name)) => (name.clone(), false),
            Some(Binding::Expression) => (AN_EXPRESSION.to_string(), false),
            Some(Binding::Unset) => (ADD_A_COLUMN.to_string(), true),
        };
        CellWords {
            word,
            value,
            previewed: false,
            empty,
            set,
            scale,
        }
    }

    /// Draw the band into `ui`, which it takes the whole width of, and answer a
    /// click on a cell by opening it.
    ///
    /// The band is [`BAND_HEIGHT`] high and its cells are
    /// [`channel::cell_widths`] wide. The Meridian theme has to be applied to
    /// `ui`'s context, for the keycap's tokens and the faces.
    pub fn show(&mut self, ui: &mut egui::Ui, mode: Mode) -> BandDrawn {
        let sem = semantic(mode.is_dark());
        let (band, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), BAND_HEIGHT),
            egui::Sense::hover(),
        );
        let widths = channel::cell_widths(band.width());
        let mut left = band.left();
        let mut cells = [band; 4];
        for (slot, width) in cells.iter_mut().zip(widths) {
            *slot = egui::Rect::from_min_max(
                egui::pos2(left, band.top()),
                egui::pos2(left + width, band.bottom()),
            );
            left += width;
        }

        let painter = ui.painter().clone();
        painter.rect_filled(band, 0.0, chrome::colour(sem.surfaces.header));

        let mut clicked = None;
        for channel in ShelfChannel::ALL {
            let cell = cells[channel.index()];
            let response = ui.interact(
                cell,
                ui.id().with(("shelf-cell", channel.index())),
                egui::Sense::click(),
            );
            let words = self.words(channel);
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("{}: {}", words.word, words.value),
                )
            });
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() {
                clicked = Some(channel);
            }

            if response.hovered() {
                painter.rect_filled(cell, 0.0, chrome::colour(sem.rows.hover_background));
            }
            painter.rect_filled(cell, 0.0, chrome::colour(channel::tint(channel, mode)));
            if channel.index() > 0 {
                painter.line_segment(
                    [
                        egui::pos2(cell.left(), band.top() + RULE_INSET),
                        egui::pos2(cell.left(), band.bottom() - RULE_INSET),
                    ],
                    egui::Stroke::new(1.0, chrome::colour(sem.borders.subtle)),
                );
            }
            paint_words(ui, &painter, cell, channel, &words, mode);
        }

        // The hairline under the band, and over it the bar of the open cell, so
        // the bar is the last thing drawn at the foot.
        painter.line_segment(
            [
                egui::pos2(band.left(), band.bottom() - 0.5),
                egui::pos2(band.right(), band.bottom() - 0.5),
            ],
            egui::Stroke::new(1.0, chrome::colour(sem.borders.divider)),
        );
        if let Some(channel) = clicked {
            self.activate(channel);
        }
        if let Some(open) = self.active {
            let cell = cells[open.index()];
            let bar = egui::Rect::from_min_max(
                egui::pos2(cell.left(), cell.bottom() - control::TAB_BAR_WIDTH),
                cell.right_bottom(),
            );
            painter.rect_filled(bar, 0.0, chrome::colour(channel::hue(open, mode)));
        }

        BandDrawn {
            rect: band,
            cells,
            clicked,
        }
    }
}

/// What one cell says.
struct CellWords {
    word: &'static str,
    value: String,
    previewed: bool,
    empty: bool,
    /// Whether any of the channel's settings rows is the analyst's, which the
    /// cell marks with a dot after its word.
    set: bool,
    /// The scale's name, where the channel's scale is not linear, which the cell
    /// reads after its word.
    scale: Option<String>,
}

/// Paint one cell's contents: the keycap, the channel's word over the column's
/// name, the chevron on the word's line and, where it fits, *preview*. The word
/// reads `x · log` while the scale is not linear, and a dot follows it while a
/// value of the channel is set.
fn paint_words(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    cell: egui::Rect,
    channel: ShelfChannel,
    words: &CellWords,
    mode: Mode,
) {
    let sem = semantic(mode.is_dark());
    let muted = chrome::colour(sem.text.muted);
    let value_ink = chrome::colour(if words.empty {
        sem.text.muted
    } else if words.previewed {
        sem.text.secondary
    } else {
        sem.text.primary
    });

    // The keycap, centred on the band's midline.
    let inner = egui::Rect::from_min_max(
        egui::pos2(cell.left() + PAD, cell.top()),
        egui::pos2(cell.right() - PAD, cell.bottom()),
    );
    let mut keycap = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let chip = key_chip(&mut keycap, cell_key(channel)).rect;
    let text_left = chip.right() + KEY_GAP;

    let label_font = egui::FontId::monospace(typography::UI_SIZE - 2.5);
    let value_font = ui_font();
    let probe = |font: &egui::FontId| {
        painter
            .layout_no_wrap("x".to_owned(), font.clone(), muted)
            .size()
            .y
    };
    let (label_h, value_h) = (probe(&label_font), probe(&value_font));
    let block_top = cell.center().y - (label_h + LINE_GAP + value_h) / 2.0;
    let label_cy = block_top + label_h / 2.0;

    // The chevron, on the word's line at the cell's trailing edge.
    let chevron = egui::Rect::from_center_size(
        egui::pos2(cell.right() - PAD - control::ICON_SM / 2.0, label_cy),
        egui::vec2(control::ICON_SM, control::ICON_SM),
    );
    icons::CHEVRON_DOWN.paint(painter, chevron, muted);
    let label_right = chevron.left() - spacing::SPACE_2;

    // The channel's word, in a text ink and not in the channel's hue, and the
    // scale's name after it where the scale is not linear.
    let room = (label_right - text_left).max(0.0);
    let said = match &words.scale {
        Some(scale) => format!("{} \u{b7} {scale}", words.word),
        None => words.word.to_owned(),
    };
    let word = text_ink::fit(painter, &said, label_font.clone(), room, muted);
    let word_width = word.size().x;
    painter.galley(egui::pos2(text_left, block_top), word, muted);

    // The dot of a value set, where the word leaves the room for it.
    let mut label_used = text_left + word_width;
    if words.set {
        let centre = label_used + spacing::SPACE_3 + MARKER_RADIUS;
        if centre + MARKER_RADIUS <= label_right {
            let dot = chrome::colour(sem.text.primary);
            painter.circle_filled(egui::pos2(centre, label_cy), MARKER_RADIUS, dot);
            label_used = centre + MARKER_RADIUS;
        }
    }

    // *preview*, where the word and the dot leave the room for it.
    if words.previewed {
        let preview = painter.layout_no_wrap(PREVIEW.to_owned(), label_font, muted);
        let at = label_right - preview.size().x;
        if at >= label_used + spacing::SPACE_3 {
            painter.galley(egui::pos2(at, block_top), preview, muted);
        }
    }

    // The column's name, cut with an ellipsis where the cell has no more room.
    let room = (cell.right() - PAD - text_left).max(0.0);
    let value = text_ink::fit(painter, &words.value, value_font, room, value_ink);
    painter.galley(
        egui::pos2(text_left, block_top + label_h + LINE_GAP),
        value,
        value_ink,
    );
}

/// The key a cell prints: the registry's binding, in its Shelf context, for the
/// verb that goes to the cell.
#[must_use]
pub fn cell_key(channel: ShelfChannel) -> &'static str {
    let verb = match channel {
        ShelfChannel::Mark => "go-to-mark-cell",
        ShelfChannel::X => "go-to-x-cell",
        ShelfChannel::Y => "go-to-y-cell",
        ShelfChannel::Colour => "go-to-colour-cell",
    };
    shelf_keys()
        .rows()
        .iter()
        .find(|b| b.context == BindingContext::Shelf && b.longname == verb)
        .map_or("?", |b| b.keystrokes)
}

/// The registry's bindings, projected into the table that resolves a keystroke
/// in a dispatch context. Built once: the registry is data.
fn shelf_keys() -> &'static ResolutionTable {
    static TABLE: OnceLock<ResolutionTable> = OnceLock::new();
    TABLE.get_or_init(|| resolution_table(&keymap_bindings(&registry())))
}

/// The registry's keystroke for an egui key the Shelf context binds, with no
/// modifier held: the band's and the list's.
fn key_token(key: egui::Key) -> Option<&'static str> {
    use egui::Key;
    Some(match key {
        Key::M => "m",
        Key::X => "x",
        Key::Y => "y",
        Key::C => "c",
        Key::H => "h",
        Key::J => "j",
        Key::K => "k",
        Key::L => "l",
        Key::U => "u",
        Key::Slash => "/",
        Key::ArrowLeft => "left",
        Key::ArrowRight => "right",
        Key::ArrowUp => "up",
        Key::ArrowDown => "down",
        Key::Enter => "enter",
        Key::Escape => "escape",
        Key::Tab => "tab",
        Key::Backspace => "backspace",
        _ => return None,
    })
}

/// The registry's keystroke for a key with the command held, which the Shelf
/// context binds where a bare letter would be typed: `cmd-z`.
fn chord_token(key: egui::Key, modifiers: egui::Modifiers) -> Option<&'static str> {
    (key == egui::Key::Z && modifiers.command && !modifiers.shift && !modifiers.alt)
        .then_some("cmd-z")
}

/// Whether `key`, with `modifiers` held, is the registry's `undo` in the Shelf
/// context: `u`, or `⌘Z`. The band's keys and the list's both resolve it, so a
/// key moved in the registry moves here.
#[must_use]
pub fn undoes(key: egui::Key, modifiers: egui::Modifiers) -> bool {
    let token = if modifiers.is_none() {
        key_token(key)
    } else {
        chord_token(key, modifiers)
    };
    token.is_some_and(|t| {
        shelf_keys()
            .resolves(t, DispatchContext::ShelfFocused)
            .contains(&UNDO)
    })
}

/// The registry's verb that takes back the last kept column.
pub const UNDO: &str = "undo";

// ---------------------------------------------------------------------------
// The settings list: an axis's rows, read from the plot.
// ---------------------------------------------------------------------------

/// The word a row carries while its value is brightfield's own.
pub const AUTO: &str = "auto";

/// What a title row reads where the axis draws no title: the file suppresses
/// it, or the channel holds no column to name it from.
pub const NO_TITLE: &str = "none";

/// The name of the title row.
pub const TITLE_ROW: &str = "title";

/// The name of the scale row.
pub const SCALE_ROW: &str = "scale";

/// The name of the format row.
pub const FORMAT_ROW: &str = "format";

/// The name of the range row.
pub const RANGE_ROW: &str = "range";

/// The foot's sentence for the range row.
const RANGE_SAYS: &str = "The two numbers the axis runs between, which auto leaves to the rows.";

/// Why the range row does not apply to an axis of names.
const RANGE_NAMES: &str = "an axis of names has no ends to set";

/// Why the range row does not apply to an axis of dates, whose reader is not cut.
const RANGE_DATES: &str = "a range on a date axis is not read yet";

/// What the refusal of a range through zero on a log axis begins with.
pub const LOG_CANNOT: &str = "a log axis cannot include zero or cross it";

/// **The two ends of a range**, low first and finite by construction, so the
/// equality the row model derives is total.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ends {
    /// The low end.
    pub lo: f64,
    /// The high end, above the low.
    pub hi: f64,
}

// `Ends::new` admits finite numbers alone, so no end is NaN and `==` is reflexive.
impl Eq for Ends {}

impl Ends {
    /// The ends `lo` and `hi`, where both are finite and the low is below the high.
    #[must_use]
    pub fn new(lo: f64, hi: f64) -> Option<Self> {
        (lo.is_finite() && hi.is_finite() && lo < hi).then_some(Self { lo, hi })
    }
}

/// An end as the row writes it: no trailing zeros, and no exponent.
#[must_use]
pub fn end_text(n: f64) -> String {
    let fixed = format!("{n:.10}");
    let trimmed = fixed.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" || trimmed == "-0" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// The two ends as the row reads them: `0 \u{2013} 100`.
fn ends_text(ends: Ends) -> String {
    format!("{} \u{2013} {}", end_text(ends.lo), end_text(ends.hi))
}

/// **What a range field holds, read as an end**: a finite number, or the sentence
/// that says why the row refuses it.
///
/// # Errors
///
/// The sentence the row prints under itself.
pub fn end_number(text: &str) -> Result<f64, String> {
    let typed = text.trim();
    typed
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .ok_or_else(|| {
            let takes = "a range takes numbers";
            if typed.is_empty() {
                return takes.to_string();
            }
            let quoted: String = typed.chars().take(QUOTED_AT_MOST).collect();
            let more = if typed.chars().count() > QUOTED_AT_MOST {
                "\u{2026}"
            } else {
                ""
            };
            format!("{takes}, and \"{quoted}{more}\" is not one")
        })
}

/// The sentence for a low end a log axis cannot draw.
fn log_refusal(lo: f64) -> String {
    format!("{LOG_CANNOT}, and {} is not above zero", end_text(lo))
}

/// What only the range row knows: the ends the axis was drawn over, the ends the
/// file sets, and whether the axis is a log scale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeFacts {
    /// The plot attribute the row writes: `xDomain`.
    pub key: &'static str,
    /// The ends the axis was drawn over, where a scale has been drawn that has
    /// two numbers for them.
    pub drawn: Option<Ends>,
    /// The ends the file sets for the axis.
    pub set: Option<Ends>,
    /// Whether the axis is a log scale, which takes no ends through zero.
    pub log: bool,
}

impl RangeFacts {
    /// Why the ends the file sets cannot be drawn, where the axis is a log scale
    /// and they reach zero: the row says so and the axis runs over its rows.
    #[must_use]
    pub fn flag(&self) -> Option<String> {
        let set = self.set?;
        (self.log && log_ends_refused(set.lo))
            .then(|| format!("{LOG_CANNOT}, so {} is not drawn", ends_text(set)))
    }
}

/// The two numbers an axis was drawn over, for a scale that has them.
fn drawn_ends(scale: &Scale) -> Option<Ends> {
    match scale {
        Scale::Linear {
            domain_min,
            domain_max,
            ..
        }
        | Scale::Log {
            domain_min,
            domain_max,
            ..
        }
        | Scale::Symlog {
            domain_min,
            domain_max,
            ..
        } => Ends::new(*domain_min, *domain_max),
        _ => None,
    }
}

/// The name of the ticks row, which is found by typing it.
pub const TICKS_ROW: &str = "ticks";

/// The name of the grid row, which is found by typing it.
pub const GRID_ROW: &str = "grid";

/// The name of the zero row, which is found by typing it.
pub const ZERO_ROW: &str = "zero";

/// The name of the reverse row, which is found by typing it.
pub const REVERSE_ROW: &str = "reverse";

/// What a switch row reads while the key is on.
pub const ON: &str = "on";

/// What a switch row reads while the key is off.
pub const OFF: &str = "off";

/// The foot's sentence for the ticks row.
const TICKS_SAYS: &str = "About how many ticks the axis draws, which auto leaves to its own step.";

/// The foot's sentence for the grid row.
const GRID_SAYS: &str = "Whether a line crosses the plot at each tick, which auto draws.";

/// The foot's sentence for the zero row.
const ZERO_SAYS: &str = "Whether the axis reaches zero, which auto leaves to the data.";

/// The foot's sentence for the reverse row.
const REVERSE_SAYS: &str = "Whether the axis runs from high to low, which auto runs low to high.";

/// The foot's sentence for the title row: what it does, and the rule for its
/// default.
const TITLE_SAYS: &str = "The words along the axis, which auto takes from the column's name.";

/// What the title row's field says when `Enter` finds no text typed in it.
pub const TITLE_NEEDS_TEXT: &str = "a title needs text";

/// The longest stretch of a refused count the sentence quotes back.
const QUOTED_AT_MOST: usize = 16;

/// **What the ticks row's field holds, read as a count**: a whole number the
/// spec's own reader takes as a target
/// ([`brightfield_spec::layout::tick_count_target`]), or the sentence that says
/// why the row refuses it.
///
/// The judge is the reader's, not a range typed here, so a count the field keeps
/// is a count the plot draws at: `0`, `2.5`, `abc` and a count past
/// [`MAX_TICK_COUNT`] are refused for the one reason, that the axis cannot aim its
/// ticks at them, and `the_field_and_the_reader_agree_over_what_is_a_count` asks
/// both sides about each count from zero past the ceiling.
///
/// # Errors
///
/// The sentence the row prints under itself.
pub fn ticks_count(text: &str) -> Result<usize, String> {
    let typed = text.trim();
    let count = typed.parse::<usize>().ok().filter(|n| {
        i64::try_from(*n).is_ok_and(|n| tick_count_target(&SpecValue::Integer(n)).is_some())
    });
    count.ok_or_else(|| {
        let takes = format!("ticks takes a whole number from 1 to {MAX_TICK_COUNT}");
        if typed.is_empty() {
            return takes;
        }
        let quoted: String = typed.chars().take(QUOTED_AT_MOST).collect();
        let more = if typed.chars().count() > QUOTED_AT_MOST {
            "\u{2026}"
        } else {
            ""
        };
        format!("{takes}, and \"{quoted}{more}\" is not one")
    })
}

/// The foot's sentence for the scale row.
const SCALE_SAYS: &str = "How values are spaced along the axis, which auto draws linear.";

/// The foot's sentence for the scale row of an axis of names, which the chart
/// draws as a band scale.
const SCALE_SAYS_BAND: &str =
    "How values are spaced along the axis, which auto draws band for names.";

/// The foot's sentence for the scale row of an axis of dates, which the chart
/// draws as a time scale.
const SCALE_SAYS_TIME: &str =
    "How values are spaced along the axis, which auto draws time for dates.";

/// What the scale row reads on an axis of names, which the chart draws as a
/// band scale.
pub const BAND_SCALE: &str = "band";

/// What the scale row reads on an axis of dates, which the chart draws as a time
/// scale.
pub const TIME_SCALE: &str = "time";

/// The foot's sentence for the format row.
const FORMAT_SAYS: &str =
    "How a tick's number or date is written, which auto leaves to the axis's own tick text.";

/// What the format row reads while its specifier is no preset's: the one the
/// field holds, or a file's.
pub const CUSTOM_FORMAT: &str = "custom";

/// The presets the format row steps through, in the order `l` steps them, and
/// the specifier each writes. `auto` writes none: it takes the key out. Custom
/// is last and has no specifier of its own: it is the field's.
///
/// Number writes `,f` and currency `$,f`, because with no type d3-format takes
/// one significant digit from the step and a population axis would print
/// `1e+4`; short is `~s` and percent `%`.
pub const FORMAT_PRESETS: [(&str, Option<&str>); 6] = [
    (AUTO, None),
    ("number", Some(",f")),
    ("short", Some("~s")),
    ("percent", Some("%")),
    ("currency", Some("$,f")),
    (CUSTOM_FORMAT, None),
];

/// The preset a specifier written to the file reads as: its name where a preset
/// writes exactly it, and custom where none does.
#[must_use]
pub fn format_preset(specifier: &str) -> &'static str {
    FORMAT_PRESETS
        .iter()
        .find(|(_, writes)| *writes == Some(specifier))
        .map_or(CUSTOM_FORMAT, |(name, _)| *name)
}

/// The names of the presets, as the foot lists them: `auto, number, short, ...`.
fn format_preset_names() -> String {
    FORMAT_PRESETS
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The letters a specifier's type may be, spaced as a sentence names them.
fn type_letters() -> String {
    NumberFormat::TYPE_LETTERS
        .chars()
        .map(String::from)
        .collect::<Vec<_>>()
        .join(" ")
}

/// What the format row's field says under the row while it is open.
const FORMAT_FIELD_SAYS: &str =
    "A d3-format specifier of your own, such as $,.2f or ~s. Enter keeps it.";

/// **What the format row's field holds, read as a specifier**: the text as d3-format
/// reads it, or the sentence that says why the row refuses it.
///
/// The judge is the spec's own, so a specifier the field keeps is one the axis
/// draws: [`NumberFormat::parse`] says whether the text is a specifier, and
/// [`NumberFormat::names_its_type`] whether its type is a letter d3-format names,
/// which the reader does not ask because it draws an unknown letter as `.12~g`.
/// `the_field_refuses_what_the_reader_takes_exactly_at_a_type_no_format_names`
/// asks both about each letter.
///
/// # Errors
///
/// The sentence the row prints under itself.
pub fn format_specifier(text: &str) -> Result<String, String> {
    let letters = type_letters();
    let takes = format!("a format takes a d3-format specifier, whose type is one of {letters}");
    if text.is_empty() {
        return Err(takes);
    }
    let quoted: String = text.chars().take(QUOTED_AT_MOST).collect();
    let more = if text.chars().count() > QUOTED_AT_MOST {
        "\u{2026}"
    } else {
        ""
    };
    match NumberFormat::parse(text) {
        None => Err(format!("\"{quoted}{more}\" is not a specifier; {takes}")),
        Some(format) if !format.names_its_type() => Err(format!(
            "\"{quoted}{more}\" ends in a type d3-format does not name; it takes {letters}"
        )),
        Some(_) => Ok(text.to_string()),
    }
}

/// The plot attribute each axis's tick format is written under, which
/// [`brightfield_spec::layout::resolve_tick_formats`] reads. The row reads the
/// same key through the same judge, and
/// `a_format_row_agrees_with_the_reader_over_what_is_a_format` holds the two
/// together.
const TICK_FORMAT_KEYS: [(ShelfChannel, &str); 2] = [
    (ShelfChannel::X, "xTickFormat"),
    (ShelfChannel::Y, "yTickFormat"),
];

/// How a settings row takes a value, which the cards that set one read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKind {
    /// One of a short list, stepped through by `h` and `l`: scale, format.
    Enumerated,
    /// Text typed into the row, in a field `Enter` opens: title and ticks. The
    /// format is enumerated and opens the same field behind its presets.
    Typed,
}

/// What a settings row is set to by a step or a `⌫`, in the terms the row reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingValue {
    /// A scale's wire name: `log`.
    Word(String),
    /// A switch: grid, zero or reverse.
    Switch(bool),
    /// A count of ticks.
    Count(usize),
    /// Text typed into a row: the title's words.
    Text(String),
    /// The two ends of a range, low first.
    Ends(Ends),
    /// Back to brightfield's own: the key comes out of the file.
    Auto,
}

/// One write a settings row asked for: which row of which axis, and to what.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowEdit {
    /// The axis the row belongs to.
    pub channel: ShelfChannel,
    /// The row's name: `scale`.
    pub row: &'static str,
    /// What the row is set to.
    pub value: SettingValue,
}

/// The scale names a scale row steps through, in the order `l` steps them.
pub const SCALE_STEPS: [&str; 3] = ["linear", "log", "symlog"];

/// The plot attribute `row` of `axis` is written under, as Mosaic spells it, for
/// the rows a step, a typed value or a `⌫` writes: title, scale, format, ticks,
/// grid, zero and reverse.
#[must_use]
pub fn row_key(axis: ShelfChannel, row: &str) -> Option<&'static str> {
    let x = match axis {
        ShelfChannel::X => true,
        ShelfChannel::Y => false,
        ShelfChannel::Mark | ShelfChannel::Colour => return None,
    };
    Some(match (row, x) {
        (TITLE_ROW, true) => "xLabel",
        (TITLE_ROW, false) => "yLabel",
        (FORMAT_ROW, true) => "xTickFormat",
        (FORMAT_ROW, false) => "yTickFormat",
        (SCALE_ROW, true) => "xScale",
        (SCALE_ROW, false) => "yScale",
        (RANGE_ROW, true) => "xDomain",
        (RANGE_ROW, false) => "yDomain",
        (TICKS_ROW, true) => "xTicks",
        (TICKS_ROW, false) => "yTicks",
        (GRID_ROW, true) => "xGrid",
        (GRID_ROW, false) => "yGrid",
        (ZERO_ROW, true) => "xZero",
        (ZERO_ROW, false) => "yZero",
        (REVERSE_ROW, true) => "xReverse",
        (REVERSE_ROW, false) => "yReverse",
        _ => return None,
    })
}

/// brightfield's own value for `row`, as the plot attribute holds it: the value
/// a row reads *auto* at. `None` for a row with no attribute of the kind a step
/// or a `⌫` writes.
#[must_use]
pub fn row_default(row: &str) -> Option<SpecValue> {
    Some(match row {
        SCALE_ROW => SpecValue::String("linear".to_string()),
        // Mosaic's own, no format: the axis draws its own tick text.
        FORMAT_ROW | RANGE_ROW => SpecValue::Null,
        TICKS_ROW => SpecValue::Integer(i64::try_from(DEFAULT_TICK_COUNT).unwrap_or(5)),
        GRID_ROW => SpecValue::Bool(true),
        ZERO_ROW | REVERSE_ROW => SpecValue::Bool(false),
        _ => return None,
    })
}

/// What a step of `h` or `l` does to a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowStep {
    /// Write this value.
    Set(SettingValue),
    /// Open the row's field on the specifier it reads: the step landed on the
    /// format's custom, which has no value of its own to write.
    Field,
}

impl SettingRow {
    /// Whether `h` and `l` step this row's value: a scale, a format or a switch
    /// that applies to the axis. The ticks and the title take typed text, which
    /// `Enter` opens a field for ([`RowField`]), and so does the format, behind its
    /// presets. A scale row that reads band or
    /// time does not step: the chart draws those two for names and dates, and
    /// the three it steps through are for numbers.
    #[must_use]
    pub fn steps(&self) -> bool {
        self.reason.is_none()
            && match self.name {
                SCALE_ROW => SCALE_STEPS.contains(&self.value.as_str()),
                FORMAT_ROW | GRID_ROW | ZERO_ROW | REVERSE_ROW => true,
                _ => false,
            }
    }

    /// Whether `Enter` opens a field on this row: the rows that take typed text
    /// and the format, which takes it behind its presets.
    #[must_use]
    pub fn takes_field(&self) -> bool {
        self.reason.is_none() && (self.kind == SettingKind::Typed || self.name == FORMAT_ROW)
    }

    /// What one step `by` from the row's own does, or `None` where the row does
    /// not step or stands at the end of its values. The format steps through its
    /// presets: the first takes the key out, the next four write their specifier,
    /// and a step that lands on custom opens the field.
    #[must_use]
    pub fn step_to(&self, by: isize) -> Option<RowStep> {
        if self.name != FORMAT_ROW {
            return self.stepped(by).map(RowStep::Set);
        }
        if !self.steps() {
            return None;
        }
        let at = FORMAT_PRESETS
            .iter()
            .position(|(name, _)| *name == self.value)?;
        let next = at
            .checked_add_signed(by)
            .filter(|n| *n < FORMAT_PRESETS.len())?;
        Some(match FORMAT_PRESETS[next] {
            (_, Some(writes)) => RowStep::Set(SettingValue::Text(writes.to_string())),
            (CUSTOM_FORMAT, None) => RowStep::Field,
            (_, None) => RowStep::Set(SettingValue::Auto),
        })
    }

    /// The value one step `by` from the row's own, or `None` where the row does
    /// not step or stands at the end of its values. A switch turns over.
    #[must_use]
    pub fn stepped(&self, by: isize) -> Option<SettingValue> {
        if !self.steps() || self.name == FORMAT_ROW {
            return None;
        }
        if self.name == SCALE_ROW {
            let at = SCALE_STEPS.iter().position(|w| *w == self.value)?;
            let next = at
                .checked_add_signed(by)
                .filter(|n| *n < SCALE_STEPS.len())?;
            return Some(SettingValue::Word(SCALE_STEPS[next].to_string()));
        }
        Some(SettingValue::Switch(self.value != ON))
    }
}

/// One row of a channel's settings list: what it is called, what it reads, and
/// whether the value is the analyst's.
///
/// **The row model every settings card inherits.** A row has a name, a value
/// text, whether it is set, a reason when it does not apply, and a kind. A
/// value is *set* when it differs from brightfield's own, by value and not by
/// whether the file wrote it, so `yScale: linear` is written and reads *auto*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingRow {
    /// The row's name: `title`.
    pub name: &'static str,
    /// What the row reads: the value as the analyst would say it.
    pub value: String,
    /// Whether the value differs from brightfield's own.
    pub set: bool,
    /// Why the row does not apply to the chart as it is, where it does not.
    /// The judges are the render crate's own and are asked, not re-derived:
    /// `axis_ends_apply` for zero, `tick_count_applies` for ticks,
    /// `axis_reverse_applies` for reverse, and `axis_keys_apply` for ticks, grid
    /// and zero under a map projection. The three head rows apply to every axis,
    /// so none carries one.
    pub reason: Option<String>,
    /// Whether the row is listed only where the query names it: ticks, grid,
    /// zero and reverse. A head row is listed with no query as well.
    pub by_name: bool,
    /// How the row takes a value.
    pub kind: SettingKind,
    /// The one sentence the list's foot reads under the cursor: what the row
    /// does, and the rule for its default.
    pub says: &'static str,
    /// The key the value is read from where the axis's own key is absent and a
    /// key for both axes stands in for it: `grid` for a row of the grid. The
    /// foot says so, and `⌫` on the row cannot take it out, since it is not the
    /// axis's own.
    pub from: Option<&'static str>,
    /// What only the format row knows: the key it writes, the specifier the file
    /// holds, and the sample. `None` on the title, ticks, grid, zero and reverse
    /// rows and on the scale row.
    pub format: Option<FormatFacts>,
    /// What only the range row knows. `None` on the title, scale, format, ticks,
    /// grid, zero and reverse rows.
    pub range: Option<RangeFacts>,
}

/// What the format row carries beyond its value, read from the plot and the scale
/// the chart was drawn against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatFacts {
    /// The plot attribute the row writes: `xTickFormat`.
    pub key: &'static str,
    /// The specifier the file holds for the axis, where it names a format the
    /// axis reads. The row's value is the preset it equals, or custom.
    pub specifier: Option<String>,
    /// What the axis's largest drawn tick prints as under the row's format, drawn
    /// in muted ink beside the value. `None` where no scale has been drawn, or the
    /// axis draws no text a number format sets.
    pub sample: Option<String>,
}

/// The settings rows of the channels that have any: x and y. Colour's and the
/// mark's are not built, so they have no rows, and `Tab` leaves their list on
/// its columns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelSettings {
    x: Vec<SettingRow>,
    y: Vec<SettingRow>,
}

impl ChannelSettings {
    /// The rows of `plot`'s two axes, with `channels` saying what each axis
    /// holds and `spec` the params a lifted scale name resolves through.
    ///
    /// Each value is the resolved one: the title is the resolver's
    /// override, suppression or derivation, the scale is the type the plot
    /// resolves to (so a name this build cannot draw reads as the linear it is
    /// drawn as), and the format is the one the judge reads.
    ///
    /// **No scale has been drawn here, so no judge speaks**: a by-name row
    /// carries no reason, and an axis of names or dates reads the type the plot
    /// resolves to. A window hands the scales its chart was drawn against to
    /// [`Self::of_plot_drawn`], which reads such an axis as band or time.
    #[must_use]
    pub fn of_plot(spec: &Spec, plot: &PlotNode, channels: &ShelfChannels) -> Self {
        Self::of_plot_drawn(spec, plot, channels, &ScaleSet::new())
    }

    /// [`Self::of_plot`] with the scales `plot` was drawn against, which the
    /// render crate's judges are asked of: a row that does not apply to the axis
    /// those scales draw carries the reason.
    #[must_use]
    pub fn of_plot_drawn(
        spec: &Spec,
        plot: &PlotNode,
        channels: &ShelfChannels,
        drawn: &ScaleSet,
    ) -> Self {
        let titles = resolve_axis_titles(plot);
        let scales = resolve_plot_scales_in(plot, &spec.params);
        let domains = read_domains_in(plot, &spec.params);
        Self {
            x: axis_rows(
                plot,
                ShelfChannel::X,
                &titles.x,
                scales.x,
                &channels.x,
                drawn,
                &domains.x,
            ),
            y: axis_rows(
                plot,
                ShelfChannel::Y,
                &titles.y,
                scales.y,
                &channels.y,
                drawn,
                &domains.y,
            ),
        }
    }

    /// `channel`'s rows, in the order the list draws them. Empty for a channel
    /// with no settings list.
    #[must_use]
    pub fn rows(&self, channel: ShelfChannel) -> &[SettingRow] {
        match channel {
            ShelfChannel::X => &self.x,
            ShelfChannel::Y => &self.y,
            ShelfChannel::Mark | ShelfChannel::Colour => &[],
        }
    }
}

/// The rows of one axis: the head rows, title, scale and format, in that order,
/// and the four found by name, ticks, grid, zero and reverse, behind them.
fn axis_rows(
    plot: &PlotNode,
    axis: ShelfChannel,
    title: &AxisTitle,
    scale: ScaleType,
    binding: &Binding,
    drawn: &ScaleSet,
    domain: &DomainReading,
) -> Vec<SettingRow> {
    // brightfield's own title is the name of the column the axis holds.
    let derived = match binding {
        Binding::Column(name) => Some(name.as_str()),
        Binding::Unset | Binding::Expression => None,
    };
    let (title_value, title_set) = match title {
        AxisTitle::Derive => (derived.unwrap_or(NO_TITLE).to_string(), false),
        AxisTitle::Override(text) => (text.clone(), derived != Some(text.as_str())),
        AxisTitle::Suppress => (NO_TITLE.to_string(), derived.is_some()),
    };
    let format_key = TICK_FORMAT_KEYS
        .iter()
        .find(|(channel, _)| *channel == axis)
        .map(|(_, key)| *key);
    let written = format_key.and_then(|key| plot.attributes.get(key));
    // The format the file names and the axis reads, and the specifier it names
    // it by: a preset's name where a preset writes exactly it, and custom where
    // none does, a date format included.
    let (format_value, format_set, format_read, specifier) =
        match (written.map(read_tick_format), written) {
            (Some(TickFormatReading::Format(read)), Some(SpecValue::String(text))) => (
                format_preset(text).to_string(),
                true,
                Some(read),
                Some(text.clone()),
            ),
            _ => (AUTO.to_string(), false, None, None),
        };
    let row = |name, value, set, kind, says| SettingRow {
        name,
        value,
        set,
        reason: None,
        by_name: false,
        kind,
        says,
        from: None,
        format: None,
        range: None,
    };
    // The scale the chart draws: band for names and time for dates, which are
    // brightfield's own choice and so leave the row unset, and otherwise the
    // type the plot resolves to.
    let channel = if axis == ShelfChannel::X {
        Channel::X
    } else {
        Channel::Y
    };
    let (scale_value, scale_says) = match drawn.get(channel) {
        Some(Scale::Band { .. }) => (BAND_SCALE, SCALE_SAYS_BAND),
        Some(Scale::Time { .. }) => (TIME_SCALE, SCALE_SAYS_TIME),
        _ => (scale.wire_name(), SCALE_SAYS),
    };
    // The range: the file's two ends, or the ends the axis was drawn over, which
    // an axis of names, an axis of dates and a map do not take.
    let scale_drawn = drawn.get(channel);
    let set_ends = match domain {
        DomainReading::Ends { lo, hi, .. } => Ends::new(*lo, *hi),
        _ => None,
    };
    let range_from = match domain {
        DomainReading::Ends {
            key: "xyDomain", ..
        } => Some("xyDomain"),
        _ => None,
    };
    let range_reason = if axis_keys_apply(drawn) {
        match scale_drawn {
            Some(named @ Scale::Band { .. })
                if matches!(axis_kind(named), Some(AxisKind::Date)) =>
            {
                Some(RANGE_DATES.to_string())
            }
            Some(Scale::Band { .. }) => Some(RANGE_NAMES.to_string()),
            Some(Scale::Time { .. }) => Some(RANGE_DATES.to_string()),
            _ => None,
        }
    } else {
        Some(PROJECTED.to_string())
    };
    let drawn_over = scale_drawn.and_then(drawn_ends);
    let range_row = SettingRow {
        reason: range_reason,
        from: range_from,
        range: Some(RangeFacts {
            key: row_key(axis, RANGE_ROW).unwrap_or("xDomain"),
            drawn: drawn_over,
            set: set_ends,
            log: matches!(scale_drawn, Some(Scale::Log { .. }))
                || (scale_drawn.is_none() && scale == ScaleType::Log),
        }),
        ..row(
            RANGE_ROW,
            set_ends.or(drawn_over).map_or_else(String::new, ends_text),
            set_ends.is_some(),
            SettingKind::Typed,
            RANGE_SAYS,
        )
    };
    let mut rows = vec![
        row(
            TITLE_ROW,
            title_value,
            title_set,
            SettingKind::Typed,
            TITLE_SAYS,
        ),
        row(
            SCALE_ROW,
            scale_value.to_string(),
            scale != ScaleType::Linear,
            SettingKind::Enumerated,
            scale_says,
        ),
        range_row,
        SettingRow {
            format: format_key.map(|key| FormatFacts {
                key,
                specifier,
                // The axis's own top tick, which the same call the chart was
                // drawn through reads off the scale it was drawn against: no
                // scale drawn, no sample.
                sample: drawn.get(channel).and_then(|scale| {
                    let target = resolve_tick_counts(plot);
                    let target = if axis == ShelfChannel::X {
                        target.x_target()
                    } else {
                        target.y_target()
                    };
                    top_tick_text(scale, target, format_read.as_ref())
                }),
            }),
            ..row(
                FORMAT_ROW,
                format_value,
                format_set,
                SettingKind::Enumerated,
                FORMAT_SAYS,
            )
        },
    ];
    rows.extend(by_name_rows(plot, axis, drawn));
    rows
}

/// The rows found by name on one axis: ticks, grid, zero and reverse, each
/// reading *auto* or the value the plot sets, and carrying the reason where the
/// render crate's judge says the key does not apply to the axis `drawn` holds.
fn by_name_rows(plot: &PlotNode, axis: ShelfChannel, drawn: &ScaleSet) -> [SettingRow; 4] {
    let x = axis == ShelfChannel::X;
    let channel = if x { Channel::X } else { Channel::Y };
    let scale = drawn.get(channel);
    let projected = !axis_keys_apply(drawn);

    let ticks = {
        let asked = resolve_tick_counts(plot);
        if x {
            asked.x
        } else {
            asked.y
        }
    };
    let grid = {
        let lines = resolve_grid_lines(plot);
        if x {
            lines.x
        } else {
            lines.y
        }
    };
    let zero = {
        let ends = resolve_axis_ends(plot);
        if x {
            ends.x.zero
        } else {
            ends.y.zero
        }
    };
    let reverse = {
        let turned = resolve_axis_reverse(plot);
        if x {
            turned.x
        } else {
            turned.y
        }
    };
    let word = |on: bool| if on { ON } else { OFF }.to_string();
    let row = |name, value, set, kind, says, reason| SettingRow {
        name,
        value,
        set,
        reason,
        by_name: true,
        kind,
        says,
        from: None,
        format: None,
        range: None,
    };
    let grid_key = if x { "xGrid" } else { "yGrid" };
    let grid_from = (!plot.attributes.contains_key(grid_key)
        && plot.attributes.contains_key("grid"))
    .then_some("grid");

    let ticks_reason = if projected {
        Some(PROJECTED.to_string())
    } else {
        scale
            .filter(|scale| !tick_count_applies(scale))
            .map(ticks_reason)
    };
    let zero_reason = if projected {
        Some(PROJECTED.to_string())
    } else {
        scale
            .filter(|scale| !axis_ends_apply(scale))
            .map(zero_reason)
    };
    [
        row(
            TICKS_ROW,
            ticks.unwrap_or(DEFAULT_TICK_COUNT).to_string(),
            ticks.is_some_and(|count| count != DEFAULT_TICK_COUNT),
            SettingKind::Typed,
            TICKS_SAYS,
            ticks_reason,
        ),
        SettingRow {
            from: grid_from,
            format: None,
            ..row(
                GRID_ROW,
                word(grid),
                !grid,
                SettingKind::Enumerated,
                GRID_SAYS,
                projected.then(|| PROJECTED.to_string()),
            )
        },
        row(
            ZERO_ROW,
            word(zero),
            zero,
            SettingKind::Enumerated,
            ZERO_SAYS,
            zero_reason,
        ),
        row(
            REVERSE_ROW,
            word(reverse),
            reverse,
            SettingKind::Enumerated,
            REVERSE_SAYS,
            (!axis_reverse_applies(drawn)).then(|| PROJECTED.to_string()),
        ),
    ]
}

/// The reason a key does not apply under a map projection, which every key of
/// the four shares: the plot's x and y are the projection's, and there is no
/// axis to set.
const PROJECTED: &str = "a map's x and y are its projection, which has no axis to set";

/// What the axis `scale` is, said with its article: the words a reason names the
/// axis it does not apply to by.
fn axis_phrase(scale: &Scale) -> &'static str {
    match scale {
        Scale::Log { .. } => "a log axis",
        Scale::Symlog { .. } => "a symlog axis",
        Scale::Time { .. } => "a time axis",
        Scale::Band { .. } => match axis_kind(scale) {
            Some(AxisKind::Date) => "an axis of days",
            _ => "an axis of names",
        },
        _ => "this axis",
    }
}

/// Why a tick count does not aim the ticks of `scale`, an axis
/// `tick_count_applies` has judged one it does not reach: a band has a tick for
/// each category, and a log or symlog axis one for each decade.
fn ticks_reason(scale: &Scale) -> String {
    let phrase = axis_phrase(scale);
    match scale {
        Scale::Band { .. } => {
            format!("{phrase} has a tick for each, which a count does not change")
        }
        Scale::Log { .. } | Scale::Symlog { .. } => {
            format!("{phrase} ticks at each decade, which a count does not change")
        }
        _ => format!("{phrase} does not tick to a count"),
    }
}

/// Why zero does not move the ends of `scale`, an axis `axis_ends_apply` has
/// judged one it does not: a linear axis's alone.
fn zero_reason(scale: &Scale) -> String {
    format!(
        "only a linear axis reaches zero, and this is {}",
        axis_phrase(scale)
    )
}

// ---------------------------------------------------------------------------
// The column list.
// ---------------------------------------------------------------------------

/// One column of the table, as the list offers it.
#[derive(Clone, Debug, PartialEq)]
pub struct ListColumn {
    /// The column's name.
    pub name: String,
    /// What the row says at its trailing end: the column's type — unless
    /// [`Self::moments`] is held, which the row draws a rug of in its place.
    pub kind: String,
    /// The numbers behind the column's spread, from the same profile the grid
    /// head's rug is drawn from. `Some` for a column the engine defines moments
    /// over, which is a numeric column with a value in it; `None` for any other,
    /// whose row keeps its type.
    pub moments: Option<ColumnMoments>,
}

/// What the window asks the list to open on.
#[derive(Clone, Debug)]
pub struct ColumnListRequest {
    /// The tile the cell belongs to, as the heading names it.
    pub tile: String,
    /// The channel whose list opens.
    pub channel: ShelfChannel,
    /// What each channel of the tile's mark is bound to, so the cursor opens on
    /// the column the channel holds and a switch of channel finds the next one's.
    pub channels: ShelfChannels,
    /// The table's columns, in the table's order.
    pub columns: Vec<ListColumn>,
}

/// What a key or a click decided, for the window to act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListReport {
    /// The cursor moved to this column — by a key, or by the pointer moving
    /// over its row — which the chart draws as a preview.
    Moved(String),
    /// `Enter`, or a click on its row, kept this column.
    Kept(String),
    /// `Esc` with the query empty: the reader backs out of the list.
    BackedOut,
    /// A channel's letter named this channel. The list is on it when it lists
    /// columns, which the mark's cell does not.
    GoTo(ShelfChannel),
    /// `h` or `l` named the channel beside, and the list stays open. The list is
    /// on it when it lists columns.
    Beside(ShelfChannel),
    /// `u`, or `⌘Z` from the query: take back the last kept column, which is
    /// the window's to do, as the kept columns are the window's.
    Undo,
    /// `Tab` turned the list to this tab. A column the cursor was previewing
    /// belongs to the columns, so the window backs it out when the list turns
    /// to the settings; turning back, the list reports the column its cursor
    /// lands on as it does when the query moves it.
    Turned(ListTab),
    /// A settings row was stepped, by `h` `l` or a click on its value, set to a
    /// value typed into its field and kept by `Enter`, or put back to auto by
    /// `⌫`: the window writes it to the plot.
    Set(RowEdit),
    /// A typed row's field holds a value the chart can draw: draw it, without
    /// keeping it. `None` takes the preview back, because the field was dropped,
    /// emptied, or holds a value the row refuses.
    Preview(Option<RowEdit>),
    /// The range row refused a range the axis cannot draw, and the field stays
    /// open: the status band says the sentence.
    Refused(String),
}

/// Which of its two states a channel's list is in: the table's columns, or
/// the channel's settings. A third, what is set on the chart, comes behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListTab {
    /// The table's columns, offered to the channel.
    Columns,
    /// The channel's settings rows.
    Settings,
}

impl ListTab {
    /// The word the tab strip prints for this tab.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Columns => "columns",
            Self::Settings => "settings",
        }
    }
}

/// **A typed settings row's field while it is open**: the title's words or the
/// tick count, being typed.
///
/// The field opens on `Enter` with the row's value selected, so the first thing
/// typed replaces it and `⌫` clears it. While it is open it has the keys, as the
/// query line does: a letter is text, `Enter` keeps the value, `Esc` drops it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowField {
    /// The row the field is on: `title` or `ticks`.
    pub row: &'static str,
    /// What has been typed.
    pub text: String,
    /// Whether the whole text is selected, which the first key typed replaces.
    pub selected: bool,
    /// The sentence the row prints under itself while the field holds a value
    /// the row refuses, in words.
    pub refusal: Option<String>,
    /// The range row's second field. `None` on a row that holds one field.
    pub ends: Option<EndFields>,
}

/// Which end of a range the open field is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// The low end, which opens first.
    Low,
    /// The high end, which `Enter` on the low moves to.
    High,
}

/// **The range row's two fields**: [`RowField::text`] holds the end under the
/// caret, and this holds the text of the other, so both are drawn side by side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndFields {
    /// The end the caret is in.
    pub on: End,
    /// What the other field holds.
    pub other: String,
}

/// The list of a channel's columns, with a query line.
///
/// Two states say who has the keys. While the rows have them, a letter is a
/// verb: `j` `k` move the cursor, `h` `l` and the channel letters go to another
/// channel, `/` gives the keys to the query. While the query has them, a letter
/// is text, and `Enter`, `Esc`, `Backspace` and the up and down arrows act as
/// keys: `a_letter_typed_in_the_query_is_text_and_not_a_verb` holds it.
/// The rows hold no query: leaving the query clears it.
#[derive(Clone, Debug)]
pub struct ColumnList {
    tile: String,
    channel: ShelfChannel,
    channels: ShelfChannels,
    columns: Vec<ListColumn>,
    query: String,
    querying: bool,
    /// The row under the cursor, as an index into `columns`.
    cursor: Option<usize>,
    /// The cursor moved, so the next frame scrolls its row into view.
    scroll: bool,
    /// Which list is showing: the columns, or the channel's settings.
    tab: ListTab,
    /// What the axes' settings read, handed in by the window, which holds the
    /// plot. Empty until [`Self::set_settings`], and then a channel with no
    /// rows has no settings tab.
    settings: ChannelSettings,
    /// The settings row under the cursor, as an index into the channel's rows.
    row: Option<usize>,
    /// The open field of a typed row, which has the keys while it is open.
    field: Option<RowField>,
}

impl ColumnList {
    /// A list open on `request`'s channel, the cursor on the column it holds.
    #[must_use]
    pub fn new(request: ColumnListRequest) -> Self {
        let mut list = Self {
            tile: request.tile,
            channel: request.channel,
            channels: request.channels,
            columns: request.columns,
            query: String::new(),
            querying: false,
            cursor: None,
            scroll: true,
            tab: ListTab::Columns,
            settings: ChannelSettings::default(),
            row: None,
            field: None,
        };
        list.cursor = list.held();
        list
    }

    /// Hand the list what the axes' settings read. The cursor's row stays where
    /// it is, as an index, and moves to the first row if the rows are fewer.
    pub fn set_settings(&mut self, settings: ChannelSettings) {
        self.settings = settings;
        let rows = self.settings.rows(self.channel).len();
        if self.tab == ListTab::Settings && rows == 0 {
            self.tab = ListTab::Columns;
        }
        self.row = self.row.filter(|r| *r < rows);
        if self.tab == ListTab::Settings && self.row.is_none() {
            self.row = self.setting_order().first().copied();
        }
    }

    /// The tab the list is on.
    #[must_use]
    pub fn tab(&self) -> ListTab {
        self.tab
    }

    /// The channel's settings rows, in the channel's order.
    #[must_use]
    pub fn settings(&self) -> &[SettingRow] {
        self.settings.rows(self.channel)
    }

    /// The settings row under the cursor.
    #[must_use]
    pub fn setting_cursor(&self) -> Option<&SettingRow> {
        self.row.and_then(|i| self.settings().get(i))
    }

    /// Whether the list has a settings tab: the channel has rows to show.
    fn has_settings(&self) -> bool {
        !self.settings().is_empty()
    }

    /// The settings rows in the order the list draws them, as indices into
    /// [`Self::settings`]: with no query the channel's head rows in the
    /// channel's order, and with one the rows, found by name or not, whose name
    /// begins with the letters, then those that hold them. A row the query does
    /// not match is not drawn, as it is a name being looked for among a handful
    /// and not a column among a table's.
    fn setting_order(&self) -> Vec<usize> {
        let rows = self.settings();
        let all: Vec<usize> = (0..rows.len()).collect();
        if self.query.is_empty() {
            return all.into_iter().filter(|i| !rows[*i].by_name).collect();
        }
        let named = |i: &usize| rows[*i].name.to_lowercase();
        let mut order: Vec<usize> = all
            .iter()
            .copied()
            .filter(|i| named(i).starts_with(&self.query))
            .collect();
        order.extend(
            all.iter()
                .copied()
                .filter(|i| !named(i).starts_with(&self.query) && named(i).contains(&self.query)),
        );
        order
    }

    /// The channel the list is on.
    #[must_use]
    pub fn channel(&self) -> ShelfChannel {
        self.channel
    }

    /// The tile the heading names.
    #[must_use]
    pub fn tile(&self) -> &str {
        &self.tile
    }

    /// What has been typed.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Whether the query has the keys.
    #[must_use]
    pub fn querying(&self) -> bool {
        self.querying
    }

    /// The open field of a typed settings row, with what has been typed in it.
    #[must_use]
    pub fn field(&self) -> Option<&RowField> {
        self.field.as_ref()
    }

    /// Whether text typed is taken as text: the query has the keys, or a typed
    /// row's field is open. The window leaves every key of such a frame to the
    /// list.
    #[must_use]
    pub fn typing(&self) -> bool {
        self.querying || self.field.is_some()
    }

    /// The channels the plot now binds, after an edit the list did not make
    /// — a column kept taken back. With nothing typed the cursor goes to the
    /// column its channel holds now; a query typed keeps the cursor where the
    /// query put it. Reports nothing: the page is already drawn from the
    /// channels the list was handed.
    pub fn rebind(&mut self, channels: ShelfChannels) {
        self.channels = channels;
        if self.query.is_empty() {
            self.cursor = self.held();
            self.scroll = true;
        }
    }

    /// The column under the cursor.
    #[must_use]
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.map(|i| self.columns[i].name.as_str())
    }

    /// The columns in the order the list draws them: those the query matches,
    /// then the rest. With no query it is the table's order.
    #[must_use]
    pub fn display(&self) -> Vec<&str> {
        self.order()
            .0
            .into_iter()
            .map(|i| self.columns[i].name.as_str())
            .collect()
    }

    /// The index of the column the list's channel holds.
    fn held(&self) -> Option<usize> {
        let Some(Binding::Column(name)) = self.channels.binding(self.channel) else {
            return None;
        };
        self.columns.iter().position(|c| &c.name == name)
    }

    /// The columns in drawing order, and how many of them the query matched.
    ///
    /// A name that begins with the letters leads one that only contains them,
    /// and every column is listed: a column the query does not match is below
    /// the divider, not gone.
    fn order(&self) -> (Vec<usize>, usize) {
        if self.query.is_empty() {
            return ((0..self.columns.len()).collect(), 0);
        }
        let named = |i: &usize| self.columns[*i].name.to_lowercase();
        let all: Vec<usize> = (0..self.columns.len()).collect();
        let mut order: Vec<usize> = all
            .iter()
            .copied()
            .filter(|i| named(i).starts_with(&self.query))
            .collect();
        order.extend(
            all.iter()
                .copied()
                .filter(|i| !named(i).starts_with(&self.query) && named(i).contains(&self.query)),
        );
        let matched = order.len();
        order.extend(
            all.iter()
                .copied()
                .filter(|i| !named(i).contains(&self.query)),
        );
        (order, matched)
    }

    /// Put the cursor on `to`, and report the column when it is a new one.
    fn land(&mut self, to: Option<usize>, out: &mut Vec<ListReport>) {
        if self.cursor == to {
            return;
        }
        self.cursor = to;
        self.scroll = true;
        if let Some(i) = to {
            out.push(ListReport::Moved(self.columns[i].name.clone()));
        }
    }

    /// Move the cursor `by` rows through the rows as drawn, and stop at the ends.
    /// On the settings the cursor moves and nothing is reported: a settings row
    /// has no column to preview.
    fn step(&mut self, by: isize, out: &mut Vec<ListReport>) {
        if self.tab == ListTab::Settings {
            self.step_setting(by);
            return;
        }
        let (order, _) = self.order();
        let Some(last) = order.len().checked_sub(1) else {
            return;
        };
        let at = self.cursor.and_then(|c| order.iter().position(|i| *i == c));
        let next = match at {
            None if by > 0 => 0,
            None => last,
            Some(i) => i.saturating_add_signed(by).min(last),
        };
        self.land(Some(order[next]), out);
    }

    /// Move the settings cursor `by` rows through the rows as drawn, and stop at
    /// the ends.
    fn step_setting(&mut self, by: isize) {
        let order = self.setting_order();
        let Some(last) = order.len().checked_sub(1) else {
            return;
        };
        let at = self.row.and_then(|r| order.iter().position(|i| *i == r));
        let next = match at {
            None if by > 0 => 0,
            None => last,
            Some(i) => i.saturating_add_signed(by).min(last),
        };
        self.row = Some(order[next]);
        self.scroll = true;
    }

    /// `Tab`: turn the list between the channel's columns and its settings.
    ///
    /// A channel with no settings rows — colour's and the mark's — stays on its
    /// columns. The query is kept across the turn and narrows the rows of the
    /// tab it turns to. Turning to the settings reports [`ListReport::Turned`],
    /// so the window backs out a preview that belongs to the columns; turning
    /// back, the cursor goes where the columns' own rule puts it (the column the
    /// channel holds, or the best match for a query still typed), and a column
    /// that is not the held one is reported as a move, so the chart previews it
    /// again.
    fn turn(&mut self, out: &mut Vec<ListReport>) {
        match self.tab {
            ListTab::Columns if self.has_settings() => {
                self.tab = ListTab::Settings;
                self.row = self.setting_order().first().copied();
                self.scroll = true;
                out.push(ListReport::Turned(ListTab::Settings));
            }
            ListTab::Columns => {}
            ListTab::Settings => {
                self.tab = ListTab::Columns;
                self.cursor = self.held();
                self.scroll = true;
                out.push(ListReport::Turned(ListTab::Columns));
                self.requery(out);
            }
        }
    }

    /// The query changed: the cursor goes to the best match, and with an empty
    /// query back to the column the channel holds.
    fn requery(&mut self, out: &mut Vec<ListReport>) {
        if self.tab == ListTab::Settings {
            self.row = self.setting_order().first().copied();
            self.scroll = true;
            return;
        }
        let to = if self.query.is_empty() {
            self.held()
        } else {
            let (order, matched) = self.order();
            (matched > 0).then(|| order[0])
        };
        self.land(to, out);
    }

    /// `Enter`, or a click on a row: keep the row under the cursor.
    fn keep(&mut self, out: &mut Vec<ListReport>) {
        // On a settings row `Enter` sets no value of its own, since a step is
        // kept as it is made; it ends the query's typing and leaves the rows it
        // narrowed to, so `h` `l` and `⌫` reach the row it found. With no query
        // typing it opens the field of a row that takes typed text.
        if self.tab == ListTab::Settings {
            if !std::mem::take(&mut self.querying) {
                self.open_field();
            }
            return;
        }
        if let Some(name) = self.cursor() {
            out.push(ListReport::Kept(name.to_string()));
            self.querying = false;
        }
    }

    /// `Enter` on a row that takes typed text and applies to the axis: open its
    /// field on the value the row reads, selected. A title the axis does not draw
    /// opens on brightfield's own, the column's name, or on nothing where the
    /// channel holds no column to name it from.
    fn open_field(&mut self) {
        let Some(row) = self.setting_cursor() else {
            return;
        };
        if !row.takes_field() {
            return;
        }
        if row.name == RANGE_ROW {
            // Both ends open at once, on the ends the axis runs over, with the low
            // selected.
            let shown = row
                .range
                .as_ref()
                .and_then(|facts| facts.set.or(facts.drawn));
            let (low, high) = shown.map_or_else(
                || (String::new(), String::new()),
                |ends| (end_text(ends.lo), end_text(ends.hi)),
            );
            self.field = Some(RowField {
                row: RANGE_ROW,
                selected: !low.is_empty(),
                text: low,
                refusal: None,
                ends: Some(EndFields {
                    on: End::Low,
                    other: high,
                }),
            });
            return;
        }
        let text = if row.name == FORMAT_ROW {
            // The specifier the file holds, which every preset is one of; auto
            // holds none, so its field opens empty.
            row.format
                .as_ref()
                .and_then(|facts| facts.specifier.clone())
                .unwrap_or_default()
        } else if row.name == TITLE_ROW && row.value == NO_TITLE {
            match self.channels.binding(self.channel) {
                Some(Binding::Column(name)) => name.clone(),
                _ => String::new(),
            }
        } else {
            row.value.clone()
        };
        self.field = Some(RowField {
            row: row.name,
            selected: !text.is_empty(),
            text,
            refusal: None,
            ends: None,
        });
    }

    /// What the open field holds, as the edit it would keep: the title's trimmed
    /// words, or the count. `Err` carries the sentence for a value the row
    /// refuses, and `Ok(None)` a field with no text in it to keep.
    fn field_edit(&self) -> Result<Option<RowEdit>, String> {
        let Some(field) = &self.field else {
            return Ok(None);
        };
        let value = match field.row {
            TITLE_ROW => {
                let words = field.text.trim();
                if words.is_empty() {
                    return Ok(None);
                }
                SettingValue::Text(words.to_string())
            }
            FORMAT_ROW => {
                if field.text.is_empty() {
                    return Ok(None);
                }
                SettingValue::Text(format_specifier(&field.text)?)
            }
            RANGE_ROW => return self.range_edit(),
            _ => {
                if field.text.trim().is_empty() {
                    return Ok(None);
                }
                SettingValue::Count(ticks_count(&field.text)?)
            }
        };
        Ok(Some(RowEdit {
            channel: self.channel,
            row: field.row,
            value,
        }))
    }

    /// What the range row's two fields hold, as the edit they would keep: the two
    /// ends, low first. `Err` is the sentence for an end the row refuses: not a
    /// number, a high end at or below the low, or an end a log axis cannot draw.
    /// A low end typed above the high is no edit and no refusal, since the high is
    /// asked next.
    fn range_edit(&self) -> Result<Option<RowEdit>, String> {
        let Some(field) = &self.field else {
            return Ok(None);
        };
        let Some(fields) = &field.ends else {
            return Ok(None);
        };
        if field.text.trim().is_empty() {
            return Ok(None);
        }
        let typed = end_number(&field.text)?;
        let other = end_number(&fields.other).ok();
        let log = self
            .setting_cursor()
            .and_then(|row| row.range.as_ref())
            .is_some_and(|facts| facts.log);
        let (lo, hi) = match fields.on {
            End::Low => {
                if log && log_ends_refused(typed) {
                    return Err(log_refusal(typed));
                }
                (Some(typed), other)
            }
            End::High => (other, Some(typed)),
        };
        let (Some(lo), Some(hi)) = (lo, hi) else {
            return Ok(None);
        };
        if fields.on == End::High && hi <= lo {
            return Err(format!(
                "the high end must be above the low end, and {} is not above {}",
                end_text(hi),
                end_text(lo)
            ));
        }
        if log && log_ends_refused(lo) {
            return Err(log_refusal(lo));
        }
        Ok(Ends::new(lo, hi).map(|ends| RowEdit {
            channel: self.channel,
            row: RANGE_ROW,
            value: SettingValue::Ends(ends),
        }))
    }

    /// `Enter` in the range row's low field: keep the low as typed and move to the
    /// high, with the drawn high selected. A low that is no number, or that a log
    /// axis cannot draw, is refused under the row and the field stays on it.
    fn keep_low(&mut self, out: &mut Vec<ListReport>) {
        let log = self
            .setting_cursor()
            .and_then(|row| row.range.as_ref())
            .is_some_and(|facts| facts.log);
        let Some(field) = self.field.as_mut() else {
            return;
        };
        let low = match end_number(&field.text) {
            Ok(low) => low,
            Err(sentence) => {
                field.refusal = Some(sentence);
                return;
            }
        };
        if log && log_ends_refused(low) {
            let sentence = log_refusal(low);
            field.refusal = Some(sentence.clone());
            out.push(ListReport::Refused(sentence));
            return;
        }
        if let Some(fields) = field.ends.as_mut() {
            let high = std::mem::replace(&mut fields.other, field.text.clone());
            fields.on = End::High;
            field.selected = !high.is_empty();
            field.text = high;
            field.refusal = None;
        }
        out.push(ListReport::Preview(self.field_edit().ok().flatten()));
    }

    /// The field's text changed: say what the chart should draw, and what the row
    /// refuses. A value the row keeps is previewed on the axis as it is typed; a
    /// value it refuses, or nothing, takes the preview back so the axis reads as
    /// before. A title emptied on the way to its replacement is not refused
    /// until `Enter`; a count that is not one is refused as it is typed.
    fn refresh_field(&mut self, out: &mut Vec<ListReport>) {
        let edit = self.field_edit();
        if let Some(field) = self.field.as_mut() {
            field.refusal = edit.as_ref().err().cloned();
        }
        out.push(ListReport::Preview(edit.ok().flatten()));
    }

    /// `Enter` in the open field: keep a value the row takes, and refuse the rest
    /// under the row, leaving the field open.
    fn keep_field(&mut self, out: &mut Vec<ListReport>) {
        if self
            .field
            .as_ref()
            .and_then(|f| f.ends.as_ref())
            .is_some_and(|fields| fields.on == End::Low)
        {
            self.keep_low(out);
            return;
        }
        match self.field_edit() {
            Ok(Some(edit)) => {
                self.field = None;
                out.push(ListReport::Set(edit));
            }
            Ok(None) => {
                let sentence = match self.field.as_ref().map(|f| f.row) {
                    Some(TITLE_ROW) => TITLE_NEEDS_TEXT.to_string(),
                    Some(FORMAT_ROW) => format_specifier("").unwrap_err(),
                    Some(RANGE_ROW) => end_number("").unwrap_err(),
                    _ => ticks_count("").unwrap_err(),
                };
                if let Some(field) = self.field.as_mut() {
                    field.refusal = Some(sentence);
                }
            }
            Err(sentence) => {
                if sentence.starts_with(LOG_CANNOT) {
                    out.push(ListReport::Refused(sentence.clone()));
                }
                if let Some(field) = self.field.as_mut() {
                    field.refusal = Some(sentence);
                }
            }
        }
    }

    /// A key press while the field has the keys. `Enter` keeps, `Esc` drops the
    /// field and the preview with it, `⌫` clears a selection and then takes the
    /// last letter; every other key is left to the text it types.
    fn field_press(
        &mut self,
        key: egui::Key,
        modifiers: egui::Modifiers,
        out: &mut Vec<ListReport>,
    ) {
        if !modifiers.is_none() {
            return;
        }
        match key {
            egui::Key::Enter => self.keep_field(out),
            egui::Key::Escape => {
                self.field = None;
                out.push(ListReport::Preview(None));
            }
            egui::Key::Backspace => {
                if let Some(field) = self.field.as_mut() {
                    if std::mem::take(&mut field.selected) {
                        field.text.clear();
                    } else {
                        field.text.pop();
                    }
                }
                self.refresh_field(out);
            }
            _ => {}
        }
    }

    /// Text typed while the field has the keys: the first of it replaces a
    /// selection.
    fn field_type(&mut self, text: &str, out: &mut Vec<ListReport>) {
        if let Some(field) = self.field.as_mut() {
            if std::mem::take(&mut field.selected) {
                field.text.clear();
            }
            field.text.extend(text.chars().filter(|c| !c.is_control()));
        }
        self.refresh_field(out);
    }

    /// `h` or `l` on the settings: step the row under the cursor `by` values and
    /// report the write. A row that does not apply, one that takes typed text,
    /// and a scale at the end of its values report nothing.
    fn step_row(&mut self, by: isize, out: &mut Vec<ListReport>) {
        let Some(row) = self.setting_cursor() else {
            return;
        };
        let (name, step) = (row.name, row.step_to(by));
        match step {
            Some(RowStep::Set(value)) => {
                out.push(ListReport::Set(RowEdit {
                    channel: self.channel,
                    row: name,
                    value,
                }));
            }
            Some(RowStep::Field) => self.open_field(),
            None => {}
        }
    }

    /// `⌫` on the settings: put the row under the cursor back to auto. A row that
    /// does not apply reports nothing.
    fn row_to_auto(&mut self, out: &mut Vec<ListReport>) {
        let Some(row) = self.setting_cursor() else {
            return;
        };
        if row.reason.is_none() && row_key(self.channel, row.name).is_some() {
            let name = row.name;
            out.push(ListReport::Set(RowEdit {
                channel: self.channel,
                row: name,
                value: SettingValue::Auto,
            }));
        }
    }

    /// `Esc`: clear the query first, and with the query empty, back out of the
    /// list: `esc_with_the_query_empty_reports_backing_out`.
    fn back(&mut self, out: &mut Vec<ListReport>) {
        if self.querying || !self.query.is_empty() {
            self.querying = false;
            self.query.clear();
            self.requery(out);
        } else {
            out.push(ListReport::BackedOut);
        }
    }

    /// Name `to`, and move the list onto it when it lists columns.
    fn go_to(&mut self, to: ShelfChannel, beside: bool, out: &mut Vec<ListReport>) {
        if to == self.channel {
            return;
        }
        if to != ShelfChannel::Mark {
            self.channel = to;
            self.query.clear();
            self.querying = false;
            self.cursor = self.held();
            self.scroll = true;
            // The tab is kept where the channel has it: x's settings to y's, and
            // colour's list, which has no settings yet, on its columns.
            if self.tab == ListTab::Settings {
                if self.has_settings() {
                    self.row = self.setting_order().first().copied();
                } else {
                    self.tab = ListTab::Columns;
                }
            }
        }
        out.push(if beside {
            ListReport::Beside(to)
        } else {
            ListReport::GoTo(to)
        });
    }

    /// The channel `by` places from this one, where there is one.
    fn go_beside(&mut self, by: isize, out: &mut Vec<ListReport>) {
        let to = self
            .channel
            .index()
            .checked_add_signed(by)
            .and_then(|i| ShelfChannel::ALL.get(i));
        if let Some(to) = to {
            self.go_to(*to, true, out);
        }
    }

    /// Answer the registry's Shelf-context verb `verb`, and say whether the list
    /// answers it.
    fn dispatch(&mut self, verb: &str, out: &mut Vec<ListReport>) -> bool {
        match verb {
            "move-shelf-next-row" => self.step(1, out),
            "move-shelf-prev-row" => self.step(-1, out),
            // On a settings row `h` and `l` step the value, and on the columns
            // they name the channel beside.
            "move-shelf-left" if self.tab == ListTab::Settings => self.step_row(-1, out),
            "move-shelf-right" if self.tab == ListTab::Settings => self.step_row(1, out),
            "move-shelf-left" => self.go_beside(-1, out),
            "move-shelf-right" => self.go_beside(1, out),
            "set-shelf-setting-to-auto" if self.tab != ListTab::Settings => return false,
            "set-shelf-setting-to-auto" => self.row_to_auto(out),
            "narrow-shelf-list" => self.querying = true,
            "turn-shelf-list" => self.turn(out),
            "keep-shelf-choice" => self.keep(out),
            "back-out-of-shelf" => self.back(out),
            "go-to-mark-cell" => self.go_to(ShelfChannel::Mark, false, out),
            "go-to-x-cell" => self.go_to(ShelfChannel::X, false, out),
            "go-to-y-cell" => self.go_to(ShelfChannel::Y, false, out),
            "go-to-colour-cell" => self.go_to(ShelfChannel::Colour, false, out),
            UNDO => out.push(ListReport::Undo),
            _ => return false,
        }
        true
    }

    /// Resolve `key` through the registry's Shelf context and answer the first
    /// verb the list takes.
    fn resolve(&mut self, key: egui::Key, out: &mut Vec<ListReport>) {
        if let Some(token) = key_token(key) {
            self.resolve_token(token, out);
        }
    }

    /// Answer the first verb the list takes that the registry's Shelf context
    /// binds to `token`.
    fn resolve_token(&mut self, token: &str, out: &mut Vec<ListReport>) {
        for verb in shelf_keys().resolves(token, DispatchContext::ShelfFocused) {
            if self.dispatch(verb, out) {
                return;
            }
        }
    }

    /// A key press. With the command held it is a chord the registry binds
    /// where a bare letter would be typed, and acts from the query too; with no
    /// modifier, while the query has the keys, a letter is text, so the registry
    /// is asked only about the keys that are not one.
    fn press(&mut self, key: egui::Key, modifiers: egui::Modifiers, out: &mut Vec<ListReport>) {
        if self.field.is_some() {
            self.field_press(key, modifiers, out);
            return;
        }
        if let Some(token) = chord_token(key, modifiers) {
            self.resolve_token(token, out);
            return;
        }
        if !modifiers.is_none() {
            return;
        }
        if !self.querying {
            self.resolve(key, out);
            return;
        }
        match key {
            egui::Key::Backspace => {
                if self.query.pop().is_none() {
                    self.querying = false;
                }
                self.requery(out);
            }
            egui::Key::Enter
            | egui::Key::Escape
            | egui::Key::ArrowUp
            | egui::Key::ArrowDown
            | egui::Key::Tab => self.resolve(key, out),
            // The arrows step a settings row from the query, where `h` and `l`
            // are text.
            egui::Key::ArrowLeft | egui::Key::ArrowRight if self.tab == ListTab::Settings => {
                self.resolve(key, out);
            }
            _ => {}
        }
    }

    /// Text typed while the query has the keys.
    fn type_text(&mut self, text: &str, out: &mut Vec<ListReport>) {
        for ch in text.chars().filter(|c| !c.is_control()) {
            self.query.extend(ch.to_lowercase());
        }
        self.requery(out);
    }

    /// Take the events a frame brought, in order, and report what they did.
    ///
    /// A key press goes through the registry while the rows have the keys. Text
    /// goes to the query while the query has them, and is dropped otherwise, so
    /// the letter that named a channel is not also typed. The one text that is
    /// not dropped is `/`, for a keyboard whose slash needs a modifier and so
    /// arrives as text alone. The `/` text that follows the `/` key press in one
    /// frame is the same keystroke and is not typed into the query it opened.
    pub fn feed_events(&mut self, events: &[egui::Event]) -> Vec<ListReport> {
        let mut out = Vec::new();
        let mut opened_by_key = false;
        // Whether the last key was the `l` that stepped onto the format's custom
        // and opened its field: the `l` text that follows it in the frame is the
        // same keystroke and is not typed into the field it opened.
        let mut stepped_into_field = false;
        for event in events {
            match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    let (was, had_field) = (self.querying, self.field.is_some());
                    self.press(*key, *modifiers, &mut out);
                    opened_by_key = !was && self.querying;
                    stepped_into_field = !had_field && self.field.is_some() && *key == egui::Key::L;
                }
                egui::Event::Text(text) if self.field.is_some() => {
                    if std::mem::take(&mut stepped_into_field) && text == "l" {
                        continue;
                    }
                    self.field_type(text, &mut out);
                }
                egui::Event::Text(text) if self.querying => {
                    if std::mem::take(&mut opened_by_key) && text == "/" {
                        continue;
                    }
                    self.type_text(text, &mut out);
                }
                egui::Event::Text(text) if text == "/" => self.querying = true,
                _ => {}
            }
        }
        out
    }
}

/// What the list drew above its rows, handed to the page that draws them.
struct SettingsHead {
    heading: egui::Rect,
    heading_text: String,
    heading_name: egui::Rect,
    tabs: Option<TabsDrawn>,
    query: egui::Rect,
}

/// What stands between the two words of the tab strip.
const TAB_SEPARATOR: &str = "\u{b7}";

/// One row of the list as it was drawn.
#[derive(Clone, Debug)]
pub struct ListRowDrawn {
    /// The column's name.
    pub column: String,
    /// What the row said at its trailing end. `None` on a row that drew a rug
    /// there instead.
    pub kind: Option<String>,
    /// The whole row.
    pub rect: egui::Rect,
    /// Where the name's ink was laid out.
    pub name_rect: egui::Rect,
    /// Where the type's ink was laid out, `None` where [`Self::kind`] is.
    pub kind_rect: Option<egui::Rect>,
    /// The rug the row drew in the type's place, `None` on a row that kept its
    /// type.
    pub rug: Option<RugDrawn>,
    /// The bar down the row's leading edge, on the row under the cursor.
    pub bar: Option<egui::Rect>,
}

/// What the list drew in one frame.
#[derive(Clone, Debug)]
pub struct ListDrawn {
    /// The whole section, from the heading to the foot.
    pub rect: egui::Rect,
    /// The heading's row.
    pub heading: egui::Rect,
    /// What the heading read.
    pub heading_text: String,
    /// Where the heading's ink was laid out.
    pub heading_name: egui::Rect,
    /// The query line.
    pub query: egui::Rect,
    /// Each column, in the order drawn.
    pub rows: Vec<ListRowDrawn>,
    /// The divider between the matches and the rest, with a query typed.
    pub divider: Option<egui::Rect>,
    /// The foot, where the keys of the state the list is in are printed.
    pub foot: egui::Rect,
    /// What a click decided this frame.
    pub reports: Vec<ListReport>,
    /// The tab the list drew.
    pub tab: ListTab,
    /// The tab strip under the heading. `None` where the channel has no
    /// settings to turn to, so no tab would name something `Tab` can reach.
    pub tabs: Option<TabsDrawn>,
    /// Each settings row, in the order drawn. Empty on the columns tab.
    pub settings: Vec<SettingRowDrawn>,
    /// The rule after the head rows, which a query typed takes away. `None` on
    /// the columns tab.
    pub rule: Option<egui::Rect>,
    /// The foot's sentence for the row under the cursor, where the ink was
    /// laid out. `None` on the columns tab, and with no row under the cursor.
    pub sentence: Option<egui::Rect>,
}

/// One tab of the strip as it was drawn.
#[derive(Clone, Debug)]
pub struct TabDrawn {
    /// Which tab the word names.
    pub tab: ListTab,
    /// Where the word's ink was laid out.
    pub word: egui::Rect,
    /// The bar under the word, in the channel's hue, on the open tab alone.
    pub bar: Option<egui::Rect>,
}

/// The tab strip as it was drawn.
#[derive(Clone, Debug)]
pub struct TabsDrawn {
    /// The strip's row, from the list's edge to its edge.
    pub rect: egui::Rect,
    /// Each tab, in the order drawn.
    pub tabs: Vec<TabDrawn>,
}

/// One settings row as it was drawn.
#[derive(Clone, Debug)]
pub struct SettingRowDrawn {
    /// The row's name.
    pub name: &'static str,
    /// The whole row.
    pub rect: egui::Rect,
    /// Where the name's ink was laid out.
    pub name_rect: egui::Rect,
    /// Where the value's ink was laid out.
    pub value_rect: egui::Rect,
    /// The marker at the trailing end: a hollow ring while the value is
    /// brightfield's own, a filled dot while it is the analyst's.
    pub marker: egui::Rect,
    /// Where the word *auto* was laid out, on a row whose value is
    /// brightfield's own.
    pub auto_rect: Option<egui::Rect>,
    /// Where the reason was laid out, under the line, on a row that does not
    /// apply to the axis; or the refusal, on a row whose open field holds a
    /// value the row refuses.
    pub reason_rect: Option<egui::Rect>,
    /// The sunken field the row's value is typed into, on the row whose field is
    /// open.
    pub field: Option<egui::Rect>,
    /// Where the format row's sample was drawn, beside its value, in muted ink,
    /// where the room the chips and the word *auto* leave holds it.
    pub sample_rect: Option<egui::Rect>,
    /// The bar down the row's leading edge, on the row under the cursor.
    pub bar: Option<egui::Rect>,
    /// Where the `←` and `→` chips were drawn, on the row under the cursor or
    /// the pointer where the row steps.
    pub chips: Option<[egui::Rect; 2]>,
    /// The part of the row a click steps: from the value's leading edge to the
    /// marker.
    pub value_zone: egui::Rect,
}

/// How wide the rug is on a numeric column's row, at the trailing end. The
/// rail is 160 wide at least, and what this leaves the name is what a name is
/// fitted to.
const LIST_RUG_WIDTH: f32 = 64.0;

/// What the query line says before anything is typed.
pub const QUERY_PLACEHOLDER: &str = "search columns";

/// The keys the foot prints while the rows have the keys.
const ROW_HINTS: [(&str, &str); 5] = [
    ("/", "search"),
    ("j k", "move"),
    ("h l", "channel"),
    ("Enter", "keep"),
    ("Esc", "back"),
];

/// The keys the foot prints on the settings.
const SETTINGS_HINTS: [(&str, &str); 4] = [
    ("/", "search"),
    ("j k", "move"),
    ("Tab", "columns"),
    ("Esc", "back"),
];

/// The keys the foot prints while a typed row's field is open.
const FIELD_HINTS: [(&str, &str); 2] = [("Enter", "keep"), ("Esc", "drop")];

/// The key the columns' foot adds where the channel has settings to turn to.
const TURN_HINT: (&str, &str) = ("Tab", "settings");

/// The radius of the marker at a settings row's trailing end.
const MARKER_RADIUS: f32 = 3.0;

/// The width and height of the box the marker sits in.
const MARKER_BOX: f32 = 2.0 * MARKER_RADIUS;

/// The height of the bar under the open tab.
const TAB_BAR: f32 = 2.0;

/// The keys the foot prints while the query has them.
const QUERY_HINTS: [(&str, &str); 3] = [
    ("\u{2191}\u{2193}", "move"),
    ("Enter", "keep"),
    ("Esc", "clear"),
];

impl ColumnList {
    /// Draw the list into `ui`, which it takes the whole width of, and answer
    /// the pointer: moving over a row moves the cursor there, which the chart
    /// draws as a preview, and a click on a row keeps it. The pointer and the
    /// keys move one cursor, so the two routes end on the same rows.
    ///
    /// The Meridian theme has to be applied to `ui`'s context, for the key
    /// chips' tokens and the faces. The row under the cursor wears a bar in the
    /// channel's hue, [`control::ROW_BAR_WIDTH`] wide, and the foot prints `/`
    /// while the rows have the keys and not while the query has them:
    /// `the_foot_prints_slash_while_the_rows_have_the_keys_and_not_while_the_query_has_them`.
    pub fn show(&mut self, ui: &mut egui::Ui, mode: Mode) -> ListDrawn {
        let sem = semantic(mode.is_dark());
        let b = control::binding(spacing::ROW_DENSE);
        let painter = ui.painter().clone();
        let width = ui.available_width();
        let muted = chrome::colour(sem.text.muted);
        let primary = chrome::colour(sem.text.primary);

        // The heading names the channel and the tile.
        let (heading, _) = ui.allocate_exact_size(egui::vec2(width, b.row), egui::Sense::hover());
        let heading_text = caption(&[
            "OUTLINE",
            &format!("{} of {}", self.channel.word(), self.tile),
        ]);
        let galley = text_ink::fit(
            &painter,
            &heading_text,
            caption_font(),
            width - 2.0 * spacing::SPACE_4,
            muted,
        );
        let heading_name = egui::Rect::from_min_size(
            egui::pos2(
                heading.left() + spacing::SPACE_4,
                heading.center().y - galley.size().y / 2.0,
            ),
            galley.size(),
        );
        painter.galley(heading_name.min, galley, muted);

        // The tab strip, where the channel has settings to turn to.
        let tabs = self.has_settings().then(|| self.show_tabs(ui, mode));

        // The query line: a sunken field, ruled under in the focus ink while it
        // has the keys.
        let (line, _) =
            ui.allocate_exact_size(egui::vec2(width, spacing::ROW_GRID), egui::Sense::hover());
        let field = line.shrink2(egui::vec2(spacing::SPACE_3, 2.0));
        painter.rect_filled(field, 0.0, chrome::colour(sem.surfaces.sunken));
        let (rule, rule_ink) = if self.querying {
            (2.0, sem.borders.focus)
        } else {
            (1.0, sem.borders.control)
        };
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(field.left(), field.bottom() - rule),
                field.right_bottom(),
            ),
            0.0,
            chrome::colour(rule_ink),
        );
        let text_left = field.left() + spacing::SPACE_3;
        let room = field.right() - spacing::SPACE_3 - text_left;
        if self.query.is_empty() && !self.querying {
            let hint = text_ink::fit(&painter, QUERY_PLACEHOLDER, caption_font(), room, muted);
            let at = egui::pos2(text_left, field.center().y - hint.size().y / 2.0);
            painter.galley(at, hint, muted);
        } else {
            let typed = text_ink::fit(&painter, &self.query, caption_font(), room, primary);
            let at = egui::pos2(text_left, field.center().y - typed.size().y / 2.0);
            let (typed_right, typed_height) = (at.x + typed.size().x, typed.size().y);
            painter.galley(at, typed, primary);
            if self.querying {
                painter.line_segment(
                    [
                        egui::pos2(typed_right + 1.0, at.y),
                        egui::pos2(typed_right + 1.0, at.y + typed_height),
                    ],
                    egui::Stroke::new(1.0, primary),
                );
            }
        }

        if self.tab == ListTab::Settings {
            return self.show_settings(
                ui,
                mode,
                SettingsHead {
                    heading,
                    heading_text,
                    heading_name,
                    tabs,
                    query: line,
                },
            );
        }

        // The rows: the matches, a divider, the rest.
        let (order, matched) = self.order();
        let searching = !self.query.is_empty();
        if searching && matched == 0 {
            let (note, _) = ui.allocate_exact_size(egui::vec2(width, b.row), egui::Sense::hover());
            let text = format!("no column's name holds \"{}\"", self.query);
            let galley = text_ink::fit(
                &painter,
                &text,
                caption_font(),
                width - 2.0 * spacing::SPACE_4,
                muted,
            );
            let at = egui::pos2(
                note.left() + spacing::SPACE_4,
                note.center().y - galley.size().y / 2.0,
            );
            painter.galley(at, galley, muted);
        }
        let hue = chrome::colour(channel::hue(self.channel, mode));
        // The grid head's compact band, whose rug a numeric row draws.
        let rug_frame = column_header_frame(GridDensity::Compact, mode);
        let mut rows = Vec::with_capacity(order.len());
        let mut divider = None;
        let mut clicked = None;
        let mut pointed = None;
        // The pointer moves the cursor only while it moves: a list scrolled
        // under a pointer standing still, or a key pressed with the pointer
        // resting on a row, leaves the cursor where the keys put it.
        let moving = ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
        for (n, &i) in order.iter().enumerate() {
            if searching && n == matched {
                let gap = 2.0 * spacing::SPACE_2 + 1.0;
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, gap), egui::Sense::hover());
                painter.line_segment(
                    [
                        egui::pos2(rect.left() + spacing::SPACE_4, rect.center().y),
                        egui::pos2(rect.right() - spacing::SPACE_4, rect.center().y),
                    ],
                    egui::Stroke::new(1.0, chrome::colour(sem.borders.subtle)),
                );
                divider = Some(rect);
            }
            let column = &self.columns[i];
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(width, b.row), egui::Sense::click());
            let on = self.cursor == Some(i);
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    true,
                    on,
                    column.name.clone(),
                )
            });
            let mut bar = None;
            if on {
                painter.rect_filled(rect, 0.0, chrome::colour(sem.rows.cursor_background));
                let strip = egui::Rect::from_min_max(
                    rect.left_top(),
                    egui::pos2(rect.left() + control::ROW_BAR_WIDTH, rect.bottom()),
                );
                painter.rect_filled(strip, 0.0, hue);
                bar = Some(strip);
                if self.scroll {
                    ui.scroll_to_rect(rect, None);
                }
            } else if response.hovered() {
                painter.rect_filled(rect, 0.0, chrome::colour(sem.rows.hover_background));
            }
            // The columns list's guard. `open_field` runs from the settings
            // tab, so `self.field` is unset here; the guard that keeps the
            // pointer off an open field is the settings list's, in
            // `show_settings`.
            if self.field.is_none() {
                if response.clicked() {
                    clicked = Some(i);
                } else if moving && response.hovered() {
                    pointed = Some(i);
                }
            }
            let content = egui::Rect::from_min_max(
                egui::pos2(rect.left() + b.pad_x + spacing::SPACE_4, rect.top()),
                egui::pos2(rect.right() - b.pad_x, rect.bottom()),
            );
            // A column the engine measured draws its spread where its type
            // would stand, and the name is fitted to what the rug leaves. A
            // column it did not measure draws its type, as the list did before
            // it drew rugs.
            let (name_rect, kind, kind_rect, rug) = if let Some(moments) = column.moments.as_ref() {
                let rug_cell = egui::Rect::from_min_max(
                    egui::pos2(content.right() - LIST_RUG_WIDTH, content.top()),
                    content.right_bottom(),
                );
                let rug = draw_rug_in(&painter, rug_cell, moments, &rug_frame);
                let room = rug_cell.left() - spacing::SPACE_3 - content.left();
                let galley = text_ink::fit(&painter, &column.name, ui_font(), room, primary);
                let at = egui::Rect::from_min_size(
                    egui::pos2(content.left(), content.center().y - galley.size().y / 2.0),
                    galley.size(),
                );
                painter.galley(at.min, galley, primary);
                (at, None, None, Some(rug))
            } else {
                let ends = text_ink::row_ends(
                    &painter,
                    content,
                    &TwoEndedRow {
                        leading: &column.name,
                        trailing: &column.kind,
                        font: ui_font(),
                        gap: spacing::SPACE_3,
                        leading_ink: primary,
                        trailing_ink: muted,
                    },
                );
                (
                    ends.leading,
                    Some(column.kind.clone()),
                    Some(ends.trailing),
                    None,
                )
            };
            rows.push(ListRowDrawn {
                column: column.name.clone(),
                kind,
                rect,
                name_rect,
                kind_rect,
                rug,
                bar,
            });
        }
        self.scroll = false;

        let (foot, _) = self.show_foot(ui, mode, None);
        let rect =
            egui::Rect::from_min_max(heading.min, egui::pos2(heading.right(), foot.bottom()));
        painter.rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.0, chrome::colour(sem.borders.focus)),
            egui::StrokeKind::Inside,
        );

        let mut reports = Vec::new();
        if let Some(i) = clicked {
            self.land(Some(i), &mut reports);
            self.keep(&mut reports);
        } else if pointed.is_some() {
            self.land(pointed, &mut reports);
        }
        ListDrawn {
            rect,
            heading,
            heading_text,
            heading_name,
            query: line,
            rows,
            divider,
            foot,
            reports,
            tab: ListTab::Columns,
            tabs,
            settings: Vec::new(),
            rule: None,
            sentence: None,
        }
    }

    /// The tab strip: *columns · settings*, the open tab's word in the text ink
    /// over a bar in the channel's hue, the other in a quieter ink, all over one
    /// rule. It takes no pointer: `Tab` turns the list, and a strip that took the
    /// pointer would be a control that took the keyboard with it.
    fn show_tabs(&self, ui: &mut egui::Ui, mode: Mode) -> TabsDrawn {
        let sem = semantic(mode.is_dark());
        let painter = ui.painter().clone();
        let width = ui.available_width();
        let (strip, _) =
            ui.allocate_exact_size(egui::vec2(width, spacing::ROW_GRID), egui::Sense::hover());
        painter.line_segment(
            [
                egui::pos2(strip.left(), strip.bottom() - 0.5),
                egui::pos2(strip.right(), strip.bottom() - 0.5),
            ],
            egui::Stroke::new(1.0, chrome::colour(sem.borders.subtle)),
        );
        let hue = chrome::colour(channel::hue(self.channel, mode));
        let primary = chrome::colour(sem.text.primary);
        let secondary = chrome::colour(sem.text.secondary);
        let muted = chrome::colour(sem.text.muted);
        let mut x = strip.left() + spacing::SPACE_4;
        let mut tabs = Vec::new();
        for (n, tab) in [ListTab::Columns, ListTab::Settings]
            .into_iter()
            .enumerate()
        {
            if n > 0 {
                let dot = painter.layout_no_wrap(TAB_SEPARATOR.to_string(), ui_font(), muted);
                x += spacing::SPACE_3;
                let at = egui::pos2(x, strip.center().y - dot.size().y / 2.0);
                x += dot.size().x + spacing::SPACE_3;
                painter.galley(at, dot, muted);
            }
            let open = tab == self.tab;
            let ink = if open { primary } else { secondary };
            let galley = painter.layout_no_wrap(tab.word().to_string(), ui_font(), ink);
            let word = egui::Rect::from_min_size(
                egui::pos2(x, strip.center().y - galley.size().y / 2.0),
                galley.size(),
            );
            painter.galley(word.min, galley, ink);
            let bar = open.then(|| {
                let bar = egui::Rect::from_min_max(
                    egui::pos2(word.left() - spacing::SPACE_2, strip.bottom() - TAB_BAR),
                    egui::pos2(word.right() + spacing::SPACE_2, strip.bottom()),
                );
                painter.rect_filled(bar, 0.0, hue);
                bar
            });
            tabs.push(TabDrawn { tab, word, bar });
            x = word.right();
        }
        TabsDrawn { rect: strip, tabs }
    }

    /// The settings page: the rows of the channel, each with its value and the
    /// marker that says whether the value is brightfield's own, the foot's
    /// sentence for the row under the cursor, and the keys.
    ///
    /// **A row reads its name, its value and a marker.** A value that is
    /// brightfield's own is in muted ink, with the word *auto* and a hollow ring;
    /// one that differs is in the text ink, with a filled dot. Nothing here
    /// changes a value, so a click or the pointer over a row moves the cursor and
    /// no more.
    ///
    /// **A row that does not apply to the axis is in muted ink throughout**, set
    /// or not, and says why on a line of its own under the name and the value. A
    /// row found by name is drawn only where the query names it.
    fn show_settings(&mut self, ui: &mut egui::Ui, mode: Mode, head: SettingsHead) -> ListDrawn {
        let sem = semantic(mode.is_dark());
        let b = control::binding(spacing::ROW_DENSE);
        let painter = ui.painter().clone();
        let width = ui.available_width();
        let muted = chrome::colour(sem.text.muted);
        let primary = chrome::colour(sem.text.primary);
        let hue = chrome::colour(channel::hue(self.channel, mode));
        let rows: Vec<SettingRow> = self.settings().to_vec();
        let order = self.setting_order();

        if order.is_empty() {
            let (note, _) = ui.allocate_exact_size(egui::vec2(width, b.row), egui::Sense::hover());
            let text = format!("no setting has \"{}\" in its name", self.query);
            let galley = text_ink::fit(
                &painter,
                &text,
                caption_font(),
                width - 2.0 * spacing::SPACE_4,
                muted,
            );
            let at = egui::pos2(
                note.left() + spacing::SPACE_4,
                note.center().y - galley.size().y / 2.0,
            );
            painter.galley(at, galley, muted);
        }

        // The names share a column as wide as the longest of the channel's, those
        // found by name included, so the values stand in one line whichever rows
        // a query leaves.
        let name_column = rows
            .iter()
            .map(|r| {
                painter
                    .layout_no_wrap(r.name.to_string(), ui_font(), primary)
                    .size()
                    .x
            })
            .fold(0.0_f32, f32::max);
        let moving = ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
        // What one chip of the `←` `→` pair measures, laid out unseen, so the pair
        // is placed at the row's trailing edge before it is drawn.
        let chip = {
            let at = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(1000.0, 1000.0));
            let mut unseen = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(at)
                    .layout(egui::Layout::left_to_right(egui::Align::Center))
                    .invisible(),
            );
            key_chip(&mut unseen, "\u{2190}").rect.size()
        };
        let mut drawn = Vec::with_capacity(order.len());
        let mut clicked = None;
        let mut pointed = None;
        let mut stepped: Option<(usize, isize)> = None;
        for &i in &order {
            let row = &rows[i];
            // The field of a typed row, open on this row.
            let field = self
                .field
                .as_ref()
                .filter(|f| self.row == Some(i) && f.row == row.name);
            // A row that does not apply says why on a line of its own under the
            // name and the value, in the room the row's content leaves; a row
            // whose field holds a value it refuses says so there, in full ink.
            let refusal = field.and_then(|f| f.refusal.clone());
            let said_in = if refusal.is_some() { primary } else { muted };
            let reason = refusal
                .or_else(|| row.reason.clone())
                .or_else(|| row.range.as_ref().and_then(RangeFacts::flag))
                .map(|text| {
                    painter.layout(
                        text,
                        caption_font(),
                        said_in,
                        width - 2.0 * spacing::SPACE_4 - 2.0 * b.pad_x,
                    )
                });
            let reason_height = reason
                .as_ref()
                .map_or(0.0, |g| g.size().y + spacing::SPACE_2);
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(width, b.row + reason_height),
                egui::Sense::click(),
            );
            let line = egui::Rect::from_min_size(rect.min, egui::vec2(width, b.row));
            let on = self.row == Some(i);
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    true,
                    on,
                    row.name.to_string(),
                )
            });
            let mut bar = None;
            if on {
                painter.rect_filled(rect, 0.0, chrome::colour(sem.rows.cursor_background));
                let strip = egui::Rect::from_min_max(
                    rect.left_top(),
                    egui::pos2(rect.left() + control::ROW_BAR_WIDTH, rect.bottom()),
                );
                painter.rect_filled(strip, 0.0, hue);
                bar = Some(strip);
                if self.scroll {
                    ui.scroll_to_rect(rect, None);
                }
            } else if response.hovered() {
                painter.rect_filled(rect, 0.0, chrome::colour(sem.rows.hover_background));
            }
            // The settings list's own guard: while a title, ticks or format
            // field is open, the pointer does not move the cursor off the
            // field's row, which would hide the field while its text still
            // took keys. The value click below carries the same condition for
            // a step.
            if self.field.is_none() {
                if response.clicked() {
                    clicked = Some(i);
                } else if moving && response.hovered() {
                    pointed = Some(i);
                }
            }
            let content = egui::Rect::from_min_max(
                egui::pos2(rect.left() + b.pad_x + spacing::SPACE_4, rect.top()),
                egui::pos2(rect.right() - b.pad_x, rect.bottom()),
            );

            // The marker at the trailing end. A row that does not apply is in
            // muted ink throughout, set or not, and says so by its reason.
            let applies = row.reason.is_none();
            let centre = egui::pos2(content.right() - MARKER_RADIUS, line.center().y);
            let marker = egui::Rect::from_center_size(centre, egui::Vec2::splat(MARKER_BOX));
            if row.set {
                painter.circle_filled(centre, MARKER_RADIUS, if applies { primary } else { muted });
            } else {
                painter.circle_stroke(
                    centre,
                    MARKER_RADIUS,
                    egui::Stroke::new(1.0, chrome::colour(sem.borders.default_)),
                );
            }
            let mut right = marker.left() - spacing::SPACE_3;

            // The `←` `→` chips, on the row under the cursor or the pointer where
            // `h` and `l` step it: at the trailing edge, beside the marker.
            // A format that reads custom draws none, as the frames draw it: the
            // room goes to its sample, and the foot says what `h` and `l` do.
            let own_specifier = row.name == FORMAT_ROW && row.value == CUSTOM_FORMAT;
            let chips =
                (row.steps() && field.is_none() && !own_specifier && (on || response.hovered()))
                    .then(|| {
                        let gap = spacing::SPACE_2;
                        let size = egui::vec2(2.0 * chip.x + gap, chip.y);
                        let at = egui::Rect::from_min_size(
                            egui::pos2(right - size.x, line.center().y - size.y / 2.0),
                            size,
                        );
                        let mut pair = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(at)
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        pair.spacing_mut().item_spacing.x = gap;
                        let back = key_chip(&mut pair, "\u{2190}").rect;
                        let forward = key_chip(&mut pair, "\u{2192}").rect;
                        right = back.left() - spacing::SPACE_3;
                        [back, forward]
                    });
            let value_zone = egui::Rect::from_min_max(
                egui::pos2(content.left() + name_column + spacing::SPACE_3, rect.top()),
                egui::pos2(marker.left(), line.bottom()),
            );
            if response.clicked() && row.steps() && self.field.is_none() {
                let at = response.interact_pointer_pos();
                if at.is_some_and(|p| value_zone.contains(p)) {
                    let back = at.is_some_and(|p| chips.is_some_and(|[back, _]| back.contains(p)));
                    stepped = Some((i, if back { -1 } else { 1 }));
                }
            }

            // The word *auto*, where the value is brightfield's own.
            let auto_rect = (!row.set && field.is_none()).then(|| {
                let galley = painter.layout_no_wrap(AUTO.to_string(), caption_font(), muted);
                let at = egui::Rect::from_min_size(
                    egui::pos2(
                        right - galley.size().x,
                        line.center().y - galley.size().y / 2.0,
                    ),
                    galley.size(),
                );
                painter.galley(at.min, galley, muted);
                right = at.left() - spacing::SPACE_3;
                at
            });

            // The name, then the value in what room is left.
            let name_ink = if applies { primary } else { muted };
            let name_galley = painter.layout_no_wrap(row.name.to_string(), ui_font(), name_ink);
            let name_rect = egui::Rect::from_min_size(
                egui::pos2(content.left(), line.center().y - name_galley.size().y / 2.0),
                name_galley.size(),
            );
            painter.galley(name_rect.min, name_galley, name_ink);
            let value_left = content.left() + name_column + spacing::SPACE_3;
            let ink = if row.set && applies { primary } else { muted };
            let (value_rect, field_rect) = if let Some(open) = field {
                // A field: a sunken ground ruled under in the focus ink, the text in
                // full ink with the selection's wash behind it while the whole is
                // selected, and the caret after it. The range row draws two, low and
                // high, with the dash between, and the caret in the one that has the
                // keys.
                let ground = egui::Rect::from_min_max(
                    egui::pos2(value_left - spacing::SPACE_2, line.top() + 2.0),
                    egui::pos2(right, line.bottom() - 2.0),
                );
                let paint = |ground: egui::Rect, text: &str, selected: bool, active: bool| {
                    painter.rect_filled(ground, 0.0, chrome::colour(sem.surfaces.sunken));
                    let left = ground.left() + spacing::SPACE_2;
                    let room = (ground.right() - spacing::SPACE_2 - left).max(0.0);
                    let typed = text_ink::fit(&painter, text, ui_font(), room, primary);
                    let at = egui::Rect::from_min_size(
                        egui::pos2(left, line.center().y - typed.size().y / 2.0),
                        typed.size(),
                    );
                    if active && selected && !text.is_empty() {
                        painter.rect_filled(
                            at.expand2(egui::vec2(1.0, 1.0)),
                            0.0,
                            chrome::colour(sem.editor.selection),
                        );
                    }
                    painter.galley(at.min, typed, primary);
                    if active {
                        painter.line_segment(
                            [
                                egui::pos2(at.right() + 1.0, at.top()),
                                egui::pos2(at.right() + 1.0, at.bottom()),
                            ],
                            egui::Stroke::new(1.0, chrome::colour(sem.editor.caret)),
                        );
                        // The rule goes on last, over the selection's wash.
                        painter.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(ground.left(), ground.bottom() - 2.0),
                                ground.right_bottom(),
                            ),
                            0.0,
                            chrome::colour(sem.borders.focus),
                        );
                    }
                    at
                };
                let at = if let Some(fields) = &open.ends {
                    let dash = painter.layout_no_wrap("\u{2013}".to_string(), ui_font(), muted);
                    let half =
                        ((ground.width() - dash.size().x - 2.0 * spacing::SPACE_2) / 2.0).max(0.0);
                    let low_ground = egui::Rect::from_min_max(
                        ground.left_top(),
                        egui::pos2(ground.left() + half, ground.bottom()),
                    );
                    let high_ground = egui::Rect::from_min_max(
                        egui::pos2(ground.right() - half, ground.top()),
                        ground.right_bottom(),
                    );
                    let dash_at = egui::pos2(
                        ground.center().x - dash.size().x / 2.0,
                        line.center().y - dash.size().y / 2.0,
                    );
                    painter.galley(dash_at, dash, muted);
                    let on_low = fields.on == End::Low;
                    let (low_text, high_text) = if on_low {
                        (open.text.as_str(), fields.other.as_str())
                    } else {
                        (fields.other.as_str(), open.text.as_str())
                    };
                    let low_at = paint(low_ground, low_text, open.selected, on_low);
                    let high_at = paint(high_ground, high_text, open.selected, !on_low);
                    if on_low {
                        low_at
                    } else {
                        high_at
                    }
                } else {
                    paint(ground, &open.text, open.selected, true)
                };
                (at, Some(ground))
            } else {
                let value = text_ink::fit(&painter, &row.value, ui_font(), right - value_left, ink);
                let at = egui::Rect::from_min_size(
                    egui::pos2(value_left, line.center().y - value.size().y / 2.0),
                    value.size(),
                );
                painter.galley(at.min, value, ink);
                (at, None)
            };
            // The format's sample, muted, after the value. It gives way to the
            // chips and the word *auto*, which have taken their room off `right`,
            // and is left out where it does not fit whole: a clipped sample reads
            // as a different number.
            let sample_rect = row
                .format
                .as_ref()
                .and_then(|facts| facts.sample.as_ref())
                .filter(|_| field_rect.is_none())
                .and_then(|text| {
                    let left = value_rect.right() + spacing::SPACE_3;
                    let galley = painter.layout_no_wrap(text.clone(), caption_font(), muted);
                    (galley.size().x <= right - left).then(|| {
                        let at = egui::Rect::from_min_size(
                            egui::pos2(left, line.center().y - galley.size().y / 2.0),
                            galley.size(),
                        );
                        painter.galley(at.min, galley, muted);
                        at
                    })
                });
            let reason_rect = reason.map(|galley| {
                let at = egui::Rect::from_min_size(
                    egui::pos2(content.left(), line.bottom()),
                    galley.size(),
                );
                painter.galley(at.min, galley, said_in);
                at
            });
            drawn.push(SettingRowDrawn {
                name: row.name,
                rect,
                name_rect,
                value_rect,
                marker,
                auto_rect,
                reason_rect,
                field: field_rect,
                sample_rect,
                bar,
                chips,
                value_zone,
            });
        }
        self.scroll = false;

        // A rule follows the head rows. A query typed takes it away: what is
        // left is what was looked for, and nothing follows it.
        let rule = self.query.is_empty().then(|| {
            let gap = 2.0 * spacing::SPACE_2 + 1.0;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, gap), egui::Sense::hover());
            painter.line_segment(
                [
                    egui::pos2(rect.left() + spacing::SPACE_4, rect.center().y),
                    egui::pos2(rect.right() - spacing::SPACE_4, rect.center().y),
                ],
                egui::Stroke::new(1.0, chrome::colour(sem.borders.subtle)),
            );
            rect
        });

        let sentence = self.setting_cursor().map(|row| {
            let typing = self.field.as_ref().is_some_and(|f| f.row == row.name);
            if typing && row.name == FORMAT_ROW {
                FORMAT_FIELD_SAYS.to_string()
            } else {
                foot_sentence(row)
            }
        });
        let (foot, said) = self.show_foot(ui, mode, sentence.as_deref());
        let rect = egui::Rect::from_min_max(
            head.heading.min,
            egui::pos2(head.heading.right(), foot.bottom()),
        );
        painter.rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.0, chrome::colour(sem.borders.focus)),
            egui::StrokeKind::Inside,
        );

        // The pointer moves the cursor and a click leaves it there; a click on
        // the value of a row that steps does what `l` does, and the chip of `←`
        // what `h` does.
        if let Some(i) = clicked.or(pointed) {
            if self.row != Some(i) {
                self.row = Some(i);
                self.scroll = true;
            }
        }
        let mut reports = Vec::new();
        if let Some((i, by)) = stepped {
            match rows[i].step_to(by) {
                Some(RowStep::Set(value)) => reports.push(ListReport::Set(RowEdit {
                    channel: self.channel,
                    row: rows[i].name,
                    value,
                })),
                Some(RowStep::Field) => self.open_field(),
                None => {}
            }
        }
        ListDrawn {
            rect,
            heading: head.heading,
            heading_text: head.heading_text,
            heading_name: head.heading_name,
            query: head.query,
            rows: Vec::new(),
            divider: None,
            foot,
            reports,
            tab: ListTab::Settings,
            tabs: head.tabs,
            settings: drawn,
            rule,
            sentence: said,
        }
    }

    /// The foot: a key chip and a word for each key the state the list is in
    /// answers, a pair to a unit and wrapped between pairs to the width.
    ///
    /// On the settings the foot opens with `sentence`, the one line of what the
    /// row under the cursor does and the rule for its default, and returns where
    /// its ink was laid out.
    fn show_foot(
        &self,
        ui: &mut egui::Ui,
        mode: Mode,
        sentence: Option<&str>,
    ) -> (egui::Rect, Option<egui::Rect>) {
        let sem = semantic(mode.is_dark());
        let muted = chrome::colour(sem.text.muted);
        let mut columns_pairs = ROW_HINTS.to_vec();
        if self.has_settings() {
            // Before `Esc`, the last key of the columns' foot.
            columns_pairs.insert(columns_pairs.len() - 1, TURN_HINT);
        }
        let pairs: &[(&str, &str)] = if self.field.is_some() {
            &FIELD_HINTS
        } else if self.querying {
            &QUERY_HINTS
        } else if self.tab == ListTab::Settings {
            &SETTINGS_HINTS
        } else {
            &columns_pairs
        };
        let avail = ui.available_rect_before_wrap();
        let room = avail.width() - 2.0 * spacing::SPACE_4;
        let row_height = ui.spacing().interact_size.y;
        let widths: Vec<f32> = pairs
            .iter()
            .map(|(key, word)| hint_width(ui, key, word, muted))
            .collect();

        let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
        let mut used = 0.0;
        for (i, width) in widths.iter().enumerate() {
            let row = rows.last_mut().expect("a row");
            let next = if row.is_empty() {
                *width
            } else {
                used + spacing::SPACE_4 + *width
            };
            if !row.is_empty() && next > room {
                rows.push(vec![i]);
                used = *width;
            } else {
                row.push(i);
                used = next;
            }
        }

        let mut top = avail.top() + spacing::SPACE_3;
        let said = sentence.map(|text| {
            let ink = chrome::colour(sem.text.secondary);
            let galley = ui.painter().layout(text.to_owned(), ui_font(), ink, room);
            let at = egui::Rect::from_min_size(
                egui::pos2(avail.left() + spacing::SPACE_4, top),
                galley.size(),
            );
            ui.painter().galley(at.min, galley, ink);
            top = at.bottom() + spacing::SPACE_3;
            at
        });
        for row in rows {
            let at = egui::Rect::from_min_size(
                egui::pos2(avail.left() + spacing::SPACE_4, top),
                egui::vec2(room, row_height),
            );
            let mut line = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(at)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            line.spacing_mut().item_spacing.x = spacing::SPACE_3;
            for (n, i) in row.into_iter().enumerate() {
                if n > 0 {
                    line.add_space(spacing::SPACE_4 - spacing::SPACE_3);
                }
                hint(&mut line, pairs[i].0, pairs[i].1, muted);
            }
            top += row_height + spacing::SPACE_2;
        }
        let foot = egui::Rect::from_min_max(avail.min, egui::pos2(avail.right(), top));
        ui.allocate_rect(foot, egui::Sense::hover());
        (foot, said)
    }
}

/// What the foot reads under the cursor's row: the reason the row does not
/// apply where it carries one, else what the row does, with the key it is read
/// from where that is not the axis's own.
#[must_use]
pub fn foot_sentence(row: &SettingRow) -> String {
    if let Some(reason) = &row.reason {
        let mut chars = reason.chars();
        let first = chars.next().map(|c| c.to_uppercase().to_string());
        return format!("{}{}.", first.unwrap_or_default(), chars.as_str());
    }
    if let Some(facts) = &row.format {
        return format_foot(row, facts);
    }
    match row.from {
        Some(from) => format!("Read from {from}, which sets both axes. {}", row.says),
        None => row.says.to_string(),
    }
}

/// What the foot reads on the format row: its sentence, the presets it steps
/// through, the specifier the file gets, and for a specifier no preset writes, what
/// `h` and `l` would do to it.
fn format_foot(row: &SettingRow, facts: &FormatFacts) -> String {
    let mut foot = format!("{} Takes: {}.", row.says, format_preset_names());
    let Some(specifier) = &facts.specifier else {
        return foot;
    };
    foot.push_str(&format!(" Writes {}: \"{specifier}\".", facts.key));
    if row.value == CUSTOM_FORMAT {
        let before = FORMAT_PRESETS[FORMAT_PRESETS.len() - 2].0;
        foot.push_str(&format!(
            " h leaves \"{specifier}\" for {before}, l has nowhere to go; u takes the change back."
        ));
    }
    foot
}

/// One key of the foot: its chip, and the word for what it does.
fn hint(ui: &mut egui::Ui, key: &str, word: &str, ink: egui::Color32) {
    key_chip(ui, key);
    ui.add(
        egui::Label::new(
            egui::RichText::new(word)
                .font(egui::FontId::monospace(typography::UI_SIZE - 2.5))
                .color(ink),
        )
        .selectable(false),
    );
}

/// How wide [`hint`] is, laid out unseen: a chip's width is the design
/// system's, and the foot breaks its rows on it rather than on a guess.
fn hint_width(ui: &mut egui::Ui, key: &str, word: &str, ink: egui::Color32) -> f32 {
    let at = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(1000.0, 1000.0));
    let mut unseen = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(at)
            .layout(egui::Layout::left_to_right(egui::Align::Center))
            .invisible(),
    );
    unseen.spacing_mut().item_spacing.x = spacing::SPACE_3;
    hint(&mut unseen, key, word, ink);
    unseen.min_rect().width()
}

// ---------------------------------------------------------------------------
// The list as a card, for a window whose navigator rail is shut
// ---------------------------------------------------------------------------

/// How wide the card is, rule to rule.
pub const CARD_WIDTH: f32 = 320.0;

/// The card's rule, one pixel in the default border ink.
const CARD_RULE: f32 = 1.0;

/// How far the card's foot, its rule included, stands from the window's when the
/// list is longer than the room under the cell: the design system's `SPACE_4`.
const CARD_MARGIN: f32 = spacing::SPACE_4;

/// The frame of a floating card: the workbench's overlay fill, a one-pixel rule
/// in the default border ink, the corner the control radius names, and the
/// shadow [`Elevation::Overlay`] declares, which has no blur.
///
/// Read off the design system rather than typed here, so a change to what an
/// overlay looks like moves the card with the rest of the chrome. It carries no
/// inner margin: the card's contents are the list's, which pads its own rows.
pub fn floating_card_frame(mode: Mode) -> egui::Frame {
    let dark = mode.is_dark();
    let mut frame = chrome::overlay_frame(mode)
        .inner_margin(egui::Margin::ZERO)
        .stroke(egui::Stroke::new(
            CARD_RULE,
            chrome::colour(semantic(dark).borders.default_),
        ))
        .corner_radius(radius::CONTROL);
    if let Some(shadow) = Elevation::Overlay.shadow(dark) {
        frame = frame.shadow(egui::epaint::Shadow {
            offset: [shadow.x as i8, shadow.y as i8],
            blur: shadow.blur as u8,
            spread: 0,
            color: chrome::colour(shadow.colour),
        });
    }
    frame
}

/// What the card drew in one frame.
#[derive(Clone, Debug)]
pub struct CardDrawn {
    /// The whole card, its rule included and its shadow not.
    pub rect: egui::Rect,
    /// The cell the card hangs from, as the band drew it.
    pub cell: egui::Rect,
    /// The list inside it, as the Outline would have drawn it. Its
    /// [`ListDrawn::reports`] are empty: the card hands them on, as the Outline
    /// does, to be acted on with the keys' on the next frame.
    pub list: ListDrawn,
}

impl ColumnList {
    /// **Draw the list as a card hung from `cell`**, the band's cell for the
    /// list's channel: the list the Outline draws, with its heading, query line,
    /// rows and keys, in [`CARD_WIDTH`], over whatever is under it. It takes no
    /// layout space, and its height is the list's up to the room the window has
    /// under the cell, where the rows scroll.
    ///
    /// This is for a window whose navigator rail is shut, where the Outline that
    /// would draw the list is not drawn. The keys are the list's own either way
    /// ([`Self::feed_events`]); what the card adds is the place they act on.
    ///
    /// **A press and release outside the card back out of the list**, as `Esc`
    /// does with the query empty: [`ListReport::BackedOut`] is among the
    /// reports. That is a press anywhere but on the card, the cell it hangs
    /// from included; a click on another cell is the band's, which opens that
    /// cell's list in its place.
    pub fn show_card(&mut self, ctx: &egui::Context, cell: egui::Rect, mode: Mode) -> CardDrawn {
        let frame = floating_card_frame(mode);
        // The frame draws its rule on the card's top and foot outside the list's
        // height, so the list is given that much less.
        let room =
            (ctx.content_rect().bottom() - cell.bottom() - CARD_MARGIN - 2.0 * CARD_RULE).max(0.0);
        let shown = egui::Area::new(egui::Id::new("shelf-column-card"))
            .order(egui::Order::Foreground)
            .fixed_pos(cell.left_bottom())
            .constrain(true)
            .fade_in(false)
            .show(ctx, |ui| {
                frame.show(ui, |ui| {
                    ui.set_width(CARD_WIDTH - 2.0 * CARD_RULE);
                    egui::ScrollArea::vertical()
                        .max_height(room)
                        .auto_shrink([false, true])
                        .show(ui, |ui| self.show(ui, mode))
                        .inner
                })
            });
        let card = shown.inner;
        let mut list = card.inner;
        let rect = card.response.rect;
        let outside = ctx.input(|i| {
            i.pointer.primary_clicked()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|at| !rect.contains(at))
        });
        if outside {
            list.reports.push(ListReport::BackedOut);
        }
        CardDrawn { rect, cell, list }
    }
}
