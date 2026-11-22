//! The operations `arc` holds, reached through the child runner.
//!
//! brightfield draws no operation's panel from its own knowledge. It asks
//! `arc operation describe <long name>` what the operation reads and takes,
//! and records a step with `arc operation record`, so the words, the
//! comparisons and the refusals are arc's. Each call runs the runner with an
//! argument list ([`arc_with_args`]), reads JSON from stdout, and treats
//! stderr as the refusal.
//!
//! **A description is read whole.** [`describe`] keeps each value's shape
//! ([`ValueShape`]: a kind of SQL, a column's name, a list of them, a whole
//! number with its bounds, one word of a few, a path, or plain text), whether
//! it is required, which values are given together or not at all, and, of each
//! table a step reads, whether it is one table or a list of them. A form reads
//! these and decides nothing of its own about what an operation takes.
//!
//! **Save records through arc.** [`record_step`] runs `arc operation record`
//! for a [`StepRequest`]: the operation, the table it is recorded on, each
//! other table it reads under the role arc names for it (`--read
//! <role>=<table>`, a list role once per table, in order), and its values. arc
//! writes notes to stderr even when it records, and they are kept with what it
//! said ([`Recorded`]). [`record`] is the same call for a step that reads one
//! table. Each view applied in the grid's foot row is
//! recorded by [`record`] as `filter-rows` for the condition, then `sort-rows`
//! for the order on the condition's new table, so the same operation writes
//! the same file however it is recorded, and the Protocol's history shows who
//! recorded what. `--on` is the table the node in view makes (for a data file,
//! the load's table, not the step). Save asks for no name: the window names
//! each step by its effect, `<table>_<effect>_by_<column>`, with the column
//! DuckDB finds in the condition (see [`crate::generated::step_name_for`]).
//! A refusal is shown in arc's words and leaves the view applied and unsaved;
//! a part arc already recorded stays recorded, and the next Save records the
//! rest on the table it must go on.
//!
//! **Save also records the statements kept from the palette**, each through
//! [`record_sql`] (`arc sql record --name <name> -- <sql>`) under the name it
//! was kept with, in the order the views and statements were made. The
//! statement comes after `--`, which ends arc's options, so arc takes it as the
//! statement whatever text it opens with. arc writes a statement it reads as
//! an operation as that operation's generated step, and any other as
//! `models/NN_<name>.sql`: `CREATE OR REPLACE TABLE "<name>" AS` on its first
//! line, the statement as typed after it, and a final newline when it had
//! none.
//!
//! **A typed statement is asked of arc too.** [`recognise`] runs
//! `arc sql recognise <sql>` and reports whether arc reads the statement as a
//! `filter-rows` (with its table and condition as typed) or as another
//! kind of statement, so the window leaves the judging of SQL to arc.
//!
//! **A description is fetched once per operation for the session**, off the
//! UI thread ([`Descriptions`]), because it cannot change while arc's pin is
//! fixed. With no runner, or a describe that fails, the foot row still asks
//! and the operation panel says why it has nothing to show.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

use crate::run::{arc_with_args, Runner};

/// What `arc operation describe` says an operation is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationDescription {
    /// The operation's long name, as `arc operation list` prints it.
    pub long_name: String,
    /// One sentence saying what the operation does.
    pub summary: String,
    /// What the step reads: the table and the column.
    pub reads: Vec<Role>,
    /// What the operation takes, in the order of `serde_json::Map`: by key
    /// unless the build enables its `preserve_order` feature, which this crate
    /// does not ask for.
    pub parameters: Vec<Parameter>,
    /// The values arc takes together or not at all: each key with the keys it
    /// needs beside it, in key order (the schema's `dependentRequired`).
    pub needed_together: Vec<(String, Vec<String>)>,
}

/// One thing a step reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Role {
    /// The role's name, such as `table`.
    pub name: String,
    /// What kind of thing it is, such as `table` or `column`.
    pub kind: String,
    /// Whether the step needs it.
    pub required: bool,
    /// arc's sentence for it.
    pub description: String,
    /// Whether the role takes several, given in order, and not one.
    pub list: bool,
    /// The fewest a list role takes; 0 when arc names no floor.
    pub min_items: usize,
}

/// The kind of SQL a value is, as arc's `x-kind` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlKind {
    /// A condition, as after `WHERE` or `ON`.
    Condition,
    /// An `ORDER BY` clause.
    Order,
    /// One expression.
    Expression,
    /// A list of expressions, as after `SELECT` or `GROUP BY`.
    SelectList,
    /// A list of expressions with no alias, as after `PARTITION BY`.
    Partition,
    /// A type's name.
    Type,
}

/// The shape of a value an operation takes, read from its JSON Schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueShape {
    /// SQL of one kind, recorded as written.
    Sql(SqlKind),
    /// One column's name: a name, not SQL, so it is given without quotes.
    Column,
    /// Several columns' names, which the command line joins with `,`.
    Columns,
    /// A whole number, with the bounds arc gives.
    Integer {
        /// The least it may be.
        minimum: Option<i64>,
        /// The most it may be.
        maximum: Option<i64>,
    },
    /// One word of those arc lists.
    Choice {
        /// The words, in arc's order.
        words: Vec<String>,
        /// The word arc takes when none is given.
        default: Option<String>,
    },
    /// A path or a URL to a file.
    Path,
    /// Text arc gives no kind.
    Text,
}

/// One value an operation takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parameter {
    /// The key `--arg` takes, such as `where`.
    pub key: String,
    /// arc's sentence for it.
    pub description: String,
    /// The words arc offers for it: `x-comparisons` or `x-directions`, in
    /// arc's order. Empty when it offers none.
    pub offered: Vec<Offered>,
    /// What the value is.
    pub shape: ValueShape,
    /// Whether the step needs it.
    pub required: bool,
}

/// A word arc offers for a value, with the sign it is written as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offered {
    /// The word, such as `is not` or `ascending`.
    pub word: String,
    /// The sign it is written as, such as `!=` or `asc`.
    pub sign: String,
}

/// arc's refusal from its stderr: the text without its `error: ` prefix and
/// without surrounding whitespace.
fn refusal(stderr: &str) -> String {
    let trimmed = stderr.trim();
    trimmed
        .strip_prefix("error: ")
        .unwrap_or(trimmed)
        .to_string()
}

/// A directory the child may stand in for calls that read no Protocol:
/// `operation list` and `operation describe` ignore it, but a child needs one.
fn neutral_dir() -> std::path::PathBuf {
    std::env::temp_dir()
}

/// The long name of each operation arc holds.
///
/// # Errors
///
/// arc's refusal, or why the runner could not be started or its answer read.
pub fn list_operations(runner: &Runner) -> Result<Vec<String>, String> {
    let out = arc_with_args(runner, &neutral_dir(), &["operation", "list", "--json"]);
    if !out.succeeded {
        return Err(refusal(&out.stderr));
    }
    let value: serde_json::Value = serde_json::from_str(&out.stdout)
        .map_err(|e| format!("arc's list of operations did not read as JSON: {e}"))?;
    let entries = value
        .as_array()
        .ok_or("arc's list of operations was not an array")?;
    Ok(entries
        .iter()
        .filter_map(|e| e.get("long_name")?.as_str().map(str::to_string))
        .collect())
}

fn text(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn offered_in(property: &serde_json::Value) -> Vec<Offered> {
    ["x-comparisons", "x-directions"]
        .iter()
        .filter_map(|key| property.get(key)?.as_array())
        .flatten()
        .map(|o| Offered {
            word: text(o, "word"),
            sign: text(o, "sign"),
        })
        .collect()
}

/// A whole number of `property` under `key`. arc writes `limit-rows`' ceiling as
/// 9223372036854775807, which is `i64::MAX`; a bound past an `i64` is read as
/// no bound.
fn whole(property: &serde_json::Value, key: &str) -> Option<i64> {
    property.get(key).and_then(serde_json::Value::as_i64)
}

/// The shape a property's schema gives its value: its `enum` first, then its
/// `type` with its `x-kind`, and an array by its items' `x-kind`. A string
/// whose `x-kind` this build does not know is [`ValueShape::Text`], so a kind
/// a later arc adds is still typed and recorded as written.
fn shape_of(property: &serde_json::Value) -> ValueShape {
    if let Some(words) = property.get("enum").and_then(serde_json::Value::as_array) {
        return ValueShape::Choice {
            words: words
                .iter()
                .filter_map(|w| w.as_str().map(str::to_string))
                .collect(),
            default: property
                .get("default")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
        };
    }
    let kind = |value: &serde_json::Value| {
        value
            .get("x-kind")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    match property.get("type").and_then(serde_json::Value::as_str) {
        Some("integer") => ValueShape::Integer {
            minimum: whole(property, "minimum"),
            maximum: whole(property, "maximum"),
        },
        Some("array") => match property.get("items").and_then(kind).as_deref() {
            Some("column") => ValueShape::Columns,
            _ => ValueShape::Text,
        },
        _ => match kind(property).as_deref() {
            Some("condition") => ValueShape::Sql(SqlKind::Condition),
            Some("order") => ValueShape::Sql(SqlKind::Order),
            Some("expression") => ValueShape::Sql(SqlKind::Expression),
            Some("select-list") => ValueShape::Sql(SqlKind::SelectList),
            Some("partition") => ValueShape::Sql(SqlKind::Partition),
            Some("type") => ValueShape::Sql(SqlKind::Type),
            Some("column") => ValueShape::Column,
            Some("path") => ValueShape::Path,
            _ => ValueShape::Text,
        },
    }
}

/// The strings of the array under `key`, or none.
fn strings(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|i| i.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// What `long_name` reads and takes, as `arc operation describe` says.
///
/// Run with the system temp directory as its working directory, since the call
/// reads no Protocol.
///
/// # Errors
///
/// arc's stderr without its `error: ` prefix when it refuses (an unknown name),
/// or why the runner could not be started or its answer read.
pub fn describe(runner: &Runner, long_name: &str) -> Result<OperationDescription, String> {
    let out = arc_with_args(
        runner,
        &neutral_dir(),
        &["operation", "describe", long_name],
    );
    if !out.succeeded {
        return Err(refusal(&out.stderr));
    }
    let value: serde_json::Value = serde_json::from_str(&out.stdout)
        .map_err(|e| format!("arc's description of {long_name} did not read as JSON: {e}"))?;
    let reads = value
        .get("reads")
        .and_then(serde_json::Value::as_array)
        .map(|roles| {
            roles
                .iter()
                .map(|r| Role {
                    name: text(r, "name"),
                    kind: text(r, "kind"),
                    required: r
                        .get("required")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                    description: text(r, "description"),
                    list: r
                        .get("list")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                    min_items: r
                        .get("min_items")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|n| usize::try_from(n).ok())
                        .unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default();
    let schema = value.get("parameters").unwrap_or(&serde_json::Value::Null);
    let required = strings(schema, "required");
    let needed_together = schema
        .get("dependentRequired")
        .and_then(serde_json::Value::as_object)
        .map(|needs| {
            needs
                .iter()
                .map(|(key, _)| (key.clone(), strings(&schema["dependentRequired"], key)))
                .collect()
        })
        .unwrap_or_default();
    let parameters = value
        .pointer("/parameters/properties")
        .and_then(serde_json::Value::as_object)
        .map(|props| {
            props
                .iter()
                .map(|(key, p)| Parameter {
                    key: key.clone(),
                    description: text(p, "description"),
                    offered: offered_in(p),
                    shape: shape_of(p),
                    required: required.contains(key),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(OperationDescription {
        long_name: text(&value, "long_name"),
        summary: text(&value, "summary"),
        reads,
        parameters,
        needed_together,
    })
}

/// What arc makes of a typed SQL statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recognised {
    /// A filter of one table: the table it reads and the condition as typed.
    FilterRows {
        /// The table the statement reads.
        on: String,
        /// The `WHERE` condition, as arc echoes it.
        condition: String,
    },
    /// An operation brightfield does not hand to the foot row, or none.
    Other,
}

/// Ask arc whether `sql` is an operation it holds.
///
/// Run with the system temp directory as its working directory, since the call
/// reads no Protocol.
///
/// # Errors
///
/// arc's refusal without its `error: ` prefix (text DuckDB cannot parse), why
/// the runner could not be started or its answer read, or that a `filter-rows`
/// answer lacked its table or condition.
pub fn recognise(runner: &Runner, sql: &str) -> Result<Recognised, String> {
    let out = arc_with_args(runner, &neutral_dir(), &["sql", "recognise", sql]);
    if !out.succeeded {
        return Err(refusal(&out.stderr));
    }
    let value: serde_json::Value = serde_json::from_str(&out.stdout)
        .map_err(|e| format!("arc's reading of the statement did not read as JSON: {e}"))?;
    match value.get("operation").and_then(serde_json::Value::as_str) {
        Some("filter-rows") => {
            let on = value.get("on").and_then(serde_json::Value::as_str);
            let condition = value
                .pointer("/arguments/where")
                .and_then(serde_json::Value::as_str);
            match (on, condition) {
                (Some(on), Some(condition)) => Ok(Recognised::FilterRows {
                    on: on.to_string(),
                    condition: condition.to_string(),
                }),
                _ => Err(
                    "arc read the statement as filter-rows without its table or condition"
                        .to_string(),
                ),
            }
        }
        _ => Ok(Recognised::Other),
    }
}

/// One step asked of `arc operation record`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepRequest {
    /// The operation's long name.
    pub long_name: String,
    /// The table the step is recorded on (`--on`); `None` for an operation
    /// that reads no table, as `read-file` does.
    pub on: Option<String>,
    /// Each other table the step reads, as `(role, table)`. A list role is
    /// given once per table, in the order arc is to read them.
    pub reads: Vec<(String, String)>,
    /// The values, as `(key, value)`. A list of columns is its names joined by
    /// `,`, arc's command-line form.
    pub args: Vec<(String, String)>,
}

/// What arc said when it recorded a step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    /// arc's stdout, trimmed.
    pub said: String,
    /// The lines of arc's stderr that open with `note: `, without that prefix,
    /// in arc's order.
    pub notes: Vec<String>,
}

/// The notes in `stderr`: each line that opens with `note: `, without it.
fn notes_in(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .filter_map(|line| line.trim().strip_prefix("note: "))
        .map(str::to_string)
        .collect()
}

/// Record `request` as a step named `name` in the Protocol in `dir`: `arc
/// operation record <long name> [--on <table>] [--read <role>=<table>]… --name
/// <name> [--arg <key>=<value>]…`. arc runs nothing.
///
/// # Errors
///
/// arc's stderr without its `error: ` prefix when it refuses, with the folder
/// untouched (`a_refused_request_returns_arcs_words_and_writes_nothing`), or
/// why the runner could not be started.
pub fn record_step(
    runner: &Runner,
    dir: &Path,
    name: &str,
    request: &StepRequest,
) -> Result<Recorded, String> {
    let reads: Vec<String> = request
        .reads
        .iter()
        .map(|(role, table)| format!("{role}={table}"))
        .collect();
    let pairs: Vec<String> = request
        .args
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    let mut argv: Vec<&str> = vec!["operation", "record", &request.long_name];
    if let Some(on) = &request.on {
        argv.push("--on");
        argv.push(on);
    }
    for read in &reads {
        argv.push("--read");
        argv.push(read);
    }
    argv.push("--name");
    argv.push(name);
    for pair in &pairs {
        argv.push("--arg");
        argv.push(pair);
    }
    // No `--dir`: the child stands in `dir`, and arc's default is its working
    // directory. Passing `dir` too would resolve a relative one twice.
    let out = arc_with_args(runner, dir, &argv);
    if out.succeeded {
        Ok(Recorded {
            said: out.stdout.trim().to_string(),
            notes: notes_in(&out.stderr),
        })
    } else {
        Err(refusal(&out.stderr))
    }
}

/// Record `operation` as a step named `name` on the table `on`, in the
/// Protocol in `dir`, with `args` as `--arg key=value`: [`record_step`] for a
/// step that reads one table. arc runs nothing.
///
/// # Errors
///
/// arc's stderr without its `error: ` prefix when it refuses, with the folder
/// untouched, or why the runner could not be started.
pub fn record(
    runner: &Runner,
    dir: &Path,
    operation: &str,
    on: &str,
    name: &str,
    args: &[(&str, &str)],
) -> Result<String, String> {
    let request = StepRequest {
        long_name: operation.to_string(),
        on: Some(on.to_string()),
        reads: Vec::new(),
        args: args
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
    };
    record_step(runner, dir, name, &request).map(|recorded| recorded.said)
}

/// Record the statement `sql` as the step `name` at the end of the Protocol in
/// `dir`, as `arc sql record` writes it. arc runs nothing.
///
/// # Errors
///
/// arc's stderr without its `error: ` prefix when it refuses, with the folder
/// untouched, or why the runner could not be started.
pub fn record_sql(runner: &Runner, dir: &Path, sql: &str, name: &str) -> Result<String, String> {
    // No `--dir`, for the reason `record` gives. The statement follows `--`,
    // so text that opens with `-` is not read as a flag.
    let out = arc_with_args(runner, dir, &["sql", "record", "--name", name, "--", sql]);
    if out.succeeded {
        Ok(out.stdout.trim().to_string())
    } else {
        Err(refusal(&out.stderr))
    }
}

/// What [`remove_step`] did to the Protocol.
#[derive(Debug, PartialEq, Eq)]
pub enum StepRemoval {
    /// The step was not in `arcform.yaml`, such as one an agent took out meanwhile, and nothing
    /// was written.
    Gone,
    /// `arcform.yaml` changed while the step was being removed, so the steps read after arc's
    /// delete are not the steps read before it less the step, and the manifest could not be put
    /// back as arc found it. No file was deleted. `missing` names the steps of the read before that
    /// the read after does not hold, in their earlier order, or is `None` when the read after does
    /// not load.
    NotPutBack {
        /// The steps missing now compared with the read before, when the read after loads.
        missing: Option<Vec<String>>,
    },
    /// The step is out of `arcform.yaml`, and no other step was added or taken out. `left_behind`
    /// says why its SQL file could not be deleted, naming it, when it could not: the removal stands
    /// even then (`a_sql_file_that_cannot_be_deleted_is_left_with_a_notice`).
    Removed {
        /// The file left behind and why, when its delete failed.
        left_behind: Option<String>,
    },
}

/// The sentence a removal says when `arcform.yaml` changed under it, so that arc's delete did not
/// take out exactly the step it was asked for, and the manifest was put back as arc found it.
pub const CHANGED_MEANWHILE: &str = "The Protocol changed while the step was being removed, so it was not removed. Save again to remove it from the Protocol as it is now.";

/// **Remove the step `name` from the Protocol in `dir`, through arc**: `arc edit-protocol delete
/// steps[<i>]`, which takes the step's entry out of `arcform.yaml` with the comment flush above it,
/// leaves every other byte as it was, checks the result still loads before it writes, and keeps the
/// state it replaces in the Protocol's history. arc refuses a result a later step's `depends_on`
/// breaks, and the folder is then left as it was.
///
/// arc deletes by position, and an agent may write `arcform.yaml` between the read that finds the
/// step's position and arc's own read, or after arc's write. So the manifest is read immediately
/// before arc is asked, and read again after, and there are three outcomes:
///
/// - **Removed.** The steps read after are exactly the steps read before, in order, less `name`:
///   [`StepRemoval::Removed`] (`removing_a_typed_sql_step_is_a_draft_until_save`). The SQL file
///   `name` named in the read before is deleted, unless a step still in the manifest names it too.
///   A file outside the Protocol's folder is never deleted: only a path inside it, written relative
///   to it, is the step's own.
/// - **Refused and put back.** Any other read after, when arc's history holds exactly two entries
///   new since before the delete and the file is byte-identical to the newer of them, which is
///   arc's save of what it wrote. The manifest is restored from the older, arc's checkpoint of what
///   it read, which holds any write that landed before arc read
///   (`a_removal_is_not_applied_when_the_manifest_changes_under_it`). No file is deleted.
/// - **Refused and not put back.** Any other read after, when the history does not show that or
///   the file has been written since arc's save: [`StepRemoval::NotPutBack`], with the steps
///   missing now (`an_agent_write_after_the_delete_is_not_put_back`). No file is deleted.
///
/// # Errors
///
/// Why the manifest could not be read before the delete, arc's refusal without its `error: `
/// prefix, or [`CHANGED_MEANWHILE`] when the delete was refused and the manifest was put back.
pub fn remove_step(runner: &Runner, dir: &Path, name: &str) -> Result<StepRemoval, String> {
    let manifest_path = dir.join("arcform.yaml");
    let read = || -> Result<brightfield_protocol::Manifest, String> {
        let text = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
        brightfield_protocol::parse_manifest_str(&text)
            .map_err(|e| format!("{}: {e}", manifest_path.display()))
    };
    let before = read()?;
    let Some(at) = before.steps.iter().position(|s| s.name == name) else {
        return Ok(StepRemoval::Gone);
    };
    // The history as it stands, so the entries arc's delete adds are known for its own.
    let history_before = history_ids(runner, dir);
    let path = format!("steps[{at}]");
    let out = arc_with_args(runner, dir, &["edit-protocol", "delete", &path]);
    if !out.succeeded {
        return Err(refusal(&out.stderr));
    }
    let after = read().ok();
    let expected: Vec<&str> = before
        .steps
        .iter()
        .map(|s| s.name.as_str())
        .filter(|n| *n != name)
        .collect();
    if let Some(after) = &after {
        let left: Vec<&str> = after.steps.iter().map(|s| s.name.as_str()).collect();
        if left == expected {
            let model = before.steps[at].sql.as_deref();
            let still_named = after.steps.iter().any(|s| s.sql.as_deref() == model);
            return Ok(StepRemoval::Removed {
                left_behind: delete_model(dir, model.filter(|_| !still_named)),
            });
        }
    }
    if restore_before_delete(runner, dir, history_before.as_deref()) {
        return Err(CHANGED_MEANWHILE.to_string());
    }
    let missing = after.map(|after| {
        before
            .steps
            .iter()
            .filter(|s| !after.steps.iter().any(|a| a.name == s.name))
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
    });
    eprintln!(
        "{}: the Protocol changed while {name} was being removed, and it could not be put back",
        manifest_path.display()
    );
    Ok(StepRemoval::NotPutBack { missing })
}

/// Delete the SQL file `model` names inside `dir`, when it names one: why it could not be, naming
/// it, when the delete failed. A file already gone is no failure.
fn delete_model(dir: &Path, model: Option<&str>) -> Option<String> {
    let file = own_file(dir, model?)?;
    match std::fs::remove_file(&file) {
        Ok(()) => None,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => Some(format!("{}: {e}", file.display())),
    }
}

/// The ids of the Protocol's history entries, oldest first, or `None` when the history cannot be
/// read.
fn history_ids(runner: &Runner, dir: &Path) -> Option<Vec<String>> {
    let out = arc_with_args(runner, dir, &["history", "list", "--json"]);
    if !out.succeeded {
        return None;
    }
    let entries = serde_json::from_str::<Vec<serde_json::Value>>(&out.stdout).ok()?;
    Some(
        entries
            .iter()
            .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
            .map(str::to_owned)
            .collect(),
    )
}

/// **Put `arcform.yaml` back as arc found it before the delete**, through `arc history restore`,
/// which checkpoints what it replaces. Answers whether the manifest was put back.
///
/// It restores when the history shows the delete's own two entries and no other new one
/// ([`delete_entries`], `a_wrong_step_removed_is_put_back_when_the_history_is_full`), and the file is still byte-identical to the newer of them, arc's save of
/// what it wrote: any other history or file means someone else wrote too, and nothing is restored.
/// The restore is from the older, arc's checkpoint of what it read.
fn restore_before_delete(runner: &Runner, dir: &Path, before: Option<&[String]>) -> bool {
    let Some(before) = before else {
        return false;
    };
    let Some(ids) = history_ids(runner, dir) else {
        return false;
    };
    let Some((checkpoint, save)) = delete_entries(before, &ids) else {
        return false;
    };
    let saved = arc_with_args(runner, dir, &["history", "show", save]);
    if !saved.succeeded {
        return false;
    }
    let Ok(now) = std::fs::read(dir.join("arcform.yaml")) else {
        return false;
    };
    if now != saved.stdout.as_bytes() {
        return false;
    }
    arc_with_args(runner, dir, &["history", "restore", checkpoint]).succeeded
}

/// **The two history entries a single edit added**, older then newer, when the history read
/// before it was `before` and is `ids` now: exactly two ids are new, they are the last two, and the
/// ids before them are a suffix of `before`. arc keeps a bounded history and prunes it from the
/// oldest end, so the entries an edit pushed out are missing from the front, never from the middle.
/// `None` when the history shows another edit's entries as well, or one new entry, or an entry gone
/// from the middle (`any_other_history_shows_no_single_edit`).
fn delete_entries<'a>(before: &[String], ids: &'a [String]) -> Option<(&'a str, &'a str)> {
    let [kept @ .., older, newer] = ids else {
        return None;
    };
    let new = |id: &String| !before.contains(id);
    (new(older) && new(newer) && before.ends_with(kept)).then_some((older, newer))
}

/// The file `relative` names inside `dir`, when it is a path inside it written relative to it:
/// no root, no drive, and no `..`.
fn own_file(dir: &Path, relative: &str) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let path = Path::new(relative);
    path.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        .then(|| dir.join(path))
}

/// Where one description stands, as [`Descriptions::get`] reports it.
#[derive(Debug, PartialEq, Eq)]
pub enum DescriptionState<'a> {
    /// The child has been asked and has not answered. A caller draws nothing
    /// for it yet and asks again on a later frame.
    Pending,
    /// arc's description.
    Ready(&'a OperationDescription),
    /// Why there is none: arc's refusal, or why the runner could not be
    /// started. Kept for the session, so a missing runner is not started again.
    Failed(&'a str),
}

enum Entry {
    Asked(Receiver<Result<OperationDescription, String>>),
    Answered(Result<OperationDescription, String>),
}

/// The descriptions fetched this session, one per operation.
///
/// `arc` does not change what it says of an operation while the pin is fixed,
/// so each is asked once. A failed describe is kept too, with its message.
///
/// The ask runs on a worker thread, because [`Descriptions::get`] is called once
/// a frame and a runner that hangs must not freeze the window. The wake
/// given to [`Descriptions::with_wake`] is called when an answer arrives, so a
/// window can ask egui for a repaint; with none, callers poll.
/// A child that hangs stays [`DescriptionState::Pending`] while the window goes on drawing.
#[derive(Default)]
pub struct Descriptions {
    by_name: BTreeMap<String, Entry>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl std::fmt::Debug for Descriptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Descriptions")
            .field("asked", &self.by_name.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Descriptions {
    /// A cache that calls `wake` from the worker when an answer arrives.
    #[must_use]
    pub fn with_wake(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            by_name: BTreeMap::new(),
            wake: Some(Arc::new(wake)),
        }
    }

    /// The state of `long_name`'s description, starting the ask on the first
    /// call (`descriptions_are_pending_then_ready_and_a_failure_is_kept`).
    pub fn get(&mut self, runner: &Runner, long_name: &str) -> DescriptionState<'_> {
        if !self.by_name.contains_key(long_name) {
            let (tx, rx) = mpsc::channel();
            let runner = runner.clone();
            let name = long_name.to_string();
            let wake = self.wake.clone();
            std::thread::spawn(move || {
                let _ = tx.send(describe(&runner, &name));
                if let Some(wake) = wake {
                    wake();
                }
            });
            self.by_name.insert(long_name.to_string(), Entry::Asked(rx));
        }
        let entry = self.by_name.get_mut(long_name).expect("just inserted");
        if let Entry::Asked(rx) = entry {
            match rx.try_recv() {
                Ok(answer) => *entry = Entry::Answered(answer),
                Err(TryRecvError::Empty) => return DescriptionState::Pending,
                Err(TryRecvError::Disconnected) => {
                    *entry = Entry::Answered(Err(format!(
                        "the ask for {long_name} stopped without an answer"
                    )));
                }
            }
        }
        match entry {
            Entry::Answered(Ok(d)) => DescriptionState::Ready(d),
            Entry::Answered(Err(e)) => DescriptionState::Failed(e),
            Entry::Asked(_) => DescriptionState::Pending,
        }
    }
}

#[cfg(test)]
mod removal_tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| (*n).to_string()).collect()
    }

    #[test]
    fn a_single_edits_two_entries_are_found_after_the_history_is_pruned() {
        let before = ids(&["a", "b", "c", "d"]);
        // Nothing pruned.
        assert_eq!(
            delete_entries(&before, &ids(&["a", "b", "c", "d", "x", "y"])),
            Some(("x", "y"))
        );
        // The bound reached: the oldest two pruned to make room.
        assert_eq!(
            delete_entries(&before, &ids(&["c", "d", "x", "y"])),
            Some(("x", "y"))
        );
        // A history that started empty.
        assert_eq!(delete_entries(&[], &ids(&["x", "y"])), Some(("x", "y")));
    }

    #[test]
    fn any_other_history_shows_no_single_edit() {
        let before = ids(&["a", "b", "c", "d"]);
        // Another edit's entries too, with and without pruning.
        assert_eq!(
            delete_entries(&before, &ids(&["a", "b", "c", "d", "p", "q", "x", "y"])),
            None
        );
        assert_eq!(
            delete_entries(&before, &ids(&["c", "d", "p", "q", "x", "y"])),
            None
        );
        // One entry new, or none.
        assert_eq!(delete_entries(&before, &ids(&["b", "c", "d", "y"])), None);
        assert_eq!(delete_entries(&before, &before), None);
        // An entry gone from the middle is not pruning.
        assert_eq!(
            delete_entries(&before, &ids(&["a", "c", "d", "x", "y"])),
            None
        );
    }
}
