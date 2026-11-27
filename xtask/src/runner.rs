//! Running steps in order, and the summary a run ends with.
//!
//! The rule is the one a pull request's jobs follow inside each job: the first
//! step that fails ends the run, and the steps after it do not start. A
//! nightly run instead carries on past a failure, skipping only the steps that
//! need what failed, so one red step does not hide the state of the rest. The
//! summary names every step in the order the table lists them, with what
//! happened to each, so a red run says which step failed and which never ran.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use crate::steps::Ctx;

/// One step of the gate.
pub struct Step {
    /// The name a workflow invokes it by: `cargo xtask ci --step <name>`.
    pub name: &'static str,
    /// What the step runs, in a line, for the step's header and `--list`.
    pub runs: &'static str,
    /// Set when the step has no local form, with the reason. A full run lists
    /// such a step without running it; `--step <name>` runs it.
    pub only_in_ci: Option<&'static str>,
    /// Set when the step is too slow or too heavy for a run on demand, with the
    /// reason. Only a nightly run starts it; `--step <name>` runs it too.
    pub only_at_night: Option<&'static str>,
    /// The step itself. `Err` carries the line the summary prints for it.
    pub run: fn(&Ctx) -> Result<(), String>,
}

/// What happened to one step in a run.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Passed(Duration),
    Failed(Duration, String),
    /// Not started, because a step before it failed.
    Skipped,
    /// Not started, because it runs only in CI; carries the reason.
    OnlyInCi(&'static str),
    /// Not started, because it runs only in a nightly run; carries the reason.
    OnlyAtNight(&'static str),
}

/// How a full run treats a failure and the nightly-only steps. The two are
/// separate fields so tests can exercise each alone; `ci --nightly` sets both.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RunMode {
    /// Carry on past a failure instead of skipping every later step.
    pub keep_going: bool,
    /// Start the steps that run only at night.
    pub nightly: bool,
}

/// The steps that cannot run unless an earlier step passed, as
/// (step, the step it needs). A run that keeps going skips a step whose entry
/// names a step that failed or was skipped. The two test steps run on the
/// target directory the warm-cache step fills; the managed DuckDB link test
/// links the library the duckdb-managed step builds, and the Store package is
/// built on it.
pub const NEEDS: &[(&str, &str)] = &[
    ("test-one-thread", "warm-cache"),
    ("test-default", "warm-cache"),
    ("managed-duckdb-test", "duckdb-managed"),
    ("package-store-selftest", "duckdb-managed"),
];

/// True when `step` needs a step that, in `rows` so far, failed or was skipped.
fn blocked(step: &Step, rows: &[Row]) -> bool {
    NEEDS
        .iter()
        .filter(|(name, _)| *name == step.name)
        .any(|(_, needed)| {
            rows.iter().any(|row| {
                row.name == *needed && matches!(row.outcome, Outcome::Failed(..) | Outcome::Skipped)
            })
        })
}

/// The outcome of a step that does not start in this run, if it is one.
fn not_started(step: &Step, mode: RunMode) -> Option<Outcome> {
    step.only_in_ci.map(Outcome::OnlyInCi).or_else(|| {
        step.only_at_night
            .filter(|_| !mode.nightly)
            .map(Outcome::OnlyAtNight)
    })
}

/// One line of the summary.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub name: &'static str,
    pub outcome: Outcome,
}

/// Run `steps` in order through `run`. Unless `mode.keep_going`, the run stops
/// at the first failure: a step after it is recorded as skipped and `run` is
/// not called for it. With `keep_going`, a step is skipped when `NEEDS` names a
/// step that failed or was skipped, as
/// `keep_going_skips_only_what_needs_the_failed_step` checks.
pub fn run_in_order<F>(steps: &[Step], mode: RunMode, mut run: F) -> Vec<Row>
where
    F: FnMut(&Step) -> Result<(), String>,
{
    let mut rows: Vec<Row> = Vec::with_capacity(steps.len());
    let mut failed = false;
    for step in steps {
        let outcome = if failed || blocked(step, &rows) {
            Outcome::Skipped
        } else if let Some(outcome) = not_started(step, mode) {
            outcome
        } else {
            let started = Instant::now();
            match run(step) {
                Ok(()) => Outcome::Passed(started.elapsed()),
                Err(reason) => {
                    failed = !mode.keep_going;
                    Outcome::Failed(started.elapsed(), reason)
                }
            }
        };
        rows.push(Row {
            name: step.name,
            outcome,
        });
    }
    rows
}

/// Run one step through `run` and time it. A panic in the step is a failure
/// of that step, whichever thread it ran on.
fn timed<F>(step: &Step, run: &F) -> Outcome
where
    F: Fn(&Step) -> Result<(), String>,
{
    let started = Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(step)))
        .unwrap_or_else(|_| Err("the step panicked".to_owned()));
    match result {
        Ok(()) => Outcome::Passed(started.elapsed()),
        Err(reason) => Outcome::Failed(started.elapsed(), reason),
    }
}

/// Run `steps` as `run_in_order` does, except that the two steps named in
/// `pair` start together when the run reaches the first of them and both run
/// to completion. If either fails, the run stops after the pair unless
/// `mode.keep_going`: every step after it is skipped, and the other of the pair
/// is not cut short. Rows stay in table order. When either name is absent from
/// `steps`, or either step does not start in this run, there is no pair and the
/// rows are `run_in_order`'s, as `without_the_pair_named_the_run_is_run_in_order`
/// checks.
pub fn run_with_pair<F>(steps: &[Step], pair: (&str, &str), mode: RunMode, run: F) -> Vec<Row>
where
    F: Fn(&Step) -> Result<(), String> + Sync,
{
    let at = |name: &str| steps.iter().position(|step| step.name == name);
    let paired = match (at(pair.0), at(pair.1)) {
        (Some(a), Some(b))
            if a != b
                && not_started(&steps[a], mode).is_none()
                && not_started(&steps[b], mode).is_none() =>
        {
            Some((a.min(b), a.max(b)))
        }
        _ => None,
    };
    let Some((first, second)) = paired else {
        return run_in_order(steps, mode, |step| run(step));
    };

    let mut rows: Vec<Row> = Vec::with_capacity(steps.len());
    let mut failed = false;
    // The later step's outcome, held until the table reaches its row.
    let mut held: Option<Outcome> = None;
    for (index, step) in steps.iter().enumerate() {
        let outcome = if let Some(outcome) = held.take_if(|_| index == second) {
            outcome
        } else if failed || blocked(step, &rows) {
            Outcome::Skipped
        } else if let Some(outcome) = not_started(step, mode) {
            outcome
        } else if index == first {
            // The later step may be blocked where the first is not, so it is
            // judged here, from the rows so far, before the two start.
            let later_blocked = blocked(&steps[second], &rows);
            let (now, later) = std::thread::scope(|scope| {
                let other = (!later_blocked).then(|| scope.spawn(|| timed(&steps[second], &run)));
                let now = timed(step, &run);
                let later = match other {
                    Some(other) => other.join().unwrap_or_else(|_| {
                        Outcome::Failed(Duration::ZERO, "the step panicked".to_owned())
                    }),
                    None => Outcome::Skipped,
                };
                (now, later)
            });
            failed = !mode.keep_going
                && [&now, &later]
                    .iter()
                    .any(|outcome| matches!(outcome, Outcome::Failed(..)));
            held = Some(later);
            now
        } else {
            let outcome = timed(step, &run);
            failed = !mode.keep_going && matches!(outcome, Outcome::Failed(..));
            outcome
        };
        rows.push(Row {
            name: step.name,
            outcome,
        });
    }
    rows
}

/// Run one step, as `--step <name>` does. Unlike a full run, a step that runs
/// only in CI runs here too, since a workflow is what names a step.
pub fn run_named<F>(step: &Step, run: F) -> Row
where
    F: FnOnce(&Step) -> Result<(), String>,
{
    let started = Instant::now();
    let outcome = match run(step) {
        Ok(()) => Outcome::Passed(started.elapsed()),
        Err(reason) => Outcome::Failed(started.elapsed(), reason),
    };
    Row {
        name: step.name,
        outcome,
    }
}

/// True when no row failed.
pub fn passed(rows: &[Row]) -> bool {
    !rows
        .iter()
        .any(|row| matches!(row.outcome, Outcome::Failed(..)))
}

/// The summary a run prints last: one line per step in the order run, then the
/// steps that run only in CI and why, then the verdict.
pub fn summary(rows: &[Row], steps: &[Step]) -> String {
    let width = rows.iter().map(|row| row.name.len()).max().unwrap_or(0);
    let mut out = String::from("cargo xtask ci: summary, in the order the steps ran\n");
    let mut total = Duration::ZERO;
    for row in rows {
        let (word, time, note) = match &row.outcome {
            Outcome::Passed(took) => {
                total += *took;
                ("ok", wall(*took), String::new())
            }
            Outcome::Failed(took, reason) => {
                total += *took;
                ("FAILED", wall(*took), format!("  {reason}"))
            }
            Outcome::Skipped => ("skipped", String::new(), String::new()),
            Outcome::OnlyInCi(_) => (
                "ci only",
                String::new(),
                "  not run here; see below".to_owned(),
            ),
            Outcome::OnlyAtNight(_) => (
                "nightly",
                String::new(),
                "  not run here; see below".to_owned(),
            ),
        };
        let line = format!("  {word:<8} {time:>8}  {:<width$}{note}", row.name);
        let _ = writeln!(out, "{}", line.trim_end());
    }
    for step in steps {
        if let Some(why) = step.only_in_ci {
            let _ = writeln!(out, "{} runs only in CI: {why}", step.name);
        }
    }
    // A nightly run starts these, so only a run that did not is told why.
    for row in rows {
        if let Outcome::OnlyAtNight(why) = row.outcome {
            let _ = writeln!(out, "{} runs only in the nightly run: {why}", row.name);
        }
    }
    let ran = rows
        .iter()
        .filter(|row| matches!(row.outcome, Outcome::Passed(_)))
        .count();
    let failures: Vec<&str> = rows
        .iter()
        .filter(|row| matches!(row.outcome, Outcome::Failed(..)))
        .map(|row| row.name)
        .collect();
    // A run that carried on past its first failure has steps that ran after
    // it, so "passed before it" would misreport its passes; it gets the form
    // that counts failures instead. A run that stopped keeps the other form.
    let carried_on = failures.first().is_some_and(|first| {
        rows.iter()
            .skip_while(|row| row.name != *first)
            .skip(1)
            .any(|row| matches!(row.outcome, Outcome::Passed(_) | Outcome::Failed(..)))
    });
    if carried_on {
        let skipped = rows
            .iter()
            .filter(|row| row.outcome == Outcome::Skipped)
            .count();
        let _ = writeln!(
            out,
            "{} step(s) failed ({}); {skipped} skipped, {ran} passed, {} in all",
            failures.len(),
            failures.join(", "),
            wall(total)
        );
        return out;
    }
    match rows
        .iter()
        .position(|row| matches!(row.outcome, Outcome::Failed(..)))
    {
        Some(at) => {
            let skipped = rows[at + 1..]
                .iter()
                .filter(|row| row.outcome == Outcome::Skipped)
                .count();
            let _ = writeln!(
                out,
                "failed at {}; {skipped} later step(s) skipped, {ran} passed before it, {} in all",
                rows[at].name,
                wall(total)
            );
        }
        None => {
            let _ = writeln!(
                out,
                "all {ran} step(s) that run here passed, {} in all",
                wall(total)
            );
        }
    }
    out
}

/// The summary as JSON, for `ci --summary-json`: each row in order with its
/// outcome (`passed`, `failed`, `skipped` or `ci-only`), what the step runs,
/// its seconds when it ran, and the reason when it failed or is CI's or the nightly's alone,
/// as `the_summary_json_names_every_step_and_its_outcome` checks.
///
/// A step left out because only the nightly runs it is written as `skipped`
/// with `"nightly_only":true`, not as an outcome of its own. A gate started
/// from a checkout older than the nightly reads the merged tree's summary
/// with a reader that knows only the four outcomes above and refuses any
/// other, and it ignores a field it does not know, so this spelling keeps
/// such a gate reading every pull request's run.
pub fn summary_json(rows: &[Row], steps: &[Step]) -> String {
    let listed: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let mut entry = serde_json::Map::new();
            entry.insert("name".into(), row.name.into());
            let (outcome, took, reason) = match &row.outcome {
                Outcome::Passed(took) => ("passed", Some(*took), None),
                Outcome::Failed(took, reason) => ("failed", Some(*took), Some(reason.as_str())),
                Outcome::Skipped => ("skipped", None, None),
                Outcome::OnlyInCi(why) => ("ci-only", None, Some(*why)),
                Outcome::OnlyAtNight(why) => ("skipped", None, Some(*why)),
            };
            entry.insert("outcome".into(), outcome.into());
            if matches!(row.outcome, Outcome::OnlyAtNight(_)) {
                entry.insert("nightly_only".into(), true.into());
            }
            if let Some(step) = steps.iter().find(|step| step.name == row.name) {
                entry.insert("runs".into(), step.runs.into());
            }
            if let Some(took) = took {
                entry.insert("seconds".into(), took.as_secs_f64().into());
            }
            if let Some(reason) = reason {
                entry.insert("reason".into(), reason.into());
            }
            serde_json::Value::Object(entry)
        })
        .collect();
    serde_json::json!({ "steps": listed }).to_string()
}

/// A wall time as a person reads it: `0.4s`, `12.3s`, `4m 05s`, `1h 02m`.
pub fn wall(took: Duration) -> String {
    let secs = took.as_secs_f64();
    if secs < 60.0 {
        format!("{secs:.1}s")
    } else if secs < 3600.0 {
        let whole = took.as_secs();
        format!("{}m {:02}s", whole / 60, whole % 60)
    } else {
        let whole = took.as_secs();
        format!("{}h {:02}m", whole / 3600, (whole % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(_: &Ctx) -> Result<(), String> {
        Ok(())
    }

    fn step(name: &'static str) -> Step {
        Step {
            name,
            runs: "",
            only_in_ci: None,
            only_at_night: None,
            run: ok,
        }
    }

    fn nightly_only(name: &'static str) -> Step {
        Step {
            only_at_night: Some("it takes hours"),
            ..step(name)
        }
    }

    const STOP: RunMode = RunMode {
        keep_going: false,
        nightly: false,
    };

    fn fails(names: &'static [&'static str]) -> impl Fn(&Step) -> Result<(), String> {
        move |s| {
            if names.contains(&s.name) {
                Err(format!("{} broke", s.name))
            } else {
                Ok(())
            }
        }
    }

    fn ci_only(name: &'static str) -> Step {
        Step {
            only_in_ci: Some("it reads the event"),
            ..step(name)
        }
    }

    #[test]
    fn a_failure_stops_the_run_and_every_later_step_is_skipped_unstarted() {
        let steps = [step("a"), step("b"), step("c"), ci_only("d"), step("e")];
        let mut started = Vec::new();
        let rows = run_in_order(&steps, STOP, |s| {
            started.push(s.name);
            if s.name == "b" {
                Err("b broke".to_owned())
            } else {
                Ok(())
            }
        });
        assert_eq!(started, ["a", "b"], "no step after the failure may start");
        let words: Vec<_> = rows
            .iter()
            .map(|r| match &r.outcome {
                Outcome::Passed(_) => "ok",
                Outcome::Failed(_, reason) => {
                    assert_eq!(reason, "b broke");
                    "failed"
                }
                Outcome::Skipped => "skipped",
                Outcome::OnlyInCi(_) => "ci only",
                Outcome::OnlyAtNight(_) => "nightly",
            })
            .collect();
        assert_eq!(words, ["ok", "failed", "skipped", "skipped", "skipped"]);
        assert!(!passed(&rows));
    }

    #[test]
    fn a_clean_run_starts_every_local_step_in_order_and_lists_the_ci_only_one() {
        let steps = [step("a"), ci_only("b"), step("c")];
        let mut started = Vec::new();
        let rows = run_in_order(&steps, STOP, |s| {
            started.push(s.name);
            Ok(())
        });
        assert_eq!(
            started,
            ["a", "c"],
            "a CI-only step does not start in a full run"
        );
        assert!(matches!(rows[0].outcome, Outcome::Passed(_)));
        assert_eq!(rows[1].outcome, Outcome::OnlyInCi("it reads the event"));
        assert!(matches!(rows[2].outcome, Outcome::Passed(_)));
        assert!(passed(&rows));
    }

    #[test]
    fn naming_a_ci_only_step_runs_it_and_reports_what_it_returned() {
        let mut started = false;
        let row = run_named(&ci_only("d"), |_| {
            started = true;
            Ok(())
        });
        assert!(
            started,
            "a step named on the command line runs even when it is CI-only"
        );
        assert!(matches!(row.outcome, Outcome::Passed(_)));
        let row = run_named(&ci_only("d"), |_| Err("the scan found a name".to_owned()));
        assert!(
            matches!(&row.outcome, Outcome::Failed(_, reason) if reason == "the scan found a name"),
            "{:?}",
            row.outcome
        );
    }

    #[test]
    fn the_summary_names_each_step_in_order_the_failed_one_and_the_ci_only_reason() {
        let steps = [
            step("first"),
            step("second"),
            ci_only("third"),
            step("fourth"),
        ];
        let rows = vec![
            Row {
                name: "first",
                outcome: Outcome::Passed(Duration::from_millis(1500)),
            },
            Row {
                name: "second",
                outcome: Outcome::Failed(Duration::from_secs(75), "exit status: 1".to_owned()),
            },
            Row {
                name: "third",
                outcome: Outcome::Skipped,
            },
            Row {
                name: "fourth",
                outcome: Outcome::Skipped,
            },
        ];
        let text = summary(&rows, &steps);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[1].contains("ok") && lines[1].contains("1.5s") && lines[1].contains("first"));
        assert!(
            lines[2].contains("FAILED")
                && lines[2].contains("1m 15s")
                && lines[2].contains("second")
        );
        assert!(lines[2].contains("exit status: 1"));
        assert!(lines[3].contains("skipped") && lines[3].contains("third"));
        assert!(lines[4].contains("skipped") && lines[4].contains("fourth"));
        assert!(text.contains("third runs only in CI: it reads the event"));
        assert!(text.contains("failed at second; 2 later step(s) skipped, 1 passed before it"));
    }

    #[test]
    fn the_summary_json_names_every_step_and_its_outcome() {
        let steps = [
            Step {
                runs: "the a check",
                ..step("a")
            },
            step("b"),
            ci_only("c"),
            step("d"),
        ];
        let rows = vec![
            Row {
                name: "a",
                outcome: Outcome::Passed(Duration::from_millis(1500)),
            },
            Row {
                name: "b",
                outcome: Outcome::Failed(Duration::from_secs(2), "b broke".to_owned()),
            },
            Row {
                name: "c",
                outcome: Outcome::OnlyInCi("it reads the event"),
            },
            Row {
                name: "d",
                outcome: Outcome::Skipped,
            },
        ];
        let doc: serde_json::Value = serde_json::from_str(&summary_json(&rows, &steps)).unwrap();
        let listed: Vec<(&str, &str)> = doc["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["name"].as_str().unwrap(), s["outcome"].as_str().unwrap()))
            .collect();
        assert_eq!(
            listed,
            [
                ("a", "passed"),
                ("b", "failed"),
                ("c", "ci-only"),
                ("d", "skipped")
            ]
        );
        assert_eq!(doc["steps"][0]["runs"], "the a check");
        assert_eq!(doc["steps"][0]["seconds"], 1.5);
        assert_eq!(doc["steps"][1]["reason"], "b broke");
        assert_eq!(doc["steps"][2]["reason"], "it reads the event");
    }

    #[test]
    fn wall_times_read_in_seconds_minutes_and_hours() {
        assert_eq!(wall(Duration::from_millis(400)), "0.4s");
        assert_eq!(wall(Duration::from_secs(245)), "4m 05s");
        assert_eq!(wall(Duration::from_secs(3720)), "1h 02m");
    }

    #[test]
    fn a_pair_starts_together_and_both_finish_when_one_fails() {
        let steps = [step("a"), step("b"), step("c"), step("d")];
        let started = std::sync::Mutex::new(Vec::new());
        let rows = run_with_pair(&steps, ("b", "c"), STOP, |s| {
            started.lock().unwrap().push(s.name);
            if s.name == "b" {
                Err("b broke".to_owned())
            } else {
                Ok(())
            }
        });
        let mut started = started.into_inner().unwrap();
        started.sort_unstable();
        assert_eq!(started, ["a", "b", "c"], "d must not start");
        let names: Vec<_> = rows.iter().map(|r| r.name).collect();
        assert_eq!(names, ["a", "b", "c", "d"]);
        assert!(matches!(rows[0].outcome, Outcome::Passed(_)));
        assert!(matches!(&rows[1].outcome, Outcome::Failed(_, r) if r == "b broke"));
        assert!(matches!(rows[2].outcome, Outcome::Passed(_)));
        assert_eq!(rows[3].outcome, Outcome::Skipped);
        assert!(!passed(&rows));
    }

    #[test]
    fn a_panic_in_the_first_of_the_pair_is_a_failed_row_and_the_second_still_finishes() {
        let steps = [step("a"), step("b"), step("c"), step("d")];
        let rows = run_with_pair(&steps, ("b", "c"), STOP, |s| {
            if s.name == "b" {
                panic!("b blew up");
            }
            Ok(())
        });
        assert!(matches!(rows[0].outcome, Outcome::Passed(_)));
        assert!(matches!(rows[1].outcome, Outcome::Failed(..)), "{rows:?}");
        assert!(matches!(rows[2].outcome, Outcome::Passed(_)));
        assert_eq!(rows[3].outcome, Outcome::Skipped);
    }

    #[test]
    fn a_pair_runs_concurrently() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let steps = [step("a"), step("b"), step("c")];
        let arrived = AtomicUsize::new(0);
        let rows = run_with_pair(&steps, ("b", "c"), STOP, |s| {
            if s.name == "a" {
                return Ok(());
            }
            // Each step waits for the other to arrive; run one after the
            // other, the first would wait out the deadline and fail.
            arrived.fetch_add(1, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(5);
            while arrived.load(Ordering::SeqCst) < 2 {
                if Instant::now() > deadline {
                    return Err(format!("{} ran alone", s.name));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(())
        });
        assert!(passed(&rows), "{rows:?}");
        assert!(matches!(rows[1].outcome, Outcome::Passed(_)));
        assert!(matches!(rows[2].outcome, Outcome::Passed(_)));
    }

    #[test]
    fn without_the_pair_named_the_run_is_run_in_order() {
        let steps = [step("a"), ci_only("b"), step("c"), step("d")];
        let behave = |s: &Step| {
            if s.name == "c" {
                Err("c broke".to_owned())
            } else {
                Ok(())
            }
        };
        let ordered = run_in_order(&steps, STOP, behave);
        let paired = run_with_pair(&steps, ("x", "y"), STOP, behave);
        let shape = |rows: &[Row]| -> Vec<String> {
            rows.iter()
                .map(|r| match &r.outcome {
                    Outcome::Passed(_) => format!("{} ok", r.name),
                    Outcome::Failed(_, why) => format!("{} failed {why}", r.name),
                    Outcome::Skipped => format!("{} skipped", r.name),
                    Outcome::OnlyInCi(why) => format!("{} ci only {why}", r.name),
                    Outcome::OnlyAtNight(why) => format!("{} nightly {why}", r.name),
                })
                .collect()
        };
        assert_eq!(shape(&paired), shape(&ordered));
        // A CI-only member of the pair leaves no pair either.
        let ci_member = run_with_pair(&steps, ("b", "c"), STOP, behave);
        assert_eq!(shape(&ci_member), shape(&ordered));
    }

    #[test]
    fn keep_going_runs_past_a_failure_and_records_every_outcome() {
        let steps = [step("a"), step("b"), step("c")];
        let mode = RunMode {
            keep_going: true,
            nightly: false,
        };
        let mut calls = 0;
        let behave = fails(&["b"]);
        let rows = run_in_order(&steps, mode, |s| {
            calls += 1;
            behave(s)
        });
        assert_eq!(calls, 3);
        assert!(matches!(rows[0].outcome, Outcome::Passed(_)));
        assert!(matches!(&rows[1].outcome, Outcome::Failed(_, r) if r == "b broke"));
        assert!(matches!(rows[2].outcome, Outcome::Passed(_)));
        assert!(!passed(&rows));
    }

    #[test]
    fn keep_going_skips_only_what_needs_the_failed_step() {
        let steps = [
            step("warm-cache"),
            step("test-one-thread"),
            step("test-default"),
            step("doc"),
        ];
        let mode = RunMode {
            keep_going: true,
            nightly: false,
        };
        let mut started = Vec::new();
        let behave = fails(&["warm-cache"]);
        let rows = run_in_order(&steps, mode, |s| {
            started.push(s.name);
            behave(s)
        });
        assert_eq!(started, ["warm-cache", "doc"]);
        assert!(matches!(rows[0].outcome, Outcome::Failed(..)));
        assert_eq!(rows[1].outcome, Outcome::Skipped);
        assert_eq!(rows[2].outcome, Outcome::Skipped);
        assert!(matches!(rows[3].outcome, Outcome::Passed(_)));
    }

    #[test]
    fn keep_going_skips_the_link_test_when_the_managed_build_failed() {
        let steps = [
            step("duckdb-managed"),
            step("managed-duckdb-test"),
            step("doc"),
        ];
        let mode = RunMode {
            keep_going: true,
            nightly: true,
        };
        let rows = run_in_order(&steps, mode, fails(&["duckdb-managed"]));
        assert!(matches!(rows[0].outcome, Outcome::Failed(..)));
        assert_eq!(rows[1].outcome, Outcome::Skipped);
        assert!(matches!(rows[2].outcome, Outcome::Passed(_)));
    }

    #[test]
    fn keep_going_with_the_pair_skips_the_pair_when_its_cache_step_failed() {
        let steps = [
            step("warm-cache"),
            step("test-one-thread"),
            step("test-default"),
            step("doc"),
        ];
        let mode = RunMode {
            keep_going: true,
            nightly: false,
        };
        let rows = run_with_pair(
            &steps,
            ("test-one-thread", "test-default"),
            mode,
            fails(&["warm-cache"]),
        );
        assert!(matches!(rows[0].outcome, Outcome::Failed(..)));
        assert_eq!(rows[1].outcome, Outcome::Skipped);
        assert_eq!(rows[2].outcome, Outcome::Skipped);
        assert!(matches!(rows[3].outcome, Outcome::Passed(_)));
        // A failure inside the pair does not stop the step after it either.
        let rows = run_with_pair(
            &steps,
            ("test-one-thread", "test-default"),
            mode,
            fails(&["test-default"]),
        );
        assert!(matches!(rows[1].outcome, Outcome::Passed(_)));
        assert!(matches!(rows[2].outcome, Outcome::Failed(..)));
        assert!(matches!(rows[3].outcome, Outcome::Passed(_)));
    }

    #[test]
    fn a_nightly_only_step_is_listed_not_run_outside_the_nightly() {
        let steps = [step("a"), nightly_only("b")];
        let mut started = Vec::new();
        let rows = run_in_order(&steps, STOP, |s| {
            started.push(s.name);
            Ok(())
        });
        assert_eq!(started, ["a"]);
        assert_eq!(rows[1].outcome, Outcome::OnlyAtNight("it takes hours"));
        assert!(passed(&rows));
        let night = RunMode {
            keep_going: false,
            nightly: true,
        };
        let mut started = Vec::new();
        let rows = run_in_order(&steps, night, |s| {
            started.push(s.name);
            Ok(())
        });
        assert_eq!(started, ["a", "b"]);
        assert!(matches!(rows[1].outcome, Outcome::Passed(_)));
    }

    #[test]
    fn the_summary_json_names_a_nightly_only_step() {
        let steps = [nightly_only("a")];
        let rows = vec![Row {
            name: "a",
            outcome: Outcome::OnlyAtNight("it takes hours"),
        }];
        let doc: serde_json::Value = serde_json::from_str(&summary_json(&rows, &steps)).unwrap();
        assert_eq!(doc["steps"][0]["outcome"], "skipped");
        assert_eq!(doc["steps"][0]["nightly_only"], true);
        assert_eq!(doc["steps"][0]["reason"], "it takes hours");
        assert!(summary(&rows, &steps).contains("a runs only in the nightly run: it takes hours"));
    }

    fn row(name: &'static str, outcome: Outcome) -> Row {
        Row { name, outcome }
    }

    #[test]
    fn a_keep_going_summary_with_one_failure_counts_failures_not_passes_before_it() {
        let steps = [step("a"), step("b"), step("c")];
        let rows = vec![
            row("a", Outcome::Passed(Duration::from_secs(1))),
            row(
                "b",
                Outcome::Failed(Duration::from_secs(1), "b broke".to_owned()),
            ),
            row("c", Outcome::Passed(Duration::from_secs(1))),
        ];
        let text = summary(&rows, &steps);
        assert!(
            text.contains("1 step(s) failed (b); 0 skipped, 2 passed"),
            "{text}"
        );
        assert!(!text.contains("passed before it"), "{text}");
    }

    #[test]
    fn a_keep_going_summary_names_every_failure() {
        let steps = [step("a"), step("b"), step("c")];
        let rows = vec![
            row(
                "a",
                Outcome::Failed(Duration::from_secs(1), "a broke".to_owned()),
            ),
            row("b", Outcome::Skipped),
            row(
                "c",
                Outcome::Failed(Duration::from_secs(1), "c broke".to_owned()),
            ),
        ];
        let text = summary(&rows, &steps);
        assert!(
            text.contains("2 step(s) failed (a, c); 1 skipped, 0 passed"),
            "{text}"
        );
    }

    #[test]
    fn a_stopped_summary_with_one_failure_keeps_its_passed_before_it_line() {
        let steps = [step("a"), step("b"), step("c")];
        let rows = vec![
            row("a", Outcome::Passed(Duration::from_secs(1))),
            row(
                "b",
                Outcome::Failed(Duration::from_secs(1), "b broke".to_owned()),
            ),
            row("c", Outcome::Skipped),
        ];
        let text = summary(&rows, &steps);
        assert!(
            text.contains("failed at b; 1 later step(s) skipped, 1 passed before it"),
            "{text}"
        );
    }
}
