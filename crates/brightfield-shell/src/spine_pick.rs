//! The spine picking a table for an operation.
//!
//! An operation that reads a second table, as a join does, or a list of them,
//! as an append does, asks for it in the spine: the place the Protocol's
//! tables are already listed. A [`SpinePick`] is that ask while it is open:
//! the tables the operation can read ([`Candidate`]), a cursor on one, and the
//! tables added so far when the operation reads a list.
//!
//! **The candidates** are every table the spine lists other than the one in
//! front. In a Protocol made from a data file those are the steps of its chain
//! ([`CandidateSource::Step`]). While the grid holds a table of a Protocol made
//! from a folder, workbook or database, they are the model's other tables, each
//! read from its grid copy ([`CandidateSource::ModelTable`]).
//!
//! **What a candidate says.** A table the Protocol's model relates to the one
//! in front trails `related`. Any other trails `shares <column>` when it holds
//! the column the grid's cursor is on, once its columns are known. A table's
//! columns are read on a worker (`read_columns`), so the spine draws without
//! waiting on DuckDB.
//!
//! **Where the cursor starts**: on the first related table, else on the first
//! that shares the cursor's column, else on the first. Until a key moves it,
//! it follows what the columns say as they come, so it does not settle on the
//! first table only because that table's columns were read first.
//!
//! **A table that shares no column.** An operation that matches columns by
//! name, as an append does, opens its pick with the table in front's columns
//! ([`SpinePick::matching_columns`]). A candidate whose columns are known and
//! share no name with them is dimmed and trails `shares no column`; the
//! cursor does not start on it, and it can be neither added nor taken.
//!
//! The candidates stay in the spine's order, so `j` and `k` move the cursor
//! down and up the rail as it is drawn.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

use brightfield_engine::PreviewStop;

use crate::typed_sql::TypedSource;

/// One table the operation can read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The table's name.
    pub table: String,
    /// Its columns, once they have been read.
    pub columns: Option<Vec<String>>,
    /// The cursor's column, when this table holds it too.
    pub shares: Option<String>,
    /// Whether the Protocol's model relates it to the table in front.
    pub related: bool,
    /// Whether its columns are known and share no name with the table in
    /// front's, for an operation that matches columns by name.
    pub disjoint: bool,
    /// Where its rows are read from.
    pub source: CandidateSource,
}

/// Where a candidate's rows are read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CandidateSource {
    /// A step of the chain after the data file's load, or the loaded table.
    Step,
    /// A table of the Protocol's model, read from the copy of its rows the
    /// grid opens.
    ModelTable {
        /// The Parquet file the copy is written to, once the table is taken.
        grid_copy: PathBuf,
    },
}

impl Candidate {
    /// What the table's row trails in the spine: `related`, or `shares
    /// <column>`, and nothing for a table that is neither.
    #[must_use]
    pub fn trail(&self) -> Option<String> {
        if self.related {
            return Some("related".to_string());
        }
        if self.disjoint {
            return Some(SHARES_NO_COLUMN.to_string());
        }
        self.shares
            .as_ref()
            .map(|column| format!("shares {column}"))
    }
}

/// What a candidate trails when it shares no column's name with the table in
/// front, for an operation that matches columns by name.
pub const SHARES_NO_COLUMN: &str = "shares no column";

/// A table the spine lists, before it is known whether an operation can read
/// it: its name, its columns where they are already known, and where its rows
/// are read from.
pub type Listed = (String, Option<Vec<String>>, CandidateSource);

/// **The candidates of `tables`**: each but `in_front`, in the order given,
/// with its columns where they are already known. A table is `related` when
/// `relationships`, the pairs of tables the Protocol's model joins, holds one
/// of it and the table in front, in either direction. Names are matched
/// without regard to case, as DuckDB resolves them.
#[must_use]
pub fn candidates(
    tables: &[Listed],
    in_front: &str,
    relationships: &[(String, String)],
) -> Vec<Candidate> {
    let front = in_front.to_lowercase();
    let related = |table: &str| {
        let table = table.to_lowercase();
        relationships.iter().any(|(from, to)| {
            let (from, to) = (from.to_lowercase(), to.to_lowercase());
            (from == table && to == front) || (from == front && to == table)
        })
    };
    tables
        .iter()
        .filter(|(table, _, _)| table.to_lowercase() != front)
        .map(|(table, columns, source)| Candidate {
            table: table.clone(),
            columns: columns.clone(),
            shares: None,
            related: related(table),
            disjoint: false,
            source: source.clone(),
        })
        .collect()
}

/// The spine's ask for the table, or the tables, an operation reads.
#[derive(Clone, Debug)]
pub struct SpinePick {
    role: &'static str,
    list: bool,
    in_front: String,
    cursor_column: Option<String>,
    /// The table in front's columns, lowercased, when the operation matches
    /// columns by name.
    matching: Option<Vec<String>>,
    candidates: Vec<Candidate>,
    cursor: usize,
    /// Whether a key has moved the cursor, after which it stays where it was put.
    moved: bool,
    added: Vec<String>,
}

impl SpinePick {
    /// An ask for the table read under arc's `role`, or for a list of them
    /// when `list`, over `candidates`, with `in_front` the table the operation
    /// is on and `cursor_column` the column the grid's cursor is on.
    #[must_use]
    pub fn new(
        role: &'static str,
        list: bool,
        in_front: &str,
        cursor_column: Option<&str>,
        candidates: Vec<Candidate>,
    ) -> Self {
        let mut pick = Self {
            role,
            list,
            in_front: in_front.to_string(),
            cursor_column: cursor_column.map(str::to_string),
            matching: None,
            candidates,
            cursor: 0,
            moved: false,
            added: Vec::new(),
        };
        for at in 0..pick.candidates.len() {
            pick.candidates[at].shares = pick.shared_by(&pick.candidates[at]);
        }
        pick.cursor = pick.best();
        pick
    }

    /// The same ask, for an operation that matches columns by name with the
    /// table in front's `columns`: a candidate sharing none of them is dimmed.
    #[must_use]
    pub fn matching_columns(mut self, columns: &[String]) -> Self {
        self.matching = Some(columns.iter().map(|c| c.to_lowercase()).collect());
        for at in 0..self.candidates.len() {
            self.candidates[at].disjoint = self.shares_nothing(&self.candidates[at]);
        }
        if !self.moved {
            self.cursor = self.best();
        }
        self
    }

    /// Whether `candidate`'s columns are known and share no name with the
    /// table in front's, when the operation matches columns by name.
    fn shares_nothing(&self, candidate: &Candidate) -> bool {
        match (&self.matching, &candidate.columns) {
            (Some(front), Some(theirs)) => !theirs
                .iter()
                .any(|column| front.contains(&column.to_lowercase())),
            _ => false,
        }
    }

    /// Whether every candidate's columns are known and none shares a name
    /// with the table in front's: the operation has nothing it can read.
    #[must_use]
    pub fn none_shares_a_column(&self) -> bool {
        self.matching.is_some() && self.candidates.iter().all(|c| c.disjoint)
    }

    /// The cursor's column as `candidate` spells it, when it holds it.
    fn shared_by(&self, candidate: &Candidate) -> Option<String> {
        let wanted = self.cursor_column.as_ref()?.to_lowercase();
        candidate
            .columns
            .as_ref()?
            .iter()
            .find(|column| column.to_lowercase() == wanted)
            .cloned()
    }

    /// Where the cursor starts: the first related candidate, else the first
    /// that shares the cursor's column, else the first that is not dimmed,
    /// else the first.
    fn best(&self) -> usize {
        self.candidates
            .iter()
            .position(|c| c.related && !c.disjoint)
            .or_else(|| {
                self.candidates
                    .iter()
                    .position(|c| c.shares.is_some() && !c.disjoint)
            })
            .or_else(|| self.candidates.iter().position(|c| !c.disjoint))
            .unwrap_or(0)
    }

    /// The role of arc's the picked table is read under.
    #[must_use]
    pub fn role(&self) -> &'static str {
        self.role
    }

    /// Whether the operation reads a list of tables, and not one.
    #[must_use]
    pub fn list(&self) -> bool {
        self.list
    }

    /// The table the operation is on.
    #[must_use]
    pub fn in_front(&self) -> &str {
        &self.in_front
    }

    /// The tables the operation can read, in the spine's order.
    #[must_use]
    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    /// The index of the candidate under the cursor.
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The candidate under the cursor, when there is any candidate.
    #[must_use]
    pub fn under_cursor(&self) -> Option<&Candidate> {
        self.candidates.get(self.cursor)
    }

    /// The tables added to the list, in the order they were added.
    #[must_use]
    pub fn added(&self) -> &[String] {
        &self.added
    }

    /// Move the cursor to the next candidate; it stops at the last.
    pub fn next(&mut self) {
        self.moved = true;
        if self.cursor + 1 < self.candidates.len() {
            self.cursor += 1;
        }
    }

    /// Move the cursor to the candidate before; it stops at the first.
    pub fn previous(&mut self) {
        self.moved = true;
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Put the cursor on `table`, when it is a candidate. Whether it was.
    pub fn put_cursor_on(&mut self, table: &str) -> bool {
        let Some(at) = self.candidates.iter().position(|c| c.table == table) else {
            return false;
        };
        self.moved = true;
        self.cursor = at;
        true
    }

    /// Add the table under the cursor to the list, or take it off when it is
    /// already there. Whether the list changed: an ask for one table adds
    /// none, and a dimmed table is not added.
    pub fn add(&mut self) -> bool {
        if !self.list {
            return false;
        }
        let Some(table) = self
            .under_cursor()
            .filter(|c| !c.disjoint)
            .map(|c| c.table.clone())
        else {
            return false;
        };
        self.moved = true;
        match self.added.iter().position(|t| *t == table) {
            Some(at) => {
                self.added.remove(at);
            }
            None => self.added.push(table),
        }
        true
    }

    /// The tables a take would hand over, without taking them: the added
    /// tables in order, or the one under the cursor when none was added.
    #[must_use]
    pub fn would_take(&self) -> Vec<String> {
        if self.added.is_empty() {
            self.under_cursor()
                .map(|c| vec![c.table.clone()])
                .unwrap_or_default()
        } else {
            self.added.clone()
        }
    }

    /// Take the tables: the added tables in order, or the one under the
    /// cursor. The list is left empty.
    pub fn take(&mut self) -> Vec<String> {
        let taken = self.would_take();
        self.added.clear();
        taken
    }

    /// Note the columns of `table`, read since the ask opened. Until a key has
    /// moved the cursor, it moves to the best candidate as the columns say.
    pub fn set_columns(&mut self, table: &str, columns: Vec<String>) {
        let Some(at) = self.candidates.iter().position(|c| c.table == table) else {
            return;
        };
        self.candidates[at].columns = Some(columns);
        self.candidates[at].shares = self.shared_by(&self.candidates[at]);
        self.candidates[at].disjoint = self.shares_nothing(&self.candidates[at]);
        if !self.moved {
            self.cursor = self.best();
        }
    }

    /// The columns of `table`, once they are known.
    #[must_use]
    pub fn columns_of(&self, table: &str) -> Option<&[String]> {
        self.candidates
            .iter()
            .find(|c| c.table == table)
            .and_then(|c| c.columns.as_deref())
    }

    /// The candidate named `table`.
    #[must_use]
    pub fn candidate(&self, table: &str) -> Option<&Candidate> {
        self.candidates.iter().find(|c| c.table == table)
    }
}

/// The columns of candidates, as a worker reads them (`read_columns`).
pub struct ColumnReads {
    told: Receiver<(String, Vec<String>)>,
    over: PreviewStop,
}

impl ColumnReads {
    /// The columns read since the last poll, each with its table.
    pub fn poll(&mut self) -> Vec<(String, Vec<String>)> {
        self.told.try_iter().collect()
    }
}

impl Drop for ColumnReads {
    fn drop(&mut self) {
        // The pick is over: the read under way is of no more use.
        self.over.stop();
    }
}

/// **Read the columns of each of `candidates` that lacks them, on a worker**,
/// one table at a time in the order given. `wake` is called as each answer is
/// ready.
///
/// A step's columns are read from a description of its table, over `source`
/// as a statement over it would be: the steps it reads are made, and no
/// other. A model table's columns are those its grid copy would hold, read
/// from the database of `copies` with no row copied: the copy is written once
/// the table is taken. A table that cannot be read answers with
/// no columns, so the ask is never left waiting on it: arc and DuckDB say what
/// is wrong with it once it is taken.
#[must_use]
pub(crate) fn read_columns(
    source: Option<TypedSource>,
    copies: Option<crate::own_files::OwnProtocol>,
    candidates: &[Candidate],
    hooks: crate::data_grid::PreviewHooks,
    wake: impl Fn() + Send + Sync + 'static,
) -> ColumnReads {
    let (tell, told) = channel();
    let over = PreviewStop::default();
    let stop = over.clone();
    let asked: Vec<(String, CandidateSource)> = candidates
        .iter()
        .filter(|c| c.columns.is_none())
        .map(|c| (c.table.clone(), c.source.clone()))
        .collect();
    std::thread::spawn(move || {
        for (table, from) in asked {
            if stop.is_stopped() {
                return;
            }
            let columns = hooks.read_inside(|| match &from {
                CandidateSource::Step => source
                    .as_ref()
                    .map(|source| step_columns(source, &table, &stop))
                    .unwrap_or_default(),
                CandidateSource::ModelTable { .. } => copies
                    .as_ref()
                    .and_then(|p| crate::own_files::grid_columns(p, &table).ok())
                    .unwrap_or_default(),
            });
            if tell.send((table, columns)).is_err() {
                return;
            }
            wake();
        }
    });
    ColumnReads { told, over }
}

/// The columns of the step `table` of `source`, without the columns the casts
/// keep each cast column's text under, which the grid hides.
///
/// They are read as rows: the last step of the chain describes the table, one
/// row a column, so a step that makes no rows still answers with its columns.
fn step_columns(source: &TypedSource, table: &str, stop: &PreviewStop) -> Vec<String> {
    let (root, mut chain) = source.read_over(source.chain_reading(&[table.to_string()]));
    chain.push((
        format!("{table}__columns"),
        brightfield_engine::PreviewOp::Statement(format!(
            "SELECT column_name FROM (DESCRIBE SELECT * FROM \"{}\")",
            table.replace('"', "\"\"")
        )),
    ));
    let hidden = source.raw_columns();
    let Ok(batches) = brightfield_engine::preview_first_page_stoppable(
        &source.data,
        source.reader,
        &root,
        &chain,
        MOST_COLUMNS,
        brightfield_engine::PREVIEW_FILE_LIMIT_BYTES,
        brightfield_engine::PREVIEW_DEADLINE,
        stop,
    ) else {
        return Vec::new();
    };
    let mut columns = Vec::new();
    for batch in &batches {
        let names = batch.column(0);
        for row in 0..batch.num_rows() {
            if let Ok(name) = arrow::util::display::array_value_to_string(names, row) {
                if !hidden.contains(&name) {
                    columns.push(name);
                }
            }
        }
    }
    columns
}

/// The most columns of a step the pick reads: more than a table holds.
const MOST_COLUMNS: u64 = 10_000;

#[cfg(test)]
mod tests {
    use super::*;

    fn table(
        name: &str,
        columns: Option<&[&str]>,
    ) -> (String, Option<Vec<String>>, CandidateSource) {
        (
            name.to_string(),
            columns.map(|c| c.iter().map(|s| (*s).to_string()).collect()),
            CandidateSource::Step,
        )
    }

    fn names(pick: &SpinePick) -> Vec<&str> {
        pick.candidates().iter().map(|c| c.table.as_str()).collect()
    }

    #[test]
    fn the_candidates_are_every_table_but_the_one_in_front_in_the_order_given() {
        let tables = [
            table("housing", Some(&["house_age"])),
            table("by_age", None),
            table("top", None),
        ];
        let pick = SpinePick::new(
            "other",
            false,
            "Housing",
            None,
            candidates(&tables, "Housing", &[]),
        );
        assert_eq!(names(&pick), ["by_age", "top"]);
        assert_eq!(pick.cursor(), 0);
        assert_eq!(pick.in_front(), "Housing");
    }

    #[test]
    fn the_cursor_starts_on_a_related_table_then_one_sharing_the_column_then_the_first() {
        let tables = [
            table("housing", Some(&["house_age"])),
            table("a", Some(&["rooms"])),
            table("b", Some(&["House_Age"])),
            table("c", Some(&["rooms"])),
        ];
        let of = |relationships: &[(String, String)], column: Option<&str>| {
            SpinePick::new(
                "other",
                false,
                "housing",
                column,
                candidates(&tables, "housing", relationships),
            )
        };
        // Nothing related and no column: the first.
        assert_eq!(of(&[], None).cursor(), 0);
        // The cursor's column, matched without regard to case and trailed as
        // the table spells it.
        let shares = of(&[], Some("house_age"));
        assert_eq!(shares.cursor(), 1);
        assert_eq!(
            shares.candidates()[1].trail().as_deref(),
            Some("shares House_Age")
        );
        assert_eq!(shares.candidates()[0].trail(), None);
        // A relationship in either direction leads, and says so in place of
        // the column it shares.
        let related = of(
            &[("c".to_string(), "HOUSING".to_string())],
            Some("house_age"),
        );
        assert_eq!(related.cursor(), 2);
        assert_eq!(related.candidates()[2].trail().as_deref(), Some("related"));
        assert_eq!(
            of(&[("housing".to_string(), "a".to_string())], None).cursor(),
            0
        );
    }

    #[test]
    fn columns_read_later_move_a_cursor_no_key_has_moved_and_leave_one_that_has() {
        let tables = [
            table("housing", Some(&["house_age"])),
            table("a", None),
            table("b", None),
        ];
        let fresh = || {
            SpinePick::new(
                "other",
                false,
                "housing",
                Some("house_age"),
                candidates(&tables, "housing", &[]),
            )
        };
        let mut pick = fresh();
        assert_eq!(pick.cursor(), 0);
        pick.set_columns("a", vec!["rooms".to_string()]);
        pick.set_columns("b", vec!["house_age".to_string()]);
        assert_eq!(pick.cursor(), 1, "the cursor follows the columns");
        assert_eq!(pick.columns_of("b"), Some(&["house_age".to_string()][..]));
        let mut moved = fresh();
        moved.next();
        moved.previous();
        moved.set_columns("b", vec!["house_age".to_string()]);
        assert_eq!(moved.cursor(), 0, "a cursor a key moved stays");
        // A table that is no candidate is ignored.
        moved.set_columns("nowhere", Vec::new());
        assert_eq!(names(&moved), ["a", "b"]);
    }

    #[test]
    fn the_cursor_stops_at_each_end() {
        let tables = [table("housing", None), table("a", None), table("b", None)];
        let mut pick = SpinePick::new(
            "other",
            false,
            "housing",
            None,
            candidates(&tables, "housing", &[]),
        );
        pick.previous();
        assert_eq!(pick.cursor(), 0);
        pick.next();
        pick.next();
        pick.next();
        assert_eq!(pick.cursor(), 1);
        assert!(pick.put_cursor_on("a"));
        assert_eq!(pick.cursor(), 0);
        assert!(!pick.put_cursor_on("housing"));
    }

    #[test]
    fn one_table_is_taken_from_under_the_cursor_and_a_list_in_the_order_added() {
        let tables = [
            table("housing", None),
            table("a", None),
            table("b", None),
            table("c", None),
        ];
        let of = |list| {
            SpinePick::new(
                "other",
                list,
                "housing",
                None,
                candidates(&tables, "housing", &[]),
            )
        };
        // An ask for one table adds none.
        let mut one = of(false);
        one.next();
        assert!(!one.add());
        assert!(one.added().is_empty());
        assert_eq!(one.take(), ["b"]);
        // A list is taken in the order added, and Space on an added table
        // takes it off.
        let mut list = of(true);
        list.next();
        list.next();
        assert!(list.add());
        list.previous();
        list.previous();
        assert!(list.add());
        assert_eq!(list.added(), ["c", "a"]);
        assert!(list.add());
        assert_eq!(list.added(), ["c"]);
        assert!(list.add());
        assert_eq!(list.would_take(), ["c", "a"]);
        assert_eq!(list.take(), ["c", "a"]);
        assert!(list.added().is_empty());
        // With none added, a list takes the one under the cursor.
        assert_eq!(of(true).take(), ["a"]);
        // No candidate, nothing to take.
        let empty = SpinePick::new("other", true, "housing", None, Vec::new());
        assert!(empty.would_take().is_empty());
    }

    #[test]
    fn a_table_sharing_no_column_is_dimmed_and_neither_added_nor_started_on() {
        let tables = [
            table("housing", Some(&["house_age", "rooms"])),
            table("counted", Some(&["n"])),
            table("top", None),
        ];
        let front = ["House_Age".to_string(), "rooms".to_string()];
        let mut pick = SpinePick::new(
            "other",
            true,
            "housing",
            None,
            candidates(&tables, "housing", &[]),
        )
        .matching_columns(&front);
        assert!(pick.candidates()[0].disjoint);
        assert_eq!(
            pick.candidates()[0].trail().as_deref(),
            Some(SHARES_NO_COLUMN)
        );
        // A table whose columns are not known yet is not dimmed.
        assert!(!pick.candidates()[1].disjoint);
        assert_eq!(pick.cursor(), 1, "the cursor starts past the dimmed table");
        assert!(!pick.none_shares_a_column());
        pick.previous();
        assert!(!pick.add(), "a dimmed table is not added");
        assert!(pick.added().is_empty());
        // Columns read later dim a table too, matched without regard to case.
        pick.set_columns("top", vec!["ROOMS".to_string()]);
        assert!(!pick.candidates()[1].disjoint);
        pick.set_columns("top", vec!["x".to_string()]);
        assert!(pick.none_shares_a_column());
        // An operation that does not match by name dims nothing.
        let plain = SpinePick::new(
            "other",
            true,
            "housing",
            None,
            candidates(&tables, "housing", &[]),
        );
        assert!(!plain.candidates()[0].disjoint);
        assert!(!plain.none_shares_a_column());
    }
}
