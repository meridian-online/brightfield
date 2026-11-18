//! The repository's gate as one command.
//!
//! `cargo xtask ci` runs what CI's checks run, in order except
//! that the two test steps run side by side, and stops at the first step that
//! fails. It ends with a summary that names each step in
//! the order run, with its wall time, the failed step, and the steps the
//! failure skipped. The workflow files call the same steps by name, one per
//! workflow step, as `cargo xtask ci --step <name>`, so what CI runs is this
//! command's list of steps, and a name the command does not carry is a red job
//! with a line naming it.
//!
//! The `workflow-coverage` step reads the workflow files and keeps the two
//! lists one list: it is red for a step no workflow invokes, for a workflow
//! invoking a step that does not exist, for a workflow that starts on its own,
//! on anything but a tag push, calling a gate script or cargo around the
//! command, and for a macOS job such a workflow runs. A pull request's checks
//! are `cargo xtask gate <pr>`, which runs this command on the reference Mac.
//! `main`'s backstop is `cargo xtask nightly`, which the reference Mac runs
//! every night with the nightly-only steps; the hosted macOS workflows run
//! only by hand or on a release tag.
//!
//! What a full run needs on the machine, each refused by name at its step when
//! absent: the pinned toolchain, `cargo-deny`, `python3`, the sibling
//! `open-analytics` checkout and the pinned DuckDB CLI.

mod duckdb_stamp;
mod flags;
mod gate;
mod isolate;
mod nightly;
mod nightly_status;
mod runner;
mod shard;
mod steps;
mod workflows;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use runner::{run_with_pair, Outcome, Row, RunMode};
use steps::{Ctx, STEPS};

const USAGE: &str = "\
usage:
  cargo xtask ci                 run every step in order, the two test steps side by side, stopping at the first that fails
  cargo xtask ci --summary-json <path>
                                 the same, and write the summary to <path> as JSON once the run ends, whatever its verdict
  cargo xtask ci --nightly [--summary-json <path>]
                                 run every step, carrying on past a failure, including the nightly-only steps
  cargo xtask ci --step <name>   run one step by name; this is how CI's jobs call the command
  cargo xtask ci --list          name every step, in order, with what it runs
  cargo xtask gate <pr>          run the full gate on a pull request merged with main, and post its verdict as the local-gate status
  cargo xtask nightly --sha <sha> [--trial]
                                 run the full gate with the nightly-only steps on this tree, whose head is the 40-character <sha>,
                                 and post the nightly-gate and nightly-network statuses; --trial raises and clears no Kata flag";

/// What the command line asked for.
#[derive(Debug, PartialEq)]
enum Invocation {
    /// A full run, writing its summary as JSON to the path when one is given;
    /// `nightly` carries on past a failure and starts the nightly-only steps.
    All {
        summary: Option<PathBuf>,
        nightly: bool,
    },
    One(String),
    List,
    Gate(u64),
    /// The nightly gate on `sha`, this tree's head; a `trial` touches no flag.
    Nightly {
        sha: String,
        trial: bool,
    },
}

/// True for a full commit SHA: 40 hexadecimal digits.
fn is_full_sha(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

fn parse(args: &[String]) -> Result<Invocation, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["ci"] => Ok(Invocation::All {
            summary: None,
            nightly: false,
        }),
        ["ci", "--nightly"] => Ok(Invocation::All {
            summary: None,
            nightly: true,
        }),
        ["ci", "--summary-json", path] => Ok(Invocation::All {
            summary: Some(PathBuf::from(path)),
            nightly: false,
        }),
        ["ci", "--nightly", "--summary-json", path]
        | ["ci", "--summary-json", path, "--nightly"] => Ok(Invocation::All {
            summary: Some(PathBuf::from(path)),
            nightly: true,
        }),
        ["ci", "--list"] => Ok(Invocation::List),
        ["ci", "--step", name] => Ok(Invocation::One((*name).to_owned())),
        ["ci", single] if single.starts_with("--step=") => {
            Ok(Invocation::One(single["--step=".len()..].to_owned()))
        }
        ["gate", n] => n
            .parse()
            .map(Invocation::Gate)
            .map_err(|_| USAGE.to_owned()),
        ["nightly", "--sha", sha] if is_full_sha(sha) => Ok(Invocation::Nightly {
            sha: (*sha).to_owned(),
            trial: false,
        }),
        ["nightly", "--sha", sha, "--trial"] if is_full_sha(sha) => Ok(Invocation::Nightly {
            sha: (*sha).to_owned(),
            trial: true,
        }),
        _ => Err(USAGE.to_owned()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let invocation = match parse(&args) {
        Ok(invocation) => invocation,
        Err(usage) => {
            eprintln!("{usage}");
            return ExitCode::from(2);
        }
    };
    let ctx = Ctx {
        root: repo_root(),
        cargo: std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()),
        log: None,
    };
    match invocation {
        Invocation::Gate(pr) => gate::run(pr),
        Invocation::Nightly { sha, trial } => nightly::run(&sha, trial),
        Invocation::List => {
            let width = STEPS.iter().map(|s| s.name.len()).max().unwrap_or(0);
            for step in STEPS {
                let tag = if step.only_in_ci.is_some() {
                    "  (CI only)"
                } else if step.only_at_night.is_some() {
                    "  (nightly only)"
                } else {
                    ""
                };
                println!("{:<width$}  {}{tag}", step.name, step.runs);
            }
            ExitCode::SUCCESS
        }
        Invocation::One(name) => {
            let Some(step) = STEPS.iter().find(|s| s.name == name) else {
                let known: Vec<&str> = STEPS.iter().map(|s| s.name).collect();
                let message = format!(
                    "unknown step '{name}': `cargo xtask ci` carries no step of that name. Its steps are: {}",
                    known.join(", ")
                );
                if std::env::var_os("GITHUB_ACTIONS").is_some() {
                    println!("::error title=unknown step::{message}");
                }
                eprintln!("cargo xtask ci: {message}");
                return ExitCode::from(2);
            };
            eprintln!("==> {}: {}", step.name, step.runs);
            // A step named here runs even when it runs only in CI: a workflow
            // is what names one, and CI is where it was asked for.
            let rows = vec![runner::run_named(step, |s| (s.run)(&ctx))];
            finish(&rows, &[])
        }
        Invocation::All {
            summary: summary_path,
            nightly,
        } => {
            let total = STEPS.len();
            // The two test steps stream at once, so their failure output would
            // interleave; test-default's goes to a log, shown if it fails.
            let log = match ctx.scratch() {
                Ok(dir) => dir.join("test-default.log"),
                Err(message) => {
                    eprintln!("cargo xtask ci: {message}");
                    return ExitCode::FAILURE;
                }
            };
            let logged = Ctx {
                root: ctx.root.clone(),
                cargo: ctx.cargo.clone(),
                log: Some(log.clone()),
            };
            // The nightly carries on past a failure so one red step does not
            // hide the state of the rest, and starts the nightly-only steps.
            let mode = RunMode {
                keep_going: nightly,
                nightly,
            };
            let rows = run_with_pair(STEPS, ("test-one-thread", "test-default"), mode, |step| {
                let position = STEPS.iter().position(|s| s.name == step.name).unwrap_or(0) + 1;
                if step.name == "test-default" {
                    eprintln!(
                        "\n==> [{position}/{total}] {}: {} (output in {})",
                        step.name,
                        step.runs,
                        log.display()
                    );
                    (step.run)(&logged)
                } else {
                    eprintln!("\n==> [{position}/{total}] {}: {}", step.name, step.runs);
                    (step.run)(&ctx)
                }
            });
            let default_failed = rows.iter().any(|row| {
                row.name == "test-default" && matches!(row.outcome, Outcome::Failed(..))
            });
            let text = std::fs::read_to_string(&log).unwrap_or_default();
            if default_failed && !text.trim().is_empty() {
                eprintln!(
                    "\ntest-default's output is in {}; its failures:",
                    log.display()
                );
                eprintln!("{}", steps::failure_excerpt(&text));
            }
            let written = summary_path.map_or(Ok(()), |path| {
                std::fs::write(&path, runner::summary_json(&rows, STEPS))
                    .map_err(|e| format!("cannot write the summary to {}: {e}", path.display()))
            });
            let verdict = finish(&rows, STEPS);
            match written {
                Ok(()) => verdict,
                Err(message) => {
                    eprintln!("cargo xtask ci: {message}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// The repository this command was built from.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(Path::new(".."))
        .to_path_buf()
}

/// Print the summary, and exit 0 only when no step failed.
fn finish(rows: &[Row], steps: &[runner::Step]) -> ExitCode {
    for row in rows {
        if let Outcome::Failed(_, reason) = &row.outcome {
            eprintln!("\n{} failed: {reason}", row.name);
        }
    }
    println!("\n{}", runner::summary(rows, steps));
    if runner::passed(rows) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn the_command_line_reads_a_gate_run() {
        assert_eq!(parse(&words("gate 431")), Ok(Invocation::Gate(431)));
        assert!(parse(&words("gate")).is_err());
        assert!(parse(&words("gate x")).is_err());
    }

    #[test]
    fn the_command_line_reads_where_a_full_run_writes_its_summary() {
        assert_eq!(
            parse(&words("ci --summary-json out/summary.json")),
            Ok(Invocation::All {
                summary: Some(PathBuf::from("out/summary.json")),
                nightly: false
            })
        );
        assert!(parse(&words("ci --summary-json")).is_err());
    }

    #[test]
    fn the_command_line_reads_a_nightly_full_run() {
        let nightly = Ok(Invocation::All {
            summary: Some(PathBuf::from("/x")),
            nightly: true,
        });
        assert_eq!(parse(&words("ci --nightly --summary-json /x")), nightly);
        assert_eq!(parse(&words("ci --summary-json /x --nightly")), nightly);
        assert_eq!(
            parse(&words("ci --nightly")),
            Ok(Invocation::All {
                summary: None,
                nightly: true
            })
        );
    }

    #[test]
    fn the_command_line_reads_a_nightly_run() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            parse(&words(&format!("nightly --sha {sha}"))),
            Ok(Invocation::Nightly {
                sha: sha.to_owned(),
                trial: false
            })
        );
        assert_eq!(
            parse(&words(&format!("nightly --sha {sha} --trial"))),
            Ok(Invocation::Nightly {
                sha: sha.to_owned(),
                trial: true
            })
        );
        assert!(parse(&words("nightly --sha 0123456")).is_err());
        assert!(parse(&words(&format!("nightly --sha {}x", &sha[..39]))).is_err());
        assert!(parse(&words("nightly")).is_err());
    }

    #[test]
    fn the_command_line_reads_a_full_run_one_step_and_the_list() {
        assert_eq!(
            parse(&words("ci")),
            Ok(Invocation::All {
                summary: None,
                nightly: false
            })
        );
        assert_eq!(
            parse(&words("ci --step fmt")),
            Ok(Invocation::One("fmt".to_owned()))
        );
        assert_eq!(
            parse(&words("ci --step=doc")),
            Ok(Invocation::One("doc".to_owned()))
        );
        assert_eq!(parse(&words("ci --list")), Ok(Invocation::List));
        assert!(parse(&words("ci --step")).is_err());
        assert!(parse(&words("lint")).is_err());
        assert!(parse(&[]).is_err());
    }
}
