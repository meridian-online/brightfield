//! **The guard for a colour written as digits.**
//!
//! The sweep that closed the mode-blind-ink card greps this crate's `src/` for
//! `INK_LIGHT` and `*_LIGHT`, which catches a paint bound to the wrong mode's
//! token. It cannot catch a paint bound to no token at all. `HEXGRID_STROKE`
//! and `GEO_STROKE_COLOUR` were `Color::new([0.72, 0.72, 0.72, 1.0])` and
//! `Color::new([0.15, 0.15, 0.15, 1.0])` — two drawing-path colours that drew
//! the same value in both modes — and they survived a lane whose entire subject
//! was mode-blind ink *because the check could not see them*.
//!
//! So this file does not read source text. Widening a grep to match
//! `Color::new(` buys exactly one round: the next literal is spelled
//! `Color::from_rgba8`, or `0xb8b8b8`, or built from a `[f32; 4]` two lines up,
//! or sits behind a `const` in a module the path glob missed. Each of those is
//! a different string and the same defect, and a text scan closes them one at a
//! time forever.
//!
//! Instead it **runs the renderers and asks the scene what they drew.** Every
//! entry in [`default_renderers`] is driven twice over one fixture — once on
//! [`ChartInk::LIGHT`], once on [`ChartInk::DARK`] — and the colours vello
//! encoded into `draw_data` are compared. A paint that took its value from the
//! mode appears in one set and not the other. A paint that did not appears in
//! both, whatever it was spelled as, and this file names it with its hex.
//!
//! Two claims, and the second is the one that stops the first going quiet:
//!
//! 1. no colour is in both modes' scenes except the ones [`MODE_INVARIANT`]
//!    names and justifies;
//! 2. the set of renderers that drew NOTHING is exactly [`SILENT`], which is
//!    empty. A fixture that stops making a mark draw turns claim 1 into a
//!    statement about an empty set, which is the shape of a guard that cannot
//!    fail — so the silence is enumerated rather than tolerated.
//!
//! Data ink is exempt, and [`on_the_fill_ramp`] is where: a sequential ramp has
//! one published value across both modes, so a heatmap cell drawing the same
//! colour in dark as in light is right. That exemption asks the SCALE whether
//! it can produce the colour rather than naming the renderers that use one,
//! because naming `Heatmap` would exempt every other paint those modules make.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arrow::array::{Float64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use vello::Scene;

use brightfield_render::channel::{Channel, ChannelMap};
use brightfield_render::ink::ChartInk;
use brightfield_render::mark::default_renderers;
use brightfield_render::scale::{Scale, ScaleSet};
use brightfield_spec::vocab::MarkKind;

/// The plot-area pixel box the fixture's scales map onto.
const X_RANGE: (f64, f64) = (40.0, 600.0);
const Y_RANGE: (f64, f64) = (440.0, 40.0);

/// Colours a mark may legitimately draw the same in both modes, each with the
/// reason it does not move. Capped and justified, on `token_discipline.rs`'s
/// pattern: the cap is what stops this list absorbing the next defect.
///
/// It ships EMPTY. Nothing in the fixture below binds a colour channel, so no
/// sequential ramp and no `viz::STATUS` ink — the two families the design crate
/// publishes as one value across modes — is reachable from it. An entry here
/// means a mark drew a fixed colour from a path that had a mode available, and
/// that is the defect this file exists for until someone writes down why it is
/// not.
const MODE_INVARIANT: &[(&str, &str)] = &[];

/// Renderers no fixture makes draw, and why — the marks the sweep above is
/// silent about.
///
/// It ships EMPTY: every kind [`default_renderers`] registers draws under at
/// least one of [`fixtures`], including the ones that read a column the SQL
/// lowerer synthesises (`__bf_count`, `__bf_hex_dx`/`__bf_hex_dy`), which the
/// grid fixture writes by hand.
///
/// The list is asserted EQUAL to what the run observes rather than as a
/// superset, which is the point of it: a fixture edit that stops driving a mark
/// does not quietly shrink what claim 1 covers, it reddens and names the mark.
const SILENT: &[(MarkKind, &str)] = &[];

/// One way of driving a renderer: the batch it reads, the channels it is bound
/// on, and the scales those channels resolve through.
///
/// Several, rather than one, because the marks in this registry do not share a
/// shape: a bar needs a band scale over a string column, a rect needs paired
/// edge channels, and a dot needs neither. A renderer counts as driven if ANY
/// fixture makes it draw.
struct Fixture {
    name: &'static str,
    batch: RecordBatch,
    channels: ChannelMap,
    scales: fn(ChartInk) -> ScaleSet,
}

/// Six rows with two positional columns, paired edges, a category, a label and
/// a GeoJSON geometry — everything a renderer can read WITHOUT a column the SQL
/// lowerer synthesises, and no fill.
fn batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("x1", DataType::Float64, false),
        Field::new("x2", DataType::Float64, false),
        Field::new("y1", DataType::Float64, false),
        Field::new("y2", DataType::Float64, false),
        Field::new("cat", DataType::Utf8, false),
        Field::new("label", DataType::Utf8, false),
        Field::new("geom", DataType::Utf8, true),
    ]));
    let poly = |x0: i32, y0: i32| {
        format!(
            "{{\"type\":\"Polygon\",\"coordinates\":[[[{x0},{y0}],[{},{y0}],[{},{}],[{x0},{}],[{x0},{y0}]]]}}",
            x0 + 8,
            x0 + 8,
            y0 + 8,
            y0 + 8
        )
    };
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0])),
            Arc::new(Float64Array::from(vec![10.0, 20.0, 15.0, 30.0, 25.0, 40.0])),
            Arc::new(Float64Array::from(vec![0.5, 1.5, 2.5, 3.5, 4.5, 5.5])),
            Arc::new(Float64Array::from(vec![1.5, 2.5, 3.5, 4.5, 5.5, 6.5])),
            Arc::new(Float64Array::from(vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0])),
            Arc::new(Float64Array::from(vec![10.0, 20.0, 15.0, 30.0, 25.0, 40.0])),
            Arc::new(StringArray::from(vec!["a", "b", "c", "d", "e", "f"])),
            Arc::new(StringArray::from(vec!["p", "q", "r", "s", "t", "u"])),
            Arc::new(StringArray::from(vec![
                Some(poly(0, 0)),
                Some(poly(10, 0)),
                Some(poly(0, 10)),
                Some(poly(10, 10)),
                Some(poly(20, 0)),
                Some(poly(20, 10)),
            ])),
        ],
    )
    .expect("fixture batch")
}

fn map(pairs: &[(Channel, &str)]) -> ChannelMap {
    let mut m = ChannelMap::new();
    for (c, col) in pairs {
        m.insert(*c, (*col).to_string());
    }
    m
}

fn linear(domain: (f64, f64), range: (f64, f64)) -> Scale {
    Scale::Linear {
        domain_min: domain.0,
        domain_max: domain.1,
        range_start: range.0,
        range_end: range.1,
    }
}

fn band(range: (f64, f64)) -> Scale {
    Scale::Band {
        categories: ["a", "b", "c", "d", "e", "f"]
            .iter()
            .map(ToString::to_string)
            .collect(),
        range_start: range.0,
        range_end: range.1,
        padding: 0.1,
    }
}

fn xy_scales(ink: ChartInk) -> ScaleSet {
    let mut s = ScaleSet::in_ink(ink);
    s.insert(Channel::X, linear((0.0, 7.0), X_RANGE));
    s.insert(Channel::Y, linear((0.0, 50.0), Y_RANGE));
    s
}

fn band_x_scales(ink: ChartInk) -> ScaleSet {
    let mut s = ScaleSet::in_ink(ink);
    s.insert(Channel::X, band(X_RANGE));
    s.insert(Channel::Y, linear((0.0, 50.0), Y_RANGE));
    s
}

fn band_y_scales(ink: ChartInk) -> ScaleSet {
    let mut s = ScaleSet::in_ink(ink);
    s.insert(Channel::X, linear((0.0, 50.0), X_RANGE));
    s.insert(Channel::Y, band(Y_RANGE));
    s
}

/// A lowered 2D-density grid: the bin centres and the `__bf_count` the density
/// lowerer emits, plus the constant hex half-extents the hexbin lowerer emits
/// in-band. Six by six so `build_kde_grid` has two occupied centres on each
/// axis, which is its floor.
///
/// The reserved `__bf_` columns are written by the SQL lowerer in production
/// and by hand here, because this crate does not depend on `brightfield-sql`
/// and a renderer that cannot be driven cannot be checked.
fn grid_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("x_bin", DataType::Float64, false),
        Field::new("y_bin", DataType::Float64, false),
        Field::new("__bf_count", DataType::Float64, false),
        Field::new("__bf_hex_dx", DataType::Float64, false),
        Field::new("__bf_hex_dy", DataType::Float64, false),
    ]));
    let (mut xs, mut ys, mut cs) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..6 {
        for j in 0..6 {
            xs.push(f64::from(i));
            ys.push(f64::from(j));
            cs.push(f64::from((i * j) % 7 + 1));
        }
    }
    let n = xs.len();
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(xs)),
            Arc::new(Float64Array::from(ys)),
            Arc::new(Float64Array::from(cs)),
            Arc::new(Float64Array::from(vec![0.4_f64; n])),
            Arc::new(Float64Array::from(vec![0.45_f64; n])),
        ],
    )
    .expect("grid batch")
}

/// A categorical x categorical batch — what `cell` reads.
fn cells_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("cat", DataType::Utf8, false),
        Field::new("cat2", DataType::Utf8, false),
        Field::new("v", DataType::Float64, false),
    ]));
    let (mut a, mut b, mut v) = (Vec::new(), Vec::new(), Vec::new());
    for (i, x) in ["a", "b", "c", "d", "e", "f"].iter().enumerate() {
        for (j, y) in ["a", "b", "c", "d", "e", "f"].iter().enumerate() {
            a.push(*x);
            b.push(*y);
            v.push((i * 6 + j) as f64);
        }
    }
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(a)),
            Arc::new(StringArray::from(b)),
            Arc::new(Float64Array::from(v)),
        ],
    )
    .expect("cells batch")
}

/// The single-row coefficient batch the regression lowerer emits — the mark
/// reads no raw x/y at all, so nothing else drives it.
fn regression_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("slope", DataType::Float64, false),
        Field::new("intercept", DataType::Float64, false),
        Field::new("n", DataType::Float64, false),
        Field::new("x_bar", DataType::Float64, false),
        Field::new("sxx", DataType::Float64, false),
        Field::new("sxy", DataType::Float64, false),
        Field::new("syy", DataType::Float64, false),
        Field::new("x_min", DataType::Float64, false),
        Field::new("x_max", DataType::Float64, false),
        Field::new("y_min", DataType::Float64, false),
        Field::new("y_max", DataType::Float64, false),
    ]));
    let one = |v: f64| Arc::new(Float64Array::from(vec![v])) as arrow::array::ArrayRef;
    RecordBatch::try_new(
        schema,
        vec![
            one(4.0),
            one(6.0),
            one(6.0),
            one(3.5),
            one(17.5),
            one(70.0),
            one(300.0),
            one(1.0),
            one(6.0),
            one(10.0),
            one(40.0),
        ],
    )
    .expect("regression batch")
}

fn cells_scales(ink: ChartInk) -> ScaleSet {
    let mut s = ScaleSet::in_ink(ink);
    s.insert(Channel::X, band(X_RANGE));
    s.insert(Channel::Y, band(Y_RANGE));
    s
}

fn grid_scales(ink: ChartInk) -> ScaleSet {
    let mut s = ScaleSet::in_ink(ink);
    s.insert(Channel::X, linear((0.0, 5.0), X_RANGE));
    s.insert(Channel::Y, linear((0.0, 5.0), Y_RANGE));
    s
}

/// The fixtures, in the order a failure lists them.
///
/// No fixture binds `fill:` or `stroke:`, deliberately. A colour channel builds
/// a `Scale::Colour` or a `Scale::Sequential`, and those carry data ink: the
/// sequential ramps have one published value across both modes by the design
/// crate's own decision, so a mark drawing ramp ink would appear here as
/// mode-blind and be wrong. What is under test is the paints a MODULE chooses,
/// which is where a literal can hide.
fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture {
            name: "xy",
            batch: batch(),
            channels: map(&[
                (Channel::X, "x"),
                (Channel::Y, "y"),
                (Channel::Text, "label"),
            ]),
            scales: xy_scales,
        },
        Fixture {
            name: "banded-x",
            batch: batch(),
            channels: map(&[(Channel::X, "cat"), (Channel::Y, "y")]),
            scales: band_x_scales,
        },
        Fixture {
            name: "banded-y",
            batch: batch(),
            channels: map(&[(Channel::X, "y"), (Channel::Y, "cat")]),
            scales: band_y_scales,
        },
        Fixture {
            name: "edges",
            batch: batch(),
            channels: map(&[
                (Channel::X, "x"),
                (Channel::Y, "y"),
                (Channel::X1, "x1"),
                (Channel::X2, "x2"),
                (Channel::Y1, "y1"),
                (Channel::Y2, "y2"),
            ]),
            scales: xy_scales,
        },
        Fixture {
            name: "density-grid",
            batch: grid_batch(),
            channels: map(&[(Channel::X, "x_bin"), (Channel::Y, "y_bin")]),
            scales: grid_scales,
        },
        Fixture {
            name: "cells",
            batch: cells_batch(),
            channels: map(&[(Channel::X, "cat"), (Channel::Y, "cat2")]),
            scales: cells_scales,
        },
        Fixture {
            name: "regression",
            batch: regression_batch(),
            channels: ChannelMap::new(),
            scales: xy_scales,
        },
    ]
}

/// The scale set a renderer actually renders against, after it has contributed
/// its own scales — which is where a sequential ramp appears.
fn resolved_scales(
    renderer: &dyn brightfield_render::mark::MarkRenderer,
    fixture: &Fixture,
    ink: ChartInk,
) -> ScaleSet {
    let mut set = (fixture.scales)(ink);
    renderer.augment_scales(
        &mut set,
        &fixture.batch,
        &fixture.channels,
        X_RANGE,
        Y_RANGE,
    );
    set
}

/// Every brush colour one renderer encodes on `ink`'s canvas over `fixture`, as
/// vello's packed premultiplied RGBA8 — the probe `dark_canvas.rs` reads, for
/// the same reason: it reports what was drawn, not what the code was handed.
fn drawn(
    renderer: &dyn brightfield_render::mark::MarkRenderer,
    fixture: &Fixture,
    ink: ChartInk,
) -> HashSet<u32> {
    let mut set = (fixture.scales)(ink);
    renderer.augment_scales(
        &mut set,
        &fixture.batch,
        &fixture.channels,
        X_RANGE,
        Y_RANGE,
    );
    let mut scene = Scene::new();
    renderer.render(&mut scene, &fixture.batch, &fixture.channels, &set, None);
    scene.encoding().draw_data.iter().copied().collect()
}

/// How close a drawn colour has to sit to the ramp to count as ramp ink: one
/// 8-bit step per channel, which is the round-trip error of premultiplying and
/// packing and nothing more.
const RAMP_TOL: f32 = 1.5 / 255.0;

/// How finely the ramp is sampled when asking whether a colour is on it. Nine
/// stops over 4096 samples is ~512 samples per linear segment, so consecutive
/// samples differ by far less than [`RAMP_TOL`] and a colour genuinely on the
/// ramp cannot fall between two of them.
const RAMP_SAMPLES: u32 = 4096;

/// **Whether a colour is one the fill scale in force can produce.**
///
/// This is the exemption for data ink, and it is written as a question to the
/// SCALE rather than as a list of renderer names on purpose. A sequential ramp
/// has one published value across both modes — the design crate says so of
/// `viz::STATUS` and ships `SEQUENTIAL_MERIDIAN` as a single ramp — so a
/// heatmap cell or a raster pixel drawing the same colour in dark as in light
/// is correct, not mode-blind. Naming `Heatmap` and `Raster` would exempt
/// whatever else those two modules ever paint, including a literal; asking the
/// ramp exempts only what the ramp can produce, for every mark equally.
///
/// The residual hole is [`RAMP_TOL`] wide: a hand-typed colour that happens to
/// land within one 8-bit step of the viridis polyline would be exempted. That
/// is a much narrower target than "any colour in a module that also draws a
/// ramp".
fn on_the_fill_ramp(packed: u32, set: &ScaleSet) -> bool {
    let Some(scale @ Scale::Sequential { .. }) = set.get(Channel::Fill) else {
        return false;
    };
    // vello packs premultiplied RGBA8 as `a<<24 | b<<16 | g<<8 | r`.
    let byte = |shift: u32| ((packed >> shift) & 0xff) as f32 / 255.0;
    let (r, g, b, a) = (byte(0), byte(8), byte(16), byte(24));
    if a <= f32::EPSILON {
        return false;
    }
    let straight = [r / a, g / a, b / a];
    let Scale::Sequential {
        domain_min,
        domain_max,
        ..
    } = scale
    else {
        return false;
    };
    (0..=RAMP_SAMPLES).any(|i| {
        let t = f64::from(i) / f64::from(RAMP_SAMPLES);
        let want = scale.map_continuous(domain_min + (domain_max - domain_min) * t);
        (0..3).all(|c| (straight[c] - want[c]).abs() <= RAMP_TOL)
    })
}

/// `#rrggbbaa` for a packed premultiplied word, so a violation names a colour a
/// reader can search the tree for.
fn hex(packed: u32) -> String {
    format!("#{packed:08x}")
}

/// The invariant list as a lookup, keyed by the hex it names.
fn invariant_hexes() -> HashMap<&'static str, &'static str> {
    MODE_INVARIANT.iter().copied().collect()
}

#[test]
fn the_invariant_list_stays_small_and_justified() {
    assert!(
        MODE_INVARIANT.len() <= 5,
        "MODE_INVARIANT has {} entries; past five this has stopped being a list \
         of exceptions and become the rule",
        MODE_INVARIANT.len()
    );
    for (hex, why) in MODE_INVARIANT {
        assert!(
            !why.trim().is_empty(),
            "MODE_INVARIANT entry {hex} carries no justification"
        );
        assert!(
            hex.starts_with('#') && hex.len() == 9,
            "MODE_INVARIANT keys are `#rrggbbaa` as this file prints them; got {hex}"
        );
    }
    for (kind, why) in SILENT {
        assert!(
            !why.trim().is_empty(),
            "SILENT entry {kind:?} carries no justification"
        );
    }
}

/// **No registered mark draws the same colour in both modes.**
///
/// The claim is about the COLOUR reaching the scene, so it holds whether the
/// paint came from a token, a `const`, a float array or a hex word.
#[test]
fn every_registered_mark_repaints_when_the_mode_changes() {
    let allowed = invariant_hexes();
    let mut violations: Vec<String> = Vec::new();

    for fixture in fixtures() {
        for (kind, renderer) in default_renderers() {
            let light = drawn(renderer.as_ref(), &fixture, ChartInk::LIGHT);
            let dark = drawn(renderer.as_ref(), &fixture, ChartInk::DARK);
            let resolved = resolved_scales(renderer.as_ref(), &fixture, ChartInk::DARK);
            let mut both: Vec<u32> = light.intersection(&dark).copied().collect();
            both.sort_unstable();
            for packed in both {
                let h = hex(packed);
                if !on_the_fill_ramp(packed, &resolved) && !allowed.contains_key(h.as_str()) {
                    violations.push(format!(
                        "{kind:?} on the `{}` fixture draws {h} in BOTH modes — \
                         that paint does not come from the mode in force",
                        fixture.name
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "mode-blind ink ({}):\n{}\n\nRoute the paint through ChartInk, or add \
         the colour to MODE_INVARIANT with the reason it has one value in both \
         modes.",
        violations.len(),
        violations.join("\n")
    );
}

/// **The sweep above is not talking about an empty set.**
///
/// A renderer that draws nothing contributes no colours, so it satisfies the
/// claim vacuously. The set that does so across every fixture is committed in
/// [`SILENT`] and asserted equal, so a fixture change that silences a mark
/// reddens instead of quietly narrowing what is being checked.
#[test]
fn the_marks_these_fixtures_drive_are_the_committed_set() {
    let all = fixtures();
    let mut observed: HashSet<String> = HashSet::new();
    let mut drove: HashSet<String> = HashSet::new();

    for (kind, renderer) in default_renderers() {
        let name = format!("{kind:?}");
        let any = all.iter().any(|f| {
            !drawn(renderer.as_ref(), f, ChartInk::LIGHT).is_empty()
                || !drawn(renderer.as_ref(), f, ChartInk::DARK).is_empty()
        });
        if any {
            drove.insert(name);
        } else {
            observed.insert(name);
        }
    }

    let expected: HashSet<String> = SILENT.iter().map(|(k, _)| format!("{k:?}")).collect();
    let mut newly_silent: Vec<&String> = observed.difference(&expected).collect();
    newly_silent.sort();
    let mut newly_drawing: Vec<&String> = expected.difference(&observed).collect();
    newly_drawing.sort();
    assert!(
        newly_silent.is_empty() && newly_drawing.is_empty(),
        "the set of renderers these fixtures leave silent has moved.\n  \
         now silent, not in SILENT: {newly_silent:?}\n  \
         in SILENT, now drawing:    {newly_drawing:?}\n\
         A mark that went silent takes itself out of the mode sweep.",
    );

    // The two the guard was widened for, by name. They are the marks whose
    // literals the grep could not see, so a fixture that stopped driving them
    // would leave this whole file green over the exact defect it was written
    // for.
    for kind in [MarkKind::Hexgrid, MarkKind::Geo] {
        assert!(
            drove.contains(&format!("{kind:?}")),
            "{kind:?} drew nothing on any fixture, so the sweep says nothing \
             about it — and it is one of the two marks this file exists for"
        );
    }
}
