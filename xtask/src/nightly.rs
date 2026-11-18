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
        }
    }
}

/// How long to wait for the gate lock when the local time is `now_local`
/// (hour, minute): until 05:30, nothing from then until noon, and from noon
/// until the next day's 05:30.
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

/// The `nightly-gate` and `nightly-network` statuses a run's summary gives:
/// the gate passes when every step but `NETWORK_STEP` passed, and a run
/// without a readable summary is a gate failure. The network status is
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
    let day = today();
    let stopped = |stage: NightStage, detail: &str| stop(kata, trial, &day, stage, detail);
    let root = crate::repo_root();
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
        );
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
        trial,
        &gate,
        network.as_ref(),
        &log,
    )
}

/// Say why the nightly stopped before its steps, and raise `nightly-missing`
/// with the stage's fixed phrase, unless this is a trial. No status is
/// posted, so the commit keeps its last real verdict.
fn stop(kata: &OsStr, trial: bool, day: &str, stage: NightStage, detail: &str) -> ExitCode {
    say(detail);
    let phrase = stage.phrase();
    println!("the nightly stopped before its steps: {phrase}");
    if !trial {
        flags::raise_with(
            kata,
            &NIGHTLY_MISSING,
            &format!("the nightly stopped on {day}: {phrase}"),
        );
    }
    ExitCode::FAILURE
}

/// Post the run's statuses on `sha` and, unless this is a trial, keep the
/// flags in step with them. Exits 0 only on two successes.
fn report(
    gh: &OsStr,
    kata: &OsStr,
    sha: &str,
    trial: bool,
    gate: &Status,
    network: Option<&Status>,
    log: &Path,
) -> ExitCode {
    let sha7: String = sha.chars().take(7).collect();
    let mut posted = true;
    for (context, (state, text)) in [(GATE_CONTEXT, Some(gate)), (NETWORK_CONTEXT, network)]
        .into_iter()
        .filter_map(|(context, status)| status.map(|status| (context, status)))
    {
        match nightly_status::post_status(gh, sha, context, state, text) {
            Ok(()) => println!("{context} {state} on {sha7}: {text}"),
            Err(e) => {
                say(&e);
                posted = false;
            }
        }
    }
    println!("the full log is {}", log.display());
    if !trial {
        let log = log.display();
        match gate.0 {
            "success" => {
                flags::clear_with(kata, &MAIN_RED, sha, &format!("main is green at {sha7}"))
            }
            _ => flags::raise_with(
                kata,
                &MAIN_RED,
                &format!(
                    "the nightly gate failed on main at {sha7}: {}; log {log}",
                    gate.1
                ),
            ),
        }
        match network.map(|(state, _)| *state) {
            Some("success") => flags::clear_with(
                kata,
                &NIGHTLY_NETWORK,
                sha,
                &format!("the network tests passed on main at {sha7}"),
            ),
            Some(_) => flags::raise_with(
                kata,
                &NIGHTLY_NETWORK,
                &format!("the network tests failed on main at {sha7}; log {log}"),
            ),
            None => {}
        }
        flags::clear_with(
            kata,
            &NIGHTLY_MISSING,
            sha,
            &format!("the nightly gave a verdict on main at {sha7}"),
        );
    }
    let green = gate.0 == "success" && network.is_some_and(|(state, _)| *state == "success");
    if posted && green {
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
