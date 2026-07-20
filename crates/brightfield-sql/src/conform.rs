//! sqlparser-rs structural conformance utilities.
//!
//! Low-level parse + compare functions. `brightfield-conformance` calls these
//! for layer-2 checks; the emitter's own tests also use them.

use sqlparser::dialect::DuckDbDialect;
use sqlparser::parser::Parser;

use crate::error::EmitError;

/// Parse a SQL string with the DuckDB dialect and return the normalised AST.
///
/// # Errors
///
/// Returns `EmitError::SqlParseError` if the SQL is not valid DuckDB syntax.
pub fn parse_and_normalise(
    sql: &str,
) -> Result<Vec<sqlparser::ast::Statement>, EmitError> {
    let dialect = DuckDbDialect {};
    Parser::parse_sql(&dialect, sql).map_err(|e| EmitError::SqlParseError {
        detail: e.to_string(),
    })
}

/// Compare two SQL strings structurally.
///
/// Tolerates whitespace, alias ordering, and keyword case differences by
/// comparing the parsed ASTs. Returns `true` if structurally equivalent.
///
/// # Errors
///
/// Returns `EmitError::SqlParseError` if either string fails to parse.
pub fn structural_eq(a: &str, b: &str) -> Result<bool, EmitError> {
    let ast_a = parse_and_normalise(a)?;
    let ast_b = parse_and_normalise(b)?;
    Ok(ast_a == ast_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dfir_ac11_structural_eq_case_insensitive() {
        let result = structural_eq(
            "SELECT * FROM t WHERE x > 1",
            "select * from t where x > 1",
        )
        .unwrap();
        assert!(result, "case-insensitive SQL should be structurally equal");
    }

    #[test]
    fn dfir_ac11_structural_eq_whitespace_tolerant() {
        let result = structural_eq(
            "SELECT  *  FROM  t  WHERE  x > 1",
            "SELECT * FROM t WHERE x > 1",
        )
        .unwrap();
        assert!(result, "whitespace differences should not matter");
    }

    #[test]
    fn dfir_ac11_structural_neq_different_clauses() {
        let result = structural_eq("SELECT * FROM t", "SELECT * FROM t WHERE TRUE").unwrap();
        assert!(
            !result,
            "structurally different SQL should not be equal"
        );
    }

    #[test]
    fn dfir_ac11_parse_error_on_invalid_sql() {
        let result = parse_and_normalise("SELCT * FORM");
        assert!(result.is_err());
    }
}
