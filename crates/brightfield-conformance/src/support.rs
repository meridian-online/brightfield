//! Preflight `SupportReport` — walks a parsed `Spec` and emits one entry
//! per component whose `ImplStatus` is `Planned` or `Unimplemented`
//! (ac-02, ac-03, ac-04).
//!
//! The walker is deterministic: the same `Spec` produces a bytewise-
//! identical `SupportReport`. Walk order is document order — the order the
//! AST exposes via its `Component::Plot(items)` / `VConcat(items)` shapes.

use brightfield_spec::{ComponentKind, ImplStatus, Mark, SourceSpan, Spec};

use crate::identity::{ComponentIdentity, Surface};

/// One record in the `SupportReport`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportEntry {
    /// Typed identity of the component (mark kind, interactor kind, etc.).
    pub identity: ComponentIdentity,
    /// The AST position this identity was observed at.
    pub surface: Surface,
    /// Implementation status from the vocabulary registry.
    pub status: ImplStatus,
    /// Best-effort source span; `None` for components without parser-provided
    /// spans (the v1 parser records spans only where the deserialiser supplies
    /// them).
    pub span: Option<SourceSpan>,
}

/// The result of a preflight walk over a parsed spec.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SupportReport {
    /// All entries in document order.
    pub entries: Vec<SupportEntry>,
}

impl SupportReport {
    /// Entries whose status blocks rendering (`ImplStatus::Unimplemented`).
    /// `Planned` entries are advisory, not blocking.
    #[must_use]
    pub fn blocking(&self) -> Vec<&SupportEntry> {
        self.entries
            .iter()
            .filter(|e| e.status == ImplStatus::Unimplemented)
            .collect()
    }

    /// `true` iff there are no blocking entries. The gate between render-ok
    /// and render-error paths.
    #[must_use]
    pub fn is_renderable(&self) -> bool {
        self.blocking().is_empty()
    }
}

/// Walk the AST and collect every component whose implementation status is
/// not `Implemented`. Walk is deterministic; same input → same output.
#[must_use]
pub fn preflight(spec: &Spec) -> SupportReport {
    let mut entries = Vec::new();
    if let Some(root) = &spec.root {
        walk_component(root, &mut entries);
    }
    SupportReport { entries }
}

fn walk_component(component: &brightfield_spec::Component, entries: &mut Vec<SupportEntry>) {
    use brightfield_spec::Component;
    match component {
        Component::Plot(plot) => {
            maybe_push_layout(ComponentKind::Plot, entries);
            for item in &plot.items {
                walk_component(item, entries);
            }
        }
        Component::HConcat(cn) => {
            maybe_push_layout(ComponentKind::HConcat, entries);
            for item in &cn.items {
                walk_component(item, entries);
            }
        }
        Component::VConcat(cn) => {
            maybe_push_layout(ComponentKind::VConcat, entries);
            for item in &cn.items {
                walk_component(item, entries);
            }
        }
        Component::HSpace(_) => maybe_push_layout(ComponentKind::HSpace, entries),
        Component::VSpace(_) => maybe_push_layout(ComponentKind::VSpace, entries),
        Component::Legend(_) => maybe_push_layout(ComponentKind::Legend, entries),
        Component::Mark(m) => push_mark(m, entries),
        Component::Interactor(i) => push_interactor(i, entries),
        Component::Input(ip) => push_input(ip, entries),
    }
}

fn maybe_push_layout(kind: ComponentKind, entries: &mut Vec<SupportEntry>) {
    let status = kind.status();
    if status != ImplStatus::Implemented {
        entries.push(SupportEntry {
            identity: ComponentIdentity::Component(kind),
            surface: Surface::Layout,
            status,
            span: None,
        });
    }
}

fn push_mark(mark: &Mark, entries: &mut Vec<SupportEntry>) {
    let status = mark.kind.status();
    if status != ImplStatus::Implemented {
        entries.push(SupportEntry {
            identity: ComponentIdentity::Mark(mark.kind),
            surface: Surface::Mark,
            status,
            span: None,
        });
    }
}

fn push_interactor(interactor: &brightfield_spec::Interactor, entries: &mut Vec<SupportEntry>) {
    let status = interactor.kind.status();
    if status != ImplStatus::Implemented {
        entries.push(SupportEntry {
            identity: ComponentIdentity::Interactor(interactor.kind),
            surface: Surface::Interactor,
            status,
            span: None,
        });
    }
}

fn push_input(input: &brightfield_spec::Input, entries: &mut Vec<SupportEntry>) {
    let status = input.kind.status();
    if status != ImplStatus::Implemented {
        entries.push(SupportEntry {
            identity: ComponentIdentity::Input(input.kind),
            surface: Surface::Input,
            status,
            span: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::{parse_spec, Format};

    fn parse(source: &str) -> Spec {
        parse_spec(source, Format::Yaml)
            .expect("valid yaml")
            .spec
    }

    #[test]
    fn dfconf_support_entry_fields_round_trip() {
        // Construct each identity variant → wrap in SupportEntry → access fields.
        use brightfield_spec::{ComponentKind, InputKind, InteractorKind, MarkKind};
        let m = SupportEntry {
            identity: ComponentIdentity::Mark(MarkKind::Line),
            surface: Surface::Mark,
            status: ImplStatus::Unimplemented,
            span: None,
        };
        let i = SupportEntry {
            identity: ComponentIdentity::Interactor(InteractorKind::IntervalX),
            surface: Surface::Interactor,
            status: ImplStatus::Unimplemented,
            span: None,
        };
        let inp = SupportEntry {
            identity: ComponentIdentity::Input(InputKind::Menu),
            surface: Surface::Input,
            status: ImplStatus::Unimplemented,
            span: None,
        };
        let c = SupportEntry {
            identity: ComponentIdentity::Component(ComponentKind::Plot),
            surface: Surface::Layout,
            status: ImplStatus::Unimplemented,
            span: None,
        };
        assert_eq!(m.identity.wire_name(), "line");
        assert_eq!(i.identity.wire_name(), "intervalX");
        assert_eq!(inp.identity.wire_name(), "menu");
        assert_eq!(c.identity.wire_name(), "plot");
    }

    #[test]
    fn dfconf_preflight_reports_unimplemented_mark_only() {
        // Preflight records only non-Implemented marks. `hexbin` is genuinely
        // unimplemented (no renderer/lowerer), so it appears; this captures the
        // spirit: one unimplemented mark → one entry naming it. (line is now
        // Implemented and would be omitted.)
        let spec = parse("plot:\n  - mark: hexbin\n    data: { from: t }\n");
        let report = preflight(&spec);
        assert!(report.entries.iter().any(|e| matches!(
            e.identity,
            ComponentIdentity::Mark(brightfield_spec::MarkKind::Hexbin)
        )));
    }

    #[test]
    fn dfconf_preflight_is_deterministic() {
        let src = "plot:\n  - mark: line\n    data: { from: t }\n  - mark: dot\n    data: { from: t }\n";
        let spec = parse(src);
        let a = preflight(&spec);
        let b = preflight(&spec);
        assert_eq!(a, b);
    }

    #[test]
    fn dfconf_blocking_filters_unimplemented_only() {
        let mut r = SupportReport::default();
        r.entries.push(SupportEntry {
            identity: ComponentIdentity::Mark(brightfield_spec::MarkKind::Line),
            surface: Surface::Mark,
            status: ImplStatus::Planned,
            span: None,
        });
        r.entries.push(SupportEntry {
            identity: ComponentIdentity::Mark(brightfield_spec::MarkKind::Dot),
            surface: Surface::Mark,
            status: ImplStatus::Unimplemented,
            span: None,
        });
        let blocking = r.blocking();
        assert_eq!(blocking.len(), 1);
        assert_eq!(blocking[0].status, ImplStatus::Unimplemented);
    }

    #[test]
    fn dfconf_is_renderable_matches_blocking() {
        let empty = SupportReport::default();
        assert!(empty.is_renderable());

        let mut planned_only = SupportReport::default();
        planned_only.entries.push(SupportEntry {
            identity: ComponentIdentity::Mark(brightfield_spec::MarkKind::Line),
            surface: Surface::Mark,
            status: ImplStatus::Planned,
            span: None,
        });
        assert!(planned_only.is_renderable());

        let mut has_unimpl = SupportReport::default();
        has_unimpl.entries.push(SupportEntry {
            identity: ComponentIdentity::Mark(brightfield_spec::MarkKind::Dot),
            surface: Surface::Mark,
            status: ImplStatus::Unimplemented,
            span: None,
        });
        assert!(!has_unimpl.is_renderable());
    }
}
