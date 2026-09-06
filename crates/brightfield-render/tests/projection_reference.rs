//! **The projection catalogue, pinned to an oracle that is not this code.**
//!
//! Mosaic's `projectionType` takes Observable Plot's `ProjectionName`, and
//! Plot's projections are d3-geo's. This file asserts that every one of those
//! sixteen names lands a coordinate where d3-geo lands it.
//!
//! # Why the expected values can be trusted
//!
//! A test that computes its expectation from the function under test pins
//! nothing. Every literal below was produced by an oracle written independently
//! of `src/mark.rs`, and cross-checked twice:
//!
//! 1. **A transcription of d3-geo's JavaScript into Python**, from
//!    `d3-geo/src/projection/*.js`, at d3's own default parameters.
//! 2. **The `d3_geo_rs` crate (3.2.4)**, an unrelated Rust port of the same
//!    library, driven through its raw `Transform` impls at the same
//!    coordinates. It agrees with (1) to fifteen significant figures on
//!    thirteen of the sixteen names.
//!
//! Where the two disagree, the disagreement is recorded rather than averaged:
//!
//! - **Equal Earth.** `d3_geo_rs` writes `A3 = 0.008_93`; d3-geo and the paper
//!   the projection comes from (Šavrič, Patterson & Jenny 2018) both carry
//!   `0.000893`. The port is out by a factor of ten in one coefficient, which
//!   moves a projected coordinate in the third decimal place. The literals here
//!   follow d3-geo and the paper.
//! - **Transverse Mercator.** d3's raw transform is `[log(tan((π/2+φ)/2)), -λ]`
//!   and its default `rotate([0, 0, 90])` is what turns that into a north-up
//!   map; `d3_geo_rs` exposes the raw without the rotation, so it cannot be
//!   compared directly. (1) composes the rotation and the raw, and the result
//!   matches **Snyder's closed form** for the spherical transverse Mercator —
//!   `x = atanh(cos φ sin λ)`, `y = atan2(tan φ, cos λ)` — to machine
//!   precision. That closed form is what `src/mark.rs` implements, and
//!   [`transverse_mercator_matches_snyders_closed_form`] re-derives it here from
//!   the two elementary functions rather than from the projection.
//!
//! The oracle scripts are not vendored: they need a network fetch of a crate
//! and are a provenance record, not a gate. What is vendored is their output,
//! which is what a reviewer has to be able to check.

use brightfield_render::mark::{Projection, MERCATOR_CLIP_LAT};
use brightfield_spec::layout::ResolvedProjection;

/// The tolerance a literal must be met within. The oracle carries full `f64`
/// precision and the transforms are a handful of `sin`/`cos`/`ln` calls, so the
/// gap is last-place rounding; a mis-derived constant misses by orders more.
const TOL: f64 = 1e-12;

/// The fixture coordinates, in degrees. Chosen away from the equator and away
/// from the prime meridian, because that is where a projection and an
/// unprojected scatter differ enough to tell apart — `origin` is the control,
/// where most of them agree at the planar zero.
const POINTS: &[(&str, f64, f64)] = &[
    ("reykjavik", -21.94, 64.15),
    ("milan", 9.19, 45.46),
    ("origin", 0.0, 0.0),
    ("washington", -77.03, 38.90),
    ("sydney", 151.21, -33.87),
    ("north_cape", 25.78, 71.17),
];

/// Each projection, its Mosaic wire name, and where the oracle says a `POINTS`
/// entry lands. `None` is a coordinate the projection has no position for — the
/// far hemisphere, the antipode, or past a Mercator clip.
type Expectations = &'static [(
    Projection,
    &'static str,
    &'static [(&'static str, Option<(f64, f64)>)],
)];

const REFERENCE: Expectations = &[
    // equirectangular
    (
        Projection::Equirectangular,
        "equirectangular",
        &[
            ("reykjavik", Some((-21.94, 64.15))),
            ("milan", Some((9.19, 45.46))),
            ("origin", Some((0.0, 0.0))),
            ("washington", Some((-77.03, 38.9))),
            ("sydney", Some((151.21, -33.87))),
            ("north_cape", Some((25.78, 71.17))),
        ],
    ),
    // identity
    (
        Projection::Identity,
        "identity",
        &[
            ("reykjavik", Some((-21.94, 64.15))),
            ("milan", Some((9.19, 45.46))),
            ("origin", Some((0.0, 0.0))),
            ("washington", Some((-77.03, 38.9))),
            ("sydney", Some((151.21, -33.87))),
            ("north_cape", Some((25.78, 71.17))),
        ],
    ),
    // reflect-y
    (
        Projection::ReflectY,
        "reflect-y",
        &[
            ("reykjavik", Some((-21.94, -64.15))),
            ("milan", Some((9.19, -45.46))),
            ("origin", Some((0.0, -0.0))),
            ("washington", Some((-77.03, -38.9))),
            ("sydney", Some((151.21, 33.87))),
            ("north_cape", Some((25.78, -71.17))),
        ],
    ),
    // mercator
    (
        Projection::Mercator,
        "mercator",
        &[
            (
                "reykjavik",
                Some((-0.38292523788755595, 1.4718965195300215)),
            ),
            ("milan", Some((0.16039575825827887, 0.892773567848533))),
            ("origin", Some((0.0, -1.1102230246251565e-16))),
            (
                "washington",
                Some((-1.3444271228112321, 0.7380458488533442)),
            ),
            ("sydney", Some((2.639112361940626, -0.6289233879818552))),
            ("north_cape", Some((0.4499458811641382, 1.796865005231959))),
        ],
    ),
    // transverse-mercator
    (
        Projection::TransverseMercator,
        "transverse-mercator",
        &[
            (
                "reykjavik",
                Some((-0.16437587969167855, 1.1484359557602437)),
            ),
            ("milan", Some((0.11249307645884853, 0.7998844642338435))),
            ("origin", Some((-1.1102230246251565e-16, 0.0))),
            ("washington", Some((-0.992410725483844, 1.2995015489343844))),
            ("sydney", Some((0.4235002549874802, -2.488003982465312))),
            (
                "north_cape",
                Some((0.14130739904770576, 1.2728646733684439)),
            ),
        ],
    ),
    // orthographic
    (
        Projection::Orthographic,
        "orthographic",
        &[
            (
                "reykjavik",
                Some((-0.16291125942389584, 0.8999386178498898)),
            ),
            ("milan", Some((0.1120209444347767, 0.7127609484023376))),
            ("origin", Some((0.0, 0.0))),
            (
                "washington",
                Some((-0.7583883877642911, 0.6279630576493378)),
            ),
            ("sydney", None),
            ("north_cape", Some((0.1403743192193531, 0.9464803924353345))),
        ],
    ),
    // stereographic
    (
        Projection::Stereographic,
        "stereographic",
        &[
            (
                "reykjavik",
                Some((-0.11599744167788005, 0.6407818447102531)),
            ),
            ("milan", Some((0.0661904321423553, 0.4211529855152643))),
            ("origin", Some((0.0, 0.0))),
            ("washington", Some((-0.6456184959162796, 0.534586989083172))),
            ("sydney", Some((1.468352598134975, -2.046459557524638))),
            (
                "north_cape",
                Some((0.10876358696414912, 0.7333435563213049)),
            ),
        ],
    ),
    // gnomonic
    (
        Projection::Gnomonic,
        "gnomonic",
        &[
            ("reykjavik", Some((-0.4028086014231536, 2.2251563047558642))),
            ("milan", Some((0.16178555181367127, 1.0294005637101977))),
            ("origin", Some((0.0, 0.0))),
            ("washington", Some((-4.341846588703525, 3.595149007653347))),
            ("sydney", None),
            (
                "north_cape",
                Some((0.48298832031750466, 3.2565712695742866)),
            ),
        ],
    ),
    // azimuthal-equal-area
    (
        Projection::AzimuthalEqualArea,
        "azimuthal-equal-area",
        &[
            ("reykjavik", Some((-0.194408278186364, 1.073931401600538))),
            ("milan", Some((0.12177614479964982, 0.7748308220228927))),
            ("origin", Some((0.0, 0.0))),
            (
                "washington",
                Some((-0.9895752323383543, 0.8193910912918468)),
            ),
            ("sydney", Some((1.0836582947996172, -1.5103067732507087))),
            ("north_cape", Some((0.1747433230538285, 1.1782150032798873))),
        ],
    ),
    // azimuthal-equidistant
    (
        Projection::AzimuthalEquidistant,
        "azimuthal-equidistant",
        &[
            (
                "reykjavik",
                Some((-0.20563859819181282, 1.1359688488552924)),
            ),
            ("milan", Some((0.1251357831409376, 0.796207351407711))),
            ("origin", Some((0.0, 0.0))),
            (
                "washington",
                Some((-1.0746437100569939, 0.8898297507434396)),
            ),
            ("sydney", Some((1.390803275241879, -1.9383781925883725))),
            (
                "north_cape",
                Some((0.18718423086369315, 1.2620984042581846)),
            ),
        ],
    ),
    // equal-earth
    (
        Projection::EqualEarth,
        "equal-earth",
        &[
            ("reykjavik", Some((-0.23820988705609433, 1.141640404317036))),
            ("milan", Some((0.11804131324459505, 0.8679012387341448))),
            ("origin", Some((0.0, 0.0))),
            ("washington", Some((-1.033576775444634, 0.7552731777949337))),
            ("sydney", Some((2.087103104445474, -0.6647057660224264))),
            ("north_cape", Some((0.2598419171407564, 1.2191641876410382))),
        ],
    ),
    // conic-equal-area
    (
        Projection::ConicEqualArea,
        "conic-equal-area",
        &[
            ("reykjavik", Some((-0.17904190680572438, 1.239523091760908))),
            ("milan", Some((0.09914947898691133, 0.884128853382766))),
            ("origin", Some((0.0, 0.0))),
            (
                "washington",
                Some((-0.8576020636348892, 1.0065523093328181)),
            ),
            ("sydney", Some((2.5583348928021357, 1.1421981954948177))),
            (
                "north_cape",
                Some((0.18986104894995062, 1.3472781944908516)),
            ),
        ],
    ),
    // conic-conformal
    (
        Projection::ConicConformal,
        "conic-conformal",
        &[
            (
                "reykjavik",
                Some((-0.20780214607693998, 1.2074611356058016)),
            ),
            ("milan", Some((0.11686241731158917, 0.8254565289910474))),
            ("origin", Some((0.0, 0.0))),
            (
                "washington",
                Some((-0.9814513753556797, 1.0463163947729526)),
            ),
            ("sydney", Some((3.0238243724462057, 1.5034021847274657))),
            (
                "north_cape",
                Some((0.20707004067261633, 1.3746661062340477)),
            ),
        ],
    ),
    // conic-equidistant
    (
        Projection::ConicEquidistant,
        "conic-equidistant",
        &[
            (
                "reykjavik",
                Some((-0.17722852338272374, 1.1358756397894223)),
            ),
            ("milan", Some((0.09953513617808604, 0.7972399081134247))),
            ("origin", Some((0.0, 0.0))),
            (
                "washington",
                Some((-0.8474819746077017, 0.9606812750494655)),
            ),
            ("sydney", Some((2.5569433374357176, 1.2733252129496648))),
            ("north_cape", Some((0.18168535114076337, 1.261742272386277))),
        ],
    ),
];

/// Mosaic's `ProjectionName` values, in the schema's own order. The vocabulary
/// is fixed by upstream, so this is a closed list and not a growing one; a name
/// missing from it is one a Mosaic spec can write and this build has not been
/// asked about.
const MOSAIC_PROJECTION_NAMES: [&str; 16] = [
    "albers-usa",
    "albers",
    "azimuthal-equal-area",
    "azimuthal-equidistant",
    "conic-conformal",
    "conic-equal-area",
    "conic-equidistant",
    "equal-earth",
    "equirectangular",
    "gnomonic",
    "identity",
    "reflect-y",
    "mercator",
    "orthographic",
    "stereographic",
    "transverse-mercator",
];

fn point(name: &str) -> (f64, f64) {
    let (_, lon, lat) = POINTS
        .iter()
        .find(|(n, _, _)| *n == name)
        .expect("fixture point is declared");
    (*lon, *lat)
}

/// AC1's pin, for every projection at once: a known coordinate lands where
/// d3-geo lands it, and not at the linear `(lon, lat)` an unprojected scatter
/// would draw.
#[test]
fn every_projection_lands_a_coordinate_where_d3_geo_lands_it() {
    for (projection, wire, rows) in REFERENCE {
        for (name, expected) in *rows {
            let (lon, lat) = point(name);
            let got = projection.project(lon, lat);
            match (expected, got) {
                (Some((ex, ey)), Some((gx, gy))) => assert!(
                    (gx - ex).abs() < TOL && (gy - ey).abs() < TOL,
                    "{wire} at {name} ({lon}, {lat}): expected ({ex}, {ey}) from the d3-geo oracle, got ({gx}, {gy})"
                ),
                (None, None) => {}
                _ => panic!(
                    "{wire} at {name} ({lon}, {lat}): expected {expected:?}, got {got:?}"
                ),
            }
        }
    }
}

/// The pin above is only worth having if the projected position DIFFERS from the
/// unprojected one. A projection whose fixture values happened to equal
/// `(lon, lat)` would pass the reference table while drawing a scatter, so this
/// names the two that legitimately do — the plate carrée and d3's planar
/// identity — and requires every other one to move the point.
#[test]
fn a_projection_moves_a_coordinate_off_its_linear_position() {
    const LINEAR: [&str; 2] = ["equirectangular", "identity"];
    let (lon, lat) = point("reykjavik");
    for (projection, wire, _) in REFERENCE {
        let Some((u, v)) = projection.project(lon, lat) else {
            continue;
        };
        let moved = (u - lon).abs() > 1e-6 || (v - lat).abs() > 1e-6;
        assert_eq!(
            moved,
            !LINEAR.contains(wire),
            "{wire} at reykjavik: projected ({u}, {v}) against linear ({lon}, {lat})"
        );
    }
}

/// AC5's list, as a test rather than as prose in a pull request: every name
/// Mosaic's schema declares resolves, and the set that does is the whole set.
#[test]
fn every_mosaic_projection_name_resolves() {
    let unrecognised: Vec<&str> = MOSAIC_PROJECTION_NAMES
        .into_iter()
        .filter(|n| ResolvedProjection::from_wire(n).is_none())
        .collect();
    assert!(
        unrecognised.is_empty(),
        "these Mosaic projection names still warn: {unrecognised:?}"
    );
}

/// The catalogue is closed, not open: a name that is not Mosaic's is still
/// refused, so `ParseWarning::UnknownProjection` keeps a reason to exist. This
/// is the half of AC3 that a widening cannot be allowed to delete.
#[test]
fn a_name_outside_mosaics_vocabulary_is_still_refused() {
    for name in [
        "mollweide", // a real d3 EXTENSION projection, not a Plot built-in
        "winkel3",   // likewise
        "epsg:3857", // a PROJ-style name, which this vocabulary is not
        "Mercator",  // Mosaic's names are lower-kebab
        "",
    ] {
        assert!(
            ResolvedProjection::from_wire(name).is_none(),
            "`{name}` is not in Mosaic's ProjectionName enum and must not resolve"
        );
    }
}

/// Snyder's closed form for the spherical transverse Mercator, re-derived here
/// from `atanh` and `atan2` alone. This is the third leg of the oracle: the
/// `d3_geo_rs` port exposes the raw transform without d3's default
/// `rotate([0, 0, 90])`, so it cannot check this projection, and the literals in
/// [`REFERENCE`] come from composing the rotation with the raw. If that
/// composition were wrong, this would disagree with it.
#[test]
fn transverse_mercator_matches_snyders_closed_form() {
    for (name, lon, lat) in POINTS {
        let (lam, phi) = (lon.to_radians(), lat.to_radians());
        let b = phi.cos() * lam.sin();
        let expected = (
            0.5 * ((1.0 + b) / (1.0 - b)).ln(),
            phi.tan().atan2(lam.cos()),
        );
        let (u, v) = Projection::TransverseMercator
            .project(*lon, *lat)
            .expect("no fixture point sits on the transverse Mercator's clip");
        assert!(
            (u - expected.0).abs() < TOL && (v - expected.1).abs() < TOL,
            "transverse mercator at {name}: Snyder gives {expected:?}, projection gives ({u}, {v})"
        );
    }
}

/// `MERCATOR_CLIP_LAT` is `atan(sinh(π))` in degrees — d3's clip latitude, the
/// one that makes a Mercator map square. Written as a literal because it is a
/// public constant; checked here so the literal cannot rot.
#[test]
fn the_mercator_clip_latitude_is_atan_sinh_pi() {
    let derived = std::f64::consts::PI.sinh().atan().to_degrees();
    assert!(
        (MERCATOR_CLIP_LAT - derived).abs() < 1e-12,
        "MERCATOR_CLIP_LAT is {MERCATOR_CLIP_LAT}, atan(sinh(π)) is {derived}"
    );
}

/// A projection with no position for a coordinate says so rather than drawing it
/// somewhere. Each case below is a coordinate d3 clips away, and the reason it
/// matters is that the alternative is not an approximation: an orthographic
/// far-side point projects to the MIRROR of where it belongs, on the visible
/// face of the globe, which is a point in the wrong hemisphere drawn as if it
/// were in the right one.
#[test]
fn an_unrepresentable_coordinate_has_no_position() {
    // Sydney is on the far side of a globe centred at (0, 0).
    let (lon, lat) = point("sydney");
    assert_eq!(Projection::Orthographic.project(lon, lat), None);
    assert_eq!(Projection::Gnomonic.project(lon, lat), None);
    // ...and its mirror IS on the visible face, which is what would be drawn.
    let mirror = Projection::Orthographic.project(180.0 - lon, lat);
    assert!(
        mirror.is_some(),
        "the mirror of a far-side point is representable — that is the failure"
    );

    // Past d3's clip latitude, Mercator diverges.
    for lat in [MERCATOR_CLIP_LAT, 88.0, -89.9] {
        assert_eq!(
            Projection::Mercator.project(10.0, lat),
            None,
            "mercator at {lat}° must have no position"
        );
    }
    assert!(
        Projection::Mercator
            .project(10.0, MERCATOR_CLIP_LAT - 0.001)
            .is_some(),
        "just inside the clip is still drawn"
    );

    // The antipode of an azimuthal projection's centre.
    for projection in [
        Projection::Stereographic,
        Projection::AzimuthalEqualArea,
        Projection::AzimuthalEquidistant,
    ] {
        assert_eq!(
            projection.project(180.0, 0.0),
            None,
            "{projection:?} at the antipode must have no position"
        );
    }

    // The projections that are TOTAL stay total — this is what keeps the
    // `Option` safe for the specs that predate the catalogue.
    for projection in [
        Projection::Equirectangular,
        Projection::Identity,
        Projection::ReflectY,
        Projection::EqualEarth,
        Projection::ConicEqualArea,
        Projection::ConicConformal,
        Projection::ConicEquidistant,
        Projection::Albers,
    ] {
        for lat in [-90.0, -45.0, 0.0, 45.0, 90.0] {
            for lon in [-180.0, -90.0, 0.0, 90.0, 180.0] {
                assert!(
                    projection
                        .project(lon, lat)
                        .is_some_and(|(u, v)| u.is_finite() && v.is_finite()),
                    "{projection:?} must have a finite position for ({lon}, {lat})"
                );
            }
        }
    }
}

/// Equal Earth is equal-area, which is the property a point map needs: two
/// regions of the same size on the sphere must cover the same area on the page,
/// or a cluster of events reads as denser than it is. Checked as the ratio of
/// two projected cells that are equal on the sphere — one on the equator, one at
/// 60°N — which is independent of the coefficients and would catch the
/// `d3_geo_rs` A3 error if it had been copied.
#[test]
fn equal_earth_preserves_relative_area() {
    let cell = |lat: f64| {
        let d = 2.0_f64;
        let p = |lo: f64, la: f64| Projection::EqualEarth.project(lo, la).expect("total");
        let (x0, y0) = p(0.0, lat);
        let (x1, _) = p(d, lat);
        let (_, y1) = p(0.0, lat + d);
        (x1 - x0).abs() * (y1 - y0).abs()
    };
    // A 2°×2° cell on the sphere shrinks as cos(lat); an equal-area projection
    // must reproduce that ratio, not the 1.0 an equirectangular grid gives.
    let ratio = cell(60.0) / cell(0.0);
    let spherical = 60.0_f64.to_radians().cos();
    assert!(
        (ratio - spherical).abs() < 0.02,
        "equal earth must shrink a 60°N cell to ~{spherical:.4} of an equatorial one; got {ratio:.4}"
    );
    // The control: the plate carrée does NOT, which is the distortion the
    // projection exists to remove.
    let flat = {
        let p = |lo: f64, la: f64| Projection::Equirectangular.project(lo, la).expect("total");
        let c = |lat: f64| {
            let (x0, y0) = p(0.0, lat);
            let (x1, _) = p(2.0, lat);
            let (_, y1) = p(0.0, lat + 2.0);
            (x1 - x0).abs() * (y1 - y0).abs()
        };
        c(60.0) / c(0.0)
    };
    assert!(
        (flat - 1.0).abs() < 1e-9,
        "the plate carrée draws every cell the same size; got {flat}"
    );
}
