//! The nightly gate: `cargo xtask nightly --sha <sha>` runs the tree's own
//! `cargo xtask ci --nightly` on main's head, under the pull-request gate's
//! lock and isolated environment, and posts its verdict on that commit as the
//! `nightly-gate` and `nightly-network` commit statuses.
//!
//! The order of a run: wait for the gate lock until 05:30 local time; find the
//! pinned DuckDB CLI and the open-analytics checkout; run the tree's
//! `cargo xtask ci --nightly` with its output teed to a log; post both
//! statuses; keep the `main-red`, `nightly-network` and `nightly-missing`
//! issues in step. The run is started from the tree it gates, a worktree the
//! launcher made at main's head, so it makes no worktree of its own.
//!
//! Nothing is posted before the verdict: GitHub keeps one status per context
//! on a commit, so a `pending` at the start, or an `error` after an early
//! stop, would overwrite an earlier night's verdict on the same head. A run
//! that stops before its steps posts nothing and raises `nightly-missing`
//! instead; a run whose steps left no summary posts a `nightly-gate` failure.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime};

use crate::flags::{self, MAIN_RED, NIGHTLY_MISSING, NIGHTLY_NETWORK};
use crate::gate::{output_of, read_summary, Verdict};
use crate::isolate::{isolated_ci, run_teed, Lock, Scratch, Unready};
use crate::nightly_status;
use crate::runner::wall;

/// The commit status context for the gate's verdict, which says whether main
/// is red.
const GATE_CONTEXT: &str = "nightly-gate";
/// The commit status context for the network tests' verdict, which does not
/// make main red.
const NETWORK_CONTEXT: &str = "nightly-network";
/// The `cargo xtask ci` step whose verdict is posted as `NETWORK_CONTEXT`
/// rather than as part of the gate's.
const NETWORK_STEP: &str = "nightly-network";
/// The cargo job and test thread caps a Claude Code session runs with, which
/// the nightly keeps to so it takes no more than one build slot.
const CAP: &str = "6";
/// GitHub caps a commit status description at 140 characters.
const DESCRIPTION_LIMIT: usize = 140;
/// The minute of the day the lock wait ends, 05:30 local, so a full run can
/// still end before the working day.
const WAIT_ENDS: u32 = 5 * 60 + 30;
/// From noon on, a nightly started by hand waits until the next 05:30.
const NOON: u32 = 12 * 60;
/// The wait when the local time cannot be read: a 02:30 run's usual wait.
const USUAL_WAIT: Duration = Duration::from_secs(3 * 60 * 60);
/// The status a run gets whose summary shows no failed step though the run
/// exited non-zero.
const EXITED_NON_ZERO: &str = "the nightly's run failed though no step did";

/// A commit status to post: its state and description.
pub type Status = (&'static str, String);

fn say(message: &str) {
    eprintln!("cargo xtask nightly: {message}");
}

/// Where a nightly stopped before its steps gave a verdict.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NightStage {
    Lock,
    OpenAnalytics,
    DuckDb,
    Start,
    NoSummary,
    Tree,
}

impl NightStage {
    /// The text for the stage. It is fixed, since the status it can become is
    /// public and the error behind it can carry the machine's paths.
    pub fn phrase(self) -> &'static str {
        match self {
            NightStage::Lock => "the gate lock was still held at 05:30",
            NightStage::OpenAnalytics => "the open-analytics checkout is missing",
            NightStage::DuckDb => "the pinned DuckDB CLI is missing",
            NightStage::Start => "could not start the nightly's run",
            NightStage::NoSummary => "the nightly's run left no summary",
            NightStage::Tree => "the nightly's tree did not match the commit it was asked to gate",
        }
    }
}

/// How long to wait for the gate lock when the local time is `now_local`
/// (hour, minute): until 05:30, no wait from then until noon, and from noon
/// until the next day's 05:30, as `the_wait_ends_at_half_past_five` checks.
pub fn deadline_after(now_local: (u32, u32)) -> Duration {
    let (hour, minute) = now_local;
    let now = (hour * 60 + minute).min(24 * 60 - 1);
    let minutes = if now <= WAIT_ENDS {
        WAIT_ENDS - now
    } else if now < NOON {
        0
    } else {
        24 * 60 - now + WAIT_ENDS
    };
    Duration::from_secs(u64::from(minutes) * 60)
}

/// The local time as (hour, minute), from `date +%H%M`.
fn local_time() -> Result<(u32, u32), String> {
    let text = output_of("date", &["+%H%M"])?;
    let text = text.trim();
    let parsed = (text.len() == 4)
        .then(|| Some((text.get(..2)?.parse().ok()?, text.get(2..)?.parse().ok()?)))
        .flatten();
    parsed.ok_or_else(|| format!("`date +%H%M` printed {text:?}, not a time"))
}

/// Today's local date as `YYYYMMDD`, from `date +%Y%m%d`.
fn today() -> String {
    match output_of("date", &["+%Y%m%d"]) {
        Ok(day) => day.trim().to_owned(),
        Err(e) => {
            say(&format!("could not read today's date: {e}"));
            "undated".to_owned()
        }
    }
}

/// The `nightly-gate` and `nightly-network` statuses a run's summary gives.
/// A failed `NETWORK_STEP` fails its own status and leaves the gate's, as
/// `network_failures_post_their_own_status_and_leave_the_gate_green` checks,
/// and a run without a readable summary is a gate failure. The network status is
/// `None` when the summary shows `NETWORK_STEP` neither passing nor failing,
/// as `a_run_without_a_summary_is_a_failure` checks for a run with no summary.
pub fn statuses_of(summary: Option<&Verdict>, took: Duration) -> (Status, Option<Status>) {
    let Some(verdict) = summary else {
        return (("failure", NightStage::NoSummary.phrase().to_owned()), None);
    };
    let time = wall(took);
    let failed: Vec<&String> = verdict
        .failed_all
        .iter()
        .filter(|name| *name != NETWORK_STEP)
        .collect();
    let gate = match failed.split_first() {
        None => (
            "success",
            format!("passed {} steps in {time}", verdict.passed),
        ),
        Some((first, rest)) => {
            let head = "failed at ";
            let more = if rest.is_empty() {
                String::new()
            } else {
                format!(" and {} more", rest.len())
            };
            let tail = format!("{more} after {} steps, {time}", verdict.passed);
            let room = DESCRIPTION_LIMIT.saturating_sub(head.len() + tail.chars().count());
            let shown = if first.chars().count() > room {
                let cut: String = first.chars().take(room.saturating_sub(1)).collect();
                format!("{cut}…")
            } else {
                (*first).clone()
            };
            ("failure", format!("{head}{shown}{tail}"))
        }
    };
    let network = if verdict.failed_all.iter().any(|name| name == NETWORK_STEP) {
        Some((
            "failure",
            "the network tests failed; they read published data and remote hosts".to_owned(),
        ))
    } else if verdict.passed_all.iter().any(|name| name == NETWORK_STEP) {
        Some(("success", "the network tests passed".to_owned()))
    } else {
        None
    };
    (gate, network)
}

/// Run the nightly gate on `sha`, the head of the tree this command was built
/// from, and post its verdict. A `trial` run posts both statuses but raises
/// and clears no flag, since its commit is not main's.
pub fn run(sha: &str, trial: bool) -> ExitCode {
    let kata = OsStr::new("kata");
    let root = crate::repo_root();
    let day = today();
    let stopped = |stage: NightStage, detail: &str| stop(kata, trial, &day, stage, detail);
    // A tree that is not the commit gives no verdict on it: refused before
    // anything is posted, as a stop before the steps.
    if let Err(code) = read_tree(&root, sha, |e| stopped(NightStage::Tree, e)) {
        return code;
    }
    let Some(home) = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
    else {
        return stopped(
            NightStage::Start,
            "HOME is not set, and the nightly keeps its lock, target and logs under it",
        );
    };
    let gate_dir = home.join(".cache/meridian/gate");
    let logs = gate_dir.join("logs");
    if let Err(e) = fs::create_dir_all(&logs) {
        return stopped(
            NightStage::Start,
            &format!("cannot create {}: {e}", logs.display()),
        );
    }
    let wait = local_time().map(deadline_after).unwrap_or_else(|e| {
        say(&format!(
            "could not read the local time, so the lock wait is the usual {}: {e}",
            wall(USUAL_WAIT)
        ));
        USUAL_WAIT
    });
    let _lock = match Lock::acquire_until(
        &gate_dir,
        Duration::from_secs(30),
        SystemTime::now() + wait,
        say,
    ) {
        Ok(lock) => lock,
        Err(e) => return stopped(NightStage::Lock, &e),
    };

    let sha7: String = sha.chars().take(7).collect();
    let log = logs.join(format!("nightly-{sha7}-{day}.log"));
    let scratch = Scratch {
        root: root.clone(),
        dir: std::env::temp_dir().join(format!("brightfield-nightly-{}", std::process::id())),
        tree: None,
    };
    let mut command = match isolated_ci(&root, &root, &home, &gate_dir, &scratch.dir) {
        Ok(command) => command,
        Err(Unready::Scratch(e)) => return stopped(NightStage::Start, &e),
        Err(Unready::DuckDb(e)) => return stopped(NightStage::DuckDb, &e),
        Err(Unready::OpenAnalytics(e)) => return stopped(NightStage::OpenAnalytics, &e),
    };
    let summary = scratch.dir.join("summary.json");
    command
        .arg("--nightly")
        .arg("--summary-json")
        .arg(&summary)
        .env("CARGO_BUILD_JOBS", CAP)
        .env("RUST_TEST_THREADS", CAP)
        .env(
            "BRIGHTFIELD_DUCKDB_MANAGED_DIR",
            gate_dir.join("duckdb-managed"),
        )
        // The parity check's two arc builds, one compiling DuckDB, kept
        // between nights as the managed build is.
        .env("BRIGHTFIELD_PARITY_DIR", gate_dir.join("duckdb-parity"));
    say(&format!(
        "running the nightly on {sha7}; the log is {}",
        log.display()
    ));
    let started = Instant::now();
    let status = match run_teed(command, &log) {
        Ok(status) => status,
        Err(e) => return stopped(NightStage::Start, &e),
    };
    let took = started.elapsed();
    let verdict = match fs::read_to_string(&summary)
        .map_err(|e| format!("cannot read {}: {e}", summary.display()))
        .and_then(|text| read_summary(&text))
    {
        Ok(verdict) => Some(verdict),
        Err(e) => {
            say(&e);
            None
        }
    };
    drop(scratch);
    let (mut gate, network) = statuses_of(verdict.as_ref(), took);
    if verdict.as_ref().is_some_and(|v| v.failed_all.is_empty()) && !status.success() {
        say(&format!(
            "the nightly's run exited with {status} though no step failed"
        ));
        gate = ("failure", EXITED_NON_ZERO.to_owned());
    }
    report(
        OsStr::new("gh"),
        kata,
        sha,
        &day,
        trial,
        &gate,
        network.as_ref(),
        &log,
    )
}

/// Refuse a tree that is not the commit the nightly is to post on: `head` is
/// the tree's `git rev-parse HEAD` and `changes` its `git status
/// --porcelain`. A status then names the commit whose code ran, as
/// `a_tree_that_is_not_the_commit_is_refused` checks.
pub fn check_tree(sha: &str, head: &str, changes: &str) -> Result<(), String> {
    let head = head.trim();
    if !head.eq_ignore_ascii_case(sha) {
        return Err(format!(
            "the tree's head is {head}, not {sha}: run the nightly from a tree at the commit it posts on"
        ));
    }
    if !changes.trim().is_empty() {
        return Err(format!(
            "the tree at {sha} has uncommitted changes, so its run would not be that commit's:\n{}",
            changes.trim_end()
        ));
    }
    Ok(())
}

/// Read the tree at `root` and hand `refuse` the reason unless it is `sha`,
/// unchanged.
fn read_tree(
    root: &Path,
    sha: &str,
    refuse: impl FnOnce(&str) -> ExitCode,
) -> Result<(), ExitCode> {
    let root = root.display().to_string();
    let checked = output_of("git", &["-C", &root, "rev-parse", "HEAD"]).and_then(|head| {
        let changes = output_of("git", &["-C", &root, "status", "--porcelain"])?;
        check_tree(sha, &head, &changes)
    });
    checked.map_err(|e| refuse(&e))
}

/// Say why the nightly stopped before its steps, and raise `nightly-missing`
/// with the stage's fixed phrase, unless this is a trial. No status is
/// posted, so the commit keeps its last real verdict.
fn stop(kata: &OsStr, trial: bool, day: &str, stage: NightStage, detail: &str) -> ExitCode {
    say(detail);
    let phrase = stage.phrase();
    println!("the nightly stopped before its steps: {phrase}");
    if !trial {
        let key = format!("the nightly stopped on {day}");
        flags::raise_with(kata, &NIGHTLY_MISSING, &key, &format!("{key}: {phrase}"));
    }
    ExitCode::FAILURE
}

/// Post the run's statuses on `sha` and, unless this is a trial, keep the
/// flags in step with the statuses that were posted. A flag follows only a
/// posted status, so it never says what no reader of the commit can see; a
/// `nightly-gate` that could not be posted raises `nightly-missing` instead,
/// as `a_verdict_that_could_not_be_posted_moves_no_flag_but_missing` checks.
/// Exits 0 only on two successes, both posted.
#[allow(clippy::too_many_arguments)]
fn report(
    gh: &OsStr,
    kata: &OsStr,
    sha: &str,
    day: &str,
    trial: bool,
    gate: &Status,
    network: Option<&Status>,
    log: &Path,
) -> ExitCode {
    let sha7: String = sha.chars().take(7).collect();
    let post = |context: &str, (state, text): &Status| match nightly_status::post_status(
        gh, sha, context, state, text,
    ) {
        Ok(()) => {
            println!("{context} {state} on {sha7}: {text}");
            true
        }
        Err(e) => {
            say(&e);
            false
        }
    };
    let gate_posted = post(GATE_CONTEXT, gate);
    let network_posted = network.map(|status| post(NETWORK_CONTEXT, status));
    println!("the full log is {}", log.display());
    if !trial {
        let log = log.display();
        if gate_posted {
            if gate.0 == "success" {
                flags::clear_with(kata, &MAIN_RED, sha, &format!("main is green at {sha7}"));
            } else {
                let key = flags::main_red_key(&sha7);
                flags::raise_with(
                    kata,
                    &MAIN_RED,
                    &key,
                    &format!("the {key}: {}; log {log}", gate.1),
                );
            }
            flags::clear_with(
                kata,
                &NIGHTLY_MISSING,
                sha,
                &format!("the nightly gave a verdict on main at {sha7}"),
            );
        } else {
            let key = format!("the nightly could not post its verdict on main at {sha7} on {day}");
            flags::raise_with(kata, &NIGHTLY_MISSING, &key, &format!("{key}; log {log}"));
        }
        match (network, network_posted) {
            (Some(("success", _)), Some(true)) => flags::clear_with(
                kata,
                &NIGHTLY_NETWORK,
                sha,
                &format!("the network tests passed on main at {sha7}"),
            ),
            (Some(_), Some(true)) => {
                let key = format!("the network tests failed on main at {sha7}");
                flags::raise_with(kata, &NIGHTLY_NETWORK, &key, &format!("{key}; log {log}"));
            }
            _ => {}
        }
    }
    let green = gate.0 == "success" && network.is_some_and(|(state, _)| *state == "success");
    if gate_posted && network_posted == Some(true) && green {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::read_summary;
    use crate::isolate::Lock;
    use std::fs;
    use std::time::{Duration, Instant, SystemTime};

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nightly-{tag}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rows(pairs: &[(&str, &str)]) -> String {
        let steps: Vec<String> = pairs
            .iter()
            .map(|(n, o)| format!(r#"{{"name":"{n}","outcome":"{o}"}}"#))
            .collect();
        format!(r#"{{"steps":[{}]}}"#, steps.join(","))
    }

    #[test]
    fn a_lock_wait_past_the_deadline_gives_up() {
        let dir = scratch("deadline");
        let held = Lock::acquire(&dir, Duration::from_millis(10), |_| {}).unwrap();
        let started = Instant::now();
        let deadline = SystemTime::now() + Duration::from_millis(100);
        let waited = Lock::acquire_until(&dir, Duration::from_millis(10), deadline, |_| {});
        assert!(waited.is_err(), "acquired a lock another holds");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "waited too long"
        );
        drop(held);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_wait_ends_at_half_past_five() {
        assert_eq!(deadline_after((2, 30)), Duration::from_secs(3 * 60 * 60));
        assert_eq!(deadline_after((5, 0)), Duration::from_secs(30 * 60));
        assert_eq!(deadline_after((6, 0)), Duration::ZERO);
        assert_eq!(
            deadline_after((13, 0)),
            Duration::from_secs((16 * 60 + 30) * 60)
        );
        assert_eq!(
            deadline_after((23, 59)),
            Duration::from_secs((5 * 60 + 31) * 60)
        );
    }

    #[test]
    fn network_failures_post_their_own_status_and_leave_the_gate_green() {
        let verdict = read_summary(&rows(&[
            ("fmt", "passed"),
            ("clippy", "passed"),
            ("nightly-network", "failed"),
            ("duckdb-managed", "passed"),
        ]))
        .unwrap();
        let (gate, network) = statuses_of(Some(&verdict), Duration::from_secs(60));
        assert_eq!(gate.0, "success", "{gate:?}");
        assert_eq!(network.map(|(state, _)| state), Some("failure"));
    }

    #[test]
    fn a_failed_gate_step_names_the_first_and_counts_the_rest() {
        let verdict = read_summary(&rows(&[
            ("fmt", "passed"),
            ("clippy", "failed"),
            ("warm-cache", "passed"),
            ("test-default", "failed"),
            ("nightly-network", "passed"),
        ]))
        .unwrap();
        let (gate, network) = statuses_of(Some(&verdict), Duration::from_secs(52 * 60 + 10));
        assert_eq!(gate.0, "failure");
        assert!(gate.1.starts_with("failed at clippy"), "{}", gate.1);
        assert!(gate.1.contains("and 1 more"), "{}", gate.1);
        assert!(gate.1.chars().count() <= 140, "{}", gate.1);
        assert_eq!(network.map(|(state, _)| state), Some("success"));
    }

    /// A shim at `dir/name` that appends its arguments, one call per line,
    /// to `dir/name.calls`, and prints an empty issue list.
    fn shim(dir: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\necho '{{\"issues\":[]}}'\n",
                dir.join(format!("{name}.calls")).display()
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn calls(dir: &Path, name: &str) -> String {
        fs::read_to_string(dir.join(format!("{name}.calls"))).unwrap_or_default()
    }

    #[test]
    fn a_trial_posts_both_statuses_and_touches_no_flag() {
        let dir = scratch("trial");
        let (gh, kata) = (shim(&dir, "gh"), shim(&dir, "kata"));
        let gate = (
            "failure",
            "failed at clippy after 3 steps, 1m 00s".to_owned(),
        );
        let network = ("success", "the network tests passed".to_owned());
        let code = report(
            gh.as_os_str(),
            kata.as_os_str(),
            "0123456789abcdef0123456789abcdef01234567",
            "20261118",
            true,
            &gate,
            Some(&network),
            Path::new("nightly.log"),
        );
        assert_eq!(code, ExitCode::FAILURE);
        let posted = calls(&dir, "gh");
        assert!(posted.contains("context=nightly-gate"), "{posted}");
        assert!(posted.contains("context=nightly-network"), "{posted}");
        assert!(!posted.contains("state=pending"), "{posted}");
        assert_eq!(calls(&dir, "kata"), "", "a trial touches no flag");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_verdict_keeps_each_flag_in_step() {
        let dir = scratch("flags");
        let (gh, kata) = (shim(&dir, "gh"), shim(&dir, "kata"));
        let gate = ("success", "passed 30 steps in 50m 00s".to_owned());
        let network = ("failure", "the network tests failed".to_owned());
        let code = report(
            gh.as_os_str(),
            kata.as_os_str(),
            "0123456789abcdef0123456789abcdef01234567",
            "20261118",
            false,
            &gate,
            Some(&network),
            Path::new("nightly.log"),
        );
        assert_eq!(
            code,
            ExitCode::FAILURE,
            "a red network run is not two successes"
        );
        let asked = calls(&dir, "kata");
        for label in ["main-red", "nightly-network", "nightly-missing"] {
            assert!(
                asked.contains(&format!("--label {label}")),
                "{label}: {asked}"
            );
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_verdict_that_could_not_be_posted_moves_no_flag_but_missing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("unposted");
        let kata = shim(&dir, "kata");
        let gh = dir.join("gh");
        // A refusal, which the post does not try again, so the test does not
        // sit through the pauses between tries.
        fs::write(&gh, "#!/bin/sh\necho 'HTTP 403: Forbidden' >&2\nexit 1\n").unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
        let code = report(
            gh.as_os_str(),
            kata.as_os_str(),
            "0123456789abcdef0123456789abcdef01234567",
            "20261118",
            false,
            &("success", "passed 30 steps in 50m 00s".to_owned()),
            Some(&("failure", "the network tests failed".to_owned())),
            Path::new("nightly.log"),
        );
        assert_eq!(code, ExitCode::FAILURE);
        let asked = calls(&dir, "kata");
        assert!(asked.contains("--label nightly-missing"), "{asked}");
        assert!(!asked.contains("--label main-red"), "{asked}");
        assert!(!asked.contains("--label nightly-network"), "{asked}");
        assert!(!asked.lines().any(|c| c.starts_with("close ")), "{asked}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_tree_that_is_not_the_commit_is_refused() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(check_tree(sha, &format!("{sha}\n"), ""), Ok(()));
        let other = "fedcba9876543210fedcba9876543210fedcba98";
        assert!(check_tree(sha, other, "").unwrap_err().contains(other));
        let dirty = check_tree(sha, sha, " M xtask/src/main.rs\n").unwrap_err();
        assert!(dirty.contains("uncommitted"), "{dirty}");
        assert!(check_tree(sha, sha, "?? new.rs\n").is_err());
    }

    #[test]
    fn a_tree_that_is_not_the_commit_raises_the_missing_flag_unless_a_trial() {
        let dir = scratch("tree-stop");
        let kata = shim(&dir, "kata");
        let tree = dir.join("tree");
        fs::create_dir_all(&tree).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(["-c", "user.name=test", "-c", "user.email=test@localhost"])
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .current_dir(&tree)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
        };
        git(&["init", "--quiet"]);
        git(&["commit", "--quiet", "--allow-empty", "-m", "base"]);
        let other = "fedcba9876543210fedcba9876543210fedcba98";
        let refuse = |trial: bool| {
            read_tree(&tree, other, |e| {
                stop(kata.as_os_str(), trial, "20261118", NightStage::Tree, e)
            })
        };
        assert_eq!(refuse(true), Err(ExitCode::FAILURE));
        assert_eq!(calls(&dir, "kata"), "", "a trial touches no flag");
        assert_eq!(refuse(false), Err(ExitCode::FAILURE));
        let asked = calls(&dir, "kata");
        assert!(asked.contains("--label nightly-missing"), "{asked}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_stop_before_the_steps_raises_the_missing_flag_unless_a_trial() {
        let dir = scratch("stop");
        let kata = shim(&dir, "kata");
        let code = stop(kata.as_os_str(), true, "20261118", NightStage::Lock, "held");
        assert_eq!(code, ExitCode::FAILURE);
        assert_eq!(calls(&dir, "kata"), "", "a trial touches no flag");
        stop(
            kata.as_os_str(),
            false,
            "20261118",
            NightStage::Lock,
            "held",
        );
        let asked = calls(&dir, "kata");
        assert!(asked.contains("--label nightly-missing"), "{asked}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_run_without_a_summary_is_a_failure() {
        let (gate, network) = statuses_of(None, Duration::from_secs(60));
        assert_eq!(gate.0, "failure");
        assert_eq!(gate.1, NightStage::NoSummary.phrase());
        assert_eq!(network, None, "no network verdict is known");
    }
}
