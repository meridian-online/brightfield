//! **Every region a fixture's plot is given, and its name, as a committed
//! table** — held ahead of any picture of it.
//!
//! The regions are computed, not authored: a change to the density estimate,
//! the merge rule or the naming thresholds can reorganise them without any
//! spec changing. A pixel snapshot shows that only as a red image, and
//! `UPDATE_SNAPSHOTS=1` would record it without a word. These tables name
//! what moved instead: one line per region, with its peak, its size, its share
//! of the rows, its bounding box, its name and where the name is written. Each
//! is compared with its file under `tests/regions/`, so a run whose regions
//! moved fails here, before a snapshot of the same plot is drawn.
//!
//! Regenerate the tables with `UPDATE_REGION_TABLES=1 cargo test -p
//! brightfield-shell --test region_baseline`, then read every line of the
//! diff: a region that moved should have moved for a reason.
//!
//! No FineType bundle sits beside a `cargo test` binary, so every column here
//! is typed by its storage type and distinct count, the path the gate runs.
//! `region_names_match_with_finetype` checks the typed path names the same
//! regions the same, where a bundle is available.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use brightfield_render::scale::ViewExtent;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::data_file;
use brightfield_shell::navigation::NavOutcome;
use brightfield_shell::pipeline::LiveDashboard;
use brightfield_shell::region_names::NamingRules;
use brightfield_shell::regions::{qualifying_layer, KnownColumn, PlotRegions, RegionJob};
use brightfield_spec::{parse_spec, Format};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn earthquake_dir() -> PathBuf {
    root()
        .join("../../examples/starts/usgs-earthquakes-2025")
        .canonicalize()
        .expect("the earthquake start is in the repository")
}

/// The earthquake start's panel over its table: the start's own model step
/// (`models/02_earthquakes.sql`) as a `query:` source reading the start's
/// Parquet, which is the table the panel reads once the start has run, as
/// `tests/regions_overlay.rs` builds it.
fn earthquake_panel() -> String {
    let dir = earthquake_dir();
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

/// The panel's hexbin layer on the map, as the panel writes it.
const HEXBIN_LAYER: &str = "      - mark: hexbin
        data: { from: earthquakes, filterBy: $sel }
        x: longitude
        y: latitude
        fill: { count: }
        binWidth: 14
";

/// The panel with its map's hexbin layer taken out, for the zoomed table.
///
/// A zoom on the whole panel does not move the map's drawn axes: the hexbin
/// layer holds them at its lattice's full extent, and the regions are found
/// over the extent the plot is drawn at, so the zoomed table would be the wide
/// one again. Without the hexbin layer the map's axes follow the zoom, and the
/// table is of the view a reader zoomed to. The grey dot layer the regions
/// describe, its axis titles and the rest of the panel are unchanged.
fn earthquake_panel_without_hexbin() -> String {
    let panel = earthquake_panel();
    assert!(
        panel.contains(HEXBIN_LAYER),
        "the panel's hexbin layer is not written as this test expects"
    );
    panel.replacen(HEXBIN_LAYER, "", 1)
}

/// Palmer Penguins as a scatter of bill length against bill depth, over the
/// start's own model step (`models/02_penguins.sql`), which reads the CSV's
/// `NA` as a missing value so the two columns are numbers.
fn penguins_scatter() -> String {
    let dir = root()
        .join("../../examples/starts/palmer-penguins")
        .canonicalize()
        .expect("the penguins start is in the repository");
    let model = std::fs::read_to_string(dir.join("models/02_penguins.sql")).expect("model");
    let csv = dir.join("data/penguins.csv");
    let select = model
        .trim()
        .trim_start_matches("CREATE OR REPLACE TABLE \"penguins\" AS")
        .replace(
            "FROM raw_penguins",
            &format!("FROM read_csv('{}')", csv.display()),
        );
    format!(
        "data:\n  penguins:\n    query: |\n      {}\nplot:\n  - mark: dot\n    \
         data: {{ from: penguins }}\n    x: bill_length_mm\n    y: bill_depth_mm\n\
         xLabel: bill length (mm)\nyLabel: bill depth (mm)\nwidth: 640\nheight: 400\n",
        select.trim()
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

/// The first plot of `doc` that has regions, by its index.
fn region_plot(doc: &ChartDoc, spec: &str) -> usize {
    let spec = parse_spec(spec, Format::Yaml).expect("parses").spec;
    doc.composed
        .plots
        .iter()
        .position(|p| qualifying_layer(&spec, &p.path).is_some())
        .expect("a plot with regions")
}

/// The earthquake map's job at its default view.
fn earthquake_wide_job() -> RegionJob {
    let spec = earthquake_panel();
    let doc = doc_over(&spec);
    doc.region_job(region_plot(&doc, &spec))
        .expect("the map has a job")
}

/// The earthquake map's job zoomed to x 140–170, y 35–60, off Kamchatka and
/// Japan's north, reached as a reader reaches it: a navigation that settles
/// and re-queries.
fn earthquake_zoomed_job() -> RegionJob {
    let spec = earthquake_panel_without_hexbin();
    let mut doc = doc_over(&spec);
    let plot = region_plot(&doc, &spec);
    let outcome = NavOutcome {
        extent: ViewExtent {
            x: Some((140.0, 170.0)),
            y: Some((35.0, 60.0)),
        },
        refused: Vec::new(),
    };
    assert!(doc.note_navigation(plot, &outcome), "the map zooms");
    doc.settle_navigation();
    doc.pump_navigation();
    let job = doc.region_job(plot).expect("the zoomed map has a job");
    assert_eq!(
        (job.extent.x, job.extent.y),
        ((140.0, 170.0), (35.0, 60.0)),
        "the map is drawn at the extent it was zoomed to"
    );
    job
}

/// The penguin scatter's job at its default view.
fn penguins_job() -> RegionJob {
    let spec = penguins_scatter();
    let doc = doc_over(&spec);
    doc.region_job(region_plot(&doc, &spec))
        .expect("the scatter has a job")
}

/// A private copy of a committed table under `tests/data`, so the generated
/// spec the open writes beside it never lands in the source tree.
fn own_copy(name: &str) -> PathBuf {
    let committed = root().join("tests/data").join(name);
    let bytes =
        std::fs::read(&committed).unwrap_or_else(|e| panic!("read {}: {e}", committed.display()));
    let thread = std::thread::current();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("region_baseline")
        .join(thread.name().unwrap_or("main").replace("::", "-"));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("make {}: {e}", dir.display()));
    let copy = dir.join(name);
    if std::fs::read(&copy).ok().as_deref() != Some(bytes.as_slice()) {
        std::fs::write(&copy, &bytes).unwrap_or_else(|e| panic!("write {}: {e}", copy.display()));
    }
    copy
}

/// The California Housing sample opened as `tests/dashboard_baseline.rs`
/// opens it, a data file, and the job for its point map, with the columns the
/// open profiled as the window hands them over.
fn housing_job() -> RegionJob {
    let path = own_copy("california_housing_sample.csv");
    let chosen = path.to_str().expect("a UTF-8 path");
    let data_file::OpenedFile {
        mut live,
        composed,
        dashboard,
        protocol,
        ..
    } = data_file::open(chosen).unwrap_or_else(|e| panic!("open {chosen}: {e}"));
    live.set_region_columns(
        protocol
            .profiles
            .iter()
            .map(|profile| KnownColumn {
                profile: profile.clone(),
                representation: dashboard.representation(&profile.name),
            })
            .collect(),
    );
    let plot = composed
        .plots
        .iter()
        .find(|p| qualifying_layer(live.spec(), &p.path).is_some())
        .expect("the point map has regions");
    live.region_job(plot).expect("the point map has a job")
}

/// `v` to six significant figures, with no trailing noise and no `-0`.
fn sig6(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return if v.is_finite() {
            "0".into()
        } else {
            v.to_string()
        };
    }
    let magnitude = v.abs().log10().floor() as i32;
    let decimals = usize::try_from((5 - magnitude).max(0)).unwrap_or(0);
    let text = format!("{v:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    };
    if text == "-0" {
        "0".into()
    } else {
        text
    }
}

/// One line per region: its index, its peak cell and that cell's centre, its
/// cell count, its share of the rows, its bounding box `[x0, x1] × [y0, y1]`,
/// its name (`-` when it has none) and where the name is written.
fn region_table(found: &PlotRegions) -> String {
    let grid = &found.grid;
    let width = (grid.x.1 - grid.x.0) / grid.cols as f64;
    let height = (grid.y.1 - grid.y.0) / grid.rows as f64;
    let x_edge = |c: usize| grid.x.0 + c as f64 * width;
    let y_edge = |r: usize| grid.y.0 + r as f64 * height;
    let mut out = format!(
        "# {} against {} over x [{}, {}], y [{}, {}], {} by {} cells\n",
        found.y,
        found.x,
        sig6(found.extent.x.0),
        sig6(found.extent.x.1),
        sig6(found.extent.y.0),
        sig6(found.extent.y.1),
        grid.cols,
        grid.rows,
    );
    out.push_str("# index | peak cell (x, y) | cells | share | box | name | anchor\n");
    for (i, region) in found.regions.iter().enumerate() {
        let (col, row) = (region.peak % grid.cols, region.peak / grid.cols);
        let cols = region.cells.iter().map(|c| c % grid.cols);
        let rows = region.cells.iter().map(|c| c / grid.cols);
        let (c0, c1) = (cols.clone().min().unwrap_or(0), cols.max().unwrap_or(0));
        let (r0, r1) = (rows.clone().min().unwrap_or(0), rows.max().unwrap_or(0));
        let (ax, ay) = found.anchors[i];
        let name = found.names[i].as_deref().unwrap_or("-");
        writeln!(
            out,
            "{i} | {} ({}, {}) | {} | {:.4} | [{}, {}] x [{}, {}] | {name} | ({}, {})",
            region.peak,
            sig6(x_edge(col) + width / 2.0),
            sig6(y_edge(row) + height / 2.0),
            region.cells.len(),
            region.share,
            sig6(x_edge(c0)),
            sig6(x_edge(c1 + 1)),
            sig6(y_edge(r0)),
            sig6(y_edge(r1 + 1)),
            sig6(ax),
            sig6(ay),
        )
        .expect("a String takes a write");
    }
    out
}

fn table_path(name: &str) -> PathBuf {
    root().join("tests/regions").join(format!("{name}.txt"))
}

/// Compare `found`'s table with the committed `tests/regions/<name>.txt`, or
/// write it there under `UPDATE_REGION_TABLES=1`.
fn assert_table(name: &str, found: &PlotRegions) {
    let table = region_table(found);
    let path = table_path(name);
    if std::env::var_os("UPDATE_REGION_TABLES").is_some() {
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("make tests/regions");
        std::fs::write(&path, &table).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        return;
    }
    let committed = committed(&path);
    assert!(
        committed == table,
        "the regions of {name} moved; read the difference, and record it with \
         UPDATE_REGION_TABLES=1 only if it moved for a reason.\n\
         committed ({}):\n{committed}\nfound:\n{table}",
        path.display()
    );
}

fn committed(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e} (record it with UPDATE_REGION_TABLES=1)",
            path.display()
        )
    })
}

/// Every name's anchor lies inside its region's bounding box, so no name is
/// written away from the cluster it names.
fn assert_anchors_inside(found: &PlotRegions) {
    let grid = &found.grid;
    let width = (grid.x.1 - grid.x.0) / grid.cols as f64;
    let height = (grid.y.1 - grid.y.0) / grid.rows as f64;
    for (i, region) in found.regions.iter().enumerate() {
        let (ax, ay) = found.anchors[i];
        let inside = region.cells.iter().any(|&c| {
            let (col, row) = ((c % grid.cols) as f64, (c / grid.cols) as f64);
            let (x0, y0) = (grid.x.0 + col * width, grid.y.0 + row * height);
            (x0..=x0 + width).contains(&ax) && (y0..=y0 + height).contains(&ay)
        });
        assert!(
            inside,
            "region {i}'s name is anchored at ({ax}, {ay}), on none of its cells"
        );
    }
}

fn run(job: RegionJob) -> PlotRegions {
    job.run().expect("the region queries run")
}

#[test]
fn the_earthquake_map_at_its_default_view() {
    let found = run(earthquake_wide_job());
    assert_table("earthquakes_wide", &found);
    assert_anchors_inside(&found);
    assert!(
        found.names.iter().flatten().any(|n| n.contains("Russia")),
        "the cluster off Kamchatka is named from its rows, which say Russia: {:?}",
        found.names
    );
}

#[test]
fn the_earthquake_map_zoomed_off_kamchatka() {
    let found = run(earthquake_zoomed_job());
    assert_table("earthquakes_zoomed", &found);
    assert_anchors_inside(&found);
}

#[test]
fn the_penguin_bills_at_their_default_view() {
    let found = run(penguins_job());
    assert_table("penguins_bills", &found);
    assert_anchors_inside(&found);
}

#[test]
fn the_housing_point_map_at_its_default_view() {
    let found = run(housing_job());
    assert_table("housing_point_map", &found);
    assert_anchors_inside(&found);
}

#[test]
fn a_bandwidth_change_reddens_the_tables() {
    let mut job = earthquake_wide_job();
    job.bandwidth_scale = 0.5;
    let halved = region_table(&run(job));
    assert_ne!(
        halved,
        committed(&table_path("earthquakes_wide")),
        "halving the bandwidth left the earthquake table as committed, so the \
         table does not pin the bandwidth"
    );
}

#[test]
fn a_label_rule_change_reddens_the_tables() {
    let mut job = earthquake_wide_job();
    job.naming = NamingRules {
        text_min_share: 0.99,
        ..NamingRules::default()
    };
    let strict = region_table(&run(job));
    assert_ne!(
        strict,
        committed(&table_path("earthquakes_wide")),
        "a text term needing 99% of a region's rows left the earthquake table \
         as committed, so the table does not pin the naming rule"
    );
}

/// The bundle named by `BRIGHTFIELD_FINETYPE_BUNDLE`, as
/// `crates/brightfield-engine/tests/finetype_bundle.rs` reads it.
fn bundle() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("BRIGHTFIELD_FINETYPE_BUNDLE")?);
    assert!(
        dir.join(brightfield_engine::semantic::EXTENSION_FILE)
            .is_file(),
        "BRIGHTFIELD_FINETYPE_BUNDLE={} carries no {}",
        dir.display(),
        brightfield_engine::semantic::EXTENSION_FILE
    );
    Some(dir)
}

/// The columns of `spec`'s sources as FineType types them.
fn typed_profiles(spec: &str, bundle: &Path) -> Vec<brightfield_engine::ColumnProfile> {
    use brightfield_engine::semantic::TypeSourceSpec;
    use brightfield_engine::{Engine, LoadOptions, NetworkPolicy, ProfileOutcome};
    let parsed = parse_spec(spec, Format::Yaml).expect("parses");
    let analysis = brightfield_spec::analysis::analyse_spec(&parsed.spec).expect("analyses");
    let options = LoadOptions {
        network: NetworkPolicy::Disabled,
        extension_directory: None,
        type_source: Some(TypeSourceSpec::Bundle(bundle.to_path_buf())),
        ..LoadOptions::default()
    };
    let load = Engine::new()
        .load_spec_with(parsed.spec, analysis, None, &options)
        .expect("the spec loads with the bundle");
    assert_eq!(
        load.session.type_source_error(),
        None,
        "the bundle failed to come up"
    );
    load.session
        .profile_sources()
        .into_iter()
        .flat_map(|p| match p.outcome {
            ProfileOutcome::Profiled { columns, .. } => columns,
            other => panic!("{} was not profiled: {other:?}", p.name),
        })
        .collect()
}

/// The names a table's lines give, in region order.
fn names_in(table: &str) -> Vec<String> {
    table
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.split(" | ").nth(5).expect("a name field").to_string())
        .collect()
}

#[test]
#[ignore = "needs a FineType bundle: set BRIGHTFIELD_FINETYPE_BUNDLE"]
fn region_names_match_with_finetype() {
    let Some(dir) = bundle() else {
        panic!("BRIGHTFIELD_FINETYPE_BUNDLE is not set");
    };
    let housing = data_file::source_spec(&own_copy("california_housing_sample.csv"));
    let fixtures: [(&str, String, RegionJob); 4] = [
        (
            "earthquakes_wide",
            earthquake_panel(),
            earthquake_wide_job(),
        ),
        (
            "earthquakes_zoomed",
            earthquake_panel_without_hexbin(),
            earthquake_zoomed_job(),
        ),
        ("penguins_bills", penguins_scatter(), penguins_job()),
        ("housing_point_map", housing, housing_job()),
    ];
    for (name, spec, mut job) in fixtures {
        job.profiles = Some(typed_profiles(&spec, &dir));
        let typed = region_table(&run(job));
        assert_eq!(
            names_in(&typed),
            names_in(&committed(&table_path(name))),
            "{name}'s regions are named differently once FineType types its columns"
        );
    }
}
