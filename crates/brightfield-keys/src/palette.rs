//! The palette filter: fuzzy-match over longname + help, scoped by the
//! focused altitude, ranked by fuzzy score (non-empty query) or frequency then
//! the palette's frecency, kept between sessions (empty query). Reserved verbs
//! are INCLUDED, flagged with their bucket — never hidden. Given what the window holds, a verb whose
//! declared need is missing is included too, disabled, with the sentence it says.

use crate::altitude::Altitude;
use crate::availability::{availability, Situation};
use crate::fuzzy::fuzzy_score;
use crate::registry::{ReservedReason, VerbEntry};
use std::collections::HashMap;

/// How long a use takes to count half as much, in seconds: fourteen days.
pub const FRECENCY_HALF_LIFE_SECS: u64 = 14 * 24 * 60 * 60;

/// How many uses [`Frecency`] keeps: the newest 1,000. An older use falls off
/// when a new one is recorded.
pub const FRECENCY_CAP: usize = 1_000;

/// The version [`Frecency::to_json`] writes and [`Frecency::from_json`] reads.
/// A file at any other version reads as no history at all.
pub const FRECENCY_VERSION: u64 = 1;

/// What one fresh use weighs. A use's weight halves once per
/// [`FRECENCY_HALF_LIFE_SECS`] of its age, and never falls below 1, so a
/// command used once long ago still ranks above one never used
/// (`frecency_prefers_recent_and_frequent`).
const FRESH_USE_WEIGHT: f64 = 1_000_000.0;

/// The palette's frecency: which commands were run from it, and when, so the
/// ones used often and lately rank first under an empty query. It is plain
/// data, kept between sessions by the host, which records each pick with the
/// time it was made.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Frecency {
    /// Each use as the command's long name and the Unix time it was made in
    /// seconds, oldest first, at most [`FRECENCY_CAP`] of them.
    uses: Vec<(String, u64)>,
}

impl Frecency {
    /// No history: nothing used yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `longname` was run at `now_secs`, dropping the oldest use
    /// when more than [`FRECENCY_CAP`] are held.
    pub fn record(&mut self, longname: &str, now_secs: u64) {
        self.uses.push((longname.to_string(), now_secs));
        if self.uses.len() > FRECENCY_CAP {
            let excess = self.uses.len() - FRECENCY_CAP;
            self.uses.drain(..excess);
        }
    }

    /// How much `longname` has been used, as of `now_secs`: the sum over its
    /// uses of a fresh use's weight halved once per fourteen days of age, so
    /// it is the count of uses times a recency factor. Zero for a command never
    /// used; higher means used more, or more lately.
    #[must_use]
    pub fn weight(&self, longname: &str, now_secs: u64) -> u64 {
        self.uses
            .iter()
            .filter(|(name, _)| name == longname)
            .map(|&(_, at)| use_weight(at, now_secs))
            .sum()
    }

    /// The used commands' [`Frecency::weight`]s at `now_secs`, read in one
    /// pass, for a ranking that asks for many of them.
    fn weights(&self, now_secs: u64) -> HashMap<&str, u64> {
        let mut weights: HashMap<&str, u64> = HashMap::new();
        for (name, at) in &self.uses {
            *weights.entry(name.as_str()).or_default() += use_weight(*at, now_secs);
        }
        weights
    }

    /// The history as JSON: `{"version":1,"uses":[["zoom-in",1764115200],...]}`.
    #[must_use]
    pub fn to_json(&self) -> String {
        let uses: Vec<serde_json::Value> = self
            .uses
            .iter()
            .map(|(name, at)| serde_json::json!([name, at]))
            .collect();
        serde_json::json!({ "version": FRECENCY_VERSION, "uses": uses }).to_string()
    }

    /// The history [`Frecency::to_json`] wrote. `None` for text that is not
    /// that JSON, or that carries another version, so the caller starts with
    /// no history rather than guessing at a format it does not know. A file
    /// holding more than [`FRECENCY_CAP`] uses keeps the newest of them.
    #[must_use]
    pub fn from_json(text: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(text).ok()?;
        if value.get("version")?.as_u64()? != FRECENCY_VERSION {
            return None;
        }
        let uses = value
            .get("uses")?
            .as_array()?
            .iter()
            .map(|entry| {
                let pair = entry.as_array().filter(|pair| pair.len() == 2)?;
                Some((pair[0].as_str()?.to_string(), pair[1].as_u64()?))
            })
            .collect::<Option<Vec<_>>>()?;
        let mut frecency = Self::new();
        let skip = uses.len().saturating_sub(FRECENCY_CAP);
        frecency.uses = uses.into_iter().skip(skip).collect();
        Some(frecency)
    }
}

/// One use's weight at `now_secs`: a fresh use's, halved once per half-life of
/// its age, and at least 1. A use stamped after `now_secs`, from a clock that
/// moved back, counts as fresh.
fn use_weight(at_secs: u64, now_secs: u64) -> u64 {
    let age = now_secs.saturating_sub(at_secs) as f64;
    let factor = 0.5_f64.powf(age / FRECENCY_HALF_LIFE_SECS as f64);
    // In range: the product lies between 0 and FRESH_USE_WEIGHT.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let weight = (FRESH_USE_WEIGHT * factor).round() as u64;
    weight.max(1)
}

/// The current Unix time in seconds, the clock a host stamps a pick with and
/// ranks the palette at. A clock set before 1970 reads as 0.
#[must_use]
pub fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
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
/// frequency tier then `frecency` as of `now_secs`.
///
/// With a `situation`, each candidate's declared needs are read against it and
/// one with a need missing is disabled and carries its sentence. With `None`,
/// no need is read.
#[must_use]
pub fn palette_filter(
    reg: &[VerbEntry],
    altitude: Altitude,
    query: &str,
    frecency: &Frecency,
    now_secs: u64,
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
        // Frequency tier, then frecency, then name — stable and deterministic.
        let weights = frecency.weights(now_secs);
        let weight = |longname: &str| weights.get(longname).copied().unwrap_or(0);
        candidates.sort_by(|a, b| {
            let fa = frequency_of(reg, a.longname);
            let fb = frequency_of(reg, b.longname);
            fb.cmp(&fa)
                .then_with(|| weight(b.longname).cmp(&weight(a.longname)))
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

    /// A fixed moment the tests rank at: 2025-11-26 00:00 UTC.
    const NOW: u64 = 1_764_115_200;
    const DAY: u64 = 24 * 60 * 60;

    #[test]
    fn frecency_prefers_recent_and_frequent() {
        let mut f = Frecency::new();
        assert_eq!(
            f.weight("zoom-in", NOW),
            0,
            "a command never used weighs nothing"
        );

        // Recent beats old, at one use each.
        f.record("old", NOW - 60 * DAY);
        f.record("new", NOW - DAY);
        assert!(f.weight("new", NOW) > f.weight("old", NOW));

        // Frequent beats rare, at one age.
        f.record("rare", NOW - 3 * DAY);
        for _ in 0..3 {
            f.record("often", NOW - 3 * DAY);
        }
        assert_eq!(f.weight("often", NOW), 3 * f.weight("rare", NOW));

        // A use halves in weight over fourteen days.
        let mut g = Frecency::new();
        g.record("a", NOW);
        let fresh = g.weight("a", NOW);
        let half = g.weight("a", NOW + FRECENCY_HALF_LIFE_SECS);
        assert_eq!(half, fresh / 2);

        // A very old use still outranks none, and a use stamped later than now
        // counts as fresh.
        let mut h = Frecency::new();
        h.record("ancient", 0);
        h.record("ahead", NOW + DAY);
        assert!(h.weight("ancient", NOW) >= 1);
        assert_eq!(h.weight("ahead", NOW), fresh);
    }

    #[test]
    fn frecency_keeps_only_the_newest_1000_uses() {
        let mut f = Frecency::new();
        f.record("oldest", NOW - DAY);
        for i in 0..FRECENCY_CAP {
            f.record(if i == 0 { "second" } else { "filler" }, NOW);
        }
        assert_eq!(f.uses.len(), FRECENCY_CAP);
        assert_eq!(f.weight("oldest", NOW), 0, "the oldest use fell off");
        assert!(f.weight("second", NOW) > 0, "the next one is still held");

        // A file holding more than the cap keeps its newest uses.
        let mut big = Frecency::new();
        big.uses = (0..FRECENCY_CAP as u64 + 5)
            .map(|i| (format!("v{i}"), NOW + i))
            .collect();
        let read = Frecency::from_json(&big.to_json()).expect("it reads back");
        assert_eq!(read.uses.len(), FRECENCY_CAP);
        assert_eq!(read.uses[0].0, "v5");
    }

    #[test]
    fn frecency_round_trips() {
        let mut f = Frecency::new();
        f.record("zoom-in", NOW - DAY);
        f.record("pan-left", NOW);
        f.record("zoom-in", NOW);
        let back = Frecency::from_json(&f.to_json()).expect("its own JSON reads back");
        assert_eq!(back, f);
        assert_eq!(back.weight("zoom-in", NOW), f.weight("zoom-in", NOW));
        assert_eq!(
            Frecency::from_json(&Frecency::new().to_json()),
            Some(Frecency::new())
        );
    }

    #[test]
    fn frecency_refuses_text_it_did_not_write() {
        for text in [
            "",
            "{",
            "[]",
            r#"{"version":99,"uses":[]}"#,
            r#"{"uses":[]}"#,
            r#"{"version":1}"#,
            r#"{"version":1,"uses":[["zoom-in"]]}"#,
            r#"{"version":1,"uses":[["zoom-in",-3]]}"#,
        ] {
            assert_eq!(Frecency::from_json(text), None, "{text:?}");
        }
    }

    #[test]
    fn query_colour_finds_cycle_colour_scheme_flagged_and_keyless() {
        // Not "ranks first": the shelf's `go-to-colour-cell` also names colour, and
        // `fuzzy_score` matches each query letter at its first occurrence, so a name
        // whose first `c` opens the word outscores `cycle-colour-scheme`, whose
        // first `c` is the `c` of `cycle`. The palette is offered this verb by
        // meaning; the ranking among the verbs that name colour is the scorer's.
        let reg = registry();
        let frecency = Frecency::new();
        let hits = palette_filter(&reg, Altitude::View, "colour", &frecency, NOW, None);
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
        let frecency = Frecency::new();
        // Empty query at the view altitude → exactly the verbs applicable there.
        let got: std::collections::HashSet<&str> =
            palette_filter(&reg, Altitude::View, "", &frecency, NOW, None)
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
            palette_filter(&reg, Altitude::Dashboard, "", &frecency, NOW, None)
                .iter()
                .map(|c| c.longname)
                .collect();
        assert!(got.contains("cycle-colour-scheme"));
        assert!(!dash.contains("cycle-colour-scheme"));
    }

    #[test]
    fn empty_query_orders_by_frequency_then_frecency() {
        let reg = registry();
        let mut frecency = Frecency::new();
        let ranked = palette_filter(&reg, Altitude::View, "", &frecency, NOW, None);
        // Frequency is non-increasing across the list.
        let freqs: Vec<u8> = ranked
            .iter()
            .map(|c| frequency_of(&reg, c.longname))
            .collect();
        assert!(
            freqs.windows(2).all(|w| w[0] >= w[1]),
            "not frequency-ordered: {freqs:?}"
        );

        // Frecency breaks ties: the two zooms have one frequency, and by name
        // zoom-in comes first. Recording zoom-out lifts it above its peer.
        assert_eq!(
            frequency_of(&reg, "zoom-out"),
            frequency_of(&reg, "zoom-in")
        );
        let position = |ranked: &[PaletteCandidate], name: &str| {
            ranked.iter().position(|c| c.longname == name).unwrap()
        };
        assert!(position(&ranked, "zoom-in") < position(&ranked, "zoom-out"));
        frecency.record("zoom-out", NOW);
        let ranked2 = palette_filter(&reg, Altitude::View, "", &frecency, NOW, None);
        assert!(
            position(&ranked2, "zoom-out") < position(&ranked2, "zoom-in"),
            "recorded verb should rank above its equal-frequency peer"
        );
    }

    #[test]
    fn reserved_verbs_appear_flagged_with_their_bucket() {
        let reg = registry();
        let frecency = Frecency::new();
        let hits = palette_filter(&reg, Altitude::View, "", &frecency, NOW, None);
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
        let frecency = Frecency::new();
        let door = Situation::default();
        let rows = palette_filter(&reg, Altitude::View, "zoom", &frecency, NOW, Some(&door));
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
        let rows = palette_filter(&reg, Altitude::View, "zoom", &frecency, NOW, None);
        let zoom = rows.iter().find(|c| c.longname == "zoom-in").unwrap();
        assert!(zoom.enabled);
        assert_eq!(zoom.missing, None);

        // And with the need met, the row is enabled.
        let chart = Situation::meeting(reg.iter().find(|v| v.longname == "zoom-in").unwrap().needs);
        let rows = palette_filter(&reg, Altitude::View, "zoom", &frecency, NOW, Some(&chart));
        let zoom = rows.iter().find(|c| c.longname == "zoom-in").unwrap();
        assert!(zoom.enabled && zoom.missing.is_none());
    }
}
