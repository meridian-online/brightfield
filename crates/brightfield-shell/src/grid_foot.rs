//! **The row at the grid's foot**: a band under the table that holds a `where`
//! field and an `order by` field, each after its word, and the count of rows
//! and columns at its trailing end.
//!
//! The field reads as the text that will be recorded, with no prompt glyph, so
//! it is an `egui::TextEdit` of brightfield's own rather than the design
//! system's query line, which draws one.
//!
//! **Asking** is the state in which one field holds text being edited. A key on
//! the grid's cursor opens `where` filled in from the cursor's cell
//! ([`ChartDoc::ask_where_at_cursor`]), and [`ChartDoc::foot_offer`] opens
//! either field with any text, so a condition can come from somewhere other
//! than the key. While asking, the grid reads through the last text that
//! parsed: each edit is tried against the step, and one DuckDB refuses leaves
//! the rows as they were and puts DuckDB's words on the status band. That read
//! is a preview, held here and apart from the document's applied view, so Esc
//! goes back to the applied view and only Enter raises the unsaved mark.
//!
//! **The key is `w` for `where` and `o` for `order by`**
//! ([`ChartDoc::ask_order_at_cursor`], which opens the field on the cursor's
//! column with the caret after it). Both are provisional until the key
//! registry's scores settle them. The literal a `where` is filled in with is
//! read from the stored value and not from the text the cell shows: strings
//! are quoted, dates and timestamps typed, a double holding a whole number
//! below 2^53 is written as its digits and any other float in a form that
//! reads back exactly, and a NULL cell offers `is null`, because `= NULL`
//! keeps nothing.
//!
//! **Enter applies** the text as the document's grid view
//! ([`ChartDoc::apply_grid_view`]), and writes no file. The field carries a dot
//! while its text is in no file, and the window's unsaved mark and its close
//! question cover the view as they cover a chart edit. Save records the view
//! through arc as a generated step (see [`crate::operation`]). An emptied field
//! and Enter show the whole table again.
//!
//! **A view refines the node in front; it is not a new node.** The grid reads
//! the same step with the condition and the order passed to the read, so a
//! chart over that step keeps its rows, and after Save the charts still draw
//! from the load.
//!
//! **Save records the condition alone, not a chart's selection.** While a
//! selection the grid's rows are filtered by holds a clause, they are narrowed
//! by it as well as by the condition, so the band's count says it is under the
//! current selection ([`count_text`]). A selection that only highlights does
//! not narrow them, and the count says nothing of it. The step Save records
//! keeps the rows the condition keeps, whatever was selected when it was saved.
//!
//! **Typed SQL lands here too.** A statement on the chart palette's statement
//! form that arc reads as a filter of the loaded table is offered to `where`
//! when the field can take it ([`ChartDoc::where_takes`]): the grid is on the
//! loaded table and no recorded condition is held. While the form shows it,
//! the grid previews the condition ([`ChartDoc::preview_typed_where`]) with
//! the first page and the count the statement's reads found on a worker, so
//! the frame does not wait on DuckDB for it
//! (`typed_where_preview_does_not_block_the_ui_thread` in
//! `tests/typed_sql_where.rs`), and Cmd+Enter applies it as Enter
//! applies the field's text ([`ChartDoc::apply_typed_where`]), keeping a
//! pending order. Every other statement is kept as a SQL step.
//!
//! **The row works on the loaded table's grid.** On a generated step's grid the
//! band shows the step's own argument read-only, and the key puts the reason on
//! the status band: a step without a run offers Run, and one that has run says
//! filtering a saved step comes in a later version. Filtering a step starts
//! from a run of it, never from its preview, and showing a run's table is later
//! work.
//!
//! [`ChartDoc::ask_where_at_cursor`]: crate::app::ChartDoc::ask_where_at_cursor
//! [`ChartDoc::ask_order_at_cursor`]: crate::app::ChartDoc::ask_order_at_cursor
//! [`ChartDoc::apply_grid_view`]: crate::app::ChartDoc::apply_grid_view
//! [`ChartDoc::foot_offer`]: crate::app::ChartDoc::foot_offer
//! [`ChartDoc::where_takes`]: crate::app::ChartDoc::where_takes
//! [`ChartDoc::preview_typed_where`]: crate::app::ChartDoc::preview_typed_where
//! [`ChartDoc::apply_typed_where`]: crate::app::ChartDoc::apply_typed_where

use brightfield_engine::{CellLiteral, RowsAudience, RowsView, Session};
use brightfield_workbench::chrome;
use meridian_design::{semantic, spacing};

use crate::app::ChartDoc;
use crate::data_grid::{cell_font, grid_state_id, GridPage};
use crate::design::Mode;
use crate::pipeline::LiveDashboard;

/// The band's height, in logical points, taken off the foot of the grid tile.
///
/// 26 is the height of the band in the frame it was designed in, not a step of
/// the design system's spacing or control scale: no rung of that ladder is 26,
/// so moving it onto the nearest token would change the band rather than tidy
/// it.
pub const FOOT_BAND_HEIGHT: f32 = 26.0;

/// The radius of the dot a field carries while its applied text is in no file,
/// and the spine's mark for a step made from typed SQL that is in no file yet.
pub(crate) const UNRECORDED_DOT_RADIUS: f32 = 3.0;

/// **The dot for a thing that is in no file yet**, centred on `centre`: a
/// field of this band whose applied text is unrecorded, and a typed step's row
/// and its table's in the spine. One function draws both, so the mark is one
/// mark wherever it appears.
pub(crate) fn paint_unrecorded_dot(painter: &egui::Painter, centre: egui::Pos2, dark: bool) {
    let ink = chrome::colour(semantic(dark).text.primary);
    painter.circle_filled(centre, UNRECORDED_DOT_RADIUS, ink);
}

/// The share of the band's room, after the count, the `where` field takes.
const WHERE_SHARE: f32 = 0.6;

/// One of the band's two fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FootField {
    /// The condition the rows must meet.
    Where,
    /// The order the rows are sorted by.
    OrderBy,
}

impl FootField {
    /// The word the field starts with.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Where => "where",
            Self::OrderBy => "order by",
        }
    }

    /// The field's part of `view`.
    fn of(self, view: &RowsView) -> Option<&str> {
        match self {
            Self::Where => view.condition.as_deref(),
            Self::OrderBy => view.order.as_deref(),
        }
    }

    /// `view` with this field's part set to `text`; empty text clears it.
    fn with(self, view: &RowsView, text: &str) -> RowsView {
        let part = (!text.trim().is_empty()).then(|| text.to_owned());
        let mut view = view.clone();
        match self {
            Self::Where => view.condition = part,
            Self::OrderBy => view.order = part,
        }
        view
    }

    /// The `TextEdit`'s id, under the grid's own so it goes where the grid
    /// goes.
    #[must_use]
    pub fn id(self) -> egui::Id {
        grid_state_id().with(("grid-foot", self.word()))
    }
}

/// A comparison a condition filled in from a cell can make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sign {
    Is,
    IsNot,
    Over,
    Under,
    AtLeast,
    AtMost,
}

impl Sign {
    const ALL: [Self; 6] = [
        Self::Is,
        Self::IsNot,
        Self::Over,
        Self::Under,
        Self::AtLeast,
        Self::AtMost,
    ];

    const fn sql(self) -> &'static str {
        match self {
            Self::Is => "=",
            Self::IsNot => "!=",
            Self::Over => ">",
            Self::Under => "<",
            Self::AtLeast => ">=",
            Self::AtMost => "<=",
        }
    }

    /// The comparison in the status band's words.
    const fn words(self) -> &'static str {
        match self {
            Self::Is => "is",
            Self::IsNot => "is not",
            Self::Over => "over",
            Self::Under => "under",
            Self::AtLeast => "at least",
            Self::AtMost => "at most",
        }
    }

    /// The comparison a typed key asks for: `=`, `!`, `>` or `<`. An `=`
    /// typed directly after a comparison key extends it, so `>` `=` asks `>=`,
    /// `<` `=` asks `<=` and `!` `=` stays `!=`.
    fn typed(text: &str, after: Option<Self>) -> Option<Self> {
        match (text, after) {
            ("=", Some(Self::Over)) => Some(Self::AtLeast),
            ("=", Some(Self::Under)) => Some(Self::AtMost),
            ("=", Some(Self::IsNot)) => Some(Self::IsNot),
            ("=", _) => Some(Self::Is),
            ("!", _) => Some(Self::IsNot),
            (">", _) => Some(Self::Over),
            ("<", _) => Some(Self::Under),
            _ => None,
        }
    }
}

/// The cell a key filled the field in from: its column as SQL spells it and as
/// the table names it.
#[derive(Clone, Debug)]
struct FromCell {
    column: String,
    name: String,
}

impl FromCell {
    /// `text` read as this column compared to a value, or `None` when the
    /// analyst has written something else.
    fn read<'t>(&self, text: &'t str) -> Option<(Sign, &'t str)> {
        let rest = text.strip_prefix(self.column.as_str())?.strip_prefix(' ')?;
        // A sign must be followed by a space, so `>` does not read `>= 28`
        // and the order the signs are tried in does not matter.
        Sign::ALL.into_iter().find_map(|sign| {
            let value = rest.strip_prefix(sign.sql())?.strip_prefix(' ')?;
            (!value.is_empty()).then_some((sign, value))
        })
    }

    /// The characters of `text` that are the value, when `text` reads as this
    /// column compared to one.
    fn value_range(&self, text: &str) -> Option<(usize, usize)> {
        let (_, value) = self.read(text)?;
        let total = text.chars().count();
        Some((total - value.chars().count(), total))
    }
}

/// The field being edited.
#[derive(Clone, Debug)]
struct Asking {
    field: FootField,
    text: String,
    from_cell: Option<FromCell>,
    /// The name of the cursor's column, when a key on the cursor opened the
    /// field: what the operation panel names as the column in hand.
    column: Option<String>,
    /// Whether the next draw gives the field the keyboard.
    focus: bool,
    /// Whether the next draw selects the value.
    select_value: bool,
    /// Whether the field held the keyboard at the end of the last draw.
    had_focus: bool,
    /// The comparison key typed last, while no other key has been since: what
    /// an `=` extends.
    last_sign: Option<Sign>,
    /// Whether Enter was pressed this frame, for the draw to apply once the
    /// field has taken the frame's own typing.
    enter: bool,
    /// The text last tried against the step.
    tried: Option<String>,
    /// The view the grid reads while asking: the last one that parsed, or
    /// `None` before any has.
    preview: Option<RowsView>,
    /// DuckDB's words for the last text tried, when it refused it.
    refusal: Option<String>,
}

/// What the band drew this frame, for the frame around it and for tests.
#[derive(Clone, Debug, PartialEq)]
pub struct FootDrawn {
    /// The band, in window space.
    pub rect: egui::Rect,
    /// The text the `where` field drew.
    pub where_text: String,
    /// The text the `order by` field drew.
    pub order_text: String,
    /// The count at the band's trailing end: `240 rows · 9 columns`.
    pub count: String,
    /// Whether the `where` field drew its not-recorded dot.
    pub where_dot: bool,
    /// Whether the `order by` field drew its not-recorded dot.
    pub order_dot: bool,
}

/// **A typed filter the grid previews**: the view it would apply, and the
/// first page and count of the rows it keeps, as a read off the frame found
/// them.
#[derive(Clone, Debug)]
pub(crate) struct TypedWhere {
    /// The applied view with the typed condition in place of its own.
    pub(crate) view: RowsView,
    /// The first page of the rows the condition keeps.
    pub(crate) page: GridPage,
    /// How many rows the condition keeps.
    pub(crate) rows: u64,
}

/// The band's state: the field being edited, if one is, the typed filter the
/// grid previews, if one is, and what the band drew. Held on the document
/// beside the view it previews.
#[derive(Clone, Debug, Default)]
pub struct FootState {
    asking: Option<Asking>,
    pub(crate) typed: Option<TypedWhere>,
    /// The names of the columns the grid drew, by index — what a key on the
    /// cursor's cell reads its column from.
    pub(crate) columns: Vec<String>,
    pub(crate) drawn: Option<FootDrawn>,
}

impl FootState {
    /// The field being edited and its text.
    #[must_use]
    pub fn asking(&self) -> Option<(FootField, &str)> {
        self.asking.as_ref().map(|a| (a.field, a.text.as_str()))
    }

    /// Start editing `field` with `text` in it.
    pub(crate) fn ask(&mut self, field: FootField, text: String) {
        self.begin(field, text, None);
    }

    fn begin(&mut self, field: FootField, text: String, from_cell: Option<FromCell>) {
        let select_value = from_cell
            .as_ref()
            .is_some_and(|cell| cell.value_range(&text).is_some());
        let column = from_cell.as_ref().map(|cell| cell.name.clone());
        // The field's own preview takes over from a typed filter's.
        self.typed = None;
        self.asking = Some(Asking {
            field,
            text,
            from_cell,
            column,
            focus: true,
            select_value,
            had_focus: false,
            last_sign: None,
            enter: false,
            tried: None,
            preview: None,
            refusal: None,
        });
    }

    /// The name of the cursor's column, when a key on the cursor opened the
    /// field being edited.
    #[must_use]
    pub fn asked_column(&self) -> Option<&str> {
        self.asking.as_ref()?.column.as_deref()
    }

    /// Stop editing, leaving the applied view as it was.
    pub(crate) fn cancel(&mut self) -> Option<FootField> {
        self.asking.take().map(|a| a.field)
    }

    /// Forget that the field being edited held the keyboard: the band was not
    /// drawn, so Enter and Esc are the window's until it is again.
    pub(crate) fn release_keys(&mut self) {
        if let Some(asking) = self.asking.as_mut() {
            asking.had_focus = false;
        }
    }

    /// Whether the field being edited held the keyboard at the end of the last
    /// draw — whether Enter and Esc are the field's rather than the window's.
    /// A field opened and not yet drawn, or drawn nowhere, does not hold them.
    #[must_use]
    pub fn holds_keys(&self) -> bool {
        self.asking.as_ref().is_some_and(|a| a.had_focus)
    }

    /// The view Enter applies: `applied` with the edited field's text, when
    /// that text has been tried and DuckDB took it. `None` while it has not,
    /// or refused it, so Enter leaves the field asking.
    pub(crate) fn entered(&self, applied: &RowsView) -> Option<(FootField, RowsView)> {
        let asking = self.asking.as_ref()?;
        if asking.refusal.is_some() || asking.tried.as_deref() != Some(asking.text.as_str()) {
            return None;
        }
        Some((asking.field, asking.field.with(applied, &asking.text)))
    }

    /// The view the grid reads: the preview while a field is edited and some
    /// text of it has parsed, then a typed filter's while the grid previews
    /// one, and otherwise `applied`.
    #[must_use]
    pub fn read_view<'a>(&'a self, applied: &'a RowsView) -> &'a RowsView {
        self.asking
            .as_ref()
            .and_then(|a| a.preview.as_ref())
            .or(self.typed.as_ref().map(|t| &t.view))
            .unwrap_or(applied)
    }

    /// Whether DuckDB refused the last text of the field being edited.
    #[must_use]
    pub fn refused(&self) -> bool {
        self.asking.as_ref().is_some_and(|a| a.refusal.is_some())
    }

    /// What the status band says about the field being edited: DuckDB's words
    /// for a condition it refused, or the comparison a condition filled in from
    /// a cell makes, in words — `house_age over 28`, `house_age is null`.
    #[must_use]
    pub fn said(&self) -> Option<String> {
        let asking = self.asking.as_ref()?;
        if let Some(refusal) = &asking.refusal {
            return Some(refusal.clone());
        }
        let cell = asking.from_cell.as_ref()?;
        if asking.text == format!("{} is null", cell.column) {
            return Some(format!("{} is null", cell.name));
        }
        let (sign, value) = cell.read(&asking.text)?;
        Some(format!("{} {} {value}", cell.name, sign.words()))
    }
}

/// `name` as SQL writes the column: bare when it is a plain lower-case
/// identifier DuckDB does not know as a word, so `house_age` reads as itself,
/// and quoted otherwise, so a column named `order` or `median income` still
/// parses (`a_null_cell_asks_is_null` in `tests/grid_foot.rs`).
fn column_sql(session: &Session, name: &str) -> String {
    let plain = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    // A keyword list DuckDB could not give is read as a keyword: quoting a name
    // that did not need it costs nothing but looks.
    if plain && !session.is_keyword(name).unwrap_or(true) {
        name.to_owned()
    } else {
        crate::sql_ident::quote(name)
    }
}

impl ChartDoc {
    /// The band's state, read-only.
    #[must_use]
    pub const fn grid_foot(&self) -> &FootState {
        &self.grid_foot
    }

    /// The field being edited and its text, or `None` when neither is.
    #[must_use]
    pub fn foot_asking(&self) -> Option<(FootField, &str)> {
        self.grid_foot.asking()
    }

    /// What the band drew on the last frame the grid drew it.
    #[must_use]
    pub const fn foot_drawn(&self) -> Option<&FootDrawn> {
        self.grid_foot.drawn.as_ref()
    }

    /// The line the status band draws for the band: for the field being
    /// edited, [`FootState::said`]; over a generated step's preview, once a key
    /// asked the band, why it cannot ([`crate::data_grid::GridPreview::offer`]);
    /// otherwise, while a recorded condition is held
    /// ([`ChartDoc::grid_condition_held`]), that it is recorded and how to
    /// change it.
    #[must_use]
    pub fn foot_said(&self) -> Option<String> {
        if self.grid_preview().is_some() {
            return self.preview_said();
        }
        self.grid_foot.said().or_else(|| {
            self.grid_condition_held().map(|step| {
                format!(
                    "where is already recorded as {step}; record or empty order by to change it"
                )
            })
        })
    }

    /// The view the grid reads this frame; see [`FootState::read_view`].
    #[must_use]
    pub fn grid_read_view(&self) -> &RowsView {
        self.grid_foot.read_view(self.grid_view())
    }

    /// **Open `field` with `text` in it**, whatever the text came from: the
    /// entry point a key, a click on the field and typed SQL share. The grid
    /// follows the text once it parses; nothing is applied until Enter.
    ///
    /// `where` does not open while a recorded condition is held: the grid
    /// shows what the folder makes plus the pending order, and an edited
    /// condition would show rows no step makes. Nor does either field open over
    /// a generated step's preview, whose band is read-only.
    pub fn foot_offer(&mut self, field: FootField, text: String) {
        if self.preview_refuses_ask() {
            return;
        }
        if field == FootField::Where && self.grid_condition_held().is_some() {
            return;
        }
        self.grid_foot.ask(field, text);
    }

    /// **Open `where` filled in from the cursor's cell**: the cell's column
    /// equal to the cell's value, with the value selected, or `is null` for a
    /// NULL cell, which no `=` matches. The literal is read from the stored
    /// value through the view the grid shows, never from the text drawn.
    ///
    /// Whether it opened: with no cursor, no live session, or a cell the step
    /// no longer has, nothing opens. Nor does it open while a recorded
    /// condition is held, or over a generated step's preview, for the reasons
    /// [`Self::foot_offer`] gives; over a preview the status band then says
    /// why.
    pub fn ask_where_at_cursor(&mut self) -> bool {
        if self.preview_refuses_ask() || self.grid_condition_held().is_some() {
            return false;
        }
        let Some((cell, literal)) = self.cursor_cell() else {
            return false;
        };
        let text = match literal {
            CellLiteral::Null => format!("{} is null", cell.column),
            CellLiteral::Value(value) => format!("{} = {value}", cell.column),
        };
        self.grid_foot.begin(FootField::Where, text, Some(cell));
        true
    }

    /// **Open `order by` with the cursor's column in it.** Whether it opened,
    /// on the terms [`Self::ask_where_at_cursor`] gives.
    pub fn ask_order_at_cursor(&mut self) -> bool {
        if self.preview_refuses_ask() {
            return false;
        }
        let Some((cell, _)) = self.cursor_cell() else {
            return false;
        };
        self.grid_foot.ask(FootField::OrderBy, cell.column);
        if let Some(asking) = self.grid_foot.asking.as_mut() {
            asking.column = Some(cell.name);
        }
        true
    }

    /// The cursor's column and the literal its cell equals, read through the
    /// view the grid shows.
    fn cursor_cell(&mut self) -> Option<(FromCell, CellLiteral)> {
        let at = self.grid_cursor?;
        let name = self.grid_foot.columns.get(at.col)?.clone();
        let mark = self.live_dashboard().map_or(0, LiveDashboard::rows_mark);
        let view = self.grid_read_view().clone();
        let session = self.live_coordinator()?.session();
        let literal = session
            .cell_literal(mark, RowsAudience::Reader, &view, at.row, &name)
            .ok()?;
        let column = column_sql(session, &name);
        Some((FromCell { column, name }, literal))
    }

    /// **Esc**: stop editing and put the field back as it was. Says which field
    /// stopped, so the window can take the keyboard from it.
    pub fn foot_cancel(&mut self) -> Option<FootField> {
        self.grid_foot.cancel()
    }

    /// **Enter**: ask the band's next draw to apply the edited field's text.
    /// The draw applies it once the field has taken the frame's own typing, so
    /// a last key or a paste in the same frame as Enter is part of what is
    /// applied ([`foot_band`]). Whether a field is being edited.
    pub fn foot_press_enter(&mut self) -> bool {
        let Some(asking) = self.grid_foot.asking.as_mut() else {
            return false;
        };
        asking.enter = true;
        true
    }

    /// **Whether a typed filter of `on` can land in `where`**: the grid shows
    /// no step's preview, so it is on the loaded table, `on` is
    /// `root_table` in any case, and no recorded condition is held, which the
    /// field would not open over ([`Self::foot_offer`]).
    pub(crate) fn where_takes(&self, on: &str, root_table: &str) -> bool {
        self.grid_preview()
            .is_none_or(crate::data_grid::GridPreview::is_typed)
            && on.eq_ignore_ascii_case(root_table)
            && self.grid_condition_held().is_none()
    }

    /// **Preview the typed `condition` in the grid**: the applied view with
    /// `condition` as its `where`, drawn from `page` and counted as `rows`,
    /// both read off the frame. A field being edited lets go, as it does when
    /// a typed statement's rows are shown.
    pub(crate) fn preview_typed_where(&mut self, condition: &str, page: GridPage, rows: u64) {
        self.grid_foot.cancel();
        self.grid_foot.typed = Some(TypedWhere {
            view: FootField::Where.with(self.grid_view(), condition),
            page,
            rows,
        });
    }

    /// Stop previewing a typed filter; the grid reads the applied view again.
    pub(crate) fn clear_typed_where(&mut self) {
        self.grid_foot.typed = None;
    }

    /// **Apply the typed `condition` as the grid's `where`**, as Enter applies
    /// the field's text: the applied view with `condition` in place of its
    /// own, so a pending order is kept, unrecorded until Save.
    pub(crate) fn apply_typed_where(&mut self, condition: &str) {
        self.clear_typed_where();
        let view = FootField::Where.with(self.grid_view(), condition);
        self.apply_foot_view(view);
    }

    /// Apply `view` as the grid's view from the band, and put the cursor on the
    /// first row shown, in its column.
    fn apply_foot_view(&mut self, view: RowsView) {
        self.apply_grid_view(view);
        if let Some(at) = self.grid_cursor.as_mut() {
            at.row = 0;
            self.grid_cursor_reveal = true;
        }
    }
}

/// What a draw of the band asks of the frame around it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FootOutcome {
    /// The band changed what the grid reads, so the next frame should draw.
    pub repaint: bool,
}

/// Try the edited field's text against the step, once per text: one DuckDB
/// takes becomes the preview the grid reads, and one it refuses leaves the
/// preview as it was and keeps DuckDB's words.
fn try_text(doc: &mut ChartDoc, asking: &mut Asking) -> bool {
    if asking.tried.as_deref() == Some(asking.text.as_str()) {
        return false;
    }
    asking.tried = Some(asking.text.clone());
    let candidate = asking.field.with(doc.grid_view(), &asking.text);
    let mark = doc.live_dashboard().map_or(0, LiveDashboard::rows_mark);
    let Some(coordinator) = doc.live_coordinator() else {
        return false;
    };
    match coordinator
        .session()
        .step_rows_count_view(mark, RowsAudience::Reader, &candidate)
    {
        Ok(_) => {
            asking.refusal = None;
            asking.preview = Some(candidate);
        }
        Err(e) => asking.refusal = Some(e.to_string()),
    }
    true
}

/// Swap the comparison for each comparison key typed while the value is
/// selected, keeping the value, and take those keys out of the frame's input
/// so the field does not also type them. The first other text ends it: that
/// key replaces the value, as the field's own typing does.
fn swap_signs(ui: &egui::Ui, asking: &mut Asking) -> bool {
    let Some(cell) = asking.from_cell.clone() else {
        return false;
    };
    let id = asking.field.id();
    let selected = |text: &str| {
        let Some((start, end)) = cell.value_range(text) else {
            return false;
        };
        egui::TextEdit::load_state(ui.ctx(), id)
            .and_then(|s| s.cursor.char_range())
            .is_some_and(|r| {
                let (a, b) = (r.primary.index.0, r.secondary.index.0);
                (a.min(b), a.max(b)) == (start, end)
            })
    };
    if !selected(&asking.text) {
        return false;
    }
    let mut swapped = false;
    let mut open = true;
    ui.input_mut(|input| {
        input.events.retain(|event| {
            let typed = match event {
                egui::Event::Text(typed) if open => typed,
                // A paste replaces the value as the field types it.
                egui::Event::Paste(_) => {
                    open = false;
                    asking.last_sign = None;
                    return true;
                }
                _ => return true,
            };
            let Some(sign) = Sign::typed(typed, asking.last_sign) else {
                open = false;
                asking.last_sign = None;
                return true;
            };
            if let Some((_, value)) = cell.read(&asking.text) {
                asking.text = format!("{} {} {value}", cell.column, sign.sql());
                asking.last_sign = Some(sign);
                swapped = true;
            }
            false
        });
    });
    if swapped {
        asking.select_value = true;
    }
    swapped
}

/// Select the value of a condition filled in from a cell, so the next key
/// replaces it.
fn select_value(ctx: &egui::Context, asking: &Asking) {
    let Some((start, end)) = asking
        .from_cell
        .as_ref()
        .and_then(|cell| cell.value_range(&asking.text))
    else {
        return;
    };
    let id = asking.field.id();
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(start),
            egui::text::CCursor::new(end),
        )));
    egui::TextEdit::store_state(ctx, id, state);
}

/// The count at the band's trailing end: `240 rows · 9 columns`, or, while a
/// chart's selection narrows the grid's own rows
/// ([`LiveDashboard::rows_narrowed`]), `12 rows under the current selection ·
/// 9 columns`, so a count read off the band is not taken for the condition's
/// alone: Save records the condition and not the selection.
#[must_use]
pub fn count_text(rows: u64, columns: usize, under_selection: bool) -> String {
    use brightfield_model::profile_model::thousands;
    let under = if under_selection {
        " under the current selection"
    } else {
        ""
    };
    format!(
        "{} rows{under} \u{b7} {} columns",
        thousands(rows),
        thousands(columns as u64)
    )
}

/// **Draw the band in `rect`**: each field after its word, the not-recorded dot
/// on a field whose applied text is in no file, and the count of `rows` and
/// `columns` at the trailing end.
///
/// The field being edited is tried against the step after each edit, which is
/// how the grid follows it. A press on the `where` field while neither field is
/// edited opens it with its applied text. The field losing the keyboard to
/// anything but Enter or Esc, which the window takes first, puts it back as
/// Esc does.
///
/// With `fixed`, the band is read-only: it is the band of a generated step's
/// preview, so each field shows the text given, neither takes the keyboard nor
/// carries a dot, and the trailing end says the rows are a preview
/// ([`crate::data_grid::PREVIEW_COUNT`]) rather than counting them.
pub fn foot_band(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    doc: &mut ChartDoc,
    state: &mut FootState,
    counts: (u64, usize),
    fixed: Option<&[String; 2]>,
    mode: Mode,
) -> FootOutcome {
    let sem = semantic(mode.is_dark());
    let painter = ui.painter_at(rect);
    let muted = chrome::colour(sem.text.muted);
    let ink = chrome::colour(sem.text.primary);
    let font = cell_font();
    let mut outcome = FootOutcome::default();

    painter.hline(
        rect.x_range(),
        rect.top(),
        egui::Stroke::new(1.0, chrome::colour(sem.borders.subtle)),
    );

    // The count, at the trailing end.
    let (rows, columns) = counts;
    let count = if fixed.is_some() {
        crate::data_grid::PREVIEW_COUNT.to_owned()
    } else {
        // A typed filter's count was read apart from the session, under no
        // selection.
        count_text(
            rows,
            columns,
            state.typed.is_none()
                && doc
                    .live_dashboard()
                    .is_some_and(LiveDashboard::rows_narrowed),
        )
    };
    let pad = spacing::SPACE_2;
    let count_galley = painter.layout_no_wrap(count.clone(), font.clone(), muted);
    let count_at = egui::pos2(
        rect.right() - pad - count_galley.size().x,
        rect.center().y - count_galley.size().y / 2.0,
    );
    painter.galley(count_at, count_galley, muted);

    // The two fields share the room the count leaves.
    let left = rect.left() + pad;
    let room = (count_at.x - spacing::SPACE_3 - left).max(0.0);
    let split = left + room * WHERE_SHARE;
    let applied = doc.grid_view().clone();
    let unrecorded = [
        fixed.is_none() && doc.grid_condition_unrecorded().is_some(),
        fixed.is_none() && applied.order.is_some(),
    ];
    let condition_held = doc.grid_condition_held().is_some();
    let mut texts = [String::new(), String::new()];
    let mut dots = [false, false];

    for (slot, (field, from, to)) in [
        (FootField::Where, left, split - spacing::SPACE_3),
        (FootField::OrderBy, split, left + room),
    ]
    .into_iter()
    .enumerate()
    {
        // The word.
        let word = painter.layout_no_wrap(field.word().to_owned(), font.clone(), muted);
        let word_size = word.size();
        painter.galley(
            egui::pos2(from, rect.center().y - word_size.y / 2.0),
            word,
            muted,
        );
        let mut x = from + word_size.x + spacing::SPACE_2;
        // The dot, while the field's applied text is in no file.
        if unrecorded[slot] {
            let centre = egui::pos2(x + UNRECORDED_DOT_RADIUS, rect.center().y);
            paint_unrecorded_dot(&painter, centre, mode.is_dark());
            x += 2.0 * UNRECORDED_DOT_RADIUS + spacing::SPACE_2;
            dots[slot] = true;
        }
        let field_rect = egui::Rect::from_min_max(
            egui::pos2(x.min(to), rect.top() + 1.0),
            egui::pos2(to, rect.bottom()),
        );
        let editing = fixed.is_none() && state.asking.as_ref().is_some_and(|a| a.field == field);
        // The order field takes typing only once a key has opened it; a press
        // opens the condition field, unless a recorded condition is held or
        // the band is read-only.
        let interactive = editing
            || (fixed.is_none()
                && field == FootField::Where
                && state.asking.is_none()
                && !condition_held);

        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(field_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let mut idle_text = match fixed {
            Some(texts) => texts[slot].clone(),
            None => field
                .of(state.typed.as_ref().map_or(&applied, |t| &t.view))
                .unwrap_or_default()
                .to_owned(),
        };
        if let Some(asking) = state.asking.as_mut().filter(|_| editing) {
            if asking.focus {
                child.memory_mut(|m| m.request_focus(field.id()));
                asking.focus = false;
            } else if child.memory(|m| m.has_focus(field.id())) && swap_signs(&child, asking) {
                outcome.repaint = true;
            }
            if std::mem::take(&mut asking.select_value) {
                select_value(child.ctx(), asking);
            }
        }
        let before_show = state
            .asking
            .as_ref()
            .filter(|_| editing)
            .map(|a| a.text.clone());
        let text: &mut String = match state.asking.as_mut().filter(|_| editing) {
            Some(asking) => &mut asking.text,
            None => &mut idle_text,
        };
        let response = egui::TextEdit::singleline(text)
            .id(field.id())
            .font(font.clone())
            .text_color(ink)
            .frame(egui::Frame::NONE)
            .desired_width(field_rect.width())
            .interactive(interactive)
            .show(&mut child)
            .response
            .response;
        texts[slot].clone_from(text);

        if editing {
            let focused = response.has_focus();
            let Some(asking) = state.asking.as_mut() else {
                continue;
            };
            if asking.had_focus && !focused {
                // The keyboard went elsewhere: back to the applied text.
                state.asking = None;
                texts[slot] = field.of(&applied).unwrap_or_default().to_owned();
                outcome.repaint = true;
                continue;
            }
            asking.had_focus = focused;
            if before_show.as_deref() != Some(asking.text.as_str()) {
                // The field typed something itself: an `=` now extends nothing.
                asking.last_sign = None;
            }
            if try_text(doc, asking) {
                outcome.repaint = true;
            }
            // Enter, resolved after the field has taken this frame's typing, so
            // what is applied is the text the field holds now.
            if std::mem::take(&mut asking.enter) {
                if let Some((_, view)) = state.entered(&applied) {
                    state.asking = None;
                    texts[slot] = field.of(&view).unwrap_or_default().to_owned();
                    doc.apply_foot_view(view);
                    child.memory_mut(|m| m.surrender_focus(field.id()));
                    outcome.repaint = true;
                }
            }
        } else if interactive && response.has_focus() {
            state.ask(field, idle_text);
            if let Some(asking) = state.asking.as_mut() {
                asking.focus = false;
                asking.had_focus = true;
                try_text(doc, asking);
            }
            outcome.repaint = true;
        }
    }

    let [where_text, order_text] = texts;
    let [where_dot, order_dot] = dots;
    state.drawn = Some(FootDrawn {
        rect,
        where_text,
        order_text,
        count,
        where_dot,
        order_dot,
    });
    outcome
}
