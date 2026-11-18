//! The local pull-request gate: `cargo xtask gate <pr>` runs the pull
//! request's own `cargo xtask ci`, merged with main, in a clean worktree and an
//! isolated environment, and posts the verdict as the `local-gate` commit
//! status on the pull request's head.
//!
//! The order of a run: take the gate lock; read main's newest nightly verdict
//! and keep the `main-red` and `nightly-missing` issues in step with it;
//! refuse a fork's head; fetch, make the worktree and merge main into it; post
//! `pending`; run the tree's gate with its output teed to a log; post
//! `success` or `failure`; clean up.
//! Every failure after the head is known is posted as a `failure` status, and
//! a run that leaves no summary is a failure, never a pass.

use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::flags::{self, MAIN_RED, NIGHTLY_MISSING};
use crate::isolate::{isolated_ci, run_teed, tree_beside_target, Lock, Scratch, Unready};
use crate::nightly_status::{self, is_missing, read_newest_verdict, Seen, VerdictState};
use crate::runner::wall;

/// The repository whose pull requests the gate runs, and whose branches only.
pub(crate) const REPO: &str = "meridian-online/brightfield";
/// The commit status context the gate posts.
const CONTEXT: &str = "local-gate";
/// The commit status context the nightly gate posts on main, whose newest
/// verdict says whether main is red.
const NIGHTLY_CONTEXT: &str = "nightly-gate";

fn say(message: &str) {
    eprintln!("cargo xtask gate: {message}");
}

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
    let tree = tree_beside_target(&gate_dir, &run);
    let scratch = Scratch {
        root: root.clone(),
        dir: std::env::temp_dir().join(&run),
        tree: Some(tree.clone()),
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
        &tree,
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
    if let Err(e) = nightly_status::post_status(gh, &head, CONTEXT, posted.state, &posted.text) {
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
    match nightly_status::post_status(gh, head, CONTEXT, "failure", text) {
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

/// Build the tree to test, run its gate, and say what to post.
fn gate_head(
    pr: u64,
    paths: &Paths,
    head: &str,
    scratch: &Scratch,
    tree: &Path,
) -> Result<Posted, Stop> {
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

    let tree_text = tree.display().to_string();
    git_in_root(&["worktree", "add", "--quiet", "--detach", &tree_text, head])
        .map_err(fail(Stage::Worktree))?;
    merge_with_main(tree, "origin/main").map_err(fail(Stage::Merge))?;

    let summary = scratch.dir.join("summary.json");
    let mut command =
        isolated_ci(root, tree, paths.home, paths.gate_dir, &scratch.dir).map_err(|unready| {
            match unready {
                Unready::Scratch(e) => fail(Stage::Worktree)(e),
                Unready::DuckDb(e) => fail(Stage::DuckDb)(e),
                Unready::OpenAnalytics(e) => fail(Stage::OpenAnalytics)(e),
            }
        })?;
    command.arg("--summary-json").arg(&summary);

    let main7: String = main_sha.chars().take(7).collect();
    nightly_status::post_status(
        OsStr::new("gh"),
        head,
        CONTEXT,
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

/// How long a `gh`, `kata` or `git` call may take before the gate gives up on
/// it. The backstop's calls and the status posts run under the gate lock, so a
/// hung one must not hold the lock for good.
pub(crate) const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Run a program and return what it printed; a non-zero exit is the error,
/// with what it said, and so is a call that outlasts `CALL_TIMEOUT`.
pub(crate) fn output_of(program: impl AsRef<OsStr>, args: &[&str]) -> Result<String, String> {
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
pub(crate) fn output_within(
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

/// Read main's newest nightly verdict and keep the `main-red` and
/// `nightly-missing` issues in step with it. Nothing here stops the gate: a red
/// or missing nightly does not block a pull request, since the fix goes
/// forward, and a failed read or issue call is printed and passed over.
fn check_backstop() {
    let query = format!("query={}", nightly_status::query(NIGHTLY_CONTEXT));
    let seen = output_of("gh", &["api", "graphql", "-f", &query])
        .and_then(|json| read_newest_verdict(&json, NIGHTLY_CONTEXT));
    let seen = match seen {
        Ok(seen) => seen,
        Err(e) => {
            say(&format!("main's nightly verdict could not be read: {e}"));
            return;
        }
    };
    if let Seen::Verdict {
        sha,
        state,
        description,
        ..
    } = &seen
    {
        let sha7: String = sha.chars().take(7).collect();
        match state {
            VerdictState::Failure => {
                say("main is red; the gate carries on, since the fix goes forward");
                flags::raise(
                    &MAIN_RED,
                    &format!("the nightly gate failed on main at {sha7}: {description}"),
                );
            }
            VerdictState::Success => {
                flags::clear(&MAIN_RED, sha, &format!("main is green at {sha7}"));
            }
        }
    }
    // A clock before 1970 reads as the epoch, which makes a verdict look
    // recent rather than raising a flag on a broken clock.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    if is_missing(&seen, now) {
        let since = match &seen {
            Seen::Verdict { at, .. } => at.get(..10).unwrap_or("ever"),
            Seen::None => "ever",
        };
        say(&format!(
            "main's nightly has not given a verdict since {since}"
        ));
        // The day it was read makes the comment new once a day, so a nightly
        // that stays missing is noted each day rather than on every gate.
        let today = match output_of("date", &["+%Y-%m-%d"]) {
            Ok(day) => day.trim().to_owned(),
            Err(e) => {
                say(&format!("could not read today's date: {e}"));
                "an unknown day".to_owned()
            }
        };
        // The gate never clears this flag; only the nightly's own verdict does.
        flags::raise(
            &NIGHTLY_MISSING,
            &format!("no nightly verdict on main since {since}, read {today}"),
        );
    }
}

/// GitHub caps a commit status description at 140 characters.
const DESCRIPTION_LIMIT: usize = 140;

/// What a full `ci` run came to: steps passed, the first step that failed,
/// and, for the nightly, which carries on past a failure, every step that
/// passed and every step that failed, in the order run.
#[derive(Debug, Default, PartialEq)]
pub struct Verdict {
    pub passed: usize,
    pub failed: Option<String>,
    pub passed_all: Vec<String>,
    pub failed_all: Vec<String>,
}

/// Read the summary file `ci --summary-json` writes:
/// `{"steps":[{"name":"fmt","outcome":"passed"}, ...]}`.
pub fn read_summary(json: &str) -> Result<Verdict, String> {
    let doc: Value = serde_json::from_str(json).map_err(|e| format!("summary is not JSON: {e}"))?;
    let steps = doc
        .get("steps")
        .and_then(Value::as_array)
        .ok_or("summary has no `steps` list")?;
    let mut passed_all = Vec::new();
    let mut failed_all = Vec::new();
    for step in steps {
        let name = step.get("name").and_then(Value::as_str);
        let outcome = step.get("outcome").and_then(Value::as_str);
        match (name, outcome) {
            (Some(name), Some("passed")) => passed_all.push(name.to_owned()),
            (Some(name), Some("failed")) => failed_all.push(name.to_owned()),
            (Some(_), Some("skipped" | "ci-only" | "nightly-only")) => {}
            _ => return Err(format!("summary has a step it cannot read: {step}")),
        }
    }
    if passed_all.is_empty() && failed_all.is_empty() {
        return Err("the gate's run recorded no step that passed or failed".to_owned());
    }
    Ok(Verdict {
        passed: passed_all.len(),
        failed: failed_all.first().cloned(),
        passed_all,
        failed_all,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
                failed: None,
                passed_all: vec!["fmt".into(), "clippy".into(), "doc".into()],
                failed_all: vec![],
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
                failed: Some("test-default".into()),
                passed_all: vec!["fmt".into()],
                failed_all: vec!["test-default".into()],
            })
        );
        assert!(read_summary("nope").is_err());
    }

    #[test]
    fn a_passing_description_names_steps_time_and_main() {
        let v = Verdict {
            passed: 3,
            ..Verdict::default()
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
            ..Verdict::default()
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
            ..Verdict::default()
        };
        let text = description(&v, Duration::from_secs(5), "09c2efb0");
        assert!(text.chars().count() <= 140, "{}", text.chars().count());
        assert!(text.contains('…'));
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gate-lock-{tag}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_summary_with_a_nightly_only_step_reads() {
        let json = rows(&[("fmt", "passed"), ("duckdb-managed", "nightly-only")]);
        assert_eq!(
            read_summary(&json),
            Ok(Verdict {
                passed: 1,
                failed: None,
                passed_all: vec!["fmt".into()],
                failed_all: vec![],
            })
        );
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
}
