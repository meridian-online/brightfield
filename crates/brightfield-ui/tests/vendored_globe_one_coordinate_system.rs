//! **The vendored globe spec draws its geo mark and its dot mark in ONE
//! coordinate system**, read off the coordinates the scene encoded.
//!
//! `crates/brightfield-spec/vendor/mosaic-specs/yaml/earthquakes-globe.yaml`
//! carries `projectionType: orthographic` at plot level over a `geo` mark, a
//! `sphere` mark and a `dot` mark. It is the only vendored spec that names a
//! projection this build did not always recognise, which makes it the one spec
//! whose picture the widened catalogue can change — so it is the one to pin.
//!
//! This drives the real chain a plot goes through: parse the file on disk, join
//! each mark to its plot the way `pipeline.rs` does (`plot_of_each_mark`),
//! deliver the plot's projection through the one delivery
//! (`ChannelMap::from_mark_in`), infer and augment the shared scales, render,
//! and read `Encoding::path_data`. Three separate seams have to be right for it
//! to pass and any one of them alone will fail it:
//!
//! 1. **the mark seam** — the plot's projection reaching a `dot` mark, not only
//!    a `geo` one. This is the defect the file was written for: with the dot
//!    unprojected, its degree domain unions with the geo mark's orthographic
//!    bbox of ±1, the land collapses to a sliver and the earthquakes spread
//!    across the frame in raw longitude and latitude;
//! 2. **the enum seam** — `From<ResolvedProjection> for Projection` mapping
//!    `Orthographic` to the orthographic and not to some other arm;
//! 3. **the plot seam** — `plot_of_each_mark` joining both marks to the plot
//!    that carries the attribute.
//!
//! Expected positions come from `u = cos φ · sin λ`, `v = sin φ` — the spherical
//! orthographic in two elementary functions, written here rather than taken from
//! `Projection::project`, so the assertion is not the code under test agreeing
//! with itself.

use std::path::PathBuf;
use std::sync::Arc;

use arrow::array::{Array, Float64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;

use brightfield_render::channel::{Channel, ChannelMap};
use brightfield_render::layout::ChartLayout;
use brightfield_render::mark::{default_renderers, find_renderer, MarkRenderer};
use brightfield_render::scale::{Scale, ScaleSet};
use brightfield_render::scene::{build_multi_mark_scene, ChartData};
use brightfield_render::ResolvedTitles;
use brightfield_spec::ast::Mark;
use brightfield_spec::vocab::MarkKind;
use brightfield_spec::{parse_spec, Format};
use brightfield_sql::{collect_marks, plot_of_each_mark};

/// The plot box the scales map onto. Square, so the orthographic disc is not
/// letterboxed by the fit and a coordinate's two pixels are comparable.
const LAYOUT: (f32, f32) = (600.0, 600.0);

/// Degrees to radians, for the oracle below.
const D2R: f64 = std::f64::consts::PI / 180.0;

/// The spherical orthographic, from Snyder's two elementary terms. **Not
/// `Projection::project`** — this is the independent answer the drawn pixels are
/// checked against.
fn orthographic(lon: f64, lat: f64) -> Option<(f64, f64)> {
    let (lam, phi) = (lon * D2R, lat * D2R);
    // The far hemisphere has no position; d3 clips it away.
    (lam.cos() * phi.cos() > 0.0).then(|| (phi.cos() * lam.sin(), phi.sin()))
}

/// Land, as the `geo` mark's GeoJSON: one square straddling the prime meridian
/// (near side, whole) and one at longitude 150° (far side under this
/// projection's default rotation, so it is dropped).
const NEAR_LAND: &str =
    r#"{"type":"Polygon","coordinates":[[[-20,-10],[20,-10],[20,10],[-20,10],[-20,-10]]]}"#;
const FAR_LAND: &str =
    r#"{"type":"Polygon","coordinates":[[[140,-10],[160,-10],[160,10],[140,10],[140,-10]]]}"#;

/// Earthquakes, as the `dot` mark's two coordinate columns. Reykjavik is the
/// far-from-equator one AC1 asks about; Sydney is on the far hemisphere and must
/// not be drawn at all.
const REYKJAVIK: (f64, f64) = (-21.94, 64.15);
const QUITO: (f64, f64) = (-78.47, -0.18);
const SYDNEY: (f64, f64) = (151.21, -33.87);

fn vendored_globe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../brightfield-spec/vendor/mosaic-specs/yaml/earthquakes-globe.yaml")
}

fn geo_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![Field::new("geom", DataType::Utf8, true)]));
    RecordBatch::try_new(
        schema,
        vec![Arc::new(StringArray::from(vec![NEAR_LAND, FAR_LAND]))],
    )
    .expect("geo batch")
}

fn dot_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("longitude", DataType::Float64, false),
        Field::new("latitude", DataType::Float64, false),
    ]));
    let pts = [REYKJAVIK, QUITO, SYDNEY];
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(
                pts.iter().map(|p| p.0).collect::<Vec<_>>(),
            )) as Arc<dyn Array>,
            Arc::new(Float64Array::from(
                pts.iter().map(|p| p.1).collect::<Vec<_>>(),
            )),
        ],
    )
    .expect("dot batch")
}

/// Every coordinate the scene encoded, as pixel pairs.
fn drawn_points(scene: &vello::Scene) -> Vec<(f64, f64)> {
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

/// The dot radius `DotRenderer` draws at, mirrored so a circle's four cardinal
/// points can be looked for.
const DOT_RADIUS: f64 = 4.0;

fn circle_drawn_at(points: &[(f64, f64)], centre: (f64, f64), tol: f64) -> bool {
    [
        (centre.0 + DOT_RADIUS, centre.1),
        (centre.0 - DOT_RADIUS, centre.1),
        (centre.0, centre.1 + DOT_RADIUS),
        (centre.0, centre.1 - DOT_RADIUS),
    ]
    .into_iter()
    .all(|c| {
        points
            .iter()
            .any(|p| (p.0 - c.0).abs() < tol && (p.1 - c.1).abs() < tol)
    })
}

/// Render the vendored globe's plot over the synthetic batches above, through
/// the production join and the production delivery.
fn render_globe() -> (ScaleSet, Vec<(f64, f64)>) {
    let yaml = std::fs::read_to_string(vendored_globe()).expect("the vendored globe spec is there");
    // The claim this whole file rests on: the spec still asks for an
    // orthographic. If the vendored file changes, this line says so rather than
    // the assertions failing for a reason nobody can read.
    assert!(
        yaml.contains("projectionType: orthographic"),
        "the vendored globe spec no longer names `orthographic`; this test is about that spec"
    );
    let parsed = parse_spec(&yaml, Format::Yaml).expect("the vendored globe spec parses");
    let marks: Vec<&Mark> = collect_marks(&parsed.spec);
    let plots = plot_of_each_mark(&parsed.spec);
    let registry = default_renderers();
    let layout = ChartLayout::new(f64::from(LAYOUT.0), f64::from(LAYOUT.1));

    // One entry per mark the plot can draw, batched by kind.
    let geo = geo_batch();
    let dot = dot_batch();
    let mut metas: Vec<(ChannelMap, MarkKind, &RecordBatch)> = Vec::new();
    for (i, mark) in marks.iter().enumerate() {
        let batch = match mark.kind {
            MarkKind::Geo => &geo,
            MarkKind::Dot => &dot,
            // `sphere` is Unimplemented and has no renderer; the engine would
            // hand this composition nothing for it.
            _ => continue,
        };
        metas.push((
            ChannelMap::from_mark_in(mark, plots.get(i).copied().flatten()),
            mark.kind,
            batch,
        ));
    }
    assert_eq!(
        metas.len(),
        2,
        "the globe's drawable marks are one geo and one dot"
    );

    let entries: Vec<ChartData<'_>> = metas
        .iter()
        .map(|(cm, kind, batch)| ChartData {
            batch,
            channel_map: cm,
            renderer: find_renderer(&registry, *kind).expect("a renderer for geo and for dot"),
            layout,
            view_extent: None,
            highlight: None,
            sample: None,
            beyond_frame: false,
        })
        .collect();
    let refs: Vec<&ChartData<'_>> = entries.iter().collect();
    let (scene, scales) = build_multi_mark_scene(&refs, false, &ResolvedTitles::default());
    (scales, drawn_points(&scene))
}

/// The plot's axes are in the orthographic's planar units — the unit disc — and
/// not in degrees.
///
/// This is the assertion the regression fails. With the dot mark unprojected its
/// longitude domain is `[-78.47, 151.21]`, `merge_linear_scale` unions that with
/// the geo mark's `[-0.34, 0.34]`, and the land collapses into three pixels at
/// the centre of a frame scaled for degrees.
#[test]
fn the_globes_axes_are_in_the_projections_planar_units() {
    let (scales, _) = render_globe();
    for channel in [Channel::X, Channel::Y] {
        let Some(Scale::Linear {
            domain_min,
            domain_max,
            ..
        }) = scales.get(channel)
        else {
            panic!("a projected plot has a linear {channel:?} scale");
        };
        assert!(
            domain_min.abs() <= 1.05 && domain_max.abs() <= 1.05,
            "the {channel:?} domain must be the orthographic's unit disc, not degrees: \
             [{domain_min}, {domain_max}]"
        );
    }
    assert_eq!(
        scales.projection(),
        Some(brightfield_render::mark::Projection::Orthographic),
        "the scale set carries the plot's projection, which is what the graticule \
         and the brush read"
    );
}

/// A far-from-equator earthquake lands at its ORTHOGRAPHIC position, and one on
/// the far hemisphere is not drawn at all.
///
/// The expected pixel is the oracle's `(u, v)` mapped through the plot's own
/// scales, so this reads the projection and the frame together — the two things
/// a delivery has to get right for the picture to mean anything.
#[test]
fn an_earthquake_lands_where_the_orthographic_puts_it() {
    let (scales, points) = render_globe();
    let (Some(x_scale), Some(y_scale)) = (scales.get(Channel::X), scales.get(Channel::Y)) else {
        panic!("a projected plot has both positional scales");
    };
    for (name, coord) in [("Reykjavik", REYKJAVIK), ("Quito", QUITO)] {
        let (u, v) = orthographic(coord.0, coord.1).expect("a near-side coordinate has a position");
        let centre = (x_scale.map_f64(u), y_scale.map_f64(v));
        assert!(
            circle_drawn_at(&points, centre, 0.05),
            "{name} must be drawn at its orthographic position {centre:?}"
        );
    }
    // Sydney is on the far hemisphere, so the projection has no position for it
    // and drawing it anywhere would be a lie. The oracle agrees it has none.
    assert!(
        orthographic(SYDNEY.0, SYDNEY.1).is_none(),
        "the oracle puts Sydney on the far side under this projection"
    );
    let linear = (x_scale.map_f64(SYDNEY.0), y_scale.map_f64(SYDNEY.1));
    assert!(
        !circle_drawn_at(&points, linear, 0.05),
        "a far-side earthquake must not be drawn at its raw lon/lat either"
    );
}

/// The land is drawn at its ORTHOGRAPHIC position too, in the same frame and
/// through the same scales the earthquakes are drawn through — and the ring on
/// the far hemisphere is dropped whole rather than chorded across the disc.
///
/// Two coordinate systems on one pair of axes shows up here as land whose
/// drawn corners are nowhere near where the projection puts them. Under the
/// regression the geo mark keeps its orthographic bbox while the scales are
/// fitted to the dot mark's degrees, so the whole square lands within a few
/// pixels of the frame's centre and none of these corners is drawn.
#[test]
fn the_land_is_drawn_where_the_orthographic_puts_it() {
    let (scales, points) = render_globe();
    let (Some(x_scale), Some(y_scale)) = (scales.get(Channel::X), scales.get(Channel::Y)) else {
        panic!("a projected plot has both positional scales");
    };
    let pixel = |lon: f64, lat: f64| {
        let (u, v) = orthographic(lon, lat).expect("near side");
        (x_scale.map_f64(u), y_scale.map_f64(v))
    };
    let drawn = |p: (f64, f64)| {
        points
            .iter()
            .any(|q| (q.0 - p.0).abs() < 0.05 && (q.1 - p.1).abs() < 0.05)
    };

    let corners = [(-20.0, -10.0), (20.0, -10.0), (20.0, 10.0), (-20.0, 10.0)];
    for (lon, lat) in corners {
        let p = pixel(lon, lat);
        assert!(
            drawn(p),
            "the near land's ({lon}, {lat}) corner must be drawn at its projected pixel {p:?}"
        );
    }

    // It occupies a real part of the frame rather than the sliver the
    // regression collapses it to.
    let xs: Vec<f64> = corners
        .iter()
        .map(|(lon, lat)| pixel(*lon, *lat).0)
        .collect();
    let width = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        - xs.iter().copied().fold(f64::INFINITY, f64::min);
    assert!(
        width > 50.0,
        "the near land must occupy a real part of a {}px frame, not a sliver: {width}px",
        LAYOUT.0
    );

    // The far-side ring has an unrepresentable vertex, so `GeoRenderer` declines
    // the whole ring: joining what remains would draw a chord across the globe.
    // Its corners are not drawn at their raw degrees either.
    for (lon, lat) in [(140.0, -10.0), (160.0, 10.0)] {
        assert!(
            orthographic(lon, lat).is_none(),
            "the oracle puts ({lon}, {lat}) on the far side"
        );
        let raw = (x_scale.map_f64(lon), y_scale.map_f64(lat));
        assert!(
            !drawn(raw),
            "a far-side land vertex must not be drawn at its raw lon/lat {raw:?}"
        );
    }
}
