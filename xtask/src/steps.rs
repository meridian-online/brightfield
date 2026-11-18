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
        name: "formula-layout-selftest",
        runs: "./scripts/check-formula-layout-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/check-formula-layout-selftest.sh", &[]),
    },
    Step {
        name: "formula-asset-selftest",
        runs: "./scripts/check-formula-asset-selftest.sh",
        only_in_ci: None,
        only_at_night: None,
        run: |c| c.script("scripts/check-formula-asset-selftest.sh", &[]),
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
        runs: "cargo test --locked --workspace --no-run; the sibling-manifest and conformance builds",
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
        runs: "cargo test --locked --workspace over every other target, then --doc",
        only_in_ci: None,
        only_at_night: None,
        run: test_default,
    },
    Step {
        name: "sibling-manifests",
        runs: "cargo test --locked -p brightfield-protocol --test sibling_manifests -- --ignored, at least 2 executed",
        only_in_ci: None,
        only_at_night: None,
        run: sibling_manifests,
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
/// Four builds, because the three steps after the suite each resolve their
/// own feature set: `-p brightfield-protocol --test sibling_manifests`,
/// `--bin conformance` and the managed edition's `-p brightfield-shell
/// --features managed-edition` each select a different graph from
/// `--workspace`, and cargo builds `libduckdb-sys` again for each graph that
/// differs. A full
/// local run uses it too, as the row that builds every test binary before the
/// two test steps start together.
fn warm_cache(ctx: &Ctx) -> Result<(), String> {
    build_tests(ctx)?;
    ctx.run_cargo(&[
        "test",
        "--locked",
        "-p",
        "brightfield-protocol",
        "--test",
        "sibling_manifests",
        "--no-run",
    ])?;
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
    let mut command = ctx.cargo(&all);
    command.env("ARC_DUCKDB_BIN", engine);
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

/// What to show of a test log after a failure: from the last detailed
/// `failures:` block onward, or the last 60 lines when the log has none.
/// libtest prints that block, with a `---- <test> stdout ----` section per
/// failed test, and then a second `failures:` block that lists only names; the
/// detailed one is the block whose first non-blank line is such a header.
pub fn failure_excerpt(log: &str) -> String {
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
        let message = format!("cargo test exited {status}: the failure above names the manifest and quotes arc's own diagnostic. Bump the pin: README section \"The arcform dependency\" (the arc rev and Cargo.lock, in one commit)");
        report_error("the pinned arc cannot load a shipped Protocol", &message);
        failures.push("the pinned arc cannot load a shipped Protocol".to_owned());
    }
    if ran < floor {
        let message = format!("expected at least {floor} ignored test(s) to execute in sibling_manifests, {ran} executed: the filter matched nothing, or a covered test was renamed, deleted or un-ignored. A green run here would prove nothing");
        report_error("executed too few tests", &message);
        failures.push(format!(
            "{ran} of at least {floor} sibling-manifest tests executed"
        ));
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
