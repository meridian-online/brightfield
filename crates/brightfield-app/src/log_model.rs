//! Reload/save feedback log (card 0017, wsc_ac02) — framework-free.
//!
//! The Log panel's model: an append-only, capacity-capped history of every
//! feedback outcome the workspace notified about (reload rejections, editor
//! save refusals/conflicts/failures). Unlike the notification layer — where
//! a successful reload CLEARS the sticky error (#47's recovery rule) — the
//! log is history: recovery never removes entries, so an author can always
//! reconstruct what happened to a save after the toast is gone. No gpui
//! import may enter this file (semantic-layer rule).

use crate::reload_feedback::Severity;

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

/// The append-only feedback log, newest first. There is deliberately NO
/// clearing API: reload recovery clears the sticky error notification, never
/// the history (wsc_ac02's no-clear-on-recovery rule).
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
    use crate::reload_feedback::{clears_errors, ReloadOutcome};

    /// wsc_ac02 (append + order): appends land newest-first with the exact
    /// severity + message pair they were given.
    #[test]
    fn wsc_ac02_append_is_newest_first_verbatim() {
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

    /// wsc_ac02 (cap): the log holds at most LOG_CAP entries — the newest
    /// survive, the oldest fall off.
    #[test]
    fn wsc_ac02_cap_drops_the_oldest() {
        let mut log = FeedbackLog::default();
        for i in 0..(LOG_CAP + 10) {
            log.append(Severity::Warning, format!("entry {i}"));
        }
        assert_eq!(log.entries().len(), LOG_CAP, "capped at {LOG_CAP}");
        let last = LOG_CAP + 9;
        assert_eq!(log.entries()[0].message, format!("entry {last}"), "newest kept");
        assert_eq!(
            log.entries()[LOG_CAP - 1].message,
            "entry 10",
            "the 10 oldest fell off"
        );
    }

    /// wsc_ac02 (no-clear-on-recovery): a successful reload clears the
    /// sticky error NOTIFICATION (reload_feedback::clears_errors) but the
    /// log keeps its history — the model exposes no clearing path, so the
    /// entries survive the same recovery verbatim.
    #[test]
    fn wsc_ac02_recovery_clears_notifications_not_the_log() {
        let mut log = FeedbackLog::default();
        log.append(Severity::Error, "Reload skipped (keeping last good chart): boom");

        // The recovery outcome that clears the notification layer…
        assert!(clears_errors(&ReloadOutcome::Applied));

        // …has no counterpart here: history is retained in full.
        assert_eq!(log.entries().len(), 1);
        assert_eq!(
            log.entries()[0].message,
            "Reload skipped (keeping last good chart): boom"
        );
    }
}
