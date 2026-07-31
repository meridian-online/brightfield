//! Activity — work in flight, as vocabulary.
//!
//! # Why this is a second enum and not five more [`RunState`]s
//!
//! [`RunState`] answers *"how does the data this surface previews relate to
//! what the pipeline currently specifies?"* — a relation between two states of
//! a document. Activity answers a different question entirely: *"is the app
//! doing something right now?"* An engine query in flight, a protocol run in
//! progress, a watched file mid-adoption — none of those is a claim about
//! data currency, and folding them into the run-state vocabulary would let a
//! spinner read as a freshness verdict. The two vocabularies are deliberately
//! disjoint, and the tests at the bottom hold them apart: no activity label
//! repeats a run-state label, and no activity ever wears the reserved status
//! inks — activity is always [`Tone::Neutral`], so progress can never be
//! mistaken for `fresh`'s green or `stale`'s amber.
//!
//! What the two vocabularies *share* is everything but the words: the same
//! [`StatusEntry`] carrier, the same [`Tone`] type, the same rail. One
//! spelling of "a line in the status rail", two things it can say.
//!
//! # The honesty line
//!
//! The speed guideline's rule — work that can exceed ~100 ms must show its
//! truth, and a cue for sub-perceptual work is decoration — is
//! [`HONESTY_LINE_MS`], declared here because both the editor's reload
//! indicator and [`ActivityLog`] compare against it. A synchronous engine
//! query marks and clears its activity inside one frame and therefore never
//! draws a cue, which is the correct answer twice over: under the line it
//! owes none, and over the line a synchronous query is blocking the frame
//! loop — there is no painted frame *during* which a cue could appear. The
//! seam is marked anyway, so the moment query work moves off the UI thread
//! (the engine's `QueryLoop` exists for exactly that) the indicator lights up
//! with no further wiring.
//!
//! [`RunState`]: crate::subject::RunState

use std::time::Instant;

use crate::subject::{HideAffordance, StatusEntry, StatusSide, Subject, Tone};

/// The honesty line, in milliseconds: work pending longer than this owes the
/// user a cue; work resolved faster owes silence. One declaration — the
/// editor's reload indicator and [`ActivityLog::entries`] both read it.
pub const HONESTY_LINE_MS: u128 = 100;

/// A kind of work the app can have in flight.
///
/// The list is the product's, not one surface's: a pane reports activity
/// through [`Activity::status_entry`] in its own [`Subject::status`], and the
/// shell's one [`ActivityIndicator`] collects every report into a single
/// rail entry — so two panes querying at once say "querying…" once, not
/// twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Activity {
    /// An engine query is in flight — a re-query behind a brush or slider, a
    /// windowed grid read, a count.
    EngineQuery,
    /// A protocol run is in progress. No run happens in-process today — runs
    /// belong to the pipeline runner — but the vocabulary carries the kind so
    /// the bridge, when it lands, reports through the same channel instead of
    /// minting a second one.
    ProtocolRun,
    /// A watched file changed and the change is being read or adopted — the
    /// window between "the watcher saw it" and "the app holds it".
    FileWatch,
}

impl Activity {
    /// Every kind, in the order the indicator lists them.
    pub const ALL: [Activity; 3] = [
        Activity::EngineQuery,
        Activity::ProtocolRun,
        Activity::FileWatch,
    ];

    /// The stable entry id this kind reports under — how the indicator
    /// recognises an activity entry in a [`Subject::status`] it is scanning.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Activity::EngineQuery => "activity-engine-query",
            Activity::ProtocolRun => "activity-protocol-run",
            Activity::FileWatch => "activity-file-watch",
        }
    }

    /// The short label. Present-progressive and ellipsised — the spelling of
    /// "still happening", never of a result.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Activity::EngineQuery => "querying…",
            Activity::ProtocolRun => "running…",
            Activity::FileWatch => "reloading…",
        }
    }

    /// A one-line explainer for an inspector or tooltip.
    #[must_use]
    pub const fn gloss(self) -> &'static str {
        match self {
            Activity::EngineQuery => "An engine query is in flight.",
            Activity::ProtocolRun => "A protocol run is in progress.",
            Activity::FileWatch => "A watched file changed on disk and is being re-read.",
        }
    }

    /// Always [`Tone::Neutral`] — the reconciliation with the run-state
    /// vocabulary, stated as code. The reserved status inks say what data
    /// *is*; activity says what the app is *doing*, and it does so in quiet
    /// text ink so the two can never be confused at a glance.
    #[must_use]
    pub const fn tone(self) -> Tone {
        Tone::Neutral
    }

    /// This activity as the status-rail entry a pane reports it with:
    /// trailing (in-flight work is standing state while it stands), cleared
    /// with the rail — an activity entry clears itself by resolving, not by
    /// being dismissed.
    #[must_use]
    pub fn status_entry(self) -> StatusEntry {
        StatusEntry {
            id: self.id(),
            side: StatusSide::Trailing,
            text: self.label().to_string(),
            tone: self.tone(),
            hide: HideAffordance::WithRail,
        }
    }

    /// Which activity `entry` reports, if it is an activity entry at all —
    /// the recognition the indicator and the shell's rail filter share, so
    /// "is this entry activity?" has one answer.
    #[must_use]
    pub fn of_entry(entry: &StatusEntry) -> Option<Activity> {
        Activity::ALL.into_iter().find(|a| a.id() == entry.id)
    }
}

// ---------------------------------------------------------------------------
// The indicator
// ---------------------------------------------------------------------------

/// The one activity indicator: every activity entry any pane reported,
/// collapsed into a single rail entry.
///
/// A namespace rather than a value — composing is a pure function of the
/// subjects handed in, and keeping it stateless is what keeps it honest: the
/// indicator can never show activity nothing is reporting this frame.
pub struct ActivityIndicator;

impl ActivityIndicator {
    /// The composed entry's stable id.
    ///
    /// **Nothing replaces anything by it.** [`Self::compose`] returns at most
    /// one entry, so there is never a second to replace — and the rail would not
    /// replace it if there were: it draws every entry it is handed. Unlike
    /// [`Activity::id`], which production reads to recognise a pane's own
    /// activity line, this one has no reader outside tests; it is how
    /// `status_rail.rs` picks the indicator out of a rail carrying several
    /// lines.
    pub const ID: &'static str = "activity";

    /// Collapse every activity entry in `subjects` into at most one rail
    /// entry. `None` when nothing reports activity — the rail draws nothing
    /// rather than an idle cue.
    ///
    /// Kinds are deduplicated and listed in [`Activity::ALL`]'s order, joined
    /// with ` · `, so "querying… · reloading…" reads the same whichever pane
    /// reported first.
    #[must_use]
    pub fn compose<'a>(subjects: impl IntoIterator<Item = &'a Subject>) -> Option<StatusEntry> {
        let mut live = [false; Activity::ALL.len()];
        for subject in subjects {
            for entry in &subject.status {
                if let Some(activity) = Activity::of_entry(entry) {
                    let index = Activity::ALL
                        .iter()
                        .position(|a| *a == activity)
                        .expect("of_entry answers only from ALL");
                    live[index] = true;
                }
            }
        }
        let text = Activity::ALL
            .into_iter()
            .zip(live)
            .filter(|(_, on)| *on)
            .map(|(a, _)| a.label())
            .collect::<Vec<_>>()
            .join(" · ");
        if text.is_empty() {
            return None;
        }
        Some(StatusEntry {
            id: Self::ID,
            side: StatusSide::Trailing,
            text,
            tone: Tone::Neutral,
            hide: HideAffordance::WithRail,
        })
    }
}

// ---------------------------------------------------------------------------
// The log — a document's in-flight work
// ---------------------------------------------------------------------------

/// The in-flight work a document currently has, for a pane to report from.
///
/// Work marks itself with [`ActivityLog::begin`] and clears with
/// [`ActivityLog::end`]; [`ActivityLog::entries`] renders what is *still*
/// live — and only past the honesty line, so a mark-and-clear inside one
/// frame (every synchronous path) never draws a cue it does not owe.
#[derive(Debug, Default)]
pub struct ActivityLog {
    live: Vec<(Activity, Instant)>,
}

impl ActivityLog {
    /// An empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark `activity` as begun. Re-beginning a live kind keeps the original
    /// clock — the honesty line measures how long the user has been waiting,
    /// not how recently the work re-asserted itself.
    pub fn begin(&mut self, activity: Activity) {
        if !self.is_live(activity) {
            self.live.push((activity, Instant::now()));
        }
    }

    /// Mark `activity` as resolved.
    pub fn end(&mut self, activity: Activity) {
        self.live.retain(|(a, _)| *a != activity);
    }

    /// Whether `activity` is currently marked.
    #[must_use]
    pub fn is_live(&self, activity: Activity) -> bool {
        self.live.iter().any(|(a, _)| *a == activity)
    }

    /// Whether anything is marked at all — what a frame loop polls to decide
    /// it owes another frame.
    #[must_use]
    pub fn any_live(&self) -> bool {
        !self.live.is_empty()
    }

    /// The status entries this log owes right now: one per live kind that has
    /// been pending past the honesty line.
    #[must_use]
    pub fn entries(&self) -> Vec<StatusEntry> {
        self.live
            .iter()
            .filter(|(_, since)| since.elapsed().as_millis() >= HONESTY_LINE_MS)
            .map(|(a, _)| a.status_entry())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Unit tests — the vocabulary reconciliation lives here
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subject::RunState;

    /// The reconciliation, half one: no activity label repeats a run-state
    /// label, so a rail can never say "fresh" about work or "querying…"
    /// about data.
    #[test]
    fn activity_labels_are_disjoint_from_run_state_labels() {
        for activity in Activity::ALL {
            for state in RunState::ALL {
                assert_ne!(
                    activity.label(),
                    state.label(),
                    "{activity:?} and {state:?} share a label"
                );
            }
        }
    }

    /// The reconciliation, half two: activity never wears the reserved status
    /// inks. `fresh` owns good, the stales own warning, `failed` owns
    /// critical — an activity in any of those colours would read as a
    /// currency claim from across the room.
    #[test]
    fn activity_is_always_neutral_never_a_status_ink() {
        for activity in Activity::ALL {
            assert_eq!(
                activity.tone(),
                Tone::Neutral,
                "{activity:?} wears a status ink"
            );
        }
    }

    /// Ids are distinct from each other and from the composed indicator's, so
    /// an entry's provenance is never ambiguous.
    #[test]
    fn activity_ids_are_distinct() {
        for a in Activity::ALL {
            for b in Activity::ALL {
                if a != b {
                    assert_ne!(a.id(), b.id());
                }
            }
            assert_ne!(a.id(), ActivityIndicator::ID);
        }
    }

    /// `of_entry` recognises exactly the entries `status_entry` writes.
    #[test]
    fn of_entry_round_trips() {
        for activity in Activity::ALL {
            let entry = activity.status_entry();
            assert_eq!(Activity::of_entry(&entry), Some(activity));
        }
        let state = RunState::Fresh.status_entry("run-state");
        assert_eq!(
            Activity::of_entry(&state),
            None,
            "a run-state entry is not activity"
        );
    }

    /// One indicator, however many panes report: two subjects reporting two
    /// kinds compose to a single entry listing both, in ALL's order.
    #[test]
    fn the_indicator_composes_every_report_into_one_entry() {
        let mut a = Subject::new("A", crate::subject::Icon("table"), test_context());
        a = a.with_status(Activity::FileWatch.status_entry());
        let mut b = Subject::new("B", crate::subject::Icon("table"), test_context());
        b = b.with_status(Activity::EngineQuery.status_entry());
        // A third pane repeats a kind — deduplicated, not repeated.
        let mut c = Subject::new("C", crate::subject::Icon("table"), test_context());
        c = c.with_status(Activity::EngineQuery.status_entry());

        let entry = ActivityIndicator::compose([&a, &b, &c]).expect("two kinds live");
        assert_eq!(entry.id, ActivityIndicator::ID);
        assert_eq!(entry.text, "querying… · reloading…");
        assert_eq!(entry.tone, Tone::Neutral);
    }

    /// Nothing reported, nothing composed — the rail draws no idle cue.
    #[test]
    fn the_indicator_is_silent_when_nothing_reports() {
        let plain = Subject::new("A", crate::subject::Icon("table"), test_context())
            .with_status(RunState::StaleUpstream.status_entry("run-state"));
        assert_eq!(
            ActivityIndicator::compose([&plain]),
            None,
            "a run-state entry alone summons no activity indicator"
        );
    }

    /// The log renders only what is live past the honesty line: a
    /// mark-and-clear inside one frame draws nothing, standing work draws
    /// its entry.
    #[test]
    fn the_log_owes_a_cue_only_past_the_honesty_line() {
        let mut log = ActivityLog::new();
        log.begin(Activity::EngineQuery);
        log.end(Activity::EngineQuery);
        assert!(!log.any_live());
        assert!(log.entries().is_empty(), "resolved work owes no cue");

        log.begin(Activity::FileWatch);
        assert!(log.any_live());
        assert!(
            log.entries().is_empty(),
            "under the honesty line, silence — a cue for sub-perceptual work \
             is decoration"
        );
        std::thread::sleep(std::time::Duration::from_millis(
            u64::try_from(HONESTY_LINE_MS).expect("small") + 20,
        ));
        let entries = log.entries();
        assert_eq!(entries.len(), 1, "past the line, the cue is owed");
        assert_eq!(entries[0].id, Activity::FileWatch.id());
    }

    fn test_context() -> brightfield_keys::BindingContext {
        brightfield_keys::BindingContext::Workspace
    }
}
