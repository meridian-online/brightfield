//! The steps, in the order a full run takes them, and what each one runs.
//!
//! The order puts what answers fastest first: the hygiene scripts, which need
//! no build; then the lint gates; then the licence and advisory checks; then
//! the suite and the two steps that read it; then the packaging self-tests.
//! Each step runs what one step of a pull request's checks ran before this
//! command existed, with the same arguments, and the workflow files now call
//! the step by name.

use std::ffi::OsString;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::runner::Step;
use crate::{shard, workflows};

/// What every step runs against: the repository root and the cargo that
/// started this command.
pub struct Ctx {
    pub root: PathBuf,
    pub cargo: OsString,
}

impl Ctx {
    /// A command run from the repository root. The variables cargo sets for
    /// the package it runs (this one) are removed, so a script or a nested
    /// cargo sees the environment CI's step would have given it.
    fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = Command::new(program);
        command.current_dir(&self.root);
        for (key, _) in std::env::vars_os() {
            let key_text = key.to_string_lossy();
            let set_for_this_package = key_text.starts_with("CARGO_PKG_")
                || matches!(
                    key_text.as_ref(),
                    "CARGO_MANIFEST_DIR"
                        | "CARGO_MANIFEST_PATH"
                        | "CARGO_CRATE_NAME"
                        | "CARGO_BIN_NAME"
                        | "CARGO_PRIMARY_PACKAGE"
                );
            if set_for_this_package {
                command.env_remove(&key);
            }
        }
        command
    }

    fn cargo(&self, args: &[&str]) -> Command {
        let mut command = self.command(&self.cargo);
        command.args(args);
        command
    }

    /// Run a cargo command, inheriting the terminal.
    fn run_cargo(&self, args: &[&str]) -> Result<(), String> {
        run(self.cargo(args), &format!("cargo {}", args.join(" ")))
    }

    /// Run a script from `scripts/`, by path, as the workflow step did.
    fn script(&self, path: &str, args: &[&str]) -> Result<(), String> {
        let mut command = self.command(self.root.join(path));
        command.args(args);
        let shown = std::iter::once(format!("./{path}"))
            .chain(args.iter().map(|a| (*a).to_owned()))
            .collect::<Vec<_>>()
            .join(" ");
        run(command, &shown)
    }

    /// A script whose interpreter is `python3`, which has to be on the path.
    fn python_script(&self, path: &str, args: &[&str]) -> Result<(), String> {
        let found = Command::new("python3")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !found {
            return Err(
                "python3 is not on the path: install Python 3 (python.org, or `brew install python`) and run again"
                    .to_owned(),
            );
        }
        self.script(path, args)
    }

    /// A scratch directory under `target/`, for files a step writes.
    fn scratch(&self) -> Result<PathBuf, String> {
        let dir = std::env::var_os("RUNNER_TEMP")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root.join("target/xtask"));
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        Ok(dir)
    }
}

/// Run `command` with the terminal inherited; a non-zero exit is the error.
fn run(mut command: Command, shown: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|e| format!("could not start `{shown}`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{shown}` exited with {status}"))
    }
}

/// Print `message` as an error, as a GitHub annotation when running in CI.
fn report_error(title: &str, message: &str) {
    if std::env::var_os("GITHUB_ACTIONS").is_some() {
        println!("::error title={title}::{message}");
    } else {
        eprintln!("error: {title}: {message}");
    }
}

/// The steps, in order.
pub static STEPS: &[Step] = &[
    Step {
        name: "workflow-coverage",
        runs: "the command's own reading of .github/workflows: every step invoked, no unknown step, no gate called around it",
        only_in_ci: None,
        run: workflow_coverage,
    },
    Step {
        name: "public-hygiene-selftest",
        runs: "./scripts/check-public-hygiene-selftest.sh",
        only_in_ci: None,
        run: |c| c.script("scripts/check-public-hygiene-selftest.sh", &[]),
    },
    Step {
        name: "public-hygiene",
        runs: "./scripts/check-public-hygiene.sh",
        only_in_ci: None,
        run: |c| c.script("scripts/check-public-hygiene.sh", &[]),
    },
    Step {
        name: "history-hygiene-selftest",
        runs: "./scripts/check-history-hygiene-selftest.sh",
        only_in_ci: None,
        run: |c| c.script("scripts/check-history-hygiene-selftest.sh", &[]),
    },
    Step {
        name: "commit-messages",
        runs: "./scripts/check-history-hygiene.sh --range <base>..HEAD, the base from HISTORY_BASE or the merge base with origin/main",
        only_in_ci: None,
        run: commit_messages,
    },
    Step {
        name: "pr-text",
        runs: "./scripts/check-history-hygiene.sh --text over PR_TITLE and PR_BODY",
        only_in_ci: Some(
            "it scans the pull request's title and body, which exist only in the event that starts a CI run; the `Private references in pull request text` check runs it on every edit of the description",
        ),
        run: pr_text,
    },
    Step {
        name: "borrowed-benchmarks-selftest",
        runs: "./scripts/check-borrowed-benchmarks-selftest.sh",
        only_in_ci: None,
        run: |c| c.script("scripts/check-borrowed-benchmarks-selftest.sh", &[]),
    },
    Step {
        name: "borrowed-benchmarks",
        runs: "./scripts/check-borrowed-benchmarks.sh",
        only_in_ci: None,
        run: |c| c.script("scripts/check-borrowed-benchmarks.sh", &[]),
    },
    Step {
        name: "scale-claims-selftest",
        runs: "./scripts/check-scale-claims.py --self-test",
        only_in_ci: None,
        run: |c| c.python_script("scripts/check-scale-claims.py", &["--self-test"]),
    },
    Step {
        name: "scale-claims",
        runs: "./scripts/check-scale-claims.py",
        only_in_ci: None,
        run: |c| c.python_script("scripts/check-scale-claims.py", &[]),
    },
    Step {
        name: "measured-figures-selftest",
        runs: "./scripts/check-measured-figures.py --self-test",
        only_in_ci: None,
        run: |c| c.python_script("scripts/check-measured-figures.py", &["--self-test"]),
    },
    Step {
        name: "measured-figures",
        runs: "./scripts/check-measured-figures.py",
        only_in_ci: None,
        run: |c| c.python_script("scripts/check-measured-figures.py", &[]),
    },
    Step {
        name: "package-version-guard-selftest",
        runs: "./scripts/package-version-guard-selftest.sh",
        only_in_ci: None,
        run: |c| c.script("scripts/package-version-guard-selftest.sh", &[]),
    },
    Step {
        name: "bundled-extension-selftest",
        runs: "./scripts/check-bundled-extension-selftest.sh",
        only_in_ci: None,
        run: |c| c.script("scripts/check-bundled-extension-selftest.sh", &[]),
    },
    Step {
        name: "finetype-pin-selftest",
        runs: "./scripts/check-finetype-pin-selftest.sh",
        only_in_ci: None,
        run: |c| c.script("scripts/check-finetype-pin-selftest.sh", &[]),
    },
    Step {
        name: "finetype-pin",
        runs: "./scripts/check-finetype-pin.sh",
        only_in_ci: None,
        run: |c| c.script("scripts/check-finetype-pin.sh", &[]),
    },
    Step {
        name: "release-readback-selftest",
        runs: "./scripts/check-release-readback.py --self-test",
        only_in_ci: None,
        run: |c| c.python_script("scripts/check-release-readback.py", &["--self-test"]),
    },
    Step {
        name: "release-readback",
        runs: "./scripts/check-release-readback.py",
        only_in_ci: None,
        run: |c| c.python_script("scripts/check-release-readback.py", &[]),
    },
    Step {
        name: "fetch-finetype-bundle-selftest",
        runs: "./scripts/fetch-finetype-bundle-selftest.sh",
        only_in_ci: None,
        run: |c| c.script("scripts/fetch-finetype-bundle-selftest.sh", &[]),
    },
    Step {
        name: "package-finetype-selftest",
        runs: "./scripts/package-finetype-selftest.sh",
        only_in_ci: None,
        run: |c| c.script("scripts/package-finetype-selftest.sh", &[]),
    },
    Step {
        name: "formula-layout-selftest",
        runs: "./scripts/check-formula-layout-selftest.sh",
        only_in_ci: None,
        run: |c| c.script("scripts/check-formula-layout-selftest.sh", &[]),
    },
    Step {
        name: "formula-asset-selftest",
        runs: "./scripts/check-formula-asset-selftest.sh",
        only_in_ci: None,
        run: |c| c.script("scripts/check-formula-asset-selftest.sh", &[]),
    },
    Step {
        name: "brew-install-selftest",
        runs: "./scripts/check-brew-install-selftest.sh",
        only_in_ci: None,
        run: |c| c.script("scripts/check-brew-install-selftest.sh", &[]),
    },
    Step {
        name: "fmt",
        runs: "cargo fmt --all --check",
        only_in_ci: None,
        run: |c| c.run_cargo(&["fmt", "--all", "--check"]),
    },
    Step {
        name: "clippy",
        runs: "cargo clippy --workspace --all-targets -- -D warnings",
        only_in_ci: None,
        run: |c| c.run_cargo(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]),
    },
    Step {
        name: "check-release",
        runs: "cargo check --release --workspace",
        only_in_ci: None,
        run: |c| c.run_cargo(&["check", "--release", "--workspace"]),
    },
    Step {
        name: "doc",
        runs: "cargo doc --workspace --no-deps --document-private-items, under RUSTDOCFLAGS=-D warnings",
        only_in_ci: None,
        run: doc,
    },
    Step {
        name: "comment-citations",
        runs: "./scripts/check-comment-citations.py, over the diff against origin/main",
        only_in_ci: None,
        run: |c| c.python_script("scripts/check-comment-citations.py", &[]),
    },
    Step {
        name: "comment-citations-selftest",
        runs: "./scripts/check-comment-citations.py --self-test",
        only_in_ci: None,
        run: |c| c.python_script("scripts/check-comment-citations.py", &["--self-test"]),
    },
    Step {
        name: "deny-licenses",
        runs: "cargo deny --all-features check licenses",
        only_in_ci: None,
        run: |c| cargo_deny(c, "licenses"),
    },
    Step {
        name: "deny-advisories",
        runs: "cargo deny --all-features check advisories",
        only_in_ci: None,
        run: |c| cargo_deny(c, "advisories"),
    },
    Step {
        name: "test-one-thread",
        runs: "cargo test --locked --workspace over the one-thread group, -- --test-threads=1",
        only_in_ci: None,
        run: test_one_thread,
    },
    Step {
        name: "test-default",
        runs: "cargo test --locked --workspace over every other target, then --doc",
        only_in_ci: None,
        run: test_default,
    },
    Step {
        name: "sibling-manifests",
        runs: "cargo test --locked -p brightfield-protocol --test sibling_manifests -- --ignored, at least 2 executed",
        only_in_ci: None,
        run: sibling_manifests,
    },
    Step {
        name: "conformance",
        runs: "cargo run --locked --bin conformance -- --layers 1,2,3,4 --corpus curated",
        only_in_ci: None,
        run: conformance,
    },
    Step {
        name: "artifact-type-source-selftest",
        runs: "./scripts/check-artifact-type-source-selftest.sh",
        only_in_ci: None,
        run: |c| c.script("scripts/check-artifact-type-source-selftest.sh", &[]),
    },
    Step {
        name: "packaged-artifact-staging-selftest",
        runs: "./scripts/package-artifact-staging-selftest.sh",
        only_in_ci: None,
        run: |c| c.script("scripts/package-artifact-staging-selftest.sh", &[]),
    },
];

fn workflow_coverage(ctx: &Ctx) -> Result<(), String> {
    let files = workflows::read_all(&ctx.root)?;
    let names: Vec<&str> = STEPS.iter().map(|s| s.name).collect();
    let problems = workflows::coverage(&names, &files);
    for problem in &problems {
        report_error("the command and the workflows disagree", problem);
    }
    if problems.is_empty() {
        println!(
            "{} step(s), each invoked by a workflow; no workflow invokes an unknown step or calls a gate around the command",
            names.len()
        );
        Ok(())
    } else {
        Err(format!("{} disagreement(s) between the command and the workflow files, listed above", problems.len()))
    }
}

fn commit_messages(ctx: &Ctx) -> Result<(), String> {
    let base = match std::env::var("HISTORY_BASE") {
        Ok(base) if !base.is_empty() => base,
        _ => {
            let output = ctx
                .command("git")
                .args(["merge-base", "HEAD", "origin/main"])
                .output()
                .map_err(|e| format!("could not start git: {e}"))?;
            if !output.status.success() {
                return Err("no merge base with origin/main: run `git fetch origin main`, or set HISTORY_BASE to the commit the range starts after".to_owned());
            }
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }
    };
    let range = format!("{base}..HEAD");
    ctx.script("scripts/check-history-hygiene.sh", &["--range", &range])
}

fn pr_text(ctx: &Ctx) -> Result<(), String> {
    let (Some(title), Some(body)) = (std::env::var_os("PR_TITLE"), std::env::var_os("PR_BODY")) else {
        return Err("pr-text reads the pull request's title and body from PR_TITLE and PR_BODY, which a pull request's workflow sets from its event; neither is set here".to_owned());
    };
    let file = ctx.scratch()?.join("pr-text.txt");
    let mut text = title.to_string_lossy().into_owned();
    text.push('\n');
    text.push_str(&body.to_string_lossy());
    text.push('\n');
    fs::write(&file, text).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
    let path = file.display().to_string();
    ctx.script("scripts/check-history-hygiene.sh", &["--text", &path])
}

fn doc(ctx: &Ctx) -> Result<(), String> {
    let args = ["doc", "--workspace", "--no-deps", "--document-private-items"];
    let mut command = ctx.cargo(&args);
    command.env("RUSTDOCFLAGS", "-D warnings");
    run(command, "RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --document-private-items")
}

fn cargo_deny(ctx: &Ctx, check: &str) -> Result<(), String> {
    let installed = ctx
        .cargo(&["deny", "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !installed {
        return Err("cargo-deny is not installed: run `cargo install --locked cargo-deny`, then run again".to_owned());
    }
    ctx.run_cargo(&[
        "deny",
        "--manifest-path",
        "./Cargo.toml",
        "--log-level",
        "warn",
        "--all-features",
        "check",
        check,
    ])
}

/// This run's test selection, read from `test.yml`, `cargo metadata` and the
/// shard environment.
fn selection(ctx: &Ctx) -> Result<shard::Selection, String> {
    let test_yml_path = ctx.root.join(".github/workflows/test.yml");
    let test_yml = fs::read_to_string(&test_yml_path)
        .map_err(|e| format!("cannot read {}: {e}", test_yml_path.display()))?;
    let (serial, seconds) = shard::read_lists(&test_yml)?;
    let output = ctx
        .cargo(&["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("could not start cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!("`cargo metadata --locked --no-deps` exited with {}", output.status));
    }
    let targets = shard::test_targets(&String::from_utf8_lossy(&output.stdout))?;
    let shard = shard::shard_from_env(|var| std::env::var(var).ok())?;
    shard::select(&serial, &seconds, &targets, shard.as_ref())
}

/// The pinned DuckDB CLI the run tests hand to arc: ARC_DUCKDB_BIN when it is
/// set, `target/duckdb-cli/duckdb` when it is not, and either way accepted by
/// `scripts/fetch-duckdb-cli.sh --check` before a test runs.
fn pinned_duckdb(ctx: &Ctx) -> Result<PathBuf, String> {
    let host = host_triple()?;
    let engine = match std::env::var_os("ARC_DUCKDB_BIN") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => ctx.root.join("target/duckdb-cli/duckdb"),
    };
    let dir = engine.parent().unwrap_or(Path::new("."));
    let accepted = engine.file_name().is_some_and(|name| name == "duckdb")
        && ctx
            .command(ctx.root.join("scripts/fetch-duckdb-cli.sh"))
            .arg("--check")
            .arg(&host)
            .arg(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
    if accepted {
        Ok(engine)
    } else {
        Err(format!(
            "the pinned DuckDB CLI is not at {}: run `scripts/fetch-duckdb-cli.sh {host} target/duckdb-cli` to fetch it there, or set ARC_DUCKDB_BIN to a duckdb that `scripts/fetch-duckdb-cli.sh --check {host} <its directory>` accepts",
            engine.display()
        ))
    }
}

fn host_triple() -> Result<String, String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .map_err(|e| format!("could not start rustc: {e}"))?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(str::to_owned)
        .ok_or_else(|| "rustc -vV printed no host line".to_owned())
}

fn cargo_test(ctx: &Ctx, engine: &Path, args: &[String], threads_one: bool) -> Result<(), String> {
    let mut all: Vec<&str> = vec!["test", "--locked", "--workspace"];
    all.extend(args.iter().map(String::as_str));
    if threads_one {
        all.extend(["--", "--test-threads=1"]);
    }
    let mut command = ctx.cargo(&all);
    command.env("ARC_DUCKDB_BIN", engine);
    run(command, &format!("cargo {}", all.join(" ")))
}

fn test_one_thread(ctx: &Ctx) -> Result<(), String> {
    let Some(part) = selection(ctx)?.one_thread else {
        println!("this shard runs the default-threads group; test-one-thread has nothing to run on it");
        return Ok(());
    };
    for line in &part.report {
        println!("{line}");
    }
    let args = part.cargo_args();
    if args.is_empty() {
        println!("no target was planned for this shard");
        return Ok(());
    }
    let engine = pinned_duckdb(ctx)?;
    cargo_test(ctx, &engine, &args, true)
}

fn test_default(ctx: &Ctx) -> Result<(), String> {
    let Some(part) = selection(ctx)?.default else {
        println!("this shard runs the one-thread group; test-default has nothing to run on it");
        return Ok(());
    };
    for line in &part.report {
        println!("{line}");
    }
    let args = part.cargo_args();
    if args.is_empty() && !part.doc() {
        println!("no target was planned for this shard");
        return Ok(());
    }
    let engine = pinned_duckdb(ctx)?;
    // With no target flag `cargo test --workspace` runs every target, so an
    // empty list runs nothing rather than that.
    if !args.is_empty() {
        cargo_test(ctx, &engine, &args, false)?;
    }
    if part.doc() {
        cargo_test(ctx, &engine, &["--doc".to_owned()], false)?;
    }
    Ok(())
}

/// The sibling open-analytics checkout: OPEN_ANALYTICS_DIR when set, else the
/// checkout beside this one, or beside the main checkout when this is a
/// worktree.
fn open_analytics(ctx: &Ctx) -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("OPEN_ANALYTICS_DIR").filter(|d| !d.is_empty()) {
        let dir = PathBuf::from(dir);
        return if dir.is_dir() {
            Ok(dir)
        } else {
            Err(format!(
                "OPEN_ANALYTICS_DIR names {}, which is not a directory: clone https://github.com/meridian-online/open-analytics there, or point the variable at a clone",
                dir.display()
            ))
        };
    }
    let mut candidates = vec![ctx.root.join("../open-analytics")];
    if let Ok(output) = ctx
        .command("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
    {
        let common = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        if let Some(main_checkout) = common.parent() {
            candidates.push(main_checkout.join("../open-analytics"));
        }
    }
    candidates
        .iter()
        .find(|dir| dir.is_dir())
        .cloned()
        .ok_or_else(|| {
            format!(
                "the sibling open-analytics checkout is missing: clone https://github.com/meridian-online/open-analytics to {}, or set OPEN_ANALYTICS_DIR to a clone",
                candidates[0].display()
            )
        })
}

/// libtest's executed count: the sum of the numbers before `passed;` on lines
/// that begin `test result:`.
pub fn executed_count(log: &str) -> usize {
    log.lines()
        .filter(|line| line.starts_with("test result:"))
        .map(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            words
                .windows(2)
                .filter(|pair| pair[1] == "passed;")
                .filter_map(|pair| pair[0].parse::<usize>().ok())
                .sum::<usize>()
        })
        .sum()
}

fn sibling_manifests(ctx: &Ctx) -> Result<(), String> {
    if !selection(ctx)?.protocols {
        println!("the pin-staleness check runs on the last default-threads shard, not this one");
        return Ok(());
    }
    let dir = open_analytics(ctx)?;
    // The number of `#[ignore]`d tests in the target, stated once so a reader
    // and the log both see what this step expects to have run.
    let floor = 2;
    let log_path = ctx.scratch()?.join("sibling-manifests.log");
    let disk = |when: &str| {
        println!("--- disk {when} the sibling-manifest step:");
        let _ = ctx.command("df").args(["-h", "/"]).status();
    };
    disk("before");
    let log = File::create(&log_path).map_err(|e| format!("cannot create {}: {e}", log_path.display()))?;
    let log_err = log.try_clone().map_err(|e| format!("cannot reopen {}: {e}", log_path.display()))?;
    let status = ctx
        .cargo(&[
            "test",
            "--locked",
            "-p",
            "brightfield-protocol",
            "--test",
            "sibling_manifests",
            "--",
            "--ignored",
            "--test-threads=1",
            "--nocapture",
        ])
        .env("OPEN_ANALYTICS_DIR", &dir)
        .stdout(log)
        .stderr(log_err)
        .status()
        .map_err(|e| format!("could not start cargo test: {e}"))?;
    let text = fs::read_to_string(&log_path).map_err(|e| format!("cannot read {}: {e}", log_path.display()))?;
    print!("{text}");
    disk("after");
    let ran = executed_count(&text);
    println!(
        "--- sibling_manifests: cargo exited {status}, {ran} test(s) executed (floor {floor})"
    );
    let mut failures = Vec::new();
    if !status.success() {
        let message = format!("cargo test exited {status}: the failure above names the manifest and quotes arc's own diagnostic. Bump the pin: README section \"The arcform dependency\" (the arc rev, the root [patch.crates-io] sqlparser rev, and Cargo.lock, in one commit)");
        report_error("the pinned arc cannot load a shipped Protocol", &message);
        failures.push("the pinned arc cannot load a shipped Protocol".to_owned());
    }
    if ran < floor {
        let message = format!("expected at least {floor} ignored test(s) to execute in sibling_manifests, {ran} executed: the filter matched nothing, or a covered test was renamed, deleted or un-ignored. A green run here would prove nothing");
        report_error("executed too few tests", &message);
        failures.push(format!("{ran} of at least {floor} sibling-manifest tests executed"));
    }
    if failures.is_empty() {
        println!("{ran} sibling-manifest test(s) executed and passed (floor {floor})");
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn conformance(ctx: &Ctx) -> Result<(), String> {
    if !selection(ctx)?.protocols {
        println!("the conformance scoreboard runs on the last default-threads shard, not this one");
        return Ok(());
    }
    ctx.run_cargo(&[
        "run",
        "--locked",
        "--bin",
        "conformance",
        "--",
        "--layers",
        "1,2,3,4",
        "--corpus",
        "curated",
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_names_are_unique() {
        let mut names: Vec<&str> = STEPS.iter().map(|s| s.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "two steps share a name");
    }

    #[test]
    fn the_executed_count_reads_libtest_summaries_and_nothing_else() {
        let log = "running 7 tests\n\
                   test a ... ok\n\
                   a line that says 9 passed; inside a test's own output\n\
                   test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 0.10s\n\
                   test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
        assert_eq!(executed_count(log), 3);
        assert_eq!(executed_count("test result: ok. 0 passed; 0 failed;\n"), 0);
    }
}
