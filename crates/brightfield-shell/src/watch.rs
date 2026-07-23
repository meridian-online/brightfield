//! The document's file watcher — external spec/data changes, noticed.
//!
//! A composed dashboard is a claim about files: the spec it was composed
//! from, and every `file:` data source that spec named. The moment one of
//! those changes on disk, the claim is behind the truth — and nothing re-runs
//! automatically (the run model is explicit: an edit marks work, the user
//! spends the run), so the *representation* has to carry the fact instead.
//! This module is the noticing half: a poll over mtimes, the same mechanism
//! and cadence as the editor's own reload poll, owned by the document rather
//! than by one pane, so a data file changing under a chart is seen even when
//! no editor tab is drawn.
//!
//! # What a notice is, and is not
//!
//! A detected change surfaces as a plain status line — "spec changed on
//! disk", "data changed on disk" — which is a fact about *files*, stated in
//! the file's own terms. It is deliberately **not** a run-state: whether the
//! materialised data is now stale against the changed spec is the engine's
//! staleness computation to record and the run contract to carry, and the
//! shell inventing that verdict from an mtime would be exactly the second
//! staleness computation the run-state work forbids. The watcher reports the
//! event; the vocabulary that judges data currency stays where it is.
//!
//! # Own writes are not external changes
//!
//! The editor saving the spec moves the same mtime an external edit does.
//! The save path therefore tells the watcher about its own writes
//! ([`FileWatcher::note_own_write`]), which re-baselines the file instead of
//! reporting it — a notice that fired every time the user pressed save would
//! train the user to ignore the one that mattered.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use brightfield_workbench::{HideAffordance, StatusEntry, StatusSide, Tone};

/// How often the watcher stats its files — the editor's own reload cadence,
/// restated here for the same reason: often enough to feel immediate, rare
/// enough to cost nothing.
pub const WATCH_POLL: Duration = Duration::from_millis(300);

/// What role a watched file plays for the document — which notice its change
/// raises.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchRole {
    /// The spec the document was composed from.
    Spec,
    /// A `file:` data source the spec named.
    Data,
}

/// One watched file: its path, its role, and the last mtime the poll saw.
///
/// `seen: None` means the file was unreadable (or absent) at the last look —
/// a real state, not an error: a data file that appears later *is* a change.
#[derive(Clone, Debug)]
struct Watched {
    path: PathBuf,
    role: WatchRole,
    seen: Option<SystemTime>,
}

/// An external change the watcher has seen and nothing has resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalChange {
    /// Which role the changed file plays.
    pub role: WatchRole,
    /// The file that changed.
    pub path: PathBuf,
}

/// The mtime poll over a document's files.
///
/// Plain state and `std::fs` — no watcher thread, no platform notification
/// API, no new dependency. The poll is throttled to [`WATCH_POLL`] by
/// [`FileWatcher::poll`]; the frame loop keeps frames coming while anything
/// is watched (a poll nobody runs watches nothing).
#[derive(Debug, Default)]
pub struct FileWatcher {
    watched: Vec<Watched>,
    changes: Vec<ExternalChange>,
    last_poll: Option<Instant>,
}

impl FileWatcher {
    /// A watcher over nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the watch list: the spec (if the document has one) and the
    /// data files it names. Baselines every file at its current mtime — a
    /// watch starts from *now*, so history is not reported as news — and
    /// drops any recorded changes, since they described files the document
    /// no longer claims.
    pub fn watch(&mut self, spec: Option<PathBuf>, data: Vec<PathBuf>) {
        self.watched = spec
            .into_iter()
            .map(|path| (path, WatchRole::Spec))
            .chain(data.into_iter().map(|path| (path, WatchRole::Data)))
            .map(|(path, role)| Watched {
                seen: mtime(&path),
                path,
                role,
            })
            .collect();
        self.changes.clear();
        self.last_poll = None;
    }

    /// Whether anything is watched at all — what the frame loop reads to
    /// decide it owes the watcher a future frame.
    #[must_use]
    pub fn has_watches(&self) -> bool {
        !self.watched.is_empty()
    }

    /// The changes seen so far and not yet resolved, in detection order.
    #[must_use]
    pub fn changes(&self) -> &[ExternalChange] {
        &self.changes
    }

    /// Stat every watched file now, unthrottled — the poll [`Self::poll`]
    /// rate-limits. Returns whether anything new was detected, so a caller
    /// can ask for a repaint exactly when one is owed.
    pub fn poll_now(&mut self) -> bool {
        let mut news = false;
        for watched in &mut self.watched {
            let now = mtime(&watched.path);
            if now == watched.seen {
                continue;
            }
            watched.seen = now;
            let change = ExternalChange {
                role: watched.role,
                path: watched.path.clone(),
            };
            if !self.changes.contains(&change) {
                self.changes.push(change);
                news = true;
            }
        }
        news
    }

    /// The throttled poll the frame loop calls. Returns whether anything new
    /// was detected this call.
    pub fn poll(&mut self) -> bool {
        let due = self.last_poll.is_none_or(|at| at.elapsed() >= WATCH_POLL);
        if !due {
            return false;
        }
        self.last_poll = Some(Instant::now());
        self.poll_now()
    }

    /// The app itself wrote `path`: re-baseline it and withdraw any notice —
    /// disk now holds what the app put there, so there is nothing external
    /// to report.
    pub fn note_own_write(&mut self, path: &Path) {
        for watched in &mut self.watched {
            if watched.path == path {
                watched.seen = mtime(&watched.path);
            }
        }
        self.changes.retain(|c| c.path != path);
    }

    /// The status entries the recorded changes owe: at most one per role —
    /// three data files changing is one "data changed on disk" line, not
    /// three — each a standing fact that clears with the rail, worded as a
    /// file event and toned as a condition, never as a run-state.
    #[must_use]
    pub fn entries(&self) -> Vec<StatusEntry> {
        let mut out = Vec::new();
        if self.changes.iter().any(|c| c.role == WatchRole::Spec) {
            out.push(StatusEntry {
                id: "watch-spec",
                side: StatusSide::Trailing,
                text: "spec changed on disk".to_string(),
                tone: Tone::Warning,
                hide: HideAffordance::WithRail,
            });
        }
        if self.changes.iter().any(|c| c.role == WatchRole::Data) {
            out.push(StatusEntry {
                id: "watch-data",
                side: StatusSide::Trailing,
                text: "data changed on disk".to_string(),
                tone: Tone::Warning,
                hide: HideAffordance::WithRail,
            });
        }
        out
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

// ---------------------------------------------------------------------------
// Unit tests — real files, real mtimes
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bf-watch-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// Touch `path` with an mtime the previous state cannot share, so the
    /// poll's equality check sees the change without the test sleeping
    /// through filesystem timestamp granularity.
    fn touch_past(path: &Path, secs_ago: u64) {
        let t = SystemTime::now() - Duration::from_secs(secs_ago);
        let f = fs::File::options()
            .write(true)
            .open(path)
            .expect("open for touch");
        f.set_modified(t).expect("set mtime");
    }

    /// An external write to a watched data file is reported once, as a data
    /// change; a spec write as a spec change.
    #[test]
    fn an_external_change_is_reported_under_its_role() {
        let dir = scratch("roles");
        let spec = dir.join("spec.yaml");
        let data = dir.join("rows.csv");
        fs::write(&spec, "a").expect("write spec");
        fs::write(&data, "b").expect("write data");

        let mut w = FileWatcher::new();
        w.watch(Some(spec.clone()), vec![data.clone()]);
        assert!(w.has_watches());
        assert!(!w.poll_now(), "nothing changed since the baseline");
        assert!(w.changes().is_empty());

        touch_past(&data, 100);
        assert!(w.poll_now(), "the data change is news");
        assert_eq!(
            w.changes(),
            &[ExternalChange {
                role: WatchRole::Data,
                path: data.clone()
            }]
        );
        assert!(!w.poll_now(), "the same change is not news twice");

        touch_past(&spec, 100);
        assert!(w.poll_now());
        assert_eq!(w.changes().len(), 2);
        assert_eq!(w.changes()[1].role, WatchRole::Spec);
    }

    /// The app's own save is not an external change: noted, the write moves
    /// the mtime and raises nothing.
    #[test]
    fn an_own_write_is_not_reported() {
        let dir = scratch("own");
        let spec = dir.join("spec.yaml");
        fs::write(&spec, "a").expect("write spec");

        let mut w = FileWatcher::new();
        w.watch(Some(spec.clone()), Vec::new());

        // The save: bytes then note, as the save path does.
        touch_past(&spec, 100);
        w.note_own_write(&spec);
        assert!(!w.poll_now(), "an acknowledged write is not news");
        assert!(w.changes().is_empty());

        // And a *later* external edit is still caught — the baseline moved
        // with the write rather than dying with it.
        touch_past(&spec, 50);
        assert!(w.poll_now());
        assert_eq!(w.changes().len(), 1);
    }

    /// An own write also withdraws a standing notice: after the user saves
    /// over an external edit, disk holds the app's bytes and the notice
    /// would be stale.
    #[test]
    fn an_own_write_withdraws_the_notice() {
        let dir = scratch("withdraw");
        let spec = dir.join("spec.yaml");
        fs::write(&spec, "a").expect("write spec");

        let mut w = FileWatcher::new();
        w.watch(Some(spec.clone()), Vec::new());
        touch_past(&spec, 100);
        assert!(w.poll_now());
        assert_eq!(w.entries().len(), 1);

        w.note_own_write(&spec);
        assert!(w.entries().is_empty(), "the notice is withdrawn");
    }

    /// A file that disappears is a change (its data is gone), and one that
    /// appears where the spec expects it is a change too.
    #[test]
    fn appearance_and_disappearance_are_changes() {
        let dir = scratch("absence");
        let data = dir.join("rows.csv");
        fs::write(&data, "b").expect("write data");

        let mut w = FileWatcher::new();
        w.watch(None, vec![data.clone()]);
        fs::remove_file(&data).expect("remove");
        assert!(w.poll_now(), "disappearance is a change");

        // A fresh watcher baselined on the absence: appearance is a change.
        let mut w = FileWatcher::new();
        w.watch(None, vec![data.clone()]);
        assert!(!w.poll_now());
        fs::write(&data, "b2").expect("recreate");
        touch_past(&data, 100);
        assert!(w.poll_now(), "appearance is a change");
    }

    /// Three data files changing owe one data notice, beside at most one
    /// spec notice — a rail, not a log.
    #[test]
    fn notices_collapse_per_role() {
        let dir = scratch("collapse");
        let a = dir.join("a.csv");
        let b = dir.join("b.csv");
        let spec = dir.join("spec.yaml");
        for p in [&a, &b, &spec] {
            fs::write(p, "x").expect("write");
        }

        let mut w = FileWatcher::new();
        w.watch(Some(spec.clone()), vec![a.clone(), b.clone()]);
        touch_past(&a, 100);
        touch_past(&b, 100);
        touch_past(&spec, 100);
        w.poll_now();
        assert_eq!(w.changes().len(), 3, "every change is recorded");

        let entries = w.entries();
        assert_eq!(entries.len(), 2, "but the rail says each fact once");
        assert_eq!(entries[0].id, "watch-spec");
        assert_eq!(entries[1].id, "watch-data");
        assert!(entries.iter().all(|e| e.tone == Tone::Warning));
    }

    /// Re-watching (a new document) drops old notices and re-baselines.
    #[test]
    fn rewatching_starts_clean() {
        let dir = scratch("rewatch");
        let spec = dir.join("spec.yaml");
        fs::write(&spec, "a").expect("write spec");

        let mut w = FileWatcher::new();
        w.watch(Some(spec.clone()), Vec::new());
        touch_past(&spec, 100);
        w.poll_now();
        assert!(!w.changes().is_empty());

        w.watch(Some(spec.clone()), Vec::new());
        assert!(w.changes().is_empty(), "a new watch starts from now");
        assert!(!w.poll_now());
    }
}
