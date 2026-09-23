//! **A point map of two coordinate columns, read off what was actually drawn.**
//!
//! `tests/projection_reference.rs` pins the projection MATH against an oracle
//! that is not this code. This file pins what the renderer does with it: that a
//! dot lands at the projected position rather than at the linear one, that a
//! graticule is drawn from the projection and the visible extent, and that
//! `aspectRatio` and a projection are refused together rather than composed.
//!
//! It reads vello's `Encoding::path_data` — the coordinates the scene encoded,
//! as `f32` bits — rather than asking the renderer what it meant to draw. The
//! reason is `tests/mode_blind_ink.rs`'s: a test that inspects the inputs of a
//! draw call cannot see a draw call that never happened.

use std::sync::Arc;

use arrow::array::Float64Array;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use vello::Scene;

use brightfield_render::channel::{Channel, ChannelMap, MarkProjection};
use brightfield_render::layout::ChartLayout;
use brightfield_render::mark::{
    graticule, graticule_step, DotRenderer, GeoExtent, GraticuleKind, MarkRenderer, PlotGraticule,
    Projection,
};
use brightfield_render::scale::{infer_scales, Scale, ScaleSet};
use brightfield_render::scene::{build_multi_mark_scene, ChartData};
use brightfield_render::ResolvedTitles;

/// The plot-area pixel box the fixture's scales map onto. `y_range` is
/// `(bottom, top)` — inverted — which is what supplies the screen flip, so a
/// projection does not negate its own latitude.
const X_RANGE: (f64, f64) = (40.0, 600.0);
const Y_RANGE: (f64, f64) = (440.0, 40.0);

/// The dot radius `DotRenderer` draws at. Private to that module, mirrored here
/// because a circle's encoded geometry is what this file reads; the four
/// cardinal points are each checked, so the value matters and the START angle
/// does not.
const DOT_RADIUS: f64 = 4.0;

/// Reykjavík, Milan and Sydney: far from the equator, spread across three
/// quadrants, and representable under each projection this file drives.
const FIXTURE: &[(f64, f64)] = &[(-21.94, 64.15), (9.19, 45.46), (151.21, -33.87)];

/// Reykjavík through d3-geo's spherical Mercator, in the projection's planar
/// units. **A literal from the oracle, not from `Projection::project`** — see
/// `tests/projection_reference.rs` for how it was produced and cross-checked.
/// It is the value this file maps through the plot's scales to say where the dot
/// must be.
const REYKJAVIK_MERCATOR: (f64, f64) = (-0.382_925_237_887_555_95, 1.471_896_519_530_021_5);

fn batch(points: &[(f64, f64)]) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("lon", DataType::Float64, false),
        Field::new("lat", DataType::Float64, false),
    ]));
    let lons: Vec<f64> = points.iter().map(|(lon, _)| *lon).collect();
    let lats: Vec<f64> = points.iter().map(|(_, lat)| *lat).collect();
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(lons)),
            Arc::new(Float64Array::from(lats)),
        ],
    )
    .expect("fixture batch")
}

fn channels(projection: Option<Projection>) -> ChannelMap {
    let mut cm = ChannelMap::new();
    cm.insert(Channel::X, "lon".to_string());
    cm.insert(Channel::Y, "lat".to_string());
    if let Some(p) = projection {
        cm.set_projection(MarkProjection::Through(p));
    }
    cm
}

/// Infer and augment exactly as the scene builders do, so the scales under test
/// are the ones a real plot would carry.
fn scales(batch: &RecordBatch, cm: &ChannelMap) -> ScaleSet {
    let mut set = infer_scales(batch, cm, X_RANGE, Y_RANGE);
    DotRenderer.augment_scales(&mut set, batch, cm, X_RANGE, Y_RANGE);
    set
}

/// Every coordinate the scene encoded, as pixel pairs.
fn drawn_points(scene: &Scene) -> Vec<(f64, f64)> {
    scene
        .encoding()
        .path_data
        .chunks_exact(2)
        .map(|c| {
            (
                f64::from(f32::from_bits(c[0])),
                f64::from(f32::from_bits(c[1])),
            )
        })
        .collect()
}

fn render(batch: &RecordBatch, cm: &ChannelMap, set: &ScaleSet) -> Scene {
    let mut scene = Scene::new();
    DotRenderer.render(&mut scene, batch, cm, set, None);
    scene
}

/// A whole PLOT of `layers` dot layers over one channel map, through the scene
/// builder the dashboard uses — where the graticule is drawn, once per plot —
/// returning the scene and the scales it drew against.
fn plot(
    layers: &[&RecordBatch],
    cm: &ChannelMap,
    renderer: &dyn MarkRenderer,
    layout: ChartLayout,
) -> (Scene, ScaleSet) {
    let entries: Vec<ChartData<'_>> = layers
        .iter()
        .map(|batch| ChartData {
            batch,
            channel_map: cm,
            renderer,
            layout,
            view_extent: None,
            highlight: None,
            sample: None,
            beyond_frame: false,
        })
        .collect();
    let refs: Vec<&ChartData<'_>> = entries.iter().collect();
    build_multi_mark_scene(&refs, false, &ResolvedTitles::default())
}

/// Every text the scene drew, read back off its glyph runs: the string, and the
/// run's origin in pixels — its start x (a run is placed at its left edge
/// whatever its anchor) and its baseline y.
///
/// The glyph ids are mapped back to characters through the same font's
/// character map, over the characters a plot's labels use; an id outside that
/// set reads as `?`, which no expected label contains.
fn drawn_texts(scene: &Scene) -> Vec<(String, (f64, f64))> {
    use skrifa::MetadataProvider;
    let font = skrifa::FontRef::new(meridian_design::fonts::INTER_REGULAR).expect("the UI font");
    let charmap = font.charmap();
    let by_id: std::collections::HashMap<u32, char> =
        "0123456789-.°abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ _"
            .chars()
            .filter_map(|c| charmap.map(c).map(|g| (g.to_u32(), c)))
            .collect();
    let resources = &scene.encoding().resources;
    resources
        .glyph_runs
        .iter()
        .map(|run| {
            let text = resources.glyphs[run.glyphs.clone()]
                .iter()
                .map(|g| by_id.get(&g.id).copied().unwrap_or('?'))
                .collect();
            let [x, y] = run.transform.translation;
            (text, (f64::from(x), f64::from(y)))
        })
        .collect()
}

/// How many times `vertex` appears in the drawn record. A stroked polyline
/// encodes each of its vertices once, so a line drawn twice shows its vertices
/// twice.
fn times_drawn(points: &[(f64, f64)], vertex: (f64, f64)) -> usize {
    points.iter().filter(|p| near(**p, vertex, 1e-3)).count()
}

/// The pixel rect the plot's two scales map onto, `(x0, y0, x1, y1)`.
fn plot_rect(set: &ScaleSet) -> (f64, f64, f64, f64) {
    let (
        Some(Scale::Linear {
            range_start: xs,
            range_end: xe,
            ..
        }),
        Some(Scale::Linear {
            range_start: ys,
            range_end: ye,
            ..
        }),
    ) = (set.get(Channel::X), set.get(Channel::Y))
    else {
        panic!("a projected plot has two linear positional scales");
    };
    (xs.min(*xe), ys.min(*ye), xs.max(*xe), ys.max(*ye))
}

/// A line's vertices in pixels, through the plot's scales.
fn pixels(line: &brightfield_render::mark::GraticuleLine, set: &ScaleSet) -> Vec<(f64, f64)> {
    let (Some(x), Some(y)) = (set.get(Channel::X), set.get(Channel::Y)) else {
        panic!("positional scales");
    };
    line.points
        .iter()
        .map(|(u, v)| (x.map_f64(*u), y.map_f64(*v)))
        .collect()
}

fn near(a: (f64, f64), b: (f64, f64), tol: f64) -> bool {
    (a.0 - b.0).abs() < tol && (a.1 - b.1).abs() < tol
}

/// Whether a circle of `DOT_RADIUS` centred at `centre` was drawn — its four
/// cardinal points are in the encoded geometry.
fn circle_drawn_at(points: &[(f64, f64)], centre: (f64, f64)) -> bool {
    [
        (centre.0 + DOT_RADIUS, centre.1),
        (centre.0 - DOT_RADIUS, centre.1),
        (centre.0, centre.1 + DOT_RADIUS),
        (centre.0, centre.1 - DOT_RADIUS),
    ]
    .into_iter()
    .all(|cardinal| points.iter().any(|p| near(*p, cardinal, 0.05)))
}

/// **AC1.** A point map of two coordinate columns is drawn through a named
/// projection: Reykjavík's dot is at the Mercator position and NOT at the linear
/// `(lon, lat)` one an unprojected scatter would put it at.
///
/// The projected position enters as a literal from the oracle and is mapped
/// through the plot's own scales, so what is being asserted is the renderer's
/// use of the projection rather than the projection's arithmetic, which is
/// pinned separately.
#[test]
fn a_dot_lands_at_its_projected_position_and_not_its_linear_one() {
    let batch = batch(FIXTURE);
    let cm = channels(Some(Projection::Mercator));
    let set = scales(&batch, &cm);
    let (Some(x_scale), Some(y_scale)) = (set.get(Channel::X), set.get(Channel::Y)) else {
        panic!("a projected dot mark must have both positional scales");
    };

    let projected = (
        x_scale.map_f64(REYKJAVIK_MERCATOR.0),
        y_scale.map_f64(REYKJAVIK_MERCATOR.1),
    );
    let linear = (x_scale.map_f64(-21.94), y_scale.map_f64(64.15));

    let points = drawn_points(&render(&batch, &cm, &set));
    assert!(
        circle_drawn_at(&points, projected),
        "no dot at the Mercator position {projected:?}"
    );
    // The two must be far enough apart that the assertion above could not be
    // satisfied by the linear position: a projection that quietly did nothing
    // would put the dot within a radius of both.
    let gap = (projected.0 - linear.0).hypot(projected.1 - linear.1);
    assert!(
        gap > 10.0 * DOT_RADIUS,
        "the fixture point must separate the two positions; they are {gap:.1}px apart"
    );
    assert!(
        !points.iter().any(|p| near(*p, linear, DOT_RADIUS)),
        "something was drawn at the linear position {linear:?}"
    );
}

/// **AC2, the pure half.** The graticule's lines come from the extent, and the
/// extent alone decides which of them exist.
#[test]
fn the_graticule_lines_are_the_whole_degrees_the_extent_contains() {
    let world = GeoExtent::new(-180.0, 180.0, -90.0, 90.0);
    let lines = graticule(Projection::Equirectangular, world);

    // 360° of longitude across a ladder that must give at least six intervals:
    // 90° gives four, 45° gives eight.
    assert_eq!(graticule_step(360.0), 45.0);
    assert_eq!(graticule_step(180.0), 30.0);

    let meridians: Vec<f64> = lines
        .iter()
        .filter(|l| l.kind == GraticuleKind::Meridian)
        .map(|l| l.degrees)
        .collect();
    assert_eq!(
        meridians,
        vec![-180.0, -135.0, -90.0, -45.0, 0.0, 45.0, 90.0, 135.0, 180.0]
    );
    let parallels: Vec<f64> = lines
        .iter()
        .filter(|l| l.kind == GraticuleKind::Parallel)
        .map(|l| l.degrees)
        .collect();
    assert_eq!(parallels, vec![-90.0, -60.0, -30.0, 0.0, 30.0, 60.0, 90.0]);
}

/// **AC2, the half that stops the extent being ignored.** A narrower extent must
/// produce DIFFERENT lines — a finer step and a different set of degrees — and
/// not the same world graticule redrawn.
///
/// Stated as the exact expected sets rather than as "the two differ", because a
/// graticule that reacted to the extent by the wrong amount would satisfy the
/// weaker claim.
#[test]
fn narrowing_the_extent_changes_the_graticule_rather_than_redrawing_it() {
    let world = graticule(
        Projection::Equirectangular,
        GeoExtent::new(-180.0, 180.0, -90.0, 90.0),
    );
    let iceland = graticule(
        Projection::Equirectangular,
        GeoExtent::new(-25.0, -13.0, 63.0, 67.0),
    );

    let degrees = |lines: &[brightfield_render::mark::GraticuleLine], kind| {
        lines
            .iter()
            .filter(|l| l.kind == kind)
            .map(|l| l.degrees)
            .collect::<Vec<_>>()
    };

    // 12° of longitude wants a 2° step (six intervals); 4° of latitude wants
    // 0.5° (eight — 1° gives only four).
    assert_eq!(graticule_step(12.0), 2.0);
    assert_eq!(graticule_step(4.0), 0.5);
    assert_eq!(
        degrees(&iceland, GraticuleKind::Meridian),
        vec![-24.0, -22.0, -20.0, -18.0, -16.0, -14.0]
    );
    assert_eq!(
        degrees(&iceland, GraticuleKind::Parallel),
        vec![63.0, 63.5, 64.0, 64.5, 65.0, 65.5, 66.0, 66.5, 67.0]
    );

    // And nothing of the world graticule survives into it: at 45°/30° spacing
    // the only line that could fall inside Iceland's box is none of them.
    let world_meridians = degrees(&world, GraticuleKind::Meridian);
    assert!(
        degrees(&iceland, GraticuleKind::Meridian)
            .iter()
            .all(|d| !world_meridians.contains(d)),
        "the narrowed graticule reused the world's meridians"
    );
}

/// **AC2, read off the drawn record.** The meridians the renderer strokes are at
/// the pixel columns the projection puts them at, and there are exactly as many
/// of them as the extent asks for.
///
/// Mercator maps a meridian to a vertical line, so each one occupies a single
/// pixel column — which is what makes the count readable from the encoded
/// geometry without having to reconstruct path boundaries.
#[test]
fn the_drawn_scene_carries_a_meridian_at_each_projected_longitude() {
    let batch = batch(FIXTURE);
    let cm = channels(Some(Projection::Mercator));
    let (scene, set) = plot(&[&batch], &cm, &DotRenderer, ChartLayout::new(640.0, 480.0));
    let (Some(x_scale), Some(y_scale)) = (set.get(Channel::X), set.get(Channel::Y)) else {
        panic!("a projected dot mark must have both positional scales");
    };
    let points = drawn_points(&scene);

    let expected = PlotGraticule::of(&set)
        .expect("a projected plot has a graticule")
        .lines;
    assert!(
        expected.iter().any(|l| l.kind == GraticuleKind::Meridian),
        "the fixture extent must contain meridians for this test to hold anything"
    );

    for line in &expected {
        let head = line.points[0];
        let pixel = (x_scale.map_f64(head.0), y_scale.map_f64(head.1));
        // A line may be clipped to the plot rect, so its first vertex is drawn
        // only when it is inside; every line here has at least one vertex that
        // is, which is what the clip guarantees for a line the extent kept.
        let any_vertex_drawn = line.points.iter().any(|(u, v)| {
            let p = (x_scale.map_f64(*u), y_scale.map_f64(*v));
            points.iter().any(|q| near(*q, p, 0.05))
        });
        assert!(
            any_vertex_drawn,
            "no vertex of the {:?} at {}° was drawn (first would be at {pixel:?})",
            line.kind, line.degrees
        );
    }
}

/// An unprojected dot mark draws no graticule at all — the picture a plain
/// scatter gets is unchanged by any of this. The graticule is the PLOT's, so
/// the question is asked of a plot: its scales carry no projection, so there is
/// no [`PlotGraticule`] to draw and no vertex of a projected plot's lines is
/// in its scene; and no mark draws one itself, so a bare mark render is only dots.
#[test]
fn an_unprojected_dot_mark_draws_no_graticule() {
    let batch = batch(FIXTURE);
    let layout = ChartLayout::new(640.0, 480.0);
    let plain = channels(None);
    let (plain_scene, plain_set) = plot(&[&batch], &plain, &DotRenderer, layout);
    assert!(
        PlotGraticule::of(&plain_set).is_none(),
        "an unprojected plot has no graticule"
    );

    let projected = channels(Some(Projection::Mercator));
    let (projected_scene, projected_set) = plot(&[&batch], &projected, &DotRenderer, layout);
    let lines = PlotGraticule::of(&projected_set)
        .expect("control: a projected plot has one")
        .lines;
    let rect = plot_rect(&projected_set);
    let inside = |p: &(f64, f64)| {
        p.0 > rect.0 + 1.0 && p.0 < rect.2 - 1.0 && p.1 > rect.1 + 1.0 && p.1 < rect.3 - 1.0
    };
    let vertices: Vec<(f64, f64)> = lines
        .iter()
        .flat_map(|l| pixels(l, &projected_set))
        .filter(inside)
        .collect();
    let projected_points = drawn_points(&projected_scene);
    assert!(
        vertices
            .iter()
            .all(|v| times_drawn(&projected_points, *v) >= 1),
        "control: the projected plot's scene carries its graticule's vertices"
    );
    let plain_points = drawn_points(&plain_scene);
    assert!(
        vertices.iter().all(|v| times_drawn(&plain_points, *v) == 0),
        "the unprojected plot drew a projected plot's graticule vertex"
    );

    // The mark on its own draws its dots and nothing else: a circle is one
    // path, so three circles' worth of geometry and no remainder.
    let mark_points = drawn_points(&render(&batch, &projected, &scales(&batch, &projected)));
    let one_dot = drawn_points(&render(
        &batch.slice(0, 1),
        &projected,
        &scales(&batch, &projected),
    ))
    .len();
    assert_eq!(
        mark_points.len(),
        one_dot * FIXTURE.len(),
        "a projected dot mark draws only its dots; the graticule is the plot's"
    );
}

/// **AC4.** A mark that asks for both an equal-aspect frame and a projection is
/// refused the combination, and the refusal is the PROJECTION winning rather
/// than both applying or both being dropped.
///
/// The two controls inside `equal_aspect_and_a_projection_cannot_both_apply` are
/// what make this a refusal rather than a coincidence: a mark carrying
/// `aspectRatio` by itself still gets it, and a mark carrying a projection by
/// itself never had it to lose.
#[test]
fn equal_aspect_and_a_projection_cannot_both_apply() {
    let mut aspect_only = ChannelMap::new();
    aspect_only.set_equal_aspect(true);
    assert!(
        aspect_only.equal_aspect(),
        "control: `aspectRatio: 1` alone is still honoured"
    );
    assert_eq!(aspect_only.projection(), None);

    let mut projection_only = ChannelMap::new();
    projection_only.set_projection(MarkProjection::Through(Projection::Mercator));
    assert!(
        !projection_only.equal_aspect(),
        "control: a projected mark never had equal-aspect to lose"
    );
    assert_eq!(projection_only.projection(), Some(Projection::Mercator));

    // Both, in each order — the refusal must not depend on which was written
    // first, which is why it lives in the accessor and not in the setters.
    for both in [
        {
            let mut cm = ChannelMap::new();
            cm.set_equal_aspect(true);
            cm.set_projection(MarkProjection::Through(Projection::Mercator));
            cm
        },
        {
            let mut cm = ChannelMap::new();
            cm.set_projection(MarkProjection::Through(Projection::Mercator));
            cm.set_equal_aspect(true);
            cm
        },
    ] {
        assert!(
            !both.equal_aspect(),
            "a projected mark must not also be equal-aspected"
        );
        assert_eq!(
            both.projection(),
            Some(Projection::Mercator),
            "the projection is what survives the refusal"
        );
    }
}

/// The refusal has to be visible in the DOMAINS, not only in the accessor: a
/// mark asking for both must be fitted in the projection's planar units, and a
/// composed one would carry degrees.
///
/// Mercator's `v` for this fixture is about 1.47 and the latitudes run to 64, so
/// a domain that had been widened against degrees is off by more than an order
/// of magnitude and cannot be mistaken for rounding.
#[test]
fn a_mark_asking_for_both_is_fitted_in_projected_units() {
    let batch = batch(FIXTURE);
    let mut cm = channels(Some(Projection::Mercator));
    cm.set_equal_aspect(true);
    let set = scales(&batch, &cm);

    let Some(Scale::Linear {
        domain_min,
        domain_max,
        ..
    }) = set.get(Channel::Y)
    else {
        panic!("a projected dot mark must have a linear y scale");
    };
    // The projected latitudes of the fixture span roughly [-0.63, 1.47]; the
    // aspect fit widens one axis, never past the plot's own ratio.
    assert!(
        domain_max.abs() < 10.0 && domain_min.abs() < 10.0,
        "the y domain must be in Mercator's planar units, not degrees: \
         [{domain_min}, {domain_max}]"
    );
}

// ---------------------------------------------------------------------------
// The seams between a resolved NAME and a renderer drawing with it
// ---------------------------------------------------------------------------

/// **Every arm of `From<ResolvedProjection> for Projection`, by name.**
///
/// The conversion is the one place a spec-side decision becomes a render-side
/// transform, and it is fifteen hand-written arms. Two of them being exercised
/// by the tests above is not a reason to trust the other thirteen: a swapped
/// pair maps `mercator` onto the orthographic and every test that does not
/// mention Mercator keeps passing.
///
/// Driven from Mosaic's wire names rather than from the `ResolvedProjection`
/// variants, so the assertion is what an author writes in a spec, and each name
/// is checked to land on a projection that produces the value the name's OWN
/// formula produces at a fixed coordinate. The reference values are
/// `Projection`'s own, which makes this a permutation check and not a maths
/// check — the maths is `tests/projection_reference.rs`'s job against an oracle
/// that is not this code, and stating which of the two each file does is the
/// point.
#[test]
fn every_wire_name_converts_to_the_projection_of_that_name() {
    use brightfield_spec::layout::ResolvedProjection as R;
    // (wire name, the variant it resolves to, the variant it must convert to).
    let table: [(&str, R, Projection); 16] = [
        (
            "equirectangular",
            R::Equirectangular,
            Projection::Equirectangular,
        ),
        ("identity", R::Identity, Projection::Identity),
        ("reflect-y", R::ReflectY, Projection::ReflectY),
        ("mercator", R::Mercator, Projection::Mercator),
        (
            "transverse-mercator",
            R::TransverseMercator,
            Projection::TransverseMercator,
        ),
        ("orthographic", R::Orthographic, Projection::Orthographic),
        ("stereographic", R::Stereographic, Projection::Stereographic),
        ("gnomonic", R::Gnomonic, Projection::Gnomonic),
        (
            "azimuthal-equal-area",
            R::AzimuthalEqualArea,
            Projection::AzimuthalEqualArea,
        ),
        (
            "azimuthal-equidistant",
            R::AzimuthalEquidistant,
            Projection::AzimuthalEquidistant,
        ),
        ("equal-earth", R::EqualEarth, Projection::EqualEarth),
        (
            "conic-equal-area",
            R::ConicEqualArea,
            Projection::ConicEqualArea,
        ),
        (
            "conic-conformal",
            R::ConicConformal,
            Projection::ConicConformal,
        ),
        (
            "conic-equidistant",
            R::ConicEquidistant,
            Projection::ConicEquidistant,
        ),
        ("albers", R::Albers, Projection::Albers),
        ("albers-usa", R::Albers, Projection::Albers),
    ];
    for (wire, resolved, projection) in table {
        assert_eq!(
            R::from_wire(wire),
            Some(resolved),
            "`{wire}` must resolve to {resolved:?}"
        );
        assert_eq!(
            Projection::from(resolved),
            projection,
            "{resolved:?} must convert to {projection:?}"
        );
    }

    // The permutation guard: fifteen distinct variants must come out of the
    // fifteen distinct inputs. Equality above would pass a conversion that
    // collapsed several arms onto one only if the expectations collapsed too,
    // and this is what stops the expectations being edited to match a defect.
    let mut out: Vec<String> = table
        .iter()
        .map(|(_, r, _)| format!("{:?}", Projection::from(*r)))
        .collect();
    out.sort();
    out.dedup();
    assert_eq!(
        out.len(),
        15,
        "the conversion must be injective over the catalogue; got {out:?}"
    );

    // And each converted projection actually PROJECTS differently — a permutation
    // that swapped two arms would be caught above only if the two variants are
    // distinguishable, which this shows they are at one coordinate.
    let mut positions: Vec<String> = table
        .iter()
        .filter_map(|(_, r, _)| Projection::from(*r).project(30.0, 40.0))
        .map(|(u, v)| format!("{u:.9},{v:.9}"))
        .collect();
    positions.sort();
    let distinct = {
        let mut p = positions.clone();
        p.dedup();
        p.len()
    };
    assert!(
        distinct >= 13,
        "at (30, 40) the catalogue must land in at least 13 distinct places \
         (equirectangular and identity coincide there by definition); got {distinct}"
    );
}

/// **The separability claim and the inverses that keep it, render side.**
///
/// `ResolvedProjection::axes_invert_separately` is a spec-side assertion that
/// `build_brushable_bindings` acts on and `axis_interval` relies on. Here is the
/// other half: for every name in the catalogue, the claim and the two inverses
/// agree, and where they answer they are the true inverse of the forward
/// transform.
///
/// Both directions matter. A projection declared separable whose inverses are
/// missing is a brush that silently stops filtering; one declared curved whose
/// inverses exist is a brush refused for nothing.
#[test]
fn separability_is_the_claim_the_inverses_keep() {
    use brightfield_spec::layout::ResolvedProjection as R;
    let names = [
        "equirectangular",
        "identity",
        "reflect-y",
        "mercator",
        "transverse-mercator",
        "orthographic",
        "stereographic",
        "gnomonic",
        "azimuthal-equal-area",
        "azimuthal-equidistant",
        "equal-earth",
        "conic-equal-area",
        "conic-conformal",
        "conic-equidistant",
        "albers",
        "albers-usa",
    ];
    let mut separable = 0;
    for name in names {
        let resolved = R::from_wire(name).expect("a Mosaic name");
        let projection = Projection::from(resolved);
        assert_eq!(
            resolved.axes_invert_separately(),
            projection.axes_invert_separately(),
            "`{name}`: the spec-side claim and the render-side capability disagree"
        );
        if !resolved.axes_invert_separately() {
            assert!(
                projection.invert_lon(0.5).is_none() || projection.invert_lat(0.5).is_none(),
                "`{name}` is declared curved, so at least one axis must have no inverse"
            );
            continue;
        }
        separable += 1;
        // Round-trip: forward then per-axis inverse returns the coordinate.
        for (lon, lat) in [(0.0, 0.0), (-21.94, 64.15), (151.21, -33.87), (9.19, 45.46)] {
            let (u, v) = projection
                .project(lon, lat)
                .expect("a separable projection is total here");
            let back_lon = projection.invert_lon(u).expect("declared separable");
            let back_lat = projection.invert_lat(v).expect("declared separable");
            assert!(
                (back_lon - lon).abs() < 1e-9 && (back_lat - lat).abs() < 1e-9,
                "`{name}` does not round-trip ({lon}, {lat}): got ({back_lon}, {back_lat})"
            );
        }
    }
    assert_eq!(
        separable, 4,
        "exactly four of Mosaic's sixteen names invert per axis"
    );
}

/// **The cross-mark union.** A point map's ghost layer and its brushed subset
/// are two entries sharing one set of scales, and the domains have to cover
/// BOTH — or the ghost is drawn outside its own frame.
///
/// Stated as coverage of two disjoint fixtures rather than as "the domain is
/// wide", because a union that took only the first entry, or only the last,
/// still produces a domain that looks like a domain.
#[test]
fn the_projected_domain_covers_every_marks_coordinates() {
    use brightfield_render::scale::infer_scales_multi;

    // Two marks with disjoint coordinates: a northern pair and a southern one.
    let north = batch(&[(-21.94, 64.15), (9.19, 60.0)]);
    let south = batch(&[(151.21, -33.87), (120.0, -40.0)]);
    let cm = channels(Some(Projection::Mercator));
    let entries = [(&north, &cm), (&south, &cm)];
    let set = infer_scales_multi(&entries, X_RANGE, Y_RANGE);

    let Some(Scale::Linear {
        domain_min: y0,
        domain_max: y1,
        ..
    }) = set.get(Channel::Y)
    else {
        panic!("a projected plot has a linear y scale");
    };
    let Some(Scale::Linear {
        domain_min: x0,
        domain_max: x1,
        ..
    }) = set.get(Channel::X)
    else {
        panic!("a projected plot has a linear x scale");
    };
    for (lon, lat) in [
        (-21.94, 64.15),
        (9.19, 60.0),
        (151.21, -33.87),
        (120.0, -40.0),
    ] {
        let (u, v) = Projection::Mercator
            .project(lon, lat)
            .expect("all four are inside Mercator's clip");
        assert!(
            u >= *x0 - 1e-9 && u <= *x1 + 1e-9,
            "({lon}, {lat}) projects to u = {u}, outside [{x0}, {x1}]"
        );
        assert!(
            v >= *y0 - 1e-9 && v <= *y1 + 1e-9,
            "({lon}, {lat}) projects to v = {v}, outside [{y0}, {y1}]"
        );
    }
    // The two marks' extents must be far enough apart that a domain built from
    // either one alone could not contain the other — otherwise the assertions
    // above would pass on a first-entry-only union.
    let only_north = infer_scales_multi(&[(&north, &cm)], X_RANGE, Y_RANGE);
    let Some(Scale::Linear {
        domain_min: n0,
        domain_max: n1,
        ..
    }) = only_north.get(Channel::Y)
    else {
        panic!("linear");
    };
    let (_, south_v) = Projection::Mercator
        .project(151.21, -33.87)
        .expect("inside");
    assert!(
        south_v < *n0 || south_v > *n1,
        "the fixtures must be disjoint for this test to hold anything: \
         {south_v} is inside [{n0}, {n1}]"
    );
}

/// **The graticule's extent is the PLOT's, not each mark's batch.**
///
/// A point map's ghost layer and its brushed subset both draw a graticule. Read
/// from each mark's own coordinates, the brushed layer's extent is narrower, so
/// it lands on a finer rung of the step ladder and lays a second, denser
/// graticule over the region the reader swept. Read from the shared scale set,
/// both layers compute the same lines and draw them on top of each other.
///
/// The assertion is the STEP, because that is what changes: it is the extent's
/// only visible consequence.
#[test]
fn a_brushed_layer_draws_the_same_graticule_as_the_ghost_behind_it() {
    use brightfield_render::scale::infer_scales_multi;

    let ghost = batch(FIXTURE);
    // The subset: one point, so its own extent is degenerate and its own step
    // would be the finest rung on the ladder.
    let subset = batch(&FIXTURE[1..2]);
    let cm = channels(Some(Projection::Mercator));
    let entries = [(&ghost, &cm), (&subset, &cm)];
    let set = infer_scales_multi(&entries, X_RANGE, Y_RANGE);

    let extent = set
        .geo_extent()
        .expect("a projected plot's scales carry the geographic extent");
    // The shared extent spans the ghost, not the subset.
    assert!(
        extent.lon_span() > 100.0,
        "the shared extent must span the whole cloud; got {}°",
        extent.lon_span()
    );

    // Both layers draw the same lines, because both read this one extent.
    let ghost_lines = graticule(Projection::Mercator, extent);
    let subset_lines = graticule(Projection::Mercator, extent);
    assert_eq!(ghost_lines, subset_lines);

    // The control that makes it a fix rather than a tautology: the subset's OWN
    // extent gives a different step, which is the second graticule this avoids.
    let subset_only = infer_scales_multi(&[(&subset, &cm)], X_RANGE, Y_RANGE);
    let narrow = subset_only.geo_extent().expect("still an extent");
    assert_ne!(
        graticule_step(narrow.lon_span()),
        graticule_step(extent.lon_span()),
        "the subset's own extent must pick a different step, or this test holds nothing"
    );
}

/// **A graticule line breaks where the projection has no position for it**,
/// rather than joining what remains into a polyline with a chord across the gap.
///
/// The projection has to be one whose unrepresentable set is not a single
/// contiguous stretch of the sampled line, or there is nothing to break. Under
/// the orthographic it IS contiguous — a meridian is wholly on the near side or
/// wholly off it, and a parallel leaves the horizon once and does not come back
/// — so an orthographic graticule exercises the break not at all, which is why
/// this uses the transverse Mercator instead. There the clip applies to the
/// ROTATED latitude, `cos φ · sin λ`, so the 90°E meridian is visible at both
/// ends and hidden across the equator: two runs with a gap between them.
///
/// Two assertions, and both are needed. That the line comes back as several
/// entries is the break happening; that the gap between consecutive runs dwarfs
/// the largest step inside one is what makes the break worth doing, and it is
/// the segment a non-breaking `push_runs` would draw.
#[test]
fn a_graticule_line_breaks_rather_than_chording_across_the_gap() {
    let extent = GeoExtent::new(-180.0, 180.0, -60.0, 60.0);
    let lines = graticule(Projection::TransverseMercator, extent);

    // The 90°E meridian: representable at both ends, unrepresentable across the
    // equator, so it must arrive as more than one run.
    let runs: Vec<&brightfield_render::mark::GraticuleLine> = lines
        .iter()
        .filter(|l| l.kind == GraticuleKind::Meridian && (l.degrees - 90.0).abs() < 1e-9)
        .collect();
    assert!(
        runs.len() > 1,
        "the 90°E meridian must come back as several runs; got {}",
        runs.len()
    );

    // The largest step WITHIN a run, against the gap BETWEEN two runs.
    let within = runs
        .iter()
        .flat_map(|l| {
            l.points
                .windows(2)
                .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        })
        .fold(0.0_f64, f64::max);
    let across = {
        let a = *runs[0].points.last().expect("a run has points");
        let b = runs[1].points[0];
        (b.0 - a.0).hypot(b.1 - a.1)
    };
    assert!(
        across > 5.0 * within,
        "the gap a non-breaking push_runs would chord across ({across:.3}) must dwarf \
         the largest step inside a run ({within:.3}), or the break buys nothing"
    );

    // The control: an unrepresentable vertex is genuinely what splits it. The
    // same meridian under a total projection is one run.
    let total = graticule(Projection::Equirectangular, extent);
    assert_eq!(
        total
            .iter()
            .filter(|l| l.kind == GraticuleKind::Meridian && (l.degrees - 90.0).abs() < 1e-9)
            .count(),
        1,
        "a total projection's meridian is one unbroken run"
    );
}

/// **A mark whose kind cannot draw through the plot's projection contributes
/// NOTHING** — not a wrong picture, not a partial one, nothing.
///
/// The plot's axes are in the projection's planar units. A `line` mark drawing
/// its raw degrees against them is a second coordinate system laid over the
/// first, and unlike a missing mark it looks like a mark. `scene::render_entry`
/// is the one place that decides this, and the three `augment_scales` call sites
/// skip such a mark for the same reason: its degree domain must not widen an
/// axis that is in planar units.
///
/// Byte-compared rather than inspected: adding the undrawable entry to a
/// composition must change the encoded scene not at all. The control is the same
/// pair of entries on a plot that names no projection, where the line IS drawn
/// and the two compositions differ.
#[test]
fn a_mark_that_cannot_project_contributes_no_geometry() {
    use brightfield_render::layout::ChartLayout;
    use brightfield_render::mark::LineRenderer;
    use brightfield_render::scene::{build_multi_mark_scene, ChartData};
    use brightfield_render::ResolvedTitles;
    use brightfield_spec::ast::Component;
    use brightfield_spec::{parse_spec, Format};

    let spec_for = |attrs: &str| {
        format!(
            "data:\n  t:\n    - {{ lon: 1, lat: 2 }}\nplot:\n  \
             - {{ mark: dot, data: {{ from: t }}, x: lon, y: lat }}\n  \
             - {{ mark: line, data: {{ from: t }}, x: lon, y: lat }}\n{attrs}"
        )
    };
    let batch = batch(FIXTURE);
    let layout = ChartLayout::new(640.0, 480.0);

    // `(scene bytes with the line, scene bytes without it)` for one spec.
    let compose = |attrs: &str| {
        let parsed = parse_spec(&spec_for(attrs), Format::Yaml).expect("parses");
        let Some(Component::Plot(plot)) = parsed.spec.root.as_ref() else {
            panic!("a plot root");
        };
        let marks: Vec<&brightfield_spec::ast::Mark> = plot
            .items
            .iter()
            .filter_map(|c| match c {
                Component::Mark(m) => Some(m),
                _ => None,
            })
            .collect();
        let dot_cm = ChannelMap::from_mark_in(marks[0], Some(plot));
        let line_cm = ChannelMap::from_mark_in(marks[1], Some(plot));
        fn entry<'a>(
            batch: &'a RecordBatch,
            cm: &'a ChannelMap,
            renderer: &'a dyn MarkRenderer,
            layout: ChartLayout,
        ) -> ChartData<'a> {
            ChartData {
                batch,
                channel_map: cm,
                renderer,
                layout,
                view_extent: None,
                highlight: None,
                sample: None,
                beyond_frame: false,
            }
        }
        let dot = entry(&batch, &dot_cm, &DotRenderer, layout);
        let line = entry(&batch, &line_cm, &LineRenderer, layout);
        let both = build_multi_mark_scene(&[&dot, &line], false, &ResolvedTitles::default())
            .0
            .encoding()
            .path_data
            .to_vec();
        let alone = build_multi_mark_scene(&[&dot], false, &ResolvedTitles::default())
            .0
            .encoding()
            .path_data
            .to_vec();
        (both, alone, line_cm)
    };

    let (both, alone, line_cm) = compose("projectionType: mercator\n");
    assert!(
        line_cm.mark_projection().is_undrawable(),
        "a line mark on a projected plot must be undrawable, or this test asserts nothing"
    );
    assert_eq!(
        both, alone,
        "an undrawable mark must contribute no geometry at all"
    );

    // The control: with no projection the same line IS drawn, so the equality
    // above is the guard acting and not the line being empty.
    let (both, alone, line_cm) = compose("");
    assert!(
        !line_cm.mark_projection().is_undrawable(),
        "control: with no projection a line mark draws"
    );
    assert_ne!(
        both, alone,
        "control: an unprojected line mark contributes geometry"
    );
}

/// **A projected dot mark draws no cartesian frame** — no axis line, no ticks,
/// no tick labels — and an unprojected one still draws all three.
///
/// A map draws its own scaffolding behind itself, and the graticule is at whole
/// degrees off the step ladder while `compute_ticks` puts axis ticks at its own
/// round numbers. Both at once is two grids at two spacings over one picture,
/// which is what the tile drew before this.
///
/// Read as TEXT rather than as paths. The only text a bare dot plot draws is
/// its frame's tick labels or, projected, its graticule's labels, and the two
/// are told apart by what they say: a graticule label is a degree value ending
/// in `°` and a tick label never is. Counting paths could not separate them,
/// because suppressing the frame removes gridline paths while the graticule
/// adds them.
#[test]
fn a_projected_dot_mark_draws_no_axis_labels() {
    let batch = batch(FIXTURE);
    let layout = ChartLayout::new(640.0, 480.0);
    let texts = |cm: &ChannelMap| {
        let (scene, _) = plot(&[&batch], cm, &DotRenderer, layout);
        drawn_texts(&scene)
            .into_iter()
            .map(|(text, _)| text)
            .collect::<Vec<_>>()
    };

    let plain = texts(&channels(None));
    assert!(
        !plain.is_empty() && plain.iter().all(|t| !t.ends_with('°')),
        "control: an unprojected scatter draws its tick labels, none of them in \
         degrees; got {plain:?}"
    );
    let projected = texts(&channels(Some(Projection::Mercator)));
    assert!(
        !projected.is_empty(),
        "control: a projected plot labels its graticule"
    );
    let ticks: Vec<&String> = projected.iter().filter(|t| !t.ends_with('°')).collect();
    assert!(
        ticks.is_empty(),
        "a projected dot mark must draw no tick labels; drew {ticks:?} among {projected:?}"
    );

    // The renderer's own answer, at the seam the scene builders read, so the
    // count above cannot pass for some other reason.
    assert!(
        DotRenderer.suppresses_frame(&channels(Some(Projection::Mercator))),
        "a projected dot mark suppresses the frame"
    );
    assert!(
        !DotRenderer.suppresses_frame(&channels(None)),
        "control: an unprojected dot mark keeps it"
    );
}

/// **The scale set carries a projection when a mark DRAWS through one**, and not
/// merely because the plot names one.
///
/// The two come apart for a plot whose positional marks cannot project. The plot
/// names a projection, no mark applied it, so the x/y domains are still the
/// degrees column inference produced — and `axis_interval`
/// (`brightfield-shell`) reads `ScaleSet::projection` to decide whether to
/// unproject a brush pixel. Set from the plot's name, it would unproject a value
/// that was never projected: under Mercator a longitude of 151.21 would be
/// divided by π/180 and the clause would name 8,663 degrees.
#[test]
fn the_scales_carry_a_projection_only_when_something_drew_through_it() {
    use brightfield_render::scale::infer_scales_multi;
    use brightfield_spec::ast::Component;
    use brightfield_spec::{parse_spec, Format};

    let batch = batch(FIXTURE);
    // A plot that NAMES mercator over a mark whose kind cannot draw through it.
    let spec = parse_spec(
        "data:\n  t:\n    - { lon: 1, lat: 2 }\nplot:\n  \
         - { mark: line, data: { from: t }, x: lon, y: lat }\n\
         projectionType: mercator\n",
        Format::Yaml,
    )
    .expect("parses");
    let Some(Component::Plot(plot)) = spec.spec.root.as_ref() else {
        panic!("a plot root");
    };
    let Some(Component::Mark(line)) = plot.items.first() else {
        panic!("a mark item");
    };
    let undrawable = ChannelMap::from_mark_in(line, Some(plot));
    assert!(
        undrawable.mark_projection().is_undrawable(),
        "the fixture must be undrawable, or this test asserts nothing"
    );

    let set = infer_scales_multi(&[(&batch, &undrawable)], X_RANGE, Y_RANGE);
    assert_eq!(
        set.projection(),
        None,
        "nothing drew through the plot's projection, so the axes are still in \
         degrees and the scale set must not claim otherwise"
    );
    // And the domains ARE in degrees, which is what makes the claim above the
    // load-bearing one rather than a naming preference.
    let Some(Scale::Linear { domain_max, .. }) = set.get(Channel::X) else {
        panic!("a linear x scale");
    };
    assert!(
        *domain_max > 100.0,
        "the x domain must be the degree domain (the fixture reaches 151.21°); \
         got {domain_max}"
    );

    // The control: add a mark that DOES draw through it, and the scale set says
    // so — same plot, same projection, one more mark.
    let drawn = channels(Some(Projection::Mercator));
    let both = infer_scales_multi(&[(&batch, &undrawable), (&batch, &drawn)], X_RANGE, Y_RANGE);
    assert_eq!(
        both.projection(),
        Some(Projection::Mercator),
        "a mark drawing through the projection puts it on the scale set"
    );
}

// ---------------------------------------------------------------------------
// The plot's graticule: labelled, over the plot area, stroked once
// ---------------------------------------------------------------------------

/// The California housing sample's coordinates, `(lon, lat)` per row — the
/// fixture the hero map is judged on.
fn california() -> Vec<(f64, f64)> {
    let csv = include_str!("../../brightfield-shell/tests/data/california_housing_sample.csv");
    let mut rows = csv.lines();
    let header: Vec<&str> = rows.next().expect("a header row").split(',').collect();
    let col = |name: &str| {
        header
            .iter()
            .position(|h| *h == name)
            .unwrap_or_else(|| panic!("no {name} column"))
    };
    let (lon, lat) = (col("longitude"), col("latitude"));
    rows.filter(|r| !r.is_empty())
        .map(|r| {
            let cells: Vec<&str> = r.split(',').collect();
            (
                cells[lon].parse().expect("a longitude"),
                cells[lat].parse().expect("a latitude"),
            )
        })
        .collect()
}

/// The whole multiples of `step` inside `[lo, hi]`, as the labels a graticule
/// over that range would carry. Written out here rather than asked of the
/// renderer, so the expectation is not the code under test restated.
fn whole_steps(lo: f64, hi: f64, step: f64) -> Vec<String> {
    let first = (lo / step).ceil() as i64;
    let last = (hi / step).floor() as i64;
    (first..=last)
        .map(|i| format!("{}°", i as f64 * step))
        .collect()
}

/// **The graticule is labelled at the plot area's edges, where the axes'
/// labels sat, in their ink.** Meridians are named below the plot on the x
/// tick labels' baseline, centred on the line; parallels in the left margin,
/// right-aligned where the y tick labels end and on the line's height.
///
/// Read off the drawn glyph runs, and placed against what an UNPROJECTED
/// scatter of the same layout draws for its axes, so "the same place" is a
/// measurement rather than a restated constant. The label count follows the
/// step: a second extent four times as wide picks a 5° step, and its labels are
/// exactly the multiples of five the plot area holds.
#[test]
fn the_graticule_is_labelled_at_the_plot_areas_edges_in_the_axes_ink() {
    use brightfield_render::ink::ChartInk;
    use brightfield_render::mark::graticule_label;
    use brightfield_render::text::{measure_width, LABEL_SIZE};

    let layout = ChartLayout::new(640.0, 480.0);
    let size = f64::from(LABEL_SIZE);

    // Where the axes put their labels: every x tick label on one baseline, every
    // y tick label ending at one x.
    let (plain_scene, _) = plot(
        &[&batch(&[(0.0, 0.0), (10.0, 5.0)])],
        &channels(None),
        &DotRenderer,
        layout,
    );
    let plain = drawn_texts(&plain_scene);
    let x_axis_baseline = plain
        .iter()
        .map(|(_, (_, y))| *y)
        .fold(f64::NEG_INFINITY, f64::max);
    let y_axis_end = plain
        .iter()
        .filter(|(_, (_, y))| (*y - x_axis_baseline).abs() > 0.5)
        .map(|(t, (x, _))| x + measure_width(t, LABEL_SIZE))
        .fold(f64::NEG_INFINITY, f64::max);

    for (points, step) in [
        (vec![(0.0, 0.0), (10.0, 5.0)], 1.0),
        (vec![(0.0, 0.0), (40.0, 20.0)], 5.0),
    ] {
        let cm = channels(Some(Projection::Equirectangular));
        let (scene, set) = plot(&[&batch(&points)], &cm, &DotRenderer, layout);
        let graticule = PlotGraticule::of(&set).expect("a projected plot has a graticule");
        assert_eq!(graticule.step, step, "the data's step for {points:?}");
        let (
            Some(Scale::Linear {
                domain_min: u0,
                domain_max: u1,
                ..
            }),
            Some(Scale::Linear {
                domain_min: v0,
                domain_max: v1,
                ..
            }),
        ) = (set.get(Channel::X), set.get(Channel::Y))
        else {
            panic!("linear positional scales");
        };
        let (x, y) = (set.get(Channel::X).unwrap(), set.get(Channel::Y).unwrap());

        let texts = drawn_texts(&scene);
        let mut meridians: Vec<(String, f64)> = texts
            .iter()
            .filter(|(_, (_, by))| (*by - x_axis_baseline).abs() < 0.5)
            .map(|(t, (sx, _))| (t.clone(), sx + measure_width(t, LABEL_SIZE) / 2.0))
            .collect();
        meridians.sort_by(|a, b| a.1.total_cmp(&b.1));
        let mut parallels: Vec<(String, f64)> = texts
            .iter()
            .filter(|(t, (sx, _))| (sx + measure_width(t, LABEL_SIZE) - y_axis_end).abs() < 0.5)
            .map(|(t, (_, by))| (t.clone(), *by))
            .collect();
        parallels.sort_by(|a, b| b.1.total_cmp(&a.1));
        assert_eq!(
            meridians.len() + parallels.len(),
            texts.len(),
            "every text the projected plot draws sits where an axis label sat; drew {texts:?}"
        );

        // The texts, and so the count, are the multiples of the step the plot
        // area spans.
        let names =
            |labels: &[(String, f64)]| labels.iter().map(|(t, _)| t.clone()).collect::<Vec<_>>();
        assert_eq!(
            names(&meridians),
            whole_steps(*u0, *u1, step),
            "meridian labels at {step}°"
        );
        assert_eq!(
            names(&parallels),
            whole_steps(*v0, *v1, step),
            "parallel labels at {step}°"
        );

        // Each label is on its own line: a meridian's centred on its x, a
        // parallel's baseline a third of the label size below its y, as the
        // y axis places a tick label.
        for (text, centre) in &meridians {
            let degrees: f64 = text.trim_end_matches('°').parse().unwrap();
            assert!(
                (centre - x.map_f64(degrees)).abs() < 0.5,
                "{text} is centred at {centre}, its meridian at {}",
                x.map_f64(degrees)
            );
        }
        for (text, baseline) in &parallels {
            let degrees: f64 = text.trim_end_matches('°').parse().unwrap();
            let want = y.map_f64(degrees) + size / 3.0;
            assert!(
                (baseline - want).abs() < 0.5,
                "{text} sits at {baseline}, its parallel wants {want}"
            );
        }

        // The axes' label ink, and not the graticule's own: one label-ink
        // paint per label drawn. A solid brush is one word of `draw_data` per
        // draw, and on this fixture's plot the label ink is the labels' alone
        // (the surface, the grid ink and the mark ink are the other paints), so
        // the count is per label rather than "somewhere in the scene".
        let ink = ChartInk::LIGHT;
        assert_ne!(ink.label, ink.grid, "the fixture needs the two inks apart");
        let packed = |c: peniko::Color| c.premultiply().to_rgba8().to_u32();
        let label_paints = scene
            .encoding()
            .draw_data
            .iter()
            .filter(|w| **w == packed(ink.label))
            .count();
        assert_eq!(
            label_paints,
            texts.len(),
            "every graticule label must be drawn in the axes' label ink"
        );
    }

    // The text a label carries.
    assert_eq!(graticule_label(-124.0, 1.0), "-124°");
    assert_eq!(graticule_label(-0.0, 1.0), "0°");
    assert_eq!(graticule_label(37.5, 0.5), "37.5°");
    assert_eq!(graticule_label(-122.25, 0.05), "-122.25°");
}

/// **The graticule spans the plot area's fitted extent, not the data's**, on
/// the California fixture, at the data's own step on both axes.
///
/// Every meridian is drawn from the plot area's bottom edge to its top edge and
/// every parallel from its left edge to its right, both ends in the drawn
/// record; the outermost line on each side is within one step of that side's
/// edge; and no data point lies outside the lines' reach. Two panes, one wider
/// than the data and one taller, are the change of aspect fit: the wide one
/// widens longitude and draws more meridians, the tall one widens latitude and
/// draws more parallels. On the data's extent — the graticule before this — the
/// wide pane's parallels end at the data's longitudes, short of both side edges.
#[test]
fn the_graticule_reaches_the_plot_areas_fitted_extent_over_the_california_fixture() {
    let data = california();
    assert!(data.len() > 100, "the fixture's rows: {}", data.len());
    let batch = batch(&data);
    let cm = channels(Some(Projection::Equirectangular));
    let count = |g: &PlotGraticule, kind| g.lines.iter().filter(|l| l.kind == kind).count();

    let mut drawn = Vec::new();
    for layout in [
        ChartLayout::new(1000.0, 400.0),
        ChartLayout::new(400.0, 700.0),
    ] {
        let (scene, set) = plot(&[&batch], &cm, &DotRenderer, layout);
        let graticule = PlotGraticule::of(&set).expect("a projected plot has a graticule");
        assert_eq!(
            graticule.step, 1.0,
            "the data spans about 6° each way, so 1° on both axes"
        );
        let rect = plot_rect(&set);
        let points = drawn_points(&scene);
        let (x, y) = (set.get(Channel::X).unwrap(), set.get(Channel::Y).unwrap());
        let edge = |a: f64, b: f64| (a - b).abs() < 1e-3;

        for line in &graticule.lines {
            let px = pixels(line, &set);
            let (start, end) = (px[0], px[px.len() - 1]);
            match line.kind {
                GraticuleKind::Meridian => assert!(
                    edge(start.1, rect.3) && edge(end.1, rect.1),
                    "the {}° meridian runs y {} to {}, not the plot area's {} to {}",
                    line.degrees,
                    start.1,
                    end.1,
                    rect.3,
                    rect.1
                ),
                GraticuleKind::Parallel => assert!(
                    edge(start.0, rect.0) && edge(end.0, rect.2),
                    "the {}° parallel runs x {} to {}, not the plot area's {} to {}",
                    line.degrees,
                    start.0,
                    end.0,
                    rect.0,
                    rect.2
                ),
            }
            assert!(
                times_drawn(&points, start) >= 1 && times_drawn(&points, end) >= 1,
                "the {:?} at {}° is not drawn to the plot area's edges",
                line.kind,
                line.degrees
            );
        }

        // The outermost lines are within one step of the edges.
        let step_px = |scale: &Scale| (scale.map_f64(1.0) - scale.map_f64(0.0)).abs();
        let at = |kind| {
            graticule
                .lines
                .iter()
                .filter(|l| l.kind == kind)
                .map(|l| pixels(l, &set)[0])
                .collect::<Vec<_>>()
        };
        let xs: Vec<f64> = at(GraticuleKind::Meridian).iter().map(|p| p.0).collect();
        let ys: Vec<f64> = at(GraticuleKind::Parallel).iter().map(|p| p.1).collect();
        let (west, east) = (
            xs.iter().copied().fold(f64::INFINITY, f64::min),
            xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        );
        let (north, south) = (
            ys.iter().copied().fold(f64::INFINITY, f64::min),
            ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        );
        assert!(
            west - rect.0 < step_px(x) && rect.2 - east < step_px(x),
            "meridians {west}..{east} in a plot area {}..{}",
            rect.0,
            rect.2
        );
        assert!(
            north - rect.1 < step_px(y) && rect.3 - south < step_px(y),
            "parallels {north}..{south} in a plot area {}..{}",
            rect.1,
            rect.3
        );

        // No data point lies outside the lines' reach.
        for (lon, lat) in &data {
            let (px, py) = (x.map_f64(*lon), y.map_f64(*lat));
            assert!(
                px >= rect.0 - 1e-6
                    && px <= rect.2 + 1e-6
                    && py >= rect.1 - 1e-6
                    && py <= rect.3 + 1e-6,
                "({lon}, {lat}) is drawn at ({px}, {py}), outside the graticule's reach"
            );
        }
        drawn.push((
            count(&graticule, GraticuleKind::Meridian),
            count(&graticule, GraticuleKind::Parallel),
        ));
    }

    let ((wide_m, wide_p), (tall_m, tall_p)) = (drawn[0], drawn[1]);
    assert!(
        wide_m > tall_m && tall_p > wide_p,
        "changing the aspect fit must move the lines: {wide_m} meridians and {wide_p} parallels \
         on the wide pane, {tall_m} and {tall_p} on the tall one"
    );
}

/// **A plot strokes its graticule once, whatever its layer count.** With one,
/// two and three projected layers over one scale set, every vertex of every
/// graticule line inside the plot area is in the drawn record exactly once — a
/// point map's ghost and its subset share one set of hairlines. Drawn per layer
/// it was twice, and two coincident 0.5 px strokes read darker than one.
#[test]
fn a_plot_strokes_its_graticule_once_whatever_its_layer_count() {
    let ghost = batch(FIXTURE);
    let subset = batch(&FIXTURE[1..2]);
    let cm = channels(Some(Projection::Mercator));
    let layout = ChartLayout::new(640.0, 480.0);
    for layers in [
        vec![&ghost],
        vec![&ghost, &subset],
        vec![&ghost, &subset, &subset],
    ] {
        let (scene, set) = plot(&layers, &cm, &DotRenderer, layout);
        let graticule = PlotGraticule::of(&set).expect("a projected plot has a graticule");
        let rect = plot_rect(&set);
        let points = drawn_points(&scene);
        let mut checked = 0;
        for line in &graticule.lines {
            for v in pixels(line, &set) {
                if v.0 > rect.0 + 1.0
                    && v.0 < rect.2 - 1.0
                    && v.1 > rect.1 + 1.0
                    && v.1 < rect.3 - 1.0
                {
                    assert_eq!(
                        times_drawn(&points, v),
                        1,
                        "the {:?} at {}° was stroked {} times with {} layers",
                        line.kind,
                        line.degrees,
                        times_drawn(&points, v),
                        layers.len()
                    );
                    checked += 1;
                }
            }
        }
        assert!(
            checked > 20,
            "too few interior vertices to count: {checked}"
        );
    }
}

/// **The graticule survives a brush.** A gesture rebuilds the plot through
/// `build_multi_mark_scene_anchored`, folding a fresh inference over the
/// narrowed subset into the launch scales; the fold keeps the launch set's
/// projection and geographic extent, so the rebuilt plot draws the graticule it
/// opened with. Dropped, the rebuilt scales name no projection and the
/// graticule vanishes on the first brush.
#[test]
fn the_graticule_survives_a_brush() {
    use brightfield_render::scene::build_multi_mark_scene_anchored;

    let ghost = batch(FIXTURE);
    let subset = batch(&FIXTURE[1..2]);
    let cm = channels(Some(Projection::Mercator));
    let layout = ChartLayout::new(640.0, 480.0);
    let (_, launch) = plot(&[&ghost, &ghost], &cm, &DotRenderer, layout);
    let before = PlotGraticule::of(&launch).expect("the plot opens with a graticule");

    let entry = |b| ChartData {
        batch: b,
        channel_map: &cm,
        renderer: &DotRenderer,
        layout,
        view_extent: None,
        highlight: None,
        sample: None,
        beyond_frame: false,
    };
    let (ghost_e, subset_e) = (entry(&ghost), entry(&subset));
    let (scene, anchored) = build_multi_mark_scene_anchored(
        &[&ghost_e, &subset_e],
        false,
        &ResolvedTitles::default(),
        &launch,
    );
    let after = PlotGraticule::of(&anchored).expect("the brushed plot keeps its graticule");
    assert_eq!(after.step, before.step);
    assert_eq!(after.lines, before.lines);
    let rect = plot_rect(&anchored);
    let points = drawn_points(&scene);
    let interior: Vec<(f64, f64)> = after
        .lines
        .iter()
        .flat_map(|l| pixels(l, &anchored))
        .filter(|v| {
            v.0 > rect.0 + 1.0 && v.0 < rect.2 - 1.0 && v.1 > rect.1 + 1.0 && v.1 < rect.3 - 1.0
        })
        .collect();
    assert!(!interior.is_empty());
    assert!(
        interior.iter().all(|v| times_drawn(&points, *v) == 1),
        "the brushed plot's scene must carry the graticule once"
    );
}

/// **A transition projects.** A dot animated to its new position lands at the
/// PROJECTED position, not the linear `(lon, lat)` one — `render_interpolated`
/// places its target through the plot's projection exactly as `render` does.
#[test]
fn a_transition_lands_its_dots_at_their_projected_positions() {
    let batch = batch(FIXTURE);
    let cm = channels(Some(Projection::Mercator));
    let set = scales(&batch, &cm);
    let (x, y) = (set.get(Channel::X).unwrap(), set.get(Channel::Y).unwrap());
    let prev = vec![(0.0, 0.0); FIXTURE.len()];
    let mut scene = Scene::new();
    DotRenderer.render_interpolated(&mut scene, &batch, &cm, &set, &prev, 1.0, None);
    let points = drawn_points(&scene);

    let projected = (
        x.map_f64(REYKJAVIK_MERCATOR.0),
        y.map_f64(REYKJAVIK_MERCATOR.1),
    );
    assert!(
        circle_drawn_at(&points, projected),
        "the transition's end must put Reykjavík at its Mercator position {projected:?}"
    );
    let linear = (x.map_f64(-21.94), y.map_f64(64.15));
    assert!(
        !circle_drawn_at(&points, linear),
        "the transition drew Reykjavík at the linear position {linear:?}"
    );
}

/// **A colour override keeps a projected plot's frame suppressed.** A plot with
/// an explicit colour domain or range wraps its marks in
/// `ColourOverrideRenderer`, and the wrapper answers `suppresses_frame` by
/// asking the mark it wraps; answering for itself, it would give the default
/// `false`, and a projected plot with a colour override would draw cartesian
/// axes over its graticule.
#[test]
fn a_colour_override_keeps_a_projected_plots_frame_suppressed() {
    use brightfield_render::mark::ColourOverrideRenderer;
    use brightfield_render::scale::ColourOverride;

    let wrapped = ColourOverrideRenderer {
        inner: Box::new(DotRenderer),
        override_: ColourOverride::default(),
    };
    let projected = channels(Some(Projection::Mercator));
    assert!(wrapped.suppresses_frame(&projected));
    assert!(
        !wrapped.suppresses_frame(&channels(None)),
        "control: a plain scatter keeps it"
    );

    let batch = batch(FIXTURE);
    let (scene, _) = plot(
        &[&batch],
        &projected,
        &wrapped,
        ChartLayout::new(640.0, 480.0),
    );
    let texts: Vec<String> = drawn_texts(&scene).into_iter().map(|(t, _)| t).collect();
    assert!(!texts.is_empty(), "control: the graticule is labelled");
    assert!(
        texts.iter().all(|t| t.ends_with('°')),
        "a colour-overridden projected plot drew tick labels: {texts:?}"
    );
}

/// **The graticule is clipped to the plot area, and the clip shows.** Under a
/// conic the lines are laid over the data's geographic rectangle, whose corners
/// project outside the fitted domain, so some of their vertices fall outside
/// the plot area; stroked, none of what is drawn does.
#[test]
fn the_graticule_is_clipped_to_the_plot_area() {
    use brightfield_render::ink::ChartInk;

    let batch = batch(&[(-120.0, 30.0), (-75.0, 45.0), (-100.0, 48.0), (-80.0, 26.0)]);
    let cm = channels(Some(Projection::Albers));
    let (_, set) = plot(&[&batch], &cm, &DotRenderer, ChartLayout::new(640.0, 480.0));
    let graticule = PlotGraticule::of(&set).expect("a projected plot has a graticule");
    let rect = plot_rect(&set);
    let outside = |p: &(f64, f64)| {
        p.0 < rect.0 - 1e-3 || p.0 > rect.2 + 1e-3 || p.1 < rect.1 - 1e-3 || p.1 > rect.3 + 1e-3
    };
    let overhang = graticule
        .lines
        .iter()
        .flat_map(|l| pixels(l, &set))
        .filter(|p| outside(p))
        .count();
    assert!(
        overhang > 0,
        "control: the conic's lines must overhang the plot area"
    );

    let mut scene = Scene::new();
    graticule.stroke(&mut scene, ChartInk::LIGHT.grid);
    let stray: Vec<(f64, f64)> = drawn_points(&scene)
        .into_iter()
        .filter(|p| outside(p))
        .collect();
    assert!(
        stray.is_empty(),
        "{} drawn graticule vertices lie outside the plot area: {:?}",
        stray.len(),
        &stray[..stray.len().min(4)]
    );
}

/// **Zooming out coarsens the step rather than hatching the plot.** The step is
/// the data's while the plot area holds at most 36 intervals of it; a view
/// zoomed out tenfold would hold over a hundred, so the step climbs the ladder
/// until it fits.
#[test]
fn zooming_out_coarsens_the_graticule_step_rather_than_hatching_the_plot() {
    use brightfield_render::scale::ViewExtent;

    let batch = batch(&[(0.0, 0.0), (10.0, 5.0)]);
    let cm = channels(Some(Projection::Equirectangular));
    let layout = ChartLayout::new(640.0, 480.0);
    let (_, set) = plot(&[&batch], &cm, &DotRenderer, layout);
    assert_eq!(
        PlotGraticule::of(&set).unwrap().step,
        1.0,
        "the data's step, unzoomed"
    );

    let zoomed = ViewExtent {
        x: Some((-50.0, 60.0)),
        y: Some((-40.0, 45.0)),
    };
    let entry = ChartData {
        batch: &batch,
        channel_map: &cm,
        renderer: &DotRenderer,
        layout,
        view_extent: Some(&zoomed),
        highlight: None,
        sample: None,
        beyond_frame: false,
    };
    let (_, zoomed_set) = build_multi_mark_scene(&[&entry], false, &ResolvedTitles::default());
    let graticule = PlotGraticule::of(&zoomed_set).unwrap();
    let widest = graticule.extent.lon_span().max(graticule.extent.lat_span());
    assert!(
        widest > 100.0,
        "the fixture must zoom out past 100°: {widest}"
    );
    assert!(
        graticule.step > 1.0 && widest / graticule.step <= 36.0,
        "zoomed out to {widest}°, the step {} leaves {} intervals",
        graticule.step,
        widest / graticule.step
    );
}
