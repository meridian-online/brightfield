//! The repository's gate as one command.
//!
//! `cargo xtask ci` runs what a pull request's checks run, in order, and stops
//! at the first step that fails. It ends with a summary that names each step in
//! the order run, with its wall time, the failed step, and the steps the
//! failure skipped. The workflow files call the same steps by name, one per
//! workflow step, as `cargo xtask ci --step <name>`, so what CI runs is this
//! command's list of steps, and a name the command does not carry is a red job
//! with a line naming it.
//!
//! The `workflow-coverage` step reads the workflow files and keeps the two
//! lists one list: it is red for a step no workflow invokes, for a workflow
//! invoking a step that does not exist, and for a workflow a pull request
//! triggers calling a gate around the command.
//!
//! One step has no local form: `pr-text`, which scans the pull request's title
//! and body, runs only in CI, and a full run's summary says so and why.
//!
//! What a full run needs on the machine, each refused by name at its step when
//! absent: the pinned toolchain, `cargo-deny`, `python3`, the sibling
//! `open-analytics` checkout and the pinned DuckDB CLI.

mod runner;
mod shard;
mod steps;
mod workflows;

use std::path::PathBuf;
use std::process::ExitCode;

use runner::{run_in_order, Outcome, Row};
use steps::{Ctx, STEPS};

const USAGE: &str = "\
usage:
  cargo xtask ci                 run every step in order, stopping at the first that fails
  cargo xtask ci --step <name>   run one step by name; this is how CI's jobs call the command
  cargo xtask ci --list          name every step, in order, with what it runs";

/// What the command line asked for.
#[derive(Debug, PartialEq)]
enum Invocation {
    All,
    One(String),
    List,
}

fn parse(args: &[String]) -> Result<Invocation, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["ci"] => Ok(Invocation::All),
        ["ci", "--list"] => Ok(Invocation::List),
        ["ci", "--step", name] => Ok(Invocation::One((*name).to_owned())),
        ["ci", single] if single.starts_with("--step=") => {
            Ok(Invocation::One(single["--step=".len()..].to_owned()))
        }
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
        root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."),
        cargo: std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()),
    };
    match invocation {
        Invocation::List => {
            let width = STEPS.iter().map(|s| s.name.len()).max().unwrap_or(0);
            for step in STEPS {
                let tag = if step.only_in_ci.is_some() { "  (CI only)" } else { "" };
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
            let rows = run_in_order(std::slice::from_ref(step), |s| (s.run)(&ctx));
            finish(&rows, std::slice::from_ref(step))
        }
        Invocation::All => {
            let total = STEPS.len();
            let rows = run_in_order(STEPS, |step| {
                let position = STEPS.iter().position(|s| s.name == step.name).unwrap_or(0) + 1;
                eprintln!("\n==> [{position}/{total}] {}: {}", step.name, step.runs);
                (step.run)(&ctx)
            });
            finish(&rows, STEPS)
        }
    }
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
    fn the_command_line_reads_a_full_run_one_step_and_the_list() {
        assert_eq!(parse(&words("ci")), Ok(Invocation::All));
        assert_eq!(parse(&words("ci --step fmt")), Ok(Invocation::One("fmt".to_owned())));
        assert_eq!(parse(&words("ci --step=doc")), Ok(Invocation::One("doc".to_owned())));
        assert_eq!(parse(&words("ci --list")), Ok(Invocation::List));
        assert!(parse(&words("ci --step")).is_err());
        assert!(parse(&words("lint")).is_err());
        assert!(parse(&[]).is_err());
    }
}
