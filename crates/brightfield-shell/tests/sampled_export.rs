//! The two halves of the sampling notice that can be machine-checked: that it
//! survives a chart-only export, and that it survives a brush.
//!
//! `capture_vello_only` rasterises the composed Vello scene and never
//! constructs an egui context, so everything the shell draws — the top bar, the
//! margin legend, any banner anyone might be tempted to add — is absent from
//! that PNG **by construction**. It is the mechanical meaning of "the notice
//! travels with the chart through the ordinary export path", which is why the
//! notice is drawn into the plot's own scene rather than into chrome. That is
//! an argument about *informing a reader who exports a chart*, and no longer an
//! argument about defeating one who crops it — anti-tamper is out of scope for
//! this surface, and designing against it buys ink without buying honesty.
//!
//! The second half goes through [`LiveDashboard::present`], which is the path
//! the live window repaints on — not the one-shot `compose_spec_sampled` call.
//! Those are two different functions gathering the facts, and only one of them
//! runs after a gesture; a suite that exercised only the one-shot call would be
//! green with the live path erasing the notice on the first drag, which is
//! precisely the invisible degradation this whole device exists to prevent.

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

/// Count pixels in the bottom band that match `want`, within a tolerance of a
/// few levels per channel for the compositor's rounding — not enough to admit a
/// different token.
fn band_pixels_matching(png: &std::path::Path, want: meridian_design::Rgba) -> u64 {
    let want = [
        (want.r * 255.0).round() as i32,
        (want.g * 255.0).round() as i32,
        (want.b * 255.0).round() as i32,
    ];
    let img = image::open(png).expect("open png").to_rgba8();
    let band_top = H - NOTICE_BAND.ceil() as u32;
    let mut hits = 0u64;
    for y in band_top..H {
        for x in 0..W {
            let p = img.get_pixel(x, y).0;
            if (0..3).all(|c| (i32::from(p[c]) - want[c]).abs() <= 4) {
                hits += 1;
            }
        }
    }
    hits
}

/// The role the notice speaks in: **info**, not warning. Sampling is a
/// deliberate performance accommodation on a healthy plot, and the band advises
/// a reader rather than reporting a fault; a caution hue spent on a working
/// state either reads as breakage or, repeated, stops meaning anything.
fn notice_role() -> meridian_design::semantic::RoleColours {
    *meridian_design::semantic(false).role(meridian_design::Role::Info)
}

/// Count pixels in the band that are the notice's **attention fill** — the
/// design system's info solid, read from the token layer rather than
/// transcribed, so a token bump moves the expectation with it.
///
/// This is the assertion `ink_in_band` cannot make. "Not near-white" was
/// satisfied by any ink at all: the old hatch, a stray axis rule, a mark that
/// escaped its clip. What has to be true is that the band a reader's eye is
/// meant to catch is *the colour that catches it*, and that it is filled rather
/// than trimmed.
fn attention_fill_in_band(png: &std::path::Path) -> u64 {
    band_pixels_matching(png, notice_role().background.base)
}

/// Count pixels of the notice's **label ink** that landed *inside the fill* —
/// the glyphs, as pixels, not as a paint the scene requested.
///
/// The design system's rule is that a status hue is never colour alone. The
/// scene-level check for that counts encoded glyph resources, which a label
/// drawn in the fill's own colour, or shoved off the edge of the band,
/// satisfies just as well. This is the version of the rule a screenshot can be
/// held to.
///
/// The horizontal window is **found, not assumed**: the scan is confined to the
/// columns the attention fill actually occupies, which is self-locating against
/// a margin change and is, incidentally, the assertion that the words are on
/// the band rather than beside it. A picture with no fill has no window and
/// scores zero.
///
/// Inside that window a pixel is scored by which of the two paints it is
/// *nearer* to, rather than by an exact match. Glyphs at label size are mostly
/// anti-aliased edge — an exact match counts 23 pixels of a plainly legible
/// sentence and would have to be floored so low it proved nothing. Nearest-of-
/// two is the same question asked at the resolution the type is actually drawn
/// at, and it still cannot be satisfied by a label painted in the fill's own
/// colour, which is the failure it exists to catch.
fn label_ink_inside_the_fill(png: &std::path::Path) -> u64 {
    let quantise = |c: meridian_design::Rgba| {
        [
            (c.r * 255.0) as f64,
            (c.g * 255.0) as f64,
            (c.b * 255.0) as f64,
        ]
    };
    let fill = quantise(notice_role().background.base);
    let ink = quantise(notice_role().foreground.base);
    let dist2 = |p: &[u8; 4], want: &[f64; 3]| {
        (0..3)
            .map(|c| (f64::from(p[c]) - want[c]).powi(2))
            .sum::<f64>()
    };
    // A pixel counts as fill only if it is genuinely that colour, so an
    // anti-aliased glyph edge cannot widen the window.
    let is_fill = |p: &[u8; 4]| (0..3).all(|c| (f64::from(p[c]) - fill[c]).abs() <= 4.0);

    let img = image::open(png).expect("open png").to_rgba8();
    let band_top = H - NOTICE_BAND.ceil() as u32;

    let mut left = None;
    let mut right = 0u32;
    for y in band_top..H {
        for x in 0..W {
            if is_fill(&img.get_pixel(x, y).0) {
                left = Some(left.map_or(x, |l: u32| l.min(x)));
                right = right.max(x);
            }
        }
    }
    let Some(left) = left else { return 0 };

    let mut hits = 0u64;
    for y in band_top..H {
        for x in left..=right {
            let p = img.get_pixel(x, y).0;
            if dist2(&p, &ink) < dist2(&p, &fill) {
                hits += 1;
            }
        }
    }
    hits
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
        "the sampled export's bottom band held {sampled_ink} inked pixels — the fill and \
         label should be plainly there. If this is near zero the notice did not survive the \
         chart-only export, which is the whole reason it is not a banner."
    );
    assert!(
        sampled_ink > complete_ink * 3,
        "the band is supposed to be the DIFFERENCE between the two exports: sampled held \
         {sampled_ink} inked pixels there, complete held {complete_ink}"
    );

    // …and the ink is the attention fill, filling the band rather than
    // trimming it. The band spans the plot's own width, so its area is at least
    // (W - both margins) × NOTICE_BAND; two thirds of the plot width, at full
    // band height, is a floor no partial treatment clears and the label's own
    // glyphs cannot lift a lesser one over.
    let filled = attention_fill_in_band(&sampled_png);
    let floor = (u64::from(W) * 2 / 3) * NOTICE_BAND.floor() as u64;
    assert!(
        filled > floor,
        "the band held {filled} pixels of the info solid, under the {floor} a filled \
         band owes — colour is what draws the eye to the sentence, and a band that is \
         only text is a band nobody looks at"
    );
    assert_eq!(
        attention_fill_in_band(&complete_png),
        0,
        "a COMPLETE export must carry none of it — an attention fill that appears on an \
         unsampled chart teaches a reader that the band means nothing"
    );

    // …and the SENTENCE landed on that fill, in pixels. Colour alone is the one
    // thing the design system forbids, and a band that is only colour says a
    // reader should care without saying what about.
    let words = label_ink_inside_the_fill(&sampled_png);
    assert!(
        words > 200,
        "only {words} pixels of the notice's label ink landed on the fill — the band is \
         colour without a sentence, which tells a reader something is different and \
         nothing about what"
    );
    assert_eq!(
        label_ink_inside_the_fill(&complete_png),
        0,
        "there is no fill on a complete export, so there is nothing for a label to be on"
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
        "the post-brush export's bottom band held {ink} inked pixels — the fill and label \
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

/// A scatter whose fill has one class far rarer than the others, and whose
/// classes' names do not sort into the order the rows arrive in.
///
/// Both properties are load-bearing. The rare class is the one a sample drops
/// outright, which is what makes the unsampled value SET do work rather than
/// merely agreeing with the drawn one. And `earlybird` arriving first while
/// sorting last is what tells first-appearance order apart from the ordering
/// rule — a domain that came out `[earlybird, …]` would be the scan's order
/// wearing the new rule's clothes.
const CATEGORICAL_SPEC: &str = "data:
  points:
    query: |
      SELECT (i * 7919 % 1009) / 10.0 AS a,
             (i * 104729 % 1013) / 10.0 AS b,
             CASE WHEN i < 3 THEN 'earlybird' ELSE 'class-' || (i % 4) END AS g
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

/// The colour a category is drawn in, read off the scale a plot handed back.
///
/// Through [`Scale::map_colour`] rather than off the category list, because the
/// colour is the thing that has to agree; the list is only how it is decided.
/// An assertion on the list alone would go green on a repair that kept the list
/// and changed the palette lookup.
fn category_ink(
    c: &brightfield_shell::pipeline::Composed,
    channel: brightfield_render::channel::Channel,
    category: &str,
) -> [f32; 4] {
    c.plots[0]
        .scales
        .get(channel)
        .unwrap_or_else(|| panic!("the plot must carry a {channel:?} scale"))
        .map_colour(category)
        .unwrap_or_else(|| panic!("{category} must have a colour on the {channel:?} scale"))
}

/// The category list of a plot's colour scale.
fn colour_categories(
    c: &brightfield_shell::pipeline::Composed,
    channel: brightfield_render::channel::Channel,
) -> Vec<String> {
    match c.plots[0].scales.get(channel) {
        Some(brightfield_render::scale::Scale::Colour { categories, .. }) => categories.clone(),
        other => panic!("expected a colour {channel:?} scale, got {other:?}"),
    }
}

/// **A sampled plot's colour channel draws, in the complete plot's colours.**
///
/// Measured before this was true: rendering this spec complete and at
/// `--force-sample 64`, the colour scale's category list came back in one order
/// complete and another sampled, so a class was drawn amber in one picture and
/// teal in the other. A palette slot is a category's INDEX in that list, the
/// list was built in first-appearance order over the rows that were DRAWN, and
/// dropping rows therefore re-assigned the colours. The sampling notice says
/// rows were dropped. It does not, and could not, say the legend was rewritten.
///
/// Asserted through `PlotHandle::scales` — the set the shell draws and legends
/// from — and per category, so a picture that agreed on the list while
/// disagreeing on any one class's ink would still fail.
#[test]
fn a_sampled_colour_channel_draws_in_the_complete_renders_colours() {
    use brightfield_render::channel::Channel;

    let dir = std::env::temp_dir().join(format!("bf-sampled-cat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let spec = dir.join("categorical.yaml");
    std::fs::write(&spec, CATEGORICAL_SPEC).expect("write spec");
    let path = spec.to_str().unwrap();

    let complete = compose_spec_sampled(path, None).expect("the complete render is unaffected");
    let rate = SampleRate::from_modulus(64).expect("power of two");
    let sampled = compose_spec_sampled(path, Some(rate))
        .expect("a sampled plot carrying a colour channel must DRAW, not be refused");

    let complete_cats = colour_categories(&complete, Channel::Fill);
    let sampled_cats = colour_categories(&sampled, Channel::Fill);
    assert_eq!(
        sampled_cats, complete_cats,
        "the sampled plot's colour domain must be the complete plot's, category for \
         category and slot for slot"
    );
    for cat in &complete_cats {
        assert_eq!(
            category_ink(&sampled, Channel::Fill, cat),
            category_ink(&complete, Channel::Fill, cat),
            "{cat} is drawn in a different colour by the two renders — the reader sees a \
             legend that was rewritten under a notice that says only that rows were dropped"
        );
    }

    // Fixture check, and the reason the unsampled value SET is doing work here
    // rather than agreeing by luck: the rare class is in the complete render's
    // domain and absent from the rows the sample drew, so a domain built from
    // the drawn rows alone would be short by one and every class after it would
    // slide a slot.
    assert!(
        complete_cats.iter().any(|c| c == "earlybird"),
        "fixture check: the complete render must see the rare class, got {complete_cats:?}"
    );
    let fact = sampled.plots[0]
        .sample
        .expect("the sampled composition's plot must carry its fact");
    assert!(
        fact.drawn > 0 && fact.drawn < fact.of,
        "fixture check: the sample must drop rows ({} of {})",
        fact.drawn,
        fact.of
    );

    // The live window takes the same path, so it draws the same colours rather
    // than opening a window onto a differently-coloured picture.
    let (_, first) = live_spec_sampled(path, Some(rate))
        .expect("the live window's --force-sample must draw the same spec");
    assert_eq!(
        colour_categories(&first, Channel::Fill),
        complete_cats,
        "the live window's first paint must agree with the one-shot render"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **The category order is a function of the category set, not of the scan.**
///
/// The rule the restoration depends on. DuckDB does not promise to repeat the
/// row order of a parallel scan, so a domain built in first-appearance order is
/// a domain that can differ between two runs of one spec — and the sampled
/// render agreeing with the complete one is worth nothing if neither agrees
/// with itself.
///
/// Two assertions, because either alone is weak. Repeated composition catches a
/// rule that is not a function of the set at all. The order the fixture arrives
/// in catches a rule that is one but is still the scan's: `earlybird` is
/// written to arrive first and sorts last, so a domain that leads with it is
/// first appearance, however stable.
#[test]
fn the_sampled_category_order_does_not_come_from_the_scan() {
    use brightfield_render::channel::Channel;

    let dir = std::env::temp_dir().join(format!("bf-sampled-order-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let spec = dir.join("categorical.yaml");
    std::fs::write(&spec, CATEGORICAL_SPEC).expect("write spec");
    let path = spec.to_str().unwrap();
    let rate = SampleRate::from_modulus(64).expect("power of two");

    let first = colour_categories(
        &compose_spec_sampled(path, Some(rate)).expect("compose sampled"),
        Channel::Fill,
    );

    let mut sorted = first.clone();
    sorted.sort();
    assert_eq!(
        first, sorted,
        "the colour domain must be ordered by the category's own text. `earlybird` is \
         written into the fixture to arrive before every `class-` row and to sort after \
         them all, so a domain in any other order is the scan's order."
    );
    assert_ne!(
        first.first().map(String::as_str),
        Some("earlybird"),
        "fixture check: the rule under test must be distinguishable from first appearance"
    );

    for run in 1..4 {
        let again = colour_categories(
            &compose_spec_sampled(path, Some(rate)).expect("compose sampled again"),
            Channel::Fill,
        );
        assert_eq!(
            again, first,
            "run {run} of the same spec produced a different colour domain — a palette \
             slot that moves between runs is a legend that moves between runs"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// A scatter positioned on a categorical x — a BAND scale, which the ordering
/// rule deliberately does not cover.
const BAND_SPEC: &str = "data:
  points:
    query: |
      SELECT 'class-' || (i % 4) AS g,
             (i * 104729 % 1013) / 10.0 AS b
      FROM range(20000) AS t(i)
plot:
  - mark: dot
    data: { from: points }
    x: g
    y: b
width: 400
height: 300
";

/// **A band scale is still refused under sampling, and says which channel.**
///
/// The refusal narrowed; it did not go. A band domain's order is where the bars
/// ARE, so the ordering that fixes a colour scale would here overwrite whatever
/// order the query put the rows in — restoring one needs the unsampled ORDER
/// and not merely the unsampled set. A sequential ramp is anchored to the drawn
/// extent, which a category list puts nothing back for either.
///
/// This is what keeps the narrowing honest: the same spec shape that now draws
/// with a colour channel is refused with a positional one, so "the refusal was
/// removed for the kinds the ordering covers" is a claim with a failing case.
#[test]
fn sampling_a_band_channel_is_still_refused_with_a_reason() {
    let dir = std::env::temp_dir().join(format!("bf-sampled-band-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let spec = dir.join("band.yaml");
    std::fs::write(&spec, BAND_SPEC).expect("write spec");
    let path = spec.to_str().unwrap();

    // Unsampled, the very same spec composes fine. The refusal is scoped to
    // sampling and does not cost an ordinary chart anything.
    compose_spec_sampled(path, None).expect("the complete render is unaffected");

    let rate = SampleRate::from_modulus(64).expect("power of two");
    let err = compose_spec_sampled(path, Some(rate))
        .err()
        .expect("sampling a band x must be refused, not drawn");
    assert!(
        err.contains("refusing to sample"),
        "the refusal must say it is refusing: {err}"
    );
    assert!(
        err.contains('x'),
        "the refusal must name the offending channel so it is actionable: {err}"
    );

    // The live window takes the same path, so it refuses on the way in rather
    // than opening a window onto a wrong picture.
    assert!(
        live_spec_sampled(path, Some(rate)).is_err(),
        "the live window's --force-sample must refuse the same spec"
    );

    // And the continuous spec beside it still samples, so the refusal is a
    // scalpel and not a blanket.
    let ok_spec = write_spec(&dir);
    compose_spec_sampled(ok_spec.to_str().unwrap(), Some(rate))
        .expect("a continuous x/y plot must still be sampleable");

    let _ = std::fs::remove_dir_all(&dir);
}

/// A scatter whose `fill` names a literal colour rather than a column.
const LITERAL_FILL_SPEC: &str = "data:
  points:
    query: |
      SELECT (i * 7919 % 1009) / 10.0 AS a, (i * 104729 % 1013) / 10.0 AS b
      FROM range(4096) AS t(i)
plot:
  - mark: dot
    data: { from: points }
    x: a
    y: b
    fill: steelblue
width: 400
height: 300
";

/// **A fill that names a colour rather than a column still carries its notice.**
///
/// The measurement that lifts the refusal is a query per colour channel, and a
/// literal colour is indistinguishable from a column name in the spec, so
/// `"steelblue"` is handed to DuckDB as an identifier and refused. That failure
/// has to stay inside its own channel: the caller reads a facts error as "no
/// facts", and a facts error would leave a plot that really is sampled drawing
/// with no notice at all — the exact invisible degradation the notice exists to
/// prevent.
#[test]
fn a_literal_colour_fill_does_not_cost_a_sampled_plot_its_notice() {
    let dir = std::env::temp_dir().join(format!("bf-sampled-literal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let spec = dir.join("literal-fill.yaml");
    std::fs::write(&spec, LITERAL_FILL_SPEC).expect("write spec");
    let path = spec.to_str().unwrap();

    let rate = SampleRate::from_modulus(8).expect("power of two");
    let sampled = compose_spec_sampled(path, Some(rate)).expect("compose sampled");
    let fact = sampled.plots[0]
        .sample
        .expect("a sampled plot whose fill is a literal colour must still carry its fact");
    assert_eq!(fact.of, 4096, "`of` is the unsampled count, measured");
    assert!(
        fact.drawn > 0 && fact.drawn < fact.of,
        "fixture check: the sample must drop rows ({} of {})",
        fact.drawn,
        fact.of
    );

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

/// An aggregating mark whose rows are BINS.
const AGGREGATING_SPEC: &str = "data:
  points:
    query: |
      SELECT (i * 7919 % 1009) / 10.0 AS a
      FROM range(4096) AS t(i)
plot:
  - mark: densityX
    data: { from: points }
    x: a
width: 400
height: 300
";

/// **A plot the rate never reached must not claim to be sampled.**
///
/// The rate lives on the session, but the emitter applies the clause only to
/// non-aggregating plans — an aggregating mark's rows are bins, and sampling
/// bins is not sampling data. The facts query did not know that: it fired for
/// every mark whenever a rate was set, and since the mark was not actually
/// sampled it reported `drawn == of`. Measured before the fix: a `densityX`
/// plot under `--force-sample 32` composed with
/// `SampleFact { drawn: 100, of: 100 }` and drew the notice band across a
/// picture that had lost nothing.
///
/// A notice that fires on a complete plot is the same lie as a notice that
/// fails to fire on a sampled one, and it is worse for the sign-off: it teaches
/// a reader that the band means nothing.
#[test]
fn an_unsampled_aggregating_plot_carries_no_sampling_notice() {
    let dir = std::env::temp_dir().join(format!("bf-sampled-agg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let spec = dir.join("aggregating.yaml");
    std::fs::write(&spec, AGGREGATING_SPEC).expect("write spec");
    let path = spec.to_str().unwrap();

    let rate = SampleRate::from_modulus(32).expect("power of two");
    let sampled = compose_spec_sampled(path, Some(rate))
        .expect("an aggregating mark composes under a rate — it simply ignores it");
    assert!(
        sampled.plots[0].sample.is_none(),
        "an aggregating mark is never sampled, so its plot must carry no fact and draw \
         no band — got {:?}",
        sampled.plots[0].sample
    );

    // And the picture is byte-for-byte the one the same spec draws with no rate
    // at all: no reserved band, no shifted geometry, nothing.
    let complete = compose_spec_sampled(path, None).expect("compose complete");
    let png_a = dir.join("agg-complete.png");
    let png_b = dir.join("agg-sampled.png");
    capture_vello_only(complete, 1.0, &png_a).expect("capture complete");
    capture_vello_only(sampled, 1.0, &png_b).expect("capture under a rate");
    assert_eq!(
        std::fs::read(&png_a).expect("read a"),
        std::fs::read(&png_b).expect("read b"),
        "setting a sample rate changed an aggregating plot's pixels — it should be inert"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
