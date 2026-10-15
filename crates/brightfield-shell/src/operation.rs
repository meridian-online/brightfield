//! The operations `arc` holds, reached through the child runner.
//!
//! brightfield draws no operation's panel from its own knowledge. It asks
//! `arc operation describe <long name>` what the operation reads and takes,
//! and records a step with `arc operation record`, so the words, the
//! comparisons and the refusals are arc's. Each call runs the runner with an
//! argument list ([`arc_with_args`]), reads JSON from stdout, and treats
//! stderr as the refusal.

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
