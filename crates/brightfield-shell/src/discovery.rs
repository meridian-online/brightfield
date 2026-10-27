//! Getting a model of an analyst's own files: the seam between the own-files flow and whatever
//! discovers how the tables relate.
//!
//! A [`Discovery`] reads a [`OwnProtocol`] (its data packages and raw tables) and answers a
//! [`Model`]. It runs in two passes, a quick one over a sample and a complete one over the whole table,
//! and each is folded into the model already held so what the analyst has decided survives both.
//! [`sample_pass`] answers the sample's model at once, and [`FullPass`] runs the full pass on a
//! worker, whose answer the holder folds into its model as that stands when the answer lands,
//! edits made in the meantime included. A pass that is superseded is told to stop, and its answer
//! goes nowhere. [`discover_model`] runs both passes in turn on the calling thread.
//!
//! Brightfield writes `model.ossie.yaml` itself, whole ([`write_model_file`]), so a pass that
//! reads it finds the file as a save left it (`full_pass_reads_the_file_the_sample_save_wrote`).
//! A discoverer run as a child writes its proposal to a file of its own ([`proposal_file`]).
//!
//! [`StandIn`] is what runs until a real discoverer is wired in: it returns the model already in
//! the Protocol folder, or else lists each table as unclear with no joins. [`Dovetail`] is where
//! dovetail will run, once it is built in.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

use brightfield_engine::ProfileOutcome;
use brightfield_ossie::{
    merge_discovery, read, Dataset, DatasetMeta, DecidedBy, Field, FieldMeta, Model, ModelMeta,
    Role, SkippedSource, LINEAGE,
};

use crate::own_files::OwnProtocol;

/// The model's file in the Protocol folder.
pub const MODEL_FILE: &str = "model.ossie.yaml";

/// The hidden folder in the Protocol where a discoverer that runs as a child writes what each pass
/// proposes.
pub const PROPOSALS_DIR: &str = ".discovery";

/// The file a child discoverer writes `pass`'s proposal to, in the Protocol folder `dir`:
/// `.discovery/sample.ossie.yaml` or `.discovery/full.ossie.yaml`.
#[must_use]
pub fn proposal_file(dir: &Path, pass: Pass) -> PathBuf {
    let name = match pass {
        Pass::Sample => "sample.ossie.yaml",
        Pass::Full => "full.ossie.yaml",
    };
    dir.join(PROPOSALS_DIR).join(name)
}

/// Write `text` as the model file in the Protocol folder `dir`: to a file beside it first, then
/// renamed over it, so a reader finds the old file or the new one and never part of either.
///
/// # Errors
///
/// The reason the file could not be written or renamed, with its path.
pub fn write_model_file(dir: &Path, text: &str) -> Result<(), String> {
    let path = dir.join(MODEL_FILE);
    let draft = dir.join(format!(".{MODEL_FILE}.writing"));
    std::fs::write(&draft, text).map_err(|e| format!("{}: {e}", draft.display()))?;
    std::fs::rename(&draft, &path).map_err(|e| format!("{}: {e}", path.display()))
}

/// How much of the data a pass reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    /// A quick look at a sample, so a first model can be shown early.
    Sample,
    /// Every row, which settles what the sample only suggested.
    Full,
}

/// Something that can propose a model for a Protocol's tables.
pub trait Discovery {
    /// The model `pass` proposes for `protocol`.
    ///
    /// # Errors
    ///
    /// The reason the pass could not run or its output could not be read.
    fn discover(&self, protocol: &OwnProtocol, pass: Pass) -> Result<Model, String>;

    /// [`Self::discover`], for a pass that may be superseded while it runs: a discoverer whose
    /// pass takes a while checks `cancel` and stops when it is asked to, and one that answers at
    /// once need not, which is what this does unless a discoverer says otherwise. What a
    /// cancelled pass answers is thrown away.
    ///
    /// # Errors
    ///
    /// As [`Self::discover`] says.
    fn discover_until(
        &self,
        protocol: &OwnProtocol,
        pass: Pass,
        cancel: &Cancel,
    ) -> Result<Model, String> {
        let _ = cancel;
        self.discover(protocol, pass)
    }
}

/// Asks a running pass to stop: set when the pass it belongs to is superseded or let go.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// Whether the pass has been asked to stop.
    #[must_use]
    pub fn requested(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    fn request(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Returns the model at `<protocol>/model.ossie.yaml` when there is one, and otherwise every table
/// as an `unclear` dataset with no joins.
pub struct StandIn;

impl Discovery for StandIn {
    fn discover(&self, protocol: &OwnProtocol, _pass: Pass) -> Result<Model, String> {
        match on_disk(protocol)? {
            Some(model) => Ok(model),
            None => Ok(unclear_tables(protocol)),
        }
    }
}

/// Dovetail, run as a child process over the Protocol folder: it reads the data packages in
/// `packages/` and the raw tables, and writes what the pass proposes to that pass's own file,
/// [`proposal_file`] (`.discovery/full.ossie.yaml` for the full pass). Brightfield reads that file
/// as the pass's answer and folds it into the model; the child never writes `model.ossie.yaml`,
/// which holds the analyst's decisions and is Brightfield's alone to write.
///
/// A pass may be superseded while its child runs, by a new pass or by the pane letting it go. The
/// child is then killed: the real implementation overrides [`Discovery::discover_until`] and kills
/// the child once `cancel` is requested, and its proposal file is not read.
///
/// It is not built into this Brightfield yet. It waits on dovetail's DuckDB 2.0 release, which
/// is also to bring Ossie output, table roles and the sample and full passes, and until then each
/// pass answers why it cannot run. Nothing calls it yet.
///
/// Left for the real implementation to settle: a join's warning has one home,
/// `RelationshipMeta.warning`, which the pane's own measuring over the silver tables writes too.
/// A warning dovetail writes and one Brightfield measures would overwrite each other there, so
/// which one stands, or how both are kept, is to be decided when dovetail writes warnings.
pub struct Dovetail;

impl Discovery for Dovetail {
    fn discover(&self, _protocol: &OwnProtocol, _pass: Pass) -> Result<Model, String> {
        Err(
            "dovetail discovery is not built into this Brightfield yet: it needs dovetail's \
             DuckDB 2.0 release"
                .to_string(),
        )
    }
}

/// The model already in the Protocol folder, if there is one.
fn on_disk(protocol: &OwnProtocol) -> Result<Option<Model>, String> {
    let file = protocol.dir.join(MODEL_FILE);
    if !file.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    read(&text)
        .map(Some)
        .map_err(|e| format!("{}: {e}", file.display()))
}

/// Every table of `protocol` as an `unclear` dataset: its columns and FineType labels from the
/// profile, no keys and no joins.
fn unclear_tables(protocol: &OwnProtocol) -> Model {
    let datasets = protocol
        .sources
        .tables
        .iter()
        .map(|table| {
            let profile = protocol.profiles.iter().find(|p| p.name == table.name);
            let (columns, rows) = match profile.map(|p| &p.outcome) {
                Some(ProfileOutcome::Profiled { columns, row_count }) => {
                    (columns.as_slice(), Some(*row_count))
                }
                _ => (&[][..], None),
            };
            let fields = columns
                .iter()
                .filter(|c| !LINEAGE.contains(&c.name.as_str()))
                .map(|c| Field {
                    name: c.name.clone(),
                    expression_sql: c.name.clone(),
                    dialects: Vec::new(),
                    dialects_only: false,
                    extras: Default::default(),
                    datatype: None,
                    meta: FieldMeta {
                        finetype: c
                            .semantic
                            .is_usable()
                            .then(|| c.semantic.label())
                            .flatten()
                            .map(str::to_string),
                        decided_by: DecidedBy::Dovetail,
                    },
                    other_extensions: Vec::new(),
                })
                .collect();
            Dataset {
                name: table.name.clone(),
                description: None,
                metrics: Vec::new(),
                extras: Default::default(),
                source: format!("raw_{}", table.name),
                primary_key: Vec::new(),
                fields,
                meta: DatasetMeta {
                    role: Role::Unclear,
                    role_evidence: Vec::new(),
                    folded_into: None,
                    excluded: false,
                    decided_by: DecidedBy::Dovetail,
                    source_file: source_file_of(&table.file, &protocol.dir),
                    source_part: table.part.clone(),
                    rows,
                },
                other_extensions: Vec::new(),
            }
        })
        .collect();
    Model {
        name: protocol
            .dir
            .file_name()
            .map_or_else(|| "files".to_string(), |n| n.to_string_lossy().into_owned()),
        description: None,
        metrics: Vec::new(),
        extras: Default::default(),
        datasets,
        relationships: Vec::new(),
        meta: ModelMeta {
            generator: "brightfield stand-in".to_string(),
            rejected: Vec::new(),
            skipped_sources: protocol
                .sources
                .skipped
                .iter()
                .map(|(path, reason)| SkippedSource {
                    path: path.to_string_lossy().into_owned(),
                    reason: reason.clone(),
                })
                .collect(),
        },
        other_extensions: Vec::new(),
    }
}

/// `file` relative to the Protocol folder when it lies inside it, else absolute.
fn source_file_of(file: &Path, dir: &Path) -> String {
    file.strip_prefix(dir)
        .unwrap_or(file)
        .to_string_lossy()
        .into_owned()
}

/// The full pass of a discovery, running on a worker thread until it answers. Dropping it, as
/// replacing it with a new pass does, asks the pass to stop and throws its answer away.
pub struct FullPass {
    answer: Receiver<Result<Model, String>>,
    cancel: Cancel,
}

impl FullPass {
    /// Run `discovery`'s full pass over `protocol` on a worker thread.
    #[must_use]
    pub fn start(discovery: Box<dyn Discovery + Send>, protocol: &OwnProtocol) -> Self {
        let (send, answer) = mpsc::channel();
        let protocol = protocol.clone();
        let cancel = Cancel::default();
        let asked = cancel.clone();
        std::thread::spawn(move || {
            let answered = discovery.discover_until(&protocol, Pass::Full, &asked);
            if !asked.requested() {
                // The holder may have stopped waiting, and then the answer goes nowhere.
                let _ = send.send(answered);
            }
        });
        Self { answer, cancel }
    }

    /// The full pass's model once it has answered, without waiting; `None` while it runs.
    pub fn try_take(&self) -> Option<Result<Model, String>> {
        match self.answer.try_recv() {
            Ok(answer) => Some(answer),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(STOPPED.to_string())),
        }
    }

    /// The full pass's model, waiting for it.
    ///
    /// # Errors
    ///
    /// The reason the pass failed.
    pub fn wait(self) -> Result<Model, String> {
        self.answer
            .recv()
            .unwrap_or_else(|_| Err(STOPPED.to_string()))
    }
}

impl Drop for FullPass {
    fn drop(&mut self) {
        self.cancel.request();
    }
}

/// What a full pass whose worker ended without answering says.
const STOPPED: &str = "the full discovery pass stopped without answering";

/// The sample pass of `discovery` over `protocol`, folded into `held`: the model the caller holds,
/// or when it holds none, the one on disk if there is one and otherwise the sample pass's own.
///
/// # Errors
///
/// The reason the pass failed, or the model on disk could not be read.
pub fn sample_pass(
    discovery: &dyn Discovery,
    protocol: &OwnProtocol,
    held: Option<&Model>,
) -> Result<Model, String> {
    let sample = discovery.discover(protocol, Pass::Sample)?;
    let held = match held {
        Some(model) => model.clone(),
        None => on_disk(protocol)?.unwrap_or_else(|| sample.clone()),
    };
    Ok(merge_discovery(&held, &sample))
}

/// The model for `protocol`: a sample pass, then a full pass, each folded into the model held so
/// far, which is the one on disk if there is one and otherwise the sample pass's own. What the
/// analyst has decided in between is kept as it is (see [`merge_discovery`]).
///
/// The passes run one after the other on the calling thread; [`FullPass`] runs the full pass on
/// a worker.
///
/// # Errors
///
/// The reason either pass failed.
pub fn discover_model(protocol: &OwnProtocol) -> Result<Model, String> {
    discover_with(&StandIn, protocol)
}

/// [`discover_model`] with the discoverer named.
///
/// # Errors
///
/// The reason either pass failed.
pub fn discover_with(discovery: &dyn Discovery, protocol: &OwnProtocol) -> Result<Model, String> {
    let after_sample = sample_pass(discovery, protocol, None)?;
    let full = discovery.discover(protocol, Pass::Full)?;
    Ok(merge_discovery(&after_sample, &full))
}
