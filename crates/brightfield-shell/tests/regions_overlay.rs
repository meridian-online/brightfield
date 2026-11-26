//! The dense regions of a two-column plot, found off the UI thread and drawn
//! over the plot.
//!
//! Every test reads what a person would see: which plots get regions, when
//! they arrive, that a zoom recomputes them over what is on screen, and that
//! two names never sit on top of each other. The fixture is the earthquake
//! start's panel, over the start's own table, with no GPU.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use brightfield_render::channel::Channel;
use brightfield_render::scale::ViewExtent;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::navigation::NavOutcome;
use brightfield_shell::pipeline::LiveDashboard;
use brightfield_shell::regions::{place_names, qualifying_layer, RegionsWorker};
use brightfield_spec::{parse_spec, Format};

fn start_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/starts/usgs-earthquakes-2025")
        .canonicalize()
        .expect("the earthquake start is in the repository")
}

/// The start's panel, unchanged, over its table: the start's own model step
/// (`models/02_earthquakes.sql`) as a `query:` source reading the start's
/// Parquet, which is the table the panel reads once the start has run.
fn earthquake_spec() -> String {
    let dir = start_dir();
    let panel = std::fs::read_to_string(dir.join("panels/dashboard.yaml")).expect("panel");
    let model = std::fs::read_to_string(dir.join("models/02_earthquakes.sql")).expect("model");
    let parquet = dir.join("data/earthquakes_2025.parquet");
    let select = model
        .trim()
        .trim_start_matches("CREATE OR REPLACE TABLE \"earthquakes\" AS")
        .replace(
            "FROM raw_earthquakes",
            &format!("FROM '{}'", parquet.display()),
        );
    let select: String = select
        .trim()
        .lines()
        .map(|line| format!("      {line}\n"))
        .collect();
    format!("data:\n  earthquakes:\n    query: |\n{select}{panel}")
}

/// The panel's map alone, its grey dot layer over the start's Parquet: a plot
/// whose axes follow a zoom. (On the whole panel the brushed hexbin layer
/// holds the drawn axes at its lattice's extent through a zoom, so the map's
/// frame does not move there.)
fn earthquake_scatter() -> String {
    let parquet = start_dir().join("data/earthquakes_2025.parquet");
    format!(
        "data:\n  earthquakes:\n    file: \"{}\"\nplot:\n  - mark: dot\n    \
         data: {{ from: earthquakes }}\n    x: longitude\n    y: latitude\n\
         width: 760\nheight: 400\n",
        parquet.display()
    )
}

/// A live chart document over `spec`, composed at the spec's own size.
fn doc_over(spec: &str) -> ChartDoc {
    let mut live = LiveDashboard::load_str(spec, None).expect("the spec loads");
    let composed = live.present().expect("the spec composes");
    let mut doc = ChartDoc::headless(composed);
    doc.attach_live(live);
    doc
}

/// Frame the document until no region run is in flight.
fn settle_regions(doc: &mut ChartDoc) {
    let deadline = Instant::now() + Duration::from_secs(60);
    doc.tend_regions(None);
    while doc.regions_pending() {
        assert!(Instant::now() < deadline, "the region run did not finish");
        std::thread::sleep(Duration::from_millis(5));
        doc.tend_regions(None);
    }
}

fn worker(doc: &ChartDoc) -> &RegionsWorker {
    doc.live_dashboard().expect("a live document").regions()
}

/// The `(lo, hi)` the plot's scale on `channel` is drawn at.
fn displayed(doc: &ChartDoc, plot: usize, channel: Channel) -> (f64, f64) {
    let scale = doc.composed.plots[plot]
        .scales
        .get(channel)
        .expect("a positional scale");
    let (a, b) = (
        scale.domain_min().expect("continuous"),
        scale.domain_max().expect("continuous"),
    );
    (a.min(b), a.max(b))
}

#[test]
fn the_earthquake_panel_qualifies_on_its_dot_layer() {
    let spec = parse_spec(&earthquake_spec(), Format::Yaml)
        .expect("parses")
        .spec;
    let doc = doc_over(&earthquake_spec());
    let map = &doc.composed.plots[0].path;
    assert_eq!(
        qualifying_layer(&spec, map),
        Some((0, "longitude".to_string(), "latitude".to_string())),
        "the grey dot layer over the whole table is the one regions describe, \
         not the brushed hexbin above it"
    );
}

#[test]
fn a_bar_or_band_plot_does_not_qualify() {
    let source = earthquake_spec();
    let spec = parse_spec(&source, Format::Yaml).expect("parses").spec;
    let doc = doc_over(&source);
    for plot in &doc.composed.plots[1..] {
        assert_eq!(
            qualifying_layer(&spec, &plot.path),
            None,
            "{} draws bars, so it has no regions",
            plot.path
        );
    }

    // A dot plot whose spec asks for a band scale on an axis.
    let banded = "data:\n  t:\n    - { a: 1, b: 2 }\n    - { a: 3, b: 4 }\n\
                  plot:\n  - mark: dot\n    data: { from: t }\n    x: a\n    y: b\nxScale: band\n";
    let spec = parse_spec(banded, Format::Yaml).expect("parses").spec;
    assert_eq!(qualifying_layer(&spec, "root"), None);

    // A dot plot over a text column: the spec cannot tell, the drawn scale
    // can, and no run starts for it.
    let mut text = String::from("data:\n  t:\n");
    for i in 0..40 {
        text.push_str(&format!("    - {{ a: \"k{}\", b: {i} }}\n", i % 4));
    }
    text.push_str("plot:\n  - mark: dot\n    data: { from: t }\n    x: a\n    y: b\n");
    let mut doc = doc_over(&text);
    settle_regions(&mut doc);
    assert_eq!(
        worker(&doc).started(),
        0,
        "a band axis starts no region run"
    );
}

#[test]
fn regions_arrive_after_the_first_compose_without_blocking_it() {
    let mut doc = doc_over(&earthquake_spec());
    let map = doc.composed.plots[0].path.clone();
    assert!(
        worker(&doc).regions(&map).is_none(),
        "nothing is computed by the compose itself"
    );
    doc.tend_regions(None);
    assert_eq!(worker(&doc).started(), 1, "the frame starts one run");
    assert!(
        worker(&doc).regions(&map).is_none() && doc.regions_pending(),
        "the frame returns with the run still in flight"
    );
    settle_regions(&mut doc);
    let regions = worker(&doc).regions(&map).expect("the run's answer lands");
    assert!(
        !regions.regions.is_empty(),
        "the earthquake map has regions"
    );
    assert_eq!(regions.names.len(), regions.regions.len());
    assert_eq!(regions.outlines.len(), regions.regions.len());
    assert_eq!(regions.anchors.len(), regions.regions.len());
    assert!(
        regions.names.iter().any(Option::is_some),
        "some region of the map is named from its rows: {:?}",
        regions.names
    );
}

#[test]
fn a_result_for_a_stale_extent_is_dropped() {
    let doc = doc_over(&earthquake_spec());
    let first = doc.region_job(0).expect("the map has a job");
    let mut second = doc.region_job(0).expect("the map has a job");
    second.extent.x = (140.0, 170.0);
    second.extent.y = (35.0, 60.0);
    let wanted = second.extent.clone();
    let map = first.plot_path.clone();

    let mut worker = RegionsWorker::default();
    worker.request(first);
    worker.request(second);
    let deadline = Instant::now() + Duration::from_secs(60);
    while worker.pending() {
        assert!(Instant::now() < deadline, "the region run did not finish");
        std::thread::sleep(Duration::from_millis(5));
        worker.poll();
    }
    assert_eq!(worker.started(), 2);
    assert_eq!(
        worker.superseded(),
        1,
        "the first run was stopped for the second"
    );
    let kept = worker
        .regions(&map)
        .expect("the second run's answer is kept");
    assert_eq!(
        kept.extent, wanted,
        "only the newer extent's answer is kept"
    );
}

#[test]
fn a_settled_zoom_recomputes_over_the_view() {
    let mut doc = doc_over(&earthquake_scatter());
    let map = doc.composed.plots[0].path.clone();
    settle_regions(&mut doc);
    let wide = worker(&doc).regions(&map).expect("wide regions").clone();

    assert!(doc.zoom_view(4.0), "the zoom settles");
    settle_regions(&mut doc);
    let zoomed = worker(&doc).regions(&map).expect("zoomed regions");
    assert_eq!(zoomed.extent.x, displayed(&doc, 0, Channel::X));
    assert_eq!(zoomed.extent.y, displayed(&doc, 0, Channel::Y));
    assert_ne!(zoomed.extent, wide.extent, "the zoom moved the grid");
    assert_ne!(
        (&zoomed.regions, &zoomed.names),
        (&wide.regions, &wide.names),
        "the regions were found again over the view"
    );
}

#[test]
fn a_zoom_onto_empty_space_draws_no_regions() {
    let mut doc = doc_over(&earthquake_scatter());
    let map = doc.composed.plots[0].path.clone();
    settle_regions(&mut doc);
    // Open ocean off South America's east coast, where no quake of 2025
    // struck.
    let outcome = NavOutcome {
        extent: ViewExtent {
            x: Some((-45.0, -40.0)),
            y: Some((-45.0, -40.0)),
        },
        refused: Vec::new(),
    };
    assert!(doc.note_navigation(0, &outcome));
    doc.settle_navigation();
    doc.pump_navigation();
    settle_regions(&mut doc);
    let regions = worker(&doc).regions(&map).expect("an answer, empty");
    assert_eq!(regions.extent.x, displayed(&doc, 0, Channel::X));
    assert!(regions.regions.is_empty(), "no rows, no regions");
    assert!(place_names(&doc.composed.plots[0], regions).is_empty());
}

#[test]
fn two_names_never_overlap() {
    let mut doc = doc_over(&earthquake_spec());
    settle_regions(&mut doc);
    let plot = &doc.composed.plots[0];
    let regions = worker(&doc).regions(&plot.path).expect("regions");
    let placed = place_names(plot, regions);
    assert!(
        placed.len() >= 2,
        "the wide map places more than one name: {placed:?}"
    );
    for (i, a) in placed.iter().enumerate() {
        for b in &placed[i + 1..] {
            let apart = a.rect.x1 <= b.rect.x0
                || b.rect.x1 <= a.rect.x0
                || a.rect.y1 <= b.rect.y0
                || b.rect.y1 <= a.rect.y0;
            assert!(apart, "{:?} and {:?} overlap", a.text, b.text);
        }
    }
}
