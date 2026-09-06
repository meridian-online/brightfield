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
use brightfield_render::mark::{
    graticule, graticule_step, DotRenderer, GeoExtent, GraticuleKind, MarkRenderer, Projection,
};
use brightfield_render::scale::{infer_scales, Scale, ScaleSet};

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
    let set = scales(&batch, &cm);
    let (Some(x_scale), Some(y_scale)) = (set.get(Channel::X), set.get(Channel::Y)) else {
        panic!("a projected dot mark must have both positional scales");
    };
    let points = drawn_points(&render(&batch, &cm, &set));

    let extent = GeoExtent::new(-21.94, 151.21, -33.87, 64.15);
    let expected = graticule(Projection::Mercator, extent);
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
/// scatter gets is unchanged by any of this.
#[test]
fn an_unprojected_dot_mark_draws_no_graticule() {
    let batch = batch(FIXTURE);
    let plain = channels(None);
    let plain_set = scales(&batch, &plain);
    let plain_points = drawn_points(&render(&batch, &plain, &plain_set));

    let projected = channels(Some(Projection::Mercator));
    let projected_set = scales(&batch, &projected);
    let projected_points = drawn_points(&render(&batch, &projected, &projected_set));

    // Three dots and nothing else: a circle is one path, and no graticule means
    // the encoded geometry is exactly three circles' worth.
    assert!(
        projected_points.len() > plain_points.len(),
        "a projected mark must draw MORE than an unprojected one (the graticule): \
         {} vs {}",
        projected_points.len(),
        plain_points.len()
    );
    assert_eq!(
        plain_points.len() % FIXTURE.len(),
        0,
        "an unprojected mark draws only its dots"
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
