//! Gate: a transform on a channel we cannot compute is SAID, not swallowed.
//!
//! `x: { bin: delay }` + `y: { count: }` is the commonest histogram idiom in
//! the Mosaic corpus, and brightfield computes neither half. Until this gate
//! went in the whole thing was silent in a way nothing else is: an unknown
//! mark errors, an unconsumed mark *option* is named, an unsubscribed param is
//! named — but a spec whose positional channels asked to be computed parsed
//! clean, exited zero, and drew an empty frame.
//!
//! It was silent for a findable reason. `x` and `y` are on
//! `CONSUMED_MARK_OPTION_KEYS`, correctly — the renderer reads them. The
//! key-level check therefore passed, and nothing looked at the value SHAPE.
//!
//! Every proof below is a vendored, byte-for-byte unmodified upstream Mosaic
//! spec, so what they assert is portability truth rather than a fixture
//! written to match the implementation.

use std::collections::BTreeSet;

use brightfield_spec::{parse_spec, Format, ParseWarning};

/// The `channel:transform` pairs a spec's parse reports as uncomputable.
fn transforms(source: &str) -> BTreeSet<String> {
    let out = parse_spec(source, Format::Yaml).expect("spec parses");
    out.warnings
        .iter()
        .filter_map(|w| match w {
            ParseWarning::UnconsumedChannelTransform { channel, transform } => {
                Some(format!("{channel}:{transform}"))
            }
            _ => None,
        })
        .collect()
}

/// The rendered lines, so a test can assert on what a person reads rather
/// than on which variant was constructed.
fn lines(source: &str) -> Vec<String> {
    let out = parse_spec(source, Format::Yaml).expect("spec parses");
    out.warnings
        .iter()
        .filter(|w| matches!(w, ParseWarning::UnconsumedChannelTransform { .. }))
        .map(std::string::ToString::to_string)
        .collect()
}

/// `flights-200k.yaml` is the canonical cross-filter histogram: three `rectY`
/// plots, each binning a column on `x` and counting on `y`. Brightfield draws
/// nothing for any of them. Both halves of the idiom are named.
#[test]
fn flights_200k_bin_and_count_are_named() {
    let found = transforms(include_str!(
        "../vendor/mosaic-specs/yaml/flights-200k.yaml"
    ));
    assert!(
        found.contains("x:bin"),
        "the uncomputed `bin` on `x` must be named: {found:?}"
    );
    assert!(
        found.contains("y:count"),
        "the uncomputed `count` on `y` must be named: {found:?}"
    );
}

/// The diagnostic names the CHANNEL and the TRANSFORM, and says what it cost.
///
/// Asserted on the rendered message, not on the variant: AC2 of the card is
/// that this test fails if the diagnostic is removed, and a `matches!` on the
/// enum would keep passing against a variant whose `Display` had been gutted.
#[test]
fn the_message_names_the_channel_the_transform_and_the_cost() {
    let rendered = lines(include_str!(
        "../vendor/mosaic-specs/yaml/flights-200k.yaml"
    ));
    assert!(
        !rendered.is_empty(),
        "flights-200k bins on `x` and counts on `y`; both must produce a line"
    );
    let bin = rendered
        .iter()
        .find(|l| l.contains("`bin`"))
        .unwrap_or_else(|| panic!("no line mentions `bin`: {rendered:?}"));
    assert!(
        bin.contains("channel `x`"),
        "the line must name the channel so a six-mark dashboard is readable: {bin}"
    );
    assert!(
        bin.contains("does not compute"),
        "the line must say brightfield cannot compute it: {bin}"
    );
    assert!(
        bin.contains("draws no ink"),
        "the line must say what it cost — the author is looking at a blank \
         frame and needs this line to account for the whole of it: {bin}"
    );
}

/// A `{sql: …}` channel expression IS lowered, and a `{param: …}` channel IS
/// bound. Neither may be reported: a diagnostic that fires on working specs
/// is one readers learn to skip, and both shapes are single-key maps on a
/// rendered channel, so both are exactly the near-miss this check must not
/// take.
#[test]
fn computed_and_bound_channels_stay_quiet() {
    let sql = transforms(
        r"
data:
  quakes: { file: data/earthquakes.parquet }
params:
  size: 4
plot:
- mark: dot
  data: { from: quakes }
  x: longitude
  y: latitude
  r: { sql: 'POW(10, magnitude)' }
  stroke: { param: size }
",
    );
    assert!(
        sql.is_empty(),
        "a `sql:` expression channel and a `param:` channel are both read — \
         neither is an uncomputed transform: {sql:?}"
    );
}

/// An aggregate on an aggregate-capable channel is consumed, and an
/// unrecognised one there already earns `UnknownAggregate`. Neither may also
/// earn this warning: two lines about one key reads as two problems.
#[test]
fn aggregate_capable_channels_are_not_reported_twice() {
    let out = parse_spec(
        r"
data:
  quakes: { file: data/earthquakes.parquet }
plot:
- mark: hexbin
  data: { from: quakes }
  x: longitude
  y: latitude
  fill: { count: }
",
        Format::Yaml,
    )
    .expect("spec parses");
    let doubled: Vec<&ParseWarning> = out
        .warnings
        .iter()
        .filter(|w| {
            matches!(
                w,
                ParseWarning::UnconsumedChannelTransform { channel, .. } if channel == "fill"
            )
        })
        .collect();
    assert!(
        doubled.is_empty(),
        "`fill: {{count:}}` is a consumed aggregate and must not be reported \
         as an uncomputed transform: {doubled:?}"
    );
}

/// The corpus regression. Every vendored spec that brightfield renders
/// end to end must stay quiet on this channel — a check whose predicate is
/// too wide would light up specs that work, and the whole corpus is the only
/// place that shows.
///
/// Pinned by NAME rather than by count: a bare count passes just as happily
/// when the set changes underneath it, which is the failure this file exists
/// to stop.
#[test]
fn only_the_specs_that_bin_or_count_positionally_are_reported() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/mosaic-specs/yaml");
    let mut speaking: BTreeSet<String> = BTreeSet::new();
    for entry in std::fs::read_dir(&dir).expect("vendored corpus is present") {
        let path = entry.expect("readable entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("readable spec");
        // A spec that does not parse is a different test's problem.
        let Ok(out) = parse_spec(&source, Format::Yaml) else {
            continue;
        };
        if out
            .warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::UnconsumedChannelTransform { .. }))
        {
            speaking.insert(
                path.file_name()
                    .expect("named file")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    let expected: BTreeSet<String> = EXPECTED_SPEAKING.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(
        speaking, expected,
        "the set of vendored specs reporting an uncomputed channel transform \
         changed. A spec that APPEARED is either a real new finding or this \
         check reaching too far; a spec that VANISHED means the diagnostic \
         stopped firing where it used to."
    );
}

/// The vendored specs that ask for a channel transform brightfield does not
/// compute — 18 of 54, each drawing a blank or a partial frame today, every
/// one of them silent before this gate.
///
/// It was 16 until the check stopped skipping multi-key maps. `moving-average`
/// and `window-frame` join on the windowed-average shape
/// (`{avg: …, orderby: …, rows: …}`), and `protein-design` — already listed —
/// went from a half-report to naming both of its marks' channels.
///
/// The transforms behind them, so a reader knows what the list is made of
/// without re-running it: the histogram idiom (`bin`, `count`), positional
/// aggregates (`sum`, `avg`, `min`, `max`), date binning (`dateMonth`,
/// `dateMonthDay`), geo centroids (`centroidX`, `centroidY`) and the
/// param-driven column selector (`column`, as `x: { column: $x }` — the
/// dropdown in `symbols.yaml` chooses a column nothing then resolves).
///
/// This list SHRINKING is the point: each entry that leaves is a capability
/// that landed. Nothing here is a defect in this check.
const EXPECTED_SPEAKING: &[&str] = &[
    "athlete-birth-waffle.yaml",
    "athlete-height.yaml",
    "crossfilter.yaml",
    "flights-10m.yaml",
    "flights-200k.yaml",
    "flights-hexbin.yaml",
    "gaia.yaml",
    "moving-average.yaml",
    "nyc-taxi-rides.yaml",
    "observable-latency.yaml",
    "protein-design.yaml",
    "seattle-temp.yaml",
    "sorted-bars.yaml",
    "symbols.yaml",
    "us-county-map.yaml",
    "us-state-map.yaml",
    "weather.yaml",
    "window-frame.yaml",
];

/// A transform carrying modifiers is still a transform, and both halves of a
/// two-part failure get named.
///
/// `protein-design.yaml` is the case that forced this. One `rectY` binds
/// `x: { bin: plddt_total, steps: 60 }` and `y: { count: }`. A check that only
/// looked at single-key maps named the `count` and said nothing about the
/// `bin` — so an author who fixed the half they were told about was still
/// looking at a blank frame with nothing left to explain it. **A partial
/// diagnostic on a two-part failure is a wrong diagnostic**, and it is worse
/// than silence because it looks like the whole answer.
#[test]
fn a_transform_with_modifiers_beside_it_is_still_named() {
    let found = transforms(include_str!(
        "../vendor/mosaic-specs/yaml/protein-design.yaml"
    ));
    assert!(
        found.contains("x:bin"),
        "`x: {{ bin: …, steps: 60 }}` carries a modifier, and is still an \
         uncomputed bin: {found:?}"
    );
    assert!(
        found.contains("y:count"),
        "the `count` half was already named and must stay named: {found:?}"
    );
    // The windowed-average shape from a different file, so this does not pass
    // on one spec's quirk.
    let ma = transforms(include_str!(
        "../vendor/mosaic-specs/yaml/moving-average.yaml"
    ));
    assert!(
        ma.contains("y:avg"),
        "`y: {{ avg: cases, orderby: day, rows: $frame }}` is a windowed average \
         brightfield does not compute: {ma:?}"
    );
}

/// The false-positive gate, and the one that matters most.
///
/// `examples/` is brightfield's OWN corpus — every spec in it is one we ship
/// and render. If this diagnostic fires on any of them it is wrong, because a
/// warning on a chart that draws correctly is how a diagnostic channel earns
/// the habit of being ignored.
///
/// The near-miss it has to survive is real: `examples/hexbin.yaml` carries
/// `fill: { count: }`, a single-key transform map on a rendered channel, and
/// it is consumed. The check must stay quiet on it for the right reason —
/// `fill` is aggregate-capable and spoken for upstream — rather than by
/// accident.
#[test]
fn brightfields_own_examples_stay_silent() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("examples");
    let mut speaking: Vec<String> = Vec::new();
    let mut checked = 0usize;
    visit(&root, &mut |path: &std::path::Path| {
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            return;
        }
        let Ok(source) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(out) = parse_spec(&source, Format::Yaml) else {
            return;
        };
        checked += 1;
        for w in &out.warnings {
            if let ParseWarning::UnconsumedChannelTransform { channel, transform } = w {
                speaking.push(format!(
                    "{}: {channel}:{transform}",
                    path.file_name().expect("named file").to_string_lossy()
                ));
            }
        }
    });
    // A zero-file walk would pass this assertion while proving nothing — the
    // exact shape of a structural guard passing on broken code. Pinned at the
    // real count rather than a loose floor: `examples/` holds 38 `.yaml`, of
    // which 36 parse as Mosaic specs and 2 are protocol manifests with no plot
    // discriminator (`protocol/degrade.yaml`, `protocol/edgar_gleif/arcform.yaml`).
    // A floor of 30 would let six examples silently stop parsing while this
    // gate still reported the corpus clean.
    assert!(
        checked >= 36,
        "expected to walk brightfield's own example corpus; only {checked} of \
         the 36 parseable specs were read, so this gate proved less than it claims"
    );
    assert!(
        speaking.is_empty(),
        "this diagnostic fired on specs brightfield renders itself, which \
         means the check reaches too far: {speaking:?}"
    );
}

/// Walk a directory tree, calling `f` on every file. Recurses so the
/// `protocol/` and `live/` subdirectories of `examples/` are covered too.
fn visit(dir: &std::path::Path, f: &mut impl FnMut(&std::path::Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit(&path, f);
        } else {
            f(&path);
        }
    }
}
