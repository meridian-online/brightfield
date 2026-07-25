//! What a spec load has to SAY.
//!
//! Brightfield already knew which Mosaic features it could not render — the
//! vocabulary registry carries an [`ImplStatus`] per name and [`preflight`]
//! walks a parsed spec collecting every non-`Implemented` one — and it told
//! the user none of it. In parallel, every spec-load entry point in the shell
//! took a `ParseOutput`, moved `.spec` out of it, and dropped `.warnings` on
//! the floor. Two mechanisms, both built, both wired to nothing.
//!
//! [`LoadDiagnostics`] is the one value that carries both, produced by the
//! act of loading a spec and consumed by whatever surface is in front of a
//! person. It is deliberately renderer-free and framework-free: a `String`
//! per line, a severity, and enough structure to group by. The window raises
//! banners off it today; a diagnostics panel will read the same value without
//! this type changing shape.
//!
//! [`ImplStatus`]: brightfield_spec::ImplStatus

use std::fmt;

use brightfield_spec::{ImplStatus, ParseWarning, Spec};

use crate::support::{preflight, SupportReport};

/// How loudly one diagnostic speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticSeverity {
    /// Something in the spec will not draw at all. The user is looking at a
    /// picture that is missing a part they asked for.
    Blocking,
    /// Something degraded, was ignored, or is suspect, but the spec still
    /// renders.
    Advisory,
}

impl DiagnosticSeverity {
    /// A short lowercase label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Blocking => "blocking",
            Self::Advisory => "advisory",
        }
    }
}

/// One thing a load has to tell the user, already worded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Blocking or advisory.
    pub severity: DiagnosticSeverity,
    /// The Mosaic **wire name** this is about — `voronoi`, `nearest`,
    /// `symbol`, `barX`. The name as written in the spec, so a reader can
    /// find it by searching their own file. Empty only for diagnostics that
    /// are about the document rather than any one name.
    pub wire_name: String,
    /// Where in the spec it appeared, in words: `mark`, `interactor`,
    /// `input`, `layout`, `parse`, `analysis`.
    pub surface: &'static str,
    /// The sentence to show.
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.wire_name.is_empty() {
            write!(f, "{}: {}", self.surface, self.message)
        } else {
            write!(f, "{} `{}`: {}", self.surface, self.wire_name, self.message)
        }
    }
}

/// Everything one spec load found worth saying.
///
/// Built by [`LoadDiagnostics::collect`] at the point of load, so no entry
/// point can forget: the parse warnings are handed in (only the caller holds
/// the `ParseOutput`), the analysis warnings are handed in, and the preflight
/// walk happens here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadDiagnostics {
    /// The document these are about — a file name, a starting-point id, or
    /// `None` for spec text with no name.
    pub source: Option<String>,
    /// The preflight walk, kept whole so a consumer that wants the typed
    /// identities rather than the worded lines has them.
    pub support: SupportReport,
    /// Every diagnostic, blocking first, then advisory, each group in the
    /// order it was discovered.
    pub diagnostics: Vec<Diagnostic>,
}

impl LoadDiagnostics {
    /// Collect everything one load found.
    ///
    /// `parse_warnings` come off the `ParseOutput` the caller holds;
    /// `analysis_warnings` off the `SpecAnalysis`. Both are taken by
    /// reference rather than looked up, because only the caller has them and
    /// a signature that could be satisfied without them is a signature that
    /// will be.
    #[must_use]
    pub fn collect(
        source: Option<String>,
        spec: &Spec,
        parse_warnings: &[ParseWarning],
        analysis_warnings: &[ParseWarning],
    ) -> Self {
        let support = preflight(spec);
        let mut diagnostics = Vec::new();

        // Blocking first: the things that will not draw.
        for entry in support.blocking() {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Blocking,
                wire_name: entry.identity.wire_name().to_string(),
                surface: entry.surface.as_str(),
                message: format!(
                    "brightfield cannot render `{}` — it is {} in this build, so this part of \
                     the spec draws nothing",
                    entry.identity.wire_name(),
                    ImplStatus::Unimplemented,
                ),
            });
        }

        // Then everything the parse and the analysis observed. Both are
        // ParseWarning, and both were being discarded.
        //
        // One exclusion: the parser raises `Unimplemented` for the same names
        // preflight has just reported as blocking, so passing both through
        // would say each unrenderable feature twice — once loudly and once
        // quietly, in different words. The blocking line is the better of the
        // two (it says what it costs), so the quiet twin is dropped. A
        // `Planned` name is NOT blocking and keeps its advisory line.
        let blocking_names: Vec<&str> = support
            .blocking()
            .iter()
            .map(|e| e.identity.wire_name())
            .collect();
        for warning in parse_warnings.iter().chain(analysis_warnings) {
            if let ParseWarning::Unimplemented { name, .. } = warning {
                if blocking_names.contains(&name.as_str()) {
                    continue;
                }
            }
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Advisory,
                wire_name: warning_wire_name(warning),
                surface: warning_surface(warning),
                message: warning.to_string(),
            });
        }

        Self {
            source,
            support,
            diagnostics,
        }
    }

    /// `true` iff the load found nothing to say.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// The diagnostics about things that will not draw.
    #[must_use]
    pub fn blocking(&self) -> Vec<&Diagnostic> {
        self.of_severity(DiagnosticSeverity::Blocking)
    }

    /// The diagnostics about things that degraded but still drew.
    #[must_use]
    pub fn advisory(&self) -> Vec<&Diagnostic> {
        self.of_severity(DiagnosticSeverity::Advisory)
    }

    fn of_severity(&self, severity: DiagnosticSeverity) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == severity)
            .collect()
    }

    /// Every distinct blocking wire name, in first-seen order. What a banner
    /// headline names: a spec with nine unrenderable `voronoi` marks has one
    /// problem, not nine.
    #[must_use]
    pub fn blocking_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for d in self.blocking() {
            if !out.contains(&d.wire_name) {
                out.push(d.wire_name.clone());
            }
        }
        out
    }

    /// One line per diagnostic, ready to render.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.diagnostics.iter().map(ToString::to_string).collect()
    }
}

/// The wire name a warning is about, or empty when it is about no one name.
fn warning_wire_name(warning: &ParseWarning) -> String {
    match warning {
        ParseWarning::Unimplemented { name, .. } => name.clone(),
        ParseWarning::UnconsumedMarkOption { mark, .. } => mark.clone(),
        ParseWarning::DeadParam { name }
        | ParseWarning::InteractorBindingMissing { name }
        | ParseWarning::InteractorBindingNonSelection { name }
        | ParseWarning::LegendBindingMissing { name }
        | ParseWarning::LegendBindingNonSelection { name }
        | ParseWarning::LegendBindingNonCrossfilter { name, .. }
        | ParseWarning::HighlightBindingMissing { name }
        | ParseWarning::HighlightBindingNonSelection { name } => name.clone(),
        ParseWarning::ParamTypeMismatch { param, .. } => param.clone(),
        ParseWarning::HighlightOnAggregate { mark, .. } => mark.clone(),
        ParseWarning::UnknownOption { key, .. } => key.clone(),
        ParseWarning::UnknownAggregate { name, .. } => name.clone(),
        ParseWarning::NonNumericInset { attribute }
        | ParseWarning::NonStringLabel { attribute } => attribute.clone(),
        ParseWarning::UnknownProjection { value } => value.clone(),
        ParseWarning::VersionMismatch { .. } => String::new(),
    }
}

/// Where in the spec a warning came from, in words.
fn warning_surface(warning: &ParseWarning) -> &'static str {
    match warning {
        ParseWarning::Unimplemented { surface, .. } => surface.label(),
        ParseWarning::UnconsumedMarkOption { .. } | ParseWarning::HighlightOnAggregate { .. } => {
            "mark"
        }
        ParseWarning::InteractorBindingMissing { .. }
        | ParseWarning::InteractorBindingNonSelection { .. }
        | ParseWarning::HighlightBindingMissing { .. }
        | ParseWarning::HighlightBindingNonSelection { .. } => "interactor",
        ParseWarning::LegendBindingMissing { .. }
        | ParseWarning::LegendBindingNonSelection { .. }
        | ParseWarning::LegendBindingNonCrossfilter { .. } => "legend",
        ParseWarning::ParamTypeMismatch { .. } | ParseWarning::DeadParam { .. } => "param",
        ParseWarning::NonNumericInset { .. }
        | ParseWarning::NonStringLabel { .. }
        | ParseWarning::UnknownProjection { .. } => "plot",
        ParseWarning::UnknownAggregate { .. } => "channel",
        ParseWarning::UnknownOption { .. } | ParseWarning::VersionMismatch { .. } => "spec",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::{parse_spec, Format};

    fn diagnose(source: &str) -> LoadDiagnostics {
        let out = parse_spec(source, Format::Yaml).expect("parses");
        let analysis = brightfield_spec::analysis::analyse_spec(&out.spec).expect("analyses");
        LoadDiagnostics::collect(
            Some("test.yaml".to_string()),
            &out.spec,
            &out.warnings,
            &analysis.warnings,
        )
    }

    /// A spec brightfield renders end to end says nothing. A diagnostics
    /// channel that speaks on a clean load is one nobody reads.
    #[test]
    fn dfconf_a_clean_spec_produces_no_diagnostics() {
        let d = diagnose(
            "data:\n  t: { file: t.parquet }\nplot:\n  - mark: dot\n    data: { from: t }\n    \
             x: a\n    y: b\n",
        );
        assert!(d.is_empty(), "clean spec must be silent: {:?}", d.lines());
    }

    /// The blocking entry names the offending feature's WIRE name and where
    /// it appeared — the two things a reader needs to find it in their file.
    #[test]
    fn dfconf_blocking_entry_names_the_wire_name_and_surface() {
        let d = diagnose("plot:\n  - mark: voronoi\n    data: { from: t }\n");
        assert_eq!(
            d.blocking_names(),
            vec!["voronoi".to_string()],
            "the unrenderable mark is named: {:?}",
            d.lines()
        );
        let blocking = d.blocking();
        assert_eq!(blocking[0].surface, "mark");
        assert!(
            blocking[0].message.contains("voronoi"),
            "the sentence names it too: {}",
            blocking[0].message
        );
    }

    /// An unrenderable feature is reported once, not twice. Preflight and the
    /// parser both notice it; only the blocking line — the one that says what
    /// it costs — is shown.
    #[test]
    fn dfconf_an_unrenderable_feature_is_not_reported_twice() {
        let d = diagnose(
            "data:\n  t: { file: t.parquet }\nplot:\n  - mark: voronoi\n    data: { from: t }\n",
        );
        assert_eq!(d.blocking().len(), 1);
        assert!(
            !d.advisory().iter().any(|a| a.message.contains("voronoi")),
            "the parser's quieter twin of the blocking line must not also show: {:?}",
            d.lines()
        );
    }

    /// Nine copies of one unrenderable mark are one problem, not nine.
    #[test]
    fn dfconf_repeated_blocker_is_named_once() {
        let mut src = String::from("plot:\n");
        for _ in 0..9 {
            src.push_str("  - mark: voronoi\n    data: { from: t }\n");
        }
        let d = diagnose(&src);
        assert_eq!(d.blocking().len(), 9, "every occurrence is still recorded");
        assert_eq!(
            d.blocking_names(),
            vec!["voronoi".to_string()],
            "but the headline names it once"
        );
    }

    /// Parse warnings reach the diagnostics. This is the half that every
    /// spec-load entry point used to drop.
    #[test]
    fn dfconf_parse_warnings_reach_the_diagnostics() {
        let d = diagnose(
            "data:\n  t: { file: t.parquet }\nplot:\n  - mark: barX\n    data: { from: t }\n    \
             x: a\n    y: b\n    sort: { y: -x, limit: 10 }\n",
        );
        let lines = d.lines();
        assert!(
            lines.iter().any(|l| l.contains("sort")),
            "the ignored mark option is said: {lines:?}"
        );
        assert!(
            d.blocking().is_empty(),
            "…as advisory, because the chart still draws"
        );
    }

    /// Analysis warnings reach them too — `ParamTypeMismatch` and
    /// `DeadParam` live there, not on the ParseOutput.
    #[test]
    fn dfconf_analysis_warnings_reach_the_diagnostics() {
        let d = diagnose(
            "params:\n  count: 42\ndata:\n  t: { file: t.parquet }\nvconcat:\n  - input: table\n    \
             as: $count\n  - plot:\n    - mark: dot\n      data: { from: t }\n      x: a\n      y: b\n",
        );
        let lines = d.lines();
        assert!(
            lines.iter().any(|l| l.contains("count")),
            "the param type mismatch reaches the surface: {lines:?}"
        );
    }
}
