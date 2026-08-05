//! Measure where a dot scene stops being drawn.
//!
//! Vello sizes its flattening and coarse-raster buffers from fixed constants
//! that never scale with the primitive count — `vello_encoding`'s buffer table
//! carries a source comment saying the numbers were hand-picked to fit that
//! project's own test scenes. Nothing re-runs coarse when one of them
//! overflows, and the render call reports success either way: it returns a
//! `Result` that is `Ok` whether or not a picture came out.
//!
//! **Above those constants the frame comes back BLANK, with no error.** Not
//! thinned, not partial: once `seg_counts` overflows, `segments` and `ptcl`
//! both come back 0 and coarse emits nothing, and the production render agrees
//! — a `brightfield-shot --vello-only` capture past the boundary exits 0 and
//! writes a PNG whose every pixel is `rgba(0, 0, 0, 0)`. So an eyeball
//! comparison against a "complete" reference is worthless unless the reference
//! is known to have drawn at all.
//!
//! So this measures it. It renders a dot scatter at a ladder of row counts
//! through the real mark renderer and the real scene builder, reads the GPU's
//! `BumpAllocators` back, and prints each counter beside the capacity it is
//! allocated against. `failed != 0` means the frame you are looking at is empty.
//!
//! There is a second, much higher ceiling this also finds, and it is not a GPU
//! limit at all: the per-draw `info` stream is carved out of a **fixed
//! 2^18-element** `bin_data` buffer, and the config computes
//! `bin_data.len() - bin_data_start` as a `u32`. One solid-colour draw
//! contributes one word to `bin_data_start`, so a scene of more than 2^18
//! filled paths makes that subtraction go negative: a debug build panics inside
//! `vello_encoding` (`config.rs:185`), and a release build wraps to ~4×10⁹ and
//! tells the binning shader it has room it does not have. No wgpu limit moves
//! it. Measured through the production path, it first fires at 262 102 dots in
//! a plot carrying 42 paths of furniture — 2^18 = 262 144, to the row.
//!
//! # This instrument can panic for reasons that are only its own
//!
//! `vello::DebugDownloads::map` is `#[cfg(feature = "debug_layers")]` and slices
//! the lines buffer to `bump.lines * size_of::<LineSoup>()` without checking
//! that against the buffer's own length. The buffer is 2^21 × 24 = 50 331 648
//! bytes, so once `lines` passes 2^21 the slice runs off the end and wgpu
//! panics — and the message says so verbatim: *"slice offset 0 size 50 739 072
//! is out of range for buffer of size 50 331 648"*. That is this instrument
//! failing, inside a code path that exists ONLY because the probe turned
//! `debug_layers` on. It is not a renderer ceiling, and the production path
//! sails past every count that produces it.
//!
//! A panic seen here is therefore not evidence about the renderer until it has
//! been reproduced without the feature, which is why the panic arm below prints
//! the message and refuses to name a cause. Reproduce with:
//!
//! ```text
//! cargo run -p brightfield-shell --bin brightfield-shot -- \
//!     --spec <a range(N) scatter> --out /tmp/n.png --vello-only
//! ```
//!
//! # The ladder is an instrument; the agreement below it is a gate
//!
//! `report_bump_headroom_across_dot_counts` prints and asserts nothing — a
//! ladder of counters is read, not passed or failed, and it is `#[ignore]`d so
//! a run has to ask for it:
//!
//! ```text
//! cargo test -p brightfield-render --features vello-bump-probe \
//!     --test vello_bump_ceiling -- --ignored --nocapture
//! ```
//!
//! `the_failed_bit_and_the_pixels_agree_about_the_same_frame` is not ignored
//! and runs whenever this file compiles, which needs the same feature. It holds
//! vello's own account of the overflow against
//! [`FrameInk`](brightfield_render::frame_ink::FrameInk) — the reading
//! `crates/brightfield-render/tests/frame_ink.rs` gates on in a normal build,
//! where these counters come back `None`. Two readings of one frame from
//! opposite ends of the pipeline: if they ever disagree, the one shipped to
//! callers is the pixel one, and that is worth finding out here.
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
use brightfield_render::vello_renderer::VelloRenderer;
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

/// One dot scatter of `rows` rows, scaled onto the device raster — the fixture
/// both the ladder and the agreement gate measure.
fn scatter_scene(rows: usize) -> vello::Scene {
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
        sample: None,
        beyond_frame: false,
    };
    let (plot, _scales) = build_multi_mark_scene(&[&data], false, &ResolvedTitles::default());
    let mut scaled = vello::Scene::new();
    scaled.append(&plot, Some(kurbo::Affine::scale(SCALE)));
    scaled
}

fn device_size() -> (u32, u32) {
    (
        (PLOT_W * SCALE).round() as u32,
        (PLOT_H * SCALE).round() as u32,
    )
}

/// vello's `failed` word for one render of `scene`, on a device of its own.
///
/// The download happens because this crate's `vello-bump-probe` feature turns
/// vello's `debug_layers` on; without it the async path sets `robust = false`
/// and the counters come back `None`, which is why the harness cannot read this
/// signal and reads pixels instead.
fn bump_failed_word(scene: &vello::Scene) -> u32 {
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("a GPU adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("bump-agreement"),
        required_features: wgpu::Features::empty(),
        required_limits: adapter.limits(),
        memory_hints: wgpu::MemoryHints::default(),
        ..Default::default()
    }))
    .expect("a GPU device");
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
    let (dev_w, dev_h) = device_size();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bump-agreement-target"),
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
    #[allow(deprecated)]
    let bump = vello::util::block_on_wgpu(
        &device,
        renderer.render_to_texture_async(
            &device,
            &queue,
            scene,
            &view,
            &vello::RenderParams {
                base_color: vello::peniko::Color::TRANSPARENT,
                width: dev_w,
                height: dev_h,
                antialiasing_method: vello::AaConfig::Msaa16,
            },
            vello::low_level::DebugLayers::none(),
        ),
    )
    .expect("render")
    .expect("the probe feature turns vello's bump readback on");
    bump.failed
}

/// One dot count vello draws whole, and one it drops work on — measured on this
/// fixture, and both kept under the count where this instrument's own
/// `DebugDownloads::map` slices past the lines buffer and panics.
const DRAWN: usize = 104_000;
const DROPPED: usize = 110_000;

/// The renderer's account of the frame, and the frame.
///
/// `failed` is what vello says it did; [`FrameInk`] is what came out. The
/// harness consumes the second — it is available with no feature on, where
/// these counters are `None` — so the pair is worth holding together at least
/// where both can be read. A disagreement would mean the shipped detector is
/// answering a different question from the renderer's own flag.
#[test]
fn the_failed_bit_and_the_pixels_agree_about_the_same_frame() {
    let (w, h) = device_size();
    let base = vello::peniko::Color::TRANSPARENT;
    let renderer = VelloRenderer::new();

    let drawn = scatter_scene(DRAWN);
    let drawn_failed = bump_failed_word(&drawn);
    let drawn_ink = renderer
        .lock()
        .expect("renderer poisoned")
        .frame_ink(&drawn, w, h, base);
    assert_eq!(
        drawn_failed, 0,
        "{DRAWN} dots must leave vello's failure word clear"
    );
    assert!(
        drawn_ink.drew_ink(),
        "and the frame it produced must carry ink: {drawn_ink:?}"
    );

    let dropped = scatter_scene(DROPPED);
    let dropped_failed = bump_failed_word(&dropped);
    let dropped_ink = renderer
        .lock()
        .expect("renderer poisoned")
        .frame_ink(&dropped, w, h, base);
    assert_ne!(
        dropped_failed, 0,
        "{DROPPED} dots must set a bit in vello's failure word"
    );
    assert!(
        !dropped_ink.drew_ink(),
        "and the frame it produced must be empty, which is what makes the \
         pixel reading a usable stand-in for a counter no normal build \
         downloads: {dropped_ink:?}"
    );
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
        let scaled = scatter_scene(rows);

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
            // The panic's own message, and NO interpretation. This arm used to
            // print a hardcoded `bin_data` explanation for any panic at all,
            // and the explanation was wrong: what it kept catching was
            // `vello::DebugDownloads::map`, a `debug_layers`-only readback
            // slicing the lines buffer past its own length — a path that exists
            // only because this probe enabled the feature. The probe was
            // measuring its own instrument, and the resulting row went into the
            // frame cap's record as a renderer property. A measurement tool
            // must never name a cause it did not observe.
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .map(|s| (*s).to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "<non-string panic payload>".to_string());
                eprintln!(
                    "{rows:>9} dots  PANIC — cause NOT diagnosed here. Re-run this count \
                     through brightfield-shot --vello-only (no debug_layers) before treating \
                     it as a renderer ceiling. Message: {msg}"
                );
            }
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
