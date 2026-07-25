//! Per-statement SQL asset extraction, and the per-CTE lineage inside a
//! statement.
//!
//! `brightfield_sql::conform::parse_and_normalise` parses multi-statement SQL
//! but fails the WHOLE string on one bad statement — so the splitter here
//! finds statement boundaries at the TOKEN level (semicolons outside
//! strings/comments) and each statement is parsed individually, degrading
//! **per-statement, never per-file**. Every fragment keeps its
//! byte range in the source file so later cards can highlight it.
//!
//! ## CTEs
//!
//! The same failure mode repeats one level down: one malformed CTE body fails
//! the whole statement, and a statement drawn as a single box hides the joins
//! that are its actual lineage. So a statement's top-level `WITH` clause is
//! also located at the TOKEN level ([`CteAssets`]), each body is parsed on its
//! own, and the statement minus its `WITH` clause is parsed to recover the main
//! body's reads and the relation it produces. Three consequences:
//!
//! - a CTE whose body fails to parse degrades **alone** — its siblings, and the
//!   statement's own target relation, survive. The recovered statement is
//!   marked `degraded` so the relation it produces draws BADGED: the reads of a
//!   body that did not parse are unknowable, and a partial lineage drawn as a
//!   clean one is a lie. A statement that recovers no relation is not promoted
//!   at all — a graph that draws only produced nodes would drop it entirely,
//!   and a silent skip is exactly what this module refuses;
//! - reads are resolved against a **scope stack**, so a name declared in more
//!   than one scope resolves to its INNERMOST declaration and a nested
//!   declaration never masquerades as lineage;
//! - a read that resolves to a top-level CTE is intra-statement
//!   ([`ScopedReads::ctes`]); one that resolves to nothing in scope is a real
//!   relation ([`ScopedReads::relations`]).
//!
//! `produced` / `consumed_relations` / `consumed_files` are still derived from
//! the WHOLE-statement parse whenever it succeeds, so a statement that parses
//! is untouched by any of this; the CTE fields are additive. One statement-level
//! behaviour DOES change, and deliberately: a statement whose whole-statement
//! parse fails but whose stripped form recovers a relation used to be a single
//! opaque chip and is now that relation, badged with the parse error. The signal
//! moves; it never disappears.

use std::collections::BTreeSet;
use std::ops::ControlFlow;

use sqlparser::ast::{
    Expr, FunctionArg, FunctionArgExpr, ObjectName, Query, Statement, TableFactor,
    TableFunctionArgs, Value, Visit, Visitor,
};
use sqlparser::dialect::DuckDbDialect;
use sqlparser::keywords::Keyword;
use sqlparser::tokenizer::{Token, Tokenizer};

/// One statement's slice of a model file, with its byte range in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment {
    /// The statement text (trimmed; no trailing semicolon).
    pub text: String,
    /// Byte offset of the fragment's first byte in the source file.
    pub start: usize,
    /// Byte offset one past the fragment's last byte.
    pub end: usize,
}

/// The assets one statement produces/consumes — or its opaque degrade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatementAssets {
    /// The statement parsed: relation produced (CREATE TABLE/VIEW ... AS) and
    /// the relations/file paths it reads.
    Parsed {
        /// Statement index within the file (0-based, post-split).
        index: usize,
        /// Relation produced by `CREATE [OR REPLACE] TABLE|VIEW <name>`.
        produced: Option<String>,
        /// Relations read (table factors), CTE-local names excluded.
        consumed_relations: BTreeSet<String>,
        /// File paths read via `read_parquet`/`read_csv[_auto]`/`read_xlsx`/
        /// `read_json[_auto]` (first string-literal argument; may be a glob).
        consumed_files: BTreeSet<String>,
        /// The statement's top-level CTEs, in declaration order. Empty when the
        /// statement has no `WITH` clause (or the clause could not be located).
        ctes: Vec<CteAssets>,
        /// What the statement's MAIN body reads — the query left once the
        /// `WITH` clause is stripped. Its `ctes` name the CTEs that feed the
        /// produced relation; its relations/files are read directly.
        main_reads: ScopedReads,
        /// `Some(error)` when the WHOLE-statement parse FAILED and the
        /// statement was recovered from its `WITH`-stripped form. Everything
        /// here is real, but nothing here is complete: whatever a malformed
        /// body read is unknowable, so the consumer must badge the produced
        /// node with this rather than draw it as a healthy relation.
        degraded: Option<String>,
        /// Byte range in the source file.
        range: (usize, usize),
    },
    /// The statement failed to parse — an opaque chip, never a silent skip.
    Opaque {
        /// Statement index within the file (0-based, post-split).
        index: usize,
        /// The parse error, for the chip's issue badge.
        error: String,
        /// Byte range in the source file.
        range: (usize, usize),
    },
}

/// What a query body reads, split by where each name resolves.
///
/// The split is the whole point: a name that resolves to a top-level CTE of the
/// same statement is intra-statement lineage (an edge inside the step), while a
/// name that resolves to nothing in scope is a real relation (an edge across
/// assets). A name that resolves to a NESTED declaration appears in neither —
/// it is scope-local, and drawing it would invent lineage that does not exist.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopedReads {
    /// Names resolving to a top-level CTE of the declaring statement.
    pub ctes: BTreeSet<String>,
    /// Names resolving to no CTE in scope — real relations.
    pub relations: BTreeSet<String>,
    /// File paths read via a DuckDB `read_*` table function.
    pub files: BTreeSet<String>,
}

/// A CTE body: what it reads, or why it degraded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CteBody {
    /// The body parsed on its own.
    Parsed(ScopedReads),
    /// The body failed to parse — an opaque chip, never a silent skip.
    Opaque {
        /// The parse error, for the chip's issue badge.
        error: String,
    },
}

/// One CTE declared in a statement's top-level `WITH` clause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CteAssets {
    /// Declared name, lowercased — the tail of the CTE's node id.
    pub name: String,
    /// Declaration order within the `WITH` clause (0-based).
    pub index: usize,
    /// `true` when the clause is `WITH RECURSIVE`, so the CTE may name itself.
    pub recursive: bool,
    /// The body's reads, or the parse error that degraded it.
    pub body: CteBody,
    /// Byte range of the whole `name AS ( … )` declaration within the
    /// statement fragment.
    pub range: (usize, usize),
}

/// DuckDB file-reading table functions whose first string argument is a path.
const READ_FNS: &[&str] = &[
    "read_parquet",
    "read_csv",
    "read_csv_auto",
    "read_xlsx",
    "read_json",
    "read_json_auto",
];

/// Byte offsets for the 1-based (line, column) locations sqlparser reports.
/// Those locations count CHARACTERS, so a column advances by chars — the
/// comments in these model files carry non-ASCII. Built once per source so
/// resolving many token positions stays linear.
struct LineIndex {
    /// Byte offset of the first byte of each line.
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(src: &str) -> Self {
        let mut starts = vec![0usize];
        for (i, ch) in src.char_indices() {
            if ch == '\n' {
                starts.push(i + ch.len_utf8());
            }
        }
        Self { starts }
    }

    /// Byte offset of the 1-based (line, column) location in `src`, clamped to
    /// `src.len()` for a location past the end.
    fn offset(&self, src: &str, line: u64, column: u64) -> usize {
        let line_idx = usize::try_from(line)
            .unwrap_or(usize::MAX)
            .saturating_sub(1);
        let Some(&line_start) = self.starts.get(line_idx) else {
            return src.len();
        };
        let col_idx = usize::try_from(column).unwrap_or(1).saturating_sub(1);
        src[line_start..]
            .char_indices()
            .nth(col_idx)
            .map_or(src.len(), |(i, _)| line_start + i)
    }
}

/// Trim `src[start..end]` and return it as a [`Fragment`] with tightened byte
/// offsets, or `None` when nothing but whitespace remains (empty fragments
/// are dropped).
fn trimmed_fragment(src: &str, start: usize, end: usize) -> Option<Fragment> {
    let slice = &src[start..end];
    let trimmed = slice.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lead = slice.len() - slice.trim_start().len();
    let frag_start = start + lead;
    Some(Fragment {
        text: trimmed.to_string(),
        start: frag_start,
        end: frag_start + trimmed.len(),
    })
}

/// Split `sql` into statement fragments on top-level semicolons.
///
/// The sqlparser `Tokenizer` (DuckDb dialect) consumes strings and comments,
/// so a `;` inside a literal or a `--` comment never splits. If tokenisation
/// itself fails, the whole file becomes ONE fragment — the per-statement
/// parse then degrades it to a single opaque chip (never a silent skip).
#[must_use]
pub fn split_statements(sql: &str) -> Vec<Fragment> {
    let dialect = DuckDbDialect {};
    let tokens = match Tokenizer::new(&dialect, sql).tokenize_with_location() {
        Ok(tokens) => tokens,
        Err(_) => return trimmed_fragment(sql, 0, sql.len()).into_iter().collect(),
    };
    let index = LineIndex::new(sql);
    let mut fragments = Vec::new();
    let mut start = 0usize;
    for tw in &tokens {
        if matches!(tw.token, Token::SemiColon) {
            let semi = index.offset(sql, tw.span.start.line, tw.span.start.column);
            fragments.extend(trimmed_fragment(sql, start, semi));
            start = (semi + 1).min(sql.len());
        }
    }
    fragments.extend(trimmed_fragment(sql, start, sql.len()));
    fragments
}

/// Collects consumed relations/files from one statement's AST. CTE names are
/// gathered separately and subtracted afterwards — a CTE reference is
/// statement-local, not lineage.
#[derive(Default)]
struct RelationCollector {
    relations: BTreeSet<String>,
    files: BTreeSet<String>,
    ctes: BTreeSet<String>,
}

/// Canonical relation key: lowercased rendering of the (possibly dotted) name.
fn object_name_key(name: &ObjectName) -> String {
    name.to_string().to_lowercase()
}

/// File path argument names some DuckDB `read_*` functions accept in named
/// form (`read_csv(path => 'p')`).
fn is_path_arg(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "path" | "path_or_paths")
}

/// Collect the file path(s) a DuckDB `read_*` table function reads: the FIRST
/// positional argument — a `'path'` string OR a `['a','b']` list literal — plus
/// any `path`/`path_or_paths` named argument. Option arguments (`delim='\t'`,
/// `header=true`, …) are deliberately NOT treated as paths, so a stray string
/// option value never masquerades as lineage.
fn read_fn_paths(args: &TableFunctionArgs) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut positional_seen = false;
    for arg in &args.args {
        match arg {
            FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) if !positional_seen => {
                collect_string_paths(expr, &mut out);
                positional_seen = true;
            }
            FunctionArg::Named {
                name,
                arg: FunctionArgExpr::Expr(expr),
                ..
            } if is_path_arg(&name.value) => {
                collect_string_paths(expr, &mut out);
            }
            FunctionArg::ExprNamed {
                name: Expr::Identifier(name),
                arg: FunctionArgExpr::Expr(expr),
                ..
            } if is_path_arg(&name.value) => {
                collect_string_paths(expr, &mut out);
            }
            _ => {}
        }
    }
    out
}

/// Gather single-quoted string literals from a path expression: a bare string,
/// or every string element of a `['a','b']` list literal.
fn collect_string_paths(expr: &Expr, out: &mut BTreeSet<String>) {
    match expr {
        Expr::Value(v) => {
            if let Value::SingleQuotedString(s) = &v.value {
                out.insert(s.clone());
            }
        }
        Expr::Array(arr) => {
            for elem in &arr.elem {
                collect_string_paths(elem, out);
            }
        }
        _ => {}
    }
}

impl Visitor for RelationCollector {
    type Break = ();

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        if let Some(with) = &query.with {
            for cte in &with.cte_tables {
                self.ctes.insert(cte.alias.name.value.to_lowercase());
            }
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, table_factor: &TableFactor) -> ControlFlow<Self::Break> {
        if let TableFactor::Table { name, args, .. } = table_factor {
            let key = object_name_key(name);
            if let Some(args) = args {
                // A table FUNCTION: read_* consumes a file; any other table
                // function (generate_series, ...) is not lineage.
                if READ_FNS.contains(&key.as_str()) {
                    self.files.extend(read_fn_paths(args));
                }
            } else {
                self.relations.insert(key);
            }
        }
        ControlFlow::Continue(())
    }
}

/// Collects a body's reads against a SCOPE STACK, so a name declared in more
/// than one scope resolves to its innermost declaration.
///
/// The bottom frame is the statement's top-level CTE names, injected by the
/// caller; every nested query pushes its own `WITH` names on top. A read
/// resolving to the bottom frame is intra-statement lineage; one resolving to
/// any deeper frame is scope-local and draws nothing; one resolving to no frame
/// is a real relation.
struct ScopedCollector {
    /// `(is_top_level, names)`, innermost last.
    scopes: Vec<(bool, BTreeSet<String>)>,
    reads: ScopedReads,
}

impl ScopedCollector {
    fn new(top_level: BTreeSet<String>) -> Self {
        Self {
            scopes: vec![(true, top_level)],
            reads: ScopedReads::default(),
        }
    }
}

impl Visitor for ScopedCollector {
    type Break = ();

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        let names = query
            .with
            .iter()
            .flat_map(|with| with.cte_tables.iter())
            .map(|cte| cte.alias.name.value.to_lowercase())
            .collect();
        self.scopes.push((false, names));
        ControlFlow::Continue(())
    }

    fn post_visit_query(&mut self, _query: &Query) -> ControlFlow<Self::Break> {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, table_factor: &TableFactor) -> ControlFlow<Self::Break> {
        if let TableFactor::Table { name, args, .. } = table_factor {
            let key = object_name_key(name);
            if let Some(args) = args {
                if READ_FNS.contains(&key.as_str()) {
                    self.reads.files.extend(read_fn_paths(args));
                }
            } else {
                match self
                    .scopes
                    .iter()
                    .rev()
                    .find(|(_, names)| names.contains(&key))
                {
                    // The innermost declaration wins: a top-level CTE is
                    // lineage, a nested one is scope-local and draws nothing.
                    Some((true, _)) => {
                        self.reads.ctes.insert(key);
                    }
                    Some((false, _)) => {}
                    None => {
                        self.reads.relations.insert(key);
                    }
                }
            }
        }
        ControlFlow::Continue(())
    }
}

/// One CTE's slices within a statement fragment.
struct CteSlice {
    /// Declared name, lowercased.
    name: String,
    /// Byte range of the body BETWEEN the declaration's parentheses.
    body: (usize, usize),
    /// Byte range of the whole `name AS ( … )` declaration.
    decl: (usize, usize),
}

/// A statement's top-level `WITH` clause, located at the token level.
struct WithClause {
    /// `true` for `WITH RECURSIVE`.
    recursive: bool,
    /// Byte offset of the `WITH` keyword.
    start: usize,
    /// Byte offset where the statement's main body begins.
    body_start: usize,
    /// The CTEs, in declaration order.
    ctes: Vec<CteSlice>,
}

/// Index one past the parenthesis group opening at `start` (`toks[start]` must
/// be an `LParen`); `None` when the group is never closed.
fn skip_group(toks: &[(usize, &Token)], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (k, (_, token)) in toks.iter().enumerate().skip(start) {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => {
                depth -= 1;
                if depth == 0 {
                    return Some(k + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// The unquoted keyword of `toks[i]`, when it is a bare word.
fn keyword_at(toks: &[(usize, &Token)], i: usize) -> Option<Keyword> {
    match toks.get(i)?.1 {
        Token::Word(w) if w.quote_style.is_none() => Some(w.keyword),
        _ => None,
    }
}

/// Locate a statement's top-level `WITH` clause at the TOKEN level.
///
/// Returns `None` for a statement with no `WITH`, and for any `WITH` that is
/// not a CTE clause (`GROUP BY … WITH ROLLUP`, a `WITH (…)` option list): the
/// shape `name [ (cols) ] AS ( body )` must hold from the first depth-0 `WITH`
/// onward, or nothing is claimed. Bailing keeps the statement on exactly the
/// behaviour it had before CTEs were modelled.
fn split_with_clause(sql: &str) -> Option<WithClause> {
    let dialect = DuckDbDialect {};
    let tokens = Tokenizer::new(&dialect, sql)
        .tokenize_with_location()
        .ok()?;
    let index = LineIndex::new(sql);
    // Whitespace tokens carry the comments too; dropping them leaves the
    // grammar tokens with their byte offsets.
    let toks: Vec<(usize, &Token)> = tokens
        .iter()
        .filter(|t| !matches!(t.token, Token::Whitespace(_)))
        .map(|t| {
            (
                index.offset(sql, t.span.start.line, t.span.start.column),
                &t.token,
            )
        })
        .collect();

    // The FIRST depth-0 `WITH` — a deeper one belongs to a subquery.
    let mut depth = 0i32;
    let mut i = 0usize;
    let mut with_start = None;
    for (k, (off, token)) in toks.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth -= 1,
            Token::Word(w)
                if depth == 0 && w.quote_style.is_none() && w.keyword == Keyword::WITH =>
            {
                with_start = Some(*off);
                i = k + 1;
                break;
            }
            _ => {}
        }
    }
    let start = with_start?;
    let recursive = keyword_at(&toks, i) == Some(Keyword::RECURSIVE);
    if recursive {
        i += 1;
    }

    let mut ctes = Vec::new();
    loop {
        let (decl_start, Token::Word(name)) = *toks.get(i)? else {
            return None;
        };
        let name = name.value.to_lowercase();
        i += 1;
        // An optional column alias list: `name (a, b) AS ( … )`.
        if matches!(toks.get(i)?.1, Token::LParen) {
            i = skip_group(&toks, i)?;
        }
        if keyword_at(&toks, i)? != Keyword::AS {
            return None;
        }
        i += 1;
        // `[NOT] MATERIALIZED` between AS and the body.
        while matches!(
            keyword_at(&toks, i),
            Some(Keyword::NOT | Keyword::MATERIALIZED)
        ) {
            i += 1;
        }
        let (lparen, token) = *toks.get(i)?;
        if !matches!(token, Token::LParen) {
            return None;
        }
        let after = skip_group(&toks, i)?;
        let rparen = toks[after - 1].0;
        ctes.push(CteSlice {
            name,
            body: (lparen + 1, rparen),
            decl: (decl_start, rparen + 1),
        });
        i = after;
        match toks.get(i) {
            Some((_, Token::Comma)) => i += 1,
            // Anything else starts the main body.
            Some((off, _)) => {
                return Some(WithClause {
                    recursive,
                    start,
                    body_start: *off,
                    ctes,
                })
            }
            // A `WITH` clause with no body is not a statement we model.
            None => return None,
        }
    }
}

/// One statement's CTE view: the declared CTEs, the main body's reads, and the
/// relation the statement produces — all derived from the statement with its
/// `WITH` clause stripped, so one malformed CTE body costs only that CTE.
struct CteView {
    ctes: Vec<CteAssets>,
    main: ScopedReads,
    produced: Option<String>,
}

/// Derive the CTE view of one statement fragment. `None` when the fragment has
/// no top-level `WITH` clause, or when the statement does not parse even with
/// that clause stripped — in which case nothing about its CTEs is claimed.
fn extract_ctes(stmt: &str) -> Option<CteView> {
    let with = split_with_clause(stmt)?;
    let names: Vec<String> = with.ctes.iter().map(|c| c.name.clone()).collect();
    let top: BTreeSet<String> = names.iter().cloned().collect();

    // The statement minus its WITH clause: the main body still carrying its
    // `CREATE … AS` prefix. This is what survives a malformed CTE body.
    let stripped = format!("{}{}", &stmt[..with.start], &stmt[with.body_start..]);
    let parsed = brightfield_sql::conform::parse_and_normalise(&stripped).ok()?;
    let mut produced = None;
    let mut main = ScopedCollector::new(top);
    for statement in &parsed {
        match statement {
            Statement::CreateTable(ct) => produced = Some(object_name_key(&ct.name)),
            Statement::CreateView(cv) => produced = Some(object_name_key(&cv.name)),
            _ => {}
        }
        let _ = statement.visit(&mut main);
    }

    let ctes = with
        .ctes
        .iter()
        .enumerate()
        .map(|(index, slice)| {
            // Earlier siblings are in scope. `WITH RECURSIVE` also puts the
            // CTE's own name in scope, so a self-reference is not misread as a
            // relation (the recursion badge itself is a later card).
            let mut scope: BTreeSet<String> = names[..index].iter().cloned().collect();
            if with.recursive {
                scope.insert(slice.name.clone());
            }
            let body = match brightfield_sql::conform::parse_and_normalise(
                &stmt[slice.body.0..slice.body.1],
            ) {
                Ok(stmts) => {
                    let mut collector = ScopedCollector::new(scope);
                    for statement in &stmts {
                        let _ = statement.visit(&mut collector);
                    }
                    CteBody::Parsed(collector.reads)
                }
                Err(e) => CteBody::Opaque {
                    error: e.to_string(),
                },
            };
            CteAssets {
                name: slice.name.clone(),
                index,
                recursive: with.recursive,
                body,
                range: slice.decl,
            }
        })
        .collect();

    Some(CteView {
        ctes,
        main: main.reads,
        produced,
    })
}

/// Split `sql` and extract per-statement assets, degrading each unparseable
/// statement to [`StatementAssets::Opaque`] while its siblings still explode.
/// Comment-only fragments contribute nothing.
///
/// A statement whose only defect is a malformed CTE body stays
/// [`StatementAssets::Parsed`] — it keeps the relation it produces and every
/// CTE that did parse, and the bad body degrades alone (see the module docs).
/// Such a statement carries `degraded: Some(error)`, because its lineage is
/// real but incomplete; a statement that recovers NO relation is never promoted
/// at all, so it cannot vanish from a graph that draws only produced nodes.
#[must_use]
pub fn extract_statement_assets(sql: &str) -> Vec<StatementAssets> {
    let mut out = Vec::new();
    for (index, frag) in split_statements(sql).iter().enumerate() {
        let range = (frag.start, frag.end);
        let view = extract_ctes(&frag.text);
        match brightfield_sql::conform::parse_and_normalise(&frag.text) {
            Ok(stmts) if stmts.is_empty() => {} // comment-only fragment
            Ok(stmts) => {
                let mut produced = None;
                let mut collector = RelationCollector::default();
                for stmt in &stmts {
                    match stmt {
                        Statement::CreateTable(ct) => produced = Some(object_name_key(&ct.name)),
                        Statement::CreateView(cv) => produced = Some(object_name_key(&cv.name)),
                        _ => {}
                    }
                    let _ = stmt.visit(&mut collector);
                }
                let mut consumed_relations: BTreeSet<String> = collector
                    .relations
                    .difference(&collector.ctes)
                    .cloned()
                    .collect();
                if let Some(p) = &produced {
                    consumed_relations.remove(p);
                }
                let (ctes, main_reads) = view.map_or_else(Default::default, |v| (v.ctes, v.main));
                out.push(StatementAssets::Parsed {
                    index,
                    produced,
                    consumed_relations,
                    consumed_files: collector.files,
                    ctes,
                    main_reads,
                    degraded: None,
                    range,
                });
            }
            Err(e) => match view {
                // The statement parses once its WITH clause is stripped AND
                // that stripped parse recovered the relation it produces, so
                // the defect is inside a CTE body: keep the target relation and
                // every good CTE, and let the bad body be the only chip.
                //
                // The recovered relation is the PRECONDITION, not a detail. A
                // targetless statement (INSERT/COPY/UPDATE) draws no node at
                // all, so promoting one would delete it from the graph — no
                // node, no chip, no badge — which is the silent skip this
                // module exists to prevent. Refuse the promotion and keep the
                // chip.
                Some(view) if view.produced.is_some() && !view.ctes.is_empty() => {
                    let mut consumed_relations = view.main.relations.clone();
                    let mut consumed_files = view.main.files.clone();
                    for cte in &view.ctes {
                        if let CteBody::Parsed(reads) = &cte.body {
                            consumed_relations.extend(reads.relations.iter().cloned());
                            consumed_files.extend(reads.files.iter().cloned());
                        }
                    }
                    if let Some(p) = &view.produced {
                        consumed_relations.remove(p);
                    }
                    out.push(StatementAssets::Parsed {
                        index,
                        produced: view.produced,
                        consumed_relations,
                        consumed_files,
                        ctes: view.ctes,
                        main_reads: view.main,
                        // What the malformed body read is unknowable, so this
                        // lineage is real but incomplete. Carry the reason so
                        // the produced node is badged, never drawn healthy.
                        degraded: Some(e.to_string()),
                        range,
                    });
                }
                _ => out.push(StatementAssets::Opaque {
                    index,
                    error: e.to_string(),
                    range,
                }),
            },
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pds_split_semicolon_in_string_never_splits() {
        let frags = split_statements("SELECT ';' AS a FROM t; SELECT 2");
        assert_eq!(frags.len(), 2);
        assert_eq!(frags[0].text, "SELECT ';' AS a FROM t");
        assert_eq!(frags[1].text, "SELECT 2");
    }

    #[test]
    fn pds_split_semicolon_in_comment_never_splits() {
        let sql = "-- not a boundary; still the same comment\nSELECT 1;\nSELECT 2";
        let frags = split_statements(sql);
        assert_eq!(frags.len(), 2, "the comment `;` must not split: {frags:?}");
        // Byte ranges point back into the source verbatim.
        for f in &frags {
            assert_eq!(&sql[f.start..f.end], f.text);
        }
    }

    #[test]
    fn pds_split_drops_empty_fragments() {
        let frags = split_statements(";;  ;SELECT 1;;\n  ");
        assert_eq!(frags.len(), 1);
        assert_eq!(frags[0].text, "SELECT 1");
    }

    #[test]
    fn pds_split_tokenizer_failure_yields_whole_file_fragment() {
        // An unterminated string literal fails tokenisation — the whole file
        // becomes ONE fragment, which the parse pass degrades to ONE chip.
        let sql = "SELECT 'unterminated";
        let frags = split_statements(sql);
        assert_eq!(frags.len(), 1);
        assert_eq!(frags[0].text, sql);
        let assets = extract_statement_assets(sql);
        assert_eq!(assets.len(), 1);
        assert!(matches!(
            assets[0],
            StatementAssets::Opaque { index: 0, .. }
        ));
    }

    #[test]
    fn middle_statement_degrades_alone() {
        let sql = "CREATE TABLE a AS SELECT 1;\n\
                   SELEC every FORM here IS deliberately unparseable;\n\
                   CREATE TABLE b AS SELECT * FROM a;";
        let assets = extract_statement_assets(sql);
        assert_eq!(assets.len(), 3);
        assert!(
            matches!(&assets[0], StatementAssets::Parsed { produced: Some(p), .. } if p == "a")
        );
        assert!(matches!(
            &assets[1],
            StatementAssets::Opaque { index: 1, .. }
        ));
        match &assets[2] {
            StatementAssets::Parsed {
                produced,
                consumed_relations,
                ..
            } => {
                assert_eq!(produced.as_deref(), Some("b"));
                assert!(consumed_relations.contains("a"), "sibling still explodes");
            }
            other => panic!("third statement must parse, got {other:?}"),
        }
    }

    #[test]
    fn pds_extract_produced_consumed_and_files() {
        let sql = "CREATE OR REPLACE TABLE t AS \
                   SELECT * FROM read_parquet('build/p.parquet') r JOIN u ON r.id = u.id";
        let assets = extract_statement_assets(sql);
        assert_eq!(assets.len(), 1);
        match &assets[0] {
            StatementAssets::Parsed {
                produced,
                consumed_relations,
                consumed_files,
                ..
            } => {
                assert_eq!(produced.as_deref(), Some("t"));
                assert_eq!(
                    consumed_relations.iter().collect::<Vec<_>>(),
                    vec![&"u".to_string()]
                );
                assert_eq!(
                    consumed_files.iter().collect::<Vec<_>>(),
                    vec![&"build/p.parquet".to_string()]
                );
            }
            other => panic!("expected Parsed, got {other:?}"),
        }
    }

    #[test]
    fn pds_read_parquet_list_form_captures_every_path() {
        // The DuckDB list form read_parquet(['a','b']) must yield BOTH file
        // edges, not silently drop the lineage (only the unnamed single-string
        // form was matched before).
        let sql =
            "CREATE TABLE t AS SELECT * FROM read_parquet(['build/a.parquet', 'build/b.parquet'])";
        let assets = extract_statement_assets(sql);
        match &assets[0] {
            StatementAssets::Parsed { consumed_files, .. } => {
                assert!(consumed_files.contains("build/a.parquet"));
                assert!(consumed_files.contains("build/b.parquet"));
                assert_eq!(consumed_files.len(), 2);
            }
            other => panic!("expected Parsed, got {other:?}"),
        }
    }

    #[test]
    fn pds_read_csv_named_path_form_captures_the_file() {
        // The named `read_csv(path = 'x')` form must capture the file edge (the
        // old first_string_arg only matched an unnamed positional string).
        let sql = "CREATE TABLE u AS SELECT * FROM read_csv(path = 'build/c.csv')";
        let assets = extract_statement_assets(sql);
        match &assets[0] {
            StatementAssets::Parsed { consumed_files, .. } => {
                assert!(
                    consumed_files.contains("build/c.csv"),
                    "named path captured: {consumed_files:?}"
                );
            }
            other => panic!("expected Parsed, got {other:?}"),
        }
    }

    #[test]
    fn pds_read_csv_named_path_and_option_strings() {
        // The named `path =>` form must be captured as lineage, while a string
        // OPTION whose value merely LOOKS like a path (`filename => 'meta/…'`)
        // must NOT be. Every argument here is named, so the pre-fix
        // `first_string_arg` (unnamed-positional-only) matched NOTHING and
        // consumed_files came back empty — so this asserts both the real
        // capture and the non-capture the fix introduced, and fails pre-fix.
        let sql = "CREATE TABLE u AS SELECT * FROM \
                   read_csv(path => 'build/real.csv', filename => 'meta/label.txt', header => true)";
        let assets = extract_statement_assets(sql);
        match &assets[0] {
            StatementAssets::Parsed { consumed_files, .. } => {
                assert!(
                    consumed_files.contains("build/real.csv"),
                    "named `path =>` is captured: {consumed_files:?}"
                );
                assert!(
                    !consumed_files.contains("meta/label.txt"),
                    "a path-like string OPTION value is not lineage: {consumed_files:?}"
                );
                assert_eq!(
                    consumed_files.len(),
                    1,
                    "only the real path: {consumed_files:?}"
                );
            }
            other => panic!("expected Parsed, got {other:?}"),
        }
    }

    /// The CTEs a statement declares are extracted in order, and each body's
    /// reads are split: a sibling CTE is intra-statement, a real relation is
    /// not, and a `read_*` path is a file.
    #[test]
    fn pds_cte_bodies_split_sibling_reads_from_relation_reads() {
        let sql = "CREATE OR REPLACE TABLE out AS \
                   WITH ck AS (SELECT * FROM read_csv('build/a.txt')), \
                        tk AS (SELECT * FROM ck JOIN real_table USING (id)) \
                   SELECT * FROM ck JOIN tk USING (id) JOIN other USING (id)";
        let assets = extract_statement_assets(sql);
        let StatementAssets::Parsed {
            ctes, main_reads, ..
        } = &assets[0]
        else {
            panic!("expected Parsed, got {:?}", assets[0])
        };
        assert_eq!(
            ctes.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["ck", "tk"]
        );
        assert_eq!(ctes[0].index, 0);
        assert_eq!(ctes[1].index, 1);
        assert!(ctes.iter().all(|c| !c.recursive));
        let CteBody::Parsed(ck) = &ctes[0].body else {
            panic!("ck parses")
        };
        assert!(ck.files.contains("build/a.txt"));
        assert!(ck.relations.is_empty());
        let CteBody::Parsed(tk) = &ctes[1].body else {
            panic!("tk parses")
        };
        assert!(tk.ctes.contains("ck"), "an earlier sibling is in scope");
        assert!(tk.relations.contains("real_table"));
        // The main body names both CTEs plus one real relation.
        assert!(main_reads.ctes.contains("ck") && main_reads.ctes.contains("tk"));
        assert_eq!(
            main_reads.relations.iter().collect::<Vec<_>>(),
            vec![&"other".to_string()]
        );
        // The declaration range points back into the fragment verbatim.
        let (s, e) = ctes[0].range;
        assert!(sql[s..e].starts_with("ck AS ("), "range: {:?}", &sql[s..e]);
        assert!(sql[s..e].ends_with(')'));
    }

    /// A name declared in more than one scope resolves to its INNERMOST
    /// declaration: the reference inside `tk` is to `tk`'s own `ck`, not to the
    /// top-level one, so it is scope-local and names no lineage.
    #[test]
    fn pds_inner_declaration_shadows_the_top_level_cte() {
        let sql = "CREATE OR REPLACE TABLE out AS \
                   WITH ck AS (SELECT * FROM source_table), \
                        tk AS (WITH ck AS (SELECT * FROM inner_source) SELECT * FROM ck) \
                   SELECT * FROM ck JOIN tk USING (id)";
        let assets = extract_statement_assets(sql);
        let StatementAssets::Parsed { ctes, .. } = &assets[0] else {
            panic!("expected Parsed")
        };
        let CteBody::Parsed(tk) = &ctes[1].body else {
            panic!("tk parses")
        };
        assert!(
            !tk.ctes.contains("ck"),
            "the innermost declaration wins: {tk:?}"
        );
        assert!(
            !tk.relations.contains("ck"),
            "a shadowed name is scope-local, never a relation: {tk:?}"
        );
        assert!(tk.relations.contains("inner_source"));
    }

    /// A recursive CTE may name itself: the self-reference resolves in scope
    /// rather than fabricating a relation of the same name.
    #[test]
    fn pds_recursive_cte_self_reference_is_not_a_relation() {
        let sql = "CREATE OR REPLACE TABLE out AS \
                   WITH RECURSIVE walk AS (SELECT * FROM seed UNION ALL SELECT * FROM walk) \
                   SELECT * FROM walk";
        let assets = extract_statement_assets(sql);
        let StatementAssets::Parsed { ctes, .. } = &assets[0] else {
            panic!("expected Parsed")
        };
        assert!(ctes[0].recursive);
        let CteBody::Parsed(walk) = &ctes[0].body else {
            panic!("walk parses")
        };
        assert!(!walk.relations.contains("walk"), "{walk:?}");
        assert!(walk.relations.contains("seed"));
    }

    /// One malformed CTE body costs only that CTE: the statement keeps the
    /// relation it produces, its good sibling still reports its reads, and the
    /// bad body carries the parse error.
    #[test]
    fn pds_malformed_cte_body_degrades_alone() {
        let sql = "CREATE OR REPLACE TABLE degraded AS \
                   WITH good AS (SELECT * FROM real_table), \
                        bad AS (SELEC every FORM here IS deliberately unparseable) \
                   SELECT * FROM good JOIN bad USING (id)";
        let assets = extract_statement_assets(sql);
        let StatementAssets::Parsed {
            produced,
            consumed_relations,
            ctes,
            main_reads,
            degraded,
            ..
        } = &assets[0]
        else {
            panic!(
                "a bad CTE body must not black-box the statement: {:?}",
                assets[0]
            )
        };
        assert_eq!(produced.as_deref(), Some("degraded"));
        assert!(consumed_relations.contains("real_table"));
        assert!(matches!(&ctes[0].body, CteBody::Parsed(_)), "good explodes");
        let CteBody::Opaque { error } = &ctes[1].body else {
            panic!("bad degrades")
        };
        assert!(!error.is_empty(), "the chip carries its parse error");
        assert!(main_reads.ctes.contains("good") && main_reads.ctes.contains("bad"));
        // Recovered, not clean: the consumer must badge what it draws.
        assert!(
            degraded.as_ref().is_some_and(|e| !e.is_empty()),
            "a recovered statement is marked incomplete"
        );
    }

    /// A statement whose stripped form recovers NO relation is never promoted.
    /// A graph that draws only produced nodes would drop it entirely, so the
    /// promotion that saves a `CREATE … AS` would delete an `INSERT` — chip and
    /// all. It stays opaque.
    #[test]
    fn pds_targetless_statement_with_bad_cte_body_stays_opaque() {
        for sql in [
            "INSERT INTO sink WITH bad AS (SELEC every FORM here IS unparseable) SELECT * FROM bad",
            "WITH bad AS () SELECT 1",
        ] {
            let assets = extract_statement_assets(sql);
            assert!(
                matches!(assets[0], StatementAssets::Opaque { index: 0, .. }),
                "{sql} produces no relation, so it keeps its chip: {:?}",
                assets[0]
            );
        }
    }

    /// A statement that parses whole is never marked incomplete, even when one
    /// of its CTE bodies fails to parse ON ITS OWN — the whole-statement parse
    /// already gave the full lineage, and a badge there would be a false alarm.
    #[test]
    fn pds_a_statement_that_parses_whole_is_never_marked_degraded() {
        let sql = "CREATE TABLE out AS WITH c AS (SELECT * FROM real_table) SELECT * FROM c";
        let assets = extract_statement_assets(sql);
        let StatementAssets::Parsed { degraded, .. } = &assets[0] else {
            panic!("expected Parsed")
        };
        assert!(degraded.is_none(), "{degraded:?}");
    }

    /// A `WITH` that is not a CTE clause claims nothing — the statement keeps
    /// exactly the behaviour it had before CTEs were modelled.
    #[test]
    fn pds_non_cte_with_clause_claims_nothing() {
        for sql in [
            "SELECT a, count(*) FROM t GROUP BY a WITH ROLLUP",
            "CREATE TABLE t (a INT) WITH (order_by = 'a')",
        ] {
            let assets = extract_statement_assets(sql);
            match &assets[0] {
                StatementAssets::Parsed { ctes, .. } => {
                    assert!(ctes.is_empty(), "{sql} declares no CTE: {ctes:?}");
                }
                // Some of these do not parse at all in this dialect; either way
                // no CTE is claimed.
                StatementAssets::Opaque { .. } => {}
            }
        }
    }

    #[test]
    fn pds_extract_cte_names_are_not_consumed_relations() {
        let sql = "CREATE TABLE out AS WITH c AS (SELECT * FROM real_table) \
                   SELECT * FROM c JOIN other ON c.id = other.id";
        let assets = extract_statement_assets(sql);
        match &assets[0] {
            StatementAssets::Parsed {
                consumed_relations, ..
            } => {
                assert!(consumed_relations.contains("real_table"));
                assert!(consumed_relations.contains("other"));
                assert!(
                    !consumed_relations.contains("c"),
                    "CTE names are statement-local"
                );
            }
            other => panic!("expected Parsed, got {other:?}"),
        }
    }
}
