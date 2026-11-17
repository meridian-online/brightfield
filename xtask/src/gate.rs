//! The local pull-request gate: `cargo xtask gate <pr>` runs the pull
//! request's own `cargo xtask ci`, merged with main, in a clean worktree and an
//! isolated environment, and posts the verdict as the `local-gate` commit
//! status on the pull request's head.
//!
//! The order of a run: take the gate lock; read main's backstop workflows and
//! keep the `main-red` issue in step with them; refuse a fork's head; fetch,
//! make the worktree and merge main into it; post `pending`; run the tree's
//! gate with its output teed to a log; post `success` or `failure`; clean up.
//! Every failure after the head is known is posted as a `failure` status, and
//! a run that leaves no summary is a failure, never a pass.

use std::ffi::OsStr;
use std::fs::{self, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::runner::wall;
use crate::steps::{self, Ctx};

/// The repository whose pull requests the gate runs, and whose branches only.
const REPO: &str = "meridian-online/brightfield";
/// The commit status context the gate posts.
const CONTEXT: &str = "local-gate";
/// The workflows that still run on main, whose latest runs say whether main
/// is red.
const BACKSTOPS: [&str; 2] = ["test.yml", "lint.yml"];
/// The label of the one open issue that says main is red.
const RED_LABEL: &str = "main-red";
const RED_BODY: &str = "`cargo xtask gate` reads main's backstop before each pull request's gate: the latest completed `test.yml` and `lint.yml` runs on main. One of them failed, so main is red. Each red run is commented here once, and the gate closes this issue when both workflows are green on one commit. A red main does not stop a pull request's gate, since the fix goes forward.";

fn say(message: &str) {
    eprintln!("cargo xtask gate: {message}");
}

/// Each backstop workflow's aggregate job, by workflow file and job name: a
/// job that only reports whether the others passed, and that fails when one
/// was cancelled. The backstop reads the jobs under it, so a cancelled run is
/// not taken for a red one.
/// `the_aggregate_jobs_the_backstop_leaves_out_are_still_in_their_workflows`
/// holds each name to its workflow.
pub const AGGREGATE_JOBS: &[(&str, &str)] = &[("test.yml", "test")];

/// Where a gate stopped before its steps gave a verdict.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stage {
    Fork,
    Fetch,
    Worktree,
    Merge,
    DuckDb,
    OpenAnalytics,
    Start,
    NoSummary,
    ExitedNonZero,
    Unexpected,
}

impl Stage {
    /// The status text for the stage. It is fixed, since the repository is
    /// public and the error behind it can carry the machine's paths.
    fn phrase(self) -> &'static str {
        match self {
            Stage::Fork => "the head is in a fork, and the gate does not run a fork's code",
            Stage::Fetch => "could not fetch the pull request",
            Stage::Worktree => "could not make the worktree",
            Stage::Merge => "merge main into the branch first",
            Stage::DuckDb => "the pinned DuckDB CLI is missing",
            Stage::OpenAnalytics => "the open-analytics checkout is missing",
            Stage::Start => "could not start the gate's run",
            Stage::NoSummary => "the gate's run left no summary",
            Stage::ExitedNonZero => "the gate's run failed though no step did",
            Stage::Unexpected => "the gate stopped unexpectedly",
        }
    }
}

/// A gate that stopped short: the stage, posted, and the error, printed.
#[derive(Debug)]
pub struct Failure {
    stage: Stage,
    detail: String,
}

impl Failure {
    pub fn new(stage: Stage, detail: impl Into<String>) -> Failure {
        Failure {
            stage,
            detail: detail.into(),
        }
    }
}

/// Why a gate stopped short of a verdict from the steps.
enum Stop {
    /// Post `failure` on the head with the stage's phrase.
    Fail(Failure),
    /// Nothing can be posted: say this and exit.
    Unposted(String),
}

/// What a gate that ran its steps posts.
struct Posted {
    state: &'static str,
    text: String,
    /// test-default's own log, kept beside the gate's, when the run left one.
    kept: Option<PathBuf>,
}

/// Run the gate on pull request `pr` and post its verdict.
pub fn run(pr: u64) -> ExitCode {
    let gh = OsStr::new("gh");
    let root = crate::repo_root();
    let Some(home) = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
    else {
        say("HOME is not set, and the gate keeps its lock, target and logs under it");
        return ExitCode::FAILURE;
    };
    let gate_dir = home.join(".cache/meridian/gate");

    if let Err(e) = fs::create_dir_all(gate_dir.join("logs")) {
        say(&format!("cannot create {}: {e}", gate_dir.display()));
        return ExitCode::FAILURE;
    }
    let _lock = match Lock::acquire(&gate_dir, Duration::from_secs(30), say) {
        Ok(lock) => lock,
        Err(e) => {
            say(&e);
            return ExitCode::FAILURE;
        }
    };

    check_backstop();

    let (head, repository) = match head_of(pr) {
        Ok(found) => found,
        Err(e) => {
            say(&e);
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = check_head_repository(&repository) {
        return post_failure(gh, &head, &Failure::new(Stage::Fork, e));
    }
    let sha7: String = head.chars().take(7).collect();
    let logs = gate_dir.join("logs");
    let log = logs.join(format!("pr-{pr}-{sha7}.log"));
    let kept = logs.join(format!("pr-{pr}-{sha7}.test-default.log"));
    let run = format!("brightfield-gate-{pr}-{}", std::process::id());
    let scratch = Scratch {
        root: root.clone(),
        dir: std::env::temp_dir().join(&run),
        tree: tree_beside_target(&gate_dir, &run),
    };
    let verdict = gate_head(
        pr,
        &Paths {
            root: &root,
            home: &home,
            gate_dir: &gate_dir,
            log: &log,
            kept: &kept,
        },
        &head,
        &scratch,
    );
    drop(scratch);
    let posted = match verdict {
        Ok(posted) => posted,
        Err(Stop::Unposted(e)) => {
            say(&e);
            return ExitCode::FAILURE;
        }
        Err(Stop::Fail(failure)) => return post_failure(gh, &head, &failure),
    };
    if let Err(e) = post_status(gh, &head, posted.state, &posted.text) {
        say(&e);
        return ExitCode::FAILURE;
    }
    println!("local-gate {} on {sha7}: {}", posted.state, posted.text);
    println!("the full log is {}", log.display());
    if let Some(kept) = &posted.kept {
        println!("test-default's own log is {}", kept.display());
    }
    if posted.state == "success" {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Post a failure that stopped the gate before its steps gave a verdict: the
/// error to the terminal, the stage's fixed phrase to the status.
fn post_failure(gh: &OsStr, head: &str, failure: &Failure) -> ExitCode {
    say(&failure.detail);
    let text = failure.stage.phrase();
    match post_status(gh, head, "failure", text) {
        Ok(()) => {
            let sha7: String = head.chars().take(7).collect();
            println!("local-gate failure on {sha7}: {text}");
        }
        Err(e) => say(&e),
    }
    ExitCode::FAILURE
}

/// The pull request's head commit, and the repository it lives in as
/// `owner/name`.
fn head_of(pr: u64) -> Result<(String, String), String> {
    let pr_text = pr.to_string();
    let json = output_of(
        "gh",
        &[
            "pr",
            "view",
            &pr_text,
            "--repo",
            REPO,
            "--json",
            "headRefOid,headRepository,headRepositoryOwner",
        ],
    )?;
    let view: Value =
        serde_json::from_str(&json).map_err(|e| format!("`gh pr view {pr}` is not JSON: {e}"))?;
    let head = view
        .get("headRefOid")
        .and_then(Value::as_str)
        .filter(|sha| !sha.is_empty())
        .ok_or_else(|| format!("pull request {pr} has no head commit"))?
        .to_owned();
    let owner = view
        .pointer("/headRepositoryOwner/login")
        .and_then(Value::as_str);
    let name = view.pointer("/headRepository/name").and_then(Value::as_str);
    let repository = match (owner, name) {
        (Some(owner), Some(name)) => format!("{owner}/{name}"),
        _ => "a repository that no longer exists".to_owned(),
    };
    Ok((head, repository))
}

/// Refuse a head outside this repository: the gate builds and runs the
/// branch's code on this machine, and its isolated environment is test
/// hygiene, not containment.
pub fn check_head_repository(name_with_owner: &str) -> Result<(), String> {
    if name_with_owner == REPO {
        Ok(())
    } else {
        Err(format!(
            "the head is in {name_with_owner}, not {REPO}: the gate does not run a fork's code"
        ))
    }
}

/// The run's temporary directory and worktree, removed when dropped.
struct Scratch {
    root: PathBuf,
    /// The run's fresh HOME and TMPDIR, in the system's temporary directory.
    dir: PathBuf,
    /// The merged worktree, from [`tree_beside_target`].
    tree: PathBuf,
}

impl Scratch {
    fn tree(&self) -> PathBuf {
        self.tree.clone()
    }
}

/// Where a run's worktree goes: under the gate's directory, beside its
/// persistent `target`, as a checkout's tree sits beside its own `target` in
/// CI. A test that spells a scratch file relative to its working directory
/// then climbs a few levels to the gate's directory, as it climbs two in CI,
/// rather than to `/`; a climb to `/` reaches the same file from anywhere
/// shallower, which `status_rail`'s relative-watch test refuses.
fn tree_beside_target(gate_dir: &Path, run: &str) -> PathBuf {
    gate_dir.join("trees").join(run)
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if self.tree().exists() {
            let _ = Command::new("git")
                .arg("-C")
                .arg(&self.root)
                .args(["worktree", "remove", "--force"])
                .arg(self.tree())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = fs::remove_dir_all(&self.dir);
        let _ = fs::remove_dir_all(&self.tree);
        let _ = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["worktree", "prune"])
            .status();
    }
}

/// The places a gate run reads and writes outside its scratch directory.
struct Paths<'a> {
    /// The checkout the gate was started from.
    root: &'a Path,
    /// The real HOME.
    home: &'a Path,
    /// The gate's cache: lock, target directory and logs.
    gate_dir: &'a Path,
    /// The run's teed output.
    log: &'a Path,
    /// Where test-default's own log is kept.
    kept: &'a Path,
}

/// True for a variable the gate's run must not inherit: one that points the
/// run back into the real HOME, or picks the toolchain over the tree's own
/// file, or changes what is built (build, per-target and profile settings, and
/// the C and C++ compilers and flags the `cc` builds read, since a `CXXFLAGS`
/// here would override the DuckDB version define in `.cargo/config.toml`), so
/// the verdict is CI's. `CARGO_BUILD_JOBS` stays, since it caps the
/// machine's build slots and changes no output, and `CARGO_TARGET_DIR` is the
/// gate's own.
fn removed_from_child(key: &str) -> bool {
    matches!(
        key,
        "RUSTUP_TOOLCHAIN"
            | "CARGO"
            | "XDG_CONFIG_HOME"
            | "XDG_CACHE_HOME"
            | "XDG_DATA_HOME"
            | "XDG_STATE_HOME"
            | "RUNNER_TEMP"
            | "RUSTFLAGS"
            | "RUSTDOCFLAGS"
            | "CARGO_ENCODED_RUSTFLAGS"
            | "CARGO_ENCODED_RUSTDOCFLAGS"
            | "RUSTC_WRAPPER"
            | "RUSTC_WORKSPACE_WRAPPER"
            | "MACOSX_DEPLOYMENT_TARGET"
    ) || is_compiler_variable(key)
        || (key.starts_with("CARGO_BUILD_") && key != "CARGO_BUILD_JOBS")
        || (key.starts_with("CARGO_TARGET_") && key != "CARGO_TARGET_DIR")
        || key.starts_with("CARGO_PROFILE_")
}

/// The C and C++ compiler and flag variables the `cc` crate reads, in each
/// form it reads them: plain, `HOST_` or `TARGET_` prefixed, and suffixed with
/// a target triple (`CXXFLAGS_aarch64_apple_darwin`, `CC_aarch64-apple-darwin`).
fn is_compiler_variable(key: &str) -> bool {
    const BASES: [&str; 7] = [
        "CC", "CXX", "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "AR",
    ];
    let bare = key
        .strip_prefix("HOST_")
        .or_else(|| key.strip_prefix("TARGET_"))
        .unwrap_or(key);
    BASES.iter().any(|base| {
        bare.strip_prefix(base)
            .is_some_and(|rest| rest.is_empty() || (bare == key && rest.starts_with('_')))
    })
}

/// Build the tree to test, run its gate, and say what to post.
fn gate_head(pr: u64, paths: &Paths, head: &str, scratch: &Scratch) -> Result<Posted, Stop> {
    let fail = |stage: Stage| move |e: String| Stop::Fail(Failure::new(stage, e));
    let root = paths.root;
    let git_in_root = |args: &[&str]| {
        let mut all = vec!["-C".to_owned(), root.display().to_string()];
        all.extend(args.iter().map(|a| (*a).to_owned()));
        let all: Vec<&str> = all.iter().map(String::as_str).collect();
        output_of("git", &all)
    };
    git_in_root(&[
        "fetch",
        "--quiet",
        "origin",
        "main",
        &format!("pull/{pr}/head"),
    ])
    .map_err(fail(Stage::Fetch))?;
    let main_sha = git_in_root(&["rev-parse", "origin/main"])
        .map(|sha| sha.trim().to_owned())
        .map_err(fail(Stage::Fetch))?;

    let tree = scratch.tree();
    let (child_home, child_tmp) = (scratch.dir.join("home"), scratch.dir.join("tmp"));
    for dir in [&child_home, &child_tmp] {
        fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))
            .map_err(fail(Stage::Worktree))?;
    }
    let tree_text = tree.display().to_string();
    git_in_root(&["worktree", "add", "--quiet", "--detach", &tree_text, head])
        .map_err(fail(Stage::Worktree))?;
    merge_with_main(&tree, "origin/main").map_err(fail(Stage::Merge))?;

    let summary = scratch.dir.join("summary.json");
    let invoking = Ctx {
        root: root.to_path_buf(),
        cargo: "cargo".into(),
        log: None,
    };
    let host = steps::host_triple().map_err(fail(Stage::DuckDb))?;
    let candidate = steps::engine_path(root, std::env::var_os("ARC_DUCKDB_BIN"));
    let engine = engine_for_tree(
        &tree.join("scripts/fetch-duckdb-cli.sh"),
        &host,
        &candidate,
        &paths.gate_dir.join("duckdb-cli"),
    )
    .map_err(fail(Stage::DuckDb))?;
    let open_analytics = steps::open_analytics(&invoking)
        .and_then(|dir| {
            fs::canonicalize(&dir).map_err(|e| format!("cannot resolve {}: {e}", dir.display()))
        })
        .map_err(fail(Stage::OpenAnalytics))?;
    let real = |var: &str, under_home: &str| {
        std::env::var_os(var)
            .filter(|v| !v.is_empty())
            .map_or_else(|| paths.home.join(under_home), PathBuf::from)
    };
    let tree_ctx = Ctx {
        root: tree.clone(),
        cargo: "cargo".into(),
        log: None,
    };
    let mut command = tree_ctx.command("cargo");
    command
        .arg("xtask")
        .arg("ci")
        .arg("--summary-json")
        .arg(&summary)
        .env("HOME", &child_home)
        .env("TMPDIR", &child_tmp)
        .env("CARGO_HOME", real("CARGO_HOME", ".cargo"))
        .env("RUSTUP_HOME", real("RUSTUP_HOME", ".rustup"))
        .env("CARGO_TARGET_DIR", paths.gate_dir.join("target"))
        .env("ARC_DUCKDB_BIN", &engine)
        .env("OPEN_ANALYTICS_DIR", &open_analytics);
    for (key, _) in std::env::vars_os() {
        if removed_from_child(&key.to_string_lossy()) {
            command.env_remove(&key);
        }
    }

    let main7: String = main_sha.chars().take(7).collect();
    post_status(
        OsStr::new("gh"),
        head,
        "pending",
        &format!("running on main {main7}"),
    )
    .map_err(Stop::Unposted)?;
    say(&format!(
        "running pull request {pr} merged with main {main7}; the log is {}",
        paths.log.display()
    ));
    // Past the pending post, a panic must still end in a posted failure, or
    // the head would stay pending.
    let own_log = tree.join("target/xtask/test-default.log");
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_and_read(command, paths, &summary, &own_log, &main_sha)
    }))
    .unwrap_or_else(|_| {
        Err(Stop::Fail(Failure::new(
            Stage::Unexpected,
            "the gate panicked while its run was under way",
        )))
    })
}

/// Run the tree's gate, keep test-default's own log, and read the verdict.
fn run_and_read(
    command: Command,
    paths: &Paths,
    summary: &Path,
    own_log: &Path,
    main_sha: &str,
) -> Result<Posted, Stop> {
    let fail = |stage: Stage| move |e: String| Stop::Fail(Failure::new(stage, e));
    // A kept log from an earlier run of this head would read as this run's.
    let _ = fs::remove_file(paths.kept);
    let started = Instant::now();
    let status = run_teed(command, paths.log).map_err(fail(Stage::Start))?;
    let took = started.elapsed();

    let kept = if own_log.is_file() {
        match fs::copy(own_log, paths.kept) {
            Ok(_) => {
                say(&format!(
                    "test-default's own log is kept at {}",
                    paths.kept.display()
                ));
                Some(paths.kept.to_path_buf())
            }
            Err(e) => {
                say(&format!(
                    "could not keep test-default's log at {}: {e}",
                    paths.kept.display()
                ));
                None
            }
        }
    } else {
        None
    };

    let text = fs::read_to_string(summary)
        .map_err(|e| format!("cannot read {}: {e}", summary.display()))
        .map_err(fail(Stage::NoSummary))?;
    let verdict = read_summary(&text).map_err(fail(Stage::NoSummary))?;
    if verdict.failed.is_none() && !status.success() {
        return Err(fail(Stage::ExitedNonZero)(format!(
            "the gate's run exited with {status} though no step failed"
        )));
    }
    let state = if verdict.failed.is_none() {
        "success"
    } else {
        "failure"
    };
    Ok(Posted {
        state,
        text: description(&verdict, took, main_sha),
        kept,
    })
}

/// Merge `main` into the worktree at `tree`. Any failure is reported as the
/// one thing to do about it; git's own account is on the terminal.
pub fn merge_with_main(tree: &Path, main: &str) -> Result<(), String> {
    let merged = Command::new("git")
        .arg("-C")
        .arg(tree)
        .args([
            "-c",
            "user.name=local-gate",
            "-c",
            "user.email=local-gate@localhost",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
            // A merge that has to make a commit is a merge, not a conflict,
            // whatever the person's own `merge.ff` says.
            "-c",
            "merge.ff=true",
            "merge",
            "--no-edit",
            "--quiet",
            main,
        ])
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("could not start git: {e}"))?;
    if merged.success() {
        Ok(())
    } else {
        Err("merge main into the branch first".to_owned())
    }
}

/// Run `command` with its output and errors copied to the terminal and to
/// `log`, and return how it exited.
fn run_teed(mut command: Command, log: &Path) -> Result<ExitStatus, String> {
    let mut file =
        fs::File::create(log).map_err(|e| format!("cannot create {}: {e}", log.display()))?;
    let (mut reader, writer) = std::io::pipe().map_err(|e| format!("cannot make a pipe: {e}"))?;
    let writer_copy = writer
        .try_clone()
        .map_err(|e| format!("cannot make a pipe: {e}"))?;
    command
        .stdin(Stdio::null())
        .stdout(writer_copy)
        .stderr(writer);
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start the tree's `cargo xtask ci`: {e}"))?;
    // The command holds the pipe's write ends; drop it so the read ends.
    drop(command);
    let mut terminal = std::io::stderr();
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                let _ = terminal.write_all(&buffer[..n]);
                let _ = file.write_all(&buffer[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    child
        .wait()
        .map_err(|e| format!("lost the tree's `cargo xtask ci`: {e}"))
}

/// How long the tree's script may take to download the DuckDB CLI.
const FETCH_TIMEOUT: Duration = Duration::from_secs(600);

/// The DuckDB CLI the merged tree pins, judged by the tree's own
/// `fetch-duckdb-cli.sh`, since the tree's `cargo xtask ci` checks the engine
/// it is handed with that script and a pull request may move the pin. The
/// invoking checkout's engine (`candidate`) is used when the tree's script
/// accepts it; then the gate's own copy in `cache`; otherwise the tree's
/// script fetches its pinned release into `cache`, checking its hashes.
fn engine_for_tree(
    script: &Path,
    host: &str,
    candidate: &Path,
    cache: &Path,
) -> Result<PathBuf, String> {
    let accepts = |dir: &Path| {
        output_within(
            script,
            &["--check", host, &dir.to_string_lossy()],
            CALL_TIMEOUT,
        )
        .is_ok()
    };
    if candidate.file_name().is_some_and(|name| name == "duckdb") {
        if let Some(dir) = candidate.parent() {
            if accepts(dir) {
                return Ok(candidate.to_path_buf());
            }
        }
    }
    if accepts(cache) {
        return Ok(cache.join("duckdb"));
    }
    fs::create_dir_all(cache).map_err(|e| format!("cannot create {}: {e}", cache.display()))?;
    output_within(script, &[host, &cache.to_string_lossy()], FETCH_TIMEOUT)
        .map_err(|e| format!("could not fetch the DuckDB CLI the branch pins: {e}"))?;
    if accepts(cache) {
        Ok(cache.join("duckdb"))
    } else {
        Err(format!(
            "the DuckDB CLI fetched into {} is not the one the branch pins",
            cache.display()
        ))
    }
}

/// Post the `local-gate` status on `sha` with `gh`, which runs with this
/// process's own HOME and so its own credentials. A post happens under the
/// gate lock, so it is held to `CALL_TIMEOUT` like the gate's other calls
/// (`a_status_post_that_hangs_is_stopped_and_is_an_error`).
pub fn post_status(gh: &OsStr, sha: &str, state: &str, text: &str) -> Result<(), String> {
    post_status_within(gh, sha, state, text, CALL_TIMEOUT)
}

fn post_status_within(
    gh: &OsStr,
    sha: &str,
    state: &str,
    text: &str,
    timeout: Duration,
) -> Result<(), String> {
    let endpoint = format!("repos/{REPO}/statuses/{sha}");
    let state = format!("state={state}");
    let context = format!("context={CONTEXT}");
    let description = format!("description={text}");
    let args = [
        "api",
        "--silent",
        &endpoint,
        "-f",
        &state,
        "-f",
        &context,
        "-f",
        &description,
    ];
    output_within(gh, &args, timeout)
        .map(|_| ())
        .map_err(|e| format!("the verdict was not posted: {e}"))
}

/// How long a `gh`, `kata` or `git` call may take before the gate gives up on
/// it. The backstop's calls and the status posts run under the gate lock, so a
/// hung one must not hold the lock for good.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Run a program and return what it printed; a non-zero exit is the error,
/// with what it said, and so is a call that outlasts `CALL_TIMEOUT`.
fn output_of(program: impl AsRef<OsStr>, args: &[&str]) -> Result<String, String> {
    output_within(program, args, CALL_TIMEOUT)
}

/// Read a child's pipe to its end on a thread of its own.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    })
}

/// `output_of` with the timeout given: a call still running after `timeout`
/// is killed, and the error names it.
fn output_within(
    program: impl AsRef<OsStr>,
    args: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    let program = program.as_ref();
    let shown = format!("{} {}", program.to_string_lossy(), args.join(" "));
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start `{shown}`: {e}"))?;
    // Read both pipes on their own threads, so a call that prints more than a
    // pipe holds cannot stall waiting for the gate to read.
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "`{shown}` timed out after {} and was stopped",
                    wall(timeout)
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("lost `{shown}`: {e}")),
        }
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    if status.success() {
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    } else {
        Err(format!(
            "`{shown}` exited with {status}: {}",
            String::from_utf8_lossy(&stderr).trim()
        ))
    }
}

/// Read main's backstop and keep the `main-red` issue in step with it. Nothing
/// here stops the gate: a red main does not block a pull request, and a
/// failed read or issue call is printed and passed over.
fn check_backstop() {
    let mut read = Vec::new();
    let mut unread = false;
    for workflow in BACKSTOPS {
        match read_one_backstop(workflow) {
            Ok(backstop) => read.push(backstop),
            Err(e) => {
                say(&format!("main's backstop: {e}"));
                unread = true;
            }
        }
    }
    // A workflow that could not be read leaves main short of green.
    match (combine(&read), unread) {
        (MainState::Red(reds), _) => {
            say("main is red; the gate carries on, since the fix goes forward");
            note_red(&reds);
        }
        (MainState::Green { sha }, false) => note_green(&sha),
        _ => say("main's backstop is unsettled; the main-red issue is left as it is"),
    }
}

fn read_one_backstop(workflow: &str) -> Result<Backstop, String> {
    let runs = output_of(
        "gh",
        &[
            "run",
            "list",
            "--repo",
            REPO,
            "--branch",
            "main",
            "--workflow",
            workflow,
            "--status",
            "completed",
            "--limit",
            "1",
            "--json",
            "databaseId,conclusion,headSha,url",
        ],
    )?;
    let id = serde_json::from_str::<Value>(&runs)
        .ok()
        .and_then(|v| v.pointer("/0/databaseId").and_then(Value::as_u64))
        .ok_or_else(|| format!("{workflow}: no completed run on main"))?;
    let jobs = output_of(
        "gh",
        &[
            "run",
            "view",
            &id.to_string(),
            "--repo",
            REPO,
            "--json",
            "jobs",
        ],
    )?;
    read_backstop(workflow, &runs, &jobs)
}

/// Run kata in brightfield's project.
fn kata(args: &[&str]) -> Result<String, String> {
    kata_with(OsStr::new("kata"), args)
}

/// [`kata`] through `program`, so a test can stand a shim in for it.
fn kata_with(program: &OsStr, args: &[&str]) -> Result<String, String> {
    let mut all = args.to_vec();
    all.extend(["--project", "brightfield"]);
    output_of(program, &all)
}

/// Run kata for its effect, printing a failure and carrying on.
fn kata_or_say(args: &[&str]) {
    if let Err(e) = kata(args) {
        say(&format!(
            "the issue tracker call failed, and the gate carries on: {e}"
        ));
    }
}

/// The short id of the open `main-red` issue, if there is one.
fn open_red_issue() -> Result<Option<String>, String> {
    open_red_issue_with(OsStr::new("kata"))
}

fn open_red_issue_with(program: &OsStr) -> Result<Option<String>, String> {
    let json = kata_with(program, &["list", "--label", RED_LABEL, "--json"])?;
    let doc: Value =
        serde_json::from_str(&json).map_err(|e| format!("`kata list` is not JSON: {e}"))?;
    Ok(doc
        .pointer("/issues/0/short_id")
        .and_then(Value::as_str)
        .map(str::to_owned))
}

fn note_red(reds: &[Backstop]) {
    let issue = match open_red_issue() {
        Ok(Some(issue)) => issue,
        Ok(None) => {
            // `--force-new` past a closed look-alike is safe because this runs under the gate lock, after finding no open one.
            let created = kata(&[
                "create",
                "main is red",
                "--body",
                RED_BODY,
                "--label",
                RED_LABEL,
                "--force-new",
                "--agent",
            ])
            .and_then(|_| open_red_issue())
            .and_then(|issue| {
                issue.ok_or_else(|| "`kata create` left no open main-red issue".to_owned())
            });
            match created {
                Ok(issue) => {
                    kata_or_say(&[
                        "meta",
                        "set",
                        &issue,
                        "work.attention",
                        "needs-human",
                        "--agent",
                    ]);
                    kata_or_say(&[
                        "meta",
                        "set",
                        &issue,
                        "work.attention_msg",
                        "main is red: a backstop workflow failed",
                        "--agent",
                    ]);
                    issue
                }
                Err(e) => {
                    say(&format!(
                        "the issue tracker call failed, and the gate carries on: {e}"
                    ));
                    return;
                }
            }
        }
        Err(e) => {
            say(&format!(
                "the issue tracker call failed, and the gate carries on: {e}"
            ));
            return;
        }
    };
    let noted = match kata(&["show", &issue, "--json"]) {
        Ok(json) => serde_json::from_str::<Value>(&json)
            .ok()
            .and_then(|doc| doc.get("comments").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|c| c.get("body").and_then(Value::as_str).map(str::to_owned))
            .collect::<Vec<_>>(),
        Err(e) => {
            say(&format!(
                "the issue tracker call failed, and the gate carries on: {e}"
            ));
            return;
        }
    };
    for red in reds {
        let Backstop::Red {
            url, sha, workflow, ..
        } = red
        else {
            continue;
        };
        if noted.iter().any(|body| body.contains(url.as_str())) {
            continue;
        }
        let sha7: String = sha.chars().take(7).collect();
        kata_or_say(&[
            "comment",
            &issue,
            "--body",
            &format!("{workflow} failed on main at {sha7}: {url}"),
            "--agent",
        ]);
    }
}

fn note_green(sha: &str) {
    note_green_with(OsStr::new("kata"), sha);
}

/// Close the open `main-red` issue with `sha`, the commit both workflows went
/// green on, as its evidence: kata refuses a done close without evidence. The
/// green note rides on the close as its `--comment`, which kata posts only once
/// the close succeeds, so a refused close leaves no note to repeat on the next
/// gate's run.
fn note_green_with(program: &OsStr, sha: &str) {
    let issue = match open_red_issue_with(program) {
        Ok(Some(issue)) => issue,
        Ok(None) => return,
        Err(e) => {
            say(&format!(
                "the issue tracker call failed, and the gate carries on: {e}"
            ));
            return;
        }
    };
    let sha7: String = sha.chars().take(7).collect();
    let message = format!(
        "main's test and lint workflows are green again on one commit, {sha7}, as the gate read them"
    );
    let note = format!("main is green at {sha7}");
    let closed = kata_with(
        program,
        &[
            "close",
            &issue,
            "--done",
            "--message",
            &message,
            "--commit",
            sha,
            "--comment",
            &note,
            "--agent",
        ],
    );
    if let Err(e) = closed {
        say(&format!(
            "the issue tracker call failed, and the gate carries on: {e}"
        ));
    }
}

/// GitHub caps a commit status description at 140 characters.
const DESCRIPTION_LIMIT: usize = 140;

/// What a full `ci` run came to: steps passed, and the step that failed.
#[derive(Debug, PartialEq)]
pub struct Verdict {
    pub passed: usize,
    pub failed: Option<String>,
}

/// Read the summary file `ci --summary-json` writes:
/// `{"steps":[{"name":"fmt","outcome":"passed"}, ...]}`.
pub fn read_summary(json: &str) -> Result<Verdict, String> {
    let doc: Value = serde_json::from_str(json).map_err(|e| format!("summary is not JSON: {e}"))?;
    let steps = doc
        .get("steps")
        .and_then(Value::as_array)
        .ok_or("summary has no `steps` list")?;
    let mut passed = 0;
    let mut failed = None;
    for step in steps {
        let name = step.get("name").and_then(Value::as_str);
        let outcome = step.get("outcome").and_then(Value::as_str);
        match (name, outcome) {
            (Some(_), Some("passed")) => passed += 1,
            (Some(name), Some("failed")) => {
                failed.get_or_insert_with(|| name.to_owned());
            }
            (Some(_), Some("skipped" | "ci-only")) => {}
            _ => return Err(format!("summary has a step it cannot read: {step}")),
        }
    }
    if passed == 0 && failed.is_none() {
        return Err("the gate's run recorded no step that passed or failed".to_owned());
    }
    Ok(Verdict { passed, failed })
}

/// The commit status description, at most 140 characters.
pub fn description(verdict: &Verdict, took: Duration, main_sha: &str) -> String {
    let sha: String = main_sha.chars().take(7).collect();
    let time = wall(took);
    match &verdict.failed {
        None => format!("passed {} steps in {time} on main {sha}", verdict.passed),
        Some(name) => {
            let head = "failed at ";
            let tail = format!(" after {} steps, {time}, on main {sha}", verdict.passed);
            let room = DESCRIPTION_LIMIT.saturating_sub(head.len() + tail.len());
            let shown = if name.chars().count() > room {
                let cut: String = name.chars().take(room.saturating_sub(1)).collect();
                format!("{cut}…")
            } else {
                name.clone()
            };
            format!("{head}{shown}{tail}")
        }
    }
}

/// One backstop workflow's latest settled run on main.
#[derive(Debug, PartialEq)]
pub enum Backstop {
    Green {
        sha: String,
    },
    Red {
        run_id: u64,
        url: String,
        sha: String,
        workflow: String,
    },
    CancelledOnly {
        run_id: u64,
        url: String,
    },
}

/// Read `gh run list --json databaseId,conclusion,headSha,url` (newest first)
/// and `gh run view <id> --json jobs` for the newest run that has finished.
/// A failed job makes it red; cancelled or timed-out jobs alone do not. The
/// workflow's aggregate job, named in `AGGREGATE_JOBS`, is left out: it fails
/// whenever a job under it was cancelled, so the jobs under it are what count.
pub fn read_backstop(workflow: &str, run_json: &str, jobs_json: &str) -> Result<Backstop, String> {
    let runs: Value = serde_json::from_str(run_json)
        .map_err(|e| format!("{workflow}: runs are not JSON: {e}"))?;
    let run = runs
        .as_array()
        .and_then(|runs| {
            runs.iter().find(|r| {
                r.get("conclusion")
                    .and_then(Value::as_str)
                    .is_some_and(|c| !c.is_empty())
            })
        })
        .ok_or_else(|| format!("{workflow}: no finished run"))?;
    let run_id = run
        .get("databaseId")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{workflow}: run has no databaseId"))?;
    let text = |key: &str| {
        run.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let (url, sha) = (text("url"), text("headSha"));

    let view: Value = serde_json::from_str(jobs_json)
        .map_err(|e| format!("{workflow}: jobs are not JSON: {e}"))?;
    let jobs = view
        .get("jobs")
        .and_then(Value::as_array)
        .filter(|jobs| !jobs.is_empty())
        .ok_or_else(|| format!("{workflow}: run {run_id} has no jobs"))?;
    let aggregate = AGGREGATE_JOBS
        .iter()
        .find(|(file, _)| *file == workflow)
        .map(|(_, job)| *job);
    let conclusions: Vec<&str> = jobs
        .iter()
        .filter(|j| aggregate.is_none() || j.get("name").and_then(Value::as_str) != aggregate)
        .map(|j| {
            j.get("conclusion")
                .and_then(Value::as_str)
                .unwrap_or_default()
        })
        .collect();
    if conclusions.is_empty() {
        return Err(format!(
            "{workflow}: run {run_id} has no jobs but its aggregate one"
        ));
    }
    if conclusions.contains(&"failure") {
        Ok(Backstop::Red {
            run_id,
            url,
            sha,
            workflow: workflow.to_owned(),
        })
    } else if conclusions
        .iter()
        .any(|c| matches!(*c, "cancelled" | "timed_out"))
    {
        Ok(Backstop::CancelledOnly { run_id, url })
    } else if conclusions
        .iter()
        .all(|c| matches!(*c, "success" | "skipped"))
    {
        Ok(Backstop::Green { sha })
    } else {
        Err(format!(
            "{workflow}: run {run_id} has jobs with unsettled conclusions"
        ))
    }
}

/// Main's state across every backstop workflow.
#[derive(Debug, PartialEq)]
pub enum MainState {
    Red(Vec<Backstop>),
    Green {
        sha: String,
    },
    /// Neither red nor wholly green: say nothing on the issue.
    Unsettled,
}

pub fn combine(each: &[Backstop]) -> MainState {
    let red: Vec<Backstop> = each
        .iter()
        .filter_map(|b| match b {
            Backstop::Red {
                run_id,
                url,
                sha,
                workflow,
            } => Some(Backstop::Red {
                run_id: *run_id,
                url: url.clone(),
                sha: sha.clone(),
                workflow: workflow.clone(),
            }),
            _ => None,
        })
        .collect();
    if !red.is_empty() {
        return MainState::Red(red);
    }
    let mut shas = Vec::new();
    for b in each {
        match b {
            Backstop::Green { sha } => shas.push(sha),
            _ => return MainState::Unsettled,
        }
    }
    match shas.split_first() {
        Some((first, rest)) if rest.iter().all(|s| s == first) => MainState::Green {
            sha: (*first).clone(),
        },
        _ => MainState::Unsettled,
    }
}

/// Held while a gate builds: an OS advisory lock on `gate.lock` in a
/// directory. The OS drops it when the holder exits, however it exits, so a
/// crashed gate never blocks the next one.
#[derive(Debug)]
pub struct Lock {
    _file: fs::File,
}

impl Lock {
    /// Take the lock, polling every `poll` while another gate holds it and
    /// saying once who does (the pid inside the file is information only).
    pub fn acquire(dir: &Path, poll: Duration, say: impl Fn(&str)) -> Result<Lock, String> {
        let path = dir.join("gate.lock");
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let mut said = false;
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) => {
                    if !said {
                        said = true;
                        let owner = fs::read_to_string(&path)
                            .ok()
                            .and_then(|t| t.trim().parse::<u32>().ok());
                        let who = owner.map_or_else(
                            || "another gate".to_owned(),
                            |p| format!("another gate (pid {p})"),
                        );
                        say(&format!("{who} holds the lock; waiting"));
                    }
                    std::thread::sleep(poll);
                }
                Err(TryLockError::Error(e)) => {
                    return Err(format!("cannot lock {}: {e}", path.display()));
                }
            }
        }
        file.set_len(0)
            .and_then(|()| file.write_all(std::process::id().to_string().as_bytes()))
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        Ok(Lock { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    fn rows(pairs: &[(&str, &str)]) -> String {
        let steps: Vec<String> = pairs
            .iter()
            .map(|(n, o)| format!(r#"{{"name":"{n}","outcome":"{o}"}}"#))
            .collect();
        format!(r#"{{"steps":[{}]}}"#, steps.join(","))
    }

    #[test]
    fn the_summary_file_is_read_into_a_verdict() {
        let ok = rows(&[
            ("fmt", "passed"),
            ("clippy", "passed"),
            ("doc", "passed"),
            ("deny", "ci-only"),
        ]);
        assert_eq!(
            read_summary(&ok),
            Ok(Verdict {
                passed: 3,
                failed: None
            })
        );
        let bad = rows(&[
            ("fmt", "passed"),
            ("test-default", "failed"),
            ("doc", "skipped"),
        ]);
        assert_eq!(
            read_summary(&bad),
            Ok(Verdict {
                passed: 1,
                failed: Some("test-default".into())
            })
        );
        assert!(read_summary("nope").is_err());
    }

    #[test]
    fn a_passing_description_names_steps_time_and_main() {
        let v = Verdict {
            passed: 3,
            failed: None,
        };
        assert_eq!(
            description(&v, Duration::from_secs(65), "09c2efb0123456"),
            "passed 3 steps in 1m 05s on main 09c2efb"
        );
    }

    #[test]
    fn a_failing_description_names_the_step() {
        let v = Verdict {
            passed: 31,
            failed: Some("test-default".into()),
        };
        let text = description(&v, Duration::from_secs(18 * 60 + 2), "09c2efb0");
        assert_eq!(
            text,
            "failed at test-default after 31 steps, 18m 02s, on main 09c2efb"
        );
    }

    #[test]
    fn a_description_never_exceeds_140_characters() {
        let v = Verdict {
            passed: 1,
            failed: Some("x".repeat(200)),
        };
        let text = description(&v, Duration::from_secs(5), "09c2efb0");
        assert!(text.chars().count() <= 140, "{}", text.chars().count());
        assert!(text.contains('…'));
    }

    fn red() -> Backstop {
        Backstop::Red {
            run_id: 1,
            url: "u".into(),
            sha: "a".into(),
            workflow: "ci".into(),
        }
    }

    #[test]
    fn one_red_workflow_makes_main_red_whatever_the_other_says() {
        let green = || Backstop::Green { sha: "b".into() };
        assert!(matches!(combine(&[red(), green()]), MainState::Red(r) if r.len() == 1));
        assert_eq!(
            combine(&[green(), green()]),
            MainState::Green { sha: "b".into() }
        );
        let cancelled = Backstop::CancelledOnly {
            run_id: 2,
            url: "u".into(),
        };
        assert_eq!(combine(&[green(), cancelled]), MainState::Unsettled);
        assert_eq!(combine(&[]), MainState::Unsettled);
    }

    const RUNS: &str = r#"[{"databaseId":77,"conclusion":"failure","headSha":"abc123","url":"https://github.com/o/r/actions/runs/77"},{"databaseId":70,"conclusion":"success","headSha":"old","url":"x"}]"#;

    fn jobs(conclusions: &[&str]) -> String {
        let jobs: Vec<String> = conclusions
            .iter()
            .enumerate()
            .map(|(i, c)| format!(r#"{{"name":"job{i}","conclusion":"{c}"}}"#))
            .collect();
        format!(r#"{{"jobs":[{}]}}"#, jobs.join(","))
    }

    #[test]
    fn a_failed_job_is_red_and_a_cancelled_one_is_not() {
        assert_eq!(
            read_backstop("ci", RUNS, &jobs(&["success", "failure"])),
            Ok(Backstop::Red {
                run_id: 77,
                url: "https://github.com/o/r/actions/runs/77".into(),
                sha: "abc123".into(),
                workflow: "ci".into()
            })
        );
        assert_eq!(
            read_backstop("ci", RUNS, &jobs(&["cancelled", "timed_out"])),
            Ok(Backstop::CancelledOnly {
                run_id: 77,
                url: "https://github.com/o/r/actions/runs/77".into()
            })
        );
        assert_eq!(
            read_backstop("ci", RUNS, &jobs(&["success", "skipped"])),
            Ok(Backstop::Green {
                sha: "abc123".into()
            })
        );
        assert!(read_backstop("ci", "[]", &jobs(&["success"])).is_err());
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gate-lock-{tag}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_second_gate_waits_until_the_first_lock_is_dropped() {
        let dir = scratch("wait");
        let first = Lock::acquire(&dir, Duration::from_millis(50), |_| {}).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let said = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (flag, count, wdir) = (Arc::clone(&done), Arc::clone(&said), dir.clone());
        let waiter = std::thread::spawn(move || {
            let held = Lock::acquire(&wdir, Duration::from_millis(50), |m| {
                assert!(m.contains("holds the lock"), "{m}");
                count.fetch_add(1, Ordering::SeqCst);
            });
            flag.store(true, Ordering::SeqCst);
            held
        });
        std::thread::sleep(Duration::from_secs(1));
        assert!(
            !done.load(Ordering::SeqCst),
            "acquired while the first was held"
        );
        drop(first);
        let second = waiter.join().unwrap().unwrap();
        assert_eq!(said.load(Ordering::SeqCst), 1, "says who holds it once");
        drop(second);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_lock_file_nobody_holds_is_acquired_whatever_it_says() {
        for (tag, text) in [
            ("garbage", "not a pid"),
            ("empty", ""),
            ("dead", "999999"),
            ("zero", "0"),
        ] {
            let dir = scratch(tag);
            fs::write(dir.join("gate.lock"), text).unwrap();
            let held = Lock::acquire(&dir, Duration::from_millis(50), |_| {}).unwrap();
            drop(held);
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn a_summary_with_no_step_passed_or_failed_is_an_error() {
        let message = "the gate's run recorded no step that passed or failed";
        assert_eq!(read_summary(r#"{"steps":[]}"#), Err(message.to_owned()));
        let skipped = rows(&[("fmt", "skipped"), ("deny", "ci-only")]);
        assert_eq!(read_summary(&skipped), Err(message.to_owned()));
    }

    /// Run git in `dir` with a fixed identity and no hooks or signing.
    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@localhost",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(status.status.success(), "git {args:?}: {status:?}");
    }

    #[test]
    fn the_merge_conflict_is_reported_before_any_step() {
        let dir = scratch("merge");
        git(&dir, &["init", "--quiet", "--initial-branch=main"]);
        fs::write(dir.join("a.txt"), "base\n").unwrap();
        git(&dir, &["add", "a.txt"]);
        git(&dir, &["commit", "--quiet", "-m", "base"]);
        git(&dir, &["checkout", "--quiet", "-b", "branch"]);
        fs::write(dir.join("a.txt"), "branch\n").unwrap();
        git(&dir, &["commit", "--quiet", "-am", "branch"]);
        git(&dir, &["checkout", "--quiet", "main"]);
        fs::write(dir.join("a.txt"), "main\n").unwrap();
        git(&dir, &["commit", "--quiet", "-am", "main"]);
        git(&dir, &["update-ref", "refs/remotes/origin/main", "main"]);
        git(&dir, &["checkout", "--quiet", "branch"]);
        assert_eq!(
            merge_with_main(&dir, "origin/main"),
            Err("merge main into the branch first".to_owned())
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_fork_is_refused() {
        let refused = check_head_repository("someone/brightfield").unwrap_err();
        assert!(refused.contains("someone/brightfield"), "{refused}");
        assert_eq!(check_head_repository("meridian-online/brightfield"), Ok(()));
    }

    #[test]
    fn a_status_that_cannot_be_posted_is_an_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("post");
        let gh = dir.join("gh");
        fs::write(
            &gh,
            "#!/bin/sh\necho 'HTTP 401: Bad credentials' >&2\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
        let error = post_status(gh.as_os_str(), "abc123", "success", "passed").unwrap_err();
        assert!(error.contains("not posted"), "{error}");
        assert!(error.contains("Bad credentials"), "{error}");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A stand-in for the tree's `fetch-duckdb-cli.sh` whose pin is `pin`:
    /// `--check` accepts a directory whose `duckdb` holds that text, and a
    /// fetch writes it, logging each fetch.
    fn fetch_script(dir: &Path, pin: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("fetch-duckdb-cli.sh");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nif [ \"$1\" = --check ]; then [ \"$(cat \"$3/duckdb\" 2>/dev/null)\" = {pin} ]; exit $?; fi\necho fetched >> '{}'\nprintf {pin} > \"$2/duckdb\"\necho \"$2/duckdb\"\n",
                dir.join("fetches").display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[test]
    fn the_engine_is_judged_by_the_trees_pin_and_fetched_once_when_it_moves() {
        let dir = scratch("engine");
        let invoking = dir.join("invoking");
        fs::create_dir_all(&invoking).unwrap();
        fs::write(invoking.join("duckdb"), "v1").unwrap();
        let cache = dir.join("cache");

        // The branch keeps main's pin: the invoking checkout's engine serves.
        let same = fetch_script(&dir, "v1");
        let engine = engine_for_tree(&same, "host", &invoking.join("duckdb"), &cache).unwrap();
        assert_eq!(engine, invoking.join("duckdb"));
        assert!(!dir.join("fetches").exists());

        // The branch moves the pin: the tree's script fetches into the cache,
        // once, and a second gate on the same pin reuses that copy.
        let moved = fetch_script(&dir, "v2");
        for _ in 0..2 {
            let engine = engine_for_tree(&moved, "host", &invoking.join("duckdb"), &cache).unwrap();
            assert_eq!(engine, cache.join("duckdb"));
        }
        let fetches = fs::read_to_string(dir.join("fetches")).unwrap();
        assert_eq!(fetches.lines().count(), 1, "{fetches}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_tree_sits_beside_the_gate_target_so_relative_climbs_stay_short() {
        let gate_dir = Path::new("/home/someone/.cache/meridian/gate");
        let tree = tree_beside_target(gate_dir, "brightfield-gate-1-2");
        let cwd = tree.join("crates/brightfield-shell");
        let scratch = gate_dir.join("target/tmp/some-test");
        let shared = cwd
            .components()
            .zip(scratch.components())
            .take_while(|(a, b)| a == b)
            .count();
        // The climb from a test's working directory to its scratch file stops
        // at the gate's directory, never at the filesystem root.
        assert_eq!(shared, gate_dir.components().count());
        assert_eq!(cwd.components().count() - shared, 4);
    }

    /// A kata that answers `list` with one open issue, logs every call, and
    /// refuses a done close without evidence, as the real one does.
    fn strict_kata(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let kata = dir.join("kata");
        let log = dir.join("calls");
        fs::write(
            &kata,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$1\" in\n  list) echo '{{\"issues\":[{{\"short_id\":\"red1\"}}]}}' ;;\n  close) case \"$*\" in *--commit*) ;; *) echo 'evidence required for reason=done' >&2; exit 3 ;; esac ;;\nesac\n",
                log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&kata, fs::Permissions::from_mode(0o755)).unwrap();
        kata
    }

    #[test]
    fn a_green_main_closes_the_red_issue_with_its_commit_and_notes_it_once() {
        let dir = scratch("green-close");
        let kata = strict_kata(&dir);
        note_green_with(kata.as_os_str(), "abc1234def");
        let calls = fs::read_to_string(dir.join("calls")).unwrap();
        let lines: Vec<&str> = calls.lines().collect();
        assert_eq!(lines.len(), 2, "{calls}");
        assert!(lines[0].starts_with("list "), "{calls}");
        assert!(lines[1].starts_with("close red1 --done"), "{calls}");
        assert!(lines[1].contains("--commit abc1234def"), "{calls}");
        assert!(
            lines[1].contains("--comment main is green at abc1234"),
            "{calls}"
        );
        assert!(!calls.contains("\ncomment "), "{calls}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_status_post_that_hangs_is_stopped_and_is_an_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("post-hang");
        let gh = dir.join("gh");
        fs::write(&gh, "#!/bin/sh\nexec sleep 30\n").unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
        let started = Instant::now();
        let error = post_status_within(
            gh.as_os_str(),
            "abc123",
            "success",
            "passed",
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(10), "{error}");
        assert!(error.contains("not posted"), "{error}");
        assert!(error.contains("timed out"), "{error}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_cancelled_cache_job_under_a_red_aggregate_is_cancelled_only() {
        // Shaped like main's run 43951187684: warm-cache cancelled, the shards
        // skipped, and only the aggregate `test` job failed.
        let runs = r#"[{"databaseId":43951187684,"conclusion":"failure","headSha":"09c2efb","url":"https://github.com/o/r/actions/runs/43951187684"}]"#;
        let jobs = r#"{"jobs":[
            {"name":"Packaging stages the type source and the DuckDB CLI into both artifacts","conclusion":"cancelled"},
            {"name":"The packaged artifact carries a type source (disk image)","conclusion":"success"},
            {"name":"warm the build cache","conclusion":"cancelled"},
            {"name":"The air-gap read jail refuses reads outside its grant","conclusion":"success"},
            {"name":"test","conclusion":"failure"},
            {"name":"test shard ${{ matrix.name }}","conclusion":"skipped"}]}"#;
        assert_eq!(
            read_backstop("test.yml", runs, jobs),
            Ok(Backstop::CancelledOnly {
                run_id: 43951187684,
                url: "https://github.com/o/r/actions/runs/43951187684".into()
            })
        );
        // A failed leaf job is still red, aggregate or not.
        let red_leaf = jobs.replace(
            r#""warm the build cache","conclusion":"cancelled""#,
            r#""warm the build cache","conclusion":"failure""#,
        );
        assert!(matches!(
            read_backstop("test.yml", runs, &red_leaf),
            Ok(Backstop::Red { .. })
        ));
        // In a workflow with no aggregate job, a job named `test` is a leaf.
        assert!(matches!(
            read_backstop("lint.yml", runs, jobs),
            Ok(Backstop::Red { .. })
        ));
    }

    #[test]
    fn a_failure_posts_a_fixed_phrase_and_no_local_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("phrase");
        let gh = dir.join("gh");
        let said = dir.join("args");
        fs::write(
            &gh,
            format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n", said.display()),
        )
        .unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
        let failure = Failure::new(
            Stage::Worktree,
            "cannot create /var/folders/k0/T/brightfield-gate-1-2/home: Permission denied",
        );
        let code = post_failure(gh.as_os_str(), "abc123", &failure);
        assert_eq!(code, ExitCode::FAILURE);
        let args = fs::read_to_string(&said).unwrap();
        let posted = args
            .lines()
            .find_map(|line| line.strip_prefix("description="))
            .expect("a description was posted");
        assert_eq!(posted, "could not make the worktree");
        assert!(!posted.contains('/'), "{posted}");
        assert!(args.lines().any(|line| line == "state=failure"), "{args}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_merge_that_must_make_a_commit_is_not_refused_by_merge_ff_only() {
        let dir = scratch("ff-only");
        git(&dir, &["init", "--quiet", "--initial-branch=main"]);
        fs::write(dir.join("a.txt"), "base\n").unwrap();
        git(&dir, &["add", "a.txt"]);
        git(&dir, &["commit", "--quiet", "-m", "base"]);
        git(&dir, &["checkout", "--quiet", "-b", "branch"]);
        fs::write(dir.join("b.txt"), "branch\n").unwrap();
        git(&dir, &["add", "b.txt"]);
        git(&dir, &["commit", "--quiet", "-m", "branch"]);
        git(&dir, &["checkout", "--quiet", "main"]);
        fs::write(dir.join("c.txt"), "main\n").unwrap();
        git(&dir, &["add", "c.txt"]);
        git(&dir, &["commit", "--quiet", "-m", "main"]);
        git(&dir, &["update-ref", "refs/remotes/origin/main", "main"]);
        git(&dir, &["checkout", "--quiet", "branch"]);
        git(&dir, &["config", "merge.ff", "only"]);
        assert_eq!(merge_with_main(&dir, "origin/main"), Ok(()));
        assert!(dir.join("c.txt").exists(), "main's change is merged in");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_child_loses_build_flags_and_keeps_the_job_cap() {
        for key in [
            "RUSTFLAGS",
            "RUSTDOCFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
            "RUSTC_WRAPPER",
            "CARGO_BUILD_TARGET",
            "CARGO_BUILD_RUSTFLAGS",
            "RUSTUP_TOOLCHAIN",
            "XDG_CONFIG_HOME",
            "CXXFLAGS",
            "CC",
            "MACOSX_DEPLOYMENT_TARGET",
            "HOST_CXXFLAGS",
            "TARGET_CC",
            "CXXFLAGS_aarch64_apple_darwin",
            "CC_aarch64-apple-darwin",
        ] {
            assert!(removed_from_child(key), "{key} should not reach the run");
        }
        for key in [
            "CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS",
            "CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER",
            "CARGO_PROFILE_DEV_OPT_LEVEL",
            "CARGO_PROFILE_RELEASE_LTO",
        ] {
            assert!(removed_from_child(key), "{key} should not reach the run");
        }
        for key in [
            "PATH",
            "CARGO_BUILD_JOBS",
            "RUST_TEST_THREADS",
            "HOME",
            "CARGO_TARGET_DIR",
            "ARC_DUCKDB_BIN",
            "CCACHE_DIR",
        ] {
            assert!(!removed_from_child(key), "{key} should reach the run");
        }
    }

    #[test]
    fn a_call_that_outlasts_its_timeout_is_killed_and_named() {
        let started = Instant::now();
        let error = output_within("sleep", &["30"], Duration::from_millis(200)).unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "it waited out the sleep"
        );
        assert!(error.contains("`sleep 30`"), "{error}");
        assert!(error.contains("timed out"), "{error}");
        assert_eq!(
            output_within("echo", &["hello"], Duration::from_secs(10)),
            Ok("hello\n".to_owned())
        );
    }

    #[test]
    fn green_on_different_shas_is_unsettled() {
        let on = |sha: &str| Backstop::Green { sha: sha.into() };
        assert_eq!(combine(&[on("a"), on("b")]), MainState::Unsettled);
        assert_eq!(
            combine(&[on("a"), on("a")]),
            MainState::Green { sha: "a".into() }
        );
    }
}
