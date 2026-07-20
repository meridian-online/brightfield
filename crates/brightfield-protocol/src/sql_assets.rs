//! Per-statement SQL asset extraction.
//!
//! `brightfield_sql::conform::parse_and_normalise` parses multi-statement SQL
//! but fails the WHOLE string on one bad statement — so the splitter here
//! finds statement boundaries at the TOKEN level (semicolons outside
//! strings/comments) and each statement is parsed individually, degrading
//! **per-statement, never per-file**. Every fragment keeps its
//! byte range in the source file so later cards can highlight it.

use std::collections::BTreeSet;
use std::ops::ControlFlow;

use sqlparser::ast::{
    Expr, FunctionArg, FunctionArgExpr, ObjectName, Query, Statement, TableFactor,
    TableFunctionArgs, Value, Visit, Visitor,
};
use sqlparser::dialect::DuckDbDialect;
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

/// DuckDB file-reading table functions whose first string argument is a path.
const READ_FNS: &[&str] = &[
    "read_parquet",
    "read_csv",
    "read_csv_auto",
    "read_xlsx",
    "read_json",
    "read_json_auto",
];

/// Byte offset of the 1-based (line, column) location in `src`. sqlparser
/// locations count CHARACTERS, so this walks chars (comments in these files
/// carry non-ASCII).
fn byte_offset(src: &str, line: u64, column: u64) -> usize {
    let (mut cur_line, mut cur_col) = (1u64, 1u64);
    for (i, ch) in src.char_indices() {
        if cur_line == line && cur_col == column {
            return i;
        }
        if ch == '\n' {
            cur_line += 1;
            cur_col = 1;
        } else {
            cur_col += 1;
        }
    }
    src.len()
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
    let mut fragments = Vec::new();
    let mut start = 0usize;
    for tw in &tokens {
        if matches!(tw.token, Token::SemiColon) {
            let semi = byte_offset(sql, tw.span.start.line, tw.span.start.column);
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
            FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => {
                if !positional_seen {
                    collect_string_paths(expr, &mut out);
                    positional_seen = true;
                }
            }
            FunctionArg::Named {
                name,
                arg: FunctionArgExpr::Expr(expr),
                ..
            } => {
                if is_path_arg(&name.value) {
                    collect_string_paths(expr, &mut out);
                }
            }
            FunctionArg::ExprNamed {
                name: Expr::Identifier(name),
                arg: FunctionArgExpr::Expr(expr),
                ..
            } => {
                if is_path_arg(&name.value) {
                    collect_string_paths(expr, &mut out);
                }
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

/// Split `sql` and extract per-statement assets, degrading each unparseable
/// statement to [`StatementAssets::Opaque`] while its siblings still explode.
/// Comment-only fragments contribute nothing.
#[must_use]
pub fn extract_statement_assets(sql: &str) -> Vec<StatementAssets> {
    let mut out = Vec::new();
    for (index, frag) in split_statements(sql).iter().enumerate() {
        let range = (frag.start, frag.end);
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
                out.push(StatementAssets::Parsed {
                    index,
                    produced,
                    consumed_relations,
                    consumed_files: collector.files,
                    range,
                });
            }
            Err(e) => out.push(StatementAssets::Opaque {
                index,
                error: e.to_string(),
                range,
            }),
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
