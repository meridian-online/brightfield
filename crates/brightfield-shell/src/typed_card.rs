//! **The statement form**: the card the chart palette becomes when the text on
//! its query line reads as SQL ([`crate::overlays::is_statement`]).
//!
//! The palette is a modal layer over a scrim, and a statement is typed to be
//! read against the rows the grid shows, so the form is drawn by brightfield
//! in an area of its own with no scrim: the page stays as bright as it was and
//! the grid beside the card stays readable while its rows follow the text.
//! It holds the statement in a monospace field that grows a line at a time,
//! the one row under it saying what keeping the statement would make, and the
//! hints for its two keys.

use egui::{Key, Modifiers, RichText};
use meridian_design::{semantic, spacing};
use meridian_egui::{key_chip, overlay_frame, MeridianUi as _, ModalChrome};

/// The area the card is drawn in, which is also how the frame names it.
pub const STATEMENT_CARD_ID: &str = "bf-overlay-statement";

/// What the row under the statement says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypedSaid {
    /// Keeping the statement would filter the rows of `table`, keeping
    /// `rows` of them when they have been counted.
    Filter {
        /// The table the filter reads.
        table: String,
        /// How many rows the condition keeps, once counted.
        rows: Option<u64>,
    },
    /// Keeping the statement would make a step of its own.
    SqlStep,
    /// The statement as it stands was refused, for this reason.
    Refused(String),
}

impl TypedSaid {
    /// The row's words.
    #[must_use]
    pub fn line(&self) -> String {
        use brightfield_model::profile_model::thousands;
        match self {
            Self::Filter {
                table,
                rows: Some(rows),
            } => format!("filter-rows on {table} \u{b7} {} rows", thousands(*rows)),
            Self::Filter { table, rows: None } => format!("filter-rows on {table}"),
            Self::SqlStep => "a SQL step".to_string(),
            Self::Refused(reason) => reason.clone(),
        }
    }
}

/// What one frame of the card asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardEvent {
    /// Cmd+Enter: keep the statement.
    Keep,
    /// Esc: close the form and keep nothing.
    Close,
    /// The text stopped being a statement, so it is a search again.
    Searching,
}

/// The statement being typed, and what the card needs to draw it again.
#[derive(Clone, Debug)]
pub struct StatementCard {
    text: String,
    replacing: Option<String>,
    /// Whether the next frame gives the field the keyboard, with the caret at
    /// the end of the text.
    focus: bool,
    /// The card's height less the field's, as the last frame drew it: what
    /// the field's room is worked out from.
    chrome_height: f32,
    rect: Option<egui::Rect>,
}

impl StatementCard {
    /// A form holding `text`, the caret at its end, over the step named
    /// `replacing` when the statement is to take that step's place.
    #[must_use]
    pub fn new(text: String, replacing: Option<String>) -> Self {
        Self {
            text,
            replacing,
            focus: true,
            chrome_height: spacing::SPACE_9 * 2.0,
            rect: None,
        }
    }

    /// The statement as it stands.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The step the statement is to take the place of, when there is one.
    #[must_use]
    pub fn replacing(&self) -> Option<&str> {
        self.replacing.as_deref()
    }

    /// Where the card was drawn on the last frame, its rule included and its
    /// shadow not.
    #[must_use]
    pub const fn rect(&self) -> Option<egui::Rect> {
        self.rect
    }

    /// **Draw the card at the top of `room`**, as wide as `room` less
    /// `SPACE_4` each side, with `said` in the row under the field. Cmd+Enter
    /// and Esc are taken before the field draws, so neither reaches it.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        room: egui::Rect,
        said: Option<&TypedSaid>,
    ) -> Option<CardEvent> {
        let keep = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
        let close = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        if close {
            return Some(CardEvent::Close);
        }
        let id = egui::Id::new(STATEMENT_CARD_ID);
        let field = id.with("statement");
        // The caret goes to the end of the text, and the field is asked for
        // the keyboard, until the field first has it: on the frame the palette
        // turns into this form, the palette's modal layer still holds the
        // keyboard. After that the field asks again whenever nothing holds the
        // keyboard, so a click on the grid beside the card, which has no scrim
        // to catch it, does not leave the keys going nowhere.
        if self.focus {
            let mut state = egui::TextEdit::load_state(ctx, field).unwrap_or_default();
            let end = egui::text::CCursor::new(self.text.chars().count());
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(end)));
            egui::TextEdit::store_state(ctx, field, state);
        }
        if self.focus || ctx.memory(|m| m.focused().is_none()) {
            ctx.memory_mut(|m| m.request_focus(field));
        }
        let gutter = spacing::SPACE_4;
        let width = (room.width() - 2.0 * gutter).max(0.0);
        let tallest = (room.height() - 2.0 * gutter).max(0.0);
        let field_room = (tallest - self.chrome_height).max(0.0);
        let chrome = ModalChrome::new().title("SQL").without_esc_hint();
        let mut field_height = 0.0;
        let shown = egui::Area::new(egui::Id::new(STATEMENT_CARD_ID))
            .order(egui::Order::Foreground)
            .fixed_pos(room.min + egui::vec2(gutter, gutter))
            .constrain(false)
            .fade_in(false)
            .show(ctx, |ui| {
                overlay_frame(ui, &chrome, |ui| {
                    let t = ui.tokens();
                    let muted = meridian_egui::theme::to_color32(
                        semantic(ui.visuals().dark_mode).text.muted,
                    );
                    // The chrome sets the card to a width rung; the form is as
                    // wide as the room it stands in instead.
                    let inner = width - 2.0 * (t.modal_padding + 1.0);
                    ui.set_width(inner.max(0.0));
                    ui.set_max_height(tallest);
                    let scrolled = egui::ScrollArea::vertical()
                        .id_salt(id.with("scroll"))
                        .max_height(field_room)
                        .auto_shrink([false, true])
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.text)
                                    .id(field)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_rows(1)
                                    .desired_width(f32::INFINITY),
                            )
                        });
                    if scrolled.inner.has_focus() {
                        self.focus = false;
                    }
                    field_height = scrolled.inner_rect.height();
                    ui.add_space(t.section_gap);
                    let line = said.map(TypedSaid::line).unwrap_or_default();
                    ui.add(egui::Label::new(RichText::new(line).small().color(muted)).wrap());
                    ui.add_space(t.section_gap);
                    // A row as tall as its chips: a right-to-left layout would
                    // take the card's whole remaining height as its own.
                    ui.horizontal(|ui| {
                        for (i, (key, verb)) in [("\u{2318}\u{21a9}", "keep"), ("esc", "close")]
                            .into_iter()
                            .enumerate()
                        {
                            if i > 0 {
                                ui.add_space(t.control_gap);
                            }
                            key_chip(ui, key);
                            ui.add_space(t.space[1]);
                            ui.label(RichText::new(verb).small().color(muted));
                        }
                    });
                });
            });
        let rect = shown.response.rect;
        self.rect = Some(rect);
        self.chrome_height = (rect.height() - field_height).max(0.0);
        if keep {
            return Some(CardEvent::Keep);
        }
        (!crate::overlays::is_statement(&self.text)).then_some(CardEvent::Searching)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_row_under_the_statement_says_what_keeping_it_makes() {
        let filter = |rows| TypedSaid::Filter {
            table: "california_housing".to_string(),
            rows,
        };
        assert_eq!(
            filter(Some(8_831)).line(),
            "filter-rows on california_housing \u{b7} 8,831 rows"
        );
        assert_eq!(filter(None).line(), "filter-rows on california_housing");
        assert_eq!(TypedSaid::SqlStep.line(), "a SQL step");
        assert_eq!(
            TypedSaid::Refused("Parser Error: syntax error".to_string()).line(),
            "Parser Error: syntax error"
        );
    }
}
