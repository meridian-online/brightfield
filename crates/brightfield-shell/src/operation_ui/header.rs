//! A column's header as a place to act: the parts an operation can start
//! from, the menu of them, the new-column cell, and the field an operation
//! asks with in place.
//!
//! **Parts.** A header cell has a name, and on the loaded table's band a type
//! and a count of missing values ([`HeaderPart`]). After the grid has drawn
//! its table, [`header_pass`] reads where those parts were drawn and senses a
//! gesture on each part some operation starts from: a double-click on the
//! name, a click on the type or on the missing count. A secondary click on a
//! cell opens a menu of each operation that starts from a header, with its
//! key, an unavailable one greyed with the sentence saying what it needs. A
//! part no operation starts from is not sensed, so a gesture on a header that is sensed starts an operation or opens the menu (`a_part_no_operation_starts_from_senses_nothing` in `tests/operation_header.rs`).
//!
//! The operations that start from a header are the built ones
//! ([`super::built`]), read each frame, so an operation joins the header's
//! menu when its slot is filled and no list of them is kept here.
//!
//! **The new-column cell** stands after the last column while an operation
//! that starts there is built and no field is open.
//!
//! **A field in place.** While the operation being made asks on a header
//! ([`InPlace`]), the cell draws the value being typed over the part the
//! operation names, and what the operation suggests for it under that. The
//! window reads the keys ([`crate::window::MeridianApp`]'s operation host);
//! this module draws and reports.
//!
//! What a gesture did is left on the document as a [`HeaderHit`], for the
//! window to start the operation it names.

use meridian_design::{semantic, spacing};

use super::open::{availability, OperationContext};
use super::{label, HeaderPart, InPlace, OperationUi, Start};
use crate::app::ChartDoc;
use crate::data_grid::TableDrawn;
use crate::design::Mode;
use brightfield_workbench::chrome;

/// What was done to a part of a header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    /// A click.
    Click,
    /// A double-click.
    DoubleClick,
    /// A secondary click.
    Secondary,
}

/// A gesture on a part of a header, or a choice from a header's menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderHit {
    /// The column; `None` for the new-column cell.
    pub column: Option<String>,
    /// The part.
    pub part: HeaderPart,
    /// The gesture.
    pub gesture: Gesture,
    /// The verb of the operation chosen from the header's menu, for a choice
    /// from it.
    pub verb: Option<&'static str>,
}

/// One row of a header's menu, as drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRowDrawn {
    /// The operation's words.
    pub words: String,
    /// Its key, when it has one.
    pub key: Option<String>,
    /// The sentence saying what it needs, when it cannot start on this column.
    pub sentence: Option<String>,
}

/// The field an operation asks with in place, as drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldDrawn {
    /// Where it was drawn.
    pub rect: egui::Rect,
    /// The text in it.
    pub text: String,
    /// What it offers under it, in order.
    pub suggestions: Vec<String>,
    /// Which of them the arrow keys are on.
    pub selected: Option<usize>,
    /// Whether its text is selected whole, so the first letter typed replaces it.
    pub whole: bool,
}

/// What the header pass sensed and drew on a frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeaderDrawn {
    /// Each part sensed: its column, the part, and where.
    pub parts: Vec<(String, HeaderPart, egui::Rect)>,
    /// Each header cell a secondary click opens the menu on.
    pub cells: Vec<(String, egui::Rect)>,
    /// The new-column cell, while it is drawn.
    pub new_column: Option<egui::Rect>,
    /// The field in place, while one is open.
    pub field: Option<FieldDrawn>,
    /// The open menu's rows.
    pub menu: Vec<MenuRowDrawn>,
}

/// The header's state on the document.
#[derive(Clone, Debug, Default)]
pub struct HeaderState {
    /// The operations that start from a header, when a test has given its own
    /// in place of the built ones.
    ops: Option<Vec<&'static OperationUi>>,
    /// What the window holds, as the availability of a menu row asks.
    context: Option<OperationContext>,
    /// A gesture not yet taken by the window.
    hit: Option<HeaderHit>,
    /// What the last pass sensed and drew.
    drawn: HeaderDrawn,
}

impl HeaderState {
    /// Forget what the last pass drew, at the head of a frame's grid.
    pub(crate) fn clear_drawn(&mut self) {
        self.drawn = HeaderDrawn::default();
    }

    /// The operations a header offers: the built ones, or a test's own.
    fn ops(&self) -> Vec<&'static OperationUi> {
        self.ops.clone().unwrap_or_else(|| super::built().collect())
    }
}

impl ChartDoc {
    /// Take the gesture a header's part or its menu reported, if one did.
    pub fn take_header_hit(&mut self) -> Option<HeaderHit> {
        self.header.hit.take()
    }

    /// What the header pass sensed and drew on the last frame the grid drew.
    #[must_use]
    pub fn header_drawn(&self) -> &HeaderDrawn {
        &self.header.drawn
    }

    /// The operations a header offers.
    #[must_use]
    pub fn header_operations(&self) -> Vec<&'static OperationUi> {
        self.header.ops()
    }

    /// Offer `ops` from a header in place of the built operations.
    #[doc(hidden)]
    pub fn set_header_operations(&mut self, ops: Vec<&'static OperationUi>) {
        self.header.ops = Some(ops);
    }

    /// Whether some operation a header offers starts from a header.
    #[must_use]
    pub fn header_has_operations(&self) -> bool {
        self.header
            .ops()
            .iter()
            .any(|op| op.starts.iter().any(|s| matches!(s, Start::Header(_))))
    }

    /// Tell the header what the window holds, for its menu's rows.
    pub(crate) fn set_header_context(&mut self, context: Option<OperationContext>) {
        self.header.context = context;
    }

    /// **The operations the inspector's Column section offers on `column`**:
    /// each operation a header offers that starts somewhere on the grid (a
    /// header, a cell or the foot row), with its key and, when it cannot start
    /// on this column, the sentence the header's menu says.
    ///
    /// Read from the same operations and the same context as the header's
    /// menu, so the two places cannot disagree about what a column can do, and
    /// an operation joins the section when its slot is filled.
    #[must_use]
    pub fn column_operations(&self, column: &str) -> Vec<(&'static OperationUi, MenuRowDrawn)> {
        rows_for(
            &self.header.ops(),
            self.header.context.as_ref(),
            column,
            |start| matches!(start, Start::Header(_) | Start::Cell | Start::FootField(_)),
        )
    }

    /// Start the operation `verb` on `column` as a choice from that column's
    /// header menu does, for the window to take with the frame's other header
    /// gestures.
    pub fn choose_column_operation(&mut self, column: &str, verb: &'static str) {
        self.header.hit = Some(HeaderHit {
            column: Some(column.to_string()),
            part: HeaderPart::Menu,
            gesture: Gesture::Click,
            verb: Some(verb),
        });
    }
}

/// The width of the new-column cell, and of the gutter the grid leaves for it.
const NEW_COLUMN_WIDTH: f32 = spacing::SPACE_7;

/// **The width the grid leaves after its columns for the new-column cell**, while a
/// built operation starts from that cell, so that on a table wider than its pane the
/// cell stands beside the last column's header rather than over it. Nothing otherwise.
#[must_use]
pub fn new_column_gutter(doc: &ChartDoc) -> f32 {
    if starting_from(&doc.header.ops(), HeaderPart::NewColumn)
        .next()
        .is_some()
    {
        NEW_COLUMN_WIDTH
    } else {
        0.0
    }
}

/// The operations of `ops` that start from `part`.
fn starting_from<'a>(
    ops: &'a [&'static OperationUi],
    part: HeaderPart,
) -> impl Iterator<Item = &'static OperationUi> + 'a {
    ops.iter()
        .copied()
        .filter(move |op| op.starts.contains(&Start::Header(part)))
}

/// The one operation of `ops` that starts from `part`, when exactly one does.
#[must_use]
pub fn the_one_from(
    ops: &[&'static OperationUi],
    part: HeaderPart,
) -> Option<&'static OperationUi> {
    let mut from = starting_from(ops, part);
    let first = from.next()?;
    from.next().is_none().then_some(first)
}

/// The key the registry gives the verb `verb`, built or not.
fn key_of(verb: &str) -> Option<String> {
    primary_key_of(verb).map(str::to_string)
}

/// The keystrokes of the first binding the registry gives the verb `verb`,
/// built or not, as the registry spells them (`shift-h`).
#[must_use]
pub fn primary_key_of(verb: &str) -> Option<&'static str> {
    brightfield_keys::registry()
        .iter()
        .find(|entry| entry.longname == verb)
        .and_then(|entry| entry.primary_key())
        .or_else(|| {
            brightfield_keys::operations::all_operation_entries()
                .into_iter()
                .find(|(_, entry)| entry.longname == verb)
                .and_then(|(_, entry)| entry.primary_key())
        })
}

/// The rows of a header's menu for `column`: each operation that starts from a
/// header, with its key and, when it cannot start on this column, why.
fn menu_rows(
    ops: &[&'static OperationUi],
    context: Option<&OperationContext>,
    column: &str,
) -> Vec<(&'static OperationUi, MenuRowDrawn)> {
    rows_for(ops, context, column, |start| {
        matches!(start, Start::Header(_))
    })
}

/// The rows of `ops` that start from a place `from` admits, for `column`, as
/// [`menu_rows`] draws them.
fn rows_for(
    ops: &[&'static OperationUi],
    context: Option<&OperationContext>,
    column: &str,
    from: impl Fn(&Start) -> bool,
) -> Vec<(&'static OperationUi, MenuRowDrawn)> {
    ops.iter()
        .copied()
        .filter(|op| op.starts.iter().any(&from))
        .map(|op| {
            let sentence = context.and_then(|cx| {
                availability(op, cx, Some(column))
                    .sentence()
                    .map(str::to_string)
            });
            (
                op,
                MenuRowDrawn {
                    words: if op.menu_words.is_empty() {
                        label(op.long_name)
                    } else {
                        op.menu_words.to_string()
                    },
                    key: key_of(op.verb),
                    sentence,
                },
            )
        })
        .collect()
}

/// **The header pass**: over the table `drawn` this frame, whose columns are
/// `names`, sense the parts operations start from, draw the new-column cell,
/// the open menu and the field in place, and leave what happened on `doc`.
pub fn header_pass(
    ui: &mut egui::Ui,
    doc: &mut ChartDoc,
    drawn: &TableDrawn,
    names: &[String],
    mode: Mode,
) {
    let ops = doc.header.ops();
    let field = doc.making().and_then(|making| {
        let field = making.spec.field?;
        let focus = making.draft.focus.or_else(|| field.keys.first().copied())?;
        Some((
            field,
            making.column.clone(),
            making.draft.value(focus).unwrap_or_default().to_string(),
            making.suggestions.clone(),
            making.suggestion,
            making.whole,
        ))
    });
    // A correction of a kept step began over that step's own table, which holds the
    // column it added already.
    let corrects = doc.making().is_some_and(|making| making.corrects.is_some());
    let begun_columns: Vec<String> = doc
        .making()
        .map(|making| making.with.columns.clone())
        .unwrap_or_default();
    let offers = ops
        .iter()
        .any(|op| op.starts.iter().any(|s| matches!(s, Start::Header(_))));
    let mut out = HeaderDrawn::default();
    if !offers && field.is_none() {
        doc.header.drawn = out;
        return;
    }
    let sem = semantic(mode.is_dark());
    let context = doc.header.context.clone();
    let mut hit = None;
    let id = ui.id().with("operation-header");
    // Where each part of each column was drawn: the band's own record, or the
    // whole cell as the name of a plain header. Each with the column's index,
    // which is how a field in place finds its column again once the preview
    // has given it another name.
    let mut parts: Vec<(usize, String, HeaderPart, egui::Rect)> = Vec::new();
    let mut cells: Vec<(usize, String, egui::Rect)> = Vec::new();
    for (col, rect, clip) in &drawn.header_cells {
        let Some(name) = names.get(*col) else {
            continue;
        };
        let visible = rect.intersect(*clip);
        if !visible.is_positive() {
            continue;
        }
        cells.push((*col, name.clone(), visible));
        match drawn.band.iter().find(|band| band.column == *col) {
            Some(band) => {
                for (part, at) in [
                    (HeaderPart::Name, Some(band.parts.name)),
                    (HeaderPart::Type, band.parts.kind),
                    (HeaderPart::Missing, Some(band.parts.missing)),
                ] {
                    if let Some(at) = at.map(|at| at.intersect(*clip)) {
                        if at.is_positive() {
                            parts.push((*col, name.clone(), part, at));
                        }
                    }
                }
            }
            None => parts.push((*col, name.clone(), HeaderPart::Name, visible)),
        }
    }

    if field.is_none() && offers {
        // Each cell is sensed once, and the part is read from where the pointer
        // was: a secondary click opens the menu wherever on the cell it lands,
        // a double-click on the name or a click on the type or the missing
        // count starts the operation that starts there.
        for (col, name, cell) in &cells {
            let response = ui.interact(*cell, id.with(("cell", *col)), egui::Sense::click());
            let sensed: Vec<(HeaderPart, egui::Rect)> = parts
                .iter()
                .filter(|(c, _, part, _)| c == col && starting_from(&ops, *part).next().is_some())
                .map(|(_, _, part, at)| (*part, *at))
                .collect();
            let under = response.interact_pointer_pos().and_then(|at| {
                // The last part that holds the pointer: a part drawn inside
                // another's rect is the nearer one.
                sensed.iter().rev().find(|(_, rect)| rect.contains(at))
            });
            let gesture = match under {
                Some((HeaderPart::Name, _)) if response.double_clicked() => {
                    Some((HeaderPart::Name, Gesture::DoubleClick))
                }
                Some((part @ (HeaderPart::Type | HeaderPart::Missing), _))
                    if response.clicked() =>
                {
                    Some((*part, Gesture::Click))
                }
                _ => None,
            };
            if let Some((part, gesture)) = gesture {
                hit = Some(HeaderHit {
                    column: Some(name.clone()),
                    part,
                    gesture,
                    verb: None,
                });
            }
            if response.secondary_clicked() {
                hit = Some(HeaderHit {
                    column: Some(name.clone()),
                    part: HeaderPart::Menu,
                    gesture: Gesture::Secondary,
                    verb: None,
                });
            }
            response.context_menu(|ui| {
                let rows = menu_rows(&ops, context.as_ref(), name);
                out.menu = rows.iter().map(|(_, row)| row.clone()).collect();
                for (op, row) in rows {
                    let text = match &row.key {
                        Some(key) => format!("{}  {key}", row.words),
                        None => row.words.clone(),
                    };
                    let enabled = row.sentence.is_none();
                    if ui.add_enabled(enabled, egui::Button::new(text)).clicked() {
                        hit = Some(HeaderHit {
                            column: Some(name.clone()),
                            part: HeaderPart::Menu,
                            gesture: Gesture::Click,
                            verb: Some(op.verb),
                        });
                        ui.close();
                    }
                    if let Some(sentence) = &row.sentence {
                        ui.label(
                            egui::RichText::new(sentence).color(chrome::colour(sem.text.muted)),
                        );
                    }
                }
            });
            // The cell senses a click, and the column's name it draws is the name a
            // stranger reads it by.
            doc.controls
                .push(chrome::NamedControl::labelled(*cell, name.clone()));
            out.cells.push((name.clone(), *cell));
            for (part, at) in sensed {
                out.parts.push((name.clone(), part, at));
            }
        }
    }

    // The new-column cell: one header row tall and as wide as the gutter the grid
    // leaves for it, after the table's last column where that leaves it inside the
    // pane, and in the gutter at the pane's trailing edge otherwise, so it is in reach
    // however wide the table is and covers no column's header
    // (`the_new_column_cell_stands_beside_a_wide_tables_last_header`). The field it
    // opens is as wide as a header row is tall.
    let room = ui.clip_rect();
    let width = NEW_COLUMN_WIDTH;
    let side = drawn.header_height.max(spacing::SPACE_6);
    let row = drawn.header_cells.first().map(|(_, rect, _)| *rect);
    let after_last = drawn
        .header_cells
        .iter()
        .find(|(col, ..)| *col + 1 == drawn.columns)
        .map(|(_, rect, _)| rect.right());
    let last = row;
    let new_column = row.map(|row| {
        let left = after_last
            .unwrap_or(f32::INFINITY)
            .min(room.right() - width);
        egui::Rect::from_min_size(egui::pos2(left, row.top()), egui::vec2(width, row.height()))
    });
    if field.is_none() && starting_from(&ops, HeaderPart::NewColumn).next().is_some() {
        if let Some(rect) = new_column {
            ui.painter()
                .rect_filled(rect, 0.0, chrome::colour(sem.surfaces.raised));
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "+",
                egui::FontId::proportional(spacing::SPACE_5),
                chrome::colour(sem.text.muted),
            );
            let response = ui.interact(rect, id.with("new-column"), egui::Sense::click());
            if response.clicked() {
                hit = Some(HeaderHit {
                    column: None,
                    part: HeaderPart::NewColumn,
                    gesture: Gesture::Click,
                    verb: None,
                });
            }
            // It draws only a `+`, so it is named by what it does.
            doc.controls.push(chrome::NamedControl::labelled(
                rect,
                "Add a column".to_string(),
            ));
            out.new_column = Some(rect);
        }
    }

    // The field in place, over the part the operation names.
    if let Some((InPlace { part, .. }, column, text, suggestions, selected, whole)) = field {
        let at = if part == HeaderPart::NewColumn {
            // Once the preview holds the new column, the table's last, over its header, so
            // its values fill under the field; until then, in the new-column cell.
            let added = (corrects || drawn.columns > begun_columns.len())
                .then(|| {
                    cells
                        .iter()
                        .find(|(col, ..)| *col + 1 == drawn.columns)
                        .map(|(.., at)| *at)
                })
                .flatten();
            // The cell is narrow, so the field it opens reaches back over the table, within
            // the pane, until the new column comes.
            let field = new_column.map(|cell| {
                let left = cell.left().min(room.right() - side).max(room.left());
                egui::Rect::from_min_size(
                    egui::pos2(left, cell.top()),
                    egui::vec2(side.min(room.width()), cell.height()),
                )
            });
            added.or(field).or(last)
        } else {
            // By the column's place in the table the draft began over, and not by
            // its name: a rename's preview has given the column another.
            let index = column
                .as_ref()
                .and_then(|column| begun_columns.iter().position(|c| c == column));
            parts
                .iter()
                .find(|(col, _, p, _)| Some(*col) == index && *p == part)
                .map(|(.., at)| *at)
                .or_else(|| {
                    cells
                        .iter()
                        .find(|(col, ..)| Some(*col) == index)
                        .map(|(.., at)| *at)
                })
        };
        if let Some(at) = at {
            let painter = ui.painter();
            painter.rect_filled(at, 0.0, chrome::colour(sem.surfaces.sunken));
            let font = egui::FontId::proportional(spacing::SPACE_5);
            let galley = painter.layout_no_wrap(
                text.clone(),
                font.clone(),
                chrome::colour(sem.text.primary),
            );
            let shown = egui::Rect::from_min_size(
                egui::pos2(at.left(), at.center().y - galley.size().y / 2.0),
                galley.size(),
            );
            if whole {
                // The text selected whole, under the editor's selection wash and with no
                // caret, as a text field draws a selection.
                painter.rect_filled(shown, 0.0, chrome::colour(sem.editor.selection));
            }
            painter.galley(shown.min, galley, chrome::colour(sem.text.primary));
            if !whole {
                // The caret, after the text.
                let end = shown.right();
                painter.line_segment(
                    [
                        egui::pos2(end, at.top() + spacing::SPACE_1),
                        egui::pos2(end, at.bottom() - spacing::SPACE_1),
                    ],
                    egui::Stroke::new(1.0, chrome::colour(sem.text.primary)),
                );
            }
            // What the operation offers, listed under the field.
            let row = at.height();
            for (i, suggestion) in suggestions.iter().enumerate() {
                let line = egui::Rect::from_min_size(
                    egui::pos2(at.left(), at.bottom() + row * i as f32),
                    egui::vec2(at.width().max(drawn.header_height), row),
                );
                let ground = if selected == Some(i) {
                    sem.surfaces.sunken
                } else {
                    sem.surfaces.raised
                };
                painter.rect_filled(line, 0.0, chrome::colour(ground));
                painter.text(
                    line.left_center(),
                    egui::Align2::LEFT_CENTER,
                    suggestion,
                    font.clone(),
                    chrome::colour(sem.text.secondary),
                );
            }
            out.field = Some(FieldDrawn {
                rect: at,
                text,
                suggestions,
                selected,
                whole,
            });
        }
    }

    if hit.is_some() {
        doc.header.hit = hit;
        ui.ctx().request_repaint();
    }
    doc.header.drawn = out;
}
