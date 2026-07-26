//! Measure where a dot scene stops being drawn *completely*.
//!
//! Vello sizes its flattening and coarse-raster buffers from fixed constants
//! that never scale with the primitive count — `vello_encoding`'s buffer table
//! carries a source comment saying the numbers were hand-picked to fit that
//! project's own test scenes. Nothing re-runs coarse when one of them
//! overflows, and this renderer reads nothing back: it calls `.expect()` on a
//! `Result` that is `Ok` either way. **Above those constants the picture is
//! silently PARTIAL, not absent** — which makes an eyeball comparison against a
//! "complete" reference worthless unless the reference is known to be complete.
//!
//! So this measures it. It renders a dot scatter at a ladder of row counts
//! through the real mark renderer and the real scene builder, reads the GPU's
//! `BumpAllocators` back, and prints each counter beside the capacity it is
//! allocated against. `failed != 0` means the frame you are looking at is
//! missing ink.
//!
//! There is a second, lower ceiling this also finds, and it is not a GPU limit
//! at all: the per-draw `info` stream is carved out of a **fixed 2^18-element**
//! `bin_data` buffer, and the config computes `bin_data.len() - bin_data_start`
//! as a `u32`. One solid-colour draw contributes one word to `bin_data_start`,
//! so a scene of more than 2^18 filled paths makes that subtraction go
//! negative: a debug build panics inside `vello_encoding`, and a release build
//! wraps to ~4×10⁹ and tells the binning shader it has room it does not have.
//! No wgpu limit moves it.
//!
//! Feature-gated and ignored by default: it needs a real GPU adapter *and*
//! vello's `debug_layers` readback, and it is an instrument, not a gate:
//!
//! ```text
//! cargo test -p brightfield-render --features vello-bump-probe \
//!     --test vello_bump_ceiling -- --ignored --nocapture
//! ```
#![cfg(feature = "vello-bump-probe")]

use std::sync::Arc;

use arrow::array::Float64Array;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;

use brightfield_render::channel::{Channel, ChannelMap};
use brightfield_render::layout::ChartLayout;
use brightfield_render::mark::DotRenderer;
use brightfield_render::scene::{build_multi_mark_scene, ChartData};
use brightfield_render::title::ResolvedTitles;
use vello::wgpu;

/// Capacities `vello_encoding::BufferSizes::new` hands the GPU regardless of
/// scene size, in ELEMENTS. Kept here as the yardstick the measured counters
/// are read against; if a vello bump changes them this report goes stale
/// loudly rather than quietly (the printed ratio stops making sense).
const CAP_LINES: u32 = 1 << 21;
const CAP_TILES: u32 = 1 << 21;
const CAP_SEG_COUNTS: u32 = 1 << 21;
const CAP_SEGMENTS: u32 = 1 << 21;
const CAP_PTCL: u32 = 1 << 23;
const CAP_BIN_DATA: u32 = 1 << 18;
const CAP_BLEND_SPILL: u32 = 1 << 20;

/// Plot size and device scale the demo captures use.
const PLOT_W: f64 = 640.0;
const PLOT_H: f64 = 480.0;
const SCALE: f64 = 2.0;

fn scatter_batch(n: usize) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
    // A deterministic space-filling spread — a linear congruential walk, so
    // the dots cover the plot rather than piling on one pixel (a pile would
    // under-report tiles and flatter the ceiling).
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut xs = Vec::with_capacity(n);
    let mut ys = Vec::with_capacity(n);
    for _ in 0..n {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        xs.push(((state >> 33) & 0xFFFF) as f64);
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ys.push(((state >> 33) & 0xFFFF) as f64);
    }
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(xs)),
            Arc::new(Float64Array::from(ys)),
        ],
    )
    .expect("scatter batch")
}

#[test]
#[ignore = "needs a real GPU adapter; an instrument, not a gate"]
fn report_bump_headroom_across_dot_counts() {
    let instance = wgpu::Instance::default();
    let Ok(adapter) =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    else {
        eprintln!("no GPU adapter — nothing measured");
        return;
    };
    let adapter_limits = adapter.limits();
    eprintln!(
        "adapter  max_storage_buffer_binding_size = {:>10} bytes ({:.0} MiB)",
        adapter_limits.max_storage_buffer_binding_size,
        adapter_limits.max_storage_buffer_binding_size as f64 / (1024.0 * 1024.0)
    );
    eprintln!(
        "defaults max_storage_buffer_binding_size = {:>10} bytes ({:.0} MiB)",
        wgpu::Limits::default().max_storage_buffer_binding_size,
        wgpu::Limits::default().max_storage_buffer_binding_size as f64 / (1024.0 * 1024.0)
    );

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("bump-probe"),
        required_features: wgpu::Features::empty(),
        required_limits: adapter_limits,
        memory_hints: wgpu::MemoryHints::default(),
        ..Default::default()
    }))
    .expect("device");

    let mut renderer = vello::Renderer::new(
        &device,
        vello::RendererOptions {
            use_cpu: false,
            antialiasing_support: vello::AaSupport::all(),
            num_init_threads: None,
            pipeline_cache: None,
        },
    )
    .expect("vello renderer");

    let dev_w = (PLOT_W * SCALE).round() as u32;
    let dev_h = (PLOT_H * SCALE).round() as u32;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bump-probe-target"),
        size: wgpu::Extent3d {
            width: dev_w,
            height: dev_h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    for rows in [
        1_000_usize,
        10_000,
        25_000,
        50_000,
        75_000,
        100_000,
        104_000,
        106_000,
        110_000,
        120_000,
        130_000,
        132_000,
        150_000,
        262_000,
        300_000,
    ] {
        let batch = scatter_batch(rows);
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        let dot = DotRenderer;
        let data = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &dot,
            layout: ChartLayout::new(PLOT_W, PLOT_H),
            view_extent: None,
            highlight: None,
        };
        let (plot, _scales) = build_multi_mark_scene(&[&data], false, &ResolvedTitles::default());
        let mut scaled = vello::Scene::new();
        scaled.append(&plot, Some(kurbo::Affine::scale(SCALE)));

        let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            #[allow(deprecated)]
            vello::util::block_on_wgpu(
                &device,
                renderer.render_to_texture_async(
                    &device,
                    &queue,
                    &scaled,
                    &view,
                    &vello::RenderParams {
                        base_color: vello::peniko::Color::WHITE,
                        width: dev_w,
                        height: dev_h,
                        antialiasing_method: vello::AaConfig::Msaa16,
                    },
                    vello::low_level::DebugLayers::none(),
                ),
            )
        }));

        match rendered {
            Err(_) => eprintln!(
                "{rows:>9} dots  ENCODE PANIC — bin_data ({CAP_BIN_DATA}) minus the per-draw info \
                 stream underflowed; a release build wraps this instead of stopping"
            ),
            Ok(Err(e)) => eprintln!("{rows:>9} dots  render error: {e:?}"),
            Ok(Ok(None)) => eprintln!(
                "{rows:>9} dots  no bump readback — build with --features vello-bump-probe"
            ),
            Ok(Ok(Some(b))) => {
                let pct = |used: u32, cap: u32| f64::from(used) * 100.0 / f64::from(cap);
                eprintln!(
                    "{rows:>9} dots  failed={:#010x}  \
                     lines {:>8} ({:5.1}%)  \
                     tiles {:>8} ({:5.1}%)  \
                     seg_counts {:>8} ({:5.1}%)  \
                     segments {:>8} ({:5.1}%)  \
                     ptcl {:>8} ({:5.1}%)  \
                     binning {:>8} ({:5.1}%)  \
                     blend {:>8} ({:5.1}%)",
                    b.failed,
                    b.lines,
                    pct(b.lines, CAP_LINES),
                    b.tile,
                    pct(b.tile, CAP_TILES),
                    b.seg_counts,
                    pct(b.seg_counts, CAP_SEG_COUNTS),
                    b.segments,
                    pct(b.segments, CAP_SEGMENTS),
                    b.ptcl,
                    pct(b.ptcl, CAP_PTCL),
                    b.binning,
                    pct(b.binning, CAP_BIN_DATA),
                    b.blend,
                    pct(b.blend, CAP_BLEND_SPILL),
                );
            }
        }
    }
}
