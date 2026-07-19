//! Altitudes on the ComponentPath tree at which a verb can be meaningful.

/// The altitude of a focused node, used to gate which verbs apply (the SAME
/// predicate governs bare-key resolution and palette candidacy — see
/// [`crate::scope::bare_applicable`]).
///
/// v1 is floored at the view/plot: there is NO mark altitude, because a mark is
/// not an addressable node in v1 (a mark ring would be visually indistinguishable
/// from its view ring, and no v1 verb distinguishes them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Altitude {
    /// The dashboard root or an intermediate concat container.
    Dashboard,
    /// A single plot / view leaf — the focus floor.
    View,
    /// The protocol asset-graph panel: focus sits on an asset node
    /// or a step seam, and the topological/fold/drill grammar applies. A
    /// distinct altitude so protocol verbs never leak into the chart grammar
    /// and vice versa.
    Protocol,
}

impl Altitude {
    /// A short human-readable label (for breadcrumbs / help grouping).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Altitude::Dashboard => "dashboard",
            Altitude::View => "view",
            Altitude::Protocol => "protocol",
        }
    }
}
