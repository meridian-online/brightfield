//! The fixed machine profile recorded beside every result. A number without
//! its machine is not a result.

use std::path::Path;

use serde::Serialize;

/// Where the numbers were produced: hardware, OS, GPU adapter, toolchain,
/// build profile, repo commit, and the moment of capture.
#[derive(Debug, Clone, Serialize)]
pub struct MachineProfile {
    /// CPU brand string (e.g. `Apple M3 Pro`).
    pub cpu: String,
    /// Logical CPU count.
    pub logical_cpus: String,
    /// Physical memory, GiB (rounded).
    pub memory_gib: String,
    /// OS name + version.
    pub os: String,
    /// wgpu adapter the frames rendered on: name, backend, device type.
    pub gpu_adapter: String,
    /// `rustc --version` of the compiler that built the harness.
    pub rustc: String,
    /// Cargo build profile the harness ran under.
    pub build_profile: String,
    /// The tree the harness ran from: `git rev-parse --short HEAD`, with
    /// `-dirty` appended when the working tree carried uncommitted tracked
    /// changes at capture.
    ///
    /// **The suffix is the point of the field.** A bare commit id says a
    /// reader can check out that commit and get these bytes, and a record
    /// captured from a dirty tree says that falsely — this repo has shipped
    /// one, naming a commit that predated the flag the run was invoked with.
    /// `a_clean_tree_and_a_dirty_one_are_not_recorded_the_same_way` in
    /// `crates/brightfield-bench/src/machine.rs` drives both branches against
    /// real repositories.
    pub commit: String,
    /// Capture timestamp, RFC 3339, local offset.
    pub captured_at: String,
    /// DuckDB library version the queries executed on.
    pub duckdb: String,
}

impl MachineProfile {
    /// Collect the profile, best-effort: a field that cannot be read records
    /// `unknown` rather than failing the run — but the run prints what it
    /// could not read, because a baseline with an anonymous machine is weaker
    /// evidence.
    pub fn collect(duckdb_version: String) -> Self {
        let profile = if cfg!(debug_assertions) {
            "debug".to_string()
        } else {
            "release".to_string()
        };
        Self {
            cpu: sh("sysctl", &["-n", "machdep.cpu.brand_string"])
                .or_else(first_cpu_model_linux)
                .unwrap_or_else(unknown),
            logical_cpus: std::thread::available_parallelism()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| unknown()),
            memory_gib: sysctl_mem_gib()
                .or_else(mem_total_gib_linux)
                .unwrap_or_else(unknown),
            os: os_string(),
            gpu_adapter: adapter_string(),
            rustc: sh("rustc", &["--version"]).unwrap_or_else(unknown),
            build_profile: profile,
            commit: commit_id_at(Path::new(".")).unwrap_or_else(unknown),
            captured_at: chrono::Local::now().to_rfc3339(),
            duckdb: duckdb_version,
        }
    }
}

fn unknown() -> String {
    "unknown".to_string()
}

/// The short commit for the tree at `dir`, suffixed `-dirty` when tracked
/// files differ from it.
///
/// `git status --porcelain --untracked-files=no` is the question, and
/// untracked files are excluded deliberately: a scratch note beside the
/// checkout does not change what the harness compiled.
///
/// **An unanswerable status question loses the whole field rather than
/// reporting a clean id.** The suffix exists because a bare id is a promise
/// that checking out that commit reproduces these bytes; an id recorded
/// without knowing whether the tree was clean makes that promise on no
/// evidence, which is the failure this replaced.
fn commit_id_at(dir: &Path) -> Option<String> {
    let head = git_at(dir, &["rev-parse", "--short", "HEAD"])?;
    let status = git_at(dir, &["status", "--porcelain", "--untracked-files=no"])?;
    Some(if status.trim().is_empty() {
        head.trim().to_string()
    } else {
        format!("{}-dirty", head.trim())
    })
}

/// `git` in `dir`, stdout as a string — `None` where it could not be run or
/// exited non-zero. Empty stdout is a real answer here, which is why this does
/// not go through [`sh`].
fn git_at(dir: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// Run a command and return its trimmed stdout, `None` on any failure. Shared
/// with the memory probe, which reads resident size the same best-effort way.
pub(crate) fn sh(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn first_cpu_model_linux() -> Option<String> {
    let text = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    text.lines()
        .find(|l| l.starts_with("model name"))
        .and_then(|l| l.split(':').nth(1))
        .map(|s| s.trim().to_string())
}

/// Physical memory in GiB via macOS `sysctl hw.memsize`, which reports exact
/// bytes; `None` off macOS or on any read/parse failure. Powers-of-two exact,
/// so integer floor and rounding agree — no rounding needed here.
fn sysctl_mem_gib() -> Option<String> {
    let bytes = sh("sysctl", &["-n", "hw.memsize"])?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(format!("{}", bytes / (1024 * 1024 * 1024)))
}

/// Physical memory in GiB from Linux `/proc/meminfo`; `None` off Linux or on
/// any read/parse failure. Split from the parse ([`parse_meminfo_total_gib`])
/// so the conversion is testable from a fixture string on any host.
fn mem_total_gib_linux() -> Option<String> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    parse_meminfo_total_gib(&text)
}

/// GiB from the `MemTotal:` line of a `/proc/meminfo` dump. The field's unit is
/// kibibytes (Linux labels it `kB` but means KiB), so GiB = KiB / 1024². Unlike
/// `hw.memsize`, `MemTotal` reports usable RAM AFTER the kernel's reservations,
/// so it always sits a little under the nominal capacity (e.g. ~15.5 for a
/// 16-GiB box); rounding recovers the number a human would name, where a floor
/// would under-report by one. Returns `None` if the line is absent or malformed.
fn parse_meminfo_total_gib(text: &str) -> Option<String> {
    let kib = text
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    let gib = (kib as f64 / (1024.0 * 1024.0)).round() as u64;
    Some(gib.to_string())
}

fn os_string() -> String {
    if cfg!(target_os = "macos") {
        let v = sh("sw_vers", &["-productVersion"]).unwrap_or_else(unknown);
        format!("macOS {v}")
    } else {
        format!(
            "{} ({})",
            std::env::consts::OS,
            sh("uname", &["-r"]).unwrap_or_else(unknown)
        )
    }
}

/// Ask wgpu for the default adapter — the same request the shell's headless
/// device makes, so the recorded adapter is the one the frames rendered on.
fn adapter_string() -> String {
    use vello::wgpu;
    let instance = wgpu::Instance::default();
    match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) {
        Ok(adapter) => {
            let info = adapter.get_info();
            format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type)
        }
        Err(_) => unknown(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed but real-shaped `/proc/meminfo` head. The parse is exercised
    /// from this fixture so the test is host-independent: it verifies the Linux
    /// path on macOS, where the file does not exist.
    const MEMINFO_16G: &str = "\
MemTotal:       16302032 kB
MemFree:         1234567 kB
MemAvailable:    8765432 kB
Buffers:          123456 kB
";

    #[test]
    fn meminfo_total_rounds_to_the_named_capacity() {
        // 16302032 KiB = 15.547 GiB — a 16-GiB box after kernel reservations.
        // Rounding recovers 16; a floor would under-report 15.
        assert_eq!(parse_meminfo_total_gib(MEMINFO_16G).as_deref(), Some("16"));
    }

    #[test]
    fn meminfo_total_reads_the_first_field_only() {
        // Exactly 8 GiB in KiB — the trailing `kB` unit must not be parsed in.
        let text = "MemTotal:        8388608 kB\n";
        assert_eq!(parse_meminfo_total_gib(text).as_deref(), Some("8"));
    }

    #[test]
    fn meminfo_missing_or_malformed_yields_none() {
        assert_eq!(parse_meminfo_total_gib("MemFree: 100 kB\n"), None);
        assert_eq!(parse_meminfo_total_gib("MemTotal:\n"), None);
        assert_eq!(
            parse_meminfo_total_gib("MemTotal:   not-a-number kB\n"),
            None
        );
        assert_eq!(parse_meminfo_total_gib(""), None);
    }

    /// **A dirty tree and a clean one at the same commit do not produce the
    /// same `commit` field.**
    ///
    /// The record this replaced named a commit whose tree had no
    /// `--open-scan-no-materialise` flag, from a run invoked with that flag —
    /// the harness had been run from a working tree carrying the flag
    /// uncommitted. The id was correct about `HEAD` and wrong about what ran,
    /// and nothing in the record could tell the two apart.
    ///
    /// Two real repositories rather than a mocked `git`, because what is being
    /// pinned is what `git status --porcelain` says, and a mock would pin this
    /// test's opinion of that instead.
    #[test]
    fn a_clean_tree_and_a_dirty_one_are_not_recorded_the_same_way() {
        let root = std::env::temp_dir().join(format!("bf-commit-id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch repo");

        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&root)
                .output()
                .expect("git runs");
            assert!(out.status.success(), "git {args:?}: {out:?}");
        };
        git(&["init", "--quiet"]);
        git(&["config", "user.email", "harness@example.invalid"]);
        git(&["config", "user.name", "harness"]);
        std::fs::write(root.join("tracked.txt"), "one\n").expect("write");
        git(&["add", "tracked.txt"]);
        git(&["commit", "--quiet", "-m", "one"]);

        let clean = commit_id_at(&root).expect("a committed tree has an id");
        assert!(
            !clean.ends_with("-dirty"),
            "a tree with nothing modified was recorded as {clean}"
        );

        // An UNTRACKED file is deliberately not a modification: it does not
        // change what was compiled, and treating it as one would mark every
        // run with a scratch file beside it.
        std::fs::write(root.join("scratch.log"), "noise\n").expect("write");
        assert_eq!(
            commit_id_at(&root).as_deref(),
            Some(clean.as_str()),
            "an untracked file changed the recorded commit"
        );

        std::fs::write(root.join("tracked.txt"), "two\n").expect("write");
        let dirty = commit_id_at(&root).expect("a modified tree still has an id");
        assert_eq!(
            dirty,
            format!("{clean}-dirty"),
            "a tracked file was modified and the id did not say so"
        );
        assert_ne!(
            dirty, clean,
            "the same string was recorded for a clean tree and a dirty one"
        );

        // Not a repository at all: the field is lost rather than guessed.
        let bare = std::env::temp_dir().join(format!("bf-commit-id-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&bare);
        std::fs::create_dir_all(&bare).expect("scratch dir");
        assert_eq!(
            commit_id_at(&bare),
            None,
            "a directory with no repository above it reported a commit"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&bare);
    }
}
