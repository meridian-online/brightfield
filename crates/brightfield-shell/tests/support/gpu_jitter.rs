//! Pixel budgets for the hosted macOS runner, whose paravirtual GPU rounds a
//! few pixels differently from the machine that draws the committed baselines.
//!
//! The reference machine is an Apple M5 Max: every baseline is drawn on it,
//! and on it every test keeps `kittest.toml`'s floor, no pixel past the
//! per-pixel threshold. The hosted runner that tests each merge to `main`
//! renders through a paravirtual adapter, and on it some baselines come out a
//! few pixels apart by rasteriser rounding. `kittest.toml`'s policy allows a
//! per-test override only when it names the jitter and the machine where it
//! was seen, so each test passes its own count, measured on a hosted run, to
//! [`options`], and that count applies on the hosted runner's adapter alone,
//! found by the name of the default wgpu adapter, which is the one the
//! headless captures render on. Every other GPU, the reference machine
//! included, keeps zero.
//!
//! A test that calls `image_snapshot` or `harness.snapshot` directly takes
//! `kittest.toml`'s floor on every machine. It moves to [`options`] when a
//! hosted run measures jitter on it, and not before.

#![allow(dead_code)]

use std::sync::OnceLock;

use egui_kittest::SnapshotOptions;

/// The adapter name the hosted macOS runner's paravirtual GPU reports.
pub const HOSTED_RUNNER: &str = "Apple Paravirtual device";

/// The name of the default wgpu adapter, or an empty string when there is
/// none. Asked once per test binary, and written to stderr then, past
/// libtest's capture, so a run's log names the GPU each binary that takes a
/// budget rendered on, failing or not.
pub fn adapter_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        use std::io::Write;
        use vello::wgpu;
        let instance = wgpu::Instance::default();
        let name =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .map(|adapter| adapter.get_info().name)
                .unwrap_or_default();
        let _ = writeln!(std::io::stderr(), "wgpu default adapter: {name:?}");
        name
    })
}

/// `pixels` when this process renders on the GPU named `gpu`, and zero on any
/// other: the value for `SnapshotOptions::failed_pixel_count_threshold`.
pub fn budget_on(gpu: &str, pixels: usize) -> usize {
    if adapter_name() == gpu {
        pixels
    } else {
        0
    }
}

/// `base` with a failed-pixel budget of `hosted_pixels` on the hosted runner
/// and zero everywhere else. A test passes the count a hosted run measured for
/// it, and zero when that run matched the baseline exactly.
pub fn options(base: SnapshotOptions, hosted_pixels: usize) -> SnapshotOptions {
    base.failed_pixel_count_threshold(budget_on(HOSTED_RUNNER, hosted_pixels))
}
