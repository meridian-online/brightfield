//! The command-log model — framework-free.
//!
//! A dedicated, append-only history of the semantic edit results the keyboard
//! command log produces ("change-mark-type: -> bar"), an "N uncommitted edits"
//! count, and a commit-barrier marker. It is the SECOND bottom-dock citizen,
//! DISTINCT from [`crate::log_model::FeedbackLog`] — which stays the
//! rejection/diagnostics log. No gpui import may enter this file (semantic-layer
//! rule, mirroring `log_model` / `spec_save`).

/// Maximum entries retained; the oldest fall off past the cap.
pub const COMMAND_LOG_CAP: usize = 200;

/// One row of the command log, newest-first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandLogEntry {
    /// A semantic edit that was applied live ("change-mark-type: -> bar").
    Edit(String),
    /// A commit barrier ("committed 3 edits to disk"). Constructed by
    /// [`CommandLog::commit`] on the deliberate cmd-s commit action.
    Commit(String),
    /// A refused edit or a no-op undo, with its reason (authoring feedback).
    Refused(String),
}

impl CommandLogEntry {
    /// The human-readable text of this row.
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            CommandLogEntry::Edit(s)
            | CommandLogEntry::Commit(s)
            | CommandLogEntry::Refused(s) => s,
        }
    }

    /// Whether this row is a commit barrier. A public predicate exercised by
    /// the unit tests; the dock panel tags rows by matching the variant
    /// directly, so this is unused in the shim today.
    #[allow(dead_code)]
    #[must_use]
    pub fn is_barrier(&self) -> bool {
        matches!(self, CommandLogEntry::Commit(_))
    }
}

/// The append-only command log, newest first, tracking the uncommitted-edit
/// count and commit barriers. Deliberately separate from
/// [`crate::log_model::FeedbackLog`].
#[derive(Debug, Default)]
pub struct CommandLog {
    /// Entries, newest at index 0, at most [`COMMAND_LOG_CAP`].
    entries: Vec<CommandLogEntry>,
    /// Number of applied edits since the last commit (edits minus commits).
    uncommitted: usize,
}

impl CommandLog {
    /// A fresh, empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an applied edit (newest-first) and bump the uncommitted count.
    pub fn record_edit(&mut self, summary: impl Into<String>) {
        self.push(CommandLogEntry::Edit(summary.into()));
        self.uncommitted += 1;
    }

    /// Record a refused edit / no-op with its reason — authoring feedback that
    /// does NOT change the uncommitted count.
    pub fn record_refused(&mut self, reason: impl Into<String>) {
        self.push(CommandLogEntry::Refused(reason.into()));
    }

    /// Undo the last applied edit: pop the newest [`CommandLogEntry::Edit`] and
    /// decrement the uncommitted count. A no-op (returns `false`) when there is
    /// no uncommitted edit to pop (an empty log, or everything committed).
    pub fn record_undo(&mut self) -> bool {
        if self.uncommitted == 0 {
            return false;
        }
        // The newest uncommitted edit is the first `Edit` from the front (a
        // commit barrier is only ever inserted with uncommitted reset to 0, so
        // while uncommitted > 0 the front rows up to it are all Edits).
        if let Some(pos) = self
            .entries
            .iter()
            .position(|e| matches!(e, CommandLogEntry::Edit(_)))
        {
            self.entries.remove(pos);
            self.uncommitted -= 1;
            true
        } else {
            false
        }
    }

    /// Commit: insert a barrier recording how many edits were flushed and reset
    /// the uncommitted count. A no-op (returns `false`) when there is nothing to
    /// commit. Called on the cmd-s commit action; unit-tested.
    pub fn commit(&mut self) -> bool {
        if self.uncommitted == 0 {
            return false;
        }
        let n = self.uncommitted;
        let plural = if n == 1 { "edit" } else { "edits" };
        self.push(CommandLogEntry::Commit(format!("committed {n} {plural} to disk")));
        self.uncommitted = 0;
        true
    }

    /// The number of applied edits not yet committed.
    #[must_use]
    pub fn uncommitted(&self) -> usize {
        self.uncommitted
    }

    /// The logged entries, newest first.
    #[must_use]
    pub fn entries(&self) -> &[CommandLogEntry] {
        &self.entries
    }

    fn push(&mut self, entry: CommandLogEntry) {
        self.entries.insert(0, entry);
        self.entries.truncate(COMMAND_LOG_CAP);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clg_ac08_append_is_newest_first_and_counts_uncommitted() {
        let mut log = CommandLog::new();
        assert_eq!(log.uncommitted(), 0);
        log.record_edit("change-mark-type: -> bar");
        log.record_edit("set-channel: x -> temp");
        assert_eq!(log.uncommitted(), 2, "count tracks edits");
        assert_eq!(log.entries()[0].text(), "set-channel: x -> temp", "newest at index 0");
        assert_eq!(log.entries()[1].text(), "change-mark-type: -> bar");
    }

    #[test]
    fn clg_ac08_commit_resets_the_count_and_inserts_a_barrier() {
        let mut log = CommandLog::new();
        log.record_edit("a");
        log.record_edit("b");
        assert!(log.commit(), "commit with pending edits succeeds");
        assert_eq!(log.uncommitted(), 0, "commit resets the count");
        assert!(log.entries()[0].is_barrier(), "a barrier is inserted newest-first");
        assert!(log.entries()[0].text().contains("committed 2 edits"));
        // A second commit with nothing pending is a no-op.
        assert!(!log.commit());
    }

    #[test]
    fn clg_ac08_undo_pops_the_last_edit() {
        let mut log = CommandLog::new();
        log.record_edit("a");
        log.record_edit("b");
        assert!(log.record_undo(), "undo pops the newest edit");
        assert_eq!(log.uncommitted(), 1);
        assert_eq!(log.entries().len(), 1);
        assert_eq!(log.entries()[0].text(), "a");
        // Undo the last one, then undo on empty is a no-op.
        assert!(log.record_undo());
        assert_eq!(log.uncommitted(), 0);
        assert!(!log.record_undo(), "undo with nothing uncommitted is a no-op");
    }

    #[test]
    fn clg_ac08_undo_does_not_cross_a_commit_barrier() {
        let mut log = CommandLog::new();
        log.record_edit("a");
        log.commit();
        // Post-commit, the edit is sealed: undo is a no-op (uncommitted == 0).
        assert!(!log.record_undo());
        assert_eq!(log.uncommitted(), 0);
        // A fresh edit after the commit is undoable, and stops at the barrier.
        log.record_edit("b");
        assert!(log.record_undo());
        assert!(!log.record_undo());
    }

    #[test]
    fn clg_ac08_refused_is_feedback_only() {
        let mut log = CommandLog::new();
        log.record_refused("would empty the plot");
        assert_eq!(log.uncommitted(), 0, "a refusal does not count as an edit");
        assert!(matches!(log.entries()[0], CommandLogEntry::Refused(_)));
    }
}
