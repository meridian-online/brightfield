//! Which DuckDB extensions a session may install and load, and the statements
//! in a piece of SQL that install or load one.
//!
//! The engine runs on one of two DuckDBs. duckdb-rs's bundled build, which the
//! full edition links, has none of the extensions Brightfield reads with, and
//! the engine installs them from DuckDB's repository as a source needs them.
//! The managed edition's build (`docs/managed-duckdb.md`) has them compiled in,
//! and a Mac App Store app may not download code, so there the engine neither
//! installs an extension nor loads one from a file: [`Extensions::BuiltIn`],
//! which `managed_refuses_install_and_load_of_anything_else` in
//! `tests/built_in_extensions.rs` holds.
//!
//! Which one a session gets follows the DuckDB the engine links, as
//! [`Extensions::linked`] says, because "built in" is a fact about that library
//! rather than a choice a caller makes. A caller can still ask for
//! [`Extensions::BuiltIn`] on the bundled DuckDB through
//! [`crate::LoadOptions::extensions`], which is how the refusals are tested in
//! every build.

/// The DuckDB extensions built into the managed edition's DuckDB, by the names
/// a statement may give them: those `scripts/duckdb-managed/extensions.cmake`
/// compiles in, and the four DuckDB's release links itself. The sqlite
/// extension answers to `sqlite` and to `sqlite_scanner`.
pub const BUILT_IN: &[&str] = &[
    "core_functions",
    "parquet",
    "json",
    "icu",
    "autocomplete",
    "excel",
    "sqlite",
    "sqlite_scanner",
    "httpfs",
    "ducklake",
    "spatial",
    "finetype",
];

/// Where a session's DuckDB extensions come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extensions {
    /// Installed from DuckDB's repository when a source needs one, then
    /// loaded: the full edition, on duckdb-rs's bundled DuckDB.
    Installable,
    /// Compiled into the DuckDB the engine links. Nothing is installed,
    /// automatic install and load are off, and a statement that installs an
    /// extension, updates extensions, or loads anything but a built-in one is
    /// refused.
    BuiltIn,
}

impl Default for Extensions {
    fn default() -> Self {
        Extensions::linked()
    }
}

impl Extensions {
    /// The extensions of the DuckDB this build links: [`Extensions::BuiltIn`]
    /// with the `managed-duckdb` feature, which links the managed edition's
    /// library, and [`Extensions::Installable`] otherwise.
    #[must_use]
    pub fn linked() -> Self {
        if cfg!(feature = "managed-duckdb") {
            Extensions::BuiltIn
        } else {
            Extensions::Installable
        }
    }

    /// Whether the engine may install an extension it needs.
    #[must_use]
    pub fn installs(self) -> bool {
        self == Extensions::Installable
    }
}

/// What a statement does to an extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// `INSTALL`, or `FORCE INSTALL`.
    Install,
    /// `LOAD`.
    Load,
    /// `UPDATE EXTENSIONS`, which downloads every installed extension again.
    Update,
}

/// A statement that installs, loads or updates DuckDB extensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionStatement {
    /// What it does.
    pub verb: Verb,
    /// The extension as the statement names it, without quotes: a name, or the
    /// path of a file. Empty for [`Verb::Update`].
    pub target: String,
}

impl ExtensionStatement {
    /// Whether the statement names a built-in extension by name, rather than a
    /// file or an extension outside [`BUILT_IN`].
    #[must_use]
    pub fn names_a_built_in(&self) -> bool {
        let is_file = self.target.contains('/') || self.target.ends_with(".duckdb_extension");
        !is_file
            && BUILT_IN
                .iter()
                .any(|b| b.eq_ignore_ascii_case(&self.target))
    }

    /// Whether a session with [`Extensions::BuiltIn`] refuses it: an install
    /// or an update, since either would download code, and a load of a
    /// file or of an extension that is not built in. A load of a built-in
    /// extension is allowed, and does nothing, since it is already loaded.
    #[must_use]
    pub fn refused_when_built_in(&self) -> bool {
        match self.verb {
            Verb::Install | Verb::Update => true,
            Verb::Load => !self.names_a_built_in(),
        }
    }

    /// The sentence an analyst reads when a session with
    /// [`Extensions::BuiltIn`] refuses the statement.
    #[must_use]
    pub fn refusal(&self) -> String {
        let target = &self.target;
        let what = match self.verb {
            Verb::Install => format!("install the DuckDB extension {target}"),
            Verb::Load => format!("load the DuckDB extension {target}"),
            Verb::Update => "update DuckDB extensions".to_string(),
        };
        format!("This edition can't {what}: it runs only the DuckDB extensions built into it.")
    }
}

/// What an analyst reads when a source on a session with
/// [`Extensions::BuiltIn`] reads as more than one statement: one after its own
/// could change the settings that keep the session to its built-in extensions.
pub const ONE_STATEMENT: &str = "This edition reads a source as one statement, and this one \
     holds more than one.";

/// The first statement in `sql` that a session with [`Extensions::BuiltIn`]
/// refuses, if any.
#[must_use]
pub fn refused_when_built_in(sql: &str) -> Option<ExtensionStatement> {
    statements(sql)
        .into_iter()
        .find(ExtensionStatement::refused_when_built_in)
}

/// The statements in `sql` that install, load or update extensions, in order.
///
/// Comments are not read, a `;` inside quotes does not end a statement, and a
/// quoted target keeps what is inside its quotes, so a path holding a space or
/// a `--` is read whole.
#[must_use]
pub fn statements(sql: &str) -> Vec<ExtensionStatement> {
    split_statements(sql)
        .iter()
        .filter_map(|statement| parse(statement))
        .collect()
}

/// One statement's words, read as an extension statement if it is one.
fn parse(statement: &str) -> Option<ExtensionStatement> {
    let words = words(statement);
    let word = |i: usize| words.get(i).map(String::as_str).unwrap_or_default();
    let is = |i: usize, keyword: &str| word(i).eq_ignore_ascii_case(keyword);
    let (verb, target) = if is(0, "force") && is(1, "install") {
        (Verb::Install, word(2))
    } else if is(0, "install") {
        (Verb::Install, word(1))
    } else if is(0, "load") {
        (Verb::Load, word(1))
    } else if is(0, "update") && is(1, "extensions") {
        (Verb::Update, "")
    } else {
        return None;
    };
    Some(ExtensionStatement {
        verb,
        target: unquote(target).to_string(),
    })
}

/// A word without the quotes around it.
fn unquote(word: &str) -> &str {
    for q in ['\'', '"'] {
        if let Some(inner) = word.strip_prefix(q).and_then(|w| w.strip_suffix(q)) {
            return inner;
        }
    }
    word
}

/// A statement's words: split at whitespace outside quotes, with a quoted
/// stretch kept in one word with its quotes.
fn words(statement: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    for c in statement.chars() {
        match quote {
            Some(q) => {
                word.push(c);
                if c == q {
                    quote = None;
                }
            }
            None if c == '\'' || c == '"' => {
                word.push(c);
                quote = Some(c);
            }
            None if c.is_whitespace() => {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
            }
            None => word.push(c),
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// `sql` cut into statements at each `;` outside quotes, with `--` and
/// `/* */` comments taken out.
fn split_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                current.push(c);
                for d in chars.by_ref() {
                    current.push(d);
                    if d == c {
                        break;
                    }
                }
            }
            '-' if chars.peek() == Some(&'-') => {
                for d in chars.by_ref() {
                    if d == '\n' {
                        current.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut last = '\0';
                for d in chars.by_ref() {
                    if last == '*' && d == '/' {
                        break;
                    }
                    last = d;
                }
                current.push(' ');
            }
            ';' => out.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    out.push(current);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(sql: &str) -> ExtensionStatement {
        let found = statements(sql);
        assert_eq!(found.len(), 1, "{sql:?} gave {found:?}");
        found.into_iter().next().unwrap()
    }

    #[test]
    fn each_form_is_read() {
        assert_eq!(one("INSTALL h3 FROM community").target, "h3");
        assert_eq!(one("force install spatial").verb, Verb::Install);
        assert_eq!(
            one("LOAD '/x y/h3.duckdb_extension'").target,
            "/x y/h3.duckdb_extension"
        );
        assert_eq!(one("update extensions").verb, Verb::Update);
        assert!(statements("SELECT 'INSTALL h3'; CREATE TABLE load (x INT)").is_empty());
    }

    #[test]
    fn comments_are_not_read_and_quotes_hold_semicolons() {
        assert!(statements("-- INSTALL h3\nSELECT 1 /* LOAD h3 */").is_empty());
        let found = statements("SELECT ';INSTALL h3'; LOAD excel; INSTALL h3");
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[1].target, "h3");
    }

    #[test]
    fn built_in_sessions_refuse_installs_and_foreign_loads_only() {
        assert!(refused_when_built_in("LOAD excel; LOAD 'spatial'").is_none());
        assert_eq!(
            refused_when_built_in("LOAD excel; INSTALL excel")
                .unwrap()
                .verb,
            Verb::Install
        );
        assert_eq!(refused_when_built_in("LOAD h3").unwrap().target, "h3");
        assert_eq!(
            refused_when_built_in("LOAD '/tmp/excel.duckdb_extension'")
                .unwrap()
                .target,
            "/tmp/excel.duckdb_extension"
        );
        assert!(refused_when_built_in("UPDATE EXTENSIONS").is_some());
    }

    #[test]
    fn the_refusal_names_the_extension() {
        assert_eq!(
            one("INSTALL h3").refusal(),
            "This edition can't install the DuckDB extension h3: it runs only the DuckDB \
             extensions built into it."
        );
    }
}
