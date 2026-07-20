//! The palette filter: fuzzy-match over longname + help, scoped by the
//! focused altitude, ranked by fuzzy score (non-empty query) or frequency then a
//! per-session recency counter (empty query). Reserved verbs are INCLUDED, flagged
//! with their bucket — never hidden.

use crate::altitude::Altitude;
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
    /// Whether the verb can actually be run (false = reserved, shown greyed).
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
#[must_use]
pub fn palette_filter(
    reg: &[VerbEntry],
    altitude: Altitude,
    query: &str,
    recency: &RecencyCounter,
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
            Some(PaletteCandidate {
                longname: v.longname,
                help: v.help,
                primary_key: v.primary_key(),
                reserved_reason: v.reserved_reason,
                enabled: !v.is_reserved(),
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
        // Fuzzy score, then frequency, then name.
        candidates.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
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
    fn kbg_ac05_query_colour_ranks_cycle_colour_scheme_first() {
        let reg = registry();
        let recency = RecencyCounter::new();
        let hits = palette_filter(&reg, Altitude::View, "colour", &recency);
        assert_eq!(hits.first().map(|c| c.longname), Some("cycle-colour-scheme"));
    }

    #[test]
    fn kbg_ac05_candidate_set_equals_scope_eligible_verbs() {
        let reg = registry();
        let recency = RecencyCounter::new();
        // Empty query at the view altitude → exactly the verbs applicable there.
        let got: std::collections::HashSet<&str> =
            palette_filter(&reg, Altitude::View, "", &recency).iter().map(|c| c.longname).collect();
        let want: std::collections::HashSet<&str> =
            reg.iter().filter(|v| v.applies_at(Altitude::View)).map(|v| v.longname).collect();
        assert_eq!(got, want);
        // cycle-colour-scheme is view-only, so it appears at View but NOT at Dashboard.
        let dash: std::collections::HashSet<&str> =
            palette_filter(&reg, Altitude::Dashboard, "", &recency).iter().map(|c| c.longname).collect();
        assert!(got.contains("cycle-colour-scheme"));
        assert!(!dash.contains("cycle-colour-scheme"));
    }

    #[test]
    fn kbg_ac05_empty_query_orders_by_frequency_then_recency() {
        let reg = registry();
        let mut recency = RecencyCounter::new();
        let ranked = palette_filter(&reg, Altitude::View, "", &recency);
        // Frequency is non-increasing across the list.
        let freqs: Vec<u8> = ranked.iter().map(|c| frequency_of(&reg, c.longname)).collect();
        assert!(freqs.windows(2).all(|w| w[0] >= w[1]), "not frequency-ordered: {freqs:?}");

        // Recency breaks ties: two freq-5 nav verbs; recording one lifts it above
        // its equal-frequency peers.
        recency.record("focus-prev-sibling");
        let ranked2 = palette_filter(&reg, Altitude::View, "", &recency);
        let pos_prev = ranked2.iter().position(|c| c.longname == "focus-prev-sibling").unwrap();
        let pos_next = ranked2.iter().position(|c| c.longname == "focus-next-sibling").unwrap();
        assert!(pos_prev < pos_next, "recorded verb should rank above its equal-frequency peer");
    }

    #[test]
    fn kbg_ac05_reserved_verbs_appear_flagged_with_their_bucket() {
        let reg = registry();
        let recency = RecencyCounter::new();
        let hits = palette_filter(&reg, Altitude::View, "", &recency);
        let filter = hits.iter().find(|c| c.longname == "filter-view").expect("reserved verb is present");
        assert!(!filter.enabled, "reserved verb is not enabled");
        assert_eq!(filter.reserved_reason, Some(ReservedReason::NeedsKeyboardTarget));
        assert!(filter.primary_key.is_none(), "reserved verb shows no bound key");
    }
}
