//! Gate: a transform on a channel we cannot compute is SAID, not swallowed.
//!
//! An unknown mark errors, an unconsumed mark *option* is named, an
//! unsubscribed param is named — but until this gate went in, a spec whose
//! positional channels asked to be computed parsed clean, exited zero and drew
//! an empty frame. It was silent for a findable reason: `x` and `y` are on
//! `CONSUMED_MARK_OPTION_KEYS`, correctly — the renderer reads them — so the
//! key-level check passed and nothing looked at the value SHAPE.
//!
//! **The list this file pins is meant to SHRINK, and it just did.**
//! `x: {bin: delay}` + `y: {count:}` is the commonest histogram idiom in the
//! Mosaic corpus and brightfield now computes it, so the six vendored specs
//! that only ever asked for that idiom went quiet in the same commit that made
//! them draw. A gate that kept warning about a chart that now renders would be
//! the same defect from the other side.
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
/// plots, each binning a column on `x` and counting on `y`. Brightfield now
/// COMPUTES that idiom, so the diagnostic must be gone from it entirely.
///
/// This test used to assert the opposite, and inverting it is the point: the
/// capability and the diagnostic move together or the product lies in one
/// direction or the other. A warning on a chart that draws is how a diagnostic
/// channel earns the habit of being ignored.
#[test]
fn flights_200k_computes_and_no_longer_speaks() {
    let found = transforms(include_str!(
        "../vendor/mosaic-specs/yaml/flights-200k.yaml"
    ));
    assert!(
        found.is_empty(),
        "every channel transform in flights-200k is a positional bin or count, \
         and both are computed — nothing here may still warn: {found:?}"
    );
}

/// The diagnostic names the CHANNEL and the TRANSFORM, and says what it cost.
///
/// Asserted on the rendered message, not on the variant: a `matches!` on the
/// enum would keep passing against a variant whose `Display` had been gutted.
///
/// Read off `protein-design.yaml`, which is where the histogram idiom still
/// goes dark: its binned `rectY` binds `fill: version`, a GROUPING colour that
/// Mosaic stacks. Brightfield does not stack yet, and merging the groups would
/// draw one bar per bin at the right TOTAL with every version's share
/// invisible — so the lift is refused and the line stays.
#[test]
fn the_message_names_the_channel_the_transform_and_the_cost() {
    let rendered = lines(include_str!(
        "../vendor/mosaic-specs/yaml/protein-design.yaml"
    ));
    assert!(
        !rendered.is_empty(),
        "protein-design bins on `x` and counts on `y` under a grouping fill; \
         both must still produce a line"
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

/// A `{param: …}` channel IS bound, and a `{sql: …}` expression channel is
/// carried as an ordinary object rather than mistaken for a typo'd aggregate.
/// Neither may be reported here: both shapes are single-key maps on a rendered
/// channel, so both are exactly the near-miss this check must not take.
///
/// The `sql:` half is an *exclusion*, not a claim that the expression is
/// lowered — nothing in `brightfield-sql` or `brightfield-render` reads `sql`
/// today. An earlier version of this docstring said it "IS lowered"; that was
/// wrong, and it matters, because it is the reason `athlete-birth-waffle.yaml`
/// cannot go quiet even in principle.
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

    // The multi-key case, which the widening brought into scope. It DOES reach
    // the channel-transform check, because `maybe_aggregate_channel` bails on
    // `m.len() != 1` — so the thing to pin is that it earns exactly ONE line,
    // not that it earns none. Without this the test's name overpromises: it
    // would only ever have exercised the single-key shape.
    let multi = parse_spec(
        r"
data:
  quakes: { file: data/earthquakes.parquet }
plot:
- mark: dot
  data: { from: quakes }
  x: longitude
  y: latitude
  fill: { avg: magnitude, orderby: time }
",
        Format::Yaml,
    )
    .expect("spec parses");
    let about_fill: Vec<String> = multi
        .warnings
        .iter()
        .filter(|w| {
            matches!(w, ParseWarning::UnconsumedChannelTransform { channel, .. } if channel == "fill")
                || matches!(w, ParseWarning::UnknownAggregate { field, .. } if field == "fill")
        })
        .map(std::string::ToString::to_string)
        .collect();
    assert_eq!(
        about_fill.len(),
        1,
        "a multi-key map on an aggregate-capable channel must earn exactly one \
         line — two would read as two problems: {about_fill:?}"
    );
    assert!(
        about_fill[0].contains("`avg`"),
        "and it should name the transform the author wrote: {about_fill:?}"
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
/// compute — **12** of 54, each drawing a blank or a partial frame, every one
/// of them silent before this gate.
///
/// It was 18. **Six left when the positional bin+count landed**: `crossfilter`,
/// `flights-200k`, `flights-10m`, `gaia`, `nyc-taxi-rides` and
/// `flights-hexbin`, whose two binned rects take both orientations. Each of
/// those now draws a histogram, which is the only reason it is allowed to stop
/// warning.
///
/// The transforms behind what remains, so a reader knows what the list is made
/// of without re-running it: positional aggregates (`sum`, `avg`, `min`,
/// `max`), the windowed average (`{avg: …, orderby: …, rows: …}`), date binning
/// (`dateMonth`, `dateMonthDay`), geo centroids (`centroidX`, `centroidY`), the
/// param-driven column selector (`column`, as `x: { column: $x }` — the
/// dropdown in `symbols.yaml` chooses a column nothing then resolves), and one
/// entry that is still the histogram idiom: `protein-design`, whose binned
/// rects carry `fill: version`. Mosaic STACKS a binned rect with a grouping
/// colour and brightfield does not yet, so the lift is refused and the
/// diagnostic is the truth about that chart.
///
/// This list SHRINKING is the point: each entry that leaves is a capability
/// that landed. Nothing here is a defect in this check.
const EXPECTED_SPEAKING: &[&str] = &[
    "athlete-birth-waffle.yaml",
    "athlete-height.yaml",
    "moving-average.yaml",
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
/// `protein-design.yaml` is the case that forced this and is still the case
/// that proves it — though for a different reason than when it was written.
/// Its `x: { bin: plddt_total, steps: 60 }` is now a shape the lowerer honours
/// modifier and all; what keeps the spec dark is `fill: version` on the same
/// mark. Both halves must still be named: an author who fixed only the half
/// they were told about would be back at a blank frame with nothing left to
/// account for it, and **a partial diagnostic on a two-part failure is a wrong
/// diagnostic** — worse than silence, because it looks like the whole answer.
#[test]
fn a_transform_with_modifiers_beside_it_is_still_named() {
    let found = transforms(include_str!(
        "../vendor/mosaic-specs/yaml/protein-design.yaml"
    ));
    assert!(
        found.contains("x:bin"),
        "the refused bin must still be named: {found:?}"
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

/// The line the whole `bin`+`count` lift walks: a colour CONSTANT on the fill
/// is a plain histogram and computes; a COLUMN on the fill is a stack, which
/// brightfield does not draw, so the pair stays uncomputed and keeps saying so.
///
/// Two identical specs but for that one word. If the predicate ever collapses
/// — every fill treated as a constant, or every fill treated as a column — one
/// half of this fails, which is the only cheap way to notice. The failure the
/// COLUMN half prevents is the expensive one: a merged histogram draws bars of
/// exactly the right total height in a single colour and looks entirely
/// correct while having discarded the grouping the author asked for.
#[test]
fn a_grouping_fill_refuses_the_lift_and_a_colour_constant_takes_it() {
    let histogram = |fill: &str| {
        transforms(&format!(
            "data:\n  flights: {{ file: data/flights.parquet }}\n\
             plot:\n- mark: rectY\n  data: {{ from: flights }}\n  \
             x: {{ bin: delay }}\n  y: {{ count: }}\n  fill: {fill}\n"
        ))
    };
    assert!(
        histogram("steelblue").is_empty(),
        "`fill: steelblue` is a CSS colour keyword, so the rect carries no \
         groups and its bin+count is computed"
    );
    assert!(
        histogram("'#4682b4'").is_empty(),
        "a hex colour is a constant too"
    );
    let grouped = histogram("version");
    assert!(
        grouped.contains("x:bin") && grouped.contains("y:count"),
        "`fill: version` names a column, which Mosaic stacks — the pair must \
         stay uncomputed and both halves must be named: {grouped:?}"
    );
    // `z` is Mosaic's explicit grouping channel and refuses on its own, so a
    // spec that grouped by `z` while colouring by a constant cannot slip past.
    let z_grouped = transforms(
        "data:
  flights: { file: data/flights.parquet }
plot:
- mark: rectY
  data: { from: flights }
  x: { bin: delay }
  y: { count: }
  z: version
  fill: steelblue
",
    );
    assert!(
        z_grouped.contains("x:bin"),
        "an explicit `z:` grouping refuses the lift whatever the fill is: {z_grouped:?}"
    );
}

/// The trap the lift was designed around: **positional aggregates other than
/// the histogram `count` are still uncomputed, and still say so.**
///
/// The cheap way to make `y: {count:}` parse would have been to add `x` and `y`
/// to the aggregate-capable channel list. That one line would also have
/// consumed `weather.yaml`'s `x: {count:}` on a `barX`, `sorted-bars.yaml`'s
/// `x: {sum: gold}` and `seattle-temp.yaml`'s `y1: {max: …}` — none of which
/// any lowerer computes. Every one would have gone quiet while still drawing
/// nothing: this milestone's defect arriving through the back door.
#[test]
fn positional_aggregates_no_lowerer_computes_still_speak() {
    let weather = transforms(include_str!("../vendor/mosaic-specs/yaml/weather.yaml"));
    assert!(
        weather.contains("x:count"),
        "`x: {{count:}}` on a barX is not the rect histogram idiom and nothing \
         computes it: {weather:?}"
    );
    let sorted = transforms(include_str!("../vendor/mosaic-specs/yaml/sorted-bars.yaml"));
    assert!(
        sorted.contains("x:sum"),
        "a positional `sum` has no lowerer: {sorted:?}"
    );
    let seattle = transforms(include_str!(
        "../vendor/mosaic-specs/yaml/seattle-temp.yaml"
    ));
    assert!(
        seattle.iter().any(|t| t.ends_with(":max")),
        "a positional `max` has no lowerer: {seattle:?}"
    );
    // And a `count` with no `bin` opposite it has nothing to group by, so it
    // is not the idiom either — the pair is what makes it computable.
    let lone_count = transforms(
        "data:
  flights: { file: data/flights.parquet }
plot:
- mark: rectY
  data: { from: flights }
  x: delay
  y: { count: }
",
    );
    assert!(
        lone_count.contains("y:count"),
        "a `count` with no positional `bin` opposite it is uncomputed: {lone_count:?}"
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
