//! **A spec that asks for an axis to hold still gets one.**
//!
//! `xDomain: Fixed` / `yDomain: Fixed` is Mosaic's instruction that a plot's
//! frame of reference must survive the dashboard being filtered around it. The
//! request was parsed and stored and nothing read it, so the pinned axis
//! re-derived itself from the rows left after each gesture — the reader saw a
//! chart redraw at a new scale where its author asked for one that held.
//!
//! Assertions about a domain read `PlotHandle::scales`, which is the set the
//! plot's ticks, gridlines and mark positions were drawn from and the set a
//! brush inverts through.
//!
//! A pinned case is worth pairing with the same spec unpinned. A domain that
//! held could be a domain nothing moved, and a test that only watched the
//! pinned arm would pass just as happily on a fixture whose filter never
//! narrowed anything. The unpinned arm is what makes the pinned arm mean
//! something — and it is also the evidence that a spec asking for no pin still
//! behaves exactly as it did.

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_render::channel::Channel;
use brightfield_render::scale::{Scale, ScaleSet, ViewExtent};
use brightfield_shell::pipeline::{Composed, LiveDashboard};
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::ast::Component;
use brightfield_spec::layout::{collect_plot_nodes, resolve_fixed_domains};
use brightfield_spec::{parse_spec, Format, Spec};
use brightfield_sql::ir::ScalarValue;

use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Reading a drawn scale
// ---------------------------------------------------------------------------

/// A continuous channel's drawn `(min, max)`, or `None` when the channel
/// resolved to something else.
fn linear_domain(scales: &ScaleSet, channel: Channel) -> Option<(f64, f64)> {
    match scales.get(channel)? {
        Scale::Linear {
            domain_min,
            domain_max,
            ..
        } => Some((*domain_min, *domain_max)),
        _ => None,
    }
}

/// A categorical channel's drawn category ORDER — the list whose index gives
/// each category its slot along the axis.
fn band_categories(scales: &ScaleSet, channel: Channel) -> Option<Vec<String>> {
    match scales.get(channel)? {
        Scale::Band { categories, .. } => Some(categories.clone()),
        _ => None,
    }
}

/// The plot at `index`, insisting it exists rather than indexing blind.
fn plot_scales(composed: &Composed, index: usize) -> &ScaleSet {
    assert!(
        composed.plots.len() > index,
        "fixture check: the dashboard composed {} plots, wanted more than {index}",
        composed.plots.len()
    );
    &composed.plots[index].scales
}

/// Drive one interval selection into `live` and re-composite.
fn brush(live: &mut LiveDashboard, from: &str, column: &str, lo: f64, hi: f64) -> Composed {
    live.apply(Interaction::Select {
        name: "brush".to_string(),
        contributor: ComponentPath(from.to_string()),
        predicate: SqlPredicate::Interval {
            column: column.to_string(),
            lo: ScalarValue::Float(lo),
            hi: ScalarValue::Float(hi),
            meta: None,
        },
    })
    .expect("the brush re-composites")
}

// ---------------------------------------------------------------------------
// A categorical axis keeps its slots
// ---------------------------------------------------------------------------

/// A scatter to brush on, and a bar chart of the same table filtered by that
/// brush. `PIN` marks where the pin goes so the two arms differ by that line
/// and nothing else.
const BARS_TEMPLATE: &str = r"
params:
  brush: { select: crossfilter }
data:
  readings:
    - { site: Alder,  temp: 2,  load: 30 }
    - { site: Birch,  temp: 6,  load: 18 }
    - { site: Cedar,  temp: 10, load: 45 }
    - { site: Dogwood, temp: 14, load: 22 }
    - { site: Elm,    temp: 18, load: 12 }
    - { site: Fir,    temp: 22, load: 38 }
hconcat:
  - plot:
      - mark: dot
        data: { from: readings }
        x: temp
        y: load
      - select: intervalX
        as: $brush
    width: 320
    height: 240
  - plot:
      - mark: barY
        data: { from: readings, filterBy: $brush }
        x: site
        y: load
    width: 320
    height: 240
PIN
";

fn bars_spec(pin: &str) -> String {
    BARS_TEMPLATE.replace("PIN", pin)
}

/// **A categorical axis keeps its slots.** A filter that empties four of six
/// bars leaves the
/// pinned axis holding all six slots, in their original order.
#[test]
fn a_pinned_band_axis_keeps_every_slot_a_filter_empties() {
    let mut live = LiveDashboard::load_str(&bars_spec("    xDomain: Fixed"), None)
        .expect("the pinned spec loads live");
    let before = live.present().expect("first composite");
    let at_rest = band_categories(plot_scales(&before, 1), Channel::X)
        .expect("the bar chart's x axis is categorical");
    assert_eq!(
        at_rest.len(),
        6,
        "fixture check: every site is on the axis before the brush, got {at_rest:?}"
    );

    let filtered = brush(&mut live, &before.plots[0].path, "temp", 0.0, 8.0);
    let after = band_categories(plot_scales(&filtered, 1), Channel::X)
        .expect("the filtered bar chart still has a categorical x axis");

    assert_eq!(
        after, at_rest,
        "the pin asked this axis to hold its slots while the filter emptied \
         them; it re-derived itself from the two sites left instead"
    );
}

/// **The same spec without the pin.** The axis re-derives
/// from the rows drawn, exactly as it did before the pin existed. This is what
/// makes the assertion above a claim about the pin rather than about the
/// fixture.
#[test]
fn an_unpinned_band_axis_still_closes_the_gap_a_filter_leaves() {
    let mut live =
        LiveDashboard::load_str(&bars_spec(""), None).expect("the unpinned spec loads live");
    let before = live.present().expect("first composite");
    let at_rest = band_categories(plot_scales(&before, 1), Channel::X)
        .expect("the bar chart's x axis is categorical");

    let filtered = brush(&mut live, &before.plots[0].path, "temp", 0.0, 8.0);
    let after = band_categories(plot_scales(&filtered, 1), Channel::X)
        .expect("the filtered bar chart still has a categorical x axis");

    assert!(
        after.len() < at_rest.len(),
        "with no pin the axis is inferred from the drawn rows, so the emptied \
         sites leave it — {at_rest:?} became {after:?}"
    );
    assert!(
        at_rest.starts_with(&after[..]),
        "fixture check: the brush keeps a leading run of sites, so the two \
         lists differ only by what it removed — {at_rest:?} vs {after:?}"
    );
}

/// **The pin drawn, not the pin reported.** The tests above read
/// `plot_scales`, which is `PlotHandle::scales` — the scale set the pipeline
/// reports it drew from. `apply_pinned_domains` (`scene.rs`) could move to
/// run after `draw_multi_mark_scene` instead of before it, and those
/// assertions would keep passing: the returned `ScaleSet` would keep
/// carrying six categories while the returned `Scene`, which none of them
/// inspect, carried ink for the two categories the filter left rather than
/// six.
///
/// `Composed::scene` is the single composited Vello scene placed on the
/// page, so reading it reads what a viewer sees rather than what the
/// pipeline reports it drew from. `compute_band_ticks` draws one tick, with
/// one label glyph, per category, so a pinned filtered scene that keeps six
/// slots carries more glyph runs than an unpinned filtered scene the axis
/// narrowed to two. Applying the pin after drawing collapses that gap, so
/// this assertion — reading the drawn scene rather than the reported scale
/// set — reddens when `apply_pinned_domains` and `draw_multi_mark_scene`
/// swap order inside `build_multi_mark_scene_pinned`.
#[test]
fn a_pinned_band_axis_draws_the_glyphs_its_slots_promise() {
    let mut pinned_live = LiveDashboard::load_str(&bars_spec("    xDomain: Fixed"), None)
        .expect("the pinned spec loads live");
    let pinned_before = pinned_live.present().expect("first composite");
    let pinned_filtered = brush(
        &mut pinned_live,
        &pinned_before.plots[0].path,
        "temp",
        0.0,
        8.0,
    );

    let mut unpinned_live =
        LiveDashboard::load_str(&bars_spec(""), None).expect("the unpinned spec loads live");
    let unpinned_before = unpinned_live.present().expect("first composite");
    let unpinned_filtered = brush(
        &mut unpinned_live,
        &unpinned_before.plots[0].path,
        "temp",
        0.0,
        8.0,
    );

    // The scale sets, not the scenes: a fixture check that the two specs
    // still disagree exactly as the tests above already established, so a
    // failure below is about drawn ink and not about the fixture drifting.
    let pinned_slots = band_categories(plot_scales(&pinned_filtered, 1), Channel::X)
        .expect("the pinned bar chart's x axis is categorical");
    let unpinned_slots = band_categories(plot_scales(&unpinned_filtered, 1), Channel::X)
        .expect("the unpinned bar chart's x axis is categorical");
    assert!(
        pinned_slots.len() > unpinned_slots.len(),
        "fixture check: the pinned scale set should report more slots than \
         the unpinned one — {pinned_slots:?} vs {unpinned_slots:?}"
    );

    let pinned_glyphs = pinned_filtered.scene.encoding().resources.glyph_runs.len();
    let unpinned_glyphs = unpinned_filtered
        .scene
        .encoding()
        .resources
        .glyph_runs
        .len();
    assert!(
        pinned_glyphs > unpinned_glyphs,
        "the pinned scale set reports {} slots against the unpinned {}, so the \
         drawn scene should carry more tick-label glyphs too — pinned scene \
         had {pinned_glyphs}, unpinned had {unpinned_glyphs}",
        pinned_slots.len(),
        unpinned_slots.len(),
    );
}

/// The plot node path of the bar chart's neighbour, read off a throwaway
/// dashboard so the test below can brush before it has composed anything.
fn brushable_path(spec_source: &str) -> String {
    let mut probe = LiveDashboard::load_str(spec_source, None).expect("the probe spec loads live");
    probe.present().expect("probe composite").plots[0]
        .path
        .clone()
}

/// **The capture moment, trap included.** Mosaic fixes a domain after the
/// first render, on whatever data the marks then hold, so a plot whose first
/// render is already filtered pins the FILTERED domain. brightfield copies
/// that: a spec renders the same way here as it does upstream, which is the one
/// thing the instruction exists for.
///
/// The first composition this dashboard ever performs is the filtered one — no
/// unfiltered `present` precedes it — and what it pins is what it saw.
#[test]
fn the_pin_takes_the_first_composition_even_when_that_one_is_already_filtered() {
    let source = bars_spec("    xDomain: Fixed");
    let path = brushable_path(&source);

    let mut live = LiveDashboard::load_str(&source, None).expect("the pinned spec loads live");
    let first = brush(&mut live, &path, "temp", 0.0, 8.0);
    let pinned = band_categories(plot_scales(&first, 1), Channel::X)
        .expect("the bar chart's x axis is categorical");
    assert_eq!(
        pinned,
        vec!["Alder".to_string(), "Birch".to_string()],
        "the first composition drew two sites, so those two are what is pinned"
    );

    // Clearing the brush brings every row back. The axis does NOT widen to
    // them: it is pinned to what the first render held, and the first render
    // was filtered.
    let cleared = brush(&mut live, &path, "temp", -100.0, 100.0);
    let after = band_categories(plot_scales(&cleared, 1), Channel::X)
        .expect("the widened bar chart still has a categorical x axis");
    assert_eq!(
        after, pinned,
        "the pin was taken from the filtered first render; widening the filter \
         re-derived the axis instead of holding it"
    );
}

// ---------------------------------------------------------------------------
// A pinned axis still answers to the reader
// ---------------------------------------------------------------------------

/// **The one brightfield-local rule in `deviations.yaml` DEV-0005.** `Fixed`
/// declines to move when the DASHBOARD moves. A pan or zoom is the reader
/// moving the frame deliberately, and a pin that outranked it would make a
/// plot silently refuse to navigate.
#[test]
fn a_pinned_axis_still_moves_when_the_reader_navigates_it() {
    let mut live = LiveDashboard::load_str(&bars_spec("    yDomain: Fixed"), None)
        .expect("the pinned spec loads live");
    let before = live.present().expect("first composite");
    let pinned = linear_domain(plot_scales(&before, 1), Channel::Y)
        .expect("the bar chart's y axis is continuous");

    let navigated = (pinned.0, pinned.1 / 2.0);
    let path = before.plots[1].path.clone();
    live.set_view_extent(
        &path,
        ViewExtent {
            x: None,
            y: Some(navigated),
        },
    );
    let after = live
        .present()
        .expect("re-composite at the navigated extent");
    let drawn = linear_domain(plot_scales(&after, 1), Channel::Y)
        .expect("the navigated y axis is still continuous");

    assert_eq!(
        drawn, navigated,
        "the reader navigated this axis; the pin put it back to {pinned:?}"
    );

    // And the pin is still the one the first composition took. Resetting the
    // navigation returns the axis to it — a pin re-read from each composition
    // would by now be holding the navigated extent, and the reset would leave
    // the reader stuck at a frame they asked to leave.
    live.set_view_extent(&path, ViewExtent { x: None, y: None });
    let reset = live.present().expect("re-composite with navigation reset");
    let restored = linear_domain(plot_scales(&reset, 1), Channel::Y)
        .expect("the reset y axis is still continuous");
    assert_eq!(
        restored, pinned,
        "resetting the navigation returns the axis to the domain pinned at the \
         first composition; it came back at {restored:?}"
    );
}

// ---------------------------------------------------------------------------
// The vendored corpus
// ---------------------------------------------------------------------------

/// The vendored upstream spec exercised here, verbatim apart from the
/// substitution below.
const VENDORED: &str = "../brightfield-spec/vendor/mosaic-specs/yaml/crossfilter.yaml";

/// A stand-in for the vendored spec's `flights` table, which reads a parquet
/// file this repository does not carry.
///
/// `delay` and `time` rise together, so an interval on one narrows the other —
/// which is what makes the cross-filter observable at all.
const STAND_IN_TABLE: &str = r"
data:
  flights:
    - { delay: -30, time: 0 }
    - { delay: -20, time: 3 }
    - { delay: -10, time: 6 }
    - { delay: 0,   time: 9 }
    - { delay: 10,  time: 12 }
    - { delay: 20,  time: 15 }
    - { delay: 40,  time: 18 }
    - { delay: 60,  time: 21 }
    - { delay: 80,  time: 23 }
";

/// The vendored spec, with its unreachable data source replaced by
/// [`STAND_IN_TABLE`]. Everything the pin turns on — the plots, their marks,
/// their interactors and their `xDomain: Fixed` attributes — is what the
/// vendored file declares, parsed by the ordinary parser.
fn vendored_crossfilter() -> Spec {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(VENDORED);
    let source = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let mut spec = parse_spec(&source, Format::Yaml)
        .unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
        .spec;
    spec.data = parse_spec(STAND_IN_TABLE, Format::Yaml)
        .expect("the stand-in table parses")
        .spec
        .data;
    spec
}

/// Every plot in `spec`, in composition order.
fn plots_of(spec: &mut Spec) -> Vec<&mut brightfield_spec::ast::PlotNode> {
    fn walk<'a>(
        component: &'a mut Component,
        out: &mut Vec<&'a mut brightfield_spec::ast::PlotNode>,
    ) {
        match component {
            Component::Plot(p) => out.push(p),
            Component::HConcat(c) | Component::VConcat(c) => {
                for item in &mut c.items {
                    walk(item, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Some(root) = spec.root.as_mut() {
        walk(root, &mut out);
    }
    out
}

/// The x domain the vendored spec's SECOND plot draws at rest and after a
/// brush on the first. Crossfilter resolution excludes a plot from its own
/// contribution, so the brushed plot is not the one under test.
fn vendored_second_plot_x_domain(spec: Spec) -> ((f64, f64), (f64, f64)) {
    let mut live = LiveDashboard::load(spec, None).expect("the vendored spec loads live");
    let before = live.present().expect("first composite");
    let at_rest =
        linear_domain(plot_scales(&before, 1), Channel::X).expect("a continuous x axis at rest");

    let filtered = brush(&mut live, &before.plots[0].path, "delay", -30.0, -5.0);
    let after = linear_domain(plot_scales(&filtered, 1), Channel::X)
        .expect("a continuous x axis after the brush");
    (at_rest, after)
}

/// **The instruction under test is the vendored corpus's own.** It is the one
/// the vendored upstream
/// corpus carries, read off the file rather than retyped.
#[test]
fn the_vendored_spec_declares_the_pin_this_reads() {
    let mut spec = vendored_crossfilter();
    let plots = plots_of(&mut spec);
    assert_eq!(
        plots.len(),
        2,
        "fixture check: the vendored spec composes two plots"
    );
    for (i, plot) in plots.iter().enumerate() {
        let pinned = brightfield_spec::layout::resolve_fixed_domains(plot);
        assert!(
            pinned.x,
            "plot {i} of the vendored spec declares xDomain: Fixed; the resolver read {pinned:?}"
        );
    }
}

/// **A continuous axis holds, on a vendored spec.** The upstream cross-filter
/// dashboard: brushing one
/// histogram leaves the other's pinned x axis exactly where it was.
#[test]
fn the_vendored_pinned_histogram_holds_its_x_domain_under_a_cross_filter() {
    let (at_rest, after) = vendored_second_plot_x_domain(vendored_crossfilter());
    assert_eq!(
        after, at_rest,
        "the vendored spec pins this axis with xDomain: Fixed; the cross-filter \
         moved it from {at_rest:?} to {after:?}"
    );
}

/// **The mutation guard for the test above.** The same vendored spec with
/// its `xDomain` attributes dropped — the state of the tree before this pin
/// existed — still re-derives the axis from the rows the filter left.
///
/// Without this arm, the assertion above would also pass on a stand-in table
/// whose cross-filter narrowed nothing.
#[test]
fn the_same_vendored_spec_unpinned_lets_the_cross_filter_move_its_x_domain() {
    let mut spec = vendored_crossfilter();
    for plot in plots_of(&mut spec) {
        plot.attributes.shift_remove("xDomain");
    }
    let (at_rest, after) = vendored_second_plot_x_domain(spec);
    assert!(
        after.1 < at_rest.1,
        "with the pin removed the axis follows the drawn rows, so the \
         cross-filter narrows it — {at_rest:?} stayed {after:?}"
    );
}

// ---------------------------------------------------------------------------
// plotDefaults reaches the plots that declare it
// ---------------------------------------------------------------------------

/// The vendored SPLOM spec's own `data/penguins.parquet` source, which this
/// repository does not carry, replaced by a stand-in with the five columns
/// its marks read: `bill_length`, `bill_depth`, `flipper_length`,
/// `body_mass` and `species`.
const PENGUINS_STAND_IN: &str = r"
data:
  penguins:
    - { bill_length: 39.1, bill_depth: 18.7, flipper_length: 181, body_mass: 3750, species: Adelie }
    - { bill_length: 46.5, bill_depth: 17.9, flipper_length: 192, body_mass: 3500, species: Chinstrap }
    - { bill_length: 50.0, bill_depth: 16.3, flipper_length: 230, body_mass: 5700, species: Gentoo }
    - { bill_length: 38.2, bill_depth: 20.0, flipper_length: 190, body_mass: 3900, species: Adelie }
    - { bill_length: 49.0, bill_depth: 19.5, flipper_length: 210, body_mass: 4300, species: Chinstrap }
    - { bill_length: 47.5, bill_depth: 14.2, flipper_length: 215, body_mass: 5000, species: Gentoo }
";

/// The vendored SPLOM file's own source text, unmodified.
fn vendored_splom_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../brightfield-spec/vendor/mosaic-specs/yaml/splom.yaml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

/// `source` with its top-level `plotDefaults:` block deleted — a text-level
/// mutation, not a post-parse one: the merge this card adds runs during
/// parsing, so clearing `Spec::plot_defaults` after the fact would not undo
/// it. Deletes the `plotDefaults:` line and every line indented under it.
fn without_plot_defaults(source: &str) -> String {
    let mut out = Vec::new();
    let mut in_block = false;
    for line in source.lines() {
        if line == "plotDefaults:" {
            in_block = true;
            continue;
        }
        if in_block {
            if line.is_empty() || line.starts_with(' ') || line.starts_with('\t') {
                continue;
            }
            in_block = false;
        }
        out.push(line);
    }
    out.join("\n")
}

/// `source`, parsed, with its unreachable `data/penguins.parquet` source
/// replaced by [`PENGUINS_STAND_IN`].
fn splom_spec_from(source: &str) -> Spec {
    let mut spec = parse_spec(source, Format::Yaml)
        .unwrap_or_else(|e| panic!("parse: {e}"))
        .spec;
    spec.data = parse_spec(PENGUINS_STAND_IN, Format::Yaml)
        .expect("the stand-in table parses")
        .spec
        .data;
    spec
}

/// The vendored SPLOM spec, with its unreachable parquet source replaced by
/// [`PENGUINS_STAND_IN`]. Every `plotDefaults` key — `xDomain: Fixed`,
/// `yDomain: Fixed` and `colorDomain: Fixed` among them — is what the
/// vendored file declares, parsed by the ordinary parser; none of its 16
/// plots sets an `xDomain` or `yDomain` of its own.
fn vendored_splom() -> Spec {
    splom_spec_from(&vendored_splom_source())
}

/// **AC2.** The vendored `splom.yaml` declares `xDomain: Fixed` and
/// `yDomain: Fixed` under `plotDefaults` and nowhere else — no plot sets
/// either key itself. Loaded through `LiveDashboard::load`, every one of its
/// 16 plots is pinned on both axes.
#[test]
fn the_vendored_splom_spec_pins_both_axes_from_plot_defaults() {
    let spec = vendored_splom();
    let live = LiveDashboard::load(spec, None).expect("the SPLOM spec loads live");
    let plots = collect_plot_nodes(live.spec());
    assert_eq!(
        plots.len(),
        16,
        "fixture check: the SPLOM composes 16 plots"
    );
    for (at, plot) in plots {
        let pinned = resolve_fixed_domains(plot);
        assert!(
            pinned.x && pinned.y,
            "{at}: splom.yaml declares xDomain/yDomain: Fixed under plotDefaults \
             and this plot sets neither itself; the resolver read {pinned:?}"
        );
    }
}

/// **The mutation guard for the test above.** The same vendored spec with its
/// `plotDefaults` block deleted from the source — the state of the tree
/// before this card's merge existed — pins nothing, because no plot in the
/// fixture carries its own `xDomain`/`yDomain`.
#[test]
fn unsetting_the_splom_plot_defaults_block_removes_the_pin() {
    let source = without_plot_defaults(&vendored_splom_source());
    let spec = splom_spec_from(&source);
    assert!(
        spec.plot_defaults.is_empty(),
        "fixture check: the plotDefaults block was not actually removed"
    );
    let live = LiveDashboard::load(spec, None).expect("the unpinned SPLOM spec loads live");
    let plots = collect_plot_nodes(live.spec());
    assert_eq!(
        plots.len(),
        16,
        "fixture check: the SPLOM composes 16 plots"
    );
    for (at, plot) in plots {
        let pinned = resolve_fixed_domains(plot);
        assert!(
            pinned.is_empty(),
            "{at}: with plotDefaults unset and no per-plot xDomain/yDomain, \
             nothing should be pinned; the resolver read {pinned:?}"
        );
    }
}

/// **AC1's third precedence pair, sourced from `plotDefaults` instead of a
/// plot's own attributes.** The same rule the earlier per-plot-pin test in
/// this file exercises: a reader's pan or zoom outranks a pin whether that
/// pin came from the plot itself or, as here, only reached the plot through
/// the whole-bag `plotDefaults` merge — `resolve_fixed_domains` reads
/// `plot.attributes` alone and cannot tell the two apart, so nothing
/// downstream can treat them differently either.
#[test]
fn a_plot_defaults_sourced_pin_still_moves_when_the_reader_navigates_it() {
    let source = r"
params:
  brush: { select: crossfilter }
plotDefaults:
  yDomain: Fixed
data:
  readings:
    - { site: Alder,  temp: 2,  load: 30 }
    - { site: Birch,  temp: 6,  load: 18 }
    - { site: Cedar,  temp: 10, load: 45 }
    - { site: Dogwood, temp: 14, load: 22 }
    - { site: Elm,    temp: 18, load: 12 }
    - { site: Fir,    temp: 22, load: 38 }
hconcat:
  - plot:
      - mark: dot
        data: { from: readings }
        x: temp
        y: load
      - select: intervalX
        as: $brush
    width: 320
    height: 240
  - plot:
      - mark: barY
        data: { from: readings, filterBy: $brush }
        x: site
        y: load
    width: 320
    height: 240
";
    let mut live =
        LiveDashboard::load_str(source, None).expect("the plotDefaults-pinned spec loads live");
    let before = live.present().expect("first composite");

    let bar_chart = collect_plot_nodes(live.spec())
        .into_iter()
        .find(|(at, _)| at.ends_with("hconcat[1]"))
        .expect("the bar chart plot node")
        .1;
    assert!(
        resolve_fixed_domains(bar_chart).y,
        "fixture check: the bar chart should be pinned on y through \
         plotDefaults before navigation is even tried"
    );

    let pinned = linear_domain(plot_scales(&before, 1), Channel::Y)
        .expect("the bar chart's y axis is continuous");

    let navigated = (pinned.0, pinned.1 / 2.0);
    let path = before.plots[1].path.clone();
    live.set_view_extent(
        &path,
        ViewExtent {
            x: None,
            y: Some(navigated),
        },
    );
    let after = live
        .present()
        .expect("re-composite at the navigated extent");
    let drawn = linear_domain(plot_scales(&after, 1), Channel::Y)
        .expect("the navigated y axis is still continuous");

    assert_eq!(
        drawn, navigated,
        "the reader navigated this axis; the plotDefaults-sourced pin put it \
         back to {pinned:?}"
    );
}
