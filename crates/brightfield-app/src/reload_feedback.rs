//! Reload-feedback routing — framework-free.
//!
//! Maps the hot-reload watcher's EXISTING outcomes onto an in-workspace
//! notification decision. The watcher's control flow is untouched: each
//! rejection branch already eprintlns and `continue`s; the tap merely asks
//! this module what (if anything) the workspace notification layer should
//! show for the same outcome. Successful reloads stay quiet — the canvas
//! repainting IS the feedback. No gpui import may enter this file
//! (semantic-layer rule).

/// The reload outcomes the watcher already distinguishes (main.rs's
/// `spawn_spec_watcher` branches, verbatim).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadOutcome<'a> {
    /// The rebuild succeeded and the plot scenes were hot-swapped.
    Applied,
    /// The pipeline failed (parse/analysis/engine error, or a contained
    /// panic) — the last good chart is kept.
    PipelineFailed(&'a str),
    /// The dashboard's plot layout changed structurally — restart to apply.
    LayoutChanged,
    /// A launch-fixed chrome facet diverged (title / legends / bindings /
    /// per-plot render metadata, named by the gate) — restart to apply.
    ChromeDiverged(&'a str),
}

/// How prominently a rejection surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The save was not applied and needs fixing (e.g. a parse error).
    Error,
    /// The save is valid but this window can't absorb it — restart to apply.
    Warning,
}

/// The workspace notification for a reload outcome, if any: rejections
/// surface (in addition to the existing stderr line), successes are silent.
#[must_use]
pub fn reload_notification(outcome: &ReloadOutcome<'_>) -> Option<(Severity, String)> {
    match outcome {
        ReloadOutcome::Applied => None,
        ReloadOutcome::PipelineFailed(e) => Some((
            Severity::Error,
            format!("Reload skipped (keeping last good chart): {e}"),
        )),
        ReloadOutcome::LayoutChanged => Some((
            Severity::Warning,
            "Reload skipped: dashboard layout changed — restart to apply".to_string(),
        )),
        ReloadOutcome::ChromeDiverged(what) => Some((
            Severity::Warning,
            format!("Reload skipped: {what} changed — restart to apply"),
        )),
    }
}

/// Whether a notification at this severity persists until dismissed or
/// superseded. Errors block the author's edit loop — a transient toast is
/// missed whenever the save came from an external editor (the window isn't
/// frontmost), leaving only the unchanged canvas as evidence. Warnings are
/// informational and may auto-hide.
#[must_use]
pub fn sticky(severity: Severity) -> bool {
    matches!(severity, Severity::Error)
}

/// Whether this outcome clears an outstanding sticky reload error: a
/// successful reload means the file is good again, and a stale error toast
/// would misreport the recovered state. Rejections don't clear — the error
/// path replaces the toast instead, so exactly one is live at a time.
#[must_use]
pub fn clears_errors(outcome: &ReloadOutcome<'_>) -> bool {
    matches!(outcome, ReloadOutcome::Applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reject-parse: a failed pipeline surfaces as an error
    /// notification carrying the pipeline's own message.
    #[test]
    fn aws_ac05_parse_failure_notifies_with_the_error() {
        let outcome = ReloadOutcome::PipelineFailed("parse error: mapping values are not allowed");
        let (severity, message) = reload_notification(&outcome).expect("rejections surface");
        assert_eq!(severity, Severity::Error);
        assert!(
            message.contains("parse error: mapping values are not allowed"),
            "the reason reaches the notification: {message}"
        );
        assert!(message.contains("last good chart"), "explains the kept chart");
    }

    /// Reject-chrome: the chrome-divergence gate's "restart to
    /// apply" surfaces as a warning naming the diverged facet; a structural
    /// layout change warns the same way.
    #[test]
    fn aws_ac05_chrome_and_layout_divergence_notify_restart_to_apply() {
        let (severity, message) =
            reload_notification(&ReloadOutcome::ChromeDiverged("dashboard title"))
                .expect("rejections surface");
        assert_eq!(severity, Severity::Warning);
        assert!(message.contains("dashboard title"), "names what diverged: {message}");
        assert!(message.contains("restart to apply"));

        let (severity, message) =
            reload_notification(&ReloadOutcome::LayoutChanged).expect("rejections surface");
        assert_eq!(severity, Severity::Warning);
        assert!(message.contains("restart to apply"), "{message}");
    }

    /// Success stays quiet: an applied reload produces NO
    /// notification — the repainted canvas is the feedback.
    #[test]
    fn aws_ac05_successful_reload_stays_quiet() {
        assert_eq!(reload_notification(&ReloadOutcome::Applied), None);
    }

    /// correction 2026-07-08: error notifications persist until
    /// the author resolves them (a transient toast was missed in product
    /// use); restart-to-apply warnings stay transient.
    #[test]
    fn aws_ac05_errors_are_sticky_warnings_transient() {
        assert!(sticky(Severity::Error));
        assert!(!sticky(Severity::Warning));
    }

    /// correction 2026-07-08: recovery is self-cleaning — only a
    /// successful reload clears the outstanding error; rejections replace
    /// it (one live toast), and warnings never touch it.
    #[test]
    fn aws_ac05_only_a_successful_reload_clears_the_error() {
        assert!(clears_errors(&ReloadOutcome::Applied));
        assert!(!clears_errors(&ReloadOutcome::PipelineFailed("parse error")));
        assert!(!clears_errors(&ReloadOutcome::LayoutChanged));
        assert!(!clears_errors(&ReloadOutcome::ChromeDiverged("title")));
    }
}
