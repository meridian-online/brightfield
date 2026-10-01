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

    /// What a composition found once the data had typed its axes: the warnings
    /// that no parse or analysis of the spec text alone could raise, worded as
    /// every other warning is. Advisory throughout: each is a thing that drew
    /// degraded, and none is a thing that did not draw.
    #[must_use]
    pub fn from_composition(warnings: &[ParseWarning]) -> Self {
        Self {
            source: None,
            support: SupportReport::default(),
            diagnostics: warnings
                .iter()
                .map(|warning| Diagnostic {
                    severity: DiagnosticSeverity::Advisory,
                    wire_name: warning_wire_name(warning),
                    surface: warning_surface(warning),
                    message: warning.to_string(),
                })
                .collect(),
        }
    }

    /// These diagnostics with `found` after them, dropping any of `found` that
    /// is already here. The source and the preflight walk are these ones': what
    /// a composition found has neither.
    ///
    /// A composition is rebuilt on every gesture and so is what it found, while
    /// the load's diagnostics are kept; putting the two together each time
    /// rather than storing one in the other is what stops a repaint repeating a
    /// line.
    #[must_use]
    pub fn merged(mut self, found: Self) -> Self {
        for diagnostic in found.diagnostics {
            if !self.diagnostics.contains(&diagnostic) {
                self.diagnostics.push(diagnostic);
            }
        }
        self
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
        ParseWarning::UnconsumedMarkOption { mark, .. } | ParseWarning::UnconsumedSort { mark } => {
            mark.clone()
        }
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
        // The transform, not the channel. A banner headlined `x` names
        // something every spec in the corpus contains; `bin` names the word
        // the author actually wrote and can search their own file for.
        ParseWarning::UnconsumedChannelTransform { transform, .. } => transform.clone(),
        // The colliding NAME, not the channel: it is the word the author wrote
        // and the one they can search their own file for.
        ParseWarning::ColourNameShadowsColumn { name, .. } => name.clone(),
        ParseWarning::NonNumericInset { attribute }
        | ParseWarning::NonStringLabel { attribute }
        | ParseWarning::InvalidTickCount { attribute }
        | ParseWarning::InvalidGridSwitch { attribute }
        | ParseWarning::InvalidAxisEndSwitch { attribute }
        | ParseWarning::InvalidAxisReverseSwitch { attribute }
        | ParseWarning::InvalidTickFormat { attribute, .. }
        | ParseWarning::UnreadDateDirective { attribute, .. }
        | ParseWarning::TickFormatOnWrongAxis { attribute, .. }
        | ParseWarning::UnreadAxisAttribute { attribute, .. } => attribute.clone(),
        ParseWarning::UnknownProjection { value } => value.clone(),
        ParseWarning::AspectRatioWithProjection { mark }
        | ParseWarning::MarkCannotProject { mark, .. } => mark.clone(),
        ParseWarning::IntervalBrushUnderCurvedProjection { interactor, .. } => interactor.clone(),
        // The widget the author asked for by name, so the banner names
        // something they can search their own file for.
        ParseWarning::IntervalSliderIncomplete { .. } => "slider".to_string(),
        ParseWarning::VersionMismatch { .. } => String::new(),
    }
}

/// Where in the spec a warning came from, in words.
fn warning_surface(warning: &ParseWarning) -> &'static str {
    match warning {
        ParseWarning::Unimplemented { surface, .. } => surface.label(),
        ParseWarning::UnconsumedMarkOption { .. }
        | ParseWarning::UnconsumedSort { .. }
        | ParseWarning::AspectRatioWithProjection { .. }
        | ParseWarning::MarkCannotProject { .. }
        | ParseWarning::HighlightOnAggregate { .. } => "mark",
        ParseWarning::InteractorBindingMissing { .. }
        | ParseWarning::InteractorBindingNonSelection { .. }
        | ParseWarning::IntervalBrushUnderCurvedProjection { .. }
        | ParseWarning::HighlightBindingMissing { .. }
        | ParseWarning::HighlightBindingNonSelection { .. } => "interactor",
        // The node parses as an interactor (`select:` wins the discriminator)
        // but what is missing is the input widget's, so the surface a reader
        // should look at is the input.
        ParseWarning::IntervalSliderIncomplete { .. } => "input",
        ParseWarning::LegendBindingMissing { .. }
        | ParseWarning::LegendBindingNonSelection { .. }
        | ParseWarning::LegendBindingNonCrossfilter { .. } => "legend",
        ParseWarning::ParamTypeMismatch { .. } | ParseWarning::DeadParam { .. } => "param",
        ParseWarning::NonNumericInset { .. }
        // `projectionType` is a plot attribute in Mosaic and this build reads
        // it nowhere else, so an unrecognised name is the plot's — held by
        // `a_mark_level_projection_is_a_key_nothing_reads` (brightfield-spec),
        // which shows a mark-level value is not judged as a projection name.
        | ParseWarning::UnknownProjection { .. }
        | ParseWarning::NonStringLabel { .. }
        | ParseWarning::InvalidTickCount { .. }
        | ParseWarning::InvalidGridSwitch { .. }
        | ParseWarning::InvalidAxisEndSwitch { .. }
        | ParseWarning::InvalidAxisReverseSwitch { .. }
        | ParseWarning::InvalidTickFormat { .. }
        | ParseWarning::UnreadDateDirective { .. }
        | ParseWarning::TickFormatOnWrongAxis { .. }
        | ParseWarning::UnreadAxisAttribute { .. } => "plot",
        ParseWarning::UnknownAggregate { .. }
        | ParseWarning::UnconsumedChannelTransform { .. }
        | ParseWarning::ColourNameShadowsColumn { .. } => "channel",
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
    ///
    /// The vehicle is a `sort:` on the wrong axis. `sort: { y: -x }` on a
    /// `barX` orders the band by the value and IS lowered, so it says nothing;
    /// `sort: { x: -y }` asks for the value axis — a continuous scale — to be
    /// re-ordered, which no lowerer does. Both are asserted, because a
    /// diagnostic that fires on the shape that works is the defect this file
    /// guards from the other side.
    #[test]
    fn dfconf_parse_warnings_reach_the_diagnostics() {
        let refused = diagnose(
            "data:\n  t: { file: t.parquet }\nplot:\n  - mark: barX\n    data: { from: t }\n    \
             x: a\n    y: b\n    sort: { x: -y, limit: 10 }\n",
        );
        let lines = refused.lines();
        assert!(
            lines.iter().any(|l| l.contains("sort")),
            "the uncomputed sort is said: {lines:?}"
        );
        assert!(
            refused.blocking().is_empty(),
            "…as advisory, because the chart still draws"
        );

        let honoured = diagnose(
            "data:\n  t: { file: t.parquet }\nplot:\n  - mark: barX\n    data: { from: t }\n    \
             x: a\n    y: b\n    sort: { y: -x, limit: 10 }\n",
        );
        assert!(
            !honoured.lines().iter().any(|l| l.contains("sort")),
            "the lowered sort must say nothing: {:?}",
            honoured.lines()
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

    /// **AC4 — a bad tick count is named in the warning banner with its
    /// key.** `xTicks: -3` is the card's own example: advisory (the plot
    /// still draws, at the default count), naming `xTicks` as the wire name
    /// and `plot` as the surface, mirroring
    /// `dfconf_blocking_entry_names_the_wire_name_and_surface`'s shape for the
    /// advisory tier.
    #[test]
    fn dfconf_advisory_entry_names_a_bad_tick_count() {
        let d = diagnose(
            "data:\n  t: { file: t.parquet }\nplot:\n  - mark: dot\n    data: { from: t }\n    \
             x: a\n    y: b\nxTicks: -3\n",
        );
        assert!(d.blocking().is_empty(), "the plot still draws: {d:?}");
        let advisory = d.advisory();
        let hit = advisory
            .iter()
            .find(|diag| diag.wire_name == "xTicks")
            .unwrap_or_else(|| panic!("no advisory names `xTicks`: {:?}", d.lines()));
        assert_eq!(hit.surface, "plot");
        assert!(
            hit.message.contains("xTicks"),
            "the sentence names it too: {}",
            hit.message
        );
    }

    /// **A gridline switch that is no `true` or `false` is named in the warning
    /// banner with its key.** `yGrid: 'off'` is an analyst's likely slip:
    /// advisory (the plot still draws, with the gridlines it draws without the
    /// key), naming `yGrid` as the wire name and `plot` as the surface, so the
    /// setting that was dropped is not dropped in silence.
    #[test]
    fn dfconf_advisory_entry_names_a_bad_grid_switch() {
        let d = diagnose(
            "data:\n  t: { file: t.parquet }\nplot:\n  - mark: dot\n    data: { from: t }\n    \
             x: a\n    y: b\nyGrid: 'off'\n",
        );
        assert!(d.blocking().is_empty(), "the plot still draws: {d:?}");
        let advisory = d.advisory();
        let hit = advisory
            .iter()
            .find(|diag| diag.wire_name == "yGrid")
            .unwrap_or_else(|| panic!("no advisory names `yGrid`: {:?}", d.lines()));
        assert_eq!(hit.surface, "plot");
        assert!(
            hit.message.contains("yGrid"),
            "the sentence names it too: {}",
            hit.message
        );

        let quiet = diagnose(
            "data:\n  t: { file: t.parquet }\nplot:\n  - mark: dot\n    data: { from: t }\n    \
             x: a\n    y: b\nyGrid: false\n",
        );
        assert!(
            quiet
                .advisory()
                .iter()
                .all(|diag| diag.wire_name != "yGrid"),
            "a literal switch is not a warning: {:?}",
            quiet.lines()
        );
    }

    /// **An axis-end switch that is no `true` or `false` is named in the warning
    /// banner with its key.** `yZero: 'yes'` is an analyst's likely slip:
    /// advisory (the plot still draws, with the axis ends it draws without the
    /// key), naming `yZero` as the wire name and `plot` as the surface, so the
    /// setting that was dropped is not dropped in silence.
    #[test]
    fn dfconf_advisory_entry_names_a_bad_axis_end_switch() {
        let d = diagnose(
            "data:\n  t: { file: t.parquet }\nplot:\n  - mark: dot\n    data: { from: t }\n    \
             x: a\n    y: b\nyZero: 'yes'\n",
        );
        assert!(d.blocking().is_empty(), "the plot still draws: {d:?}");
        let advisory = d.advisory();
        let hit = advisory
            .iter()
            .find(|diag| diag.wire_name == "yZero")
            .unwrap_or_else(|| panic!("no advisory names `yZero`: {:?}", d.lines()));
        assert_eq!(hit.surface, "plot");
        assert!(
            hit.message.contains("yZero"),
            "the sentence names it too: {}",
            hit.message
        );

        let quiet = diagnose(
            "data:\n  t: { file: t.parquet }\nplot:\n  - mark: dot\n    data: { from: t }\n    \
             x: a\n    y: b\nyZero: true\n",
        );
        assert!(
            quiet
                .advisory()
                .iter()
                .all(|diag| diag.wire_name != "yZero"),
            "a literal switch is not a warning: {:?}",
            quiet.lines()
        );
    }

    /// **An axis-reverse switch that is no `true` or `false` is named in the
    /// warning banner with its key.** `yReverse: 'yes'` is an analyst's likely
    /// slip: advisory (the plot still draws, with the axis running the way it
    /// does without the key), naming `yReverse` as the wire name and `plot` as
    /// the surface, so the setting that was dropped is not dropped in silence.
    #[test]
    fn dfconf_advisory_entry_names_a_bad_axis_reverse_switch() {
        let d = diagnose(
            "data:\n  t: { file: t.parquet }\nplot:\n  - mark: dot\n    data: { from: t }\n    \
             x: a\n    y: b\nyReverse: 'yes'\n",
        );
        assert!(d.blocking().is_empty(), "the plot still draws: {d:?}");
        let advisory = d.advisory();
        let hit = advisory
            .iter()
            .find(|diag| diag.wire_name == "yReverse")
            .unwrap_or_else(|| panic!("no advisory names `yReverse`: {:?}", d.lines()));
        assert_eq!(hit.surface, "plot");
        assert!(
            hit.message.contains("yReverse"),
            "the sentence names it too: {}",
            hit.message
        );

        let quiet = diagnose(
            "data:\n  t: { file: t.parquet }\nplot:\n  - mark: dot\n    data: { from: t }\n    \
             x: a\n    y: b\nyReverse: true\n",
        );
        assert!(
            quiet
                .advisory()
                .iter()
                .all(|diag| diag.wire_name != "yReverse"),
            "a literal switch is not a warning: {:?}",
            quiet.lines()
        );
    }

    /// **A bad tick format is named in the warning banner with its key and
    /// its value.** `xTickFormat: "~~"` is the card's own example: advisory
    /// (the axis still draws, its default text), naming `xTickFormat` as the
    /// wire name, `plot` as the surface, and the value in the sentence, since
    /// the key alone does not say what to change.
    #[test]
    fn dfconf_advisory_entry_names_a_bad_tick_format_and_its_value() {
        let d = diagnose(
            "data:\n  t: { file: t.parquet }\nplot:\n  - mark: dot\n    data: { from: t }\n    \
             x: a\n    y: b\nxTickFormat: \"~~\"\n",
        );
        assert!(d.blocking().is_empty(), "the plot still draws: {d:?}");
        let advisory = d.advisory();
        let hit = advisory
            .iter()
            .find(|diag| diag.wire_name == "xTickFormat")
            .unwrap_or_else(|| panic!("no advisory names `xTickFormat`: {:?}", d.lines()));
        assert_eq!(hit.surface, "plot");
        assert!(
            hit.message.contains("xTickFormat") && hit.message.contains("~~"),
            "the sentence names the key and the value: {}",
            hit.message
        );
    }

    /// A date format with a directive this build does not read is named in the
    /// warning banner with its key, its value and the directive: advisory (the
    /// axis still draws, its default text), naming `xTickFormat` as the wire
    /// name and `plot` as the surface.
    #[test]
    fn dfconf_advisory_entry_names_an_unread_date_directive() {
        let d = diagnose(
            "data:\n  t: { file: t.parquet }\nplot:\n  - mark: dot\n    data: { from: t }\n    \
             x: a\n    y: b\nxTickFormat: \"%B %K\"\n",
        );
        assert!(d.blocking().is_empty(), "the plot still draws: {d:?}");
        let advisory = d.advisory();
        let hit = advisory
            .iter()
            .find(|diag| diag.wire_name == "xTickFormat")
            .unwrap_or_else(|| panic!("no advisory names `xTickFormat`: {:?}", d.lines()));
        assert_eq!(hit.surface, "plot");
        assert!(
            hit.message.contains("xTickFormat")
                && hit.message.contains("%B %K")
                && hit.message.contains("`%K`"),
            "the sentence names the key, the value and the directive: {}",
            hit.message
        );
    }

    /// A tick format on an axis of the other kind is known only to a
    /// composition, which hands it in as a warning. It reads as any other
    /// advisory does, keeps the order it was found in, and is put after a
    /// load's own without repeating one the load already holds.
    #[test]
    fn dfconf_a_crossed_tick_format_from_a_composition_is_an_advisory_and_merges_once() {
        let crossed = ParseWarning::TickFormatOnWrongAxis {
            attribute: "yTickFormat".to_string(),
            value: "%b".to_string(),
            format: "date".to_string(),
            axis: "number".to_string(),
        };
        let found = LoadDiagnostics::from_composition(std::slice::from_ref(&crossed));
        assert!(found.blocking().is_empty());
        let advisory = found.advisory();
        assert_eq!(advisory.len(), 1, "{:?}", found.lines());
        assert_eq!(advisory[0].wire_name, "yTickFormat");
        assert_eq!(advisory[0].surface, "plot");
        assert!(
            advisory[0].message.contains("`%b`")
                && advisory[0]
                    .message
                    .contains("a date format on a number axis"),
            "{}",
            advisory[0].message
        );

        let load = diagnose(
            "data:\n  t: { file: t.parquet }\nplot:\n  - mark: dot\n    data: { from: t }\n    \
             x: a\n    y: b\nxTickFormat: \"~~\"\n",
        );
        let merged = load.clone().merged(found.clone()).merged(found);
        assert_eq!(
            merged.lines().len(),
            load.lines().len() + 1,
            "{:?}",
            merged.lines()
        );
        assert_eq!(
            merged.lines()[..load.lines().len()],
            load.lines()[..],
            "the load's lines come first, as they were"
        );
    }

    /// A format a spec is entitled to write draws with no warning: the four
    /// number formats of the vendored corpus, and the date formats this build
    /// reads.
    #[test]
    fn dfconf_a_readable_or_deferred_tick_format_says_nothing() {
        for format in ["s", "d", "%", "+f", ".2s", "%b", "%Y-%m"] {
            let d = diagnose(&format!(
                "data:\n  t: {{ file: t.parquet }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    \
                 x: a\n    y: b\nyTickFormat: '{format}'\n"
            ));
            assert!(
                d.advisory()
                    .iter()
                    .all(|diag| diag.wire_name != "yTickFormat"),
                "`{format}` is a format and must not be warned about: {:?}",
                d.lines()
            );
        }
    }
}
