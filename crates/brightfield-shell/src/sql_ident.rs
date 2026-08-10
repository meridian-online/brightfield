//! A column's name, and the SQL identifier that spells it.
//!
//! Two halves of one fact, in one file so they cannot drift apart. A clause
//! that names a column has to carry the **identifier** — `observed by hour =
//! '2026-01-02 05'` is a parser error where `"observed by hour" = '2026-01-02
//! 05'` is a filter — while the code that matches a committed clause back
//! against the column a plot drew holds the bare **name**. Both directions are
//! used inside one gesture, so [`quote`] and [`name_of`] are inverses and are
//! tested as a round trip rather than a pair.
//!
//! Quoting is unconditional, not "when it looks like it needs it". The
//! alternative wants a list of the words DuckDB reserves, and a column named
//! `select` is as legal in a file as one named with a space; a list that has to
//! be complete to be correct is the wrong shape for this. It is also what the
//! mark lowerers already do — `brightfield_sql`'s lowerers write a channel
//! column as `"{col}"` everywhere — so a clause built this way binds the same
//! column the mark it came from drew.

use std::borrow::Cow;

/// `name` written as a SQL identifier: double-quoted, with an embedded quote
/// doubled.
#[must_use]
pub(crate) fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// The name a SQL identifier expression spells — [`quote`]'s inverse.
///
/// A bare name comes back unchanged, so this normalises both of the spellings a
/// clause's column arrives in: the identifier a gesture publishes and the plain
/// name a plot handle carries.
///
/// An expression that is not one whole quoted identifier — `"a" || "b"`,
/// `lower("a")` — is not a name and comes back unchanged rather than being
/// mangled into one.
#[must_use]
pub(crate) fn name_of(expr: &str) -> Cow<'_, str> {
    let Some(inner) = expr
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return Cow::Borrowed(expr);
    };
    let mut name = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '"' {
            // Inside one identifier every quote is half of a doubled pair. A
            // lone one means the quotes at the ends were not a matched pair
            // around a name — `"a" || "b"` — so nothing here is an identifier.
            if chars.next() != Some('"') {
                return Cow::Borrowed(expr);
            }
        }
        name.push(c);
    }
    Cow::Owned(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every name survives the round trip, including the two that make the
    /// escaping necessary: a space, which is why this module exists, and an
    /// embedded quote, which is what the doubling carries.
    #[test]
    fn a_name_survives_being_written_as_an_identifier_and_read_back() {
        for name in [
            "region",
            "observed by hour",
            "sales region",
            "select",
            "we\"ird",
            "\"",
            "",
            "a\"\"b",
        ] {
            assert_eq!(name_of(&quote(name)), name, "{name:?}");
        }
    }

    /// A bare name reads as itself, so a clause built anywhere else — an
    /// interval slider's spec column, a hand-written probe — still matches the
    /// plot that drew it.
    #[test]
    fn a_bare_name_reads_as_itself() {
        assert_eq!(name_of("region"), "region");
        assert_eq!(name_of("observed by hour"), "observed by hour");
    }

    /// What is not one identifier is handed back whole. Reading `"a" || "b"` as
    /// the name `a" || "b` would match a column nobody has, quietly, which is
    /// worse than not matching at all.
    #[test]
    fn an_expression_that_is_not_one_identifier_is_left_alone() {
        for expr in ["\"a\" || \"b\"", "lower(\"a\")", "\"unclosed", "trailing\""] {
            assert_eq!(name_of(expr), expr, "{expr:?}");
        }
    }
}
