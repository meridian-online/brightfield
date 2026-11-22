//! What a command needs before it can act, and the sentence it says when one
//! of those inputs is missing.
//!
//! A verb declares its needs in the registry ([`VerbEntry::needs`]). The shell
//! describes what the window holds as a [`Situation`], and [`availability`]
//! reads one against the other. The palette and the menus both ask it, so a
//! command that cannot act says the same sentence wherever it is listed.

use crate::registry::{registry, VerbEntry};

/// One input a command needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// At least this many sources in the Protocol.
    Sources(u8),
    /// A column of this kind under the Outline's cursor.
    Column(ColumnKind),
    /// Something of this kind selected, open or on the canvas.
    Selected(Selected),
    /// An edition or plan that grants the command.
    Grant(Grant),
    /// Something the window holds that is none of the above.
    Condition(Condition),
}

/// The kind of column a command acts on, in the workbench's field-type words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    /// A column of whichever kind.
    Any,
    /// A number that measures.
    Quantitative,
    /// A category.
    Categorical,
    /// A date or a time.
    Temporal,
    /// True or false.
    Boolean,
}

/// What a command needs selected, open or on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selected {
    /// A Protocol is open: the window is past the front door.
    Protocol,
    /// The Protocol's graph is on the canvas.
    Graph,
    /// A chart is on the canvas.
    Chart,
    /// A table whose rows the grid shows.
    Rows,
    /// The grid's cursor is on a cell.
    GridCell,
    /// An asset is selected on the spine.
    Asset,
    /// The selected asset's step can be removed.
    Step,
    /// The model is open.
    Model,
    /// A table is selected in the model.
    ModelTable,
    /// A join is selected in the model.
    ModelJoin,
}

/// An edition or plan that grants a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    /// The full edition of Brightfield.
    FullEdition,
}

/// Something the window holds that a command depends on and that is neither a
/// count of sources, a column, a selection nor a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    /// The chart holds a selection.
    SelectionHeld,
    /// A column was kept from the shelf since the last Save.
    EditToTakeBack,
    /// A change is not saved yet.
    UnsavedWork,
    /// The window holds a Protocol that Run acts on: one made from a data file
    /// or a folder, one whose panels it holds as tiles, or the one the model
    /// pane is over. A chart spec and a shipped start are neither.
    ProtocolToRun,
    /// The window holds something Save the spec writes: a Protocol made from a
    /// data file or a folder, or a panel open from its tile.
    ProtocolToSave,
    /// The Protocol is a folder on disk, as one opened or made is. A chart
    /// spec, and a start that ships as a graph alone, have no folder.
    ProtocolFolder,
    /// The Protocol is trusted.
    Trusted,
    /// The Protocol is one Brightfield made, which sources can be added to
    /// and which can be renamed.
    TakesSources,
    /// A data file is open and can be read again.
    FileToReload,
    /// The chart's shelf is drawn.
    ShelfDrawn,
}

impl Need {
    /// The sentence a command says when this need is unmet and it declares no
    /// sentence of its own. `{command}` stands for the command's label.
    #[must_use]
    pub const fn default_sentence(self) -> &'static str {
        match self {
            Need::Sources(2) => "{command} needs two sources.",
            Need::Sources(_) => "{command} needs more sources.",
            Need::Column(ColumnKind::Any) => "{command} needs a column under the Outline's cursor.",
            Need::Column(ColumnKind::Quantitative) => {
                "{command} needs a quantitative column under the Outline's cursor."
            }
            Need::Column(ColumnKind::Categorical) => {
                "{command} needs a categorical column under the Outline's cursor."
            }
            Need::Column(ColumnKind::Temporal) => {
                "{command} needs a date or time column under the Outline's cursor."
            }
            Need::Column(ColumnKind::Boolean) => {
                "{command} needs a true or false column under the Outline's cursor."
            }
            Need::Selected(Selected::Protocol) => "{command} needs a Protocol open.",
            Need::Selected(Selected::Graph) => {
                "{command} needs the Protocol's graph on the canvas."
            }
            Need::Selected(Selected::Chart) => "{command} needs a chart on the canvas.",
            Need::Selected(Selected::Rows) => "{command} needs a table's rows.",
            Need::Selected(Selected::GridCell) => "{command} needs the grid's cursor on a cell.",
            Need::Selected(Selected::Asset) => "{command} needs an asset selected on the spine.",
            Need::Selected(Selected::Step) => {
                "{command} needs a step selected on the spine that can be removed."
            }
            Need::Selected(Selected::Model) => "{command} needs the model open.",
            Need::Selected(Selected::ModelTable) => {
                "{command} needs a table selected in the model."
            }
            Need::Selected(Selected::ModelJoin) => "{command} needs a join selected in the model.",
            Need::Grant(Grant::FullEdition) => "{command} needs the full edition of Brightfield.",
            Need::Condition(Condition::SelectionHeld) => {
                "{command} needs a selection on the chart."
            }
            Need::Condition(Condition::EditToTakeBack) => {
                "{command} needs a column kept from the shelf since the last Save."
            }
            Need::Condition(Condition::UnsavedWork) => {
                "{command} needs a change that is not saved yet."
            }
            Need::Condition(Condition::ProtocolToRun) => {
                "{command} needs a Protocol to act on: open one, or open a data file or a folder."
            }
            Need::Condition(Condition::ProtocolToSave) => {
                "{command} needs a Protocol to save: open a data file or a folder, or open a \
                 panel from its tile."
            }
            Need::Condition(Condition::ProtocolFolder) => {
                "{command} needs a Protocol opened from a folder."
            }
            Need::Condition(Condition::Trusted) => {
                "{command} needs the Protocol trusted: trust it from the band over its canvas."
            }
            Need::Condition(Condition::TakesSources) => {
                "Sources are added to Protocols Brightfield made, and this one was not."
            }
            Need::Condition(Condition::FileToReload) => "{command} needs a data file open.",
            Need::Condition(Condition::ShelfDrawn) => "{command} needs the chart's shelf.",
        }
    }
}

/// One declared need, with the sentence the command says when it is unmet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Requirement {
    /// The input needed.
    pub need: Need,
    /// The sentence said without it. `{command}` stands for the command's
    /// label.
    pub missing: &'static str,
}

impl Requirement {
    /// A need with a sentence of the command's own.
    #[must_use]
    pub const fn new(need: Need, missing: &'static str) -> Self {
        Self { need, missing }
    }

    /// A need with its default sentence ([`Need::default_sentence`]).
    #[must_use]
    pub const fn of(need: Need) -> Self {
        Self {
            need,
            missing: need.default_sentence(),
        }
    }

    /// The sentence with `{command}` replaced by `label`.
    #[must_use]
    pub fn sentence(&self, label: &str) -> String {
        self.missing.replace("{command}", label)
    }
}

/// What the window holds, as far as a command's needs can ask.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Situation {
    /// How many sources the Protocol has.
    pub sources: usize,
    /// What is selected, open or on the canvas.
    pub selected: Vec<Selected>,
    /// The kind of the column under the Outline's cursor, if it is on one.
    pub column: Option<ColumnKind>,
    /// The grants the running edition holds.
    pub grants: Vec<Grant>,
    /// The conditions that hold.
    pub conditions: Vec<Condition>,
}

impl Situation {
    /// A situation in which each of `needs` is met and nothing else is held.
    #[must_use]
    pub fn meeting(needs: &[Requirement]) -> Self {
        let mut situation = Self::default();
        for requirement in needs {
            match requirement.need {
                Need::Sources(count) => {
                    situation.sources = situation.sources.max(usize::from(count));
                }
                Need::Column(kind) => situation.column = Some(kind),
                Need::Selected(selected) => situation.selected.push(selected),
                Need::Grant(grant) => situation.grants.push(grant),
                Need::Condition(condition) => situation.conditions.push(condition),
            }
        }
        situation
    }

    /// Whether `need` is met here. A need for a column of any kind is met by a
    /// column of whichever kind; a typed need only by that kind.
    #[must_use]
    pub fn meets(&self, need: Need) -> bool {
        match need {
            Need::Sources(count) => self.sources >= usize::from(count),
            Need::Column(ColumnKind::Any) => self.column.is_some(),
            Need::Column(kind) => self.column == Some(kind),
            Need::Selected(selected) => self.selected.contains(&selected),
            Need::Grant(grant) => self.grants.contains(&grant),
            Need::Condition(condition) => self.conditions.contains(&condition),
        }
    }
}

/// Whether a command can act, and what it says when it cannot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// Each declared need is met.
    Available,
    /// A need is unmet; the sentence names the first one.
    Missing(String),
}

impl Availability {
    /// Whether the command can act.
    #[must_use]
    pub fn is_available(&self) -> bool {
        matches!(self, Availability::Available)
    }

    /// The sentence said when the command cannot act.
    #[must_use]
    pub fn missing(&self) -> Option<&str> {
        match self {
            Availability::Available => None,
            Availability::Missing(sentence) => Some(sentence),
        }
    }
}

/// Read a verb's declared needs against what the window holds. The needs are
/// read in the order declared, and the first unmet one's sentence is the one
/// said.
#[must_use]
pub fn availability(verb: &VerbEntry, situation: &Situation) -> Availability {
    verb.needs
        .iter()
        .find(|requirement| !situation.meets(requirement.need))
        .map_or(Availability::Available, |requirement| {
            Availability::Missing(requirement.sentence(verb.label))
        })
}

/// The sentence the verb named `longname` declares for `need`, with its label
/// filled in. `None` when the registry has no such verb or the verb does not
/// declare that need, so a handler refusing for an undeclared reason has no
/// sentence to say.
#[must_use]
pub fn sentence(longname: &str, need: Need) -> Option<String> {
    let reg = registry();
    let verb = reg.iter().find(|v| v.longname == longname)?;
    verb.needs
        .iter()
        .find(|requirement| requirement.need == need)
        .map(|requirement| requirement.sentence(verb.label))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verb(longname: &str) -> VerbEntry {
        registry()
            .into_iter()
            .find(|v| v.longname == longname)
            .unwrap_or_else(|| panic!("{longname} is in the registry"))
    }

    /// A verb that declares one need, for the needs no registry verb declares
    /// yet.
    fn needing(need: Need) -> VerbEntry {
        let needs: &'static [Requirement] = Box::leak(Box::new([Requirement::of(need)]));
        VerbEntry {
            label: "Map",
            needs,
            ..verb("open")
        }
    }

    #[test]
    fn a_command_with_no_needs_is_available_on_the_front_door() {
        assert!(verb("open").needs.is_empty());
        assert!(availability(&verb("open"), &Situation::default()).is_available());
    }

    /// Run acts on a Protocol held as tiles or under the model pane, where
    /// Save the spec has nothing to write.
    #[test]
    fn save_waits_on_something_to_write_where_run_can_act() {
        let s = Situation {
            conditions: vec![Condition::ProtocolToRun, Condition::Trusted],
            ..Situation::default()
        };
        assert_eq!(
            availability(&verb("run-protocol"), &s),
            Availability::Available
        );
        assert_eq!(
            availability(&verb("save-spec"), &s),
            Availability::Missing(
                "Save the spec needs a Protocol to save: open a data file or a folder, or open \
                 a panel from its tile."
                    .into()
            )
        );
        let s = Situation {
            conditions: vec![Condition::ProtocolToSave],
            ..Situation::default()
        };
        assert_eq!(
            availability(&verb("save-spec"), &s),
            Availability::Available
        );
    }

    #[test]
    fn the_first_missing_need_says_its_sentence() {
        // Nothing held: Run's first need, a Protocol to act on.
        assert_eq!(
            availability(&verb("run-protocol"), &Situation::default()),
            Availability::Missing(
                "Run needs a Protocol to act on: open one, or open a data file or a folder.".into()
            )
        );
        // That one held: its second.
        let s = Situation {
            conditions: vec![Condition::ProtocolToRun],
            ..Situation::default()
        };
        assert_eq!(
            availability(&verb("run-protocol"), &s),
            Availability::Missing(
                "Run needs the Protocol trusted: trust it from the band over its canvas.".into()
            )
        );
        // Both held: it can act.
        let s = Situation {
            conditions: vec![Condition::ProtocolToRun, Condition::Trusted],
            ..Situation::default()
        };
        assert_eq!(
            availability(&verb("run-protocol"), &s),
            Availability::Available
        );
    }

    #[test]
    fn a_typed_column_need_is_met_only_by_that_kind() {
        let typed = needing(Need::Column(ColumnKind::Quantitative));
        let holding = |column| Situation {
            column,
            ..Situation::default()
        };
        assert_eq!(
            availability(&typed, &holding(None)),
            Availability::Missing(
                "Map needs a quantitative column under the Outline's cursor.".into()
            )
        );
        for other in [ColumnKind::Any, ColumnKind::Categorical] {
            assert!(!availability(&typed, &holding(Some(other))).is_available());
        }
        assert!(availability(&typed, &holding(Some(ColumnKind::Quantitative))).is_available());
        // A need for any column is met by a column of whichever kind, and by
        // no column at all it is not.
        let any = needing(Need::Column(ColumnKind::Any));
        assert!(!availability(&any, &holding(None)).is_available());
        for kind in [
            ColumnKind::Any,
            ColumnKind::Quantitative,
            ColumnKind::Categorical,
            ColumnKind::Temporal,
            ColumnKind::Boolean,
        ] {
            assert!(availability(&any, &holding(Some(kind))).is_available());
        }
    }

    #[test]
    fn a_sources_need_counts_at_least() {
        let joins = verb("organise-joins");
        let with = |sources| Situation {
            sources,
            ..Situation::default()
        };
        for unmet in [0, 1] {
            assert_eq!(
                availability(&joins, &with(unmet)),
                Availability::Missing(
                    "Organising joins needs a Protocol of two sources or more.".into()
                )
            );
        }
        for met in [2, 3] {
            assert!(availability(&joins, &with(met)).is_available());
        }
    }

    #[test]
    fn a_grant_need_is_met_by_the_edition_that_holds_it() {
        let granted = needing(Need::Grant(Grant::FullEdition));
        assert_eq!(
            availability(&granted, &Situation::default()),
            Availability::Missing("Map needs the full edition of Brightfield.".into())
        );
        let full = Situation {
            grants: vec![Grant::FullEdition],
            ..Situation::default()
        };
        assert!(availability(&granted, &full).is_available());
    }

    #[test]
    fn meeting_meets_every_listed_need() {
        let mut declaring = 0;
        for v in registry() {
            let met = Situation::meeting(v.needs);
            assert!(
                availability(&v, &met).is_available(),
                "{} is unavailable where its needs are met",
                v.longname
            );
            // And each need is one the verb waits on: with nothing held, the
            // verb says its first need's sentence.
            if let Some(first) = v.needs.first() {
                declaring += 1;
                assert_eq!(
                    availability(&v, &Situation::default()),
                    Availability::Missing(first.sentence(v.label)),
                    "{}",
                    v.longname
                );
            }
        }
        assert!(declaring > 0, "no verb declares a need");
    }

    #[test]
    fn sentence_fills_in_the_label() {
        assert_eq!(
            sentence("organise-joins", Need::Sources(2)).as_deref(),
            Some("Organising joins needs a Protocol of two sources or more.")
        );
        assert_eq!(
            sentence("zoom-in", Need::Selected(Selected::Chart)).as_deref(),
            Some("Zoom in needs a chart on the canvas.")
        );
        // A need the verb does not declare, and a verb the registry lacks.
        assert_eq!(sentence("zoom-in", Need::Sources(2)), None);
        assert_eq!(
            sentence("no-such-verb", Need::Selected(Selected::Chart)),
            None
        );
    }
}
