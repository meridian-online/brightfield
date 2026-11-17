//! Which step of a Protocol, if any, this edition of Brightfield cannot run.
//!
//! The managed edition runs arc's own operators and the DuckDB extensions built into the app
//! ([`crate::edition`]). A Protocol with a step outside that opens restricted, as an untrusted one
//! does, and its band names the first such step, the reason, and where the full edition is. Trust
//! does not lift this restriction, so [`crate::trust::refuses_run`] refuses a run of such a
//! Protocol however it is trusted (`trust_does_not_lift_the_edition_restriction`).
//!
//! A step is beyond the managed edition when it is:
//!
//! - a `command:` step, which is a line of shell;
//! - a step with a shell precondition, which is shell too, and so is a tool precondition that
//!   reports its tool's version with a shell command;
//! - an operator that starts a program from outside the app, such as `uv` for the operators that
//!   run a pinned Python script, with one exception: FineType's CLI, which the managed edition
//!   ships as a helper of its own, so arc's FineType operators run;
//! - a step that installs or loads a DuckDB extension the app does not have built in, or loads one
//!   from a file.
//!
//! The lifecycle hooks are steps too, and are asked about the same way, after the steps.
//!
//! Which operators start which program is read from arc's operator code at the revision this
//! workspace pins, and kept in [`OPERATORS`]. An operator not listed there, which a newer arc may
//! add, is taken to start a program of its own until it is listed: the managed edition opens the
//! Protocol restricted rather than run a step nobody has looked at.

use std::path::Path;

use arc::spec::{Manifest, Precondition, Step};

use crate::edition::{Capability, Edition};

/// The first step of a Protocol this edition cannot run, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeyondEdition {
    /// The step's name, as its manifest gives it.
    pub step: String,
    /// Why this edition cannot run it.
    pub reason: Reason,
}

/// Why an edition cannot run a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// A `command:` step, with the program its command starts.
    CommandStep(String),
    /// A precondition that runs a shell command.
    Precondition,
    /// An operator that starts a program from outside the app, named here.
    OutsideProgram(String),
    /// A DuckDB extension the app does not have built in, as the step names it: a name, or the
    /// path of a file the step loads one from.
    Extension(String),
}

/// The DuckDB extensions built into the managed edition, which a step may install and load: those
/// `scripts/duckdb-managed/extensions.cmake` compiles into its library, by the names a statement
/// may give them. The sqlite extension answers to `sqlite` and to `sqlite_scanner`.
pub const BUILT_IN_EXTENSIONS: &[&str] = &[
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

/// The program FineType's operators start, which the managed edition ships as a helper.
pub const FINETYPE: &str = "finetype";

/// Each operator in arc's catalog, by name, and the program it starts from outside the app, if
/// any. `None` is an operator arc runs itself, or through the DuckDB the app carries.
pub const OPERATORS: &[(&str, Option<&str>)] = &[
    ("parquet_export", None),
    ("archive_extract", None),
    ("datapackage_describe", Some(FINETYPE)),
    ("finetype_validate", Some(FINETYPE)),
    ("splink_resolve", Some("uv")),
    ("gleif_ra_fetch", Some("uv")),
    ("umap_project", Some("uv")),
    ("text_embed", Some("uv")),
    ("uv", Some("uv")),
    ("ducklake_publish", None),
    ("http_fetch", None),
    ("html_link_discover", None),
    ("ducklake_write", None),
];

/// The operator keys that name a DuckDB extension file the step loads.
const EXTENSION_KEYS: &[&str] = &["extension"];

/// The first step of the Protocol in `dir` that `edition` cannot run, reading its manifest from
/// there. A folder whose manifest does not load answers `None`: what is wrong with it is said by
/// whatever tries to open or run it.
#[must_use]
pub fn of_folder(dir: &Path, edition: Edition) -> Option<BeyondEdition> {
    let manifest = Manifest::load(dir).ok()?;
    beyond_edition(dir, &manifest, edition)
}

/// The first step of `manifest`, a Protocol in `dir`, that `edition` cannot run: its steps in
/// order, then its hooks. `dir` is where its steps' SQL files are read from.
#[must_use]
pub fn beyond_edition(dir: &Path, manifest: &Manifest, edition: Edition) -> Option<BeyondEdition> {
    every_step_beyond(dir, manifest, edition).into_iter().next()
}

/// The steps of `manifest`, a Protocol in `dir`, that `edition` cannot run, in the order
/// [`beyond_edition`] asks about them, each with the first reason found for it.
#[must_use]
pub fn every_step_beyond(dir: &Path, manifest: &Manifest, edition: Edition) -> Vec<BeyondEdition> {
    let hooks = &manifest.hooks;
    manifest
        .steps
        .iter()
        .chain(hooks.on_init.iter())
        .chain(hooks.on_success.iter())
        .chain(hooks.on_failure.iter())
        .chain(hooks.on_exit.iter())
        .filter_map(|step| {
            step_beyond(dir, step, edition).map(|reason| BeyondEdition {
                step: step.name.clone(),
                reason,
            })
        })
        .collect()
}

/// Whether a SQL statement recorded as the step `name` is beyond `edition`: whether it installs or
/// loads an extension the app does not have built in. What the window asks as a statement is added.
#[must_use]
pub fn statement(name: &str, sql: &str, edition: Edition) -> Option<BeyondEdition> {
    if edition.allows(Capability::ExtensionInstall) {
        return None;
    }
    sql_extension_beyond(sql).map(|ext| BeyondEdition {
        step: name.to_string(),
        reason: Reason::Extension(ext),
    })
}

/// The sentence that says why `b`'s step does not run, for the band and the banners.
#[must_use]
pub fn sentence(b: &BeyondEdition) -> String {
    let step = &b.step;
    match &b.reason {
        Reason::CommandStep(program) | Reason::OutsideProgram(program) => {
            format!("Step {step} runs {program}, which this edition can't run.")
        }
        Reason::Precondition => format!(
            "Step {step} checks whether it is up to date with a shell command, which this edition \
             can't run."
        ),
        Reason::Extension(ext) => {
            format!("Step {step} loads the DuckDB extension {ext}, which this edition can't load.")
        }
    }
}

/// Why `edition` cannot run `step`, if it cannot.
fn step_beyond(dir: &Path, step: &Step, edition: Edition) -> Option<Reason> {
    if !edition.allows(Capability::CommandSteps) {
        if let Some(command) = &step.command {
            return Some(Reason::CommandStep(program_of(command)));
        }
        if step.preconditions.iter().any(runs_shell) {
            return Some(Reason::Precondition);
        }
    }
    if !edition.allows(Capability::OutsidePrograms) {
        if let Some(op) = &step.op {
            let name = op.split_once('@').map_or(op.as_str(), |(n, _)| n).trim();
            match OPERATORS.iter().find(|(n, _)| *n == name) {
                Some((_, Some(program))) if *program != FINETYPE => {
                    return Some(Reason::OutsideProgram((*program).to_string()));
                }
                Some(_) => {}
                None => return Some(Reason::OutsideProgram(name.to_string())),
            }
        }
    }
    if !edition.allows(Capability::ExtensionInstall) {
        if let Some(with) = step.with.as_ref().and_then(serde_yaml::Value::as_mapping) {
            for key in EXTENSION_KEYS {
                if let Some(file) = with.get(*key).and_then(serde_yaml::Value::as_str) {
                    return Some(Reason::Extension(file.to_string()));
                }
            }
        }
        if let Some(sql) = &step.sql {
            if let Ok(text) = std::fs::read_to_string(dir.join(sql)) {
                if let Some(ext) = sql_extension_beyond(&text) {
                    return Some(Reason::Extension(ext));
                }
            }
        }
    }
    None
}

/// Whether a precondition runs a shell command.
fn runs_shell(p: &Precondition) -> bool {
    match p {
        Precondition::Command { .. } => true,
        Precondition::Tool { tool } => tool.version.is_some(),
        _ => false,
    }
}

/// The program a shell command line starts: its first word that is not a variable assignment,
/// without its folder. A line with no such word is named `sh`, which is what runs it.
fn program_of(command: &str) -> String {
    command
        .split_whitespace()
        .find(|word| !is_assignment(word))
        .map(|word| word.rsplit('/').next().unwrap_or(word).to_string())
        .filter(|word| !word.is_empty())
        .unwrap_or_else(|| "sh".to_string())
}

/// Whether a shell word is a `NAME=value` assignment, which sets a variable for the program after
/// it rather than naming one.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// The first extension `sql` installs or loads that the managed edition does not have built in,
/// as written: a bare name not in [`BUILT_IN_EXTENSIONS`], or any file path, since the managed
/// edition loads no extension from a file. Comments are not read.
fn sql_extension_beyond(sql: &str) -> Option<String> {
    let text = without_comments(sql);
    for statement in text.split(';') {
        let words: Vec<&str> = statement.split_whitespace().collect();
        let named = match words.as_slice() {
            [force, install, name, ..]
                if force.eq_ignore_ascii_case("force")
                    && install.eq_ignore_ascii_case("install") =>
            {
                name
            }
            [verb, name, ..]
                if verb.eq_ignore_ascii_case("install") || verb.eq_ignore_ascii_case("load") =>
            {
                name
            }
            _ => continue,
        };
        let name = named.trim_matches(|c| c == '\'' || c == '"');
        let is_file = name.contains('/') || name.ends_with(".duckdb_extension");
        let built_in = BUILT_IN_EXTENSIONS
            .iter()
            .any(|b| b.eq_ignore_ascii_case(name));
        if is_file || !built_in {
            return Some(name.to_string());
        }
    }
    None
}

/// `sql` with its `--` and `/* */` comments taken out, and its quoted text kept as written, so a
/// comment's words are not read as a statement and a path in quotes is not cut at a `--`.
fn without_comments(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                out.push(c);
                for d in chars.by_ref() {
                    out.push(d);
                    if d == c {
                        break;
                    }
                }
            }
            '-' if chars.peek() == Some(&'-') => {
                for d in chars.by_ref() {
                    if d == '\n' {
                        out.push('\n');
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
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leading_assignment_is_not_the_program() {
        assert_eq!(program_of("FOO=1 /usr/local/bin/uv run x.py"), "uv");
        assert_eq!(program_of("  "), "sh");
    }

    #[test]
    fn a_commented_load_is_not_read_and_a_quoted_file_is() {
        assert_eq!(sql_extension_beyond("-- LOAD h3\nSELECT 1"), None);
        assert_eq!(sql_extension_beyond("/* INSTALL h3; */ SELECT 1"), None);
        assert_eq!(
            sql_extension_beyond("LOAD '/x/finetype.duckdb_extension';"),
            Some("/x/finetype.duckdb_extension".to_string())
        );
        assert_eq!(
            sql_extension_beyond("FORCE INSTALL h3 FROM community;"),
            Some("h3".to_string())
        );
        assert_eq!(sql_extension_beyond("INSTALL Spatial; LOAD spatial;"), None);
    }
}
