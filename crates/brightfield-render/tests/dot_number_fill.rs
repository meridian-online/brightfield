//! **A dot whose `fill` names a number column paints each point along a ramp.**
//!
//! Generic column inference types a number fill `Linear`, which no mark's paint
//! reads, and `DotRenderer` built no ramp of its own — so a dot bound to
//! a house-value column drew its points in the default ink and the picture did
//! not change when the column did. These tests read what the renderer *drew*, as
//! `tests/mode_blind_ink.rs` does: the colours vello encoded into `draw_data`,
//! one word per filled circle, in row order. A test that inspected the scale
//! alone could not see a paint that did not consult it.
//!
//! The oracle for a ramped point is the ramp's own end stops
//! ([`SequentialScheme::stops`]) and its own [`Scale::map_continuous`]; the
//! oracle for a categorical or literal fill is the scale or literal the spec
//! wrote. Colours are compared *as encoded* — [`packed`] pushes an expected
//! colour through the same `Scene::fill` the renderer uses, so the comparison
//! does not depend on how vello packs a word.

use std::sync::Arc;

use arrow::array::{Float64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use kurbo::{Affine, Circle};
use peniko::{Color, Fill};
use vello::Scene;

use brightfield_render::channel::{Channel, ChannelMap};
use brightfield_render::ink::ChartInk;
use brightfield_render::layout::ChartLayout;
use brightfield_render::mark::{
    configured_renderer, default_renderers_at, DotRenderer, MarkRenderer,
};
use brightfield_render::scale::{infer_scales, Scale, ScaleSet, SequentialScheme};
use brightfield_render::scene::{build_multi_mark_scene, ChartData};
use brightfield_render::ResolvedTitles;
use brightfield_spec::vocab::MarkKind;

const X_RANGE: (f64, f64) = (40.0, 600.0);
const Y_RANGE: (f64, f64) = (440.0, 40.0);

/// The ghost's grey — a literal that must survive as it is spelled.
const GHOST: Color = Color::new([0.6, 0.6, 0.6, 1.0]);

/// A batch of `n` points on a diagonal with a number column `v`.
fn number_batch(values: &[Option<f64>]) -> RecordBatch {
    let n = values.len();
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("v", DataType::Float64, true),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(
                (0..n).map(|i| i as f64).collect::<Vec<_>>(),
            )),
            Arc::new(Float64Array::from(
                (0..n).map(|i| (i * 2) as f64).collect::<Vec<_>>(),
            )),
            Arc::new(Float64Array::from(values.to_vec())),
        ],
    )
    .expect("fixture batch")
}

/// The same diagonal with a string column `g`.
fn string_batch(groups: &[&str]) -> RecordBatch {
    let n = groups.len();
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("g", DataType::Utf8, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(
                (0..n).map(|i| i as f64).collect::<Vec<_>>(),
            )),
            Arc::new(Float64Array::from(
                (0..n).map(|i| (i * 2) as f64).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(groups.to_vec())),
        ],
    )
    .expect("fixture batch")
}

/// A dot's channels: `x`, `y`, and `fill` bound to a column when named.
fn column_channels(fill: Option<&str>) -> ChannelMap {
    let mut cm = ChannelMap::new();
    cm.insert(Channel::X, "x".to_string());
    cm.insert(Channel::Y, "y".to_string());
    if let Some(col) = fill {
        cm.insert(Channel::Fill, col.to_string());
    }
    cm
}

/// A dot's channels with `fill` a colour literal, as the ghost layer writes it.
fn literal_channels(colour: Color) -> ChannelMap {
    let mut cm = column_channels(None);
    cm.insert_colour(Channel::Fill, colour);
    cm
}

/// Infer and augment exactly as the scene builders do for one layer.
fn scales_of(batch: &RecordBatch, cm: &ChannelMap) -> ScaleSet {
    let mut set = infer_scales(batch, cm, X_RANGE, Y_RANGE);
    DotRenderer::default().augment_scales(&mut set, batch, cm, X_RANGE, Y_RANGE);
    set
}

/// What `DotRenderer` drew: the colour word of each filled circle, in row order.
fn drawn(batch: &RecordBatch, cm: &ChannelMap, scales: &ScaleSet) -> Vec<u32> {
    let mut scene = Scene::new();
    DotRenderer::default().render(&mut scene, batch, cm, scales, None);
    scene.encoding().draw_data.to_vec()
}

/// A colour as the scene encodes it, by drawing one circle in it.
fn packed(colour: Color) -> u32 {
    let mut scene = Scene::new();
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        colour,
        None,
        &Circle::new((0.0, 0.0), 1.0),
    );
    let words: Vec<u32> = scene.encoding().draw_data.to_vec();
    assert_eq!(words.len(), 1, "one solid fill encodes one colour word");
    words[0]
}

fn ramp_at(scales: &ScaleSet, value: f64) -> u32 {
    let scale = scales.get(Channel::Fill).expect("a fill scale");
    packed(Color::new(scale.map_continuous(value)))
}

/// **AC1, the paint.** Each point takes the colour its value maps to along a
/// sequential ramp, the ends of the ramp are the scheme's own end stops, and a
/// point whose value is null draws in the null ink — not in a colour the ramp
/// could produce, and not in the default ink a number fill drew before.
#[test]
fn a_number_fill_paints_each_point_along_the_ramp_and_a_null_in_the_null_ink() {
    let values = [Some(0.0), Some(50.0), Some(100.0), None, Some(25.0)];
    let batch = number_batch(&values);
    let cm = column_channels(Some("v"));
    let scales = scales_of(&batch, &cm);

    match scales.get(Channel::Fill) {
        Some(Scale::Sequential {
            domain_min,
            domain_max,
            ..
        }) => {
            assert_eq!(
                (*domain_min, *domain_max),
                (0.0, 100.0),
                "a non-negative column anchors at zero and ends at its maximum"
            );
        }
        other => panic!("a number fill must build a Sequential fill scale, got {other:?}"),
    }

    let ink = ChartInk::LIGHT;
    let stops = SequentialScheme::default().stops();
    let colours = drawn(&batch, &cm, &scales);
    assert_eq!(colours.len(), 5, "one circle per row");

    assert_eq!(
        colours[0],
        packed(Color::new(*stops.first().expect("stops"))),
        "the column's minimum is the ramp's first stop"
    );
    assert_eq!(
        colours[2],
        packed(Color::new(*stops.last().expect("stops"))),
        "the column's maximum is the ramp's last stop"
    );
    assert_eq!(
        colours[1],
        ramp_at(&scales, 50.0),
        "the midpoint's own colour"
    );
    assert_eq!(colours[4], ramp_at(&scales, 25.0), "a quarter's own colour");
    assert_ne!(
        colours[1], colours[4],
        "two different values drew one colour — the ramp is flat"
    );
    assert_eq!(
        colours[3],
        packed(ink.null),
        "a null value draws in the null ink"
    );
    assert!(
        colours
            .iter()
            .enumerate()
            .all(|(i, c)| i == 3 || *c != packed(ink.mark_default)),
        "a valued point drew in the default ink, which is what a number fill drew before"
    );
}

/// A layer per entry through the scene builder the dashboard uses, returning
/// the plot's scales — where the layers' `augment_scales` calls run against one
/// shared set.
fn plot_scales(layers: &[(&RecordBatch, &ChannelMap)]) -> ScaleSet {
    let dot = DotRenderer::default();
    let entries: Vec<ChartData<'_>> = layers
        .iter()
        .map(|(batch, cm)| ChartData {
            batch,
            channel_map: cm,
            renderer: &dot,
            layout: ChartLayout::new(640.0, 480.0),
            view_extent: None,
            highlight: None,
            sample: None,
            beyond_frame: false,
        })
        .collect();
    let refs: Vec<&ChartData<'_>> = entries.iter().collect();
    build_multi_mark_scene(&refs, false, &ResolvedTitles::default()).1
}

/// **AC2.** On a two-layer tile whose first dot carries a literal fill and
/// whose second a number column, the first layer's points keep the literal ink
/// and the second's take the ramp — and the same holds with the layers in the
/// other order, where the literal layer's `augment_scales` runs *after* the
/// ramp's and must leave it standing.
#[test]
fn a_literal_ghost_keeps_its_ink_beside_a_number_fill_that_takes_the_ramp() {
    let ghost = number_batch(&[Some(1.0), Some(2.0), Some(3.0), Some(4.0)]);
    let subset = number_batch(&[Some(10.0), Some(20.0), Some(30.0)]);
    let ghost_cm = literal_channels(GHOST);
    let subset_cm = column_channels(Some("v"));

    for ghost_first in [true, false] {
        let layers: Vec<(&RecordBatch, &ChannelMap)> = if ghost_first {
            vec![(&ghost, &ghost_cm), (&subset, &subset_cm)]
        } else {
            vec![(&subset, &subset_cm), (&ghost, &ghost_cm)]
        };
        let scales = plot_scales(&layers);

        let order = if ghost_first {
            "ghost first"
        } else {
            "ghost last"
        };
        assert!(
            matches!(scales.get(Channel::Fill), Some(Scale::Sequential { .. })),
            "{order}: the plot's fill scale is not a ramp: {:?}",
            scales.get(Channel::Fill)
        );

        let ghost_colours = drawn(&ghost, &ghost_cm, &scales);
        assert_eq!(ghost_colours.len(), 4, "{order}: one circle per ghost row");
        assert!(
            ghost_colours.iter().all(|c| *c == packed(GHOST)),
            "{order}: the ghost left its literal ink"
        );

        let subset_colours = drawn(&subset, &subset_cm, &scales);
        let expect: Vec<u32> = [10.0, 20.0, 30.0]
            .iter()
            .map(|v| ramp_at(&scales, *v))
            .collect();
        assert_eq!(
            subset_colours, expect,
            "{order}: the second layer's points are not on the ramp"
        );
        assert_ne!(
            subset_colours[0], subset_colours[2],
            "{order}: the ramp is flat across the subset's values"
        );
        assert!(
            subset_colours.iter().all(|c| *c != packed(GHOST)),
            "{order}: a ramped point drew in the ghost's ink"
        );
    }
}

/// **AC3, a string column.** A dot filled by a string column still paints each
/// point by its category, through the categorical scale, and builds no ramp.
#[test]
fn a_string_fill_still_paints_by_category_and_builds_no_ramp() {
    let batch = string_batch(&["a", "b", "a", "c"]);
    let cm = column_channels(Some("g"));
    let scales = scales_of(&batch, &cm);

    let Some(scale @ Scale::Colour { .. }) = scales.get(Channel::Fill) else {
        panic!(
            "a string fill must keep its categorical scale, got {:?}",
            scales.get(Channel::Fill)
        );
    };
    let by_category = |cat: &str| {
        packed(Color::new(
            scale.map_colour(cat).expect("a category the scale holds"),
        ))
    };
    assert_eq!(
        drawn(&batch, &cm, &scales),
        vec![
            by_category("a"),
            by_category("b"),
            by_category("a"),
            by_category("c")
        ],
        "a point's colour is its category's"
    );
}

/// **AC3, a literal and no fill.** A literal fill paints the points in that
/// literal and leaves the plot with no fill scale; a dot with no fill channel
/// draws the default ink.
#[test]
fn a_literal_fill_and_no_fill_paint_as_they_did() {
    let batch = number_batch(&[Some(1.0), Some(5.0), Some(9.0)]);

    let cm = literal_channels(GHOST);
    let scales = scales_of(&batch, &cm);
    assert!(
        scales.get(Channel::Fill).is_none(),
        "a literal fill builds no fill scale, got {:?}",
        scales.get(Channel::Fill)
    );
    assert_eq!(
        drawn(&batch, &cm, &scales),
        vec![packed(GHOST); 3],
        "a literal fill is that colour on each row"
    );

    let cm = column_channels(None);
    let scales = scales_of(&batch, &cm);
    assert!(scales.get(Channel::Fill).is_none());
    assert_eq!(
        drawn(&batch, &cm, &scales),
        vec![packed(ChartInk::LIGHT.mark_default); 3],
        "a dot with no fill draws the default ink"
    );
}

/// **AC1, the animated draw.** A transition between two states draws through
/// `render_interpolated`, a second copy of the per-row loop; it must paint the
/// same ramp as `render` or a dot would change colour scheme mid-animation.
#[test]
fn the_interpolated_draw_paints_the_same_ramp_as_the_still_one() {
    let batch = number_batch(&[Some(0.0), Some(50.0), Some(100.0), None]);
    let cm = column_channels(Some("v"));
    let scales = scales_of(&batch, &cm);

    let mut scene = Scene::new();
    DotRenderer::default().render_interpolated(&mut scene, &batch, &cm, &scales, &[], 1.0, None);
    let interpolated: Vec<u32> = scene.encoding().draw_data.to_vec();

    let still = drawn(&batch, &cm, &scales);
    assert_eq!(interpolated.len(), 4, "one circle per row");
    assert_eq!(
        interpolated, still,
        "the two draws disagree on a point's colour"
    );
    assert_ne!(
        interpolated[0], interpolated[2],
        "the ramp is flat across the column's ends"
    );
    assert_eq!(
        interpolated[3],
        packed(ChartInk::LIGHT.null),
        "a null draws null ink"
    );
}

/// The schemes a plot's `colorScheme` can name, as the renderer lists them.
const SCHEMES: [SequentialScheme; 5] = SequentialScheme::ALL;

/// The dot kinds the registry builds a `DotRenderer` for.
const DOT_KINDS: [MarkKind; 4] = [
    MarkKind::Dot,
    MarkKind::DotX,
    MarkKind::DotY,
    MarkKind::Circle,
];

/// What `renderer` made of a number-column fill: the fill scale's stops and the
/// colour word of each circle, for `values` on the fixture diagonal.
fn painted_by(renderer: &dyn MarkRenderer, values: &[Option<f64>]) -> (Vec<[f32; 4]>, Vec<u32>) {
    let batch = number_batch(values);
    let cm = column_channels(Some("v"));
    let mut set = infer_scales(&batch, &cm, X_RANGE, Y_RANGE);
    renderer.augment_scales(&mut set, &batch, &cm, X_RANGE, Y_RANGE);
    let stops = match set.get(Channel::Fill) {
        Some(Scale::Sequential { stops, .. }) => stops.clone(),
        other => panic!("a number fill must build a Sequential fill scale, got {other:?}"),
    };
    let mut scene = Scene::new();
    renderer.render(&mut scene, &batch, &cm, &set, None);
    (stops, scene.encoding().draw_data.to_vec())
}

/// **AC1, the paint, at each scheme.** A dot built at a scheme builds its fill
/// ramp from that scheme's stops and paints the column's minimum and maximum in
/// the scheme's first and last stop, so the legend, which reads that ramp, is
/// the same stops. The default and viridis draw what a dot drew before.
#[test]
fn a_dot_built_at_a_scheme_paints_along_that_schemes_ramp() {
    let values = [Some(0.0), Some(50.0), Some(100.0)];
    let (default_stops, default_colours) = painted_by(&DotRenderer::default(), &values);
    assert_eq!(
        default_stops,
        SequentialScheme::Viridis.stops(),
        "a dot built with no scheme paints viridis, as it did"
    );
    for scheme in SCHEMES {
        let (stops, colours) = painted_by(
            &DotRenderer {
                scheme,
                ..DotRenderer::default()
            },
            &values,
        );
        let name = scheme.wire_name();
        assert_eq!(
            stops,
            scheme.stops(),
            "{name}: the ramp is the scheme's stops"
        );
        assert_eq!(
            colours[0],
            packed(Color::new(*scheme.stops().first().expect("stops"))),
            "{name}: the column's minimum is the scheme's first stop"
        );
        assert_eq!(
            colours[2],
            packed(Color::new(*scheme.stops().last().expect("stops"))),
            "{name}: the column's maximum is the scheme's last stop"
        );
        if scheme == SequentialScheme::Viridis {
            assert_eq!(
                colours, default_colours,
                "viridis draws what the default drew"
            );
        } else {
            assert_ne!(colours, default_colours, "{name} drew viridis");
        }
    }
}

/// **AC1, the seam.** `configured_renderer` builds a scheme-carrying dot for
/// each of the four dot kinds, and `default_renderers_at` hands the scheme to
/// the registry's dot entries — the registry the shell draws a plot through.
#[test]
fn the_seams_carry_a_scheme_to_each_dot_kind() {
    let values = [Some(0.0), Some(100.0)];
    let registry = default_renderers_at(SequentialScheme::Blues);
    for kind in DOT_KINDS {
        let configured = configured_renderer(kind, SequentialScheme::Blues, None, None, None)
            .unwrap_or_else(|| panic!("{kind:?}: configured_renderer built no renderer"));
        let (stops, _) = painted_by(configured.as_ref(), &values);
        assert_eq!(
            stops,
            SequentialScheme::Blues.stops(),
            "{kind:?}: configured_renderer's dot is not at the scheme"
        );
        let entry = registry
            .iter()
            .find(|(k, _)| *k == kind)
            .unwrap_or_else(|| panic!("{kind:?}: no registry entry"));
        let (stops, _) = painted_by(entry.1.as_ref(), &values);
        assert_eq!(
            stops,
            SequentialScheme::Blues.stops(),
            "{kind:?}: default_renderers_at's dot is not at the scheme"
        );
    }
}
