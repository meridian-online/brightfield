//! Canonicalisation helpers for deterministic DDL snapshots and IR → SQL rendering.

use crate::binding::Binding;
use crate::emit::SourceDdl;
use crate::ir::{Predicate, QueryPlan, SortDir};

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

/// Render a `QueryPlan` to DuckDB-dialect SQL, accumulating `Binding` entries
/// for parameterised positions.
///
/// Source renders as a bare table name (the VIEW created by card 0004's DDL).
/// Filter renders a WHERE clause. Aggregation uses DuckDB positional GROUP BY.
/// Bin renders a `width_bucket()` call.
pub fn render_query(plan: &QueryPlan, bindings: &mut Vec<Binding>) -> String {
    match plan {
        QueryPlan::Source { table } => {
            format!("SELECT * FROM \"{table}\"")
        }
        QueryPlan::Filter { input, predicate } => {
            let inner = render_query(input, bindings);
            let pred_sql = render_predicate(predicate, bindings);
            format!("SELECT * FROM ({inner}) AS _f WHERE {pred_sql}")
        }
        QueryPlan::Projection { input, columns } => {
            let inner = render_query(input, bindings);
            let cols = columns.join(", ");
            format!("SELECT {cols} FROM ({inner}) AS _p")
        }
        QueryPlan::Aggregation {
            input,
            group_by,
            aggregates,
        } => {
            let inner = render_query(input, bindings);
            let mut select_cols = group_by.clone();
            select_cols.extend(aggregates.iter().cloned());
            let select = select_cols.join(", ");
            // DuckDB positional GROUP BY
            let positions: Vec<String> = (1..=group_by.len()).map(|i| i.to_string()).collect();
            let group = positions.join(", ");
            format!("SELECT {select} FROM ({inner}) AS _a GROUP BY {group}")
        }
        QueryPlan::AggregateScalar { input, aggregates } => {
            let inner = render_query(input, bindings);
            let select = aggregates.join(", ");
            format!("SELECT {select} FROM ({inner}) AS _as")
        }
        QueryPlan::Bin {
            input,
            column,
            width,
            alias,
        } => {
            let inner = render_query(input, bindings);
            format!(
                "SELECT *, width_bucket(\"{column}\", {width}) AS \"{alias}\" FROM ({inner}) AS _b"
            )
        }
        QueryPlan::Order { input, keys } => {
            let inner = render_query(input, bindings);
            let order_parts: Vec<String> = keys
                .iter()
                .map(|(col, dir)| {
                    let d = match dir {
                        SortDir::Asc => "ASC",
                        SortDir::Desc => "DESC",
                    };
                    format!("{col} {d}")
                })
                .collect();
            format!("SELECT * FROM ({inner}) AS _o ORDER BY {}", order_parts.join(", "))
        }
        QueryPlan::Limit {
            input,
            limit,
            offset,
        } => {
            let inner = render_query(input, bindings);
            match offset {
                Some(off) => format!("SELECT * FROM ({inner}) AS _l LIMIT {limit} OFFSET {off}"),
                None => format!("SELECT * FROM ({inner}) AS _l LIMIT {limit}"),
            }
        }
    }
}

/// Render a predicate to SQL, recording bindings for `Param` nodes.
fn render_predicate(predicate: &Predicate, bindings: &mut Vec<Binding>) -> String {
    match predicate {
        Predicate::Expr(s) => s.clone(),
        Predicate::Param { name, .. } => {
            let position = bindings.len();
            bindings.push(Binding::Scalar {
                param: name.clone(),
                position,
            });
            "?".to_string()
        }
        Predicate::And(preds) => {
            if preds.is_empty() {
                return "TRUE".to_string();
            }
            let parts: Vec<String> = preds
                .iter()
                .map(|p| render_predicate(p, bindings))
                .collect();
            format!("({})", parts.join(" AND "))
        }
        Predicate::Or(preds) => {
            if preds.is_empty() {
                return "FALSE".to_string();
            }
            let parts: Vec<String> = preds
                .iter()
                .map(|p| render_predicate(p, bindings))
                .collect();
            format!("({})", parts.join(" OR "))
        }
        Predicate::True => "TRUE".to_string(),
        Predicate::False => "FALSE".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conform::parse_and_normalise;
    use crate::emit::{SourceDdl, SourceKindTag};

    /// Helper: assert SQL is valid DuckDB syntax via sqlparser-rs.
    fn assert_valid_sql(sql: &str) {
        parse_and_normalise(sql).unwrap_or_else(|e| {
            panic!("SQL failed to parse:\n  SQL: {sql}\n  Error: {e}");
        });
    }

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

    #[test]
    fn dfir_ac04_render_source() {
        let plan = QueryPlan::Source {
            table: "flights".to_string(),
        };
        let mut bindings = Vec::new();
        let sql = render_query(&plan, &mut bindings);
        assert_eq!(sql, "SELECT * FROM \"flights\"");
        assert!(bindings.is_empty());
        assert_valid_sql(&sql);
    }

    #[test]
    fn dfir_ac04_render_filter() {
        let plan = QueryPlan::Filter {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            predicate: Predicate::Expr("x > 1".to_string()),
        };
        let mut bindings = Vec::new();
        let sql = render_query(&plan, &mut bindings);
        assert!(sql.contains("WHERE x > 1"));
        assert_valid_sql(&sql);
    }

    #[test]
    fn dfir_ac04_render_projection() {
        let plan = QueryPlan::Projection {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            columns: vec!["x".to_string(), "y".to_string()],
        };
        let mut bindings = Vec::new();
        let sql = render_query(&plan, &mut bindings);
        assert!(sql.contains("SELECT x, y"));
        assert_valid_sql(&sql);
    }

    #[test]
    fn dfir_ac04_render_aggregation() {
        let plan = QueryPlan::Aggregation {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            group_by: vec!["x".to_string()],
            aggregates: vec!["SUM(y) AS total".to_string()],
        };
        let mut bindings = Vec::new();
        let sql = render_query(&plan, &mut bindings);
        assert!(sql.contains("GROUP BY 1"), "should use positional group by");
        assert!(sql.contains("SUM(y) AS total"));
        assert_valid_sql(&sql);
    }

    #[test]
    fn dfir_ac04_render_composed_plan() {
        // Source → Filter → Projection
        let plan = QueryPlan::Projection {
            input: Box::new(QueryPlan::Filter {
                input: Box::new(QueryPlan::Source {
                    table: "t".to_string(),
                }),
                predicate: Predicate::Expr("x > 1".to_string()),
            }),
            columns: vec!["x".to_string()],
        };
        let mut bindings = Vec::new();
        let sql = render_query(&plan, &mut bindings);
        assert!(sql.contains("WHERE x > 1"));
        assert!(sql.starts_with("SELECT x FROM"));
        assert_valid_sql(&sql);
    }

    #[test]
    fn dfir_ac04_render_param_predicate_records_binding() {
        let plan = QueryPlan::Filter {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            predicate: Predicate::Param {
                name: "threshold".to_string(),
                placeholder_index: 0,
            },
        };
        let mut bindings = Vec::new();
        let sql = render_query(&plan, &mut bindings);
        assert!(sql.contains("WHERE ?"));
        assert_eq!(bindings.len(), 1);
        assert_eq!(
            bindings[0],
            Binding::Scalar {
                param: "threshold".to_string(),
                position: 0,
            }
        );
    }

    #[test]
    fn dfir_ac04_render_order() {
        let plan = QueryPlan::Order {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            keys: vec![
                ("x".to_string(), SortDir::Asc),
                ("y".to_string(), SortDir::Desc),
            ],
        };
        let mut bindings = Vec::new();
        let sql = render_query(&plan, &mut bindings);
        assert!(sql.contains("ORDER BY x ASC, y DESC"));
        assert_valid_sql(&sql);
    }

    #[test]
    fn gomb_ac01_aggregate_scalar_emits_no_group_by() {
        let plan = QueryPlan::AggregateScalar {
            input: Box::new(QueryPlan::Source {
                table: "athletes".to_string(),
            }),
            aggregates: vec!["regr_slope(height, weight) AS slope".to_string()],
        };
        let mut bindings = Vec::new();
        let sql = render_query(&plan, &mut bindings);
        assert!(
            sql.contains("SELECT regr_slope(height, weight) AS slope FROM"),
            "expected scalar aggregate select prefix, got: {sql}"
        );
        assert!(
            !sql.contains("GROUP BY"),
            "AggregateScalar must not emit GROUP BY, got: {sql}"
        );
        assert_valid_sql(&sql);
    }

    #[test]
    fn dfir_ac04_render_limit() {
        let plan = QueryPlan::Limit {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            limit: 100,
            offset: Some(10),
        };
        let mut bindings = Vec::new();
        let sql = render_query(&plan, &mut bindings);
        assert!(sql.contains("LIMIT 100 OFFSET 10"));
        assert_valid_sql(&sql);
    }
}
