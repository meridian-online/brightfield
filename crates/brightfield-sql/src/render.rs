//! Canonicalisation helpers for deterministic DDL snapshots.

use crate::emit::SourceDdl;

/// Produce a canonical string form of a set of DDL statements.
///
/// - Statements sorted alphabetically by `view_name`.
/// - Within each statement, kwargs are sorted alphabetically.
/// - Whitespace normalised: single spaces, no trailing whitespace, LF line endings.
///
/// This function is the single canonical form used by both snapshot generation
/// and conformance comparison.
pub fn canonicalise_ddl(statements: &[SourceDdl]) -> String {
    let mut sorted: Vec<&SourceDdl> = statements.iter().collect();
    sorted.sort_by(|a, b| a.view_name.cmp(&b.view_name));

    let lines: Vec<String> = sorted
        .iter()
        .map(|ddl| canonicalise_statement(&ddl.sql))
        .collect();

    // Join with LF, trailing LF
    let mut output = lines.join("\n");
    if !output.is_empty() {
        output.push('\n');
    }
    output
}

/// Canonicalise a single SQL statement.
///
/// - Collapses runs of whitespace to single spaces.
/// - Trims leading/trailing whitespace.
/// - Sorts kwargs inside function calls alphabetically.
fn canonicalise_statement(sql: &str) -> String {
    // Normalise whitespace: collapse runs to single space, trim
    let normalised: String = sql
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ");

    // Sort kwargs inside function calls like read_csv('path', auto_detect=true, delim='|')
    // We look for patterns like func_name('first_arg', key=val, key=val)
    canonicalise_kwargs(&normalised)
}

/// Sort kwargs in function calls alphabetically.
///
/// Recognises patterns: `func_name('path_arg', kwarg1=v1, kwarg2=v2)`
/// Sorts kwargs after the first positional arg.
fn canonicalise_kwargs(sql: &str) -> String {
    // Find function calls with kwargs: look for read_csv, read_parquet, etc.
    let func_names = [
        "read_csv(",
        "read_parquet(",
        "read_json_auto(",
        "ST_Read(",
    ];

    let mut result = sql.to_string();

    for func in &func_names {
        if let Some(func_start) = result.find(func) {
            let args_start = func_start + func.len();
            // Find the matching closing paren
            if let Some(paren_end) = find_matching_paren(&result, args_start) {
                let args_str = &result[args_start..paren_end];
                let sorted_args = sort_kwargs_in_args(args_str);
                result = format!(
                    "{}{}{}{}",
                    &result[..args_start],
                    sorted_args,
                    ")",
                    &result[paren_end + 1..]
                );
            }
        }
    }

    result
}

/// Find the matching closing parenthesis, handling nested parens.
fn find_matching_paren(s: &str, start: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 1u32;
    let mut in_quote = false;

    for i in start..bytes.len() {
        match bytes[i] {
            b'\'' if !in_quote => in_quote = true,
            b'\'' if in_quote => in_quote = false,
            b'(' if !in_quote => depth += 1,
            b')' if !in_quote => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Sort kwargs after the first positional argument.
///
/// Input: `'path_arg', kwarg2=val2, kwarg1=val1`
/// Output: `'path_arg', kwarg1=val1, kwarg2=val2`
fn sort_kwargs_in_args(args_str: &str) -> String {
    // Split on commas respecting quotes
    let parts = split_args(args_str);

    if parts.len() <= 1 {
        return args_str.to_string();
    }

    // First part is the positional arg (e.g. 'path')
    let positional = &parts[0];

    // Remaining parts are kwargs
    let mut kwargs: Vec<String> = parts[1..].iter().map(|s| s.trim().to_string()).collect();

    // Sort kwargs alphabetically
    kwargs.sort();

    let mut result = positional.to_string();
    for kwarg in &kwargs {
        result.push_str(", ");
        result.push_str(kwarg);
    }

    result
}

/// Split args on commas, respecting single-quoted strings.
fn split_args(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut depth = 0u32;

    for ch in s.chars() {
        match ch {
            '\'' => {
                in_quote = !in_quote;
                current.push(ch);
            }
            '(' if !in_quote => {
                depth += 1;
                current.push(ch);
            }
            ')' if !in_quote && depth > 0 => {
                depth -= 1;
                current.push(ch);
            }
            ',' if !in_quote && depth == 0 => {
                parts.push(current.trim().to_string());
                current = String::new();
            }
            _ => current.push(ch),
        }
    }

    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }

    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emit::{SourceDdl, SourceKindTag};

    #[test]
    fn dfsql_canonicalise_is_deterministic() {
        let stmts = vec![
            SourceDdl {
                view_name: "b".to_string(),
                sql: "CREATE OR REPLACE VIEW \"b\" AS SELECT * FROM read_parquet('b.parquet')"
                    .to_string(),
                source_kind: SourceKindTag::Parquet,
            },
            SourceDdl {
                view_name: "a".to_string(),
                sql: "CREATE OR REPLACE VIEW \"a\" AS SELECT * FROM read_csv('a.csv', auto_detect=true)"
                    .to_string(),
                source_kind: SourceKindTag::Csv,
            },
        ];

        let first = canonicalise_ddl(&stmts);
        let second = canonicalise_ddl(&stmts);
        assert_eq!(first, second, "canonicalise_ddl must be deterministic");
    }

    #[test]
    fn dfsql_canonicalise_sorts_by_view_name() {
        let stmts = vec![
            SourceDdl {
                view_name: "zebra".to_string(),
                sql: "CREATE OR REPLACE VIEW \"zebra\" AS SELECT 1".to_string(),
                source_kind: SourceKindTag::Query,
            },
            SourceDdl {
                view_name: "alpha".to_string(),
                sql: "CREATE OR REPLACE VIEW \"alpha\" AS SELECT 2".to_string(),
                source_kind: SourceKindTag::Query,
            },
        ];

        let result = canonicalise_ddl(&stmts);
        let lines: Vec<&str> = result.trim().lines().collect();
        assert!(lines[0].contains("\"alpha\""), "alpha should come first");
        assert!(lines[1].contains("\"zebra\""), "zebra should come second");
    }
}
