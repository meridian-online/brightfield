//! The scope resolver and the shared bare/palette applicability
//! predicate.
//!
//! Selection-first: a bare verb acts on the focused node; `g` broadcasts to the
//! single root and is legal only for runtime verbs. Off-altitude and reserved
//! verbs are rejected with a structured, human-readable reason — never a silent
//! no-op, never an auto-walk.

use crate::altitude::Altitude;
use crate::registry::{Drives, ReservedReason, VerbEntry};
use brightfield_spec::analysis::ComponentPath;

/// The focus context a verb resolves against.
#[derive(Debug, Clone, Copy)]
pub struct ScopeContext<'a> {
    /// The currently focused node's path (the bare target).
    pub focused_path: &'a ComponentPath,
    /// The focused node's altitude.
    pub focused_altitude: Altitude,
    /// The single dashboard root path (the `g`-broadcast target).
    pub root_path: &'a ComponentPath,
}

/// The resolved target of a verb invocation, or a structured rejection.
#[derive(Debug, Clone, PartialEq)]
pub enum ScopeResolution {
    /// The verb resolves to act on this path.
    Resolved(ComponentPath),
    /// The verb is rejected, with a reason.
    Rejected(RejectReason),
}

/// Why a verb invocation was rejected (always carries a human-readable reason).
#[derive(Debug, Clone, PartialEq)]
pub enum RejectReason {
    /// The verb does not apply at the focused altitude.
    OffAltitude {
        /// The verb longname.
        verb: &'static str,
        /// The altitude focus was at.
        altitude: Altitude,
    },
    /// The verb is reserved (deferred); carries its bucket reason.
    Reserved(ReservedReason),
    /// `g` (broadcast) was used with a verb that is not a runtime verb.
    GNotAllowed {
        /// The verb longname.
        verb: &'static str,
    },
}

impl RejectReason {
    /// A human-readable message for the breadcrumb / palette flag.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            RejectReason::OffAltitude { verb, altitude } => {
                format!("`{verb}` does not apply at the {} altitude", altitude.label())
            }
            RejectReason::Reserved(reason) => reason.reason().to_string(),
            RejectReason::GNotAllowed { verb } => {
                format!("`g` (broadcast) is only for runtime verbs, not `{verb}`")
            }
        }
    }
}

/// Whether a verb is eligible for the `g` broadcast prefix — runtime verbs only.
#[must_use]
fn g_eligible(verb: &VerbEntry) -> bool {
    verb.drives == Drives::RuntimeDispatch
}

/// The shared applicability predicate: whether a BARE key for `verb`
/// fires at `altitude`. This is the exact same predicate the palette uses to
/// decide which verbs it *enables* at that altitude — so a bare key and the
/// palette agree, and an off-altitude verb rejects identically either way.
#[must_use]
pub fn bare_applicable(verb: &VerbEntry, altitude: Altitude) -> bool {
    !verb.is_reserved() && verb.applies_at(altitude)
}

/// Resolve `(verb, focus, g_prefix)` to a target path or a structured rejection.
///
/// - `g` resolves to the single root and is legal only for runtime verbs.
/// - a bare verb resolves to the focused node.
/// - a reserved verb rejects with its bucket reason.
/// - an off-altitude bare verb rejects with an off-altitude reason.
///
/// The `g` mechanism is fully resolved and tested even though no `g`-key is wired
/// in v1 (its first live use ships with the deferred cross-filter verbs).
#[must_use]
pub fn resolve_scope(verb: &VerbEntry, ctx: ScopeContext, g_prefix: bool) -> ScopeResolution {
    if g_prefix {
        // The broadcast prefix: legal only for runtime verbs, resolves to root.
        // (A structural / presentation / reserved verb under `g` is rejected
        // before the reserved check so the message names the real problem.)
        return if g_eligible(verb) {
            ScopeResolution::Resolved(ctx.root_path.clone())
        } else {
            ScopeResolution::Rejected(RejectReason::GNotAllowed { verb: verb.longname })
        };
    }

    // Bare invocation.
    if let Some(reason) = verb.reserved_reason {
        return ScopeResolution::Rejected(RejectReason::Reserved(reason));
    }
    if !verb.applies_at(ctx.focused_altitude) {
        return ScopeResolution::Rejected(RejectReason::OffAltitude {
            verb: verb.longname,
            altitude: ctx.focused_altitude,
        });
    }
    ScopeResolution::Resolved(ctx.focused_path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::registry;

    fn verb(longname: &str) -> VerbEntry {
        registry().into_iter().find(|v| v.longname == longname).unwrap()
    }

    fn ctx<'a>(
        focused: &'a ComponentPath,
        altitude: Altitude,
        root: &'a ComponentPath,
    ) -> ScopeContext<'a> {
        ScopeContext { focused_path: focused, focused_altitude: altitude, root_path: root }
    }

    #[test]
    fn bare_clear_selection_at_a_view_resolves_the_focused_plot() {
        let focused = ComponentPath("root/hconcat[1]".into());
        let root = ComponentPath("root".into());
        let res = resolve_scope(&verb("clear-selection"), ctx(&focused, Altitude::View, &root), false);
        assert_eq!(res, ScopeResolution::Resolved(focused));
    }

    #[test]
    fn g_resolves_to_the_single_root_for_a_runtime_verb() {
        let focused = ComponentPath("root/hconcat[1]".into());
        let root = ComponentPath("root".into());
        // g is tested even though no g-key is wired in v1.
        let res = resolve_scope(&verb("clear-selection"), ctx(&focused, Altitude::View, &root), true);
        assert_eq!(res, ScopeResolution::Resolved(root));
    }

    #[test]
    fn g_on_a_structural_or_non_runtime_verb_is_rejected() {
        let focused = ComponentPath("root/hconcat[0]".into());
        let root = ComponentPath("root".into());
        // Presentation is not a runtime verb.
        let res = resolve_scope(&verb("toggle-presentation"), ctx(&focused, Altitude::View, &root), true);
        assert!(matches!(res, ScopeResolution::Rejected(RejectReason::GNotAllowed { .. })));
        // A structural (SpecEdit) verb under g is likewise rejected — it is
        // Built but not runtime-dispatch, so not g-broadcast eligible.
        let res2 = resolve_scope(&verb("change-mark-type"), ctx(&focused, Altitude::View, &root), true);
        assert!(matches!(res2, ScopeResolution::Rejected(RejectReason::GNotAllowed { .. })));
    }

    #[test]
    fn bare_verb_off_its_altitude_is_rejected_with_reason() {
        let focused = ComponentPath("root".into());
        let root = ComponentPath("root".into());
        // cycle-colour-scheme is view-only; at the dashboard altitude it rejects.
        let res = resolve_scope(&verb("cycle-colour-scheme"), ctx(&focused, Altitude::Dashboard, &root), false);
        match res {
            ScopeResolution::Rejected(reason @ RejectReason::OffAltitude { .. }) => {
                assert!(reason.message().contains("dashboard"));
            }
            other => panic!("expected off-altitude rejection, got {other:?}"),
        }
    }

    #[test]
    fn bare_reserved_verb_rejects_with_its_bucket() {
        let focused = ComponentPath("root/hconcat[0]".into());
        let root = ComponentPath("root".into());
        let res = resolve_scope(&verb("filter-view"), ctx(&focused, Altitude::View, &root), false);
        assert_eq!(res, ScopeResolution::Rejected(RejectReason::Reserved(ReservedReason::NeedsKeyboardTarget)));
        // The command-log verbs (undo etc.) are now Built, so the
        // second still-reserved bucket case uses another NeedsKeyboardTarget verb.
        let res2 = resolve_scope(&verb("set-param"), ctx(&focused, Altitude::View, &root), false);
        assert_eq!(res2, ScopeResolution::Rejected(RejectReason::Reserved(ReservedReason::NeedsKeyboardTarget)));
    }

    #[test]
    fn bare_applicability_matches_altitude_gate_and_excludes_reserved() {
        let reg = registry();
        for v in &reg {
            for altitude in [Altitude::Dashboard, Altitude::View] {
                // The shared predicate is: not reserved AND applies at the altitude.
                assert_eq!(
                    bare_applicable(v, altitude),
                    !v.is_reserved() && v.applies_at(altitude),
                    "predicate mismatch for {} at {:?}",
                    v.longname,
                    altitude
                );
            }
        }
    }
}
