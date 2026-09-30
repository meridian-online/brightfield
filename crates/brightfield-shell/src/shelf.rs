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
//! mark's cell on the left and at colour on the right, and `Esc` leaves the
//! open cell. The keycap a cell prints is read from the registry's binding for
//! the verb that goes to it, so a key moved there moves on the band.
//! `the_cell_keys_printed_on_the_band_are_the_registrys` holds it. The list of
//! columns that opens under a cell, its query, `j` `k` and `Enter` belong to
//! the list, and the band returns `false` for them, as
//! `a_key_with_a_modifier_and_the_lists_keys_are_not_the_bands` holds.

use std::sync::OnceLock;

use brightfield_keys::dispatch::{resolution_table, DispatchContext, ResolutionTable};
use brightfield_keys::registry::{keymap_bindings, registry, BindingContext};
use brightfield_spec::ast::{Mark, PlotNode, SpecValue, ValueOrParamRef};
use brightfield_spec::vocab::is_colour_literal;
use brightfield_workbench::channel::{self, ShelfChannel, BAND_HEIGHT};
use brightfield_workbench::chrome;
use meridian_design::{control, semantic, spacing, typography};
use meridian_egui::{icons, key_chip};

use crate::design::Mode;
use crate::protocol::ui_font;
use crate::shelf_edit::{column_of, marks_of, reads_selection};
use crate::text_ink;

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
}

impl ShelfBand {
    /// A band over `channels` with no cell open.
    #[must_use]
    pub fn new(channels: ShelfChannels) -> Self {
        Self {
            channels,
            active: None,
            preview: None,
        }
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
        if let Some((on, column)) = &self.preview {
            if *on == channel {
                return CellWords {
                    word,
                    value: column.clone(),
                    previewed: true,
                    empty: false,
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
}

/// Paint one cell's contents: the keycap, the channel's word over the column's
/// name, the chevron on the word's line and, where it fits, *preview*.
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

    // The channel's word, in a text ink and not in the channel's hue.
    let room = (label_right - text_left).max(0.0);
    let word = text_ink::fit(painter, words.word, label_font.clone(), room, muted);
    let word_width = word.size().x;
    painter.galley(egui::pos2(text_left, block_top), word, muted);

    // *preview*, where the word leaves the room for it.
    if words.previewed {
        let preview = painter.layout_no_wrap(PREVIEW.to_owned(), label_font, muted);
        let at = label_right - preview.size().x;
        if at >= text_left + word_width + spacing::SPACE_3 {
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

/// The registry's keystroke for an egui key the band answers, with no modifier
/// held.
fn key_token(key: egui::Key) -> Option<&'static str> {
    use egui::Key;
    Some(match key {
        Key::M => "m",
        Key::X => "x",
        Key::Y => "y",
        Key::C => "c",
        Key::H => "h",
        Key::L => "l",
        Key::ArrowLeft => "left",
        Key::ArrowRight => "right",
        Key::Escape => "escape",
        _ => return None,
    })
}
