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
//! the Protocol folder, or else lists each table as unclear with no joins. [`Dovetail`] runs
//! dovetail as a child of this binary, one child per pass.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use brightfield_engine::ProfileOutcome;
use brightfield_ossie::{
    merge_discovery, read, Dataset, DatasetMeta, DecidedBy, Field, FieldMeta, Model, ModelMeta,
    Role, SkippedSource, LINEAGE,
};

use crate::own_files::{self, OwnProtocol};
use crate::run::{Runner, RUNNER_ENV};

/// The model's file in the Protocol folder.
pub const MODEL_FILE: &str = "model.ossie.yaml";

/// The hidden folder in the Protocol where a discoverer that runs as a child writes what each pass
/// proposes.
pub const PROPOSALS_DIR: &str = ".discovery";

/// The environment variable that makes a process of this binary dovetail instead of a window.
/// The binary runs dovetail as a child of itself because dovetail's entry point reads the
/// process's argv, and the window's process has argv of its own.
pub const DOVETAIL_ENV: &str = "BRIGHTFIELD_RUN_AS_DOVETAIL";

/// Become dovetail if [`DOVETAIL_ENV`] is set: run its command line and end the process, so `main`
/// opens no window and reads no file to open (`the_binary_serves_as_dovetail_under_its_variable`).
///
/// Dovetail reads the argv itself and returns an exit code. A success ends this process with 0 and
/// any other code with 1: the parent reads only success or failure, and the reason from stderr.
pub fn serve_as_dovetail_if_asked() {
    if std::env::var_os(DOVETAIL_ENV).is_some() {
        let code = if dovetail::cli_main() == std::process::ExitCode::SUCCESS {
            0
        } else {
            1
        };
        std::process::exit(code);
    }
}

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

    /// Ask the pass to stop: what the pass's owner calls when it supersedes the pass or lets it
    /// go. A pass that has already answered is not affected.
    pub fn request(&self) {
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

/// Dovetail, run as a child of this binary over the Protocol folder, one child per pass: it reads
/// the data packages in `packages/` and the raw tables, and writes what the pass proposes. The
/// child never writes `model.ossie.yaml`, which holds the analyst's decisions and is Brightfield's
/// alone to write; it is handed that file as `--base`, so the analyst's decisions carry through.
///
/// The child reads a copy of the Protocol's database, taken under [`own_files::database_lock`],
/// which the own-files flow's writes to that database hold, so none of them is part way through
/// when the copy is taken. The lock is let go before the child starts, so a run waits on a pass
/// while it copies and not while its child runs. Each pass has
/// files of its own, a copy named for the run and an out file beside it, so a superseded child
/// being killed and its successor write separate files. Once the child exits 0, its out file is
/// renamed to the pass's [`proposal_file`], and that is the pass's answer. A pass removes its
/// proposal file before it runs, and a sample pass the full one too, so a proposal on disk is
/// always the latest pass's own. The full pass is given the sample proposal to verify when there is
/// one, and removes it once it has run, unless a newer sample pass has started since, whose
/// proposal is its successor's to verify.
///
/// A pass may be superseded while its child runs, by a new pass or by the pane letting it go.
/// [`Discovery::discover_until`] then kills the child, waits for it, removes the run's files and
/// answers that it was superseded (`a_cancelled_pass_kills_its_child_and_leaves_no_files`), and a
/// pass that is no longer the latest of its kind for the folder leaves its successor's proposal
/// where it is.
///
/// A join's warning has one home, `RelationshipMeta.warning`. Dovetail's full pass writes its own
/// there, and it stands until the next run of the Protocol, when Brightfield's measurement over the
/// silver tables overwrites it: that measurement is over the tables the charts read, so it is the
/// one to trust when both exist. Dovetail's next full pass writes its own again.
#[derive(Debug, Clone)]
pub struct Dovetail {
    program: PathBuf,
}

impl Dovetail {
    /// Dovetail run by `runner`'s program, the binary that serves as dovetail under
    /// [`DOVETAIL_ENV`].
    #[must_use]
    pub fn new(runner: &Runner) -> Self {
        Self {
            program: runner.program().to_path_buf(),
        }
    }
}

impl Discovery for Dovetail {
    fn discover(&self, protocol: &OwnProtocol, pass: Pass) -> Result<Model, String> {
        self.discover_until(protocol, pass, &Cancel::default())
    }

    fn discover_until(
        &self,
        protocol: &OwnProtocol,
        pass: Pass,
        cancel: &Cancel,
    ) -> Result<Model, String> {
        let dir = &protocol.dir;
        let started = begin_pass(dir, pass);
        let answered = self.run_child(protocol, pass, started, cancel);
        if pass == Pass::Full {
            let passes = passes();
            // A sample pass started since this one has a newer proposal, which is its successor's
            // to verify; any older one is removed whatever this pass answered, so a later full pass
            // never verifies a stale sample.
            if passes
                .get(&(dir.clone(), Pass::Sample))
                .copied()
                .unwrap_or(0)
                == started.sample
            {
                let _ = std::fs::remove_file(proposal_file(dir, Pass::Sample));
            }
        }
        answered
    }
}

/// What a pass that was cancelled, or overtaken by a newer pass of its kind, answers.
const SUPERSEDED: &str = "the discovery pass was superseded";

/// How often a pass looks at its child, to see whether it has exited or the pass was cancelled.
const POLL: Duration = Duration::from_millis(50);

/// Numbers each pass's run files, across every Protocol this process discovers.
static RUNS: AtomicU64 = AtomicU64::new(0);

/// The latest generation of each pass of each Protocol folder, taken as a pass starts. Held while
/// a pass removes or renames a proposal file, so what it checks is still so when it acts.
fn passes() -> MutexGuard<'static, HashMap<(PathBuf, Pass), u64>> {
    static PASSES: OnceLock<Mutex<HashMap<(PathBuf, Pass), u64>>> = OnceLock::new();
    PASSES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// The generations a pass started with: its own, and the sample pass's latest at the time.
#[derive(Debug, Clone, Copy)]
struct Started {
    own: u64,
    sample: u64,
}

/// Take the next generation of `pass` for `dir` and remove the proposals it makes stale: its own,
/// and for a sample pass the full one, which was folded from an older sample.
fn begin_pass(dir: &Path, pass: Pass) -> Started {
    let mut passes = passes();
    let own = passes.entry((dir.to_path_buf(), pass)).or_insert(0);
    *own += 1;
    let own = *own;
    let _ = std::fs::remove_file(proposal_file(dir, pass));
    if pass == Pass::Sample {
        let _ = std::fs::remove_file(proposal_file(dir, Pass::Full));
    }
    let sample = passes
        .get(&(dir.to_path_buf(), Pass::Sample))
        .copied()
        .unwrap_or(0);
    Started { own, sample }
}

/// A run's own files: the database copy and its `.wal`, the child's out file and the `.writing`
/// file beside it. Dropping it removes them, however the run ended.
struct RunFiles {
    copy: PathBuf,
    out: PathBuf,
}

impl RunFiles {
    fn new(dir: &Path, pass: Pass) -> Self {
        // The process id keeps one process's run files apart from another's: a Brightfield that
        // quit during a pass leaves its files, and perhaps a child still reading them, behind.
        let n = RUNS.fetch_add(1, Ordering::SeqCst);
        let id = format!("{}-{n}", std::process::id());
        let proposals = dir.join(PROPOSALS_DIR);
        let pass = match pass {
            Pass::Sample => "sample",
            Pass::Full => "full",
        };
        Self {
            copy: proposals.join(format!("run-{id}.duckdb")),
            out: proposals.join(format!("run-{id}.{pass}.ossie.yaml")),
        }
    }
}

impl Drop for RunFiles {
    fn drop(&mut self) {
        for file in [
            self.copy.clone(),
            wal_of(&self.copy),
            self.out.clone(),
            beside(&self.out, ".writing"),
        ] {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// `path` with `suffix` added to its file name.
fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// The write-ahead log DuckDB keeps beside the database `db`.
fn wal_of(db: &Path) -> PathBuf {
    beside(db, ".wal")
}

impl Dovetail {
    /// Copy the database, run the child over the copy until it exits or `cancel` is requested, and
    /// read what it proposed.
    fn run_child(
        &self,
        protocol: &OwnProtocol,
        pass: Pass,
        started: Started,
        cancel: &Cancel,
    ) -> Result<Model, String> {
        let dir = &protocol.dir;
        let proposals = dir.join(PROPOSALS_DIR);
        std::fs::create_dir_all(&proposals).map_err(|e| format!("{}: {e}", proposals.display()))?;
        let files = RunFiles::new(dir, pass);
        copy_database(dir, &files.copy)?;
        if cancel.requested() {
            return Err(SUPERSEDED.to_string());
        }

        let mut command = Command::new(&self.program);
        command
            .env(DOVETAIL_ENV, "1")
            .env("NO_COLOR", "1")
            .env_remove(RUNNER_ENV)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .arg("model")
            .arg("--db")
            .arg(&files.copy)
            .arg("--packages")
            .arg(dir.join("packages"))
            .arg("--pass")
            .arg(match pass {
                Pass::Sample => "sample",
                Pass::Full => "full",
            })
            .arg("--name")
            .arg(dir.file_name().unwrap_or(dir.as_os_str()))
            .arg("--out")
            .arg(&files.out);
        let base = dir.join(MODEL_FILE);
        if base.is_file() {
            command.arg("--base").arg(base);
        }
        let sample = proposal_file(dir, Pass::Sample);
        if pass == Pass::Full && sample.is_file() {
            command.arg("--verify").arg(sample);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("{}: {e}", self.program.display()))?;
        // Read on a thread of its own, so a child that writes more than a pipe holds is never
        // blocked waiting for the parent to read.
        let stderr = child.stderr.take().map(|mut pipe| {
            std::thread::spawn(move || {
                let mut text = String::new();
                let _ = pipe.read_to_string(&mut text);
                text
            })
        });
        let exited = wait_unless_cancelled(&mut child, cancel);
        let stderr = stderr
            .and_then(|reader| reader.join().ok())
            .unwrap_or_default();
        let status = match exited {
            Some(Ok(status)) => status,
            Some(Err(e)) => return Err(format!("{}: {e}", self.program.display())),
            None => return Err(SUPERSEDED.to_string()),
        };
        if !status.success() {
            let reason = stderr.trim();
            return Err(if reason.is_empty() {
                format!("dovetail ended without a reason ({status})")
            } else {
                reason.to_string()
            });
        }

        let text = std::fs::read_to_string(&files.out)
            .map_err(|e| format!("{}: {e}", files.out.display()))?;
        // Read before it is renamed into place, so a proposal Brightfield cannot read never becomes
        // the pass's proposal file, nor the sample a later full pass verifies.
        let model = read(&text).map_err(|e| format!("{}: {e}", files.out.display()))?;
        let proposal = proposal_file(dir, pass);
        {
            let passes = passes();
            let latest = passes.get(&(dir.clone(), pass)).copied().unwrap_or(0);
            if cancel.requested() || latest != started.own {
                return Err(SUPERSEDED.to_string());
            }
            std::fs::rename(&files.out, &proposal)
                .map_err(|e| format!("{}: {e}", proposal.display()))?;
        }
        Ok(model)
    }
}

/// Copy the Protocol's database in `dir`, and its write-ahead log when there is one, to `copy`,
/// while none of the own-files flow's writes to it is running.
fn copy_database(dir: &Path, copy: &Path) -> Result<(), String> {
    let db = arc::spec::protocol_db_path(dir).map_err(|e| e.to_string())?;
    // A log left beside the copy's name would be replayed into the copy as if it were the
    // database's own.
    let stale = wal_of(copy);
    match std::fs::remove_file(&stale) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!("{}: {e}", stale.display()));
        }
        _ => {}
    }
    let _writing = own_files::database_lock();
    std::fs::copy(&db, copy).map_err(|e| format!("{}: {e}", db.display()))?;
    let wal = wal_of(&db);
    if wal.is_file() {
        std::fs::copy(&wal, wal_of(copy)).map_err(|e| format!("{}: {e}", wal.display()))?;
    }
    Ok(())
}

/// Wait for `child` to exit, looking at it each [`POLL`], and answer how it exited; or, once
/// `cancel` is requested, kill it, wait for it so it is not left a zombie, and answer `None`.
fn wait_unless_cancelled(
    child: &mut Child,
    cancel: &Cancel,
) -> Option<std::io::Result<std::process::ExitStatus>> {
    loop {
        if cancel.requested() {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        match child.try_wait() {
            Ok(Some(status)) => return Some(Ok(status)),
            Ok(None) => std::thread::sleep(POLL),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Some(Err(e));
            }
        }
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
                        other: serde_json::Map::new(),
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
                    role_scores: None,
                    folded_into: None,
                    fold_column: None,
                    combined_into: None,
                    combination: None,
                    excluded: false,
                    decided_by: DecidedBy::Dovetail,
                    source_file: source_file_of(&table.file, &protocol.dir),
                    source_part: table.part.clone(),
                    rows,
                    other: serde_json::Map::new(),
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
            pass: None,
            typing: None,
            sample_rows: None,
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
            other: serde_json::Map::new(),
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
