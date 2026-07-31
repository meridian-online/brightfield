//! Gate: a colour LITERAL in a spec reaches the canvas **as that colour**.
//!
//! `fill: steelblue` drew Harbour blue. So did `fill: crimson`, and so did
//! `fill: '#ccc'`. Every structural check passed the whole time — the string
//! parsed, `is_colour_literal` classified it correctly, the binned rect lifted
//! on the strength of that classification, the SQL emitted, the scene encoded,
//! the process exited 0 and wrote a valid PNG with bars in it. The chart looked
//! finished. It was simply never the colour the author asked for.
//!
//! That is why every assertion here is a PIXEL COUNT off an exported picture.
//! A test that asked whether the channel map held a colour would pass on a
//! build that recorded the constant and never read it; one that asked
//! `parse_colour_literal` for `steelblue` would pass with the render path
//! untouched. Only the picture can tell the difference, and this whole defect
//! existed *because* nobody had looked at one.
//!
//! Each case asserts **two** things, and the pair is what makes it a gate:
//!
//! 1. the named colour appears on the page in an amount the same chart drawn
//!    with no colour channel does NOT already have, and
//! 2. **no** default mark ink is left on the page.
//!
//! (1) is measured as a difference against a control render rather than as a
//! raw count, because the picture is not empty: gridlines are `#d4d2cf` and a
//! spec asking for `#ccc` — which a curated spec does — sits inside the
//! anti-aliasing tolerance of them. A raw count would let that case pass on
//! chrome the mark never touched.
//!
//! (2) is the half that reddens hardest. Before the keyword table landed, a
//! `fill: crimson` chart held zero crimson pixels and a full set of bars in
//! default blue — both halves inverted. A change that resolves the constant but
//! forgets to *use* it fails (1); one that paints it somewhere harmless while
//! the marks keep their old ink fails (2).

use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::pipeline::compose_spec_str;
use std::path::{Path, PathBuf};

/// Per-channel tolerance: the shape's core and the first ring of its
/// anti-aliasing. Matches `bar_orientation.rs` and `binned_histogram.rs`, which
/// measure ink the same way.
const TOL: i32 = 20;

/// How much of a colour counts as "the mark is drawn in it", over and above
/// what the same chart holds without a colour channel.
///
/// Every fixture below draws a filled area or a 2 px stroke across a 640×400
/// plot at 2× device scale, so the true counts run from ~6,000 (a line) to
/// ~160,000 (a histogram). A thousand pixels is far below any real mark and far
/// above the anti-aliasing spill a neighbouring hue contributes.
const INK_FLOOR: u64 = 1_000;

/// CSS steelblue — the keyword six curated specs write, and the one the
/// corpus's portability claim actually rests on.
const STEELBLUE: [u8; 3] = [0x46, 0x82, 0xb4];

/// The ink a mark takes when its spec names no colour at all. Read from the
/// token layer, so a palette bump moves this with the picture.
fn default_mark_ink() -> [u8; 3] {
    let c = meridian_design::viz::MARK_DEFAULT_LIGHT;
    [
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
    ]
}

/// Compose a spec source and export it, returning the PNG path.
fn export(name: &str, source: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("bf-colour-literals");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let png = dir.join(format!("{name}.png"));
    let composed = compose_spec_str(source, None)
        .unwrap_or_else(|e| panic!("the {name} fixture must compose: {e}\n{source}"));
    capture_vello_only(composed, 1.0, &png).expect("export");
    png
}

/// Pixels within [`TOL`] of `want` on every channel.
fn pixels_near(png: &Path, want: [u8; 3]) -> u64 {
    let img = image::open(png).expect("open png").to_rgba8();
    img.pixels()
        .filter(|p| (0..3).all(|c| (i32::from(p.0[c]) - i32::from(want[c])).abs() <= TOL))
        .count() as u64
}

/// A chart fixture: one spec shape, with the colour slot left for the caller.
///
/// Held as a function rather than a string so every case can render its own
/// CONTROL — the identical chart with the colour line removed — which is what
/// makes the measurement a difference rather than a raw count.
type Fixture = fn(&str) -> String;

/// A bar chart over inline rows.
fn bars(colour_line: &str) -> String {
    format!(
        "data:\n  d:\n    - {{ u: A, v: 3 }}\n    - {{ u: B, v: 5 }}\n    \
         - {{ u: C, v: 2 }}\n    - {{ u: D, v: 6 }}\nplot:\n  - mark: barY\n    \
         data: {{ from: d }}\n    x: u\n    y: v\n{colour_line}\nwidth: 640\n\
         height: 400\n"
    )
}

/// A line chart over inline rows — the stroke side of the same question, and a
/// renderer that never consulted a colour channel at all: it took the default
/// ink directly, so "the channel map holds the constant" could be true here
/// while the picture stayed wrong.
fn line(colour_line: &str) -> String {
    format!(
        "data:\n  d:\n    - {{ i: 0, v: 1 }}\n    - {{ i: 1, v: 4 }}\n    \
         - {{ i: 2, v: 2 }}\n    - {{ i: 3, v: 6 }}\n    - {{ i: 4, v: 3 }}\n    \
         - {{ i: 5, v: 7 }}\nplot:\n  - mark: line\n    data: {{ from: d }}\n    \
         x: i\n    y: v\n{colour_line}\nwidth: 640\nheight: 400\n"
    )
}

/// The whole assertion for one fixture and one colour line: the named ink is on
/// the page in an amount the control does not have, and the default ink is gone.
fn assert_painted(name: &str, fixture: Fixture, colour_line: &str, want: [u8; 3]) {
    let control = export(&format!("{name}-control"), &fixture(""));
    let png = export(name, &fixture(colour_line));

    let baseline = pixels_near(&control, want);
    let got = pixels_near(&png, want);
    let gained = got.saturating_sub(baseline);
    assert!(
        gained >= INK_FLOOR,
        "{name}: `{colour_line}` gained {gained} px of \
         #{:02x}{:02x}{:02x} over the same chart with no colour channel \
         ({got} vs {baseline}). A colour channel bound as a COLUMN name finds no \
         such column in the batch and falls through to the default mark ink — \
         which is what this drew before brightfield could resolve a literal.\n  \
         {}\n  {}",
        want[0],
        want[1],
        want[2],
        png.display(),
        control.display()
    );

    let leftover = pixels_near(&png, default_mark_ink());
    assert_eq!(
        leftover, 0,
        "{name}: {leftover} px of the DEFAULT mark ink are still on the page \
         beside {got} px of the colour `{colour_line}` asked for. The mark cannot \
         be both; a picture holding the default is one whose constant never \
         reached the canvas. {}",
        png.display()
    );
}

// ---------------------------------------------------------------------------
// The colour the spec named is the colour on the page
// ---------------------------------------------------------------------------

/// A CSS keyword on `fill` paints that keyword's colour.
///
/// Four keywords, not one, and each a different value: a resolver wired to a
/// single hard-coded colour, or one returning the table's first or last entry
/// for everything, passes a one-keyword test and fails here.
#[test]
fn a_css_keyword_on_fill_paints_that_colour() {
    for (keyword, want) in [
        ("steelblue", STEELBLUE),
        ("crimson", [0xdc, 0x14, 0x3c]),
        ("gold", [0xff, 0xd7, 0x00]),
        // Mixed case, because CSS keywords are case-insensitive and a lookup
        // that forgot to fold only fails on a spec written this way.
        ("RebeccaPurple", [0x66, 0x33, 0x99]),
    ] {
        assert_painted(
            &format!("fill-{}", keyword.to_ascii_lowercase()),
            bars,
            &format!("    fill: {keyword}"),
            want,
        );
    }
}

/// A CSS hex literal on `fill` paints that colour too.
///
/// Hex was as broken as the keywords — nothing resolved a colour-channel string
/// of any form — and two curated specs write `#ccc` / `#aaa`, so a fix handling
/// only the named colours would have left the corpus half-served.
#[test]
fn a_css_hex_literal_on_fill_paints_that_colour() {
    for (hex, want) in [
        // `#ccc` sits inside the anti-aliasing tolerance of the gridline ink,
        // which is exactly why this file measures against a control.
        ("#ccc", [0xcc, 0xcc, 0xcc]),
        ("#aaa", [0xaa, 0xaa, 0xaa]),
        ("#4682b4", STEELBLUE),
        // `#rgba`: the fourth length `is_colour_literal` classifies as a
        // colour. Classified and not parseable is the same defect one rung down.
        ("#46af", [0x44, 0x66, 0xaa]),
    ] {
        assert_painted(
            &format!("fill-hex-{}", hex.trim_start_matches('#')),
            bars,
            &format!("    fill: '{hex}'"),
            want,
        );
    }
}

/// A CSS keyword on `stroke` paints that colour.
#[test]
fn a_css_keyword_on_stroke_paints_that_colour() {
    assert_painted(
        "stroke-crimson",
        line,
        "    stroke: crimson",
        [0xdc, 0x14, 0x3c],
    );
}

/// The control for the whole file: a spec that names NO colour keeps the
/// default mark ink, and acquires no other.
///
/// Without this, every assertion above is satisfiable by repainting the default
/// itself, and the suite would report a working resolver on a build that had
/// merely changed one design token.
#[test]
fn a_spec_that_names_no_colour_keeps_the_default_ink() {
    let png = export("no-colour", &bars(""));
    let got = pixels_near(&png, default_mark_ink());
    assert!(
        got >= INK_FLOOR,
        "an unbound fill must still take the default mark ink; found {got} px"
    );
    for other in [STEELBLUE, [0xdc, 0x14, 0x3c]] {
        assert_eq!(
            pixels_near(&png, other),
            0,
            "a spec that names no colour must not acquire one"
        );
    }
}

/// `currentColor` is recognised as a literal and keeps the DEFAULT ink.
///
/// It names an inherited text colour out of a CSS cascade this renderer does
/// not have, so there is nothing honest to resolve it to. A curated spec writes
/// it, which is why the gap is pinned rather than left to be discovered: if it
/// ever starts resolving, that is a real capability and this assertion is what
/// should be rewritten to say so — silently is the one way it must not change.
#[test]
fn currentcolor_is_recognised_and_deliberately_unresolved() {
    assert!(brightfield_spec::vocab::is_colour_literal("currentColor"));
    let png = export("currentcolor", &bars("    fill: currentColor"));
    assert!(
        pixels_near(&png, default_mark_ink()) >= INK_FLOOR,
        "`currentColor` has no value here, so the mark keeps the default ink"
    );
}

// ---------------------------------------------------------------------------
// The shadow case — a keyword that is also a column name
// ---------------------------------------------------------------------------

/// `fill: gold` over a source that HAS a `gold` column reads as the COLOUR, and
/// the collision is still reported.
///
/// Both halves matter and they pull in opposite directions. The colour reading
/// is Mosaic's — the string is classified lexically, before any schema is
/// consulted — so resolving the shadow against the data instead would quietly
/// diverge from upstream on the one case where a spec is ambiguous. And the
/// warning is the only thing telling an author who meant the COLUMN that the
/// spec does not say so, so painting the colour must not silence it.
#[test]
fn a_colour_name_that_shadows_a_column_paints_the_colour_and_still_warns() {
    fn shadowed(colour_line: &str) -> String {
        format!(
            "data:\n  obs:\n    - {{ v: 1, gold: alpha }}\n    \
             - {{ v: 3, gold: beta }}\n    - {{ v: 4, gold: alpha }}\n    \
             - {{ v: 6, gold: beta }}\n    - {{ v: 7, gold: alpha }}\n    \
             - {{ v: 9, gold: beta }}\n    - {{ v: 11, gold: alpha }}\n    \
             - {{ v: 12, gold: beta }}\nplot:\n  - mark: rectY\n    \
             data: {{ from: obs }}\n    x: {{ bin: v }}\n    y: {{ count: }}\n\
             {colour_line}\nwidth: 640\nheight: 400\n"
        )
    }
    assert_painted(
        "shadow-gold",
        shadowed,
        "    fill: gold",
        [0xff, 0xd7, 0x00],
    );

    let source = shadowed("    fill: gold");
    let parsed = brightfield_spec::parse_spec(&source, brightfield_spec::Format::Yaml)
        .expect("the shadow fixture parses");
    let said = parsed
        .warnings
        .iter()
        .find(|w| {
            matches!(
                w,
                brightfield_spec::parse::ParseWarning::ColourNameShadowsColumn { .. }
            )
        })
        .map(ToString::to_string)
        .expect(
            "painting the colour must not silence the shadow warning — it is the \
             only thing that tells an author who meant the column that the spec \
             does not say so",
        );
    assert!(
        !said.contains("default colour"),
        "the warning may no longer claim the bars stay the default colour; the \
         pixels above are #ffd700 — {said}"
    );
}

// ---------------------------------------------------------------------------
// The corpus
// ---------------------------------------------------------------------------

/// Every colour literal the curated corpus writes on `fill` or `stroke` reaches
/// the canvas as that colour.
///
/// The list is READ OFF the corpus rather than typed here, so a curated spec
/// that starts writing a new keyword is covered the day it lands rather than
/// the day someone remembers to extend a list.
///
/// **What this does and does not prove.** Five of the six specs read parquet
/// files that are not vendored into this tree, so they cannot be exported at
/// all; each literal is therefore rendered through a fixture that binds it on
/// the same channel. That proves the corpus's colour VOCABULARY reaches the
/// canvas. It does not prove every curated MARK honours it — `hexgrid`, which
/// writes the corpus's `#aaa`, draws its lattice in a design token and ignores
/// the spec's stroke.
#[test]
fn every_colour_literal_the_curated_corpus_writes_is_painted() {
    use brightfield_spec::ast::{SpecValue, ValueOrParamRef};

    let entries = brightfield_conformance::curated_entries().expect("load the curated corpus");
    // (spec name, literal) for every colour-channel literal in the corpus.
    let mut found: Vec<(String, String)> = Vec::new();

    for entry in &entries {
        let source = std::fs::read_to_string(&entry.source_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", entry.name));
        let parsed = brightfield_spec::parse_spec(&source, brightfield_spec::Format::Yaml)
            .unwrap_or_else(|e| panic!("parse {}: {e}", entry.name));
        for mark in brightfield_sql::collect_marks(&parsed.spec) {
            for channel in ["fill", "stroke"] {
                if let Some(ValueOrParamRef::Value(SpecValue::String(s))) = mark.options.get(channel)
                {
                    if brightfield_spec::vocab::is_colour_literal(s) {
                        found.push((entry.name.clone(), s.clone()));
                    }
                }
            }
        }
    }

    let names = |keep: fn(&str) -> bool| {
        let mut v: Vec<&str> = found
            .iter()
            .filter(|(_, lit)| keep(lit))
            .map(|(spec, _)| spec.as_str())
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    // Two counts, because they are different facts and the card that opened
    // this work turned on the first: SIX specs write a bare colour KEYWORD, and
    // a seventh writes only hex. An enumeration that quietly returned nothing
    // is how a corpus gate passes without checking anything, so both are
    // asserted rather than assumed.
    let keyworded = names(|l| !l.starts_with('#'));
    let any_literal = names(|_| true);
    assert_eq!(
        keyworded.len(),
        6,
        "six curated specs write a bare colour keyword: {keyworded:?}"
    );
    assert_eq!(
        any_literal.len(),
        7,
        "…and a seventh writes a colour literal in hex only: {any_literal:?}"
    );

    let mut literals: Vec<String> = found.iter().map(|(_, l)| l.clone()).collect();
    literals.sort();
    literals.dedup();
    assert!(
        literals.iter().any(|l| l == "steelblue"),
        "steelblue is the corpus's most-written colour; not finding it means the \
         walk is not reading colour channels: {literals:?}"
    );

    for literal in &literals {
        // `currentColor` is the one literal the corpus writes that this build
        // cannot resolve; it has its own test above.
        if literal.eq_ignore_ascii_case("currentcolor") {
            continue;
        }
        let want = renderer_colour(literal).unwrap_or_else(|| {
            panic!(
                "the curated corpus writes `{literal}` on a colour channel and \
                 nothing in this build turns it into an RGB — a spec the chart \
                 draws in the wrong ink"
            )
        });
        assert_painted(
            &format!("corpus-{}", sanitise(literal)),
            bars,
            &format!("    fill: '{literal}'"),
            want,
        );
    }
}

/// Resolve a literal through the RENDERER's own resolver, so the expectation
/// cannot come from a second private table that agrees with nothing on canvas.
fn renderer_colour(literal: &str) -> Option<[u8; 3]> {
    let [r, g, b, _] = brightfield_render::mark::parse_colour_literal(literal)?.components;
    Some([
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
    ])
}

/// A filename-safe form of a literal (`#ccc` → `-ccc`).
fn sanitise(literal: &str) -> String {
    literal
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}
