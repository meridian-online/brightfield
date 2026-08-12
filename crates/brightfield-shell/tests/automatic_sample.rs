//! **Nothing had to be told.** The sampling policy, held to the boundary it
//! draws and to the query it issues.
//!
//! The mechanism this exercises shipped with a manual driver: `--force-sample`
//! on either binary and nothing else. These are the assertions that a plot now
//! decides for itself.
//!
//! The boundary cases are built at row counts taken FROM the ceiling constant
//! rather than written out here, so moving the constant moves those fixtures
//! with it instead of leaving a suite testing a boundary the product no longer
//! draws. The committed ten-million-row example is the exception, and has to
//! be: its count is the magnitude the scale claim is made at, not a position
//! relative to the ceiling.

use std::path::{Path, PathBuf};

use brightfield_protocol::layout::Flow;
use brightfield_render::channel::Channel;
use brightfield_render::sample_notice::NOTICE_BAND;
use brightfield_render::sample_policy::{renders_complete, sample_exponent, MEASURED_INKED_MAX};
use brightfield_render::scale::Scale;
use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::{compose_spec_sampled, live_spec_sampled, Composed};
use brightfield_shell::window::{chart_window_size, Boot, MeridianApp};
use brightfield_sql::ir::SampleRate;

const W: u32 = 640;
const H: u32 = 480;

/// A row-level dot scatter over `rows` rows generated in DuckDB, so a fixture
/// is a row count and nothing else.
fn scatter(rows: u64) -> String {
    format!(
        "data:
  points:
    query: |
      SELECT
        (i * 2654435761 % 100003) / 1000.0            AS spread,
        ((i * 40503 + 12345) % 100019) / 1000.0       AS depth
      FROM range({rows}) AS t(i)
plot:
  - mark: dot
    data: {{ from: points }}
    x: spread
    y: depth
width: {W}
height: {H}
"
    )
}

/// A row-level dot scatter whose `band_axis` is a CATEGORY, over `rows` rows
/// and `classes` classes — the shape every bar chart and categorical scatter
/// shares, and the one whose positional scale is a band.
///
/// The class names are laid down in descending numeric order, so the order the
/// rows produce is neither the ascending one an author would guess nor the
/// ascending-by-text one a comparator would impose. A restoration that sorted
/// the list, or that installed the drawn rows' own order, lands somewhere else.
fn categorical_scatter(rows: u64, classes: u64, band_axis: &str) -> String {
    let value_axis = if band_axis == "x" { "y" } else { "x" };
    format!(
        "data:
  points:
    query: |
      SELECT
        'class-' || ({classes} - 1 - (i % {classes}))::VARCHAR   AS band,
        ((i * 40503 + 12345) % 100019) / 1000.0                  AS depth
      FROM range({rows}) AS t(i)
plot:
  - mark: dot
    data: {{ from: points }}
    {band_axis}: band
    {value_axis}: depth
width: {W}
height: {H}
"
    )
}

/// The category list an axis will lay out, in the order it will lay it out.
fn band_order(composed: &Composed, channel: Channel) -> Vec<String> {
    match composed.plots[0].scales.get(channel) {
        Some(Scale::Band { categories, .. }) => categories.clone(),
        other => panic!("expected a {channel:?} band scale, got {other:?}"),
    }
}

/// Write a fixture into a test-private directory and hand back its path.
fn fixture(tag: &str, source: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("bf-auto-sample-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("spec.yaml");
    std::fs::write(&path, source).expect("write spec");
    (dir, path)
}

/// Compose with no rate named — the command line with no flag on it.
fn compose_unflagged(path: &Path) -> Composed {
    compose_spec_sampled(path.to_str().expect("utf-8 path"), None).expect("compose")
}

/// Count pixels in the bottom `NOTICE_BAND` rows that are not the near-white
/// page or surface tone — the band the notice reserves.
fn ink_in_band(png: &Path) -> u64 {
    let img = image::open(png).expect("open png").to_rgba8();
    let band_top = H - NOTICE_BAND.ceil() as u32;
    let mut ink = 0u64;
    for y in band_top..H {
        for x in 0..W {
            let p = img.get_pixel(x, y).0;
            if p[0] < 0xf0 || p[1] < 0xf0 || p[2] < 0xf0 {
                ink += 1;
            }
        }
    }
    ink
}

/// The bottom `NOTICE_BAND` rows as raw bytes — the notice's text and its
/// geometry together, with nothing about them inferred.
fn band_bytes(png: &Path) -> Vec<u8> {
    let img = image::open(png).expect("open png").to_rgba8();
    let band_top = H - NOTICE_BAND.ceil() as u32;
    let mut out = Vec::new();
    for y in band_top..H {
        for x in 0..W {
            out.extend_from_slice(&img.get_pixel(x, y).0);
        }
    }
    out
}

/// **Both sides of the boundary, one row apart, with no flag on the command
/// line.**
///
/// The two counts are adjacent: [`MEASURED_INKED_MAX`] is the largest count
/// measured to ink a frame. A policy that compared the wrong way round, or
/// that used a different threshold from the one the constant names, cannot
/// satisfy both rows — which is what makes this a boundary test rather than
/// two spot checks.
#[test]
fn a_spec_above_the_ceiling_samples_itself_and_one_below_does_not() {
    let (complete_dir, complete_spec) = fixture("below", &scatter(MEASURED_INKED_MAX));
    let (sampled_dir, sampled_spec) = fixture("above", &scatter(MEASURED_INKED_MAX + 1));

    let complete = compose_unflagged(&complete_spec);
    let sampled = compose_unflagged(&sampled_spec);

    assert!(
        complete.plots[0].sample.is_none(),
        "a spec at the largest count measured to ink a frame must render COMPLETE \
         with no flag — it draws, and a sample would drop half its points for nothing"
    );
    let fact = sampled.plots[0]
        .sample
        .expect("one primitive past the ceiling must render SAMPLED with no flag");
    assert_eq!(
        fact.of,
        MEASURED_INKED_MAX + 1,
        "`of` is the unsampled count the plot would have drawn"
    );
    assert!(
        fact.drawn > 0 && fact.drawn < fact.of,
        "the automatic sample drew {} of {} — expected some but not all",
        fact.drawn,
        fact.of
    );

    // …and the notice is in the picture, not only on the handle. The band is
    // taken out of the plot's margin rather than added to the image, so the two
    // exports are the same size and directly comparable.
    let complete_png = complete_dir.join("complete.png");
    let sampled_png = sampled_dir.join("sampled.png");
    assert_eq!((complete.width, complete.height), (W, H));
    assert_eq!((sampled.width, sampled.height), (W, H));
    capture_vello_only(complete, 1.0, &complete_png).expect("capture complete");
    capture_vello_only(sampled, 1.0, &sampled_png).expect("capture sampled");

    let below = ink_in_band(&complete_png);
    let above = ink_in_band(&sampled_png);
    assert!(
        above > 400,
        "the automatically sampled export's bottom band held {above} inked pixels — \
         the fill and the label should be plainly there"
    );
    assert!(
        above > below * 3,
        "the band is the DIFFERENCE between the two: above the ceiling held {above} \
         inked pixels there, below held {below}"
    );

    let _ = std::fs::remove_dir_all(&complete_dir);
    let _ = std::fs::remove_dir_all(&sampled_dir);
}

/// **The first query already carries the predicate.**
///
/// A full result set materialised and then thrown away costs the memory the
/// push-down exists to avoid. So: one execute per row-level mark, that
/// execute's SQL carrying the sample clause, and the rows it materialised
/// being the sampled count.
#[test]
fn the_first_execution_of_a_row_level_mark_is_already_sampled() {
    let (dir, spec) = fixture("first-query", &scatter(MEASURED_INKED_MAX + 1));

    let (mut dash, composed) =
        live_spec_sampled(spec.to_str().expect("utf-8 path"), None).expect("live, unflagged");

    // `drawn` is the drawn batch's own row count — the rows that execution
    // materialised, not a figure inferred from the modulus.
    let fact = composed.plots[0]
        .sample
        .expect("fixture check: this spec is above the ceiling and must sample itself");

    let session = dash.coordinator().session();
    let executes = session.duckdb_execute_count();
    assert_eq!(
        executes, 1,
        "the one mark was executed {executes} times. Twice means an unsampled result \
         set was materialised and discarded before the sampled one was asked for"
    );

    // The statements the session actually sent to DuckDB through its execution
    // choke points, in order — so this is the query that ran, not a second
    // emission asked for afterwards.
    //
    // Selected by the predicate rather than by position, because the record
    // also holds the statements that restore what the sample dropped: the
    // unsampled row count, the positional domains, the band order. Those are
    // emitted with NO rate, by construction — they are the picture the sample
    // is a sample OF — so the one statement carrying the clause is the one the
    // mark was DRAWN from, and a second would be the discarded materialisation
    // this test exists to catch.
    let executed = session.executed_sql();
    let modulus =
        1_u32 << sample_exponent(fact.of).expect("fixture check: this count needs a sample");
    let predicate = format!("hash(_s) % {modulus} = 0");
    let sampled: Vec<&String> = executed
        .iter()
        .filter(|sql| sql.contains(&predicate))
        .collect();
    assert_eq!(
        sampled.len(),
        1,
        "one mark, one statement at the chosen modulus — got {executed:#?}"
    );
    assert!(
        fact.drawn < fact.of,
        "a 1-in-{modulus} sample of {} rows materialised {} — the clause is in the \
         SQL but nothing was dropped",
        fact.of,
        fact.drawn
    );
    assert!(
        renders_complete(fact.drawn),
        "the point of the exercise: {} rows arrived, which is still past the ceiling",
        fact.drawn
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **The notice does not depend on who chose the rate.**
///
/// Same spec, same effective modulus, one arrived at by the policy and one by
/// `--force-sample`. The bottom band of the chart-only export is compared byte
/// for byte, which is text and geometry at once and admits no "close enough":
/// a different sentence, a different position, a different band height all
/// change those bytes.
#[test]
fn an_automatic_notice_is_byte_identical_to_a_forced_one_at_the_same_modulus() {
    let rows = MEASURED_INKED_MAX + 1;
    let (dir, spec) = fixture("same-modulus", &scatter(rows));
    let path = spec.to_str().expect("utf-8 path");

    let exponent = sample_exponent(rows).expect("fixture check: this count needs a sample");
    let forced = SampleRate::from_exponent(exponent).expect("a policy exponent is a legal rate");

    let automatic = compose_spec_sampled(path, None).expect("compose unflagged");
    let explicit = compose_spec_sampled(path, Some(forced)).expect("compose forced");

    assert_eq!(
        automatic.plots[0].sample, explicit.plots[0].sample,
        "fixture check: the two paths must have reached the same rate before their \
         notices can be compared"
    );

    let automatic_png = dir.join("automatic.png");
    let explicit_png = dir.join("explicit.png");
    capture_vello_only(automatic, 1.0, &automatic_png).expect("capture automatic");
    capture_vello_only(explicit, 1.0, &explicit_png).expect("capture forced");

    let (a, b) = (band_bytes(&automatic_png), band_bytes(&explicit_png));
    assert!(!a.is_empty(), "the band has pixels to compare");
    let differing = a.iter().zip(&b).filter(|(x, y)| x != y).count();
    assert_eq!(
        differing,
        0,
        "{differing} of {} band bytes differ between the automatic notice and the \
         forced one at the same modulus",
        a.len()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **`--force-sample` still outranks the policy, in both directions.**
///
/// It is how one spec produces a complete picture and a sampled one over the
/// same rows, which is the comparison the sign-off is judged on. A policy that
/// overrode a rate someone typed would take that away; one that could not
/// sample a plot below the ceiling would take away the other half.
#[test]
fn an_explicit_rate_outranks_the_policy() {
    let (small_dir, small) = fixture("explicit-below", &scatter(4096));
    let (big_dir, big) = fixture("explicit-above", &scatter(MEASURED_INKED_MAX + 1));

    let forced = SampleRate::from_modulus(8).expect("power of two");
    let below = compose_spec_sampled(small.to_str().expect("utf-8 path"), Some(forced))
        .expect("compose forced, below the ceiling");
    assert!(
        below.plots[0].sample.is_some(),
        "a rate named on the command line must apply to a plot the policy would \
         have left alone"
    );

    let coarser = SampleRate::from_exponent(
        sample_exponent(MEASURED_INKED_MAX + 1).expect("fixture check: needs a sample") + 3,
    )
    .expect("a legal rate");
    let above = compose_spec_sampled(big.to_str().expect("utf-8 path"), Some(coarser))
        .expect("compose forced, above the ceiling");
    let fact = above.plots[0]
        .sample
        .expect("a plot above the ceiling is sampled either way");
    let policy_drawn = compose_unflagged(&big).plots[0]
        .sample
        .expect("…and the policy's own answer is a sample too")
        .drawn;
    assert!(
        fact.drawn < policy_drawn,
        "the named rate drew {} rows and the policy's own answer draws {} — a \
         coarser rate someone typed must not be silently refined to the policy's",
        fact.drawn,
        policy_drawn
    );

    let _ = std::fs::remove_dir_all(&small_dir);
    let _ = std::fs::remove_dir_all(&big_dir);
}

/// **An aggregating plot is not sampled, and does not claim to be.**
///
/// The estimate counts row-level primitives, and an aggregating mark's rows
/// are bins: its picture is O(bins) at any table size, and the emitter refuses
/// the clause anyway. A policy that counted its rows would sample a plot that
/// never needed it, and — because the emitter drops the clause — would draw a
/// notice over a picture that had lost nothing.
#[test]
fn an_aggregating_plot_is_not_sampled_however_many_rows_it_summarises() {
    let source = format!(
        "data:
  points:
    query: |
      SELECT (i * 7919 % 1009) / 10.0 AS a
      FROM range({}) AS t(i)
plot:
  - mark: densityX
    data: {{ from: points }}
    x: a
width: {W}
height: {H}
",
        MEASURED_INKED_MAX * 4
    );
    let (dir, spec) = fixture("aggregating", &source);

    let composed = compose_unflagged(&spec);
    assert!(
        composed.plots[0].sample.is_none(),
        "an aggregating plot draws one primitive per BIN — sampling it is sampling \
         bins, and the notice would describe a loss that did not happen"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The committed ten-million-row scatter, at the magnitude the scale claim is
/// made at.
const TEN_MILLION: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/ten-million-scatter.yaml"
);

/// **Ten million rows, no flag, a picture with a notice on it.**
///
/// The magnitude is the point. The boundary cases above are specs written by
/// a test; this one is a file in `examples/`, opened the way the shell opens a
/// spec named on the command line — [`Boot::open`], the one place a spec is
/// classified — and then drawn through the headless layout pass the capture
/// tiers use.
///
/// Unsampled it would draw ten million primitives. What makes this the
/// criterion rather than a demo is that nothing in the file, on the command
/// line, or in this test asks for a sample.
#[test]
fn the_committed_ten_million_row_example_opens_and_samples_itself() {
    let boot = Boot::open(TEN_MILLION, Flow::Vertical, None).expect("the example opens");
    assert!(
        !boot.is_empty(),
        "the example loaded nothing — a boot with no document is not an open spec"
    );

    let composed = compose_unflagged(Path::new(TEN_MILLION));
    let fact = composed.plots[0]
        .sample
        .expect("the committed ten-million-row example must render SAMPLED under no flag");
    assert_eq!(
        fact.of, 10_000_000,
        "the example's row count moved; this criterion is about the magnitude"
    );
    assert!(
        renders_complete(fact.drawn),
        "the sample drew {} primitives, which is still past the ceiling",
        fact.drawn
    );
    // Not `drawn * modulus == of`: a hash sample is a predicate on the row's
    // own bytes, not a partition, so the surviving count is near `of / modulus`
    // rather than exactly it. What has to hold is that the picture fits.
    let modulus = 1_u64 << sample_exponent(fact.of).expect("ten million needs a sample");
    assert!(
        fact.drawn < fact.of,
        "a 1-in-{modulus} sample of {} rows drew {} — nothing was dropped",
        fact.of,
        fact.drawn
    );

    // The notice, in the exported chart's own ink.
    let dir = std::env::temp_dir().join(format!("bf-auto-sample-10m-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let png = dir.join("ten-million.png");
    assert_eq!((composed.width, composed.height), (W, H));
    capture_vello_only(composed, 1.0, &png).expect("capture");
    let band = ink_in_band(&png);
    assert!(
        band > 400,
        "the export's bottom band held {band} inked pixels — a reader shown a \
         sample is owed the sentence saying so"
    );

    // …and it draws in the shell, at the window the content asks for, rather
    // than only composing. Two frames, as the capture path runs: font atlas
    // then layout settle.
    let composed = compose_unflagged(Path::new(TEN_MILLION));
    let (w, h) = chart_window_size(&composed);
    let mut app = MeridianApp::headless(Boot::charts(composed), Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(w, h),
        )),
        ..Default::default()
    };
    for _ in 0..2 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    assert!(
        app.chart_doc().raster_rect.is_some_and(|r| r.area() > 0.0),
        "the chart pane recorded no raster — the example composed but did not draw"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **A categorical positional axis above the ceiling opens, and carries the
/// notice.**
///
/// The suite above this point was green on a tree where a plot with a band
/// scale did not open at all: the refusal that guards a sampled plot's
/// restorable domains fires per plot, the policy sets the rate without being
/// asked, and `Boot::open` propagates the error out of `main` — no window, no
/// PNG, exit 1. Nothing about the spec asks for a sample, and nothing about it
/// should have to.
#[test]
fn a_categorical_axis_above_the_ceiling_opens_and_samples_itself() {
    let (dir, spec) = fixture(
        "band-above",
        &categorical_scatter(MEASURED_INKED_MAX + 1, 12, "x"),
    );
    let path = spec.to_str().expect("utf-8 path");

    let boot = Boot::open(path, Flow::Vertical, None).expect(
        "a spec with a band positional scale must open with nothing on the command line — \
         the sample is the policy's decision, not the author's request",
    );
    assert!(!boot.is_empty(), "the spec loaded no document");

    let composed = compose_unflagged(&spec);
    let fact = composed.plots[0]
        .sample
        .expect("one primitive past the ceiling must render SAMPLED with no flag");
    assert_eq!(fact.of, MEASURED_INKED_MAX + 1);
    assert!(
        renders_complete(fact.drawn),
        "the sample drew {} primitives, which is still past the ceiling",
        fact.drawn
    );
    assert_eq!(
        band_order(&composed, Channel::X).len(),
        12,
        "the axis must carry every class the whole table holds, not the ones the \
         sample happened to keep"
    );

    // …and the notice is in the picture, which is the second half of what the
    // refusal took away: a render that does not happen carries no notice.
    let png = dir.join("band.png");
    assert_eq!((composed.width, composed.height), (W, H));
    capture_vello_only(composed, 1.0, &png).expect("capture");
    let band = ink_in_band(&png);
    assert!(
        band > 400,
        "the export's bottom band held {band} inked pixels — a reader shown a sample is \
         owed the sentence saying so"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **A sampled band axis lays its categories out where the complete one does.**
///
/// One spec, rendered both ways over the same rows, and the two category lists
/// compared as lists — which is order as well as membership, and order is the
/// quantity a band scale reads. A category's index in that list is the slot it
/// takes along the axis, so a list that differs by one entry slides every later
/// bar, under a notice that says only that rows were dropped.
///
/// The fixture is built to have teeth. The band is on **y**, so the wire name
/// the measurement is keyed by is not the one the x case would pass on. The
/// sample is forced coarse enough that it draws fewer rows than there are
/// classes — asserted below, and each drawn row carries one class, so the drawn
/// rows cannot cover them all. The drawn order is therefore short, and a
/// restoration that installed it, or that installed nothing, produces a
/// different list. The class names run descending, so one that sorted the list
/// produces a different list too.
#[test]
fn a_sampled_band_axis_lays_its_categories_out_where_the_complete_one_does() {
    const CLASSES: u64 = 200;
    let (dir, spec) = fixture("band-order", &categorical_scatter(6_400, CLASSES, "y"));
    let path = spec.to_str().expect("utf-8 path");

    let complete = compose_spec_sampled(path, None).expect("compose unflagged");
    assert!(
        complete.plots[0].sample.is_none(),
        "fixture check: the complete side must be below the ceiling, or it is not the \
         picture the sampled one is a sample OF"
    );

    let forced = SampleRate::from_modulus(64).expect("power of two");
    let sampled = compose_spec_sampled(path, Some(forced)).expect(
        "a plot with a band positional scale must compose under a sample rather than \
         refusing to draw",
    );
    let fact = sampled.plots[0]
        .sample
        .expect("fixture check: the forced rate must have applied");
    assert!(
        fact.drawn < CLASSES,
        "fixture check: the sample drew {} rows over {CLASSES} classes. It has to draw \
         fewer rows than there are classes for the drawn rows to be missing one, and \
         without that this compares two lists that agree by luck",
        fact.drawn
    );

    let expected = band_order(&complete, Channel::Y);
    assert_eq!(
        expected.len(),
        CLASSES as usize,
        "fixture check: the complete render must carry every class"
    );
    assert_ne!(
        expected,
        {
            let mut sorted = expected.clone();
            sorted.sort();
            sorted
        },
        "fixture check: the unsampled order must not already be the sorted one, or a \
         restoration that sorted the list would pass this"
    );
    assert_eq!(
        band_order(&sampled, Channel::Y),
        expected,
        "the sampled axis lays its categories out somewhere other than the complete \
         axis does — the same class is drawn in a different place under a notice that \
         says only that rows were dropped"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// -----------------------------------------------------------------------
// A settled navigation re-asks the policy
// -----------------------------------------------------------------------
//
// The mechanism above answers "what rate does N rows need"; these answer
// "what rate does the plot actually run at once a gesture has changed how
// many rows are inside its frame." Before this, the answer was frozen at
// open — `sample_policy::sample_exponent` was correct and tested, and the
// settled navigation re-query never called it a second time.

/// A row-level dot scatter whose x column is `0..rows` — unique per row, so a
/// navigation extent that names an interval on x selects an EXACT,
/// predictable row count rather than an estimate a reader would have to trust.
fn indexed_scatter(rows: u64) -> String {
    format!(
        "data:
  points:
    query: |
      SELECT CAST(i AS DOUBLE) AS gx, CAST(i AS DOUBLE) AS gy
      FROM range({rows}) AS t(i)
plot:
  - mark: dot
    data: {{ from: points }}
    x: gx
    y: gy
width: {W}
height: {H}
"
    )
}

/// **AC1: a settled navigation re-picks the modulus for the narrowed extent.**
///
/// Measured before this closed: a navigated 3,892,783-row extent drew 30,265
/// (1/128 — the rate the FULL 10,000,000-row table needed). A fresh open of a
/// 3,899,983-row extent drew 60,858 (1/64). Reproduced deterministically here:
/// the modulus a settled navigation lands on must equal the modulus a fresh
/// open of the identical row count lands on, on a case where that differs
/// from the open-time modulus by more than one step — so the two agreeing is
/// not available by coincidence.
#[test]
fn a_settled_navigation_repicks_the_modulus_for_the_narrowed_extent() {
    use brightfield_engine::coordinator::Interaction;
    use brightfield_engine::{AxisExtent, NavigationExtent};
    use brightfield_spec::analysis::ComponentPath;

    let full_rows = MEASURED_INKED_MAX * 40;
    let narrow_rows = MEASURED_INKED_MAX * 3;

    let (full_dir, full_spec) = fixture("navigate-full", &indexed_scatter(full_rows));
    let (narrow_dir, narrow_spec) = fixture("navigate-fresh", &indexed_scatter(narrow_rows));

    let (mut dash, first) = live_spec_sampled(full_spec.to_str().expect("utf-8 path"), None)
        .expect("live, unflagged");
    let open_rate = dash
        .coordinator()
        .session()
        .sample()
        .expect("fixture check: the full table must need a sample at open");
    assert_eq!(
        open_rate.exponent(),
        sample_exponent(full_rows).expect("fixture check: needs a sample"),
        "fixture check: the open-time rate must be the policy's answer for the full table"
    );

    let plot_path = first.plots[0].path.clone();
    let navigated = dash
        .apply(Interaction::Navigate {
            plot: ComponentPath(plot_path),
            extent: NavigationExtent {
                x: Some(AxisExtent::new("gx", 0.0, (narrow_rows - 1) as f64)),
                y: None,
            },
        })
        .expect("the navigation re-composites");

    let narrowed_rate = dash
        .coordinator()
        .session()
        .sample()
        .expect("fixture check: the narrowed extent must still need a sample");
    assert_ne!(
        narrowed_rate.exponent(),
        open_rate.exponent(),
        "fixture check: the narrowed extent's rate must differ from the open-time rate by \
         at least one step, or this case proves nothing about a re-pick happening"
    );

    let (mut fresh_dash, _fresh_first) =
        live_spec_sampled(narrow_spec.to_str().expect("utf-8 path"), None)
            .expect("live, unflagged, a fresh open of the identical row count");
    let fresh_rate = fresh_dash
        .coordinator()
        .session()
        .sample()
        .expect("fixture check: a fresh open of this row count must need a sample too");

    assert_eq!(
        narrowed_rate, fresh_rate,
        "a navigation gesture that narrowed the extent to {narrow_rows} rows chose 1-in-{} \
         while a fresh open of the identical {narrow_rows}-row table chose 1-in-{} — the \
         settled re-query must ask `sample_exponent` again for the row count now inside the \
         frame, not keep the modulus chosen for the whole table",
        narrowed_rate.modulus(),
        fresh_rate.modulus()
    );

    // And the plot's own sampling fact agrees with what the session chose —
    // the picture drawn is the picture the modulus says it is.
    let fact = navigated.plots[0]
        .sample
        .expect("the narrowed plot must still carry a sampling fact");
    assert_eq!(
        fact.of, narrow_rows,
        "`of` is the row count measured INSIDE the navigated extent"
    );

    let _ = std::fs::remove_dir_all(&full_dir);
    let _ = std::fs::remove_dir_all(&narrow_dir);
}

/// **AC3: zooming out re-coarsens by the same rule.**
///
/// A repair that only ever narrows the modulus to match whatever is currently
/// in view — but never widens it back out — is a one-way ratchet: the picture
/// gets finer forever and never returns to the rate the plot opened with.
/// Proven on the exact in-then-out pair `navigation::zoom`'s reciprocal step
/// produces, reading off the DISPLAYED scales at each step exactly as a real
/// gesture does (`navigation::pan`'s own doc: "the scales a plot was last
/// drawn with already carry whatever extent is in force").
#[test]
fn zooming_out_after_in_returns_the_rate_the_plot_opened_with() {
    use brightfield_engine::coordinator::Interaction;
    use brightfield_engine::{AxisExtent, NavigationExtent};
    use brightfield_shell::navigation::{self, AxisLock};
    use brightfield_spec::analysis::ComponentPath;

    let rows = 1_000_000u64;
    let (dir, spec) = fixture("zoom-round-trip", &indexed_scatter(rows));
    let path = spec.to_str().expect("utf-8 path");

    let (mut dash, first) = live_spec_sampled(path, None).expect("live, unflagged");
    let opened_rate = dash
        .coordinator()
        .session()
        .sample()
        .expect("fixture check: the full table must need a sample at open");

    let plot_path = first.plots[0].path.clone();
    let scales_full = first.plots[0].scales.clone();

    // In: a quarter of the frame, about its own centre (a keyboard zoom names
    // no cursor).
    let zoom_in = navigation::zoom(&scales_full, AxisLock::XOnly, None, 4.0);
    let (lo, hi) = zoom_in.extent.x.expect("x zoomed in");
    let narrowed = dash
        .apply(Interaction::Navigate {
            plot: ComponentPath(plot_path.clone()),
            extent: NavigationExtent {
                x: Some(AxisExtent::new("gx", lo, hi)),
                y: None,
            },
        })
        .expect("zoom in re-composites");

    let narrowed_rate = dash
        .coordinator()
        .session()
        .sample()
        .expect("fixture check: the zoomed-in extent must still need a sample");
    assert!(
        narrowed_rate < opened_rate,
        "fixture check: zooming in to a quarter of the rows must land on a FINER (smaller \
         modulus) rate than the full table's, or the round trip below proves nothing about \
         re-coarsening — opened at 1-in-{}, zoomed to 1-in-{}",
        opened_rate.modulus(),
        narrowed_rate.modulus()
    );

    // Out: the reciprocal step, read off the scales the FIRST zoom actually
    // left on the plot — not a remembered launch domain.
    let scales_narrowed = narrowed.plots[0].scales.clone();
    let zoom_out = navigation::zoom(&scales_narrowed, AxisLock::XOnly, None, 0.25);
    let (lo2, hi2) = zoom_out.extent.x.expect("x zoomed out");
    let widened = dash
        .apply(Interaction::Navigate {
            plot: ComponentPath(plot_path),
            extent: NavigationExtent {
                x: Some(AxisExtent::new("gx", lo2, hi2)),
                y: None,
            },
        })
        .expect("zoom out re-composites");

    let final_rate = dash
        .coordinator()
        .session()
        .sample()
        .expect("fixture check: the round trip must land back on a sampled extent");
    assert_eq!(
        final_rate, opened_rate,
        "a zoom in then its reciprocal zoom out chose 1-in-{} where the plot opened at \
         1-in-{} — the round trip left it stuck at the finer, zoomed-in rate instead of \
         re-coarsening back to the rate the plot opened with",
        final_rate.modulus(),
        opened_rate.modulus()
    );

    let fact = widened.plots[0]
        .sample
        .expect("the widened plot must still carry a sampling fact");
    assert_eq!(
        fact.of, rows,
        "the round trip must land back on the full, un-navigated row count"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **AC4: zooming in densifies; it does not reshuffle.**
///
/// The nesting property [`TEN_MILLION`]'s own `description` states: "every
/// point drawn here is also drawn at every coarser rate." The mechanism is a
/// hash predicate on the row's own bytes (`hash(_s) % modulus = 0`, see
/// `render.rs`), which nests for any two power-of-two moduli independently of
/// which extent the query happens to be scoped to — so the claim has to keep
/// holding across a rate CHANGE a navigation triggers, not only across one
/// made by hand with `--force-sample`.
///
/// The coarser rate's drawn rows, over the SAME navigated extent, are the
/// control this test compares against: forcing the pre-zoom rate back on and
/// re-querying through the identical machinery, rather than reconstructing the
/// predicate by hand and risking a second, independently-wrong copy of it.
#[test]
fn zooming_in_keeps_every_row_the_coarser_rate_drew() {
    use arrow::array::Float64Array;
    use brightfield_engine::coordinator::Interaction;
    use brightfield_engine::{AxisExtent, NavigationExtent};
    use brightfield_shell::navigation::{self, AxisLock};
    use brightfield_spec::analysis::ComponentPath;
    use std::collections::HashSet;

    fn drawn_row_identities(dash: &mut brightfield_shell::pipeline::LiveDashboard) -> HashSet<(u64, u64)> {
        let batches = dash.coordinator().chart_rows(0).expect("chart rows");
        let mut set = HashSet::new();
        for batch in &batches {
            let spread_idx = batch.schema().index_of("spread").expect("spread column");
            let depth_idx = batch.schema().index_of("depth").expect("depth column");
            let spread = batch.column(spread_idx)
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("f64 spread");
            let depth = batch.column(depth_idx)
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("f64 depth");
            for i in 0..batch.num_rows() {
                set.insert((spread.value(i).to_bits(), depth.value(i).to_bits()));
            }
        }
        set
    }

    let (mut dash, first) = live_spec_sampled(TEN_MILLION, None).expect("live, unflagged");
    let coarse_rate = dash
        .coordinator()
        .session()
        .sample()
        .expect("fixture check: the committed ten-million-row example must sample itself");

    let plot_path = first.plots[0].path.clone();
    let scales_full = first.plots[0].scales.clone();
    let zoom_in = navigation::zoom(&scales_full, AxisLock::XOnly, None, 4.0);
    let (lo, hi) = zoom_in.extent.x.expect("x zoomed in");

    dash.apply(Interaction::Navigate {
        plot: ComponentPath(plot_path),
        extent: NavigationExtent {
            x: Some(AxisExtent::new("spread", lo, hi)),
            y: None,
        },
    })
    .expect("zoom in re-composites");

    let fine_rate = dash
        .coordinator()
        .session()
        .sample()
        .expect("fixture check: the zoomed-in extent must still need a sample");
    assert!(
        fine_rate < coarse_rate,
        "fixture check: zooming in to a quarter of the rows must pick a FINER rate, or this \
         test cannot show nesting — opened at 1-in-{}, zoomed to 1-in-{}",
        coarse_rate.modulus(),
        fine_rate.modulus()
    );

    let fine_rows = drawn_row_identities(&mut dash);
    assert!(
        !fine_rows.is_empty(),
        "fixture check: the finer, zoomed-in rate must still draw something"
    );

    // The control: force the coarser (pre-zoom) rate back on, over the SAME
    // navigated extent.
    dash.set_sample(Some(coarse_rate));
    let coarse_rows = drawn_row_identities(&mut dash);
    assert!(
        !coarse_rows.is_empty(),
        "fixture check: the coarser rate must still draw something over the narrowed extent"
    );

    let missing = coarse_rows.difference(&fine_rows).count();
    assert_eq!(
        missing, 0,
        "{missing} of {} rows the coarser 1-in-{} rate drew over the navigated extent are \
         absent from the finer 1-in-{} rate's drawing of the SAME extent — the picture \
         reshuffled instead of densifying when navigation changed the rate",
        coarse_rows.len(),
        coarse_rate.modulus(),
        fine_rate.modulus()
    );
}
