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
//! worker reads.

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

/// The answer a worker sends: the table's columns, or why there are none.
type Answer = Result<Vec<ColumnFacts>, String>;

/// The most profiles kept at once. A profile is a table's worth of column
/// facts, and the inspector is asked about the tables a reader visits, so a
/// handful covers going back and forth between them.
const PROFILES_KEPT: usize = 8;

/// The profiles read so far, and the one being read.
#[derive(Default)]
pub struct ColumnProfiles {
    kept: Vec<(ProfileKey, ProfileRead)>,
    reading: Option<(ProfileKey, Receiver<Answer>, PreviewStop)>,
    started: usize,
}

impl ColumnProfiles {
    /// Where the profile of `table` of `source` stands, starting a read of it
    /// on a worker when none is kept or under way. A read under way for
    /// another table is stopped: the inspector describes one table at a time,
    /// and the one asked about now is the one it is describing.
    pub(crate) fn read(
        &mut self,
        table: &str,
        content_hash: Option<&str>,
        source: &TypedSource,
        hooks: &crate::data_grid::PreviewHooks,
    ) -> ProfileRead {
        let chain = source.read_over(source.chain_reading(&[table.to_string()]));
        let key = ProfileKey {
            table: table.to_string(),
            content_hash: content_hash.map(str::to_owned),
            chain,
        };
        self.poll();
        if let Some((_, read)) = self.kept.iter().find(|(k, _)| *k == key) {
            return read.clone();
        }
        if self.reading.as_ref().is_some_and(|(k, _, _)| *k == key) {
            return ProfileRead::Reading;
        }
        self.start(key, source, hooks.clone());
        ProfileRead::Reading
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

    /// Keep the answer of the read under way, once it has one.
    fn poll(&mut self) {
        let Some((_, answer, _)) = &self.reading else {
            return;
        };
        let read = match answer.try_recv() {
            Ok(Ok(columns)) => ProfileRead::Read(columns.into()),
            Ok(Err(reason)) => ProfileRead::Failed(reason),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                ProfileRead::Failed("The read of the columns stopped without an answer.".into())
            }
        };
        let (key, _, _) = self.reading.take().expect("a read is under way");
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
    ) {
        if let Some((_, _, stop)) = self.reading.take() {
            stop.stop();
        }
        let (tell, answer) = channel();
        let stop = PreviewStop::default();
        let (root, chain) = key.chain.clone();
        let table = key.table.clone();
        let data = source.data.clone();
        let reader = source.reader;
        let hidden = source.raw_columns();
        let stopped = stop.clone();
        std::thread::spawn(move || {
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
                .map_err(|refusal| format!("The columns could not be read: {refusal}."))
            });
            let _ = tell.send(answer);
        });
        self.reading = Some((key, answer, stop));
        self.started += 1;
    }
}

impl Drop for ColumnProfiles {
    fn drop(&mut self) {
        if let Some((_, _, stop)) = self.reading.take() {
            stop.stop();
        }
    }
}
