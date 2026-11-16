//! Pixel budgets for GPUs whose rasteriser rounds a few pixels differently
//! from the machines that shot the committed baselines.
//!
//! `kittest.toml` allows no pixel past its per-pixel threshold, and its policy
//! allows a per-test override only when it names the jitter and the machine
//! where it was seen. A budget here applies on the named GPU alone, found by
//! the name of the default wgpu adapter, which is the one the headless
//! captures render on. Every other GPU, CI's runners included, keeps zero, so
//! the merge gate stays as strict as it was.

#![allow(dead_code)]

use std::sync::OnceLock;

/// The adapter name an Apple M5 Max (40-core GPU, Metal 4, macOS 27.2)
/// reports. On it, a few baselines render 1 to 6 pixels past the threshold
/// that differ from the committed PNG by at most 7 of 255 in any channel, the
/// same on every run; each test that allows for it says by how much.
pub const APPLE_M5_MAX: &str = "Apple M5 Max";

/// The name of the default wgpu adapter, or an empty string when there is
/// none. Asked once per test binary.
fn adapter_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        use vello::wgpu;
        let instance = wgpu::Instance::default();
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .map(|adapter| adapter.get_info().name)
            .unwrap_or_default()
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
