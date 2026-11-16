//! The pure pieces of the local pull-request gate: the status text, the
//! reading of a `ci --summary-json` file and of the backstop workflows' runs,
//! and the lock that keeps two gates from building at once.

use std::fs::{self, TryLockError};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::runner::wall;

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
/// A failed job makes it red; cancelled or timed-out jobs alone do not.
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
    let conclusions: Vec<&str> = jobs
        .iter()
        .map(|j| {
            j.get("conclusion")
                .and_then(Value::as_str)
                .unwrap_or_default()
        })
        .collect();
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
