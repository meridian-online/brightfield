//! The steps, in the order a full run takes them, and what each one runs.
//!
//! The order puts what answers fastest first: the script gates, which need
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
use crate::{duckdb_stamp, shard, workflows};

/// What every step runs against: the repository root and the cargo that
/// started this command.
pub struct Ctx {
    pub root: PathBuf,
    pub cargo: OsString,
    /// When set, the test runs of the step this context is handed to write
    /// their output to this file instead of the terminal.
    pub log: Option<PathBuf>,
}

impl Ctx {
    /// A command run from the repository root. The variables cargo sets for
    /// the package it runs (this one) are removed, so a script or a nested
    /// cargo sees the environment CI's step would have given it.
    pub(crate) fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
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
    pub fn scratch(&self) -> Result<PathBuf, String> {
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
        runs: "the command's own reading of .github/workflows: every step invoked, no unknown step, no gate called around it, no macOS job off a release tag",
        only_in_ci: None,
        only_at_night: None,
        run: workflow_coverage,
    },
    Step {
        name: "borrowed-benchmarks-selftest",
        runs: "./scripts/check-borrowed-benchmarks-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/check-borrowed-benchmarks-selftest.sh", &[]),
    },
    Step {
        name: "borrowed-benchmarks",
        runs: "./scripts/check-borrowed-benchmarks.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/check-borrowed-benchmarks.sh", &[]),
    },
    Step {
        name: "scale-claims-selftest",
        runs: "./scripts/check-scale-claims.py --self-test",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.python_script("scripts/check-scale-claims.py", &["--self-test"]),
    },
    Step {
        name: "scale-claims",
        runs: "./scripts/check-scale-claims.py",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.python_script("scripts/check-scale-claims.py", &[]),
    },
    Step {
        name: "measured-figures-selftest",
        runs: "./scripts/check-measured-figures.py --self-test",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.python_script("scripts/check-measured-figures.py", &["--self-test"]),
    },
    Step {
        name: "measured-figures",
        runs: "./scripts/check-measured-figures.py",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.python_script("scripts/check-measured-figures.py", &[]),
    },
    Step {
        name: "package-version-guard-selftest",
        runs: "./scripts/package-version-guard-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/package-version-guard-selftest.sh", &[]),
    },
    Step {
        name: "bundled-extension-selftest",
        runs: "./scripts/check-bundled-extension-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/check-bundled-extension-selftest.sh", &[]),
    },
    Step {
        name: "finetype-pin-selftest",
        runs: "./scripts/check-finetype-pin-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/check-finetype-pin-selftest.sh", &[]),
    },
    Step {
        name: "finetype-pin",
        runs: "./scripts/check-finetype-pin.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/check-finetype-pin.sh", &[]),
    },
    Step {
        name: "release-readback-selftest",
        runs: "./scripts/check-release-readback.py --self-test",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.python_script("scripts/check-release-readback.py", &["--self-test"]),
    },
    Step {
        name: "release-readback",
        runs: "./scripts/check-release-readback.py",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.python_script("scripts/check-release-readback.py", &[]),
    },
    Step {
        name: "release-tag-selftest",
        runs: "./scripts/check-release-tag.py --self-test",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.python_script("scripts/check-release-tag.py", &["--self-test"]),
    },
    Step {
        name: "release-tag-wiring",
        runs: "./scripts/check-release-tag.py --wiring",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.python_script("scripts/check-release-tag.py", &["--wiring"]),
    },
    Step {
        name: "release-tag-rehearsal",
        runs: "./scripts/check-release-tag.py --release-rehearsal",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.python_script("scripts/check-release-tag.py", &["--release-rehearsal"]),
    },
    Step {
        name: "fetch-finetype-bundle-selftest",
        runs: "./scripts/fetch-finetype-bundle-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/fetch-finetype-bundle-selftest.sh", &[]),
    },
    Step {
        name: "package-finetype-selftest",
        runs: "./scripts/package-finetype-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/package-finetype-selftest.sh", &[]),
    },
    Step {
        name: "cask-layout-selftest",
        runs: "./scripts/check-cask-layout-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/check-cask-layout-selftest.sh", &[]),
    },
    Step {
        name: "require-notarised-selftest",
        runs: "./scripts/require-notarised-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/require-notarised-selftest.sh", &[]),
    },
    Step {
        name: "cask-asset-selftest",
        runs: "./scripts/check-cask-asset-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/check-cask-asset-selftest.sh", &[]),
    },
    Step {
        name: "brew-install-selftest",
        runs: "./scripts/check-brew-install-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/check-brew-install-selftest.sh", &[]),
    },
    Step {
        name: "drop-fetch-steps-selftest",
        runs: "./scripts/drop-fetch-steps-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/drop-fetch-steps-selftest.sh", &[]),
    },
    Step {
        name: "duckdb-parity-selftest",
        runs: "./scripts/check-duckdb-parity-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/check-duckdb-parity-selftest.sh", &[]),
    },
    Step {
        name: "airgap-jail-selftest",
        runs: "./scripts/airgap-jail-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/airgap-jail-selftest.sh", &[]),
    },
    Step {
        name: "fmt",
        runs: "cargo fmt --all --check",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.run_cargo(&["fmt", "--all", "--check"]),
    },
    Step {
        name: "clippy",
        runs: "cargo clippy --workspace --all-targets -- -D warnings",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.run_cargo(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]),
    },
    Step {
        name: "check-release",
        runs: "cargo check --release --workspace",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.run_cargo(&["check", "--release", "--workspace"]),
    },
    Step {
        name: "doc",
        runs: "cargo doc --workspace --no-deps --document-private-items, under RUSTDOCFLAGS=-D warnings",
        only_in_ci: None,
        only_at_night: None,
        run: doc,
    },
    Step {
        name: "comment-citations",
        runs: "./scripts/check-comment-citations.py, over the diff against origin/main",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.python_script("scripts/check-comment-citations.py", &[]),
    },
    Step {
        name: "comment-citations-selftest",
        runs: "./scripts/check-comment-citations.py --self-test",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.python_script("scripts/check-comment-citations.py", &["--self-test"]),
    },
    Step {
        name: "nightly-launcher-selftest",
        runs: "./scripts/nightly/launcher-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/nightly/launcher-selftest.sh", &[]),
    },
    Step {
        name: "deny-licenses",
        runs: "cargo deny --all-features check licenses",
        only_in_ci: None,
        only_at_night: None,
        run: |c| cargo_deny(c, "licenses"),
    },
    Step {
        name: "deny-advisories",
        runs: "cargo deny --all-features check advisories",
        only_in_ci: None,
        only_at_night: None,
        run: |c| cargo_deny(c, "advisories"),
    },
    Step {
        name: "warm-cache",
        runs: "cargo test --locked --workspace --no-run; the conformance and managed-edition builds",
        only_in_ci: None,
        only_at_night: None,
        run: warm_cache,
    },
    Step {
        name: "test-one-thread",
        runs: "cargo test --locked --workspace over the one-thread group, -- --test-threads=1",
        only_in_ci: None,
        only_at_night: None,
        run: test_one_thread,
    },
    Step {
        name: "test-default",
        runs: "cargo nextest run --locked --workspace over every other target, then cargo test --doc",
        only_in_ci: None,
        only_at_night: None,
        run: test_default,
    },
    Step {
        name: "conformance",
        runs: "cargo run --locked --bin conformance -- --layers 1,2,3,4 --corpus curated",
        only_in_ci: None,
        only_at_night: None,
        run: conformance,
    },
    Step {
        name: "vendored-libduckdb-sys",
        runs: "./scripts/check-vendored-libduckdb-sys.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/check-vendored-libduckdb-sys.sh", &[]),
    },
    Step {
        name: "managed-edition",
        runs: "cargo test --locked -p brightfield-shell --features managed-edition --test edition --test grants --test grant_on_open --test refused_access --test child_grants, then --test grants -- --ignored, at least 1 executed",
        only_in_ci: None,
        only_at_night: None,
        run: managed_edition,
    },
    Step {
        name: "artifact-type-source-selftest",
        runs: "./scripts/check-artifact-type-source-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/check-artifact-type-source-selftest.sh", &[]),
    },
    Step {
        name: "package-artifact-staging-selftest",
        runs: "./scripts/package-artifact-staging-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/package-artifact-staging-selftest.sh", &[]),
    },
    Step {
        name: "sibling-manifests",
        runs: "cargo test --locked -p brightfield-protocol --test sibling_manifests -- --ignored, at least 2 executed: a report of the open-analytics Protocols the pinned arc refuses",
        only_in_ci: None,
        only_at_night: Some(
            "it reads open-analytics' main, whose Protocols may use operators newer than the \
             arc this repo pins; the nightly runs it as a report on its own status, outside \
             the gate's verdict, and --step runs it here",
        ),
        run: sibling_manifests,
    },
    Step {
        name: "nightly-network",
        runs: "./scripts/nightly-network.sh",
        only_in_ci: None,
        only_at_night: Some(
            "it reads published data over the network, so a red says the data or the host \
             moved as often as the commit did; the nightly runs it, and --step runs it here",
        ),
        run: |c| c.script("scripts/nightly-network.sh", &[]),
    },
    Step {
        name: "duckdb-managed",
        runs: "./scripts/build-duckdb.sh unless its inputs are unchanged since the last build, then ./scripts/build-duckdb-selftest.sh",
        only_in_ci: None,
        only_at_night: Some(
            "it builds DuckDB and seven extensions from source, 30 to 90 minutes cold; \
             the nightly runs it, and --step runs it here",
        ),
        run: duckdb_managed,
    },
    Step {
        name: "managed-duckdb-test",
        runs: "cargo test --locked --features managed-duckdb, -p brightfield-engine --test managed_duckdb and -p brightfield-shell --test managed_duckdb_link",
        only_in_ci: None,
        only_at_night: Some(
            "it links the library the duckdb-managed step builds; the nightly runs it \
             after that step, and --step runs it here once the library is built",
        ),
        run: |c| {
            c.run_cargo(&[
                "test",
                "--locked",
                "-p",
                "brightfield-engine",
                "--features",
                "managed-duckdb",
                "--test",
                "managed_duckdb",
            ])?;
            c.run_cargo(&[
                "test",
                "--locked",
                "-p",
                "brightfield-shell",
                "--features",
                "managed-duckdb",
                "--test",
                "managed_duckdb_link",
            ])
        },
    },
    Step {
        name: "duckdb-parity",
        runs: "./scripts/check-duckdb-parity.sh",
        only_in_ci: None,
        only_at_night: Some(
            "it runs the shipped Protocols on the library the duckdb-managed step builds and \
             builds arc to run its engine tests there, 20 to 30 minutes; the nightly runs it \
             after that step, and --step runs it here once the library is built",
        ),
        run: |c| c.script("scripts/check-duckdb-parity.sh", &[]),
    },
];

/// Where the managed DuckDB build is kept: `BRIGHTFIELD_DUCKDB_MANAGED_DIR`
/// when set, which `scripts/build-duckdb.sh` reads too, else
/// `target/duckdb-managed` under `root`.
fn managed_duckdb_dir(root: &Path, var: Option<OsString>) -> PathBuf {
    match var {
        Some(dir) if !dir.is_empty() => root.join(dir),
        _ => root.join("target/duckdb-managed"),
    }
}

/// Build the managed edition's DuckDB unless the build on disk was made from
/// the current inputs, then check it. The stamp is removed before a rebuild
/// starts and written after it succeeds, so a build that stopped half way is
/// rebuilt rather than trusted. The self-test runs either way.
fn duckdb_managed(ctx: &Ctx) -> Result<(), String> {
    let dir = managed_duckdb_dir(
        &ctx.root,
        std::env::var_os("BRIGHTFIELD_DUCKDB_MANAGED_DIR"),
    );
    let hash = duckdb_stamp::inputs_hash(&ctx.root)?;
    if duckdb_stamp::build_is_current(&dir, &hash) {
        println!("the managed DuckDB build matches its inputs; not rebuilding");
    } else {
        duckdb_stamp::forget_build(&dir)?;
        ctx.script("scripts/build-duckdb.sh", &[])?;
        duckdb_stamp::record_build(&dir, &hash)?;
    }
    ctx.script("scripts/build-duckdb-selftest.sh", &[])
}

/// Compile what the test shards compile, so the job ahead of them can save
/// it under the lockfile's cache key and each shard restores an exact match.
/// Three builds, because the two steps after the suite each resolve their
/// own feature set: `--bin conformance` and the managed edition's
/// `-p brightfield-shell --features managed-edition` each select a different
/// graph from `--workspace`, and cargo builds `libduckdb-sys` again for each
/// graph that differs. The report on open-analytics builds its own at night.
/// A full
/// local run uses it too, as the row that builds every test binary before the
/// two test steps start together.
fn warm_cache(ctx: &Ctx) -> Result<(), String> {
    build_tests(ctx)?;
    ctx.run_cargo(&["build", "--locked", "--bin", "conformance"])?;
    ctx.run_cargo(&[
        "test",
        "--locked",
        "-p",
        "brightfield-shell",
        "--features",
        "managed-edition",
        "--test",
        "edition",
        "--test",
        "grants",
        "--test",
        "grant_on_open",
        "--test",
        "refused_access",
        "--test",
        "child_grants",
        "--no-run",
    ])
}

/// Compile every test target in the workspace without running any, so the
/// test steps that follow find their binaries built.
pub fn build_tests(ctx: &Ctx) -> Result<(), String> {
    ctx.run_cargo(&["test", "--locked", "--workspace", "--no-run"])
}

fn workflow_coverage(ctx: &Ctx) -> Result<(), String> {
    let files = workflows::read_all(&ctx.root)?;
    let names: Vec<&str> = STEPS.iter().map(|s| s.name).collect();
    let problems = workflows::coverage(&names, &files);
    for problem in &problems {
        report_error("the command and the workflows disagree", problem);
    }
    if problems.is_empty() {
        println!(
            "{} step(s), each invoked by a workflow; no workflow invokes an unknown step, calls a gate around the command, or starts a macOS job on its own off a release tag",
            names.len()
        );
        Ok(())
    } else {
        Err(format!(
            "{} disagreement(s) between the command and the workflow files, listed above",
            problems.len()
        ))
    }
}

fn doc(ctx: &Ctx) -> Result<(), String> {
    let args = [
        "doc",
        "--workspace",
        "--no-deps",
        "--document-private-items",
    ];
    let mut command = ctx.cargo(&args);
    command.env("RUSTDOCFLAGS", "-D warnings");
    run(
        command,
        "RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --document-private-items",
    )
}

fn cargo_deny(ctx: &Ctx, check: &str) -> Result<(), String> {
    let installed = ctx
        .cargo(&["deny", "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !installed {
        return Err(
            "cargo-deny is not installed: run `cargo install --locked cargo-deny`, then run again"
                .to_owned(),
        );
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
        return Err(format!(
            "`cargo metadata --locked --no-deps` exited with {}",
            output.status
        ));
    }
    let targets = shard::test_targets(&String::from_utf8_lossy(&output.stdout))?;
    let shard = shard::shard_from_env(|var| std::env::var(var).ok())?;
    shard::select(&serial, &seconds, &targets, shard.as_ref())
}

/// Where the DuckDB CLI is: `set`, ARC_DUCKDB_BIN's value, read against
/// `root` when relative, or `target/duckdb-cli/duckdb` under `root` when it is
/// unset or empty. Always absolute when `root` is.
pub(crate) fn engine_path(root: &Path, set: Option<OsString>) -> PathBuf {
    match set {
        Some(path) if !path.is_empty() => root.join(path),
        _ => root.join("target/duckdb-cli/duckdb"),
    }
}

/// The pinned DuckDB CLI the run tests hand to arc: ARC_DUCKDB_BIN when it is
/// set, `target/duckdb-cli/duckdb` when it is not, and either way accepted by
/// `scripts/fetch-duckdb-cli.sh --check` before a test runs.
pub(crate) fn pinned_duckdb(ctx: &Ctx) -> Result<PathBuf, String> {
    let host = host_triple()?;
    let engine = engine_path(&ctx.root, std::env::var_os("ARC_DUCKDB_BIN"));
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

pub(crate) fn host_triple() -> Result<String, String> {
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
    run_tests(ctx, engine, &all)
}

/// Run `cargo nextest run` over `args`, at the run's test threads: those
/// `RUST_TEST_THREADS` names when it is set, as cargo test reads it. nextest
/// takes its count from `--test-threads` or `NEXTEST_TEST_THREADS`.
fn cargo_nextest(ctx: &Ctx, engine: &Path, args: &[String]) -> Result<(), String> {
    let installed = ctx
        .cargo(&["nextest", "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !installed {
        return Err(
            "cargo-nextest is not installed: run `cargo install --locked cargo-nextest`, then run again"
                .to_owned(),
        );
    }
    let threads = std::env::var("RUST_TEST_THREADS")
        .ok()
        .filter(|value| value.parse::<usize>().is_ok_and(|n| n > 0));
    let mut all: Vec<&str> = vec!["nextest", "run", "--locked", "--workspace"];
    all.extend(args.iter().map(String::as_str));
    if let Some(threads) = &threads {
        all.extend(["--test-threads", threads]);
    }
    run_tests(ctx, engine, &all)
}

/// Run a cargo test command with the pinned DuckDB CLI and a temporary
/// folder of its own, writing to the step's log when it has one.
fn run_tests(ctx: &Ctx, engine: &Path, all: &[&str]) -> Result<(), String> {
    let mut command = ctx.cargo(all);
    command.env("ARC_DUCKDB_BIN", engine);
    let tmp = RunTmp::make()?;
    command.env("TMPDIR", &tmp.dir);
    let shown = format!("cargo {}", all.join(" "));
    if let Some(path) = &ctx.log {
        let open = || {
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| format!("cannot open {}: {e}", path.display()))
        };
        command.stdout(open()?).stderr(open()?);
    }
    run(command, &shown)
}

/// The prefix of a test run's temporary folder, in the system's.
const RUN_TMP_PREFIX: &str = "brightfield-tests-";

/// A test run's own temporary folder, `brightfield-tests-<pid>-<n>` in the
/// system's, removed when the run ends. Many test files keep a scratch folder
/// named for the test process under `std::env::temp_dir()`, and under
/// cargo-nextest every test is a process, so without this a session's
/// temporary folder would keep thousands of them per run. Folders a killed
/// run left, whose process is gone, are removed when the next run starts.
struct RunTmp {
    dir: PathBuf,
}

impl RunTmp {
    fn make() -> Result<Self, String> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let system = std::env::temp_dir();
        sweep_run_tmps(&system);
        let pid = std::process::id();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = system.join(format!("{RUN_TMP_PREFIX}{pid}-{n}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        Ok(Self { dir })
    }
}

impl Drop for RunTmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Remove the test-run folders in `system` whose process has ended.
fn sweep_run_tmps(system: &Path) {
    let Ok(entries) = fs::read_dir(system) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = run_tmp_pid(&name.to_string_lossy()) else {
            continue;
        };
        if pid != std::process::id() && !process_alive(pid) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// The process id in a test-run folder's name, `brightfield-tests-<pid>-<n>`.
fn run_tmp_pid(name: &str) -> Option<u32> {
    let (pid, n) = name.strip_prefix(RUN_TMP_PREFIX)?.split_once('-')?;
    n.parse::<usize>().ok()?;
    pid.parse().ok()
}

/// True when a process with this id is running, as `kill -0` answers.
fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// What to show of a test log after a failure. When cargo-nextest's run
/// failed, each failed test's block and the run's summary; otherwise libtest's
/// excerpt, from the last detailed `failures:` block onward, or the last 60
/// lines when the log has none. libtest prints that block, with a
/// `---- <test> stdout ----` section per failed test, and then a second
/// `failures:` block that lists only names; the detailed one is the block
/// whose first non-blank line is such a header.
pub fn failure_excerpt(log: &str) -> String {
    nextest_excerpt(log).unwrap_or_else(|| libtest_excerpt(log))
}

/// nextest's status word at the head of a line, as in `FAIL [   0.4s] ...`,
/// or `Summary` on its summary line.
fn nextest_status(line: &str) -> Option<&str> {
    let (word, rest) = line.trim_start().split_once(' ')?;
    let is_word = word == "Summary"
        || (!word.is_empty()
            && word
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-'));
    (is_word && rest.trim_start().starts_with('[')).then_some(word)
}

/// True for a status nextest gives a test that did not pass.
fn nextest_failed(word: &str) -> bool {
    matches!(word, "FAIL" | "TIMEOUT" | "ABORT" | "LEAK-FAIL") || word.starts_with("SIG")
}

/// Each failed test's block from a failed nextest run, then its summary
/// onward; `None` when the log holds no nextest summary after a failure.
fn nextest_excerpt(log: &str) -> Option<String> {
    let lines: Vec<&str> = log.lines().collect();
    let summary = lines
        .iter()
        .rposition(|line| nextest_status(line) == Some("Summary"))?;
    let failed_after = lines[summary + 1..]
        .iter()
        .any(|line| nextest_status(line).is_some_and(nextest_failed));
    if !failed_after {
        return None;
    }
    let mut excerpt: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in &lines[..summary] {
        match nextest_status(line) {
            Some(word) => inside = nextest_failed(word),
            None if line.starts_with("    ") || line.starts_with("  std") || line.is_empty() => {}
            None => inside = false,
        }
        if inside {
            excerpt.push(line);
        }
    }
    excerpt.extend(&lines[summary..]);
    Some(excerpt.join("\n"))
}

fn libtest_excerpt(log: &str) -> String {
    let lines: Vec<&str> = log.lines().collect();
    let blocks: Vec<usize> = (0..lines.len())
        .filter(|&at| lines[at].trim() == "failures:")
        .collect();
    let detailed = blocks.iter().rev().copied().find(|&at| {
        lines[at + 1..]
            .iter()
            .map(|line| line.trim())
            .find(|line| !line.is_empty())
            .is_some_and(|line| line.starts_with("----") && line.ends_with("stdout ----"))
    });
    let from = detailed
        .or(blocks.last().copied())
        .unwrap_or_else(|| lines.len().saturating_sub(60));
    lines[from..].join("\n")
}

fn test_one_thread(ctx: &Ctx) -> Result<(), String> {
    let Some(part) = selection(ctx)?.one_thread else {
        println!(
            "this shard runs the default-threads group; test-one-thread has nothing to run on it"
        );
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
    if let Some(path) = &ctx.log {
        // Start the log afresh; each cargo run below appends to it.
        File::create(path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    }
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
    // With no target flag `cargo nextest run --workspace` runs every target,
    // so an empty list runs nothing rather than that. nextest does not run
    // doctests, so cargo test runs them.
    if !args.is_empty() {
        cargo_nextest(ctx, &engine, &args)?;
    }
    if part.doc() {
        cargo_test(ctx, &engine, &["--doc".to_owned()], false)?;
    }
    Ok(())
}

/// The sibling open-analytics checkout: OPEN_ANALYTICS_DIR when set, else the
/// checkout beside this one, or beside the main checkout when this is a
/// worktree.
pub(crate) fn open_analytics(ctx: &Ctx) -> Result<PathBuf, String> {
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

/// The commit a git checkout at `dir` has out, shortened, or `unknown`.
fn short_head(ctx: &Ctx, dir: &Path) -> String {
    ctx.command("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|sha| !sha.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

/// The arcform revision `Cargo.lock` resolves `arc` to, shortened, read from
/// the `#<sha>` its git source ends with.
pub fn pinned_arc(lock: &str) -> Option<String> {
    let mut in_arcform = false;
    for line in lock.lines() {
        if line == "[[package]]" {
            in_arcform = false;
        } else if line == "name = \"arcform\"" {
            in_arcform = true;
        } else if in_arcform {
            if let Some(source) = line.strip_prefix("source = \"git+") {
                let sha = source.trim_end_matches('"').rsplit('#').next()?;
                return Some(sha.chars().take(7).collect());
            }
        }
    }
    None
}

/// What the report's tests found, from their output: the lines that begin
/// `open-analytics: `, which each test prints once with its finding. On one
/// thread with `--nocapture`, libtest prints `test <name> ... ` before a test
/// runs and no line break, so a test's first line follows that prefix; a
/// marker is read there too, and nowhere else in a line.
fn findings(log: &str) -> Vec<&str> {
    const MARKER: &str = "open-analytics: ";
    log.lines()
        .filter_map(|line| {
            let line = match line.strip_prefix("test ") {
                Some(rest) => rest.split_once(" ... ").map_or(line, |(_, after)| after),
                None => line,
            };
            line.strip_prefix(MARKER)
        })
        .map(str::trim)
        .collect()
}

/// The report on open-analytics: which of the Protocols on its main the arc
/// this repo pins refuses, and which parse but draw no lineage. A Protocol
/// refused is reported, not failed, since open-analytics' operators may run
/// ahead of the pin. The step fails only when it could not report: no
/// checkout, a checkout under the floors the test holds it to, or fewer
/// tests run than it has, since a report over nothing reads as full coverage.
fn sibling_manifests(ctx: &Ctx) -> Result<(), String> {
    if !selection(ctx)?.protocols {
        println!(
            "the report on open-analytics runs on the last default-threads shard, not this one"
        );
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
    let log = File::create(&log_path)
        .map_err(|e| format!("cannot create {}: {e}", log_path.display()))?;
    let log_err = log
        .try_clone()
        .map_err(|e| format!("cannot reopen {}: {e}", log_path.display()))?;
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
    let text = fs::read_to_string(&log_path)
        .map_err(|e| format!("cannot read {}: {e}", log_path.display()))?;
    print!("{text}");
    disk("after");
    let ran = executed_count(&text);
    println!(
        "--- sibling_manifests: cargo exited {status}, {ran} test(s) executed (floor {floor})"
    );
    let mut failures = Vec::new();
    if !status.success() {
        let message = format!("cargo test exited {status}: the panic above says why. A Protocol the pinned arc refuses does not fail this step, so the cause is the checkout (missing, or under its floors) or the test itself");
        report_error("the report on open-analytics could not run", &message);
        failures.push("the report on open-analytics could not run".to_owned());
    }
    if ran < floor {
        let message = format!("expected at least {floor} ignored test(s) to execute in sibling_manifests, {ran} executed: the filter matched nothing, or a covered test was renamed, deleted or un-ignored. A report here would cover nothing");
        report_error("executed too few tests", &message);
        failures.push(format!(
            "{ran} of at least {floor} sibling-manifest tests executed"
        ));
    }
    if !failures.is_empty() {
        return Err(failures.join("; "));
    }
    let lock = fs::read_to_string(ctx.root.join("Cargo.lock"))
        .map_err(|e| format!("cannot read Cargo.lock: {e}"))?;
    let arc = pinned_arc(&lock).unwrap_or_else(|| "unknown".to_owned());
    // What the pin refuses leads, since GitHub cuts a status at 140
    // characters and the nightly posts this line as one.
    let mut found = findings(&text);
    found.sort_by_key(|finding| !finding.contains("pinned arc"));
    let found = if found.is_empty() {
        "the tests printed no finding".to_owned()
    } else {
        found.join("; ")
    };
    println!(
        "open-analytics report: arc {arc} against open-analytics {}: {found}",
        short_head(ctx, &dir)
    );
    Ok(())
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

/// The managed edition's own tests, in a build with its feature on: the
/// workspace suite builds the full edition only, so nothing else compiles or
/// runs them. It runs on the last default-threads shard beside the two steps
/// before it, since its build of the shell crate is a second one.
///
/// The second command runs the grant store's ignored test, which attaches a
/// disk image to unmount a granted folder's drive. Its executed count is held
/// to a floor, because `-- --ignored` that matches nothing exits 0.
fn managed_edition(ctx: &Ctx) -> Result<(), String> {
    if !selection(ctx)?.protocols {
        println!("the managed edition's tests run on the last default-threads shard, not this one");
        return Ok(());
    }
    let feature = [
        "test",
        "--locked",
        "-p",
        "brightfield-shell",
        "--features",
        "managed-edition",
    ];
    let suite: Vec<&str> = feature
        .iter()
        .copied()
        .chain([
            "--test",
            "edition",
            "--test",
            "grants",
            "--test",
            "grant_on_open",
            "--test",
            "refused_access",
            "--test",
            "child_grants",
        ])
        .collect();
    ctx.run_cargo(&suite)?;
    if !cfg!(target_os = "macos") {
        println!("the grant store's disk-image test attaches a disk image with hdiutil, so it is built and run on macOS only");
        return Ok(());
    }
    let floor = 1;
    let log_path = ctx.scratch()?.join("managed-edition-ignored.log");
    let log = File::create(&log_path)
        .map_err(|e| format!("cannot create {}: {e}", log_path.display()))?;
    let log_err = log
        .try_clone()
        .map_err(|e| format!("cannot reopen {}: {e}", log_path.display()))?;
    let ignored: Vec<&str> = feature
        .iter()
        .copied()
        .chain(["--test", "grants", "--", "--ignored", "--test-threads=1"])
        .collect();
    let status = ctx
        .cargo(&ignored)
        .stdout(log)
        .stderr(log_err)
        .status()
        .map_err(|e| format!("could not start cargo test: {e}"))?;
    let text = fs::read_to_string(&log_path)
        .map_err(|e| format!("cannot read {}: {e}", log_path.display()))?;
    print!("{text}");
    let ran = executed_count(&text);
    if !status.success() {
        return Err(format!(
            "`cargo {}` exited with {status}",
            ignored.join(" ")
        ));
    }
    if ran < floor {
        let message = format!("expected at least {floor} ignored test(s) to execute in the grant store's tests, {ran} executed: a covered test was renamed, deleted or un-ignored");
        report_error("executed too few tests", &message);
        return Err(format!(
            "{ran} of at least {floor} disk-image tests executed"
        ));
    }
    println!("{ran} ignored grant-store test(s) executed and passed (floor {floor})");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_duckdb_path_is_read_from_the_checkout_root() {
        let root = Path::new("/checkout");
        assert_eq!(
            engine_path(root, Some("target/duckdb-cli/duckdb".into())),
            PathBuf::from("/checkout/target/duckdb-cli/duckdb")
        );
        assert_eq!(
            engine_path(root, Some("/opt/duckdb/duckdb".into())),
            PathBuf::from("/opt/duckdb/duckdb")
        );
        assert_eq!(
            engine_path(root, Some("".into())),
            PathBuf::from("/checkout/target/duckdb-cli/duckdb")
        );
        assert_eq!(
            engine_path(root, None),
            PathBuf::from("/checkout/target/duckdb-cli/duckdb")
        );
    }

    #[test]
    fn the_managed_build_lives_where_the_script_puts_it() {
        let root = Path::new("/checkout");
        assert_eq!(
            managed_duckdb_dir(root, None),
            PathBuf::from("/checkout/target/duckdb-managed")
        );
        assert_eq!(
            managed_duckdb_dir(root, Some("/cache/duckdb".into())),
            PathBuf::from("/cache/duckdb")
        );
    }

    #[test]
    fn the_nightly_only_steps_come_last() {
        let night: Vec<&str> = STEPS
            .iter()
            .filter(|s| s.only_at_night.is_some())
            .map(|s| s.name)
            .collect();
        assert_eq!(
            night,
            [
                "sibling-manifests",
                "nightly-network",
                "duckdb-managed",
                "managed-duckdb-test",
                "duckdb-parity"
            ]
        );
        assert!(STEPS[STEPS.len() - night.len()..]
            .iter()
            .all(|s| s.only_at_night.is_some()));
    }

    #[test]
    fn warm_cache_runs_locally_before_the_suite() {
        let at = |name: &str| STEPS.iter().position(|s| s.name == name).expect(name);
        assert!(STEPS[at("warm-cache")].only_in_ci.is_none());
        assert!(at("warm-cache") < at("test-one-thread"));
    }

    #[test]
    fn the_failures_section_is_taken_from_the_log() {
        let with = "running 2 tests\ntest a ... ok\ntest b ... FAILED\n\nfailures:\n\n---- b stdout ----\nthread 'b' panicked at src/lib.rs:3:5:\nassertion failed: the panic message\n\n\nfailures:\n    b\n\ntest result: FAILED. 1 passed; 1 failed\n";
        let excerpt = failure_excerpt(with);
        assert!(excerpt.starts_with("failures:\n\n---- b stdout ----"));
        assert!(excerpt.contains("assertion failed: the panic message"));
        assert!(excerpt.ends_with("test result: FAILED. 1 passed; 1 failed"));
        let without: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        let tail = failure_excerpt(&without);
        assert_eq!(tail.lines().count(), 60);
        assert!(tail.starts_with("line 41\n") && tail.ends_with("line 100"));
    }

    #[test]
    fn a_failed_nextest_run_shows_each_failed_test_and_the_summary() {
        let log = "   Compiling a v0.1.0\n        PASS [   1.0s] (1/3) a::t one\n        FAIL [   0.4s] (2/3) a::t two\n  stdout ───\n\n    running 1 test\n    test two ... FAILED\n\n  stderr ───\n\n    thread 'two' panicked at t.rs:3:5:\n    the message\n\n        PASS [   2.0s] (3/3) a::t three\n     SIGSEGV [   7.3s] (4/4) a::t four\n  stdout ───\n    test four ... ok\n    (test aborted with signal 11: SIGSEGV)\n\n────────────\n     Summary [  10.0s] 4 tests run: 2 passed, 2 failed, 0 skipped\n        FAIL [   0.4s] (2/3) a::t two\n     SIGSEGV [   7.3s] (4/4) a::t four\nerror: test run failed\n";
        let excerpt = failure_excerpt(log);
        assert!(
            excerpt.starts_with("        FAIL [   0.4s] (2/3) a::t two"),
            "{excerpt}"
        );
        assert!(excerpt.contains("the message"));
        assert!(excerpt.contains("(test aborted with signal 11: SIGSEGV)"));
        assert!(!excerpt.contains("a::t one") && !excerpt.contains("a::t three"));
        assert!(!excerpt.contains("────────────"));
        assert!(excerpt.ends_with("error: test run failed"));
    }

    #[test]
    fn a_passed_nextest_run_leaves_a_doctest_failure_to_libtest() {
        let log = "        PASS [   1.0s] (1/1) a::t one\n     Summary [   1.0s] 1 test run: 1 passed, 0 skipped\n   Doc-tests a\nfailures:\n\n---- src/lib.rs - f (line 3) stdout ----\nboom\n\nfailures:\n    src/lib.rs - f (line 3)\n\ntest result: FAILED. 0 passed; 1 failed\n";
        let excerpt = failure_excerpt(log);
        assert!(excerpt.starts_with("failures:\n\n---- src/lib.rs - f (line 3) stdout ----"));
    }

    #[test]
    fn a_test_run_folder_is_swept_once_its_process_has_ended() {
        let system = std::env::temp_dir().join(format!("xtask-sweep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&system);
        fs::create_dir_all(&system).unwrap();
        let mut ended = Command::new("true").spawn().unwrap();
        let ended_pid = ended.id();
        ended.wait().unwrap();
        let mut live = Command::new("sleep").arg("30").spawn().unwrap();
        let names = [
            format!("{RUN_TMP_PREFIX}{ended_pid}-0"),
            format!("{RUN_TMP_PREFIX}{}-3", live.id()),
            format!("{RUN_TMP_PREFIX}{}-1", std::process::id()),
            format!("{RUN_TMP_PREFIX}{ended_pid}"),
            "bf-scenes-1".to_owned(),
        ];
        for name in &names {
            fs::create_dir_all(system.join(name)).unwrap();
        }
        sweep_run_tmps(&system);
        let kept: Vec<bool> = names.iter().map(|n| system.join(n).exists()).collect();
        let _ = live.kill();
        let _ = live.wait();
        let _ = fs::remove_dir_all(&system);
        assert_eq!(kept, [false, true, true, true, true], "{names:?}");
    }

    #[test]
    fn a_test_run_folder_is_removed_when_the_run_ends() {
        let tmp = RunTmp::make().unwrap();
        let dir = tmp.dir.clone();
        assert!(dir.is_dir());
        assert_eq!(
            run_tmp_pid(&dir.file_name().unwrap().to_string_lossy()),
            Some(std::process::id())
        );
        drop(tmp);
        assert!(!dir.exists());
    }

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

    #[test]
    fn the_pinned_arc_is_read_from_the_lockfile_and_this_one_has_it() {
        let lock = "[[package]]\nname = \"arc-other\"\nsource = \"git+https://x?rev=aaa#1111111111\"\n\n\
                    [[package]]\nname = \"arcform\"\nversion = \"0.2.0\"\n\
                    source = \"git+https://github.com/meridian-online/arcform?rev=f5f3f07#f5f3f071bd10c04a\"\n";
        assert_eq!(pinned_arc(lock).as_deref(), Some("f5f3f07"));
        assert_eq!(pinned_arc("[[package]]\nname = \"arcform\"\n"), None);
        let ours = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../Cargo.lock"))
            .unwrap();
        assert!(pinned_arc(&ours).is_some_and(|sha| sha.len() == 7));
    }

    #[test]
    fn the_findings_are_the_lines_the_tests_mark() {
        let log = "test a ... ok\nopen-analytics: the pinned arc refuses 1 of 25 Protocols\n\
                   a line naming open-analytics: in passing\n\
                   open-analytics: all 24 that parse draw lineage\n";
        assert_eq!(
            findings(log),
            [
                "the pinned arc refuses 1 of 25 Protocols",
                "all 24 that parse draw lineage"
            ]
        );
        // The shape a one-thread `--nocapture` run prints: each test's first
        // line follows libtest's `test <name> ... ` on the same line.
        let real = "test reports_the_shipped_protocols_that_draw_no_lineage ... open-analytics: all 38 that parse draw lineage\n\
                    ok\n\
                    test reports_the_shipped_protocols_the_pinned_arc_refuses ... refused by the pinned arc: datasets/x/arcform.yaml: unknown operator 'dlt'\n\
                    open-analytics: the pinned arc refuses 1 of 39 Protocol(s): datasets/x/arcform.yaml\n\
                    ok\n\
                    test a_name ... a line naming open-analytics: in passing\n";
        assert_eq!(
            findings(real),
            [
                "all 38 that parse draw lineage",
                "the pinned arc refuses 1 of 39 Protocol(s): datasets/x/arcform.yaml"
            ]
        );
    }
}
