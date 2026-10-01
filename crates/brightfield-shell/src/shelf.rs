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
        Key::Slash => "/",
        Key::ArrowLeft => "left",
        Key::ArrowRight => "right",
        Key::ArrowUp => "up",
        Key::ArrowDown => "down",
        Key::Enter => "enter",
        Key::Escape => "escape",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// The column list.
// ---------------------------------------------------------------------------

/// One column of the table, as the list offers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListColumn {
    /// The column's name.
    pub name: String,
    /// What the row says at its trailing end: the column's type.
    pub kind: String,
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
    /// The cursor moved to this column, which the chart draws as a preview.
    Moved(String),
    /// `Enter` kept this column.
    Kept(String),
    /// `Esc` with the query empty: the reader backs out of the list.
    BackedOut,
    /// A channel's letter named this channel. The list is on it when it lists
    /// columns, which the mark's cell does not.
    GoTo(ShelfChannel),
    /// `h` or `l` named the channel beside, and the list stays open. The list is
    /// on it when it lists columns.
    Beside(ShelfChannel),
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
        };
        list.cursor = list.held();
        list
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
    fn step(&mut self, by: isize, out: &mut Vec<ListReport>) {
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

    /// The query changed: the cursor goes to the best match, and with an empty
    /// query back to the column the channel holds.
    fn requery(&mut self, out: &mut Vec<ListReport>) {
        let to = if self.query.is_empty() {
            self.held()
        } else {
            let (order, matched) = self.order();
            (matched > 0).then(|| order[0])
        };
        self.land(to, out);
    }

    /// `Enter`: keep the row under the cursor.
    fn keep(&mut self, out: &mut Vec<ListReport>) {
        if let Some(name) = self.cursor() {
            out.push(ListReport::Kept(name.to_string()));
            self.querying = false;
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
            "move-shelf-left" => self.go_beside(-1, out),
            "move-shelf-right" => self.go_beside(1, out),
            "narrow-shelf-list" => self.querying = true,
            "keep-shelf-choice" => self.keep(out),
            "back-out-of-shelf" => self.back(out),
            "go-to-mark-cell" => self.go_to(ShelfChannel::Mark, false, out),
            "go-to-x-cell" => self.go_to(ShelfChannel::X, false, out),
            "go-to-y-cell" => self.go_to(ShelfChannel::Y, false, out),
            "go-to-colour-cell" => self.go_to(ShelfChannel::Colour, false, out),
            _ => return false,
        }
        true
    }

    /// Resolve `key` through the registry's Shelf context and answer the first
    /// verb the list takes.
    fn resolve(&mut self, key: egui::Key, out: &mut Vec<ListReport>) {
        let Some(token) = key_token(key) else {
            return;
        };
        for verb in shelf_keys().resolves(token, DispatchContext::ShelfFocused) {
            if self.dispatch(verb, out) {
                return;
            }
        }
    }

    /// A key press, with no modifier held. While the query has the keys a letter
    /// is text, so the registry is asked only about the keys that are not one.
    fn press(&mut self, key: egui::Key, modifiers: egui::Modifiers, out: &mut Vec<ListReport>) {
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
            egui::Key::Enter | egui::Key::Escape | egui::Key::ArrowUp | egui::Key::ArrowDown => {
                self.resolve(key, out)
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
        for event in events {
            match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    let was = self.querying;
                    self.press(*key, *modifiers, &mut out);
                    opened_by_key = !was && self.querying;
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

/// One row of the list as it was drawn.
#[derive(Clone, Debug)]
pub struct ListRowDrawn {
    /// The column's name.
    pub column: String,
    /// What the row said at its trailing end.
    pub kind: String,
    /// The whole row.
    pub rect: egui::Rect,
    /// Where the name's ink was laid out.
    pub name_rect: egui::Rect,
    /// Where the type's ink was laid out.
    pub kind_rect: egui::Rect,
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
}

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

/// The keys the foot prints while the query has them.
const QUERY_HINTS: [(&str, &str); 3] = [
    ("\u{2191}\u{2193}", "move"),
    ("Enter", "keep"),
    ("Esc", "clear"),
];

impl ColumnList {
    /// Draw the list into `ui`, which it takes the whole width of, and answer a
    /// click on a row by moving the cursor there.
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
        let mut rows = Vec::with_capacity(order.len());
        let mut divider = None;
        let mut clicked = None;
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
            if response.clicked() {
                clicked = Some(i);
            }
            let ends = text_ink::row_ends(
                &painter,
                egui::Rect::from_min_max(
                    egui::pos2(rect.left() + b.pad_x + spacing::SPACE_4, rect.top()),
                    egui::pos2(rect.right() - b.pad_x, rect.bottom()),
                ),
                &TwoEndedRow {
                    leading: &column.name,
                    trailing: &column.kind,
                    font: ui_font(),
                    gap: spacing::SPACE_3,
                    leading_ink: primary,
                    trailing_ink: muted,
                },
            );
            rows.push(ListRowDrawn {
                column: column.name.clone(),
                kind: column.kind.clone(),
                rect,
                name_rect: ends.leading,
                kind_rect: ends.trailing,
                bar,
            });
        }
        self.scroll = false;

        let foot = self.show_foot(ui, mode);
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
        }
    }

    /// The foot: a key chip and a word for each key the state the list is in
    /// answers, a pair to a unit and wrapped between pairs to the width.
    fn show_foot(&self, ui: &mut egui::Ui, mode: Mode) -> egui::Rect {
        let muted = chrome::colour(semantic(mode.is_dark()).text.muted);
        let pairs: &[(&str, &str)] = if self.querying {
            &QUERY_HINTS
        } else {
            &ROW_HINTS
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
        foot
    }
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
