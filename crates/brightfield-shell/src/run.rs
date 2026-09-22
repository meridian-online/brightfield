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
//! [`std::env::current_exe`], by way of [`Runner::this_binary`]. When it is
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
    /// process is `brightfield-shell`.
    ///
    /// `None` when the operating system will not say where the executable is.
    #[must_use]
    pub fn this_binary() -> Option<Self> {
        std::env::current_exe().ok().map(Self::at)
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

/// Start the child, wait for it, and say what it left.
///
/// Which record is this run's is decided by **what was not there before**,
/// not by which is newest: a run that ends before its step loop writes no
/// record, and reading the newest one then would report the previous run's
/// outcome as this one's.
fn run_to_completion(program: &Path, dir: &Path) -> Finished {
    let before: BTreeSet<PathBuf> = record_paths_newest_first(dir).into_iter().collect();
    let mut command = Command::new(program);
    command
        .arg("run")
        .current_dir(dir)
        .env(RUNNER_ENV, "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null());
    if let Some(engine) = staged_engine_beside(program) {
        command.env(ENGINE_ENV, engine);
    }
    let output = command.output();
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
}
