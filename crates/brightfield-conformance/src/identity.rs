//! Stable identities for AST components (ac-02).
//!
//! The deviation registry and the `SupportReport` both key against typed
//! identities sourced from `brightfield-spec`'s kind-registry enums — never
//! against strings. This guarantees that a deviation recorded today cannot
//! drift out of reference when the parser renames a variant.

use brightfield_spec::{ComponentKind, InputKind, InteractorKind, MarkKind};

/// Typed identity of a Mosaic-recognised component.
///
/// Sealed: adding a new kind of identity requires a registry edit, not a
/// stringly-typed surface. Each variant wraps a kind enum from
/// `brightfield-spec`; `.status()` flows through to the underlying
/// `ImplStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentIdentity {
    /// A mark — `mark: line`, `mark: dot`, etc.
    Mark(MarkKind),
    /// An interactor — `select: intervalX`, etc.
    Interactor(InteractorKind),
    /// An input widget — `input: menu`, etc.
    Input(InputKind),
    /// A layout/composition component — plot, hconcat, vconcat, legend, spacer.
    Component(ComponentKind),
}

impl ComponentIdentity {
    /// Canonical wire name of the component (matches the spec source).
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Mark(k) => k.wire_name(),
            Self::Interactor(k) => k.wire_name(),
            Self::Input(k) => k.wire_name(),
            Self::Component(k) => k.wire_name(),
        }
    }

    /// Implementation status from the underlying vocabulary registry.
    #[must_use]
    pub fn status(self) -> brightfield_spec::ImplStatus {
        match self {
            Self::Mark(k) => k.status(),
            Self::Interactor(k) => k.status(),
            Self::Input(k) => k.status(),
            Self::Component(k) => k.status(),
        }
    }
}

/// The AST position kind where an identity was observed.
///
/// Used by `SupportReport` to distinguish "this mark is unimplemented" from
/// "this interactor is unimplemented" when the same kind-name coincidence
/// happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Surface {
    /// Inside a `plot:` item, or as a bare composition-level `mark:`.
    Mark,
    /// Inside a `plot:` item as a `select:` entry.
    Interactor,
    /// As a composition-level `input:` widget.
    Input,
    /// A layout shape — `plot`, `hconcat`, `vconcat`, `hspace`, `vspace`, `legend`.
    Layout,
}

impl Surface {
    /// Human-readable name for diagnostics and reports.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mark => "mark",
            Self::Interactor => "interactor",
            Self::Input => "input",
            Self::Layout => "layout",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dfconf_identity_wire_name_flows_through() {
        let id = ComponentIdentity::Mark(MarkKind::Line);
        assert_eq!(id.wire_name(), "line");
    }

    #[test]
    fn dfconf_identity_status_flows_through() {
        // MarkKind::Rect is genuinely Unimplemented (no renderer/lowerer), so it
        // proves Unimplemented status flows through ComponentIdentity. (Line is
        // now Implemented and renders end-to-end.)
        let id = ComponentIdentity::Mark(MarkKind::Rect);
        assert_eq!(id.status(), brightfield_spec::ImplStatus::Unimplemented);
    }

    #[test]
    fn dfconf_surface_as_str() {
        assert_eq!(Surface::Mark.as_str(), "mark");
        assert_eq!(Surface::Interactor.as_str(), "interactor");
        assert_eq!(Surface::Input.as_str(), "input");
        assert_eq!(Surface::Layout.as_str(), "layout");
    }
}
