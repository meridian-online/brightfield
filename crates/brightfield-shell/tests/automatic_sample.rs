//! **Nothing had to be told.** The sampling policy, held to the boundary it
//! draws and to the query it issues.
//!
//! The mechanism this exercises shipped with a manual driver: `--force-sample`
//! on either binary and nothing else. Above the renderer's drawn-primitive
//! ceiling the failure is a blank frame at exit 0 — no error, no diagnostic,
//! a written PNG — so a plot nobody thought to flag drew nothing and said
//! nothing about it. These are the assertions that a plot now decides for
//! itself.
//!
//! Every case is built at a row count taken FROM the ceiling constant rather
//! than written out here, so moving the constant moves the fixtures with it
//! instead of leaving a suite testing a boundary the product no longer draws.

use std::path::{Path, PathBuf};

use brightfield_render::sample_notice::NOTICE_BAND;
use brightfield_render::sample_policy::{renders_complete, sample_exponent, MEASURED_INKED_MAX};
use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::pipeline::{compose_spec_sampled, live_spec_sampled, Composed};
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
/// measured to ink a frame, and one primitive more is a frame that would come
/// back blank. A policy that compared the wrong way round, or that used a
/// different threshold from the one the constant names, cannot satisfy both
/// rows — which is what makes this a boundary test rather than two spot checks.
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
    let fact = sampled.plots[0].sample.expect(
        "one primitive past the ceiling must render SAMPLED with no flag — unsampled \
         it comes back blank at exit 0, which is the failure nothing reports",
    );
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
/// The decision cannot be made after the fact — above the ceiling there is no
/// error to catch and retry, and a full result set materialised and then
/// thrown away costs the memory the push-down exists to avoid. So: one execute
/// per row-level mark, that execute's SQL carrying the sample clause, and the
/// rows it materialised being the sampled count.
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
    let executed = session.executed_sql();
    assert_eq!(
        executed.len(),
        1,
        "one mark, one statement — got {executed:#?}"
    );
    let sql = &executed[0];
    let modulus =
        1_u32 << sample_exponent(fact.of).expect("fixture check: this count needs a sample");
    assert!(
        sql.contains(&format!("hash(_s) % {modulus} = 0")),
        "the executed SQL does not carry the sample predicate at the chosen \
         modulus:\n{sql}"
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
