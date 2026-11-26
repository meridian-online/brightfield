//! The palette filter: fuzzy-match over longname + help, scoped by the
//! focused altitude, ranked by fuzzy score (non-empty query) or frequency then a
//! per-session recency counter (empty query). Reserved verbs are INCLUDED, flagged
//! with their bucket — never hidden. Given what the window holds, a verb whose
//! declared need is missing is included too, disabled, with the sentence it says.

use crate::altitude::Altitude;
use crate::availability::{availability, Situation};
use crate::fuzzy::fuzzy_score;
use crate::registry::{ReservedReason, VerbEntry};
use std::collections::HashMap;

/// A per-session recency counter (a deliberate v1 simplification of the research's
/// per-user counter). Higher rank = used more recently.
#[derive(Debug, Default, Clone)]
pub struct RecencyCounter {
    seq: HashMap<&'static str, u64>,
    tick: u64,
}

impl RecencyCounter {
    /// A fresh counter (nothing used yet).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that a verb was just run (bumps it to most-recent).
    pub fn record(&mut self, longname: &'static str) {
        self.tick += 1;
        self.seq.insert(longname, self.tick);
    }

    /// The recency rank of a verb (0 if never used).
    #[must_use]
    pub fn rank(&self, longname: &str) -> u64 {
        self.seq.get(longname).copied().unwrap_or(0)
    }
}

/// A palette candidate row, ready to render.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteCandidate {
    /// The verb.
    pub longname: &'static str,
    /// One-line help.
    pub help: &'static str,
    /// The bound key shown inline; `None` for reserved.
    pub primary_key: Option<&'static str>,
    /// If reserved, its bucket reason (the greyed flag).
    pub reserved_reason: Option<ReservedReason>,
    /// The sentence the verb says when a need it declares is missing from the
    /// situation the filter was given; `None` when it is available, and when
    /// no situation was given.
    pub missing: Option<String>,
    /// Whether the verb can be run: it is not reserved, and no need is missing.
    pub enabled: bool,
    /// The fuzzy score (0 for an empty query).
    pub score: i32,
}

/// Filter and rank the registry for the palette at `altitude` with `query`.
///
/// Candidate set = verbs applicable at the altitude whose longname or help fuzzy-
/// matches the query (an empty query matches all). Reserved verbs are included,
/// flagged. Non-empty query ranks by fuzzy score; an empty query ranks by
/// frequency tier then recency.
///
/// With a `situation`, each candidate's declared needs are read against it and
/// one with a need missing is disabled and carries its sentence. With `None`,
/// no need is read.
#[must_use]
pub fn palette_filter(
    reg: &[VerbEntry],
    altitude: Altitude,
    query: &str,
    recency: &RecencyCounter,
    situation: Option<&Situation>,
) -> Vec<PaletteCandidate> {
    let mut candidates: Vec<PaletteCandidate> = reg
        .iter()
        .filter(|v| v.applies_at(altitude))
        .filter_map(|v| {
            // Fuzzy over BOTH longname and help; take the better of the two.
            let name_score = fuzzy_score(query, v.longname);
            let help_score = fuzzy_score(query, v.help);
            let score = match (name_score, help_score) {
                (Some(a), Some(b)) => a.max(b),
                (Some(a), None) | (None, Some(a)) => a,
                (None, None) => return None,
            };
            // A reserved verb says its reason, and declares no need.
            let missing = situation
                .filter(|_| !v.is_reserved())
                .and_then(|situation| availability(v, situation).missing().map(str::to_string));
            Some(PaletteCandidate {
                longname: v.longname,
                help: v.help,
                primary_key: v.primary_key(),
                reserved_reason: v.reserved_reason,
                enabled: !v.is_reserved() && missing.is_none(),
                missing,
                score,
            })
        })
        .collect();

    if query.is_empty() {
        // Frequency tier, then recency, then name — stable and deterministic.
        candidates.sort_by(|a, b| {
            let fa = frequency_of(reg, a.longname);
            let fb = frequency_of(reg, b.longname);
            fb.cmp(&fa)
                .then_with(|| recency.rank(b.longname).cmp(&recency.rank(a.longname)))
                .then_with(|| a.longname.cmp(b.longname))
        });
    } else {
        // A command named exactly by the query first, so typing `open` gives
        // Open ahead of the commands whose names or help merely contain it.
        // Then fuzzy score, then frequency, then name.
        let exact = |c: &PaletteCandidate| c.longname.eq_ignore_ascii_case(query);
        candidates.sort_by(|a, b| {
            exact(b)
                .cmp(&exact(a))
                .then_with(|| b.score.cmp(&a.score))
                .then_with(|| frequency_of(reg, b.longname).cmp(&frequency_of(reg, a.longname)))
                .then_with(|| a.longname.cmp(b.longname))
        });
    }

    candidates
}

fn frequency_of(reg: &[VerbEntry], longname: &str) -> u8 {
    reg.iter()
        .find(|v| v.longname == longname)
        .and_then(|v| v.scores.as_ref())
        .map_or(0, |s| s.frequency)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::registry;

    #[test]
    fn a_query_that_is_a_commands_whole_name_lists_that_command_first() {
        let reg = registry();
        let got = palette_filter(
            &reg,
            Altitude::Protocol,
            "open",
            &RecencyCounter::new(),
            None,
        );
        assert_eq!(got.first().map(|c| c.longname), Some("open"));
        assert!(
            got.iter().any(|c| c.longname == "quick-open"),
            "quick open still matches `open`"
        );
    }

    #[test]
    fn query_colour_finds_cycle_colour_scheme_flagged_and_keyless() {
        // Not "ranks first": the shelf's `go-to-colour-cell` also names colour, and
        // `fuzzy_score` matches each query letter at its first occurrence, so a name
        // whose first `c` opens the word outscores `cycle-colour-scheme`, whose
        // first `c` is the `c` of `cycle`. The palette is offered this verb by
        // meaning; the ranking among the verbs that name colour is the scorer's.
        let reg = registry();
        let recency = RecencyCounter::new();
        let hits = palette_filter(&reg, Altitude::View, "colour", &recency, None);
        let hit = hits
            .iter()
            .find(|c| c.longname == "cycle-colour-scheme")
            .expect("a colour query finds the colour preview verb");
        // Reserved: listed with its reason, with no key and not to be run.
        assert!(!hit.enabled);
        assert_eq!(hit.reserved_reason, Some(ReservedReason::NeedsChartEditing));
        assert_eq!(hit.primary_key, None);
    }

    #[test]
    fn candidate_set_equals_scope_eligible_verbs() {
        let reg = registry();
        let recency = RecencyCounter::new();
        // Empty query at the view altitude → exactly the verbs applicable there.
        let got: std::collections::HashSet<&str> =
            palette_filter(&reg, Altitude::View, "", &recency, None)
                .iter()
                .map(|c| c.longname)
                .collect();
        let want: std::collections::HashSet<&str> = reg
            .iter()
            .filter(|v| v.applies_at(Altitude::View))
            .map(|v| v.longname)
            .collect();
        assert_eq!(got, want);
        // cycle-colour-scheme is view-only, so it appears at View but NOT at Dashboard.
        let dash: std::collections::HashSet<&str> =
            palette_filter(&reg, Altitude::Dashboard, "", &recency, None)
                .iter()
                .map(|c| c.longname)
                .collect();
        assert!(got.contains("cycle-colour-scheme"));
        assert!(!dash.contains("cycle-colour-scheme"));
    }

    #[test]
    fn empty_query_orders_by_frequency_then_recency() {
        let reg = registry();
        let mut recency = RecencyCounter::new();
        let ranked = palette_filter(&reg, Altitude::View, "", &recency, None);
        // Frequency is non-increasing across the list.
        let freqs: Vec<u8> = ranked
            .iter()
            .map(|c| frequency_of(&reg, c.longname))
            .collect();
        assert!(
            freqs.windows(2).all(|w| w[0] >= w[1]),
            "not frequency-ordered: {freqs:?}"
        );

        // Recency breaks ties: the two zooms have one frequency, and by name
        // zoom-in comes first. Recording zoom-out lifts it above its peer.
        assert_eq!(
            frequency_of(&reg, "zoom-out"),
            frequency_of(&reg, "zoom-in")
        );
        let position = |ranked: &[PaletteCandidate], name: &str| {
            ranked.iter().position(|c| c.longname == name).unwrap()
        };
        assert!(position(&ranked, "zoom-in") < position(&ranked, "zoom-out"));
        recency.record("zoom-out");
        let ranked2 = palette_filter(&reg, Altitude::View, "", &recency, None);
        assert!(
            position(&ranked2, "zoom-out") < position(&ranked2, "zoom-in"),
            "recorded verb should rank above its equal-frequency peer"
        );
    }

    #[test]
    fn reserved_verbs_appear_flagged_with_their_bucket() {
        let reg = registry();
        let recency = RecencyCounter::new();
        let hits = palette_filter(&reg, Altitude::View, "", &recency, None);
        let filter = hits
            .iter()
            .find(|c| c.longname == "filter-view")
            .expect("reserved verb is present");
        assert!(!filter.enabled, "reserved verb is not enabled");
        assert_eq!(
            filter.reserved_reason,
            Some(ReservedReason::NeedsKeyboardTarget)
        );
        assert!(
            filter.primary_key.is_none(),
            "reserved verb shows no bound key"
        );
    }

    #[test]
    fn an_unavailable_command_is_listed_disabled_with_its_sentence() {
        let reg = registry();
        let recency = RecencyCounter::new();
        let door = Situation::default();
        let rows = palette_filter(&reg, Altitude::View, "zoom", &recency, Some(&door));
        let zoom = rows
            .iter()
            .find(|c| c.longname == "zoom-in")
            .expect("zoom-in is listed with its need missing");
        assert!(!zoom.enabled);
        assert_eq!(
            zoom.missing.as_deref(),
            Some("Zoom in needs a chart on the canvas.")
        );
        assert_eq!(zoom.reserved_reason, None);

        // With no situation given, no need is read.
        let rows = palette_filter(&reg, Altitude::View, "zoom", &recency, None);
        let zoom = rows.iter().find(|c| c.longname == "zoom-in").unwrap();
        assert!(zoom.enabled);
        assert_eq!(zoom.missing, None);

        // And with the need met, the row is enabled.
        let chart = Situation::meeting(reg.iter().find(|v| v.longname == "zoom-in").unwrap().needs);
        let rows = palette_filter(&reg, Altitude::View, "zoom", &recency, Some(&chart));
        let zoom = rows.iter().find(|c| c.longname == "zoom-in").unwrap();
        assert!(zoom.enabled && zoom.missing.is_none());
    }
}
