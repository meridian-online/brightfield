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
//!   frequency/recency-ordered on an empty query. Reserved verbs are shown
//!   flagged and refuse to run, exactly as the corpus says.
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
use brightfield_keys::{
    palette_filter, registry, Altitude, HelpRow, PaletteCandidate, RecencyCounter, VerbEntry,
};
use brightfield_model::arg_collector::{ArgCollector, ArgOutcome};
use brightfield_spec::edit::ChartEdit;
use meridian_egui::{PickerDelegate, PickerHint, PickerOutcome, PickerRow};

// ---------------------------------------------------------------------------
// CommandPalette
// ---------------------------------------------------------------------------

/// The verb longnames the chart view's palette lists — exactly what
/// `MeridianApp::apply`'s `ViewKind::Charts` arm dispatches (`clear-selection`,
/// the navigation family), plus `open-home`, which that method handles before
/// the per-view match runs, so it takes effect no matter which view is
/// active. (`apply` is private, so this names it as plain code rather than
/// as a doc link — a link would resolve under `--document-private-items` but
/// not the public build, and doc links do not get to widen an API.)
///
/// `Altitude::View` in the registry is deliberately broader than this: it
/// also names verbs the chart view's editing bridge will wire later
/// (`add-mark`, `set-channel`, `undo`, ...) and meta verbs this shell handles
/// elsewhere rather than through `MeridianApp::apply`
/// (`open-help`, `toggle-focus`, `reload-spec`, `toggle-presentation`,
/// `cycle-colour-scheme`, ...) — see `window.rs:2074`'s reasoning. Listing
/// the raw altitude scope on the chart view would put rows in the palette
/// that confirm and silently do nothing; this curated list is what keeps
/// that from happening. `overlay_wiring.rs`'s dispatchability test walks
/// [`chart_palette_candidates`] (the SAME list the palette actually shows,
/// not a hand-copied mirror of it) and proves every one of these longnames
/// changes real state when confirmed.
pub const CHART_PALETTE_VERBS: &[&str] = &[
    "clear-selection",
    "open-home",
    crate::navigation::verb::PAN_LEFT,
    crate::navigation::verb::PAN_RIGHT,
    crate::navigation::verb::PAN_UP,
    crate::navigation::verb::PAN_DOWN,
    crate::navigation::verb::ZOOM_IN,
    crate::navigation::verb::ZOOM_OUT,
    crate::navigation::verb::CYCLE_AXIS_LOCK,
    crate::navigation::verb::RESET_EXTENT,
];

/// The command palette: every verb applicable at one altitude, fuzzy-matched
/// over longname + help, ranked by score (non-empty query) or frequency then
/// per-session recency (empty query). Reserved verbs are included, flagged,
/// and refuse to run — never hidden.
pub struct CommandPalette {
    reg: Vec<VerbEntry>,
    altitude: Altitude,
    recency: RecencyCounter,
    matches: Vec<PaletteCandidate>,
    hint: Option<PickerHint>,
    picked: Option<&'static str>,
}

impl CommandPalette {
    /// A palette over the keyboard registry at `altitude`. `recency` is the
    /// host's per-session counter, snapshotted at open — the host records
    /// the pick into its own copy so the *next* open ranks it.
    #[must_use]
    pub fn new(altitude: Altitude, recency: RecencyCounter) -> Self {
        Self {
            reg: registry(),
            altitude,
            recency,
            matches: Vec::new(),
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
    pub fn new_restricted(altitude: Altitude, recency: RecencyCounter, allow: &[&str]) -> Self {
        Self {
            reg: registry()
                .into_iter()
                .filter(|v| allow.contains(&v.longname))
                .collect(),
            altitude,
            recency,
            matches: Vec::new(),
            hint: None,
            picked: None,
        }
    }

    /// The verb the user confirmed, surrendered once. The host dispatches it
    /// and records it into its recency counter.
    pub fn take_picked(&mut self) -> Option<&'static str> {
        self.picked.take()
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
    let mut p =
        CommandPalette::new_restricted(Altitude::View, RecencyCounter::new(), CHART_PALETTE_VERBS);
    p.update_query("");
    (0..p.match_count())
        .map(|i| p.candidate(i).unwrap().longname)
        .collect()
}

impl PickerDelegate for CommandPalette {
    fn update_query(&mut self, query: &str) {
        self.matches = palette_filter(&self.reg, self.altitude, query, &self.recency);
        self.hint = None;
    }

    fn match_count(&self) -> usize {
        self.matches.len()
    }

    fn row(&self, index: usize) -> PickerRow {
        let c = &self.matches[index];
        let mut row = PickerRow::new(c.longname);
        row = match c.reserved_reason {
            Some(reason) => row.detail(format!("{} — {}", c.help, reason.reason())),
            None => row.detail(c.help),
        };
        if let Some(key) = c.primary_key {
            row = row.keystroke(key);
        }
        row
    }

    fn confirm(&mut self, index: Option<usize>, _query: &str) -> PickerOutcome {
        let Some(c) = index.and_then(|i| self.matches.get(i)) else {
            return PickerOutcome::KeepOpen;
        };
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

/// The keyboard help sheet: every verb with its keys and help, grouped by the
/// altitudes it applies at, filterable, read-only. Enter dismisses — there is
/// nothing to run from a reference sheet.
pub struct HelpSheet {
    lines: Vec<HelpLine>,
    matches: Vec<usize>,
}

impl HelpSheet {
    /// The sheet over the whole registry, grouped by altitude set in a fixed
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
            row = row.keystroke(*key);
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
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::analysis::ComponentPath;

    // -- CommandPalette -----------------------------------------------------

    fn palette_at(altitude: Altitude) -> CommandPalette {
        let mut p = CommandPalette::new(altitude, RecencyCounter::new());
        p.update_query("");
        p
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
        assert_eq!(top.keystroke.as_deref(), Some("shift-s"));
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
        for name in &names {
            assert!(
                CHART_PALETTE_VERBS.contains(name),
                "{name} is listed on the chart palette but not in CHART_PALETTE_VERBS"
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
            "change-mark-type awaits the editing bridge — it must not leak \
             onto the chart palette: {names:?}"
        );
    }

    #[test]
    fn new_restricted_excludes_a_view_scoped_verb_outside_the_allow_list() {
        let mut p = CommandPalette::new_restricted(
            Altitude::View,
            RecencyCounter::new(),
            &["clear-selection"],
        );
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
    fn recency_lifts_a_recorded_verb_under_an_empty_query() {
        let mut recency = RecencyCounter::new();
        recency.record("protocol-sibling-prev");
        let mut p = CommandPalette::new(Altitude::Protocol, recency);
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
    fn the_sheet_covers_the_whole_registry_and_reads_in_groups() {
        let mut sheet = HelpSheet::new();
        sheet.update_query("");
        assert_eq!(sheet.match_count(), registry().len());
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

    #[test]
    fn the_sheet_filters_without_losing_its_grouping() {
        let mut sheet = HelpSheet::new();
        sheet.update_query("drill");
        let full = registry().len();
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
