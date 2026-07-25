//! Compose-time client memory — what the process is holding while the scene
//! is built.
//!
//! `compose_from_results` assembles EVERY materialised Arrow chunk of EVERY
//! mark and holds them all while it lays out and rasters the scene. At ten
//! million row-level rows that is the whole result set resident in the client,
//! and until this module existed nothing in the record named the number.
//!
//! Two quantities are recorded, because neither alone is honest:
//!
//! - **Arrow bytes held** — exact and deterministic. Summed from the batches
//!   themselves (`RecordBatch::get_array_memory_size`), so it is the same on
//!   any host and independent of the allocator. This is the data the compose
//!   holds; it is not the process's footprint.
//! - **Resident set size across the compose window** — the process's actual
//!   footprint, sampled from the OS. It sees DuckDB's C++ allocations (which
//!   own the imported Arrow buffers) and the allocator's retained pages, which
//!   the Arrow figure cannot. It is a *single sample of a whole-process
//!   quantity* and does not reproduce on its own: the harness opens two
//!   windows per scenario over the same compose work (pre-aggregation off and
//!   on), and in the committed record their peaks differ by as much as 2.4x.
//!   Report the pair, or report the peak with that spread stated.
//!
//! The pre-compose reading is **not** a floor that only rises. The OS reclaims
//! pages between windows — in the committed record `rss_before` falls sharply
//! between adjacent scenarios as often as it climbs — so `rss_growth_mib` is
//! neither the compose's cost nor a lower bound on it, and any reading built
//! on "RSS is cumulative within a run" is unsound here.
//!
//! What the window DOES depend on is the caller: resident size counts
//! everything the process holds, so every earlier phase's result set must be
//! dropped before [`RssSampler::start`] is called. The scenario module does
//! that by moving the coordinator phase's batches into a consuming shape pass.
//!
//! Resident size is read from the OS the same way the machine profile reads
//! physical memory: a best-effort platform probe with the parse split from the
//! I/O so the conversion is covered by a fixture test on any host. Linux reads
//! `/proc/self/status`; macOS shells out to `ps`, which costs ~2.5 ms per
//! sample and therefore sets the sampler's resolution. A host that is neither
//! records `null` and names itself in `sampler`, rather than reporting a
//! number it did not measure.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use serde::Serialize;

/// Bytes in a mebibyte.
const MIB: f64 = 1024.0 * 1024.0;

/// How often the sampler polls resident size inside a compose window. On
/// macOS one sample forks `ps` (~2.5 ms), so a shorter interval would spend
/// more time sampling than the thing being sampled; on Linux the read is a
/// file parse and this is comfortably slack.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(5);

/// What the compose held, and what the process was holding while it did.
///
/// Every field is stated in mebibytes so the record reads directly. The
/// resident fields are `null` on a host with no probe — a gap the reader can
/// see, never a zero that reads as a measurement.
#[derive(Debug, Clone, Serialize)]
pub struct ComposeMemory {
    /// How resident size was read on the measuring host (or why it was not).
    pub sampler: &'static str,
    /// Resident set size immediately before the compose, MiB. Not a monotone
    /// baseline: the OS reclaims pages between windows, so this falls between
    /// adjacent scenarios as often as it rises.
    pub rss_before_mib: Option<f64>,
    /// Highest resident set size observed across the compose window, MiB. One
    /// sample of a whole-process quantity — pair it with the other
    /// configuration's window before quoting it.
    pub rss_peak_mib: Option<f64>,
    /// `rss_peak_mib - rss_before_mib`. NOT the compose's cost in either
    /// direction: the floor moves both ways between windows, so a small growth
    /// may mean a high floor and a large one may mean a reclaimed floor.
    pub rss_growth_mib: Option<f64>,
    /// Resident-size samples taken inside the compose window, the two
    /// end-point reads included. A small count means coarse resolution, not a
    /// small peak.
    pub rss_samples: u64,
    /// Exact Arrow bytes across every materialised chunk of every mark — what
    /// the compose is handed and holds.
    pub arrow_chunks_mib: f64,
    /// Exact Arrow bytes across the assembled per-mark drawable batches. A
    /// mark that materialised as ONE chunk assembles by pass-through, so its
    /// bytes are the SAME allocation counted again here, not a second copy;
    /// only a multi-chunk mark's assembly is a genuine additional buffer.
    pub arrow_assembled_mib: f64,
}

/// Current resident set size of this process in bytes, or `None` where no
/// probe exists.
#[must_use]
pub fn rss_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/self/status").ok()?;
        parse_status_vmrss_kib(&text).map(|kib| kib * 1024)
    }
    #[cfg(target_os = "macos")]
    {
        let pid = std::process::id().to_string();
        let out = crate::machine::sh("ps", &["-o", "rss=", "-p", &pid])?;
        parse_ps_rss_kib(&out).map(|kib| kib * 1024)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// The name of the probe [`rss_bytes`] uses on this host — recorded beside the
/// numbers so a reader knows what produced them.
#[must_use]
pub const fn sampler_name() -> &'static str {
    if cfg!(target_os = "linux") {
        "/proc/self/status VmRSS"
    } else if cfg!(target_os = "macos") {
        "ps -o rss (~5ms resolution)"
    } else {
        "unavailable on this platform"
    }
}

/// Kibibytes from the `VmRSS:` line of a `/proc/self/status` dump. Linux
/// labels the unit `kB` and means KiB. `None` if the line is absent or
/// malformed — the same best-effort contract the machine profile uses.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_status_vmrss_kib(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()
}

/// Kibibytes from `ps -o rss=` output — one whitespace-padded integer, in KiB
/// on both macOS and Linux. `None` on anything else.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_ps_rss_kib(text: &str) -> Option<u64> {
    text.split_whitespace().next()?.parse::<u64>().ok()
}

/// A background poll of this process's resident size, running for the length
/// of one compose.
///
/// Deliberately a separate thread rather than an instrumented allocator: the
/// Arrow chunks are imported from DuckDB over the C data interface, so their
/// bytes were allocated by DuckDB's C++ allocator and a Rust global-allocator
/// counter would not see them at all. Resident size sees everything the
/// process actually holds.
pub struct RssSampler {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    samples: Arc<AtomicU64>,
    handle: Option<JoinHandle<()>>,
    before: Option<u64>,
}

impl RssSampler {
    /// Read the pre-compose resident size and start polling.
    #[must_use]
    pub fn start() -> Self {
        let before = rss_bytes();
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(before.unwrap_or(0)));
        let samples = Arc::new(AtomicU64::new(u64::from(before.is_some())));
        let handle = if before.is_some() {
            let (stop_t, peak_t, samples_t) = (stop.clone(), peak.clone(), samples.clone());
            Some(std::thread::spawn(move || {
                while !stop_t.load(Ordering::Relaxed) {
                    if let Some(rss) = rss_bytes() {
                        peak_t.fetch_max(rss, Ordering::Relaxed);
                        samples_t.fetch_add(1, Ordering::Relaxed);
                    }
                    std::thread::sleep(SAMPLE_INTERVAL);
                }
            }))
        } else {
            // No probe on this host: nothing to poll, and a thread that could
            // only record zeroes would make the gap look like a measurement.
            None
        };
        Self {
            stop,
            peak,
            samples,
            handle,
            before,
        }
    }

    /// Stop polling, take one final reading, and report the window.
    ///
    /// `arrow_chunk_bytes` / `arrow_assembled_bytes` are the exact Arrow
    /// figures the caller measured from the batches themselves; they are
    /// carried here so the whole memory picture lands in one record field.
    #[must_use]
    pub fn finish(mut self, arrow_chunk_bytes: u64, arrow_assembled_bytes: u64) -> ComposeMemory {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(rss) = rss_bytes() {
            self.peak.fetch_max(rss, Ordering::Relaxed);
            self.samples.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let mib = |b: u64| (b as f64 / MIB * 100.0).round() / 100.0;
        let before = self.before;
        let peak = before.map(|_| self.peak.load(Ordering::Relaxed));
        ComposeMemory {
            sampler: sampler_name(),
            rss_before_mib: before.map(mib),
            rss_peak_mib: peak.map(mib),
            rss_growth_mib: match (before, peak) {
                (Some(b), Some(p)) => Some(mib(p.saturating_sub(b))),
                _ => None,
            },
            rss_samples: self.samples.load(Ordering::Relaxed),
            arrow_chunks_mib: mib(arrow_chunk_bytes),
            arrow_assembled_mib: mib(arrow_assembled_bytes),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed but real-shaped `/proc/self/status` head. Exercising the
    /// parse from a fixture keeps the Linux path covered on macOS, where the
    /// file does not exist — the same trick the machine profile's meminfo
    /// parse uses.
    const STATUS: &str = "\
Name:\tbrightfield-bench
State:\tR (running)
VmPeak:\t 4210984 kB
VmSize:\t 4110984 kB
VmRSS:\t  1048576 kB
RssAnon:\t 1040000 kB
";

    #[test]
    fn vmrss_reads_the_kibibyte_field_only() {
        // 1048576 KiB = exactly 1 GiB; the trailing `kB` unit must not parse in.
        assert_eq!(parse_status_vmrss_kib(STATUS), Some(1_048_576));
    }

    #[test]
    fn vmrss_ignores_the_other_vm_lines() {
        // VmPeak and VmSize both precede VmRSS and both start with `Vm` — a
        // prefix match on the wrong line would report the peak as the current.
        assert_ne!(parse_status_vmrss_kib(STATUS), Some(4_210_984));
    }

    #[test]
    fn vmrss_missing_or_malformed_yields_none() {
        assert_eq!(parse_status_vmrss_kib("VmSize: 100 kB\n"), None);
        assert_eq!(parse_status_vmrss_kib("VmRSS:\n"), None);
        assert_eq!(parse_status_vmrss_kib("VmRSS:  not-a-number kB\n"), None);
        assert_eq!(parse_status_vmrss_kib(""), None);
    }

    #[test]
    fn ps_rss_parses_the_padded_integer() {
        // `ps -o rss=` emits a right-aligned, header-less integer in KiB.
        assert_eq!(parse_ps_rss_kib("  262144\n"), Some(262_144));
        assert_eq!(parse_ps_rss_kib("262144"), Some(262_144));
        assert_eq!(parse_ps_rss_kib(""), None);
        assert_eq!(parse_ps_rss_kib("  RSS\n"), None);
    }

    #[test]
    fn the_sampler_names_the_probe_it_used() {
        // The recorded name must describe THIS host's probe, so a reader of a
        // committed record can tell how the number was obtained.
        let name = sampler_name();
        if cfg!(any(target_os = "linux", target_os = "macos")) {
            assert_ne!(name, "unavailable on this platform");
            assert!(
                rss_bytes().is_some_and(|b| b > 0),
                "a host that names a probe must be able to read a non-zero RSS"
            );
        }
    }

    #[test]
    fn a_window_reports_a_peak_at_least_its_starting_point() {
        let s = RssSampler::start();
        // Hold a real allocation across the window so the peak has something
        // to see; keep it alive past `finish` so it cannot be freed early.
        let ballast: Vec<u8> = vec![7u8; 64 * 1024 * 1024];
        std::thread::sleep(Duration::from_millis(30));
        let m = s.finish(1024, 512);
        assert_eq!(ballast[0], 7, "the ballast must outlive the window");
        assert!(m.rss_samples >= 1, "at least the end-point reads land");
        assert!((m.arrow_chunks_mib - 0.0).abs() < 1.0);
        if let (Some(before), Some(peak)) = (m.rss_before_mib, m.rss_peak_mib) {
            assert!(
                peak >= before,
                "a peak below its own starting sample is impossible: {peak} < {before}"
            );
        }
    }
}
