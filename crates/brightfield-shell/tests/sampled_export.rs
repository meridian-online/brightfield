//! The two halves of the sampling notice that can be machine-checked: that it
//! survives a chart-only export, and that it survives a brush.
//!
//! `capture_vello_only` rasterises the composed Vello scene and never
//! constructs an egui context, so everything the shell draws — the top bar, the
//! margin legend, any banner anyone might be tempted to add — is absent from
//! that PNG **by construction**. That is the mechanical meaning of "survives
//! being cropped out of a screenshot", and it is exactly why the notice is
//! drawn into the plot's own scene rather than into chrome.
//!
//! The second half goes through [`LiveDashboard::present`], which is the path
//! the live window repaints on — not the one-shot `compose_spec_sampled` call.
//! Those are two different functions gathering the facts, and only one of them
//! runs after a gesture; a suite that exercised only the one-shot call would be
//! green with the live path erasing the notice on the first drag, which is
//! precisely the invisible degradation this whole device exists to prevent.
//!
//! What is left for a human eye is the rest — whether a sampled render reads as
//! sampled without reading the words. Nothing here claims to test that.

use std::path::PathBuf;

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_render::sample_notice::NOTICE_BAND;
use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::pipeline::{compose_spec_sampled, live_spec_sampled};
use brightfield_spec::analysis::ComponentPath;
use brightfield_sql::ir::SampleRate;

/// Small enough to render fast, dense enough to be a real scatter.
const SPEC: &str = "data:
  points:
    query: |
      SELECT (i * 7919 % 1009) / 10.0 AS a, (i * 104729 % 1013) / 10.0 AS b
      FROM range(4096) AS t(i)
plot:
  - mark: dot
    data: { from: points }
    x: a
    y: b
width: 400
height: 300
";

/// The same scatter, brushable — the shape the live window actually runs, and
/// the only shape in which a re-present after a gesture can be observed.
const BRUSHABLE_SPEC: &str = "params:
  brush:
    select: intersect
data:
  points:
    query: |
      SELECT (i * 7919 % 1009) / 10.0 AS a, (i * 104729 % 1013) / 10.0 AS b
      FROM range(4096) AS t(i)
plot:
  - mark: dot
    data: { from: points, filterBy: $brush }
    x: a
    y: b
width: 400
height: 300
";

const W: u32 = 400;
const H: u32 = 300;

fn write_spec(dir: &std::path::Path) -> PathBuf {
    let p = dir.join("sampled-export.yaml");
    std::fs::write(&p, SPEC).expect("write spec");
    p
}

/// Count pixels that are not the near-white page/surface, in the bottom
/// `NOTICE_BAND` logical rows — the band the notice reserves.
fn ink_in_band(png: &std::path::Path) -> u64 {
    let img = image::open(png).expect("open png").to_rgba8();
    let band_top = H - NOTICE_BAND.ceil() as u32;
    let mut ink = 0u64;
    for y in band_top..H {
        for x in 0..W {
            let p = img.get_pixel(x, y).0;
            // The page and surface tokens are both above 0xf8 on every
            // channel; anything appreciably darker is ink someone drew.
            if p[0] < 0xf0 || p[1] < 0xf0 || p[2] < 0xf0 {
                ink += 1;
            }
        }
    }
    ink
}

#[test]
fn the_sampling_notice_is_in_the_chart_only_export() {
    let dir = std::env::temp_dir().join(format!("bf-sampled-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let spec = write_spec(&dir);

    let complete = compose_spec_sampled(spec.to_str().unwrap(), None).expect("compose complete");
    let (cw, ch) = (complete.width, complete.height);
    let rate = SampleRate::from_modulus(8).expect("power of two");
    let sampled =
        compose_spec_sampled(spec.to_str().unwrap(), Some(rate)).expect("compose sampled");

    // The fact reached the plot handle, and only on the sampled composition.
    assert!(
        complete.plots.iter().all(|p| p.sample.is_none()),
        "an unsampled composition must carry no sampling fact"
    );
    let fact = sampled.plots[0]
        .sample
        .expect("the sampled composition's plot must carry its fact");
    let sampled_size = (sampled.width, sampled.height);
    assert_eq!(fact.of, 4096, "`of` is the unsampled count, measured");
    assert!(
        fact.drawn > 0 && fact.drawn < fact.of,
        "a 1-in-8 sample of 4096 rows drew {} — expected some but not all",
        fact.drawn
    );

    // Same canvas either way: the band is taken out of the plot's margin, not
    // added to the image, so the two PNGs are directly comparable.
    assert_eq!((cw, ch), (W, H));
    assert_eq!(sampled_size, (W, H));

    let complete_png = dir.join("complete.png");
    let sampled_png = dir.join("sampled.png");
    capture_vello_only(complete, 1.0, &complete_png).expect("capture complete");
    capture_vello_only(sampled, 1.0, &sampled_png).expect("capture sampled");

    let complete_ink = ink_in_band(&complete_png);
    let sampled_ink = ink_in_band(&sampled_png);

    assert!(
        sampled_ink > 400,
        "the sampled export's bottom band held {sampled_ink} inked pixels — the hatch and \
         label should be plainly there. If this is near zero the notice did not survive the \
         chart-only export, which is the whole reason it is not a banner."
    );
    assert!(
        sampled_ink > complete_ink * 3,
        "the band is supposed to be the DIFFERENCE between the two exports: sampled held \
         {sampled_ink} inked pixels there, complete held {complete_ink}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **A brush must not erase the notice.** Driven through
/// [`LiveDashboard::present`] — the function the live window repaints on —
/// rather than the one-shot compose call the test above uses.
///
/// The distinction is the whole point. `compose_spec_sampled` gathers the
/// unsampled facts once and drops the session; `present` gathers them again on
/// every repaint, and it is the only one of the two that runs after a gesture.
/// Emptying `present`'s facts vector leaves every crate in this workspace green
/// without this test, and leaves a sampled plot drawing as if it were complete
/// the moment anyone touches it.
///
/// Both halves are asserted: the fact on the plot handle, and the ink in the
/// exported PNG's notice band — so a repair that kept the struct field alive
/// while dropping the drawn band would still be caught.
#[test]
fn a_brush_does_not_erase_the_sampling_notice() {
    let dir = std::env::temp_dir().join(format!("bf-sampled-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let spec = dir.join("sampled-live.yaml");
    std::fs::write(&spec, BRUSHABLE_SPEC).expect("write spec");

    let rate = SampleRate::from_modulus(32).expect("power of two");
    let (mut dash, first) =
        live_spec_sampled(spec.to_str().unwrap(), Some(rate)).expect("live sampled");

    let before = first.plots[0]
        .sample
        .expect("the first paint's plot must carry its sampling fact");
    assert_eq!(before.of, 4096, "`of` is the unsampled count, measured");
    assert!(
        before.drawn > 0 && before.drawn < before.of,
        "a 1-in-32 sample of 4096 rows drew {} — expected some but not all",
        before.drawn
    );

    // A brush, pushed the way a real drag pushes one. The contributor path is
    // deliberately not this plot's own, so self-exclusion does not drop it.
    let after_composed = dash
        .apply(Interaction::Select {
            name: "brush".to_string(),
            contributor: ComponentPath("root/vconcat[99]".to_string()),
            predicate: SqlPredicate::Expr("a < 50.0".to_string()),
        })
        .expect("re-present after the brush");

    let after = after_composed.plots[0]
        .sample
        .expect("the notice must survive a brush — this is the invisible failure");
    assert!(
        after.drawn < before.drawn,
        "fixture check: the brush must actually narrow the picture ({} -> {}), or the \
         assertion above proves nothing about a re-present",
        before.drawn,
        after.drawn
    );
    assert!(
        after.of < before.of,
        "`of` is re-measured under the live selection, not carried: a notice quoting the \
         pre-brush total would be wrong in a way nobody could see ({} -> {})",
        before.of,
        after.of
    );
    assert!(
        after.drawn > 0,
        "fixture check: the brushed sample must still draw something"
    );

    // And the ink, in the band, in the chart-only export — after the gesture.
    let png = dir.join("after-brush.png");
    capture_vello_only(after_composed, 1.0, &png).expect("capture after the brush");
    let ink = ink_in_band(&png);
    assert!(
        ink > 400,
        "the post-brush export's bottom band held {ink} inked pixels — the hatch and label \
         should still be plainly there"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **A sampled plot's positional scales are the complete plot's.** Asserted
/// where a gesture reads them — `PlotHandle::scales`, the set the shell inverts
/// pixels through — rather than on the function that widens them.
///
/// This is the difference between a sampled picture being a thinner drawing of
/// the same chart and being a different chart. Ticks in the same places, and a
/// drag from the same pixel resolving to the same data interval. It is also
/// what keeps the sign-off honest: a human comparing the two renders should be
/// judging the sampling treatment, not noticing that the axes moved.
#[test]
fn a_sampled_plot_keeps_the_complete_plots_positional_scales() {
    use brightfield_render::channel::Channel;
    use brightfield_render::scale::Scale;

    let dir = std::env::temp_dir().join(format!("bf-sampled-domains-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let spec = write_spec(&dir);
    let path = spec.to_str().unwrap();

    let complete = compose_spec_sampled(path, None).expect("compose complete");
    let rate = SampleRate::from_modulus(32).expect("power of two");
    let sampled = compose_spec_sampled(path, Some(rate)).expect("compose sampled");

    let domain =
        |c: &brightfield_shell::pipeline::Composed, ch: Channel| match c.plots[0].scales.get(ch) {
            Some(Scale::Linear {
                domain_min,
                domain_max,
                ..
            }) => (*domain_min, *domain_max),
            other => panic!("expected a linear {ch:?} scale, got {other:?}"),
        };

    for ch in [Channel::X, Channel::Y] {
        assert_eq!(
            domain(&sampled, ch),
            domain(&complete, ch),
            "the sampled plot's {ch:?} domain must equal the complete plot's. If it has \
             narrowed, the axis ticks have moved and a brush on this plot now inverts to a \
             different data interval than the same brush on the complete one — a difference \
             a reader cannot see and would not attribute to sampling."
        );
    }

    // Not vacuous: the sample really did drop rows, so the domains had to be
    // restored rather than merely happening to agree.
    let fact = sampled.plots[0].sample.expect("sampled");
    assert!(
        fact.drawn * 4 < fact.of,
        "fixture check: a 1-in-32 sample must drop most of the rows ({} of {})",
        fact.drawn,
        fact.of
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A four-class scatter — the shape `--force-sample` silently miscolours, and
/// the shape the refusal exists for.
const CATEGORICAL_SPEC: &str = "data:
  points:
    query: |
      SELECT (i * 7919 % 1009) / 10.0 AS a,
             (i * 104729 % 1013) / 10.0 AS b,
             'class-' || (i % 4) AS g
      FROM range(20000) AS t(i)
plot:
  - mark: dot
    data: { from: points }
    x: a
    y: b
    fill: g
width: 400
height: 300
";

/// **Sampling a categorical channel is refused, not drawn.**
///
/// Measured before the refusal existed: rendering this spec complete and at
/// `--force-sample 64`, the colour scale's category list came back as
/// `[class-0, class-1, class-2, class-3]` complete and
/// `[class-0, class-2, class-1, class-3]` sampled — so `class-1` was drawn
/// amber in one picture and teal in the other. A palette slot is a category's
/// INDEX in that list, and the list is built in first-appearance order over the
/// rows that were drawn, so dropping rows re-assigns the colours. The sampling
/// notice says rows were dropped. It does not, and could not, say the legend
/// was rewritten.
///
/// `--force-sample` is a shipped flag on `brightfield-shot` and on the live
/// window, so until categorical domains can be restored deterministically the
/// only honest answer at the point of use is to decline and say why.
#[test]
fn sampling_a_categorical_channel_is_refused_with_a_reason() {
    let dir = std::env::temp_dir().join(format!("bf-sampled-cat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let spec = dir.join("categorical.yaml");
    std::fs::write(&spec, CATEGORICAL_SPEC).expect("write spec");
    let path = spec.to_str().unwrap();

    // Unsampled, the very same spec composes fine. The refusal is scoped to
    // sampling and does not cost an ordinary chart anything.
    compose_spec_sampled(path, None).expect("the complete render is unaffected");

    let rate = SampleRate::from_modulus(64).expect("power of two");
    let err = compose_spec_sampled(path, Some(rate))
        .err()
        .expect("sampling a categorical fill must be refused, not drawn");
    assert!(
        err.contains("refusing to sample"),
        "the refusal must say it is refusing: {err}"
    );
    assert!(
        err.contains("fill"),
        "the refusal must name the offending channel so it is actionable: {err}"
    );

    // The live window takes the same path, so it refuses on the way in rather
    // than opening a window onto a wrong picture.
    let live = live_spec_sampled(path, Some(rate));
    assert!(
        live.is_err(),
        "the live window's --force-sample must refuse the same spec"
    );

    // And the continuous spec beside it still samples, so the refusal is a
    // scalpel and not a blanket.
    let ok_spec = write_spec(&dir);
    compose_spec_sampled(ok_spec.to_str().unwrap(), Some(rate))
        .expect("a continuous x/y plot must still be sampleable");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The sign-off apparatus, verified rather than asserted: the matched control
/// really does draw the rows the sampled render draws.
///
/// `examples/sampled-matched.yaml` writes the sampling clause out by hand — the
/// same subquery-alias row hash at the same modulus — so brightfield loads it
/// as an ordinary complete dataset. That is what makes it a control: two
/// pictures of the SAME points at the SAME density, one carrying the band and
/// one not, so a human judging them is judging the treatment and nothing else.
///
/// If the two counts ever diverge, the control has quietly become a comparison
/// between two different point sets and the sitting is back to judging density.
#[test]
fn the_matched_control_draws_exactly_the_rows_the_sample_draws() {
    let examples = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/");
    let rate = SampleRate::from_modulus(32).expect("power of two");

    let sampled = compose_spec_sampled(&format!("{examples}sampled.yaml"), Some(rate))
        .expect("compose the sampled demo");
    let fact = sampled.plots[0]
        .sample
        .expect("the demo spec must be sampleable");
    assert_eq!(fact.of, 75_000, "the demo spec's row count moved");

    let matched = compose_spec_sampled(&format!("{examples}sampled-matched.yaml"), None)
        .expect("compose the matched control");
    assert!(
        matched.plots[0].sample.is_none(),
        "the control must carry NO sampling fact — it is the picture without the band"
    );

    // The control's own row count, read back through the grid-side count the
    // engine keeps for the mark.
    let mut live = brightfield_shell::pipeline::LiveDashboard::load_str(
        &std::fs::read_to_string(format!("{examples}sampled-matched.yaml")).expect("read control"),
        None,
    )
    .expect("load the control live");
    let control_rows = live
        .coordinator()
        .session()
        .step_rows_count(0)
        .expect("count the control's rows");

    assert_eq!(
        control_rows, fact.drawn,
        "the matched control draws {control_rows} rows and the sampled render draws {}. \
         They are supposed to be the same set, chosen by the same hash at the same modulus — \
         if they have drifted, the pairing no longer isolates the treatment.",
        fact.drawn
    );
}
