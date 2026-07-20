//! SQL expression tokeniser with literal-awareness.
//!
//! Mirrors the upstream Mosaic 0.24.x behaviour of
//! `packages/vgplot/spec/src/ExpressionNode.js`:
//!
//! - `$ident` outside literals lifts to a [`ParamRef`]; `ident` matches
//!   `[A-Za-z_][A-Za-z0-9_]*`.
//! - `$` inside a single-quoted string literal (`'...'`) is preserved as
//!   literal text. Single quotes are escaped by doubling (`''`).
//! - `$` inside a double-quoted identifier (`"..."`) is preserved as literal
//!   text. Double quotes are escaped by doubling (`""`).
//! - `$` inside a line comment (`-- ... <newline>`) is preserved as literal
//!   text. The comment extends to the next newline or end-of-input.
//! - `$` inside a block comment (`/* ... */`) is preserved as literal text.
//!   Block comments are flat (not nested) per SQL conventions.
//!
//! The tokeniser is deliberately narrow: it recognises only these literal
//! contexts. It does not implement full SQL lexing — it only needs to know
//! enough to avoid treating `$foo` inside literal contexts as a param ref.

use crate::ast::{ExpressionNode, ParamRef};

/// Tokenise a SQL expression string into an [`ExpressionNode`].
///
/// The result's invariant holds: `spans.len() == params.len() + 1`. A string
/// with no param refs returns an `ExpressionNode` with a single span and no
/// params.
#[must_use]
pub fn tokenise(src: &str) -> ExpressionNode {
    let bytes = src.as_bytes();
    let mut spans: Vec<String> = Vec::new();
    let mut params: Vec<ParamRef> = Vec::new();
    let mut current = String::new();
    let mut i = 0usize;
    let n = bytes.len();

    while i < n {
        let b = bytes[i];

        // Single-quoted string literal: copy verbatim including any `$`.
        if b == b'\'' {
            current.push('\'');
            i += 1;
            while i < n {
                let c = bytes[i];
                if c == b'\'' {
                    // Doubled single quote is an embedded quote, still inside.
                    if i + 1 < n && bytes[i + 1] == b'\'' {
                        current.push('\'');
                        current.push('\'');
                        i += 2;
                        continue;
                    }
                    current.push('\'');
                    i += 1;
                    break;
                }
                // Preserve raw byte as char via UTF-8 safe append using str slicing.
                let ch_end = utf8_char_end(bytes, i);
                current.push_str(&src[i..ch_end]);
                i = ch_end;
            }
            continue;
        }

        // Double-quoted identifier: copy verbatim including any `$`.
        if b == b'"' {
            current.push('"');
            i += 1;
            while i < n {
                let c = bytes[i];
                if c == b'"' {
                    if i + 1 < n && bytes[i + 1] == b'"' {
                        current.push('"');
                        current.push('"');
                        i += 2;
                        continue;
                    }
                    current.push('"');
                    i += 1;
                    break;
                }
                let ch_end = utf8_char_end(bytes, i);
                current.push_str(&src[i..ch_end]);
                i = ch_end;
            }
            continue;
        }

        // Line comment: `--` to end-of-line.
        if b == b'-' && i + 1 < n && bytes[i + 1] == b'-' {
            current.push_str("--");
            i += 2;
            while i < n && bytes[i] != b'\n' {
                let ch_end = utf8_char_end(bytes, i);
                current.push_str(&src[i..ch_end]);
                i = ch_end;
            }
            if i < n {
                current.push('\n');
                i += 1;
            }
            continue;
        }

        // Block comment: `/* ... */` (flat, non-nested).
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            current.push_str("/*");
            i += 2;
            while i < n {
                if bytes[i] == b'*' && i + 1 < n && bytes[i + 1] == b'/' {
                    current.push_str("*/");
                    i += 2;
                    break;
                }
                let ch_end = utf8_char_end(bytes, i);
                current.push_str(&src[i..ch_end]);
                i = ch_end;
            }
            continue;
        }

        // `$ident` lift candidate.
        if b == b'$' && i + 1 < n && is_ident_start(bytes[i + 1]) {
            // Scan the identifier.
            let start = i + 1;
            let mut j = start;
            while j < n && is_ident_continue(bytes[j]) {
                j += 1;
            }
            let name = &src[start..j];
            spans.push(std::mem::take(&mut current));
            params.push(ParamRef::new(name.to_string()));
            i = j;
            continue;
        }

        // Anything else: copy the next UTF-8 char verbatim.
        let ch_end = utf8_char_end(bytes, i);
        current.push_str(&src[i..ch_end]);
        i = ch_end;
    }

    spans.push(current);
    ExpressionNode { spans, params }
}

/// Advance past one UTF-8 character starting at `i`, returning the byte index
/// one past its end. Safe on valid UTF-8; if called on the last byte of a
/// multibyte sequence the continuation rules still advance by a sensible step.
fn utf8_char_end(bytes: &[u8], i: usize) -> usize {
    let b = bytes[i];
    let step = if b < 0x80 {
        1
    } else if b < 0xC0 {
        // Should not happen on boundary; advance by 1 defensively.
        1
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    };
    (i + step).min(bytes.len())
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test 1: `$ident` outside literal contexts lifts to a ParamRef.
    #[test]
    fn dfspec_ac06_params_outside_literals() {
        let node = tokenise("x > $lo AND x < $hi");
        assert_eq!(node.spans, vec!["x > ", " AND x < ", ""]);
        assert_eq!(node.params.len(), 2);
        assert_eq!(node.params[0], ParamRef::new("lo"));
        assert_eq!(node.params[1], ParamRef::new("hi"));
        assert_eq!(node.to_wire(), "x > $lo AND x < $hi");
    }

    /// test 2: `$` inside a single-quoted string literal is preserved
    /// verbatim and does not lift to a ParamRef.
    #[test]
    fn dfspec_ac06_dollar_in_single_quoted_literal() {
        let node = tokenise("label = '$100 raised'");
        assert_eq!(node.spans, vec!["label = '$100 raised'"]);
        assert!(node.params.is_empty());
        assert!(node.is_literal());
    }

    /// test 3: `$` inside a double-quoted identifier is preserved
    /// verbatim and does not lift to a ParamRef.
    #[test]
    fn dfspec_ac06_dollar_in_double_quoted_identifier() {
        let node = tokenise(r#"SELECT "col$name" FROM t"#);
        assert_eq!(node.spans, vec![r#"SELECT "col$name" FROM t"#]);
        assert!(node.params.is_empty());
    }

    /// test 4: `$` inside a `-- line comment` is preserved verbatim
    /// through end-of-line.
    #[test]
    fn dfspec_ac06_dollar_in_line_comment() {
        let node = tokenise("x -- free $cash until eol\nAND $real");
        // The `$real` after the comment *is* a param ref; `$cash` inside is not.
        assert_eq!(node.params, vec![ParamRef::new("real")]);
        assert_eq!(
            node.spans,
            vec!["x -- free $cash until eol\nAND ", ""]
        );
    }

    /// test 5: `$` inside a `/* block comment */` is preserved verbatim.
    #[test]
    fn dfspec_ac06_dollar_in_block_comment() {
        let node = tokenise("x /* ignore $this */ AND $that");
        assert_eq!(node.params, vec![ParamRef::new("that")]);
        assert_eq!(
            node.spans,
            vec!["x /* ignore $this */ AND ", ""]
        );
    }

    /// Extra sanity: unterminated single-quote runs to end-of-input without
    /// lifting an interior `$ident`.
    #[test]
    fn dfspec_ac06_unterminated_single_quote_preserves_dollar() {
        let node = tokenise("'$oops");
        assert_eq!(node.spans, vec!["'$oops"]);
        assert!(node.params.is_empty());
    }

    /// Extra sanity: a bare `$` with no identifier after it is literal, not
    /// a lift.
    #[test]
    fn dfspec_ac06_bare_dollar_is_literal() {
        let node = tokenise("total = $");
        assert_eq!(node.spans, vec!["total = $"]);
        assert!(node.params.is_empty());
    }

    /// Extra sanity: round-trip via `to_wire()` is exact when input uses only
    /// `$ident` lift form.
    #[test]
    fn dfspec_ac06_to_wire_roundtrip() {
        for src in [
            "",
            "plain text",
            "$a",
            "a $b c",
            "a $b $c $d",
            "'$in' + $out",
        ] {
            let node = tokenise(src);
            assert_eq!(node.to_wire(), src, "round-trip mismatch for {src:?}");
        }
    }
}
