//! The operations `arc` holds, reached through the child runner.
//!
//! brightfield draws no operation's panel from its own knowledge. It asks
//! `arc operation describe <long name>` what the operation reads and takes,
//! and records a step with `arc operation record`, so the words, the
//! comparisons and the refusals are arc's. Each call runs the runner with an
//! argument list ([`arc_with_args`]), reads JSON from stdout, and treats
//! stderr as the refusal.
//!
//! **Save records through arc.** Each view applied in the grid's foot row is
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
                })
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
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(OperationDescription {
        long_name: text(&value, "long_name"),
        summary: text(&value, "summary"),
        reads,
        parameters,
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

/// Record `operation` as a step named `name` on the table `on`, in the
/// Protocol in `dir`, with `args` as `--arg key=value`. arc runs nothing.
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
    let pairs: Vec<String> = args.iter().map(|(k, v)| format!("{k}={v}")).collect();
    let mut argv: Vec<&str> = vec!["operation", "record", operation, "--on", on, "--name", name];
    for pair in &pairs {
        argv.push("--arg");
        argv.push(pair);
    }
    // No `--dir`: the child stands in `dir`, and arc's default is its working
    // directory. Passing `dir` too would resolve a relative one twice.
    let out = arc_with_args(runner, dir, &argv);
    if out.succeeded {
        Ok(out.stdout.trim().to_string())
    } else {
        Err(refusal(&out.stderr))
    }
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
    /// The step is out of `arcform.yaml`. `left_behind` says why its SQL file could not be
    /// deleted, naming it, when it could not: the removal stands even then
    /// (`a_sql_file_that_cannot_be_deleted_is_left_with_a_notice`).
    Removed {
        /// The file left behind and why, when its delete failed.
        left_behind: Option<String>,
    },
}

/// The sentence a removal says when `arcform.yaml` changed under it, so that arc's delete did not
/// take out exactly the step it was asked for.
pub const CHANGED_MEANWHILE: &str = "The Protocol changed while the step was being removed, so it was not removed. Save again to remove it from the Protocol as it is now.";

/// **Remove the step `name` from the Protocol in `dir`, through arc**: `arc edit-protocol delete
/// steps[<i>]`, which takes the step's entry out of `arcform.yaml` with the comment flush above it,
/// leaves every other byte as it was, checks the result still loads before it writes, and keeps the
/// state it replaces in the Protocol's history. arc refuses a result a later step's `depends_on`
/// breaks, and the folder is then left as it was.
///
/// arc deletes by position, and an agent may write `arcform.yaml` between the read that finds the
/// step's position and arc's own read. So the manifest is read immediately before arc is asked,
/// and read again after: the delete stands only when exactly the step `name` is gone and every
/// other step read before is still there. Otherwise no file is deleted, the manifest is put back
/// as arc found it ([`restore_before_delete`]), and the refusal is [`CHANGED_MEANWHILE`].
///
/// Once the delete stands, the SQL file the step `name` named in the manifest read before is
/// deleted, unless a step still in the manifest names it too. A file outside the Protocol's folder
/// is never deleted: only a path inside it, written relative to it, is the step's own.
///
/// # Errors
///
/// Why the manifest could not be read, arc's refusal without its `error: ` prefix, or
/// [`CHANGED_MEANWHILE`].
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
    let left: Option<Vec<&str>> = after
        .as_ref()
        .map(|m| m.steps.iter().map(|s| s.name.as_str()).collect());
    if left.as_deref() != Some(expected.as_slice()) {
        if !restore_before_delete(runner, dir) {
            eprintln!(
                "{}: arc's delete took out another step than {name}, and the manifest could not be put back",
                manifest_path.display()
            );
        }
        return Err(CHANGED_MEANWHILE.to_string());
    }
    let Some(after) = after else {
        return Err(CHANGED_MEANWHILE.to_string());
    };
    let model = before.steps[at]
        .sql
        .as_deref()
        .filter(|model| !after.steps.iter().any(|s| s.sql.as_deref() == Some(*model)));
    let left_behind = model
        .and_then(|model| own_file(dir, model))
        .and_then(|file| match std::fs::remove_file(&file) {
            Ok(()) => None,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => Some(format!("{}: {e}", file.display())),
        });
    Ok(StepRemoval::Removed { left_behind })
}

/// **Put `arcform.yaml` back as arc found it before its last edit**, from the checkpoint that edit
/// recorded in the Protocol's history: the entry before the newest, the newest being the save of
/// what arc wrote. Put back through `arc history restore`, which checkpoints what it replaces.
///
/// Only while the file is still what arc wrote: a file written again since is someone's newer
/// work, and is left as it is. Answers whether the manifest was put back.
fn restore_before_delete(runner: &Runner, dir: &Path) -> bool {
    let out = arc_with_args(runner, dir, &["history", "list", "--json"]);
    if !out.succeeded {
        return false;
    }
    let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(&out.stdout) else {
        return false;
    };
    let ids: Vec<&str> = entries
        .iter()
        .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
        .collect();
    let [.., earlier, newest] = ids[..] else {
        return false;
    };
    let shown = arc_with_args(runner, dir, &["history", "show", newest]);
    let now = std::fs::read_to_string(dir.join("arcform.yaml")).unwrap_or_default();
    if !shown.succeeded || shown.stdout != now {
        return false;
    }
    arc_with_args(runner, dir, &["history", "restore", earlier]).succeeded
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
