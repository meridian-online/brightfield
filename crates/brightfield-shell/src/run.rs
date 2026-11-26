//! Running the Protocol a data file opened as, and finding the record the run
//! left beside it.
//!
//! # Who runs it
//!
//! `arc` does, and brightfield still executes no SQL of its own. What changed
//! is that the window can ask for a run rather than only read one that someone
//! started from a terminal.
//!
//! **The runner is a child process of this same binary**, and the reason is
//! what `arc` publishes. Its library target exports `arc::spec` alone; the engine — runner, DuckDB bridge, run contract — is private. The one
//! other item at its root, `arc::cli_main`, is its binary's entry point, and
//! each of the three things it does rules out calling it on a worker thread of
//! a window: it parses the **process's** argv, `arc run` reads the
//! **process's** working directory to find the manifest, and an error ends the
//! **process** with `std::process::exit`. So `main` checks [`RUNNER_ENV`]
//! before any window work and, when it is set, hands the process to
//! `arc::cli_main` — the three-line shape `arc`'s own binary has — and
//! [`Run::begin`] starts that child with `run` as its argument and the
//! Protocol's directory as its working directory.
//!
//! What that buys over spawning an `arc` found on the `PATH`: the runner that
//! writes the record is built from the same pinned rev as the loader that
//! reads it, so the two cannot disagree about the contract's shape. What it
//! costs: one more process per run, and a window that is also, under one
//! environment variable, a command-line tool.
//!
//! # Which DuckDB runs the steps
//!
//! `arc run` does not embed DuckDB: it executes each SQL step by spawning a
//! DuckDB executable, the one [`ENGINE_ENV`] names or else `duckdb` on the
//! search path. A packaged build carries the official CLI beside its own
//! executable (`scripts/package.sh` stages it), and [`staged_engine_beside`]
//! finds it from the runner program — in the live app that is
//! [`std::env::current_exe`], or the Mac App Store bundle's helper copy of it,
//! by way of [`Runner::this_binary`]. When it is
//! there the child is told it in [`ENGINE_ENV`], so a stranger's run needs no
//! second install. When it is not — a `cargo run`, a test harness — the
//! variable is left as the parent had it, so an inherited [`ENGINE_ENV`] or the
//! search path answers.
//!
//! # Where the record is
//!
//! Where `arc run` writes it: `<dir>/build/.arcform/runs/<run_id>.json`, with
//! `<dir>` the Protocol's own directory — for a data file, the file's
//! directory, where [`crate::one_step::OneStepProtocol::save_to`] writes the
//! spec. Brightfield writes nothing under `build/` itself. [`records_newest_first`]
//! is the read side, and it is what a launch reads as well as what a finished
//! run reads, so the strip after a run and the strip on the next launch come
//! out of one function.
//!
//! # What is kept only for the session
//!
//! The run's log. `arc` writes the contract and a status stream and no log
//! file, so what the child printed is carried back in [`Finished::log`] and
//! lives as long as the document. A relaunch reads the record's outcome and
//! not the words the run printed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use brightfield_protocol::ContractView;

/// The environment variable that turns this binary into `arc`.
///
/// Set on the child [`Run::begin`] starts, and read by `main` before it reads
/// its own arguments. The name says what it does rather than hiding it: a
/// person who finds it in a process listing should be able to tell what that
/// process is.
pub const RUNNER_ENV: &str = "BRIGHTFIELD_RUN_AS_ARC";

/// The environment variable `arc run` reads for the DuckDB executable it runs
/// SQL steps with: `DUCKDB_BIN_ENV` in arc's engine module, which is private,
/// so the name is spelled here. Set, it is final — arc refuses a value that is
/// not an executable file rather than falling back to the search path.
pub const ENGINE_ENV: &str = "ARC_DUCKDB_BIN";

/// The DuckDB CLI a packaged build staged beside `program`, when there is one.
///
/// Two layouts, the two `scripts/package.sh` produces: the tarball puts
/// `engine/duckdb` beside the `brightfield` executable, and the app puts it at
/// `Contents/Helpers/duckdb` while the executable sits in `Contents/MacOS/`.
/// `None` when neither is a file — a `cargo run` or a test harness, whose
/// binary sits in `target/` with nothing staged beside it
/// (`the_staged_engine_is_found_in_either_packaged_layout_and_nowhere_else`).
#[must_use]
pub fn staged_engine_beside(program: &Path) -> Option<PathBuf> {
    let dir = program.parent()?;
    [
        dir.join("engine").join("duckdb"),
        dir.join("../Helpers/duckdb"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

/// The file name of the helper copy of Brightfield's executable that a Mac App
/// Store bundle carries in `Contents/Helpers/`, which runs as arc and as
/// dovetail ([`Runner::for_executable`]). `scripts/package-store.sh` lays it out
/// under this name.
pub const HELPER_NAME: &str = "brightfield-helper";

/// The environment variable arc reads for FineType's command line, which its
/// `finetype` operator and its profiling run: `finetype_bin` in arc's private
/// `finetype_cli` module, so the name is spelled here. Unset, arc runs
/// `finetype` from the search path.
pub const FINETYPE_ENV: &str = "FINETYPE_BIN";

/// The `finetype` helper an app bundle carries, when there is one.
///
/// Only the app layout, `Contents/Helpers/finetype` beside `Contents/MacOS/`:
/// the sandboxed Store build carries it because it cannot run a `finetype` the
/// analyst installed, and the full edition carries none and keeps arc's search
/// of the path. `None` when it is not a file
/// (`without_a_bundled_finetype_the_child_keeps_what_it_inherits`).
#[must_use]
pub fn staged_finetype_beside(program: &Path) -> Option<PathBuf> {
    let candidate = program.parent()?.join("../Helpers/finetype");
    candidate.is_file().then_some(candidate)
}

/// The query both DuckDBs answer for `--check-engine`: the release and the
/// commit it was built from, so two builds of one version string still differ.
pub const ENGINE_VERSION_QUERY: &str = "SELECT library_version, source_id FROM pragma_version()";

/// The variable DuckDB's 2.0 CLI reads to decide whether to answer in its agent
/// output mode (markdown results, JSON errors), which it otherwise switches on
/// by itself when an agent's variable is set and stdout is not a terminal. `0`
/// keeps the output the probe parses, as arc sets it for its own steps.
const AGENT_MODE_ENV: &str = "DUCKDB_AGENT_MODE";

/// What `--check-engine` found beside a program.
#[derive(Debug, PartialEq, Eq)]
pub enum EngineCheck {
    /// The staged engine ran the version query and answered with the DuckDB
    /// this binary links.
    Matches { engine: PathBuf, version: String },
    /// An engine is staged and it did not run, or it is another DuckDB.
    Broken { engine: PathBuf, reason: String },
    /// Nothing is staged beside the program.
    Absent,
}

/// The DuckDB this binary links, as `<library_version>,<source_id>`: the form
/// the CLI prints for [`ENGINE_VERSION_QUERY`] under `-csv -noheader`.
///
/// # Errors
///
/// When an in-memory database cannot be opened or does not answer the query.
pub fn linked_duckdb_version() -> Result<String, String> {
    let conn = duckdb::Connection::open_in_memory().map_err(|e| e.to_string())?;
    conn.query_row(ENGINE_VERSION_QUERY, [], |row| {
        Ok(format!(
            "{},{}",
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?
        ))
    })
    .map_err(|e| e.to_string())
}

/// Run the engine staged beside `program` and compare the DuckDB it reports
/// with `linked`, the one this binary carries ([`linked_duckdb_version`]).
///
/// The engine is started with an empty environment, a fresh `HOME` and working
/// directory, and an empty init file, which DuckDB's CLI reads in place of
/// `~/.duckdbrc`
/// (`the_staged_engine_runs_with_an_empty_environment_and_its_own_home`).
/// So what it prints is the engine's answer and not the caller's settings. A run that writes a Protocol and a window that reads it are
/// only one engine if this matches, which is why a mismatch is a failure and
/// not a note.
#[must_use]
pub fn check_staged_engine(program: &Path, linked: &str) -> EngineCheck {
    let Some(engine) = staged_engine_beside(program) else {
        return EngineCheck::Absent;
    };
    let broken = |reason: String| EngineCheck::Broken {
        engine: engine.clone(),
        reason,
    };
    let sealed = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(e) => return broken(format!("cannot make a sealed working directory: {e}")),
    };
    let init = sealed.path().join("empty-init.sql");
    if let Err(e) = std::fs::write(&init, b"") {
        return broken(format!("cannot write an empty init file: {e}"));
    }
    let output = Command::new(&engine)
        .arg("-batch")
        .arg("-noheader")
        .arg("-csv")
        .arg("-init")
        .arg(&init)
        .arg("-c")
        .arg(ENGINE_VERSION_QUERY)
        .env_clear()
        .env("HOME", sealed.path())
        .env(AGENT_MODE_ENV, "0")
        .current_dir(sealed.path())
        .stdin(Stdio::null())
        .output();
    let output = match output {
        Ok(output) => output,
        Err(e) => return broken(format!("could not be started: {e}")),
    };
    if !output.status.success() {
        return broken(format!(
            "exited with {} running the version query: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let reported = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if reported == linked {
        EngineCheck::Matches {
            engine,
            version: reported,
        }
    } else {
        broken(format!(
            "reports DuckDB `{reported}` and this binary links `{linked}`"
        ))
    }
}

/// Hand this process to `arc` if it was started as the runner, and return only
/// if it was not.
///
/// Called first in `main`, ahead of argument parsing, so the child never reads
/// a layout, opens a window or interprets `run` as a file to open. `arc`
/// parses the argv itself and ends the process with its own exit code on an
/// error; a run that succeeds returns here and the process ends with 0.
pub fn serve_as_runner_if_asked() {
    if std::env::var_os(RUNNER_ENV).is_some() {
        arc::cli_main();
        std::process::exit(0);
    }
}

/// The program a run is handed to: a binary whose `main` calls
/// [`serve_as_runner_if_asked`].
///
/// Named rather than assumed, because the process a window is in is not always
/// such a binary. The live window is — [`Runner::this_binary`] — but a test's
/// window lives in the test harness, and a harness started with `run` as its
/// argument runs every test whose name contains *run*. So a window has no
/// runner until one is given to it: `main` gives it this binary, and a suite
/// gives it the `brightfield-shell` binary cargo built for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runner {
    program: PathBuf,
}

impl Runner {
    /// The binary this process is running, which is the runner when this
    /// process is `brightfield-shell`, or its helper copy when the bundle
    /// carries one ([`Self::for_executable`]).
    ///
    /// `None` when the operating system will not say where the executable is.
    #[must_use]
    pub fn this_binary() -> Option<Self> {
        std::env::current_exe().ok().map(Self::for_executable)
    }

    /// The runner for a window running `exe`: the helper copy of `exe` that a
    /// Mac App Store bundle carries at `Contents/Helpers/`[`HELPER_NAME`] when
    /// it is there, and otherwise `exe` itself.
    ///
    /// The Store bundle signs its main executable with the app's entitlements
    /// and every program it starts with only `app-sandbox` and `inherit`, as
    /// Apple documents for a sandboxed app's helpers. The helper is the same
    /// executable, so it serves as arc and dovetail by the same variables, and
    /// it finds the DuckDB CLI, FineType's CLI and FineType's model from its
    /// own path as the main executable does, since `Contents/Helpers/` and
    /// `Contents/MacOS/` sit at the same depth. The full edition carries no
    /// helper and its window starts itself, as before.
    #[must_use]
    pub fn for_executable(exe: PathBuf) -> Self {
        let helper = exe
            .parent()
            .map(|dir| dir.join("../Helpers").join(HELPER_NAME))
            .filter(|helper| helper.is_file());
        Self::at(helper.unwrap_or(exe))
    }

    /// The runner at `program`.
    #[must_use]
    pub fn at(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// The program this runner starts.
    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }
}

/// Where `arc run` writes a run's contract and stream for the Protocol in
/// `dir`.
///
/// Spelled here rather than read from `arc`, because the function that says it
/// is behind `arc`'s private line. `brightfield_protocol::contract`'s module
/// doc names the same path, and
/// `taking_run_writes_the_record_where_arc_writes_it` in
/// `tests/run_control.rs` holds this against what a real run writes.
#[must_use]
pub fn runs_dir(dir: &Path) -> PathBuf {
    dir.join("build").join(".arcform").join("runs")
}

/// The run contracts under `dir`'s [`runs_dir`], newest first.
///
/// Newest by modification time, then by file name: a run id is a timestamp to
/// the second followed by random hex, so two runs in one second sort by the
/// hex and not by which finished last, and the file's own time is what breaks
/// that tie. A directory that is not there is no runs rather than an error.
#[must_use]
pub fn record_paths_newest_first(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(runs_dir(dir)) else {
        return Vec::new();
    };
    let mut records: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| {
            let modified = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            (modified, path)
        })
        .collect();
    records.sort_by(|a, b| b.cmp(a));
    records.into_iter().map(|(_, path)| path).collect()
}

/// The runs of the Protocol named `protocol` recorded under `dir`, newest
/// first, each read into the view the shell draws.
///
/// **Filtered by the Protocol's name**, because a directory is not one
/// Protocol's: two data files side by side each open as a Protocol whose
/// directory is that one directory, and both write their runs into the same
/// `runs/`. A record that will not parse is reported and passed over rather
/// than ending the search, so one damaged file does not hide the run before it.
pub fn records_newest_first<'a>(
    dir: &'a Path,
    protocol: &'a str,
) -> impl Iterator<Item = ContractView> + 'a {
    record_paths_newest_first(dir)
        .into_iter()
        .filter_map(
            move |path| match brightfield_protocol::load_contract(&path) {
                Ok(view) => (view.run.protocol == protocol).then_some(view),
                Err(e) => {
                    eprintln!("passing over the run record {}: {e}", path.display());
                    None
                }
            },
        )
}

/// The file in a Protocol's folder that names, one run id a line, the runs Brightfield started
/// itself to preview the Protocol's sources: the runs on an open, a reload and a model save.
///
/// A file of Brightfield's own beside the manifest, because the record arc writes cannot carry
/// the mark: arc records only the params a manifest declares, so a `--param` the manifest does not
/// declare is not on the record, and the manifest is the analyst's and is not written to mark a
/// run. Kept in the folder rather than in the window, so the mark survives a relaunch and goes with
/// the folder when it is renamed or moved.
pub const PREVIEWS_FILE: &str = ".previews";

/// How many preview run ids [`PREVIEWS_FILE`] keeps: the most recent, so the file stays small
/// however many records arc keeps. A preview whose id has been dropped reads as a whole run again,
/// which takes more than this many previews since the Protocol's last run with their records all
/// still kept.
pub const MAX_PREVIEWS: usize = 200;

/// Mark the run `finished` left in `dir` as a preview, so the door does not read it as the
/// Protocol's run. Ids whose record has gone are dropped as the file is written again. A run that
/// left no record has nothing to mark, and a file that cannot be written is reported, not fatal:
/// the run's tables are made either way.
pub fn mark_preview(dir: &Path, finished: &Finished) {
    let Some(id) = finished
        .record
        .as_deref()
        .and_then(Path::file_stem)
        .map(|stem| stem.to_string_lossy().into_owned())
    else {
        return;
    };
    let runs = runs_dir(dir);
    let mut ids: Vec<String> = preview_ids(dir)
        .into_iter()
        .filter(|kept| runs.join(format!("{kept}.json")).is_file())
        .collect();
    ids.sort();
    if !ids.contains(&id) {
        ids.push(id);
    }
    // Run ids lead with their time, so sorted they run oldest to newest; the oldest go first.
    if ids.len() > MAX_PREVIEWS {
        ids.drain(..ids.len() - MAX_PREVIEWS);
    }
    let file = dir.join(PREVIEWS_FILE);
    if let Err(e) = std::fs::write(&file, ids.join("\n") + "\n") {
        eprintln!("could not mark a preview run in {}: {e}", file.display());
    }
}

/// The ids of the runs in `dir` marked as previews by [`mark_preview`].
#[must_use]
pub fn preview_ids(dir: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(dir.join(PREVIEWS_FILE))
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The paths of the records of whole runs under `dir`, newest first: the records less those
/// [`mark_preview`] marked. A whole run is one the analyst asked for, Run in the window or the run
/// an open from a link starts.
#[must_use]
pub fn whole_run_paths_newest_first(dir: &Path) -> Vec<PathBuf> {
    let previews = preview_ids(dir);
    record_paths_newest_first(dir)
        .into_iter()
        .filter(|path| {
            path.file_stem()
                .is_none_or(|stem| !previews.contains(stem.to_string_lossy().as_ref()))
        })
        .collect()
}

/// What a finished run hands back to the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finished {
    /// The contract this run wrote, when it wrote one.
    ///
    /// `None` when the run ended before `arc` reached its step loop — a
    /// manifest it refused, a runner that would not start — which is the case
    /// where there is no record to read and the log is the whole of the answer.
    /// A run whose step **failed** still writes one, with the outcome `error`.
    pub record: Option<PathBuf>,
    /// What the child printed, standard output then standard error, with the
    /// terminal colour codes `arc` writes taken out.
    pub log: String,
    /// Whether the child exited 0.
    pub succeeded: bool,
}

/// A run on a worker thread.
///
/// The shape [`crate::remote::Fetch`] has, for the reason it has it: the
/// window polls [`Run::take`] once a frame and keeps drawing in between, and
/// the worker wakes the event loop through the callback it was given when the
/// child exits.
pub struct Run {
    dir: PathBuf,
    outcome: Receiver<Finished>,
    done: bool,
}

impl Run {
    /// Start `runner` on the Protocol whose spec is in `dir`.
    ///
    /// The spec has to be on disk already — `arc run` reads `arcform.yaml` from
    /// its working directory, and this writes nothing.
    #[must_use]
    pub fn begin(runner: &Runner, dir: &Path, wake: impl Fn() + Send + 'static) -> Self {
        let (tx, outcome) = mpsc::channel();
        // A Protocol the analyst stopped trusting while it was open runs nothing, whichever part
        // of the window asks: the run ends at once, unrun, saying why.
        // So does one with a step this edition cannot run, whatever its trust.
        // The same refusal as [`crate::trust::refuses_run`], with the manifest read once for both
        // the refusal and the words that give its reason.
        let beyond = crate::beyond_edition::of_folder(dir, crate::edition::current());
        let refused = match &beyond {
            Some(beyond) => Some(format!(
                "{} was not run. {} The full edition runs it: {}",
                dir.display(),
                crate::beyond_edition::sentence(beyond),
                crate::edition::full_edition_url()
            )),
            None if crate::trust::revoked_and_untrusted(dir) => Some(format!(
                "{} is not trusted, so it was not run; trust it from the band over its canvas \
                 once it is opened again",
                dir.display()
            )),
            None => None,
        };
        if let Some(log) = refused {
            tx.send(Finished {
                record: None,
                log,
                succeeded: false,
            })
            .ok();
            wake();
            return Self {
                dir: dir.to_path_buf(),
                outcome,
                done: false,
            };
        }
        let program = runner.program.clone();
        let worker_dir = dir.to_path_buf();
        std::thread::spawn(move || {
            // A failed send is a window that dropped this `Run` — it went home
            // or opened something else. The child has already finished and its
            // record is on disk for whichever window opens this Protocol next.
            tx.send(run_to_completion(&program, &worker_dir)).ok();
            wake();
        });
        Self {
            dir: dir.to_path_buf(),
            outcome,
            done: false,
        }
    }

    /// The directory of the Protocol this run is running.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Wait for the run to finish, and say what it left: for a caller with no
    /// frames to keep drawing, such as a boot that composes once the run has
    /// made its tables.
    #[must_use]
    pub fn wait(mut self) -> Finished {
        if let Some(finished) = self.take() {
            return finished;
        }
        self.outcome.recv().unwrap_or_else(|_| Finished {
            record: None,
            log: format!(
                "the run of {} stopped without an answer",
                self.dir.display()
            ),
            succeeded: false,
        })
    }

    /// The run's result, once there is one — `None` while the child is still
    /// running.
    ///
    /// A disconnected channel with nothing on it is a worker that panicked,
    /// reported as a run that ended without a record rather than left as a
    /// control that reads *running* for the rest of the session.
    pub fn take(&mut self) -> Option<Finished> {
        if self.done {
            return None;
        }
        match self.outcome.try_recv() {
            Ok(finished) => {
                self.done = true;
                Some(finished)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.done = true;
                Some(Finished {
                    record: None,
                    log: format!(
                        "the run of {} stopped without an answer",
                        self.dir.display()
                    ),
                    succeeded: false,
                })
            }
        }
    }
}

/// The child that is `arc` run with `args` in `dir`: this binary under
/// [`RUNNER_ENV`], colour off, no stdin, its history recorded under the way
/// `brightfield`, and the staged engine and `finetype` helper when they sit
/// beside `program`.
fn arc_command(program: &Path, dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(dir)
        .env(RUNNER_ENV, "1")
        .env("NO_COLOR", "1")
        .env(arc::spec::HISTORY_WAY_ENV, "brightfield")
        .stdin(Stdio::null());
    if let Some(engine) = staged_engine_beside(program) {
        command.env(ENGINE_ENV, engine);
    }
    if let Some(finetype) = staged_finetype_beside(program) {
        command.env(FINETYPE_ENV, finetype);
    }
    command
}

/// **The folders a run of the Protocol in `dir` reads and writes**, each once: the Protocol folder,
/// the folder of each source its record of sources names, and the Protocols folder, where arc's
/// outputs for a Protocol made there go. Relative source paths are read against `dir`.
#[must_use]
pub fn folders_of(dir: &Path) -> Vec<PathBuf> {
    let mut folders = vec![dir.to_path_buf()];
    for file in crate::own_files::source_files(dir).unwrap_or_default() {
        let file = if file.is_absolute() {
            file
        } else {
            dir.join(file)
        };
        if let Some(parent) = file.parent() {
            folders.push(parent.to_path_buf());
        }
    }
    folders.push(crate::protocols_dir::root());
    let mut seen: Vec<PathBuf> = Vec::new();
    folders.retain(|f| {
        let new = !seen.contains(f);
        seen.push(f.clone());
        new
    });
    folders
}

/// **Keep access to the folders a child of the Protocol in `dir` uses started until the guard
/// drops**: what a run, a call of `arc` and a join proposal take before they start their child. A
/// sandboxed app's child reads and writes the folders the app holds grants on, as measured on
/// macOS 27.2, so nothing is handed to it; the hold is what keeps the grant it reads through from
/// ending while it reads.
pub fn hold_for(dir: &Path) -> crate::grants::GrantHold<'static> {
    crate::grants::store().hold(&folders_of(dir))
}

/// What one call of `arc` left: its two streams apart, and whether it exited 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArcOutput {
    /// What `arc` printed on stdout, which is where its JSON and its one-line
    /// answers go.
    pub stdout: String,
    /// What `arc` printed on stderr with its colour codes removed, which is
    /// where a refusal goes.
    pub stderr: String,
    /// Whether `arc` exited 0.
    pub succeeded: bool,
}

/// Run `arc` with `args` in `dir` through `runner`, and wait for it.
///
/// Unlike a run, which merges the two streams into one log, this keeps them
/// apart: a caller reads JSON from stdout and shows stderr as the refusal. A
/// runner that cannot be started comes back as a failed output whose stderr
/// says so.
#[must_use]
pub fn arc_with_args(runner: &Runner, dir: &Path, args: &[&str]) -> ArcOutput {
    arc_with_args_and_env(runner, dir, args, &[])
}

/// [`arc_with_args`], with each of `env` set for the child as a variable and
/// its value.
#[must_use]
pub(crate) fn arc_with_args_and_env(
    runner: &Runner,
    dir: &Path,
    args: &[&str],
    env: &[(&str, &Path)],
) -> ArcOutput {
    let _hold = hold_for(dir);
    let mut command = arc_command(runner.program(), dir, args);
    for (variable, value) in env {
        command.env(variable, value);
    }
    match command.output() {
        Ok(output) => ArcOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: without_colour_codes(&String::from_utf8_lossy(&output.stderr)),
            succeeded: output.status.success(),
        },
        Err(e) => ArcOutput {
            stdout: String::new(),
            stderr: format!(
                "could not start the runner {}: {e}",
                runner.program().display()
            ),
            succeeded: false,
        },
    }
}

/// Start the child, wait for it, and say what it left.
///
/// Which record is this run's is decided by **what was not there before**,
/// not by which is newest: a run that ends before its step loop writes no
/// record, and reading the newest one then would report the previous run's
/// outcome as this one's.
fn run_to_completion(program: &Path, dir: &Path) -> Finished {
    let _hold = hold_for(dir);
    let before: BTreeSet<PathBuf> = record_paths_newest_first(dir).into_iter().collect();
    let output = arc_command(program, dir, &["run"]).output();
    let output = match output {
        Ok(output) => output,
        Err(e) => {
            return Finished {
                record: None,
                log: format!("could not start the runner {}: {e}", program.display()),
                succeeded: false,
            }
        }
    };
    let record = record_paths_newest_first(dir)
        .into_iter()
        .find(|path| !before.contains(path));
    let mut log = without_colour_codes(&String::from_utf8_lossy(&output.stdout));
    let stderr = without_colour_codes(&String::from_utf8_lossy(&output.stderr));
    if !stderr.trim().is_empty() {
        if !log.is_empty() && !log.ends_with('\n') {
            log.push('\n');
        }
        log.push_str(&stderr);
    }
    Finished {
        record,
        log,
        succeeded: output.status.success(),
    }
}

/// `text` with its ANSI escape sequences removed.
///
/// `arc` bolds a step's name and colours its tick through a crate that does
/// not read `NO_COLOR` for the bold or the tick, so the codes arrive in piped output
/// anyway; a pane that drew them would print `[1m` beside the step. An escape
/// is `ESC [`, any parameter bytes, and one final byte in `@`..=`~`.
#[must_use]
pub fn without_colour_codes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The live streams, `*.jsonl`, in the [`runs_dir`] of the Protocol in `dir`.
#[must_use]
pub fn streams_in(dir: &Path) -> BTreeSet<PathBuf> {
    let Ok(entries) = std::fs::read_dir(runs_dir(dir)) else {
        return BTreeSet::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .collect()
}

/// The live stream of a run over the Protocol in `dir` that began when the streams there were
/// `earlier`: the newest `*.jsonl` in [`runs_dir`] that is not one of them.
///
/// `arc` names the stream by a run id it chooses itself and does not hand back, so the window
/// finds it as the file that was not there before. Not by time: a file system that keeps
/// modification times to the second or two would date a stream written just after the run began
/// before it. `None` until `arc` has written the file, which it opens as the step loop begins, so
/// a run refused before that loop reads as having no stream
/// (`the_live_stream_is_the_newest_jsonl_not_there_when_the_run_began`).
#[must_use]
pub fn live_stream(dir: &Path, earlier: &BTreeSet<PathBuf>) -> Option<PathBuf> {
    streams_in(dir)
        .into_iter()
        .filter(|path| !earlier.contains(path))
        .map(|path| {
            let modified = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            (modified, path)
        })
        .max()
        .map(|(_, path)| path)
}

/// How far a run under way has got, as its live stream says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunProgress {
    /// The step the stream says is running, when it says one is.
    ///
    /// `arc` writes a step's line when the step finishes, and writes no `running` line, so a
    /// stream from today's `arc` leaves this `None`: the count is what it reports.
    pub running: Option<String>,
    /// How many steps the stream has seen finish, whatever their outcome.
    pub done: usize,
    /// How many steps the Protocol has.
    pub total: usize,
}

/// The progress a stream folded to `state` reports, for a Protocol of `total` steps.
///
/// A step counts as done once its latest line names a state other than `running`. The count is
/// held to `total`, so a stream that names a step the Protocol no longer has does not report more
/// done than there are.
#[must_use]
pub fn progress(state: &brightfield_protocol::stream::StreamState, total: usize) -> RunProgress {
    let running = state
        .steps
        .iter()
        .find(|(_, entry)| entry.state == "running")
        .map(|(step, _)| step.clone());
    let done = state
        .steps
        .values()
        .filter(|entry| entry.state != "running")
        .count()
        .min(total);
    RunProgress {
        running,
        done,
        total,
    }
}

/// When the Protocol last ran, and how that run went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastRun {
    /// The day it ran, as "ran 23 Oct", with the year when it is not this one.
    pub when: String,
    /// How long ago it ran, as "29 days ago".
    pub age: String,
    /// The run's state, as the window draws it.
    pub state: brightfield_workbench::RunState,
}

/// The day a run that finished at `finished_at`, an RFC 3339 time as `arc` records it, ran, and
/// how long before `now` that was; `None` for a time that does not parse.
///
/// The day is in local time: "ran 23 Oct" in `now`'s year, and "ran 23 Oct 2025" otherwise. The
/// age reads "just now" under a minute, then "N minutes ago", "N hours ago" and "N days ago",
/// singular at one. A time after `now`, from a clock that moved back, reads "just now".
#[must_use]
pub fn describe_ran(
    finished_at: &str,
    now: chrono::DateTime<chrono::Local>,
) -> Option<(String, String)> {
    use chrono::Datelike;
    let ran = chrono::DateTime::parse_from_rfc3339(finished_at)
        .ok()?
        .with_timezone(&chrono::Local);
    let when = if ran.year() == now.year() {
        ran.format("ran %-d %b").to_string()
    } else {
        ran.format("ran %-d %b %Y").to_string()
    };
    let seconds = (now - ran).num_seconds().max(0);
    let ago = |n: i64, unit: &str| {
        if n == 1 {
            format!("1 {unit} ago")
        } else {
            format!("{n} {unit}s ago")
        }
    };
    let age = match seconds {
        s if s < 60 => "just now".to_string(),
        s if s < 3_600 => ago(s / 60, "minute"),
        s if s < 86_400 => ago(s / 3_600, "hour"),
        s => ago(s / 86_400, "day"),
    };
    Some((when, age))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Watched redden, three mutations, each alone: the tarball candidate
    /// renamed to `engine/duckdb-cli`, the app candidate to
    /// `../Resources/duckdb`, and the `is_file` filter replaced by
    /// `parent().is_some()`.
    #[test]
    fn the_staged_engine_is_found_in_either_packaged_layout_and_nowhere_else() {
        let tmp = std::env::temp_dir().join(format!("bf-engine-{}", std::process::id()));
        let tar = tmp.join("tarball");
        let app = tmp.join("Brightfield.app/Contents");
        std::fs::create_dir_all(tar.join("engine")).unwrap();
        std::fs::create_dir_all(app.join("MacOS")).unwrap();
        std::fs::create_dir_all(app.join("Helpers")).unwrap();
        std::fs::write(tar.join("engine/duckdb"), b"x").unwrap();
        std::fs::write(app.join("Helpers/duckdb"), b"x").unwrap();
        std::fs::create_dir_all(tmp.join("bare")).unwrap();

        assert_eq!(
            staged_engine_beside(&tar.join("brightfield")),
            Some(tar.join("engine/duckdb"))
        );
        assert_eq!(
            staged_engine_beside(&app.join("MacOS/brightfield")),
            Some(app.join("MacOS/../Helpers/duckdb"))
        );
        assert_eq!(staged_engine_beside(&tmp.join("bare/brightfield")), None);
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// **A window in a bundle that carries the helper copy starts the helper**,
    /// and finds the bundle's DuckDB CLI from it; a window with no helper beside
    /// it, as in the full edition's bundle and a `cargo run`, starts itself.
    #[test]
    fn the_runner_is_the_helper_copy_when_the_bundle_carries_one() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("Brightfield.app/Contents");
        std::fs::create_dir_all(app.join("MacOS")).unwrap();
        std::fs::create_dir_all(app.join("Helpers")).unwrap();
        let exe = app.join("MacOS/brightfield");
        std::fs::write(&exe, b"x").unwrap();
        std::fs::write(app.join("Helpers/duckdb"), b"x").unwrap();

        assert_eq!(Runner::for_executable(exe.clone()).program(), exe);

        std::fs::write(app.join("Helpers").join(HELPER_NAME), b"x").unwrap();
        let runner = Runner::for_executable(exe.clone());
        assert_eq!(
            runner.program(),
            app.join("MacOS/../Helpers").join(HELPER_NAME)
        );
        assert_eq!(
            staged_engine_beside(runner.program()),
            Some(app.join("MacOS/../Helpers/../Helpers/duckdb"))
        );

        let bare = tmp.path().join("target/release/brightfield-shell");
        assert_eq!(Runner::for_executable(bare.clone()).program(), bare);
    }

    /// A tarball-shaped directory whose `engine/duckdb` is `script`, and the
    /// program path the engine is looked up from.
    #[cfg(unix)]
    fn staged(script: &str) -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("engine")).unwrap();
        let engine = dir.path().join("engine/duckdb");
        std::fs::write(&engine, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o755)).unwrap();
        let program = dir.path().join("brightfield");
        (dir, program)
    }

    #[cfg(unix)]
    #[test]
    fn a_staged_engine_reporting_the_linked_duckdb_matches() {
        let (_dir, program) = staged("echo 'v9.9.9,abc123'");
        assert!(matches!(
            check_staged_engine(&program, "v9.9.9,abc123"),
            EngineCheck::Matches { version, .. } if version == "v9.9.9,abc123"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_staged_engine_of_another_version_or_commit_is_broken() {
        let (_dir, program) = staged("echo 'v9.9.9,abc123'");
        for linked in ["v9.9.8,abc123", "v9.9.9,def456"] {
            assert!(matches!(
                check_staged_engine(&program, linked),
                EngineCheck::Broken { reason, .. } if reason.contains("v9.9.9,abc123")
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_staged_engine_that_fails_is_broken_and_says_why() {
        let (_dir, program) = staged("echo 'Catalog Error' >&2; exit 1");
        assert!(matches!(
            check_staged_engine(&program, "v9.9.9,abc123"),
            EngineCheck::Broken { reason, .. } if reason.contains("Catalog Error")
        ));
    }

    /// The engine is asked with nothing inherited: the test process's `PATH`
    /// (cargo's, longer than any shell default) does not reach it, `HOME` is
    /// not the caller's, and the init file it is handed is empty.
    #[cfg(unix)]
    #[test]
    fn the_staged_engine_runs_with_an_empty_environment_and_its_own_home() {
        let (_dir, program) = staged(
            r#"if [ -s "$5" ]; then init=has-content; elif [ -f "$5" ]; then init=empty; else init=missing; fi
echo "${PATH:-unset}|${HOME}|$(pwd -P)|$1 $2 $3 $4|$5|$init|${DUCKDB_AGENT_MODE:-unset}""#,
        );
        let caller_path = std::env::var("PATH").unwrap_or_default();
        let caller_home = std::env::var("HOME").unwrap_or_default();
        let found = check_staged_engine(&program, "never");
        let EngineCheck::Broken { reason, .. } = found else {
            panic!("expected the probe's answer to differ from `never`, got {found:?}");
        };
        let reported = reason
            .split('`')
            .nth(1)
            .expect("the reason quotes what the engine reported");
        let fields: Vec<&str> = reported.split('|').collect();
        // Not "unset": a `/bin/sh` started with no PATH supplies its own default.
        assert_ne!(
            fields[0], caller_path,
            "the caller's PATH reached the engine"
        );
        assert_ne!(fields[1], caller_home, "the engine saw the caller's HOME");
        // Compared by its last component: the directory is gone by now, and
        // `pwd -P` resolves macOS's `/var` to `/private/var` where HOME does not.
        let home_dir = Path::new(fields[1])
            .file_name()
            .expect("HOME names a directory");
        assert_eq!(
            Path::new(fields[2]).file_name(),
            Some(home_dir),
            "the engine's HOME is not its working directory"
        );
        assert_eq!(fields[3], "-batch -noheader -csv -init");
        // The init file DuckDB reads in place of `~/.duckdbrc` is an empty file
        // inside that same sealed directory.
        assert_eq!(
            Path::new(fields[4]).parent().and_then(Path::file_name),
            Some(home_dir),
            "the init file is not in the sealed directory"
        );
        assert_eq!(
            fields[5], "empty",
            "the init file the engine read is not empty"
        );
        assert_eq!(fields[6], "0", "the engine's agent output mode is not off");
    }

    #[test]
    fn nothing_staged_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            check_staged_engine(&dir.path().join("brightfield"), "v9.9.9,abc123"),
            EngineCheck::Absent
        );
    }

    #[test]
    fn the_linked_duckdb_reports_a_release_and_a_commit() {
        let linked = linked_duckdb_version().unwrap();
        let (release, commit) = linked.split_once(',').unwrap();
        assert!(release.starts_with('v'), "{linked}");
        assert!(!commit.is_empty(), "{linked}");
    }

    /// The linked DuckDB names the release the staged CLI is pinned to, and not
    /// a development build of it. libduckdb-sys 1.20000.0 compiles DuckDB 2.0.0
    /// under the version `v2.0.0-dev88614`, which would fetch extensions from a
    /// path nothing is published under; `.cargo/config.toml` defines the
    /// release's version for that build.
    #[test]
    fn the_linked_duckdb_is_the_release_the_cli_is_pinned_to() {
        let script = include_str!("../../../scripts/fetch-duckdb-cli.sh");
        let pinned = script
            .lines()
            .find_map(|line| line.strip_prefix("VERSION=\""))
            .and_then(|rest| rest.strip_suffix('"'))
            .expect("scripts/fetch-duckdb-cli.sh has a VERSION= line");
        let linked = linked_duckdb_version().unwrap();
        let (release, _) = linked.split_once(',').unwrap();
        assert_eq!(release, pinned, "{linked}");
    }

    /// Watched redden, one mutation: the `chars.next()` that consumes the
    /// `[` removed, so `[` is taken as the final byte and `1m` survives.
    #[test]
    fn colour_codes_come_out_and_the_words_stay() {
        assert_eq!(
            without_colour_codes("[1/1] \u{1b}[1mload\u{1b}[0m ..."),
            "[1/1] load ..."
        );
        assert_eq!(
            without_colour_codes("\u{1b}[32m✓\u{1b}[39m 1/1 steps succeeded."),
            "✓ 1/1 steps succeeded."
        );
        assert_eq!(without_colour_codes("no codes [here]"), "no codes [here]");
    }
    #[test]
    fn progress_counts_the_steps_done_and_names_the_one_running() {
        let s = brightfield_protocol::stream::fold_stream(
            "{\"step\":\"fetch\",\"state\":\"success\",\"ts\":\"t\"}\n\
             {\"step\":\"rank\",\"state\":\"running\",\"ts\":\"t\"}\n",
        );
        assert_eq!(
            progress(&s, 3),
            RunProgress {
                running: Some("rank".into()),
                done: 1,
                total: 3
            }
        );
    }

    /// arc writes a line as each step finishes, a failed one included, and no `running` line.
    #[test]
    fn a_stream_of_finished_steps_counts_each_and_names_none_running() {
        let s = brightfield_protocol::stream::fold_stream(
            "{\"step\":\"a\",\"state\":\"success\"}\n{\"step\":\"b\",\"state\":\"error\"}\n",
        );
        assert_eq!(
            progress(&s, 4),
            RunProgress {
                running: None,
                done: 2,
                total: 4
            }
        );
        assert_eq!(progress(&s, 1).done, 1, "done is held to the step count");
    }

    #[test]
    fn a_run_is_described_by_its_day_and_its_age() {
        use chrono::TimeZone;
        // An instant, then read in the local zone: the age is time elapsed, whatever the zone.
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 11, 21, 10, 0, 0)
            .unwrap()
            .with_timezone(&chrono::Local);
        let at = |t: chrono::DateTime<chrono::Local>| t.to_rfc3339();
        assert_eq!(
            describe_ran("2026-10-23T09:14:00+00:00", now).map(|(w, _)| w),
            Some("ran 23 Oct".into())
        );
        assert_eq!(
            describe_ran("2026-10-23T09:14:00+00:00", now).map(|(_, a)| a),
            Some("29 days ago".into())
        );
        let before = |d: chrono::TimeDelta| describe_ran(&at(now - d), now).map(|(_, a)| a);
        assert_eq!(
            before(chrono::TimeDelta::seconds(59)),
            Some("just now".into())
        );
        assert_eq!(
            before(chrono::TimeDelta::minutes(1)),
            Some("1 minute ago".into())
        );
        assert_eq!(
            before(chrono::TimeDelta::minutes(59)),
            Some("59 minutes ago".into())
        );
        assert_eq!(
            before(chrono::TimeDelta::hours(1)),
            Some("1 hour ago".into())
        );
        assert_eq!(
            before(chrono::TimeDelta::hours(23)),
            Some("23 hours ago".into())
        );
        assert_eq!(before(chrono::TimeDelta::days(1)), Some("1 day ago".into()));
        assert_eq!(
            before(chrono::TimeDelta::seconds(-30)),
            Some("just now".into())
        );
        let last_year = chrono::Local
            .with_ymd_and_hms(2025, 10, 23, 12, 0, 0)
            .unwrap();
        assert_eq!(
            describe_ran(&at(last_year), now).map(|(w, _)| w),
            Some("ran 23 Oct 2025".into())
        );
        assert_eq!(describe_ran("not a time", now), None);
    }

    #[test]
    fn the_live_stream_is_the_newest_jsonl_not_there_when_the_run_began() {
        let tmp = std::env::temp_dir().join(format!(
            "bf-live-stream-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos())
        ));
        assert_eq!(
            live_stream(&tmp, &BTreeSet::new()),
            None,
            "no runs folder, no stream"
        );
        let runs = runs_dir(&tmp);
        std::fs::create_dir_all(&runs).unwrap();
        assert_eq!(live_stream(&tmp, &BTreeSet::new()), None);

        let old = runs.join("old.jsonl");
        std::fs::write(&old, b"").unwrap();
        let earlier = streams_in(&tmp);
        assert_eq!(earlier, BTreeSet::from([old.clone()]));
        assert_eq!(
            live_stream(&tmp, &earlier),
            None,
            "an earlier run's stream is not this one, however recent"
        );

        let new = runs.join("new.jsonl");
        std::fs::write(&new, b"").unwrap();
        std::fs::write(runs.join("new.json"), b"{}").unwrap();
        assert_eq!(live_stream(&tmp, &earlier), Some(new.clone()));
        let later = runs.join("later.jsonl");
        std::fs::write(&later, b"").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&later)
            .unwrap()
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(
            live_stream(&tmp, &earlier),
            Some(later),
            "of two new streams, the newer"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
