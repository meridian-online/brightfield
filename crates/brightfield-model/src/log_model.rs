//! Reload/save feedback log — framework-free.
//!
//! The Log panel's model: an append-only, capacity-capped history of every
//! feedback outcome the workspace notified about (reload rejections, editor
//! save refusals/conflicts/failures). Unlike the notification layer — where
//! a successful reload CLEARS the sticky error (#47's recovery rule) — the
//! log is history: recovery never removes entries, so an author can always
//! reconstruct what happened to a save after the toast is gone. No gpui
//! import may enter this file (semantic-layer rule).

/// How prominently a feedback outcome surfaces — the notification-severity
/// vocabulary shared by this log and the reload-feedback router (the router
/// still lives app-side until its own lift re-homes it; it re-exports this
/// type so its callers are unmoved).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The save was not applied and needs fixing (e.g. a parse error).
    Error,
    /// The save is valid but this window can't absorb it — restart to apply.
    Warning,
}

/// Maximum entries the log retains; the oldest fall off past the cap.
pub const LOG_CAP: usize = 200;

/// One logged feedback outcome: the SAME severity + message pair the
/// workspace notification carried (the log is the toasts' persistent
/// sibling).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// How prominently the outcome surfaced (error vs warning).
    pub severity: Severity,
    /// The notification's message, verbatim.
    pub message: String,
}

/// The Log-dock entries one reload/launch pass appends for assembly-time
/// menu resolution warnings: exactly one [`Severity::Warning`]
/// per warning string, per assembly pass — independent of whether the pass
/// subsequently gates (the watcher appends these BEFORE the
/// same_layout/chrome_divergence `continue`s, so a gated "restart to apply"
/// still co-surfaces the explanation instead of dropping it).
#[must_use]
pub fn resolution_warning_entries(warnings: &[String]) -> Vec<(Severity, String)> {
    warnings
        .iter()
        .map(|w| (Severity::Warning, w.clone()))
        .collect()
}

/// The append-only feedback log, newest first. There is deliberately NO
/// clearing API: reload recovery clears the sticky error notification, never
/// the history (the no-clear-on-recovery rule).
#[derive(Debug, Default)]
pub struct FeedbackLog {
    /// Entries, newest at index 0, at most [`LOG_CAP`].
    entries: Vec<LogEntry>,
}

impl FeedbackLog {
    /// Append an outcome (newest first); past [`LOG_CAP`] the oldest entry
    /// falls off.
    pub fn append(&mut self, severity: Severity, message: impl Into<String>) {
        self.entries.insert(
            0,
            LogEntry {
                severity,
                message: message.into(),
            },
        );
        self.entries.truncate(LOG_CAP);
    }

    /// The logged entries, newest first.
    #[must_use]
    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Append + order: appends land newest-first with the exact
    /// severity + message pair they were given.
    #[test]
    fn append_is_newest_first_verbatim() {
        let mut log = FeedbackLog::default();
        assert!(log.entries().is_empty());

        log.append(Severity::Error, "parse error: bad yaml");
        log.append(Severity::Warning, "restart to apply");

        assert_eq!(log.entries().len(), 2);
        assert_eq!(
            log.entries()[0],
            LogEntry {
                severity: Severity::Warning,
                message: "restart to apply".to_string()
            },
            "newest entry sits at index 0"
        );
        assert_eq!(
            log.entries()[1],
            LogEntry {
                severity: Severity::Error,
                message: "parse error: bad yaml".to_string()
            }
        );
    }

    /// Cap: the log holds at most LOG_CAP entries — the newest
    /// survive, the oldest fall off.
    #[test]
    fn cap_drops_the_oldest() {
        let mut log = FeedbackLog::default();
        for i in 0..(LOG_CAP + 10) {
            log.append(Severity::Warning, format!("entry {i}"));
        }
        assert_eq!(log.entries().len(), LOG_CAP, "capped at {LOG_CAP}");
        let last = LOG_CAP + 9;
        assert_eq!(
            log.entries()[0].message,
            format!("entry {last}"),
            "newest kept"
        );
        assert_eq!(
            log.entries()[LOG_CAP - 1].message,
            "entry 10",
            "the 10 oldest fell off"
        );
    }

    // The no-clear-on-recovery property is pinned at the SHELL level
    // (shell.rs's recovery_clears_the_notification_not_the_log,
    // review F4): it drives the real notify_reload_rejection +
    // clear_reload_error pair and asserts the notification cleared while
    // the log retained its entry. The model side of the guarantee is
    // structural — FeedbackLog deliberately exposes NO clearing API.
}
