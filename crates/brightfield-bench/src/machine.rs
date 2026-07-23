//! The fixed machine profile recorded beside every result. A number without
//! its machine is not a result.

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
    /// `git rev-parse --short HEAD` of the tree the harness was built from.
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
            memory_gib: sh("sysctl", &["-n", "hw.memsize"])
                .and_then(|b| b.trim().parse::<u64>().ok())
                .map(|b| format!("{}", b / (1024 * 1024 * 1024)))
                .unwrap_or_else(unknown),
            os: os_string(),
            gpu_adapter: adapter_string(),
            rustc: sh("rustc", &["--version"]).unwrap_or_else(unknown),
            build_profile: profile,
            commit: sh("git", &["rev-parse", "--short", "HEAD"]).unwrap_or_else(unknown),
            captured_at: chrono::Local::now().to_rfc3339(),
            duckdb: duckdb_version,
        }
    }
}

fn unknown() -> String {
    "unknown".to_string()
}

/// Run a command and return its trimmed stdout, `None` on any failure.
fn sh(cmd: &str, args: &[&str]) -> Option<String> {
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
