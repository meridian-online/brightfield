//! The blank-frame detector, driven through a real GPU.
//!
//! `frame_ink.rs`'s counting is unit-tested in the crate against buffers a test
//! wrote by hand. That proves the arithmetic and nothing about the failure the
//! detector exists for, which only a GPU produces: vello's flattening and
//! coarse-raster buffers are fixed sizes chosen inside `vello_encoding`, and a
//! scene that overflows one of them rasters to an empty target while the render
//! returns `Ok`. Nothing in the pipeline raises an error, so an empty frame and
//! a fast frame are the same event to any caller that does not look.
//!
//! So these render. Three scenes go through
//! [`VelloRenderer`](brightfield_render::vello_renderer::VelloRenderer) — a
//! scatter at the drawn-primitive count the performance harness lets through,
//! one past the measured overflow onset, and one with nothing in it — and the
//! detector's verdict is asserted against each. The two scatter counts bracket
//! the onset for this fixture, which is what makes the pair a gate rather than
//! two samples: a vello bump that moved the ceiling under the harness's cap
//! would redden the first of them.
//!
//! Not `#[ignore]`d and behind no feature. It needs a GPU adapter, as the
//! shell's canvas and snapshot suites already do on the same runner.

use std::sync::Arc;

use arrow::array::Float64Array;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;

use brightfield_render::channel::{Channel, ChannelMap};
use brightfield_render::frame_ink::FrameInk;
use brightfield_render::layout::ChartLayout;
use brightfield_render::mark::DotRenderer;
use brightfield_render::scene::{build_multi_mark_scene, ChartData};
use brightfield_render::title::ResolvedTitles;
use brightfield_render::vello_renderer::VelloRenderer;

/// Plot size and device scale the performance harness's frame suites use.
const PLOT_W: f64 = 640.0;
const PLOT_H: f64 = 480.0;
const SCALE: f64 = 2.0;

/// The drawn-primitive count the performance harness admits for a frame suite.
/// A one-scatter scene of this many dots is what its two closest-to-the-line
/// cells submit, so this is the count whose inking the cap's margin rests on.
const AT_THE_HARNESS_CAP: usize = 100_000;

/// A count past the overflow onset measured for this fixture, and well under
/// the exact `bin_data` ceiling (2^18 filled paths) where the encode panics
/// instead of drawing nothing.
const PAST_THE_ONSET: usize = 150_000;

/// The base the offscreen capture path clears to — `brightfield-shot
/// --vello-only` renders on it, and it is the base the recorded blank frames
/// were read against.
const BASE: vello::peniko::Color = vello::peniko::Color::TRANSPARENT;

/// A deterministic space-filling scatter — a linear congruential walk, so the
/// dots cover the plot instead of piling on one pixel. A pile would under-report
/// tiles and flatter the ceiling.
fn scatter_batch(n: usize) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
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

/// One dot scatter of `rows` rows through the real mark renderer and the real
/// scene builder, scaled onto the device raster as the capture paths do.
fn scatter_scene(rows: usize) -> vello::Scene {
    let batch = scatter_batch(rows);
    let mut channels = ChannelMap::new();
    channels.insert(Channel::X, "x".to_string());
    channels.insert(Channel::Y, "y".to_string());
    let dot = DotRenderer;
    let data = ChartData {
        batch: &batch,
        channel_map: &channels,
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

/// Render `scene` and ask the renderer what came out.
fn ink_of(scene: &vello::Scene) -> FrameInk {
    let renderer = VelloRenderer::new();
    let (w, h) = device_size();
    let ink = renderer
        .lock()
        .expect("renderer poisoned")
        .frame_ink(scene, w, h, BASE);
    assert_eq!(
        ink.total_pixels(),
        u64::from(w) * u64::from(h),
        "the detector measured the whole target"
    );
    ink
}

/// The margin the harness's cap rests on, measured rather than assumed. If a
/// vello bump lowers the overflow onset under this count, the cap stops being
/// a safe policy line and this is where that shows up.
#[test]
fn a_scatter_at_the_harness_cap_reads_back_inked() {
    let ink = ink_of(&scatter_scene(AT_THE_HARNESS_CAP));
    assert!(
        ink.drew_ink(),
        "a {AT_THE_HARNESS_CAP}-dot scatter must reach the target: {ink:?}"
    );
    assert!(
        !ink.is_uniform(),
        "an inked frame carries more than one pixel value: {ink:?}"
    );
}

/// The failure the detector exists for: the render returns success, a frame is
/// produced, and there is nothing in it.
#[test]
fn a_scatter_past_the_overflow_onset_reads_back_blank() {
    let ink = ink_of(&scatter_scene(PAST_THE_ONSET));
    assert!(
        !ink.drew_ink(),
        "a {PAST_THE_ONSET}-dot scatter overflows and draws nothing, and the \
         detector must say so: {ink:?}"
    );
    assert_eq!(ink.inked_pixels(), 0);
    assert!(
        ink.is_uniform(),
        "a target nothing wrote to holds one value edge to edge: {ink:?}"
    );
}

/// The negative control for the pair above: a scene with nothing in it is
/// blank for a reason that has nothing to do with an overflow, so a detector
/// that answered "inked" for it would be reading the wrong thing.
#[test]
fn a_scene_with_nothing_in_it_reads_back_blank() {
    let ink = ink_of(&vello::Scene::new());
    assert!(!ink.drew_ink(), "{ink:?}");
    assert!(ink.is_uniform(), "{ink:?}");
}
