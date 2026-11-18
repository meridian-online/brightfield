//! What the pull-request gate and the nightly gate share: the gate lock, a
//! run's scratch directory and worktree, and the isolated environment the
//! tree's `cargo xtask ci` runs under, with its output teed to a log.

use std::fs::{self, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, SystemTime};

use crate::gate::{output_within, CALL_TIMEOUT};
use crate::steps::{self, Ctx};

/// A run's temporary directory, and its worktree when it made one, removed
/// when dropped.
pub struct Scratch {
    /// The checkout the worktree was made from.
    pub root: PathBuf,
    /// The run's fresh HOME and TMPDIR, in the system's temporary directory.
    pub dir: PathBuf,
    /// The run's own worktree, from [`tree_beside_target`]. The nightly runs
    /// in the tree it was started from and has none, since that tree is not
    /// the run's to remove.
    pub tree: Option<PathBuf>,
}

/// Where a run's worktree goes: under the gate's directory, beside its
/// persistent `target`, as a checkout's tree sits beside its own `target` in
/// CI. A test that spells a scratch file relative to its working directory
/// then climbs a few levels to the gate's directory, as it climbs two in CI,
/// rather than to `/`; a climb to `/` reaches the same file from anywhere
/// shallower, which `status_rail`'s relative-watch test refuses.
pub fn tree_beside_target(gate_dir: &Path, run: &str) -> PathBuf {
    gate_dir.join("trees").join(run)
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
        let Some(tree) = &self.tree else {
            return;
        };
        if tree.exists() {
            let _ = Command::new("git")
                .arg("-C")
                .arg(&self.root)
                .args(["worktree", "remove", "--force"])
                .arg(tree)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = fs::remove_dir_all(tree);
        let _ = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["worktree", "prune"])
            .status();
    }
}

/// True for a variable the gate's run must not inherit: one that points the
/// run back into the real HOME, or picks the toolchain over the tree's own
/// file, or changes what is built (build, per-target and profile settings, and
/// the C and C++ compilers and flags the `cc` builds read, since a `CXXFLAGS`
/// here would override the DuckDB version define in `.cargo/config.toml`), so
/// the verdict is CI's. `CARGO_BUILD_JOBS` stays, since it caps the
/// machine's build slots and changes no output, and `CARGO_TARGET_DIR` is the
/// gate's own.
pub fn removed_from_child(key: &str) -> bool {
    matches!(
        key,
        "RUSTUP_TOOLCHAIN"
            | "CARGO"
            | "XDG_CONFIG_HOME"
            | "XDG_CACHE_HOME"
            | "XDG_DATA_HOME"
            | "XDG_STATE_HOME"
            | "RUNNER_TEMP"
            | "RUSTFLAGS"
            | "RUSTDOCFLAGS"
            | "CARGO_ENCODED_RUSTFLAGS"
            | "CARGO_ENCODED_RUSTDOCFLAGS"
            | "RUSTC_WRAPPER"
            | "RUSTC_WORKSPACE_WRAPPER"
            | "MACOSX_DEPLOYMENT_TARGET"
    ) || is_compiler_variable(key)
        || (key.starts_with("CARGO_BUILD_") && key != "CARGO_BUILD_JOBS")
        || (key.starts_with("CARGO_TARGET_") && key != "CARGO_TARGET_DIR")
        || key.starts_with("CARGO_PROFILE_")
}

/// The C and C++ compiler and flag variables the `cc` crate reads, in each
/// form it reads them: plain, `HOST_` or `TARGET_` prefixed, and suffixed with
/// a target triple (`CXXFLAGS_aarch64_apple_darwin`, `CC_aarch64-apple-darwin`).
fn is_compiler_variable(key: &str) -> bool {
    const BASES: [&str; 7] = [
        "CC", "CXX", "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "AR",
    ];
    let bare = key
        .strip_prefix("HOST_")
        .or_else(|| key.strip_prefix("TARGET_"))
        .unwrap_or(key);
    BASES.iter().any(|base| {
        bare.strip_prefix(base)
            .is_some_and(|rest| rest.is_empty() || (bare == key && rest.starts_with('_')))
    })
}

/// Why the isolated run could not be set up.
#[derive(Debug)]
pub enum Unready {
    /// The run's fresh HOME or TMPDIR could not be made.
    Scratch(String),
    /// The DuckDB CLI the tree pins could not be found or fetched.
    DuckDb(String),
    /// The real `open-analytics` checkout could not be found.
    OpenAnalytics(String),
}

/// The tree's `cargo xtask ci`, unstarted, under the gate's isolated
/// environment: a fresh HOME and TMPDIR under `scratch_dir`, the real cargo
/// and rustup homes under `home`, the persistent target directory under
/// `gate_dir`, the DuckDB CLI the tree pins, the real `open-analytics`
/// checkout found from `invoking`, and none of the variables
/// `removed_from_child` names. The caller adds the run's own arguments.
pub fn isolated_ci(
    invoking: &Path,
    tree: &Path,
    home: &Path,
    gate_dir: &Path,
    scratch_dir: &Path,
) -> Result<Command, Unready> {
    let (child_home, child_tmp) = (scratch_dir.join("home"), scratch_dir.join("tmp"));
    for dir in [&child_home, &child_tmp] {
        fs::create_dir_all(dir)
            .map_err(|e| Unready::Scratch(format!("cannot create {}: {e}", dir.display())))?;
    }
    let invoking = Ctx {
        root: invoking.to_path_buf(),
        cargo: "cargo".into(),
        log: None,
    };
    let host = steps::host_triple().map_err(Unready::DuckDb)?;
    let candidate = steps::engine_path(&invoking.root, std::env::var_os("ARC_DUCKDB_BIN"));
    let engine = engine_for_tree(
        &tree.join("scripts/fetch-duckdb-cli.sh"),
        &host,
        &candidate,
        &gate_dir.join("duckdb-cli"),
    )
    .map_err(Unready::DuckDb)?;
    let open_analytics = steps::open_analytics(&invoking)
        .and_then(|dir| {
            fs::canonicalize(&dir).map_err(|e| format!("cannot resolve {}: {e}", dir.display()))
        })
        .map_err(Unready::OpenAnalytics)?;
    let real = |var: &str, under_home: &str| {
        std::env::var_os(var)
            .filter(|v| !v.is_empty())
            .map_or_else(|| home.join(under_home), PathBuf::from)
    };
    let tree_ctx = Ctx {
        root: tree.to_path_buf(),
        cargo: "cargo".into(),
        log: None,
    };
    let mut command = tree_ctx.command("cargo");
    command
        .arg("xtask")
        .arg("ci")
        .env("HOME", &child_home)
        .env("TMPDIR", &child_tmp)
        .env("CARGO_HOME", real("CARGO_HOME", ".cargo"))
        .env("RUSTUP_HOME", real("RUSTUP_HOME", ".rustup"))
        .env("CARGO_TARGET_DIR", gate_dir.join("target"))
        .env("ARC_DUCKDB_BIN", &engine)
        .env("OPEN_ANALYTICS_DIR", &open_analytics);
    for (key, _) in std::env::vars_os() {
        if removed_from_child(&key.to_string_lossy()) {
            command.env_remove(&key);
        }
    }
    Ok(command)
}

/// Run `command` with its output and errors copied to the terminal and to
/// `log`, and return how it exited.
pub fn run_teed(mut command: Command, log: &Path) -> Result<ExitStatus, String> {
    let mut file =
        fs::File::create(log).map_err(|e| format!("cannot create {}: {e}", log.display()))?;
    let (mut reader, writer) = std::io::pipe().map_err(|e| format!("cannot make a pipe: {e}"))?;
    let writer_copy = writer
        .try_clone()
        .map_err(|e| format!("cannot make a pipe: {e}"))?;
    command
        .stdin(Stdio::null())
        .stdout(writer_copy)
        .stderr(writer);
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start the tree's `cargo xtask ci`: {e}"))?;
    // The command holds the pipe's write ends; drop it so the read ends.
    drop(command);
    let mut terminal = std::io::stderr();
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                let _ = terminal.write_all(&buffer[..n]);
                let _ = file.write_all(&buffer[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    child
        .wait()
        .map_err(|e| format!("lost the tree's `cargo xtask ci`: {e}"))
}

/// How long the tree's script may take to download the DuckDB CLI.
const FETCH_TIMEOUT: Duration = Duration::from_secs(600);

/// The DuckDB CLI the tree pins, judged by the tree's own
/// `fetch-duckdb-cli.sh`, since the tree's `cargo xtask ci` checks the engine
/// it is handed with that script and a pull request may move the pin. The
/// invoking checkout's engine (`candidate`) is used when the tree's script
/// accepts it; then the gate's own copy in `cache`; otherwise the tree's
/// script fetches its pinned release into `cache`, checking its hashes.
fn engine_for_tree(
    script: &Path,
    host: &str,
    candidate: &Path,
    cache: &Path,
) -> Result<PathBuf, String> {
    let accepts = |dir: &Path| {
        output_within(
            script,
            &["--check", host, &dir.to_string_lossy()],
            CALL_TIMEOUT,
        )
        .is_ok()
    };
    if candidate.file_name().is_some_and(|name| name == "duckdb") {
        if let Some(dir) = candidate.parent() {
            if accepts(dir) {
                return Ok(candidate.to_path_buf());
            }
        }
    }
    if accepts(cache) {
        return Ok(cache.join("duckdb"));
    }
    fs::create_dir_all(cache).map_err(|e| format!("cannot create {}: {e}", cache.display()))?;
    output_within(script, &[host, &cache.to_string_lossy()], FETCH_TIMEOUT)
        .map_err(|e| format!("could not fetch the DuckDB CLI the branch pins: {e}"))?;
    if accepts(cache) {
        Ok(cache.join("duckdb"))
    } else {
        Err(format!(
            "the DuckDB CLI fetched into {} is not the one the branch pins",
            cache.display()
        ))
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
        Lock::acquire_by(dir, poll, None, say)
    }

    /// `acquire`, giving up once `deadline` has passed with the lock still
    /// held elsewhere, as `a_lock_wait_past_the_deadline_gives_up` checks. A
    /// deadline already past still tries the lock once, as
    /// `a_free_lock_is_taken_even_past_the_deadline` checks.
    pub fn acquire_until(
        dir: &Path,
        poll: Duration,
        deadline: SystemTime,
        say: impl Fn(&str),
    ) -> Result<Lock, String> {
        Lock::acquire_by(dir, poll, Some(deadline), say)
    }

    fn acquire_by(
        dir: &Path,
        poll: Duration,
        deadline: Option<SystemTime>,
        say: impl Fn(&str),
    ) -> Result<Lock, String> {
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
                    if deadline.is_some_and(|deadline| SystemTime::now() >= deadline) {
                        return Err(format!(
                            "another gate still held {} at the deadline",
                            path.display()
                        ));
                    }
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

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
    fn a_free_lock_is_taken_even_past_the_deadline() {
        let dir = scratch("free-late");
        let past = SystemTime::now() - Duration::from_secs(60);
        let held = Lock::acquire_until(&dir, Duration::from_millis(10), past, |_| {}).unwrap();
        drop(held);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A stand-in for the tree's `fetch-duckdb-cli.sh` whose pin is `pin`:
    /// `--check` accepts a directory whose `duckdb` holds that text, and a
    /// fetch writes it, logging each fetch.
    fn fetch_script(dir: &Path, pin: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("fetch-duckdb-cli.sh");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nif [ \"$1\" = --check ]; then [ \"$(cat \"$3/duckdb\" 2>/dev/null)\" = {pin} ]; exit $?; fi\necho fetched >> '{}'\nprintf {pin} > \"$2/duckdb\"\necho \"$2/duckdb\"\n",
                dir.join("fetches").display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[test]
    fn the_engine_is_judged_by_the_trees_pin_and_fetched_once_when_it_moves() {
        let dir = scratch("engine");
        let invoking = dir.join("invoking");
        fs::create_dir_all(&invoking).unwrap();
        fs::write(invoking.join("duckdb"), "v1").unwrap();
        let cache = dir.join("cache");

        // The branch keeps main's pin: the invoking checkout's engine serves.
        let same = fetch_script(&dir, "v1");
        let engine = engine_for_tree(&same, "host", &invoking.join("duckdb"), &cache).unwrap();
        assert_eq!(engine, invoking.join("duckdb"));
        assert!(!dir.join("fetches").exists());

        // The branch moves the pin: the tree's script fetches into the cache,
        // once, and a second gate on the same pin reuses that copy.
        let moved = fetch_script(&dir, "v2");
        for _ in 0..2 {
            let engine = engine_for_tree(&moved, "host", &invoking.join("duckdb"), &cache).unwrap();
            assert_eq!(engine, cache.join("duckdb"));
        }
        let fetches = fs::read_to_string(dir.join("fetches")).unwrap();
        assert_eq!(fetches.lines().count(), 1, "{fetches}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_tree_sits_beside_the_gate_target_so_relative_climbs_stay_short() {
        let gate_dir = Path::new("/home/someone/.cache/meridian/gate");
        let tree = tree_beside_target(gate_dir, "brightfield-gate-1-2");
        let cwd = tree.join("crates/brightfield-shell");
        let scratch = gate_dir.join("target/tmp/some-test");
        let shared = cwd
            .components()
            .zip(scratch.components())
            .take_while(|(a, b)| a == b)
            .count();
        // The climb from a test's working directory to its scratch file stops
        // at the gate's directory, never at the filesystem root.
        assert_eq!(shared, gate_dir.components().count());
        assert_eq!(cwd.components().count() - shared, 4);
    }

    #[test]
    fn a_scratch_without_a_tree_leaves_the_tree_it_ran_in() {
        let dir = scratch("no-tree");
        let tree = dir.join("tree");
        let run_dir = dir.join("run");
        fs::create_dir_all(&tree).unwrap();
        fs::create_dir_all(&run_dir).unwrap();
        drop(Scratch {
            root: tree.clone(),
            dir: run_dir.clone(),
            tree: None,
        });
        assert!(tree.is_dir(), "the tree the run was started from is kept");
        assert!(!run_dir.exists(), "the run's own directory is removed");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_child_loses_build_flags_and_keeps_the_job_cap() {
        for key in [
            "RUSTFLAGS",
            "RUSTDOCFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
            "RUSTC_WRAPPER",
            "CARGO_BUILD_TARGET",
            "CARGO_BUILD_RUSTFLAGS",
            "RUSTUP_TOOLCHAIN",
            "XDG_CONFIG_HOME",
            "CXXFLAGS",
            "CC",
            "MACOSX_DEPLOYMENT_TARGET",
            "HOST_CXXFLAGS",
            "TARGET_CC",
            "CXXFLAGS_aarch64_apple_darwin",
            "CC_aarch64-apple-darwin",
        ] {
            assert!(removed_from_child(key), "{key} should not reach the run");
        }
        for key in [
            "CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS",
            "CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER",
            "CARGO_PROFILE_DEV_OPT_LEVEL",
            "CARGO_PROFILE_RELEASE_LTO",
        ] {
            assert!(removed_from_child(key), "{key} should not reach the run");
        }
        for key in [
            "PATH",
            "CARGO_BUILD_JOBS",
            "RUST_TEST_THREADS",
            "HOME",
            "CARGO_TARGET_DIR",
            "ARC_DUCKDB_BIN",
            "CCACHE_DIR",
        ] {
            assert!(!removed_from_child(key), "{key} should reach the run");
        }
    }
}
