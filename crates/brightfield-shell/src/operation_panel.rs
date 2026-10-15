//! **The operation panel**: a pane of the Inspector rail that explains the
//! operation in hand while the grid's foot row asks one.
//!
//! While `where` is asked the operation in hand is `filter-rows`, and while
//! `order by` is asked it is `sort-rows`. The panel draws what arc says the
//! operation reads and takes, each with arc's own sentence, beside the value
//! brightfield holds for it and where that value came from. arc's words are
//! the panel's words; the values, the keys and where each value came from are
//! brightfield's.
//!
//! **The description is the window's to fetch.** It is asked through the
//! window's runner and kept for the session in [`crate::operation::Descriptions`],
//! which outlives any one document, so the window reads the state of the one in
//! hand each frame and hands it to the document ([`ChartDoc::set_described`])
//! before the panes draw. The pane only reads it.
//!
//! **The panel changes no value.** Editing a value here is later work.

use brightfield_keys::BindingContext;
use brightfield_workbench::{
    chrome, EmptyState, Icon, Item, ItemCtx, ItemId, ItemSpec, Slot, Subject, Verb,
};
use meridian_design::{semantic, spacing};

use crate::app::ChartDoc;
use crate::design::Mode;
use crate::grid_foot::FootField;
use crate::operation::{OperationDescription, Parameter};
use crate::protocol::ui_font;

/// The panel's stable id.
pub const OPERATION: ItemId = ItemId::new("operation-inspector");

/// The icon is a name, resolved to paint when the icon set lands.
const ICON_OPERATION: Icon = Icon("filter");

/// Where a value the panel shows came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueFrom {
    /// The node whose rows the grid shows.
    NodeInView,
    /// The grid's cursor.
    Cursor,
    /// The field at the grid's foot, while it is being asked.
    Asked,
}

impl ValueFrom {
    /// The words the panel draws for it.
    #[must_use]
    pub const fn words(self) -> &'static str {
        match self {
            Self::NodeInView => "the node in view",
            Self::Cursor => "the cursor",
            Self::Asked => "asked",
        }
    }
}

/// The operation the grid's foot row is asking, and the values brightfield
/// holds for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationInHand {
    /// arc's long name for it: `filter-rows` or `sort-rows`.
    pub long_name: &'static str,
    /// The table the node in view makes.
    pub table: String,
    /// Where [`Self::table`] came from.
    pub table_from: ValueFrom,
    /// The column the cursor was on when its key opened the field, or `None`
    /// when the field was opened another way.
    pub column: Option<String>,
    /// Where [`Self::column`] came from.
    pub column_from: ValueFrom,
    /// The field's text, or `None` while it is empty.
    pub value: Option<String>,
}

/// The operation a field of the foot row asks, by arc's long name.
#[must_use]
pub const fn operation_of(field: FootField) -> &'static str {
    match field {
        FootField::Where => "filter-rows",
        FootField::OrderBy => "sort-rows",
    }
}

/// The key of the operation's parameter a field of the foot row holds.
#[must_use]
pub const fn parameter_of(field: FootField) -> &'static str {
    match field {
        FootField::Where => "where",
        FootField::OrderBy => "order_by",
    }
}

/// Where the description of the operation in hand stands, as the window last
/// handed it to the document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Described {
    /// No operation is in hand.
    #[default]
    Nothing,
    /// arc has been asked and has not answered.
    Reading,
    /// arc's description.
    Ready(OperationDescription),
    /// Why there is none: arc's refusal, or why it could not be asked.
    Missing(String),
}

/// What the panel reads off the document: the table the node in view makes,
/// and the description the window last handed it.
#[derive(Clone, Debug, Default)]
pub struct OperationState {
    /// The table the node in view makes, set when the documents are joined.
    pub(crate) table: Option<String>,
    /// The description of the operation in hand.
    pub(crate) described: Described,
}

impl ChartDoc {
    /// **The operation the foot row is asking**, or `None` while neither field
    /// is being edited or no table is in view.
    #[must_use]
    pub fn operation_in_hand(&self) -> Option<OperationInHand> {
        let (field, text) = self.foot_asking()?;
        let table = self.operation.table.clone()?;
        let column = self.grid_foot().asked_column().map(str::to_owned);
        Some(OperationInHand {
            long_name: operation_of(field),
            table,
            table_from: ValueFrom::NodeInView,
            column_from: if column.is_some() {
                ValueFrom::Cursor
            } else {
                ValueFrom::Asked
            },
            column,
            value: (!text.is_empty()).then(|| text.to_owned()),
        })
    }

    /// Say which table the node in view makes: what the panel names as the
    /// table in hand.
    pub fn set_operation_table(&mut self, table: Option<String>) {
        self.operation.table = table;
    }

    /// Hand the panel the state of the description of the operation in hand.
    pub fn set_described(&mut self, described: Described) {
        self.operation.described = described;
    }

    /// The state of the description the panel draws from.
    #[must_use]
    pub const fn described(&self) -> &Described {
        &self.operation.described
    }
}

/// The panel's registry entry, appended to the chart view's registry
/// ([`crate::app::chart_registry`]).
///
/// It is shown by the key that asks, not by a verb of its own; the toggle it
/// names is the Inspector rail's, the rail it is a pane of, because a pane
/// outside the centre must name the verb that shows and hides it.
#[must_use]
pub fn operation_spec() -> ItemSpec<ChartDoc> {
    ItemSpec {
        id: OPERATION,
        slot: Slot::CentreTab,
        toggle: Some(Verb::new("toggle-controls-rail")),
        make: || Box::new(OperationPane),
    }
}

/// The pane. A unit struct: what it draws is the document's.
pub struct OperationPane;

impl Item<ChartDoc> for OperationPane {
    fn item_id(&self) -> ItemId {
        OPERATION
    }

    /// Empty while no operation is in hand: it names the keys that put one
    /// there.
    fn empty_state(&self, doc: &ChartDoc) -> Option<EmptyState> {
        doc.operation_in_hand().is_none().then(|| {
            EmptyState::new(
                ICON_OPERATION,
                "No operation in hand",
                "Press w on a cell of the grid to filter its rows, or o to sort \
                 them, and the operation is explained here.",
            )
        })
    }

    fn describe(&self, _doc: &ChartDoc) -> Subject {
        Subject::new("Operation", ICON_OPERATION, BindingContext::Workspace)
    }

    fn ui(&mut self, doc: &mut ChartDoc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        let Some(hand) = doc.operation_in_hand() else {
            return;
        };
        let field = doc.foot_asking().map(|(field, _)| field);
        let mode = cx.mode;
        let sem = semantic(mode.is_dark());
        let primary = chrome::colour(sem.text.primary);
        let secondary = chrome::colour(sem.text.secondary);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(hand.long_name)
                        .font(ui_font())
                        .color(primary),
                );
                let description = match doc.described() {
                    Described::Ready(d) => d,
                    Described::Reading | Described::Nothing => {
                        ui.label(
                            egui::RichText::new(format!(
                                "Reading arc's description of {}\u{2026}",
                                hand.long_name
                            ))
                            .font(ui_font())
                            .color(secondary),
                        );
                        return;
                    }
                    Described::Missing(why) => {
                        ui.label(
                            egui::RichText::new(format!(
                                "No description of {}: {why}",
                                hand.long_name
                            ))
                            .font(ui_font())
                            .color(secondary),
                        );
                        return;
                    }
                };
                ui.label(
                    egui::RichText::new(&description.summary)
                        .font(ui_font())
                        .color(secondary),
                );

                ui.add_space(spacing::SPACE_4);
                caption(ui, mode, "TARGET");
                for role in &description.reads {
                    let (value, from) = match role.kind.as_str() {
                        "table" => (Some(hand.table.as_str()), hand.table_from),
                        "column" => (hand.column.as_deref(), hand.column_from),
                        _ => (None, ValueFrom::Asked),
                    };
                    row(
                        ui,
                        mode,
                        &role.name,
                        value,
                        value.map(|_| from),
                        &role.description,
                    );
                }

                ui.add_space(spacing::SPACE_4);
                caption(ui, mode, "VALUES");
                for parameter in &description.parameters {
                    // The field's own parameter reads as asked even while the
                    // field holds nothing.
                    let held = field.is_some_and(|f| parameter_of(f) == parameter.key);
                    row(
                        ui,
                        mode,
                        &parameter.key,
                        hand.value.as_deref().filter(|_| held),
                        held.then_some(ValueFrom::Asked),
                        &parameter.description,
                    );
                    offered(ui, mode, parameter);
                }
            });
    }
}

/// A small heading over a block of rows.
fn caption(ui: &mut egui::Ui, mode: Mode, text: &str) {
    let sem = semantic(mode.is_dark());
    ui.label(
        egui::RichText::new(text)
            .font(ui_font())
            .color(chrome::colour(sem.text.muted)),
    );
}

/// One thing the operation reads or takes: its name, the value brightfield
/// holds for it and where that came from, and arc's sentence for it.
fn row(
    ui: &mut egui::Ui,
    mode: Mode,
    name: &str,
    value: Option<&str>,
    from: Option<ValueFrom>,
    said: &str,
) {
    let sem = semantic(mode.is_dark());
    let muted = chrome::colour(sem.text.muted);
    ui.add_space(spacing::SPACE_2);
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(name).font(ui_font()).color(muted));
        if let Some(value) = value {
            ui.label(
                egui::RichText::new(value)
                    .font(ui_font())
                    .color(chrome::colour(sem.text.primary)),
            );
        }
        if let Some(from) = from {
            ui.label(
                egui::RichText::new(from.words())
                    .font(ui_font())
                    .color(muted),
            );
        }
    });
    ui.label(
        egui::RichText::new(said)
            .font(ui_font())
            .color(chrome::colour(sem.text.secondary)),
    );
}

/// The words arc offers for a value, each with the sign it is written as.
fn offered(ui: &mut egui::Ui, mode: Mode, parameter: &Parameter) {
    if parameter.offered.is_empty() {
        return;
    }
    let sem = semantic(mode.is_dark());
    ui.horizontal_wrapped(|ui| {
        for offer in &parameter.offered {
            ui.label(
                egui::RichText::new(&offer.word)
                    .font(ui_font())
                    .color(chrome::colour(sem.text.primary)),
            );
            ui.label(
                egui::RichText::new(&offer.sign)
                    .font(ui_font())
                    .color(chrome::colour(sem.text.muted)),
            );
            ui.add_space(spacing::SPACE_2);
        }
    });
}
