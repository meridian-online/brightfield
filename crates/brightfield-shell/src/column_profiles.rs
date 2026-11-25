//! **The column profiles of any table the inspector is asked about.**
//!
//! The window's session holds one table, the one the file opened as, and its
//! columns are profiled when the file opens. A table a later step makes has no
//! profile until something asks for one. The first ask starts a read on a
//! worker, and the answer is kept, so a second ask of the same table is served
//! from what was read.
//!
//! **The read is a preview's**: the data file is loaded on a connection of its
//! own, the steps the table is made through are run on it, and the table is
//! profiled there by the engine's own profile pass
//! ([`brightfield_engine::preview_profile_stoppable`]). Nothing reaches the
//! window's session, and the read is held to the preview's size, memory and
//! time limits.
//!
//! **What a kept profile is kept against** ([`ProfileKey`]): the table's name,
//! the content hash the run record gives it, and the steps it is made through.
//! A run that changes the table changes its hash, and an edit to one of its
//! steps changes the steps, so either reads the table again.
//!
//! **A read under way is said to be under way** ([`ProfileRead::Reading`]), so
//! the inspector draws a line saying so rather than drawing nothing while the
//! worker reads (`a_profile_still_reading_says_so_rather_than_showing_nothing`
//! in `tests/inspector_contract.rs`).

use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use brightfield_engine::{PreviewOp, PreviewStop};

use crate::one_step::ColumnFacts;
use crate::typed_sql::TypedSource;

/// What the inspector knows about a table's columns.
#[derive(Clone, Debug, PartialEq)]
pub enum ProfileRead {
    /// The table's columns are being read.
    Reading,
    /// The table's columns, in the table's order.
    Read(Rc<[ColumnFacts]>),
    /// Why the table's columns could not be read, in a sentence.
    Failed(String),
}

/// What a kept profile was read for: the table, the content hash the run
/// record gives it, and the steps it is made through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileKey {
    /// The table's name.
    pub table: String,
    /// The run record's content hash of the table, when it gives one.
    pub content_hash: Option<String>,
    /// The table the data file loads as, and each step the read makes after
    /// it, in order.
    pub chain: (String, Vec<(String, PreviewOp)>),
}

/// Why a worker found no columns: the sentence the inspector shows, and
/// whether asking again could answer otherwise.
struct Refused {
    sentence: String,
    /// A read that ran out of time, was stopped, or ended without an answer:
    /// the table is not what failed, so the next ask reads it again.
    passing: bool,
}

/// The answer a worker sends: the table's columns, or why there are none.
type Answer = Result<Vec<ColumnFacts>, Refused>;

/// The most profiles kept at once. A profile is a table's worth of column
/// facts, and the inspector is asked about the tables a reader visits, so a
/// handful covers going back and forth between them.
const PROFILES_KEPT: usize = 8;

/// A read under way.
struct Reading {
    key: ProfileKey,
    answer: Receiver<Answer>,
    stop: PreviewStop,
    /// Whether the inspector started it, so the inspector may stop it once
    /// the canvas no longer holds the table. A read a caller asked for by name
    /// is left to finish.
    for_inspector: bool,
}

/// The profiles read so far, and the one being read.
#[derive(Default)]
pub struct ColumnProfiles {
    /// Each profile read, or the refusal that will not change by asking again.
    kept: Vec<(ProfileKey, ProfileRead)>,
    /// A refusal that may: shown until the table is next asked about afresh
    /// (see [`is_fresh_ask`]), then read again.
    passing: Option<(ProfileKey, String)>,
    /// The table the inspector has been describing since the canvas came to
    /// it. The inspector asks on each frame it draws, and only the first of
    /// those asks is a fresh one.
    viewing: Option<ProfileKey>,
    reading: Option<Reading>,
    /// Each worker started and not yet seen to finish, the stopped ones
    /// among them, so dropping the profiles can wait for them.
    workers: Vec<std::thread::JoinHandle<()>>,
    started: usize,
}

/// How long dropping the profiles waits for its stopped workers to finish
/// before leaving them. A stopped read ends at its next check of the stop,
/// so this is a bound for a worker held at the preview gate, not the wait
/// a read normally takes.
const WORKERS_AWAITED: std::time::Duration = std::time::Duration::from_secs(10);

impl ColumnProfiles {
    /// What a profile of `table` of `source` is kept against.
    pub(crate) fn key(table: &str, content_hash: Option<&str>, source: &TypedSource) -> ProfileKey {
        ProfileKey {
            table: table.to_string(),
            content_hash: content_hash.map(str::to_owned),
            chain: source.read_over(source.chain_reading(&[table.to_string()])),
        }
    }

    /// Where the profile under `key` stands, without starting a read: what is
    /// kept for it, that a read of it is under way, or the refusal held for
    /// it that may pass; `None` when there is no such answer.
    pub(crate) fn peek(&mut self, key: &ProfileKey) -> Option<ProfileRead> {
        self.poll();
        if let Some((_, read)) = self.kept.iter().find(|(k, _)| k == key) {
            return Some(read.clone());
        }
        if self.reading.as_ref().is_some_and(|r| r.key == *key) {
            return Some(ProfileRead::Reading);
        }
        self.passing
            .as_ref()
            .filter(|(k, _)| k == key)
            .map(|(_, sentence)| ProfileRead::Failed(sentence.clone()))
    }

    /// Where the profile under `key` stands, starting a read of it on a worker
    /// unless a profile is kept for it or a read of it is under way.
    ///
    /// A refusal that may pass is shown, and read again only on a fresh ask
    /// ([`is_fresh_ask`], whose cases `only_a_new_ask_reads_a_passing_refusal_again`
    /// holds), so a table whose profile keeps running out of time
    /// says so rather than being read again on each frame. A read under way
    /// for another table is stopped: one table is asked about at a time, and
    /// the one asked about now is the one wanted.
    pub(crate) fn read(
        &mut self,
        key: ProfileKey,
        source: &TypedSource,
        hooks: &crate::data_grid::PreviewHooks,
        for_inspector: bool,
    ) -> ProfileRead {
        let known = self.peek(&key);
        let passing_here = self.passing.as_ref().is_some_and(|(k, _)| *k == key);
        let fresh = is_fresh_ask(self.viewing.as_ref(), &key, for_inspector);
        if for_inspector {
            self.viewing = Some(key.clone());
        }
        if let Some(read) = known.filter(|_| !(passing_here && fresh)) {
            return read;
        }
        self.passing = None;
        self.start(key, source, hooks.clone(), for_inspector);
        ProfileRead::Reading
    }

    /// Stop the read under way when the inspector started it: the canvas has
    /// moved off the table it was reading, and a preview of the table now in
    /// view should not wait behind it.
    pub(crate) fn stop_the_inspectors(&mut self) {
        self.viewing = None;
        if self.reading.as_ref().is_some_and(|r| r.for_inspector) {
            if let Some(reading) = self.reading.take() {
                reading.stop.stop();
            }
        }
    }

    /// The kept columns `read` holds, borrowed from the cache.
    #[must_use]
    pub fn kept(&self, read: &Rc<[ColumnFacts]>) -> Option<&[ColumnFacts]> {
        self.kept.iter().find_map(|(_, kept)| match kept {
            ProfileRead::Read(columns) if Rc::ptr_eq(columns, read) => Some(&**columns),
            _ => None,
        })
    }

    /// How many reads have been started, which a test reads to see a second
    /// ask served from the profile already read.
    #[must_use]
    pub const fn started(&self) -> usize {
        self.started
    }

    /// Whether a read is under way.
    #[must_use]
    pub const fn is_reading(&self) -> bool {
        self.reading.is_some()
    }

    /// Keep the answer of the read under way, once it has one.
    fn poll(&mut self) {
        let Some(reading) = &self.reading else {
            return;
        };
        let answer = match reading.answer.try_recv() {
            Ok(answer) => answer,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(Refused {
                sentence: "The read of the columns stopped without an answer.".into(),
                passing: true,
            }),
        };
        let key = self.reading.take().expect("a read is under way").key;
        let read = match answer {
            Ok(columns) => ProfileRead::Read(columns.into()),
            Err(Refused {
                sentence,
                passing: true,
            }) => {
                self.passing = Some((key, sentence));
                return;
            }
            Err(Refused { sentence, .. }) => ProfileRead::Failed(sentence),
        };
        if self.kept.len() >= PROFILES_KEPT {
            self.kept.remove(0);
        }
        self.kept.push((key, read));
    }

    fn start(
        &mut self,
        key: ProfileKey,
        source: &TypedSource,
        hooks: crate::data_grid::PreviewHooks,
        for_inspector: bool,
    ) {
        if let Some(reading) = self.reading.take() {
            reading.stop.stop();
        }
        let (tell, answer) = channel();
        let stop = PreviewStop::default();
        let (root, chain) = key.chain.clone();
        let table = key.table.clone();
        let data = source.data.clone();
        let reader = source.reader;
        let hidden = source.raw_columns();
        let stopped = stop.clone();
        self.workers.retain(|worker| !worker.is_finished());
        let worker = std::thread::spawn(move || {
            let answer: Answer = hooks.read_inside(|| {
                brightfield_engine::preview_profile_stoppable(
                    &data,
                    reader,
                    &root,
                    &chain,
                    &table,
                    brightfield_engine::PREVIEW_FILE_LIMIT_BYTES,
                    brightfield_engine::PREVIEW_DEADLINE,
                    &stopped,
                )
                .map(|(_, columns)| {
                    columns
                        .iter()
                        .filter(|column| !hidden.contains(&column.name))
                        .map(crate::one_step::facts_of_column)
                        .collect()
                })
                .map_err(|refusal| Refused {
                    passing: matches!(
                        refusal,
                        brightfield_engine::PreviewRefusal::TookTooLong { .. }
                            | brightfield_engine::PreviewRefusal::Superseded
                    ),
                    sentence: format!("The columns could not be read: {refusal}."),
                })
            });
            let _ = tell.send(answer);
        });
        self.workers.push(worker);
        self.reading = Some(Reading {
            key,
            answer,
            stop,
            for_inspector,
        });
        self.started += 1;
    }
}

/// Whether an ask for `key` is a fresh one: a caller's ask by name always is,
/// and the inspector's is when the table it describes has changed since it
/// last asked, which includes the canvas coming back to the table after
/// leaving it (`only_a_new_ask_reads_a_passing_refusal_again`).
fn is_fresh_ask(viewing: Option<&ProfileKey>, key: &ProfileKey, for_inspector: bool) -> bool {
    !for_inspector || viewing != Some(key)
}

/// Dropping the profiles stops the read under way and waits for every
/// worker to finish: a worker still inside DuckDB when the process exits
/// crashes it on the way out.
impl Drop for ColumnProfiles {
    fn drop(&mut self) {
        if let Some(reading) = self.reading.take() {
            reading.stop.stop();
        }
        let until = std::time::Instant::now() + WORKERS_AWAITED;
        for worker in self.workers.drain(..) {
            while !worker.is_finished() && std::time::Instant::now() < until {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(table: &str) -> ProfileKey {
        ProfileKey {
            table: table.to_string(),
            content_hash: None,
            chain: (table.to_string(), Vec::new()),
        }
    }

    /// The inspector's ask on each frame after the first is not fresh, so a
    /// refusal that may pass is shown rather than read again at once; a new
    /// table, a return to the table, and a caller's ask by name are fresh.
    #[test]
    fn only_a_new_ask_reads_a_passing_refusal_again() {
        let old = key("old");
        assert!(is_fresh_ask(None, &old, true), "the canvas came to old");
        assert!(
            !is_fresh_ask(Some(&old), &old, true),
            "the inspector asking again, frame after frame"
        );
        assert!(is_fresh_ask(Some(&key("top")), &old, true), "another table");
        assert!(
            is_fresh_ask(Some(&old), &old, false),
            "a caller asking by name"
        );
    }

    /// A refusal that may pass, held for the table in view, is answered as it
    /// is while the inspector keeps asking: no read is started for it.
    #[test]
    fn a_passing_refusal_is_shown_while_the_inspector_keeps_asking() {
        let old = key("old");
        let mut profiles = ColumnProfiles::default();
        profiles.passing = Some((old.clone(), "took too long".to_string()));
        profiles.viewing = Some(old.clone());
        assert_eq!(
            profiles.peek(&old),
            Some(ProfileRead::Failed("took too long".to_string()))
        );
        let passing_here = profiles.passing.as_ref().is_some_and(|(k, _)| *k == old);
        assert!(passing_here && !is_fresh_ask(profiles.viewing.as_ref(), &old, true));
        profiles.stop_the_inspectors();
        assert!(
            is_fresh_ask(profiles.viewing.as_ref(), &old, true),
            "leaving the table makes the next ask a fresh one"
        );
    }
}
