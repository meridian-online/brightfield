//! The overlay delegates: the domain halves of every "choose a thing" moment.
//!
//! `meridian-egui` owns the chrome — one [`Picker`](meridian_egui::Picker)
//! draws every delegate through the same query line, list rows, keystroke
//! chips and empty text, inside the same
//! [`ModalLayer`](meridian_egui::ModalLayer) card. What lives here is only
//! what is domain-shaped: which corpus, how a query filters and ranks it,
//! what a row says, and what confirming means. Every corpus is already
//! framework-free and unit-tested in `brightfield-keys` /
//! `brightfield-model`; these delegates adapt it to
//! [`PickerDelegate`] without re-deriving any of it.
//!
//! Five delegates, four picker shapes:
//!
//! - [`CommandPalette`] — query + ranked corpus:
//!   `brightfield_keys::palette_filter` over the registry at one altitude,
//!   read on an empty query under the ribbon's stage headings, in the
//!   ribbon's order, and by frequency then frecency within each; a typed
//!   query ranks by match with no headings. A reserved verb the palette
//!   lists is flagged and refuses to run, as the corpus says; the chart's
//!   palette is an allow-list of built verbs and lists none.
//! - [`ArgPrompt`] — the step-wise argument collection for the two
//!   argument-taking verbs, driven by `brightfield_model::arg_collector`.
//!   Enumerable steps read as a filtered pick list; a step with no
//!   enumerable options is the typed-value shape (the query *is* the value).
//! - [`JumpToNode`] / [`JumpToColumn`] — flat jump lists: fuzzy filter,
//!   rows, confirm.
//! - [`HelpSheet`] — the grouped read-only sheet over
//!   `brightfield_keys::help_sheet`; enter is another way out.
//!
//! A delegate never draws and never dispatches. It records what was chosen
//! — [`CommandPalette::take_picked`], [`ArgPrompt::take_ready`],
//! [`JumpToNode::take_picked`] — and the host acts on it after the picker
//! reports its event. That keeps every delegate assertable headlessly, with
//! no window and no modal open.
//!
//! Wiring status: the command palette, help sheet and node jump are live in
//! [`crate::window::MeridianApp`] on the protocol view's grammar. The
//! argument prompt and column jump are complete and tested here, and go live
//! with the chart view's editing bridge — the half that knows a focused plot
//! and applies a `ChartEdit`, which this shell does not carry yet.

use brightfield_keys::fuzzy::fuzzy_score;
use brightfield_keys::ribbon::Stage;
use brightfield_keys::{
    keys_label, palette_filter, registry, unix_now_secs, Altitude, Frecency, HelpRow, KeyPlatform,
    PaletteCandidate, Situation, VerbEntry,
};
use brightfield_model::arg_collector::{ArgCollector, ArgOutcome};
use brightfield_spec::edit::ChartEdit;
use meridian_egui::{PickerDelegate, PickerHint, PickerOutcome, PickerRow};

// ---------------------------------------------------------------------------
// CommandPalette
// ---------------------------------------------------------------------------

/// The verb longnames the chart view's palette lists — exactly what
/// `MeridianApp::apply`'s chart arm dispatches (`clear-selection`,
/// the navigation family), plus `open-home`, which that method handles before
/// the per-view match runs, so it takes effect no matter which view is
/// active. (`apply` is private, so this names it as plain code rather than
/// as a doc link — a link would resolve under `--document-private-items` but
/// not the public build, and doc links do not get to widen an API.)
///
/// `Altitude::View` in the registry is deliberately broader than this: it
/// also names reserved verbs no handler answers yet (`add-mark`,
/// `toggle-focus`, `reload-spec`, `toggle-presentation`,
/// `cycle-colour-scheme`, ...) and meta verbs this shell handles elsewhere
/// rather than through `MeridianApp::apply` (`open-help`, ...). Listing
/// the raw altitude scope on the chart view would put rows in the palette
/// that confirm and silently do nothing; this curated list is what keeps
/// that from happening. `overlay_wiring.rs`'s dispatchability test walks
/// [`chart_palette_candidates`] (the SAME list the palette actually shows,
/// not a hand-copied mirror of it) and proves each of these longnames
/// changes real state when confirmed.
///
/// These are the verbs that mean the same over whichever window. The two that
/// act on the Protocol behind the chart, Save and Run, are in the private
/// `PROTOCOL_CHART_VERBS`, and [`chart_palette_verbs`] is the two lists
/// with each built operation's verb after them. A row whose command is missing an input here is listed greyed
/// with the sentence it declares, from the situation the window gives the
/// palette ([`CommandPalette::with_situation`]).
pub const CHART_PALETTE_VERBS: &[&str] = &[
    "clear-selection",
    "open-home",
    crate::data_grid::MOVE_GRID,
    crate::navigation::verb::PAN_LEFT,
    crate::navigation::verb::PAN_RIGHT,
    crate::navigation::verb::PAN_UP,
    crate::navigation::verb::PAN_DOWN,
    crate::navigation::verb::ZOOM_IN,
    crate::navigation::verb::ZOOM_OUT,
    crate::navigation::verb::CYCLE_AXIS_LOCK,
    crate::navigation::verb::RESET_EXTENT,
    crate::window::COPY_CHART_PICTURE,
    crate::window::EXPORT_CHART_PICTURE,
];

/// The two chart palette verbs that act on the Protocol behind the chart:
/// `save-spec` writes it (`MeridianApp::save_protocol`) and `run-protocol` runs
/// what Save writes (`MeridianApp::run_protocol`). Each declares that it needs
/// a Protocol to act on, so over a chart spec or a shipped start its row is
/// greyed with that sentence
/// (`the_chart_palette_greys_save_where_there_is_nothing_to_save`).
///
/// They are apart from [`CHART_PALETTE_VERBS`] because the inspector rail reads
/// that list for the pane entries it may draw, and the entry a pane declares
/// for `save-spec` is the editor's own buffer save, a different write under the
/// same verb name (`inspector::dispatchable`).
const PROTOCOL_CHART_VERBS: &[&str] = &["save-spec", crate::window::RUN_PROTOCOL];

/// **Whether `text` on the chart palette's query line is a statement** rather
/// than a search: its first word, after leading whitespace, is `select`,
/// `with` or `from` in any case, and the text ends there or goes on with a
/// character that cannot continue an identifier. So `selection` is a search,
/// and `select*` and `from(` are statements.
#[must_use]
pub fn is_statement(text: &str) -> bool {
    let text = text.trim_start();
    ["select", "with", "from"].iter().any(|word| {
        text.get(..word.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(word))
            && text[word.len()..]
                .chars()
                .next()
                .is_none_or(|next| !(next.is_alphanumeric() || next == '_' || next == '$'))
    })
}

/// The verbs the chart palette lists, in declaration order — what the
/// restricted palette is built from. The ribbon's verbs come after the
/// operations', and are apart from [`CHART_PALETTE_VERBS`] for the reason
/// `PROTOCOL_CHART_VERBS` is: the inspector rail reads that list for pane
/// entries, and the ribbon is the window's, not a pane's.
#[must_use]
pub fn chart_palette_verbs() -> Vec<&'static str> {
    let operations: Vec<&'static str> = crate::operation_ui::built().map(|ui| ui.verb).collect();
    CHART_PALETTE_VERBS
        .iter()
        .chain(PROTOCOL_CHART_VERBS)
        .chain(&operations)
        .chain(&brightfield_keys::ribbon::RIBBON_VERBS)
        .copied()
        .collect()
}

/// **The verbs the Protocol palette lists**, in the registry's order: the
/// verbs the registry scopes to [`Altitude::Protocol`]. One whose input is
/// missing is listed greyed with its sentence: the three that put the
/// Outline's column on a channel while the graph is on the canvas, and
/// `remove-step` while no removable step is selected.
/// `every_row_the_protocol_palette_offers_as_enabled_does_something_when_chosen`
/// chooses each row left enabled.
#[must_use]
pub fn protocol_palette_verbs() -> Vec<&'static str> {
    registry()
        .iter()
        .filter(|v| v.applies_at(Altitude::Protocol))
        .map(|v| v.longname)
        .collect()
}

/// The stage of the work the palette lists `longname` under: a built
/// operation's own, else the stage whose ribbon tab lists it. A form joins its
/// stage when it is built, with no list naming it here.
#[must_use]
pub fn stage_of_verb(longname: &str) -> Option<Stage> {
    crate::operation_ui::by_verb(longname)
        .map(|ui| ui.stage)
        .or_else(|| brightfield_keys::ribbon::stage_of(longname))
}

/// The heading of the rows on an empty query that belong to no stage.
pub const OTHER_HEADING: &str = "Other";

/// Where `longname`'s heading reads on an empty query: the stages in their
/// tabs' order, then the rows of no stage.
fn stage_rank(longname: &str) -> usize {
    stage_of_verb(longname)
        .and_then(|stage| Stage::ALL.iter().position(|&s| s == stage))
        .unwrap_or(Stage::ALL.len())
}

/// The command palette: every verb applicable at one altitude, fuzzy-matched
/// over longname + help, ranked by score (non-empty query), or on an empty
/// query read under the stage headings in the ribbon's order, by frequency
/// then the palette's frecency within each. Reserved verbs are included, flagged,
/// and refuse to run — never hidden. So is a verb missing an input in the
/// situation the palette was given, with the sentence it declares.
pub struct CommandPalette {
    reg: Vec<VerbEntry>,
    altitude: Altitude,
    frecency: Frecency,
    /// The Unix time in seconds the palette ranks its rows at, read when it
    /// opens.
    now_secs: u64,
    situation: Option<Situation>,
    matches: Vec<PaletteCandidate>,
    /// Whether the rows read under stage headings: an empty query, with at
    /// least one row that belongs to a stage. A list with none would carry
    /// one "Other" heading that tells nothing apart.
    grouped: bool,
    hint: Option<PickerHint>,
    picked: Option<&'static str>,
}

impl CommandPalette {
    /// A palette over the keyboard registry at `altitude`. `frecency` is the
    /// host's history of picks, snapshotted at open — the host records the
    /// pick into its own copy so the *next* open ranks it.
    #[must_use]
    pub fn new(altitude: Altitude, frecency: Frecency) -> Self {
        Self {
            reg: registry(),
            altitude,
            frecency,
            now_secs: unix_now_secs(),
            situation: None,
            matches: Vec::new(),
            grouped: false,
            hint: None,
            picked: None,
        }
    }

    /// A palette over `altitude`, restricted to registry entries whose
    /// longname is also in `allow` — the shell's declaration of which
    /// registry entries it can actually dispatch there, layered over the
    /// registry's own (broader, forward-looking) altitude scope. See
    /// [`CHART_PALETTE_VERBS`] for why the chart view needs this and
    /// [`CommandPalette::new`] does not.
    #[must_use]
    pub fn new_restricted(altitude: Altitude, frecency: Frecency, allow: &[&str]) -> Self {
        Self {
            reg: registry()
                .into_iter()
                .filter(|v| allow.contains(&v.longname))
                .collect(),
            altitude,
            frecency,
            now_secs: unix_now_secs(),
            situation: None,
            matches: Vec::new(),
            grouped: false,
            hint: None,
            picked: None,
        }
    }

    /// The same palette reading each row's declared needs against what the
    /// window holds: a row with a need missing is greyed with its sentence and
    /// refuses to run. Without a situation no need is read.
    #[must_use]
    pub fn with_situation(mut self, situation: Situation) -> Self {
        self.situation = Some(situation);
        self
    }

    /// The verb the user confirmed, surrendered once. The host dispatches it
    /// and records it into its frecency.
    pub fn take_picked(&mut self) -> Option<&'static str> {
        self.picked.take()
    }

    /// The altitude the palette lists the verbs of.
    #[must_use]
    pub const fn altitude(&self) -> Altitude {
        self.altitude
    }

    /// The current candidate at `index`, for a test that wants the corpus
    /// row rather than the rendered [`PickerRow`].
    #[must_use]
    pub fn candidate(&self, index: usize) -> Option<&PaletteCandidate> {
        self.matches.get(index)
    }
}

/// The chart palette's candidate longnames under an empty query, in the
/// order the palette shows them — a test hook so a dispatchability sweep
/// walks the SAME list the palette actually lists, not a hand-copied mirror
/// of [`CHART_PALETTE_VERBS`]. A verb added to that list without a matching
/// arm in the sweep is a gap the sweep itself refuses to pass over in
/// silence — see `overlay_wiring.rs`.
#[must_use]
pub fn chart_palette_candidates() -> Vec<&'static str> {
    chart_palette_rows(None)
        .into_iter()
        .map(|c| c.longname)
        .collect()
}

/// The chart palette's rows under an empty query, read against `situation`
/// when one is given.
fn chart_palette_rows(situation: Option<Situation>) -> Vec<PaletteCandidate> {
    let allow = chart_palette_verbs();
    let mut p = CommandPalette::new_restricted(Altitude::View, Frecency::new(), &allow);
    if let Some(situation) = situation {
        p = p.with_situation(situation);
    }
    p.update_query("");
    (0..p.match_count())
        .filter_map(|i| p.candidate(i).cloned())
        .collect()
}

impl PickerDelegate for CommandPalette {
    fn update_query(&mut self, query: &str) {
        self.matches = palette_filter(
            &self.reg,
            self.altitude,
            query,
            &self.frecency,
            self.now_secs,
            self.situation.as_ref(),
        );
        // The filter has ordered an empty query by frequency then frecency, and
        // a stable sort keeps that order within each stage. A greyed row keeps
        // its place among its stage's rows.
        self.grouped = query.is_empty()
            && self
                .matches
                .iter()
                .any(|c| stage_of_verb(c.longname).is_some());
        if self.grouped {
            self.matches.sort_by_key(|c| stage_rank(c.longname));
        }
        self.hint = None;
    }

    fn match_count(&self) -> usize {
        self.matches.len()
    }

    fn row(&self, index: usize) -> PickerRow {
        let c = &self.matches[index];
        let mut row = PickerRow::new(c.longname);
        // A row missing an input is drawn greyed, with its sentence in place of
        // its help; a reserved row still reads by its detail.
        row = match (c.reserved_reason, &c.missing) {
            (Some(reason), _) => row.detail(format!("{} — {}", c.help, reason.reason())),
            (None, Some(sentence)) => row.detail(c.help).unavailable(sentence.as_str()),
            (None, None) => row.detail(c.help),
        };
        if let Some(key) = c.primary_key {
            row = row.keystroke(keys_label(key, KeyPlatform::host()));
        }
        row
    }

    fn header_before(&self, index: usize) -> Option<String> {
        if !self.grouped {
            return None;
        }
        let rank = stage_rank(self.matches[index].longname);
        let opens = index == 0 || stage_rank(self.matches[index - 1].longname) != rank;
        opens.then(|| {
            Stage::ALL
                .get(rank)
                .map_or(OTHER_HEADING, |stage| stage.label())
                .to_owned()
        })
    }

    fn confirm(&mut self, index: Option<usize>, _query: &str) -> PickerOutcome {
        let Some(c) = index.and_then(|i| self.matches.get(i)) else {
            return PickerOutcome::KeepOpen;
        };
        if let Some(sentence) = &c.missing {
            self.hint = Some(PickerHint::Error(sentence.clone()));
            return PickerOutcome::KeepOpen;
        }
        if !c.enabled {
            let reason = c
                .reserved_reason
                .map_or("reserved", brightfield_keys::ReservedReason::reason);
            self.hint = Some(PickerHint::Error(format!(
                "{} is not available yet — {reason}",
                c.longname
            )));
            return PickerOutcome::KeepOpen;
        }
        self.picked = Some(c.longname);
        PickerOutcome::Close
    }

    fn placeholder(&self) -> String {
        "Run a command…".to_owned()
    }

    fn hint(&self) -> Option<PickerHint> {
        self.hint.clone()
    }

    fn empty_text(&self) -> Option<String> {
        Some("No matching command".to_owned())
    }
}

// ---------------------------------------------------------------------------
// HelpSheet
// ---------------------------------------------------------------------------

/// One help-sheet line, pre-grouped: the row and the altitude group it reads
/// under.
struct HelpLine {
    group: String,
    row: HelpRow,
}

/// The keyboard help sheet: each built verb with its keys and help, grouped by
/// the altitudes it applies at, filterable, read-only. A reserved verb has no
/// key and is left out. Enter dismisses — there is nothing to run from a
/// reference sheet.
pub struct HelpSheet {
    lines: Vec<HelpLine>,
    matches: Vec<usize>,
}

impl HelpSheet {
    /// The sheet over the registry's built verbs, grouped by altitude set in a fixed
    /// reading order (dashboard-and-view rows first, protocol rows after),
    /// registry order within a group.
    #[must_use]
    pub fn new() -> Self {
        let mut lines: Vec<HelpLine> = brightfield_keys::help_sheet(&registry())
            .into_iter()
            .map(|row| HelpLine {
                group: row
                    .altitudes
                    .iter()
                    .map(|a| a.label())
                    .collect::<Vec<_>>()
                    .join(" · "),
                row,
            })
            .collect();
        // Group rows by their group's first appearance, keeping registry
        // order both across and within groups — a *stable* sort on the
        // group's first index, not on its label, so the reading order stays
        // the registry's rather than the alphabet's.
        let snapshot: Vec<String> = lines.iter().map(|l| l.group.clone()).collect();
        lines.sort_by_key(|l| {
            snapshot
                .iter()
                .position(|g| *g == l.group)
                .unwrap_or(usize::MAX)
        });
        Self {
            lines,
            matches: Vec::new(),
        }
    }
}

impl Default for HelpSheet {
    fn default() -> Self {
        Self::new()
    }
}

impl PickerDelegate for HelpSheet {
    fn update_query(&mut self, query: &str) {
        self.matches = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                fuzzy_score(query, l.row.longname).is_some()
                    || fuzzy_score(query, l.row.help).is_some()
            })
            .map(|(i, _)| i)
            .collect();
    }

    fn match_count(&self) -> usize {
        self.matches.len()
    }

    fn row(&self, index: usize) -> PickerRow {
        let line = &self.lines[self.matches[index]];
        let mut row = PickerRow::new(line.row.longname);
        row = match line.row.reserved_reason {
            Some(reason) => row.detail(format!("{} — {}", line.row.help, reason.reason())),
            None => row.detail(line.row.help),
        };
        if let Some(key) = line.row.keys.first() {
            row = row.keystroke(keys_label(key, KeyPlatform::host()));
        }
        row
    }

    fn confirm(&mut self, _index: Option<usize>, _query: &str) -> PickerOutcome {
        // Unreachable behind `confirmable() == false`; stated anyway.
        PickerOutcome::Close
    }

    fn placeholder(&self) -> String {
        "Filter the sheet…".to_owned()
    }

    fn header_before(&self, index: usize) -> Option<String> {
        let group = &self.lines[self.matches[index]].group;
        if index == 0 || self.lines[self.matches[index - 1]].group != *group {
            Some(group.clone())
        } else {
            None
        }
    }

    fn confirmable(&self) -> bool {
        false
    }

    fn empty_text(&self) -> Option<String> {
        Some("No matching command".to_owned())
    }
}

// ---------------------------------------------------------------------------
// Flat jump lists
// ---------------------------------------------------------------------------

/// One jumpable target: a stable id to hand back, and what to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JumpTarget {
    /// What confirming yields.
    pub id: String,
    /// The row's primary text.
    pub label: String,
    /// Muted annotation — the dotted address, a type, a kind.
    pub detail: Option<String>,
}

/// The shared engine of the two flat jump lists: fuzzy filter over label +
/// id, caller's order on an empty query, score order otherwise, confirm
/// yields the id.
struct JumpList {
    targets: Vec<JumpTarget>,
    /// `(target index, score)` for the current query.
    matches: Vec<(usize, i32)>,
    picked: Option<String>,
}

impl JumpList {
    fn new(targets: Vec<JumpTarget>) -> Self {
        Self {
            targets,
            matches: Vec::new(),
            picked: None,
        }
    }

    fn update_query(&mut self, query: &str) {
        self.matches = self
            .targets
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                let label = fuzzy_score(query, &t.label);
                let id = fuzzy_score(query, &t.id);
                match (label, id) {
                    (Some(a), Some(b)) => Some((i, a.max(b))),
                    (Some(a), None) | (None, Some(a)) => Some((i, a)),
                    (None, None) => None,
                }
            })
            .collect();
        if !query.is_empty() {
            // Stable, so equal scores keep the caller's order.
            self.matches
                .sort_by_key(|(_, score)| std::cmp::Reverse(*score));
        }
    }

    fn row(&self, index: usize) -> PickerRow {
        let t = &self.targets[self.matches[index].0];
        let mut row = PickerRow::new(&t.label);
        if let Some(detail) = &t.detail {
            row = row.detail(detail);
        }
        row
    }

    fn confirm(&mut self, index: Option<usize>) -> PickerOutcome {
        let Some(&(target, _)) = index.and_then(|i| self.matches.get(i)) else {
            return PickerOutcome::KeepOpen;
        };
        self.picked = Some(self.targets[target].id.clone());
        PickerOutcome::Close
    }
}

/// Fuzzy-jump to a node of the graph in view — the `/` finder. The host
/// supplies the targets (the outline, in its topological order) and moves the
/// selection to [`JumpToNode::take_picked`]'s answer.
pub struct JumpToNode(JumpList);

impl JumpToNode {
    /// A jump list over `targets`, kept in the given order under an empty
    /// query.
    #[must_use]
    pub fn new(targets: Vec<JumpTarget>) -> Self {
        Self(JumpList::new(targets))
    }

    /// The confirmed target's id, surrendered once.
    pub fn take_picked(&mut self) -> Option<String> {
        self.0.picked.take()
    }
}

impl PickerDelegate for JumpToNode {
    fn update_query(&mut self, query: &str) {
        self.0.update_query(query);
    }
    fn match_count(&self) -> usize {
        self.0.matches.len()
    }
    fn row(&self, index: usize) -> PickerRow {
        self.0.row(index)
    }
    fn confirm(&mut self, index: Option<usize>, _query: &str) -> PickerOutcome {
        self.0.confirm(index)
    }
    fn placeholder(&self) -> String {
        "Jump to an asset…".to_owned()
    }
    fn empty_text(&self) -> Option<String> {
        Some("No matching asset".to_owned())
    }
}

/// Fuzzy-jump to a column of the focused source — the pick list the editing
/// bridge opens where a verb needs a column and the profile can enumerate
/// them.
pub struct JumpToColumn(JumpList);

impl JumpToColumn {
    /// A jump list over `columns`, kept in profile order under an empty
    /// query.
    #[must_use]
    pub fn new(columns: Vec<String>) -> Self {
        Self(JumpList::new(
            columns
                .into_iter()
                .map(|c| JumpTarget {
                    id: c.clone(),
                    label: c,
                    detail: None,
                })
                .collect(),
        ))
    }

    /// The confirmed column, surrendered once.
    pub fn take_picked(&mut self) -> Option<String> {
        self.0.picked.take()
    }
}

impl PickerDelegate for JumpToColumn {
    fn update_query(&mut self, query: &str) {
        self.0.update_query(query);
    }
    fn match_count(&self) -> usize {
        self.0.matches.len()
    }
    fn row(&self, index: usize) -> PickerRow {
        self.0.row(index)
    }
    fn confirm(&mut self, index: Option<usize>, _query: &str) -> PickerOutcome {
        self.0.confirm(index)
    }
    fn placeholder(&self) -> String {
        "Jump to a column…".to_owned()
    }
    fn empty_text(&self) -> Option<String> {
        Some("No matching column".to_owned())
    }
}

// ---------------------------------------------------------------------------
// ArgPrompt
// ---------------------------------------------------------------------------

/// The argument prompt for the two argument-taking verbs: `add-mark` collects
/// a KIND, `set-channel` a CHANNEL then a COLUMN. A running
/// [`ArgCollector`] owns the steps and the validation; this delegate owns
/// only how the current step reads as a picker.
///
/// Two shapes in one, decided by the step's option list:
///
/// - options to enumerate → a filtered pick list, confirm picks the
///   selected option;
/// - nothing to enumerate (a column step with no profiled source) → the
///   typed-value shape: no rows, no empty-state narration, and the typed
///   query is the value, validated by the collector and narrated through the
///   hint.
pub struct ArgPrompt {
    collector: ArgCollector,
    columns: Vec<String>,
    options: Vec<String>,
    /// `(option index, score)` for the current query.
    matches: Vec<(usize, i32)>,
    hint: Option<PickerHint>,
    ready: Option<ChartEdit>,
    advanced: bool,
}

impl ArgPrompt {
    /// A prompt over a running collection. `columns` is the focused source's
    /// column list, consulted only when the collection reaches a column step
    /// — empty when no profile is at hand, which flips that step to the
    /// typed-value shape.
    #[must_use]
    pub fn new(collector: ArgCollector, columns: Vec<String>) -> Self {
        let mut prompt = Self {
            collector,
            columns,
            options: Vec::new(),
            matches: Vec::new(),
            hint: None,
            ready: None,
            advanced: false,
        };
        prompt.refresh_options();
        prompt
    }

    /// The completed edit, surrendered once. The host applies it and closes.
    pub fn take_ready(&mut self) -> Option<ChartEdit> {
        self.ready.take()
    }

    /// Whether the collection advanced a step since the last ask — the
    /// host's cue to clear the picker's query so the next step starts fresh.
    pub fn take_advanced(&mut self) -> bool {
        std::mem::take(&mut self.advanced)
    }

    fn refresh_options(&mut self) {
        self.options = self.collector.options(&self.columns);
    }

    fn pick(&mut self, choice: &str) -> PickerOutcome {
        match self.collector.pick(choice) {
            ArgOutcome::Ready(edit) => {
                self.ready = Some(edit);
                PickerOutcome::Close
            }
            ArgOutcome::Pending => {
                self.advanced = true;
                self.refresh_options();
                self.update_query("");
                self.hint = None;
                PickerOutcome::KeepOpen
            }
            ArgOutcome::Invalid => {
                self.hint = Some(PickerHint::Error(format!(
                    "{choice:?} is not a {}",
                    self.collector.step().prompt()
                )));
                PickerOutcome::KeepOpen
            }
        }
    }
}

impl PickerDelegate for ArgPrompt {
    fn update_query(&mut self, query: &str) {
        self.matches = self
            .options
            .iter()
            .enumerate()
            .filter_map(|(i, option)| fuzzy_score(query, option).map(|score| (i, score)))
            .collect();
        if !query.is_empty() {
            self.matches
                .sort_by_key(|(_, score)| std::cmp::Reverse(*score));
        }
    }

    fn match_count(&self) -> usize {
        self.matches.len()
    }

    fn row(&self, index: usize) -> PickerRow {
        PickerRow::new(&self.options[self.matches[index].0])
    }

    fn confirm(&mut self, index: Option<usize>, query: &str) -> PickerOutcome {
        match index.and_then(|i| self.matches.get(i)) {
            Some(&(option, _)) => {
                let choice = self.options[option].clone();
                self.pick(&choice)
            }
            // No matches: the typed query is the value (the typed-value
            // prompt shape). An empty query is nothing to validate.
            None if !query.is_empty() => {
                let choice = query.to_owned();
                self.pick(&choice)
            }
            None => PickerOutcome::KeepOpen,
        }
    }

    fn placeholder(&self) -> String {
        format!("Pick a {}…", self.collector.step().prompt())
    }

    fn hint(&self) -> Option<PickerHint> {
        self.hint.clone()
    }

    fn empty_text(&self) -> Option<String> {
        if self.options.is_empty() {
            // The typed-value shape: no matches is the normal condition, not
            // an outcome to narrate.
            None
        } else {
            Some("No matching option".to_owned())
        }
    }
}

// ---------------------------------------------------------------------------
// CloseQuestion
// ---------------------------------------------------------------------------

/// What a person answered when the window asked whether to close over a chart
/// edit that is not saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAnswer {
    /// Write the chart as the Save verb does, then close.
    SaveAndClose,
    /// Close without writing the chart file, keeping the chart as a Save would
    /// have written it as a version of the file, which the Versions panel
    /// lists as `closed unsaved`. When it cannot be kept the window stays and
    /// says why, and the same answer given again closes with nothing kept.
    CloseWithoutSaving,
    /// Stay: the window as it was, the edit still drawn and the mark still
    /// in the title.
    KeepEditing,
}

/// The headline of the question when only the chart holds unsaved edits, the title row of its
/// card.
pub const CLOSE_QUESTION_TITLE: &str = "Save the chart before closing?";

/// The headline of the question when only the Protocol holds unsaved edits: a step written,
/// corrected or removed, a grid view not yet recorded, or the model.
pub const CLOSE_PROTOCOL_QUESTION_TITLE: &str = "Save the Protocol before closing?";

/// The headline of the question when the Protocol and its chart both hold unsaved edits.
pub const CLOSE_BOTH_QUESTION_TITLE: &str = "Save the Protocol and its chart before closing?";

/// Each answer, its label and the key beside it, in the order they are drawn:
/// Enter, D and Esc. The keys are the accepted design's and provisional.
pub const CLOSE_ANSWERS: [(CloseAnswer, &str, &str); 3] = [
    (CloseAnswer::SaveAndClose, "Save and close", "Enter"),
    (CloseAnswer::CloseWithoutSaving, "Close without saving", "D"),
    (CloseAnswer::KeepEditing, "Keep editing", "Esc"),
];

/// The headline of the question asked before a rename of the Protocol.
pub const RENAME_QUESTION_TITLE: &str = "Save before renaming?";

/// The headline of the question asked before the Protocol's folder is moved.
pub const MOVE_QUESTION_TITLE: &str = "Save before moving?";

/// The headline of the question asked before a source is added to the Protocol, which replaces
/// what its canvas shows.
pub const ADD_SOURCE_QUESTION_TITLE: &str = "Save before adding a source?";

/// The headline of the question asked before New Protocol makes a Protocol in the window's place.
pub const MAKE_QUESTION_TITLE: &str = "Save before making a new Protocol?";

/// The headline of the question asked before the open Protocol is moved into a new Protocols
/// folder chosen in Preferences, with the others in the old one.
pub const MOVE_TO_FOLDER_QUESTION_TITLE: &str =
    "Save before moving it to the new Protocols folder?";

/// The headline of the question asked before the Protocol's data is read again from its file,
/// which opens the Protocol afresh and replaces what its canvas shows.
pub const RELOAD_QUESTION_TITLE: &str = "Save before reloading?";

/// What the close question is asked before: the window closing or the Protocol left, or its
/// folder renamed or moved, after which the window opens it again where it now is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AskedBefore {
    /// Closing the window, going Home, opening another Protocol or quitting.
    #[default]
    Leaving,
    /// Renaming the Protocol.
    Renaming,
    /// Moving the Protocol's folder.
    Moving,
    /// Adding a source to the Protocol.
    AddingSource,
    /// Making a new Protocol, which takes the window's place.
    Making,
    /// Moving the open Protocol into the new Protocols folder with the others. Keeping editing
    /// leaves it where it is, and the others still move.
    MovingToNewFolder,
    /// Reading the Protocol's data again from its file, which opens it afresh.
    Reloading,
}

/// How many rows the list of unsaved edits takes at most: the two the accepted
/// frame draws, so the card is no taller with more edits than it is there.
/// Past them, the last row counts the edits not listed.
pub const CLOSE_LIST_ROWS: usize = 2;

/// **What the close question says**: the chart file it is about, the unsaved
/// edits in the shelf's words, and, once a close without saving could not keep
/// the chart, why.
///
/// Built once when the question opens, because reading the edits reads the
/// chart file; the question is modal, so they cannot change while it is up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseQuestion {
    /// The chart file, as the window names it: relative to the Protocol's
    /// folder, `panels/california_housing.yaml`.
    pub file: String,
    /// Each unsaved edit in the shelf's words, led by its tile, as the
    /// Versions panel words a change: `Map · x axis: longitude →
    /// median_income`. The chart's edits come first, then the Protocol's.
    pub edits: Vec<String>,
    /// How many of [`Self::edits`], from the first, are edits to the chart; the rest change the
    /// Protocol: a SQL step, a grid view, a removed step or the model.
    pub chart_edits: usize,
    /// The Protocol's name, as the title band names it, which the headline names when the
    /// Protocol holds an edit.
    pub protocol: String,
    /// Why a close without saving could not keep the chart, once one has been
    /// tried: the next one closes with nothing kept.
    pub not_kept: Option<String>,
    /// The answer that held the keyboard focus when the card last drew, which
    /// Enter gives: `None` before the card has drawn.
    pub focused: Option<CloseAnswer>,
    /// What the question is asked before, which words its title and answers.
    pub before: AskedBefore,
}

impl CloseQuestion {
    /// The title row of the card: [`CLOSE_QUESTION_TITLE`] before leaving, and the rename's or
    /// the move's own before those.
    #[must_use]
    pub fn title(&self) -> &'static str {
        match self.before {
            AskedBefore::Leaving => match (self.chart_edits > 0, self.protocol_edits() > 0) {
                (false, true) => CLOSE_PROTOCOL_QUESTION_TITLE,
                (true, true) => CLOSE_BOTH_QUESTION_TITLE,
                _ => CLOSE_QUESTION_TITLE,
            },
            AskedBefore::Renaming => RENAME_QUESTION_TITLE,
            AskedBefore::Moving => MOVE_QUESTION_TITLE,
            AskedBefore::AddingSource => ADD_SOURCE_QUESTION_TITLE,
            AskedBefore::Making => MAKE_QUESTION_TITLE,
            AskedBefore::MovingToNewFolder => MOVE_TO_FOLDER_QUESTION_TITLE,
            AskedBefore::Reloading => RELOAD_QUESTION_TITLE,
        }
    }

    /// The key chip beside the answer `which`, whose key is `key` in [`CLOSE_ANSWERS`]. Keep editing
    /// has no chip before the move into a new Protocols folder, where Esc cancels the whole move
    /// rather than keeping editing.
    #[must_use]
    pub fn answer_key(&self, which: CloseAnswer, key: &'static str) -> Option<&'static str> {
        (!(self.before == AskedBefore::MovingToNewFolder && which == CloseAnswer::KeepEditing))
            .then_some(key)
    }

    /// The label of the answer `which`: [`CLOSE_ANSWERS`]' before leaving, and before a rename or
    /// a move the words of what it does there. Not saving still keeps the edits as a version, and
    /// then renames or moves.
    #[must_use]
    pub fn answer_label(&self, which: CloseAnswer) -> &'static str {
        match (self.before, which) {
            (AskedBefore::Renaming, CloseAnswer::SaveAndClose) => "Save and rename",
            (AskedBefore::Renaming, CloseAnswer::CloseWithoutSaving) => {
                "Keep as a version and rename"
            }
            (AskedBefore::Moving, CloseAnswer::SaveAndClose) => "Save and move",
            (AskedBefore::Moving, CloseAnswer::CloseWithoutSaving) => "Keep as a version and move",
            (AskedBefore::AddingSource, CloseAnswer::SaveAndClose) => "Save and add",
            (AskedBefore::AddingSource, CloseAnswer::CloseWithoutSaving) => {
                "Keep as a version and add"
            }
            (AskedBefore::Making, CloseAnswer::SaveAndClose) => "Save and make",
            (AskedBefore::Making, CloseAnswer::CloseWithoutSaving) => "Keep as a version and make",
            (AskedBefore::MovingToNewFolder, CloseAnswer::SaveAndClose) => "Save and move",
            (AskedBefore::MovingToNewFolder, CloseAnswer::CloseWithoutSaving) => {
                "Keep as a version and move"
            }
            (AskedBefore::MovingToNewFolder, CloseAnswer::KeepEditing) => {
                "Keep editing, and leave it"
            }
            (AskedBefore::Reloading, CloseAnswer::SaveAndClose) => "Save and reload",
            (AskedBefore::Reloading, CloseAnswer::CloseWithoutSaving) => {
                "Keep as a version and reload"
            }
            _ => CLOSE_ANSWERS
                .iter()
                .find(|(answer, _, _)| *answer == which)
                .map_or("", |(_, label, _)| label),
        }
    }

    /// How many of [`Self::edits`] change the Protocol rather than its chart.
    #[must_use]
    pub fn protocol_edits(&self) -> usize {
        self.edits.len().saturating_sub(self.chart_edits)
    }

    /// The line under the title: what holds the edits that are not saved, and how many of each
    /// kind. A chart's edits are counted against its file and a Protocol's against the Protocol,
    /// so a SQL step edited is never called a chart edit.
    #[must_use]
    pub fn headline(&self) -> String {
        let count = |n: usize, kind: &str| {
            if n == 1 {
                format!("1 {kind} edit")
            } else {
                format!("{n} {kind} edits")
            }
        };
        match (self.chart_edits, self.protocol_edits()) {
            (0, 0) => format!("{} holds a chart edit that is not saved.", self.file),
            (charts, 0) => format!(
                "{} has {} that {} not saved.",
                self.file,
                count(charts, "chart"),
                if charts == 1 { "is" } else { "are" }
            ),
            (0, steps) => format!(
                "{} has {} that {} not saved.",
                self.protocol,
                count(steps, "Protocol"),
                if steps == 1 { "is" } else { "are" }
            ),
            (charts, steps) => format!(
                "{} has {}, and {} in {}, that are not saved.",
                self.protocol,
                count(steps, "Protocol"),
                count(charts, "chart"),
                self.file
            ),
        }
    }

    /// The rows of the list: the edits themselves when they fit in
    /// [`CLOSE_LIST_ROWS`], and otherwise the first edits and a last row that
    /// counts the rest, so the edits listed and the count add up to
    /// [`Self::edits`].
    #[must_use]
    pub fn listed(&self) -> Vec<String> {
        let n = self.edits.len();
        if n <= CLOSE_LIST_ROWS {
            return self.edits.clone();
        }
        let shown = CLOSE_LIST_ROWS - 1;
        let rest = n - shown;
        let mut rows = self.edits[..shown].to_vec();
        rows.push(format!(
            "and {rest} more {}",
            if rest == 1 { "edit" } else { "edits" }
        ));
        rows
    }

    /// What closing without saving keeps, or, once it could not keep the
    /// chart, that it could not and why.
    #[must_use]
    pub fn keeps(&self) -> String {
        match &self.not_kept {
            Some(why) => format!(
                "The chart could not be kept as a version: {why}. \
                 Close without saving again to close with nothing kept."
            ),
            None if self.edits.len() > 1 => {
                "Closing without saving keeps these as a version you can step back to.".to_string()
            }
            None => {
                "Closing without saving keeps it as a version you can step back to.".to_string()
            }
        }
    }
}

/// **The answer typed this frame**, read before the card draws: D, with or
/// without shift, is *Close without saving*, and Enter is *Save and close*
/// unless another answer holds the focus, whose button then takes the Enter
/// itself. Esc is the card's own dismissal, which the host reads as *Keep
/// editing*.
///
/// Enter is read here and not left to the focused button, because on the frame
/// the question opens no answer holds the focus yet, and a widget under the
/// card may.
pub fn close_question_keys(ctx: &egui::Context, question: &CloseQuestion) -> Option<CloseAnswer> {
    let d = ctx.input_mut(|i| {
        i.consume_key(egui::Modifiers::NONE, egui::Key::D)
            || i.consume_key(egui::Modifiers::SHIFT, egui::Key::D)
    });
    if d {
        return Some(CloseAnswer::CloseWithoutSaving);
    }
    let enter_saves = matches!(question.focused, None | Some(CloseAnswer::SaveAndClose));
    (enter_saves && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)))
        .then_some(CloseAnswer::SaveAndClose)
}

/// Draw the body of the close question and report the answer clicked, if any.
///
/// The body of a [`ModalLayer`](meridian_egui::ModalLayer) card the host
/// draws: this owns the words and the three answers, each with its key in a
/// chip beside it, and the host owns what an answer does. The card has no
/// footer, because each answer carries its key.
///
/// *Save and close* takes the keyboard focus while no answer holds it, so
/// Enter saves; a tab to another answer moves it and Enter then gives that one.
/// Which answer holds it is written to [`CloseQuestion::focused`].
pub fn close_question_body(ui: &mut egui::Ui, question: &mut CloseQuestion) -> Option<CloseAnswer> {
    use meridian_egui::MeridianUi as _;
    let gap = ui.tokens().section_gap;
    let control_gap = ui.tokens().control_gap;
    let chip_gap = ui.tokens().space[1];
    let sem = meridian_design::semantic(ui.visuals().dark_mode);
    let ink = |rgba| brightfield_workbench::chrome::colour(rgba);
    ui.label(question.headline());
    let listed = question.listed();
    if !listed.is_empty() {
        ui.add_space(gap);
        for row in listed {
            ui.label(
                egui::RichText::new(format!("\u{2022}  {row}"))
                    .monospace()
                    .small()
                    .color(ink(sem.text.secondary)),
            );
        }
    }
    ui.add_space(gap);
    let keeps = egui::RichText::new(question.keeps()).small();
    ui.label(match question.not_kept {
        // The danger role's solid, as ink on the card.
        Some(_) => keeps.color(ink(sem.role(meridian_design::Role::Danger).background.base)),
        None => keeps.color(ink(sem.text.muted)),
    });
    ui.add_space(gap);
    let mut answer = None;
    let mut focused = None;
    let mut save = None;
    ui.horizontal(|ui| {
        for (i, (which, _, key)) in CLOSE_ANSWERS.into_iter().enumerate() {
            let label = question.answer_label(which);
            if i > 0 {
                ui.add_space(control_gap);
            }
            let button = if which == CloseAnswer::SaveAndClose {
                let accent = sem.role(meridian_design::Role::Accent);
                ui.add(
                    egui::Button::new(
                        egui::RichText::new(label).color(ink(accent.foreground.base)),
                    )
                    .fill(ink(accent.background.base)),
                )
            } else {
                ui.button(label)
            };
            if button.has_focus() {
                focused = Some(which);
            }
            if which == CloseAnswer::SaveAndClose {
                save = Some(button.clone());
            }
            if let Some(key) = question.answer_key(which, key) {
                ui.add_space(chip_gap);
                meridian_egui::key_chip(ui, key);
            }
            if button.clicked() {
                answer = Some(which);
            }
        }
    });
    if focused.is_none() {
        if let Some(save) = save {
            save.request_focus();
            focused = Some(CloseAnswer::SaveAndClose);
        }
    }
    question.focused = focused;
    answer
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::analysis::ComponentPath;

    fn question(edits: &[&str], chart_edits: usize) -> CloseQuestion {
        CloseQuestion {
            file: "panels/sales.yaml".to_string(),
            edits: edits.iter().map(|e| (*e).to_string()).collect(),
            chart_edits,
            protocol: "sales".to_string(),
            not_kept: None,
            focused: None,
            before: super::AskedBefore::Leaving,
        }
    }

    /// Chart edits are counted against the chart's file, and a draft that holds both kinds counts
    /// each against what it changes.
    #[test]
    fn the_close_question_counts_each_kind_of_edit_against_what_it_changes() {
        let charts = question(&["Map \u{b7} x axis: a \u{2192} b"], 1);
        assert_eq!(charts.title(), CLOSE_QUESTION_TITLE);
        assert_eq!(
            charts.headline(),
            "panels/sales.yaml has 1 chart edit that is not saved."
        );
        let both = question(
            &[
                "Map \u{b7} x axis: a \u{2192} b",
                "Map \u{b7} y axis: c \u{2192} d",
                "SQL step \u{b7} sales_sql",
            ],
            2,
        );
        assert_eq!(both.title(), CLOSE_BOTH_QUESTION_TITLE);
        assert_eq!(
            both.headline(),
            "sales has 1 Protocol edit, and 2 chart edits in panels/sales.yaml, that are not saved."
        );
    }

    // -- is_statement -------------------------------------------------------

    #[test]
    fn a_statement_starts_with_select_with_or_from_as_a_whole_word() {
        for text in [
            "select",
            "  SELECT",
            "select *",
            "select*",
            "With t as (select 1) select * from t",
            "\nfrom(",
            "FROM california_housing",
        ] {
            assert!(is_statement(text), "{text:?} is a statement");
        }
        for text in [
            "",
            "   ",
            "selection",
            "select_rows",
            "fromage",
            "within",
            "clear-selection",
            "sel",
        ] {
            assert!(!is_statement(text), "{text:?} is a search");
        }
    }

    // -- CommandPalette -----------------------------------------------------

    fn palette_at(altitude: Altitude) -> CommandPalette {
        let mut p = CommandPalette::new(altitude, Frecency::new());
        p.update_query("");
        p
    }

    fn chart_palette() -> CommandPalette {
        let allow = chart_palette_verbs();
        let mut p = CommandPalette::new_restricted(Altitude::View, Frecency::new(), &allow);
        p.update_query("");
        p
    }

    /// The heading a row reads under on an empty query.
    fn stage_heading(longname: &str) -> &'static str {
        stage_of_verb(longname).map_or(OTHER_HEADING, Stage::label)
    }

    /// The headings `p` draws, in order, after checking that the first row
    /// opens one and that every row reads under the heading of its own stage.
    fn headings(p: &CommandPalette) -> Vec<String> {
        assert!(p.match_count() > 0 && p.header_before(0).is_some());
        let mut drawn = Vec::new();
        let mut under = String::new();
        for i in 0..p.match_count() {
            if let Some(h) = p.header_before(i) {
                under.clone_from(&h);
                drawn.push(h);
            }
            let longname = p.candidate(i).unwrap().longname;
            assert_eq!(stage_heading(longname), under, "{longname}");
        }
        drawn
    }

    /// A palette lists the stages its rows have: each draws its own, in the
    /// ribbon's order, once each, and the rows of no stage last. The window's
    /// draws no Shape or Publish; the chart's draws all four, with Connect for
    /// Add sources, which is how a file is read in.
    #[test]
    fn the_empty_palette_reads_in_stage_order_under_each_stages_name() {
        let order: Vec<&str> = Stage::ALL
            .iter()
            .map(|s| s.label())
            .chain([OTHER_HEADING])
            .collect();
        for (p, expected) in [
            (
                palette_at(Altitude::Protocol),
                vec!["Connect", "Model", "Other"],
            ),
            // Add sources reads a file in, under Connect; Append rows, a Model
            // operation, starts from the grid's cursor.
            (
                chart_palette(),
                vec!["Connect", "Shape", "Model", "Publish", "Other"],
            ),
        ] {
            let drawn = headings(&p);
            let ranks: Vec<usize> = drawn
                .iter()
                .map(|h| order.iter().position(|o| o == h).unwrap())
                .collect();
            assert!(ranks.windows(2).all(|w| w[0] < w[1]), "{drawn:?}");
            assert_eq!(drawn, expected);
        }
    }

    #[test]
    fn a_built_form_reads_under_its_own_stage() {
        for ui in crate::operation_ui::built() {
            assert_eq!(stage_of_verb(ui.verb), Some(ui.stage), "{}", ui.verb);
        }
        assert_eq!(
            stage_of_verb(brightfield_keys::ribbon::SHOW_SHAPE_TAB),
            brightfield_keys::ribbon::stage_of(brightfield_keys::ribbon::SHOW_SHAPE_TAB)
        );
    }

    #[test]
    fn a_typed_query_ranks_by_match_and_draws_no_headers() {
        let mut p = chart_palette();
        p.update_query("zoom");
        assert!(p.match_count() > 0);
        assert!((0..p.match_count()).all(|i| p.header_before(i).is_none()));
        let scores: Vec<i32> = (0..p.match_count())
            .map(|i| p.candidate(i).unwrap().score)
            .collect();
        assert!(
            scores.windows(2).all(|w| w[0] >= w[1]),
            "ranked by match: {scores:?}"
        );
    }

    #[test]
    fn the_palette_shows_keys_as_keycaps() {
        let p = chart_palette();
        let mut seen = 0;
        for i in 0..p.match_count() {
            let c = p.candidate(i).unwrap();
            assert_eq!(
                p.row(i).keystroke,
                c.primary_key.map(|k| keys_label(k, KeyPlatform::host())),
                "{}",
                c.longname
            );
            seen += usize::from(c.primary_key.is_some());
        }
        assert!(seen > 0, "some row has a key");
    }

    #[test]
    fn the_palette_scopes_to_its_altitude() {
        let mut protocol = palette_at(Altitude::Protocol);
        protocol.update_query("");
        let names: Vec<&str> = (0..protocol.match_count())
            .map(|i| protocol.candidate(i).unwrap().longname)
            .collect();
        assert!(names.contains(&"open-steps-sheet"), "{names:?}");
        assert!(
            !names.contains(&"cycle-colour-scheme"),
            "a view-altitude verb leaked into the protocol palette: {names:?}"
        );
    }

    #[test]
    fn a_row_carries_the_help_and_the_registry_keystroke() {
        let mut p = palette_at(Altitude::Protocol);
        p.update_query("steps");
        let top = p.row(0);
        assert_eq!(top.label, "open-steps-sheet");
        assert_eq!(
            top.keystroke,
            Some(keys_label("shift-s", KeyPlatform::host()))
        );
        assert!(top.detail.is_some());
    }

    #[test]
    fn confirming_an_enabled_verb_closes_with_the_pick() {
        let mut p = palette_at(Altitude::Protocol);
        p.update_query("yank");
        let i = (0..p.match_count())
            .find(|&i| p.candidate(i).unwrap().longname == "yank-address")
            .expect("yank-address is in the protocol palette");
        assert_eq!(p.confirm(Some(i), "yank"), PickerOutcome::Close);
        assert_eq!(p.take_picked(), Some("yank-address"));
        assert_eq!(p.take_picked(), None, "a pick is surrendered once");
    }

    #[test]
    fn a_reserved_verb_is_flagged_and_refuses_to_run() {
        // The inspector rail's toggle rather than the outline rail's: the
        // outline rail's toggle is bound and performed now that the navigator
        // rail is a region of the window, so it is no longer an example of
        // this. The claim is unchanged and the bucket is the same one.
        let mut p = palette_at(Altitude::Protocol);
        p.update_query("inspector");
        let i = (0..p.match_count())
            .find(|&i| p.candidate(i).unwrap().longname == "toggle-inspector-rail")
            .expect("the reserved rail toggle is shown, not hidden");
        let row = p.row(i);
        assert!(
            row.detail
                .as_deref()
                .unwrap_or("")
                .contains("workspace shell"),
            "the reserved flag names its bucket: {:?}",
            row.detail
        );
        assert!(row.keystroke.is_none(), "reserved verbs show no key");
        assert_eq!(p.confirm(Some(i), "inspector"), PickerOutcome::KeepOpen);
        assert!(p.take_picked().is_none());
        assert!(
            matches!(p.hint(), Some(PickerHint::Error(_))),
            "refusal is narrated, not silent"
        );
    }

    #[test]
    fn the_chart_palette_lists_only_the_allowed_verbs() {
        let names = chart_palette_candidates();
        assert!(!names.is_empty(), "the chart palette lists nothing");
        let allowed = chart_palette_verbs();
        for name in &names {
            assert!(
                allowed.contains(name),
                "{name} is listed on the chart palette and is not one of its verbs"
            );
        }
        // A verb genuinely applicable at `Altitude::View` but NOT wired to
        // `MeridianApp::apply`'s Charts arm — exactly the silent-no-op risk
        // `CHART_PALETTE_VERBS` exists to keep off this list.
        assert!(
            !names.contains(&"open-help"),
            "open-help is View-scoped but not restricted-allowed — it must not \
             leak onto the chart palette: {names:?}"
        );
        assert!(
            !names.contains(&"change-mark-type"),
            "change-mark-type is reserved until the chart can be edited from \
             the keys — it must not leak onto the chart palette: {names:?}"
        );
    }

    /// **What a window offers is a property of that window.** Over a chart
    /// with no Protocol to act on, Save and Run are listed greyed, each with
    /// the sentence it declares; over one with a Protocol they are live. No
    /// other row moves with that.
    ///
    /// The rows used to be live over each window, so confirming Save over a
    /// shipped start closed the palette and did nothing.
    #[test]
    fn the_chart_palette_greys_save_where_there_is_nothing_to_save() {
        use brightfield_keys::{Condition, Selected};
        let chart_start = Situation {
            selected: vec![Selected::Protocol, Selected::Chart],
            ..Situation::default()
        };
        let data_file = Situation {
            conditions: vec![
                Condition::ProtocolToRun,
                Condition::ProtocolToSave,
                Condition::Trusted,
            ],
            ..chart_start.clone()
        };
        let without = chart_palette_rows(Some(chart_start));
        let with = chart_palette_rows(Some(data_file));
        let row = |rows: &[PaletteCandidate], verb: &str| {
            rows.iter()
                .find(|c| c.longname == verb)
                .unwrap_or_else(|| panic!("{verb} is not listed on the chart palette"))
                .clone()
        };
        let on_the_protocol = ["save-spec", crate::window::RUN_PROTOCOL];
        for verb in on_the_protocol {
            let greyed = row(&without, verb);
            assert!(!greyed.enabled, "{verb} is live with no Protocol to act on");
            assert!(
                greyed
                    .missing
                    .as_deref()
                    .is_some_and(|sentence| sentence.contains("needs a Protocol to ")),
                "{verb} says {:?}",
                greyed.missing
            );
            let live = row(&with, verb);
            assert!(
                live.enabled && live.missing.is_none(),
                "{verb} is greyed over a Protocol: {:?}",
                live.missing
            );
        }
        // Nothing else moves with the Protocol.
        for candidate in &without {
            if on_the_protocol.contains(&candidate.longname) {
                continue;
            }
            assert_eq!(
                row(&with, candidate.longname).enabled,
                candidate.enabled,
                "{} moved with the Protocol",
                candidate.longname
            );
        }
        assert_eq!(without.len(), with.len());
    }

    #[test]
    fn a_missing_candidate_is_a_disabled_row_with_its_sentence() {
        let allow = chart_palette_verbs();
        let mut p = CommandPalette::new_restricted(Altitude::View, Frecency::new(), &allow)
            .with_situation(Situation::default());
        p.update_query("zoom-in");
        let i = (0..p.match_count())
            .find(|&i| p.candidate(i).unwrap().longname == "zoom-in")
            .expect("zoom-in is listed with its input missing");
        let sentence = "Zoom in needs a chart on the canvas.";
        assert_eq!(p.row(i).unavailable.as_deref(), Some(sentence));
        assert!(
            p.row(i)
                .detail
                .as_deref()
                .is_some_and(|d| !d.contains(sentence)),
            "the sentence rides on the row, not in its detail too: {:?}",
            p.row(i).detail
        );
        assert_eq!(p.confirm(Some(i), "zoom-in"), PickerOutcome::KeepOpen);
        assert!(p.take_picked().is_none());
        assert_eq!(p.hint(), Some(PickerHint::Error(sentence.to_owned())));
    }

    #[test]
    fn new_restricted_excludes_a_view_scoped_verb_outside_the_allow_list() {
        let mut p =
            CommandPalette::new_restricted(Altitude::View, Frecency::new(), &["clear-selection"]);
        p.update_query("");
        let names: Vec<&str> = (0..p.match_count())
            .map(|i| p.candidate(i).unwrap().longname)
            .collect();
        assert_eq!(
            names,
            vec!["clear-selection"],
            "new_restricted let through a verb outside its allow list: {names:?}"
        );
    }

    #[test]
    fn frecency_lifts_a_recorded_verb_under_an_empty_query() {
        let mut frecency = Frecency::new();
        frecency.record("protocol-sibling-prev", unix_now_secs());
        let mut p = CommandPalette::new(Altitude::Protocol, frecency);
        p.update_query("");
        let pos = |p: &CommandPalette, name: &str| {
            (0..p.match_count())
                .find(|&i| p.candidate(i).unwrap().longname == name)
                .unwrap()
        };
        assert!(
            pos(&p, "protocol-sibling-prev") < pos(&p, "protocol-sibling-next"),
            "the recorded verb ranks above its equal-frequency peer"
        );
    }

    // -- HelpSheet ----------------------------------------------------------

    #[test]
    fn the_sheet_covers_the_built_verbs_and_reads_in_groups() {
        let mut sheet = HelpSheet::new();
        sheet.update_query("");
        let built = registry().iter().filter(|v| !v.is_reserved()).count();
        assert_eq!(sheet.match_count(), built);
        // A row with no key would list a command no key reaches.
        for i in 0..sheet.match_count() {
            let row = sheet.row(i);
            assert!(row.keystroke.is_some(), "{} shows no key", row.label);
        }
        // Headers appear exactly at group changes, and every group appears
        // exactly once — grouping is contiguous, not interleaved.
        let mut seen = Vec::new();
        for i in 0..sheet.match_count() {
            if let Some(header) = sheet.header_before(i) {
                assert!(
                    !seen.contains(&header),
                    "group {header:?} appears twice — rows are interleaved"
                );
                seen.push(header);
            }
        }
        assert!(seen.len() > 1, "one group means grouping proved nothing");
        assert!(!sheet.confirmable(), "a reference sheet runs nothing");
    }

    /// The sheet and the palette show a binding the same way, as a keycap.
    #[test]
    fn the_sheet_shows_keys_as_the_palette_does() {
        let mut sheet = HelpSheet::new();
        sheet.update_query("open-steps-sheet");
        let mut palette = palette_at(Altitude::Protocol);
        palette.update_query("open-steps-sheet");
        let keycap = Some(keys_label("shift-s", KeyPlatform::host()));
        assert_eq!(sheet.row(0).label, "open-steps-sheet");
        assert_eq!(sheet.row(0).keystroke, keycap);
        assert_eq!(palette.row(0).keystroke, keycap);
    }

    #[test]
    fn the_sheet_filters_without_losing_its_grouping() {
        let mut sheet = HelpSheet::new();
        sheet.update_query("drill");
        let full = registry().iter().filter(|v| !v.is_reserved()).count();
        let hits = sheet.match_count();
        assert!(hits > 0, "nothing matched a real verb fragment");
        assert!(
            hits < full,
            "the filter kept everything, so it filtered nothing"
        );
        for i in 0..hits {
            let row = sheet.row(i);
            // The filter is the palette's fuzzy subsequence matcher over
            // longname + help, so hold rows to that predicate, not to a
            // substring the matcher never promised.
            assert!(
                fuzzy_score("drill", &row.label).is_some()
                    || fuzzy_score("drill", row.detail.as_deref().unwrap_or("")).is_some(),
                "{:?} does not match the query",
                row.label
            );
        }
        assert!(sheet.header_before(0).is_some(), "the first row is titled");
    }

    // -- Jump lists ---------------------------------------------------------

    fn nodes() -> Vec<JumpTarget> {
        ["raw.filings", "clean.filings", "crosswalk.edgar_gleif"]
            .iter()
            .map(|id| JumpTarget {
                id: (*id).to_owned(),
                label: id.rsplit('.').next().unwrap().to_owned(),
                detail: Some((*id).to_owned()),
            })
            .collect()
    }

    #[test]
    fn an_empty_query_keeps_the_callers_order() {
        let mut jump = JumpToNode::new(nodes());
        jump.update_query("");
        assert_eq!(jump.match_count(), 3);
        assert_eq!(jump.row(0).label, "filings");
        assert_eq!(jump.row(2).label, "edgar_gleif");
    }

    #[test]
    fn a_query_ranks_and_confirm_yields_the_id() {
        let mut jump = JumpToNode::new(nodes());
        jump.update_query("edgar");
        assert!(jump.match_count() >= 1);
        assert_eq!(jump.row(0).label, "edgar_gleif");
        assert_eq!(jump.confirm(Some(0), "edgar"), PickerOutcome::Close);
        assert_eq!(jump.take_picked().as_deref(), Some("crosswalk.edgar_gleif"));
    }

    #[test]
    fn confirming_into_no_matches_keeps_the_jump_open() {
        let mut jump = JumpToNode::new(nodes());
        jump.update_query("zzz");
        assert_eq!(jump.match_count(), 0);
        assert_eq!(jump.confirm(None, "zzz"), PickerOutcome::KeepOpen);
        assert!(jump.take_picked().is_none());
    }

    #[test]
    fn the_column_jump_is_the_same_shape_over_a_profile() {
        let mut jump = JumpToColumn::new(vec!["temp".into(), "depth".into()]);
        jump.update_query("dep");
        assert_eq!(jump.match_count(), 1);
        assert_eq!(jump.confirm(Some(0), "dep"), PickerOutcome::Close);
        assert_eq!(jump.take_picked().as_deref(), Some("depth"));
    }

    // -- ArgPrompt ----------------------------------------------------------

    fn plot() -> ComponentPath {
        ComponentPath("root".to_owned())
    }

    #[test]
    fn add_mark_offers_kinds_and_completes_on_one_pick() {
        let mut prompt = ArgPrompt::new(ArgCollector::add_mark(plot()), Vec::new());
        prompt.update_query("");
        assert!(prompt.match_count() > 0, "kinds are enumerable");
        assert!(prompt.placeholder().contains("mark kind"));
        let i = (0..prompt.match_count())
            .find(|&i| prompt.row(i).label == "barY")
            .expect("barY renders and is offered");
        assert_eq!(prompt.confirm(Some(i), ""), PickerOutcome::Close);
        match prompt.take_ready() {
            Some(ChartEdit::AddMark { plot: p, .. }) => assert_eq!(p, plot()),
            other => panic!("expected a completed AddMark, got {other:?}"),
        }
    }

    #[test]
    fn set_channel_advances_channel_then_column() {
        let columns = vec!["temp".to_owned(), "depth".to_owned()];
        let mut prompt = ArgPrompt::new(ArgCollector::set_channel(plot(), 0), columns);
        prompt.update_query("");
        assert!(prompt.placeholder().contains("channel"));
        let x = (0..prompt.match_count())
            .find(|&i| prompt.row(i).label == "x")
            .expect("x is a channel");
        assert_eq!(prompt.confirm(Some(x), ""), PickerOutcome::KeepOpen);
        assert!(prompt.take_advanced(), "the step advanced");
        assert!(prompt.placeholder().contains("column"));
        // The new step's options are the caller's columns.
        prompt.update_query("");
        let depth = (0..prompt.match_count())
            .find(|&i| prompt.row(i).label == "depth")
            .expect("the profile's columns are offered");
        assert_eq!(prompt.confirm(Some(depth), ""), PickerOutcome::Close);
        match prompt.take_ready() {
            Some(ChartEdit::SetChannel {
                channel, column, ..
            }) => {
                assert_eq!(channel, "x");
                assert_eq!(column, "depth");
            }
            other => panic!("expected a completed SetChannel, got {other:?}"),
        }
    }

    #[test]
    fn an_unprofiled_column_step_is_the_typed_value_shape() {
        let mut prompt = ArgPrompt::new(ArgCollector::set_channel(plot(), 0), Vec::new());
        prompt.update_query("");
        let y = (0..prompt.match_count())
            .find(|&i| prompt.row(i).label == "y")
            .expect("y is a channel");
        assert_eq!(prompt.confirm(Some(y), ""), PickerOutcome::KeepOpen);
        assert!(prompt.take_advanced());
        prompt.update_query("");
        assert_eq!(prompt.match_count(), 0, "no columns to enumerate");
        assert_eq!(
            prompt.empty_text(),
            None,
            "no matches is the normal condition here, not an outcome to narrate"
        );
        // The typed query is the value.
        assert_eq!(prompt.confirm(None, "depth"), PickerOutcome::Close);
        assert!(matches!(
            prompt.take_ready(),
            Some(ChartEdit::SetChannel { .. })
        ));
    }

    #[test]
    fn an_invalid_pick_narrates_and_stays_open() {
        let mut prompt = ArgPrompt::new(ArgCollector::set_channel(plot(), 0), Vec::new());
        prompt.update_query("");
        assert_eq!(prompt.confirm(None, "wobble"), PickerOutcome::KeepOpen);
        assert!(
            matches!(prompt.hint(), Some(PickerHint::Error(_))),
            "an invalid pick is narrated"
        );
        assert!(prompt.take_ready().is_none());
        assert!(!prompt.take_advanced());
    }

    #[test]
    fn an_empty_typed_confirm_is_inert() {
        let mut prompt = ArgPrompt::new(ArgCollector::set_channel(plot(), 0), Vec::new());
        prompt.update_query("zzz");
        assert_eq!(prompt.match_count(), 0);
        assert_eq!(prompt.confirm(None, ""), PickerOutcome::KeepOpen);
        assert!(prompt.hint().is_none(), "nothing was validated");
    }
}
