//! The dense regions of a two-column plot and their names, computed on a
//! worker thread and drawn over the plot once they arrive.
//!
//! A plot qualifies when one of its layers is a `dot` or `hexbin` over two plain
//! columns that it draws on linear axes ([`qualifying_layer`], then
//! [`RegionJob::for_plot`]). For each such plot a [`RegionJob`] runs, off the
//! UI thread:
//!
//! 1. the grid query counts the layer's rows in a [`GRID_BINS`]-square grid over
//!    the plot's displayed extent ([`region_grid_sql`]);
//! 2. the counts are smoothed by a Gaussian KDE whose bandwidth is Silverman's,
//!    per axis, read from the counts as weights ([`kde_2d`],
//!    [`silverman_2d_per_axis_weighted`]);
//! 3. the smoothed grid is cut into regions ([`find_regions`]), and regions
//!    under [`MIN_DRAWN_SHARE`] of the view's density are dropped;
//! 4. each region's outline and the spot for its name are traced
//!    ([`region_outline`], [`label_anchor`]);
//! 5. the label query summarises each candidate column per region
//!    ([`region_label_sql`]) and the summary becomes the names
//!    ([`name_regions_shown`]).
//!
//! The [`RegionsWorker`] keeps one run per plot, stops a run a newer request
//! replaces, and hands the answers over on a later frame. [`ink_regions`] draws
//! an answer while its extent is the one the plot is drawn at, and not after a
//! zoom moves the plot, so the old regions are hidden until the new ones
//! arrive (`old_regions_are_hidden_while_a_zoom_is_answered` in
//! `tests/regions_overlay.rs`).
//!
//! Regions are a render-time overlay rather than a mark injected into the spec,
//! because mark indices identify layers throughout the pipeline and a new mark
//! would shift them.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

use brightfield_engine::{ColumnProfile, RecordBatch, RegionReader, SemanticType, Session};
use brightfield_render::channel::Channel;
use brightfield_render::kde::{kde_2d, silverman_2d_per_axis_weighted};
use brightfield_render::mark::Projection;
use brightfield_render::regions::{
    find_regions, label_anchor, region_at, region_outline, DensityGrid, Region,
};
use brightfield_render::scale::Scale;
use brightfield_render::text::{draw_text, measure_width, TextAnchor, LABEL_SIZE};
use brightfield_spec::ast::{Mark, PlotNode, SpecValue, ValueOrParamRef};
use brightfield_spec::layout::collect_plot_nodes;
use brightfield_spec::parse::REGIONS_KEY;
use brightfield_spec::vocab::MarkKind;
use brightfield_spec::Spec;
use brightfield_sql::emit::{collect_marks, collect_plot_groups};
use brightfield_sql::ir::Predicate;
use brightfield_sql::regions::{region_grid_sql, region_label_sql, GridExtent};
use duckdb::arrow::array::{Array, Float64Array, Int32Array, Int64Array, StringArray};
use kurbo::{Affine, BezPath, Rect, Stroke};
use vello::peniko::Fill;
use vello::Scene;

use crate::pipeline::{Composed, PlotHandle};
use crate::region_names::{
    label_columns, name_regions_shown, LabelColumn, NamingRules, ShownColumn,
};
use crate::representation::Representation;

/// Cells along each side of the grid a plot's regions are found on, over the
/// plot's displayed extent whatever its zoom.
pub const GRID_BINS: u32 = 100;

/// The least share of the view's density mass a region must hold to be drawn
/// or named.
///
/// The density estimate spreads every row a little, so the faint, wide tails
/// between clusters can come out as regions of their own that hold almost
/// nothing: an outline around empty-looking space, and a name for it. That is
/// the clutter no name is better than. A hundredth of the view keeps every
/// cluster a reader can see at this zoom; one smaller than that is found by
/// zooming in, where it holds more of the view.
pub const MIN_DRAWN_SHARE: f64 = 0.01;

/// The width of a region's outline, in pixels: a hairline, so the points
/// under it stay the picture.
const OUTLINE_WIDTH: f64 = 1.0;

/// The width of the outline of the region a click has selected: heavy enough
/// to tell from the hairlines beside it at a glance, still a line rather than
/// a fill, so the selected points stay visible inside it.
const HELD_OUTLINE_WIDTH: f64 = 2.5;

/// The space kept clear around a name, in pixels, when names are placed apart.
const NAME_PAD: f64 = 2.0;

/// How far the knockout ink reaches around each glyph, in pixels.
const KNOCKOUT: f64 = 1.0;

/// A column the shell already knows how to show: its profile, from the open
/// that profiled the table, and how its axis prints its values.
#[derive(Clone, Debug, PartialEq)]
pub struct KnownColumn {
    pub profile: ColumnProfile,
    pub representation: Representation,
}

/// The mark index and the x and y columns of the layer whose rows `plot_path`'s
/// regions describe: the first `dot` or `hexbin` layer, in mark order, whose
/// `x` and `y` each name a column (not a `bin:` channel, an aggregate or an
/// expression).
///
/// `None` when no layer is like that, when the plot asks for `regions: off`,
/// or when it names a scale other than `linear` for either axis: the grid is
/// equal-width in data units, and a linear axis draws equal widths in data as
/// equal widths on screen.
/// Whether the two columns are numbers is the data's to say, so
/// [`RegionJob::for_plot`] also checks that the plot drew both axes on
/// linear scales.
#[must_use]
pub fn qualifying_layer(spec: &Spec, plot_path: &str) -> Option<(usize, String, String)> {
    if regions_switched_off(spec, plot_path) {
        return None;
    }
    drawable_layer(spec, plot_path)
}

/// Whether the plot at `plot_path` asks for `regions: off`.
#[must_use]
pub fn regions_switched_off(spec: &Spec, plot_path: &str) -> bool {
    collect_plot_nodes(spec)
        .into_iter()
        .find(|(path, _)| path == plot_path)
        .is_some_and(|(_, plot)| regions_off(plot))
}

/// Whether `plot` offers the switch that turns its regions off and on: its
/// regions could be drawn at the extent it is drawn at, were they on.
///
/// [`qualifying_layer`] without the `regions: off` it reads, so a plot
/// switched off keeps the switch that turns it back on, and the drawn-scale
/// check [`RegionJob::for_plot`] makes, so a plot over a text column, or one
/// drawn through a projection that bends its axes, offers no switch
/// (`the_switch_shows_only_on_qualifying_plots` holds the log-axis case).
#[must_use]
pub fn offers_regions_switch(spec: &Spec, plot: &PlotHandle) -> bool {
    drawable_layer(spec, &plot.path).is_some() && displayed_extent(plot).is_some()
}

/// [`qualifying_layer`], whatever the plot's `regions` key says.
fn drawable_layer(spec: &Spec, plot_path: &str) -> Option<(usize, String, String)> {
    let (_, plot) = collect_plot_nodes(spec)
        .into_iter()
        .find(|(path, _)| path == plot_path)?;
    if ["xScale", "yScale"].iter().any(|k| not_linear(plot, k)) {
        return None;
    }
    let marks = collect_marks(spec);
    let group = collect_plot_groups(spec)
        .into_iter()
        .find(|g| g.plot_path == plot_path)?;
    group.mark_indices.into_iter().find_map(|i| {
        let mark = marks.get(i)?;
        if !matches!(mark.kind, MarkKind::Dot | MarkKind::Hexbin) {
            return None;
        }
        Some((i, plain_field(mark, "x")?, plain_field(mark, "y")?))
    })
}

fn regions_off(plot: &PlotNode) -> bool {
    plot.attributes
        .get(REGIONS_KEY)
        .is_some_and(brightfield_spec::parse::is_regions_off)
}

fn not_linear(plot: &PlotNode, key: &str) -> bool {
    matches!(plot.attributes.get(key), Some(SpecValue::String(v)) if v != "linear")
}

/// The column `channel` names on `mark`, when it names one plainly.
fn plain_field(mark: &Mark, channel: &str) -> Option<String> {
    match mark.options.get(channel)? {
        ValueOrParamRef::Value(SpecValue::String(column)) => Some(column.clone()),
        _ => None,
    }
}

/// The column `channel` reads on `mark`, plainly or through `{ bin: column }`.
fn axis_column(mark: &Mark, channel: &str) -> Option<String> {
    match mark.options.get(channel)? {
        ValueOrParamRef::Value(SpecValue::String(column)) => Some(column.clone()),
        ValueOrParamRef::Value(SpecValue::Object(map)) => match map.get("bin")? {
            SpecValue::String(column) => Some(column.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// The title each column's axis is given somewhere in the spec: the `xLabel` or
/// `yLabel` of a plot whose first layer reading a column on that axis reads
/// this one. The first plot to title a column wins.
fn axis_titles(spec: &Spec) -> BTreeMap<String, String> {
    let marks = collect_marks(spec);
    let groups = collect_plot_groups(spec);
    let mut titles = BTreeMap::new();
    for (path, plot) in collect_plot_nodes(spec) {
        let Some(group) = groups.iter().find(|g| g.plot_path == path) else {
            continue;
        };
        for (channel, key) in [("x", "xLabel"), ("y", "yLabel")] {
            let Some(SpecValue::String(title)) = plot.attributes.get(key) else {
                continue;
            };
            let column = group
                .mark_indices
                .iter()
                .find_map(|&i| axis_column(marks.get(i)?, channel));
            if let Some(column) = column {
                titles.entry(column).or_insert_with(|| title.clone());
            }
        }
    }
    titles
}

/// The grid over the extent `plot` is drawn at, when it is drawn on two linear
/// axes in data units: no projection, or one that leaves longitude and
/// latitude as they are.
fn displayed_extent(plot: &PlotHandle) -> Option<GridExtent> {
    if !matches!(
        plot.scales.projection(),
        None | Some(Projection::Equirectangular | Projection::Identity)
    ) {
        return None;
    }
    let axis = |channel| match plot.scales.get(channel)? {
        Scale::Linear {
            domain_min,
            domain_max,
            ..
        } => Some((domain_min.min(*domain_max), domain_max.max(*domain_min))),
        _ => None,
    };
    Some(GridExtent {
        x: axis(Channel::X)?,
        y: axis(Channel::Y)?,
        bins: GRID_BINS,
    })
}

/// What a run answers: the layer, its columns, its rows and the extent. A
/// request for what is already answered, or being answered, starts nothing.
///
/// The rows are the layer's rows SQL, which carries the spec's parameters as
/// they stand and no selection: a slider that filters the layer changes the
/// rows, and so asks again, even when the drawn extent stays where it was,
/// while a brush on another plot changes nothing here.
#[derive(Clone, Debug, PartialEq)]
struct JobKey {
    mark: usize,
    x: String,
    y: String,
    rows_sql: String,
    extent: GridExtent,
}

/// The key for `plot` as it is drawn now. Builds SQL text and runs nothing.
fn key_for(session: &Session, spec: &Spec, plot: &PlotHandle) -> Option<JobKey> {
    let (mark, x, y) = qualifying_layer(spec, &plot.path)?;
    let extent = displayed_extent(plot)?;
    Some(JobKey {
        mark,
        x,
        y,
        rows_sql: session.region_rows_sql(mark).ok()?,
        extent,
    })
}

/// One plot's regions to find: the rows, the extent and the columns, and a
/// reader of its own to run the queries on.
pub struct RegionJob {
    pub plot_path: String,
    pub mark: usize,
    pub extent: GridExtent,
    /// The layer's rows, with the spec's parameters and no selection, so a
    /// brush on another plot does not move the regions.
    pub rows_sql: String,
    pub x: String,
    pub y: String,
    /// The table's profiles, when the shell has them from the open. `None` has
    /// the run profile the rows itself, by storage type and distinct count.
    pub profiles: Option<Vec<ColumnProfile>>,
    /// How each column's axis prints its values, which a range name is
    /// written in. A column not here is printed plainly.
    pub representations: BTreeMap<String, Representation>,
    /// The title each column's axis is given, which a range name is written
    /// after. A column not here is written by its name.
    pub titles: BTreeMap<String, String>,
    /// What Silverman's bandwidth is multiplied by: 1.0, except where a test
    /// changes it to show the regions follow the bandwidth.
    pub bandwidth_scale: f64,
    /// The thresholds a region's name must clear: the defaults, except where a
    /// test changes one to show the names follow the rule.
    pub naming: NamingRules,
    reader: RegionReader,
}

impl RegionJob {
    /// The job for `plot` at the extent it is drawn at, or `None` when it does
    /// not qualify or its rows cannot be read off the UI thread.
    #[must_use]
    pub fn for_plot(
        session: &Session,
        spec: &Spec,
        plot: &PlotHandle,
        known: &[KnownColumn],
    ) -> Option<Self> {
        let key = key_for(session, spec, plot)?;
        let reader = session.region_reader().ok()?;
        Some(Self {
            plot_path: plot.path.clone(),
            mark: key.mark,
            extent: key.extent,
            rows_sql: key.rows_sql,
            x: key.x,
            y: key.y,
            profiles: (!known.is_empty())
                .then(|| known.iter().map(|k| k.profile.clone()).collect()),
            representations: known
                .iter()
                .map(|k| (k.profile.name.clone(), k.representation.clone()))
                .collect(),
            titles: axis_titles(spec),
            bandwidth_scale: 1.0,
            naming: NamingRules::default(),
            reader,
        })
    }

    fn key(&self) -> JobKey {
        JobKey {
            mark: self.mark,
            x: self.x.clone(),
            y: self.y.clone(),
            rows_sql: self.rows_sql.clone(),
            extent: self.extent.clone(),
        }
    }

    /// Find the regions here and now, on this thread.
    ///
    /// `None` when a query fails.
    #[must_use]
    pub fn run(self) -> Option<PlotRegions> {
        self.run_unless(&AtomicBool::new(false))
    }

    /// [`Self::run`], giving up between steps once `cancelled` is set. The
    /// reader's interrupt stops a query under way, but one sent before a query
    /// starts is lost, so the flag is read before each.
    fn run_unless(self, cancelled: &AtomicBool) -> Option<PlotRegions> {
        let stopped = || cancelled.load(Ordering::Relaxed);
        let read = |sql: &str| -> Option<Vec<RecordBatch>> {
            if stopped() {
                return None;
            }
            match self.reader.query(sql) {
                Ok(batches) => Some(batches),
                Err(e) => {
                    if !stopped() {
                        eprintln!("warning: a region query failed: {e}");
                    }
                    None
                }
            }
        };
        let Some(grid_sql) = region_grid_sql(&self.rows_sql, &self.x, &self.y, &self.extent) else {
            return Some(PlotRegions::none(self.x, self.y, self.extent));
        };
        let grid = density_grid(&read(&grid_sql)?, &self.extent, self.bandwidth_scale);
        let mut regions = find_regions(&grid);
        regions.retain(|r| r.share >= MIN_DRAWN_SHARE);
        let outlines = regions.iter().map(|r| region_outline(&grid, r)).collect();
        let anchors = regions.iter().map(|r| label_anchor(&grid, r)).collect();
        let names = if regions.is_empty() {
            Vec::new()
        } else {
            let profiles = match &self.profiles {
                Some(profiles) => profiles.clone(),
                None => profile_rows(&read, &self.rows_sql)?,
            };
            let columns = label_columns(&profiles, &self.x, &self.y);
            let cells = cell_regions(&regions);
            match region_label_sql(
                &self.rows_sql,
                &self.x,
                &self.y,
                &self.extent,
                &cells,
                &columns,
            ) {
                Some(sql) => {
                    let summary = read(&sql)?;
                    let shown = self.shown(&columns);
                    name_regions_shown(&self.naming, &shown, &summary, regions.len())
                }
                None => vec![None; regions.len()],
            }
        };
        Some(PlotRegions {
            x: self.x,
            y: self.y,
            extent: self.extent,
            grid,
            regions,
            outlines,
            names,
            anchors,
        })
    }

    /// `columns` as a name writes them. A range is written after the shown
    /// column's name, so each carries its axis title in place of its name
    /// where it has one.
    fn shown(&self, columns: &[LabelColumn]) -> Vec<ShownColumn> {
        columns
            .iter()
            .map(|c| ShownColumn {
                column: LabelColumn {
                    name: self.titles.get(&c.name).unwrap_or(&c.name).clone(),
                    role: c.role,
                },
                representation: self
                    .representations
                    .get(&c.name)
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect()
    }
}

/// `(cell, region)` for every cell of every region, regions numbered in order.
fn cell_regions(regions: &[Region]) -> Vec<(u32, u32)> {
    regions
        .iter()
        .enumerate()
        .flat_map(|(i, r)| {
            let region = u32::try_from(i).unwrap_or(u32::MAX);
            r.cells
                .iter()
                .filter_map(move |&c| Some((u32::try_from(c).ok()?, region)))
        })
        .collect()
}

/// The smoothed density over `extent` from the grid query's counts.
///
/// The bandwidth on each axis is Silverman's, from the occupied cells' centres
/// weighted by their counts, times `scale`. An axis with no spread of its own
/// (every row in one column of cells) has no Silverman bandwidth, and is
/// smoothed over one cell instead.
fn density_grid(batches: &[RecordBatch], extent: &GridExtent, scale: f64) -> DensityGrid {
    let bins = extent.bins as usize;
    let step = |(lo, hi): (f64, f64)| (hi - lo) / f64::from(extent.bins);
    let (dx, dy) = (step(extent.x), step(extent.y));
    let mut counts = vec![0_u32; bins * bins];
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    for batch in batches {
        let (Some(bx), Some(by), Some(n)) = (
            batch.column(0).as_any().downcast_ref::<Int32Array>(),
            batch.column(1).as_any().downcast_ref::<Int32Array>(),
            batch.column(2).as_any().downcast_ref::<Float64Array>(),
        ) else {
            continue;
        };
        for i in 0..batch.num_rows() {
            let (Ok(col), Ok(row)) = (usize::try_from(bx.value(i)), usize::try_from(by.value(i)))
            else {
                continue;
            };
            if col >= bins || row >= bins {
                continue;
            }
            let count = n.value(i);
            // A row count, so a whole number well inside u32.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                counts[row * bins + col] = count.round().max(0.0) as u32;
            }
            #[allow(clippy::cast_precision_loss)]
            {
                xs.push((extent.x.0 + (col as f64 + 0.5) * dx, count));
                ys.push((extent.y.0 + (row as f64 + 0.5) * dy, count));
            }
        }
    }
    let density = if xs.is_empty() {
        vec![0.0; bins * bins]
    } else {
        let (hx, hy) = silverman_2d_per_axis_weighted(&xs, &ys);
        let or_cell = |h: f64, cell: f64| if h.is_finite() && h > 0.0 { h } else { cell };
        kde_2d(
            &counts,
            (bins, bins),
            (or_cell(hx, dx) * scale, or_cell(hy, dy) * scale),
            (dx, dy),
        )
    };
    DensityGrid {
        cols: bins,
        rows: bins,
        x: extent.x,
        y: extent.y,
        density,
    }
}

/// The rows' columns profiled by storage type and distinct count, the way the
/// open profiles a table with no FineType: one `DESCRIBE` and one aggregate.
fn profile_rows(
    read: &dyn Fn(&str) -> Option<Vec<RecordBatch>>,
    rows_sql: &str,
) -> Option<Vec<ColumnProfile>> {
    let described = read(&format!("DESCRIBE SELECT * FROM ({rows_sql}) AS __bf_rows"))?;
    let mut columns: Vec<(String, String)> = Vec::new();
    for batch in &described {
        let text = |name: &str| {
            batch
                .column_by_name(name)?
                .as_any()
                .downcast_ref::<StringArray>()
                .cloned()
        };
        let (Some(names), Some(types)) = (text("column_name"), text("column_type")) else {
            continue;
        };
        for i in 0..batch.num_rows() {
            columns.push((names.value(i).to_owned(), types.value(i).to_owned()));
        }
    }
    columns.retain(|(name, _)| !name.starts_with("__bf"));
    if columns.is_empty() {
        return Some(Vec::new());
    }
    let asked: Vec<String> = columns
        .iter()
        .map(|(name, _)| {
            let q = crate::sql_ident::quote(name);
            format!("count({q}), approx_count_distinct({q})")
        })
        .collect();
    let batches = read(&format!(
        "SELECT count(*), {} FROM ({rows_sql}) AS __bf_rows",
        asked.join(", ")
    ))?;
    let batch = batches.iter().find(|b| b.num_rows() > 0)?;
    let count = |i: usize| -> Option<u64> {
        let column = batch.column(i).as_any().downcast_ref::<Int64Array>()?;
        u64::try_from(column.value(0)).ok()
    };
    let rows = count(0)?;
    columns
        .into_iter()
        .enumerate()
        .map(|(i, (name, type_name))| {
            let non_null = count(1 + 2 * i)?;
            Some(ColumnProfile {
                name,
                type_name,
                non_null,
                nulls: rows.saturating_sub(non_null),
                distinct: count(2 + 2 * i)?,
                min: None,
                max: None,
                semantic: SemanticType::NotAsked,
                moments: None,
            })
        })
        .collect()
}

/// One plot's regions over one extent.
#[derive(Clone, Debug, PartialEq)]
pub struct PlotRegions {
    /// The columns the grid is laid over, x then y: the layer's own.
    pub x: String,
    pub y: String,
    pub extent: GridExtent,
    pub grid: DensityGrid,
    /// Highest peak first, each holding at least [`MIN_DRAWN_SHARE`].
    pub regions: Vec<Region>,
    /// Each region's boundary as closed rings in data units, in region order.
    pub outlines: Vec<Vec<Vec<(f64, f64)>>>,
    /// Each region's name, or `None` when no column names it.
    pub names: Vec<Option<String>>,
    /// Where each region's name is written, in data units.
    pub anchors: Vec<(f64, f64)>,
}

impl PlotRegions {
    /// No regions over `extent`, which has no grid.
    fn none(x: String, y: String, extent: GridExtent) -> Self {
        Self {
            x,
            y,
            grid: DensityGrid {
                cols: 0,
                rows: 0,
                x: extent.x,
                y: extent.y,
                density: Vec::new(),
            },
            extent,
            regions: Vec::new(),
            outlines: Vec::new(),
            names: Vec::new(),
            anchors: Vec::new(),
        }
    }
}

/// A wake the worker calls from its thread when an answer is ready.
type Wake = Arc<dyn Fn() + Send + Sync>;

/// A run under way and how to stop it.
struct Flight {
    cancelled: Arc<AtomicBool>,
    interrupt: Arc<duckdb::InterruptHandle>,
    answer: Receiver<Option<PlotRegions>>,
}

impl Flight {
    fn stop(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.interrupt.interrupt();
    }
}

/// The region runs of one dashboard: at most one under way per plot, and the
/// last answer for each.
#[derive(Default)]
pub struct RegionsWorker {
    in_flight: BTreeMap<String, Flight>,
    answered: BTreeMap<String, PlotRegions>,
    /// What each plot was last asked for, answered or not.
    asked: BTreeMap<String, JobKey>,
    wake: Option<Wake>,
    started: usize,
    superseded: usize,
}

impl RegionsWorker {
    /// Have each finished run call `wake` from its thread, so the window draws
    /// the frame that takes the answer.
    pub fn set_wake(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.wake = Some(Arc::new(wake));
    }

    /// Whether a wake is set.
    #[must_use]
    pub fn has_wake(&self) -> bool {
        self.wake.is_some()
    }

    /// Start `job` on its own thread. A run under way for the same plot is
    /// stopped and counted as superseded, and its answer never lands.
    pub fn request(&mut self, job: RegionJob) {
        let path = job.plot_path.clone();
        if let Some(flight) = self.in_flight.remove(&path) {
            flight.stop();
            self.superseded += 1;
        }
        self.asked.insert(path.clone(), job.key());
        let cancelled = Arc::new(AtomicBool::new(false));
        let interrupt = job.reader.interrupt_handle();
        let (send, answer) = channel();
        let (stopped, wake) = (Arc::clone(&cancelled), self.wake.clone());
        std::thread::spawn(move || {
            let answer = job.run_unless(&stopped);
            if !stopped.load(Ordering::Relaxed) {
                let _ = send.send(answer);
                if let Some(wake) = wake {
                    wake();
                }
            }
        });
        self.in_flight.insert(
            path,
            Flight {
                cancelled,
                interrupt,
                answer,
            },
        );
        self.started += 1;
    }

    /// Request a run for each plot of `plots` that qualifies and whose layer
    /// or extent is not the one last asked for, and forget the regions of each
    /// plot that no longer qualifies.
    pub(crate) fn request_for(
        &mut self,
        session: &Session,
        spec: &Spec,
        plots: &[PlotHandle],
        known: &[KnownColumn],
    ) {
        for plot in plots {
            let Some(key) = key_for(session, spec, plot) else {
                self.forget(&plot.path);
                continue;
            };
            if self.asked.get(&plot.path) == Some(&key) {
                continue;
            }
            match RegionJob::for_plot(session, spec, plot, known) {
                Some(job) => self.request(job),
                None => {
                    // No reader opened. Remember the key all the same, so the
                    // next frame does not try again until the plot changes.
                    self.forget(&plot.path);
                    self.asked.insert(plot.path.clone(), key);
                }
            }
        }
    }

    /// Take each finished run's answer. True when an answer landed.
    ///
    /// A run's answer is to the latest request for its plot, since a newer
    /// request stops the run before it. A run that failed leaves no answer.
    pub fn poll(&mut self) -> bool {
        let finished: Vec<(String, Option<PlotRegions>)> = self
            .in_flight
            .iter()
            .filter_map(|(path, flight)| match flight.answer.try_recv() {
                Ok(answer) => Some((path.clone(), answer)),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some((path.clone(), None)),
            })
            .collect();
        let mut landed = false;
        for (path, answer) in finished {
            self.in_flight.remove(&path);
            if let Some(answer) = answer {
                self.answered.insert(path, answer);
                landed = true;
            }
        }
        landed
    }

    /// The last answer for the plot at `plot_path`, whatever extent the plot is
    /// drawn at now.
    #[must_use]
    pub fn regions(&self, plot_path: &str) -> Option<&PlotRegions> {
        self.answered.get(plot_path)
    }

    /// The answer drawn on `plot`: the last one, while its extent is the one
    /// the plot is drawn at, and none once a zoom or pan has moved the plot
    /// and the new answer has not landed.
    #[must_use]
    pub fn drawn_on(&self, plot: &PlotHandle) -> Option<&PlotRegions> {
        let answer = self.answered.get(&plot.path)?;
        (displayed_extent(plot).as_ref() == Some(&answer.extent)).then_some(answer)
    }

    /// Whether a run is under way.
    #[must_use]
    pub fn pending(&self) -> bool {
        !self.in_flight.is_empty()
    }

    /// How many runs have started.
    #[must_use]
    pub const fn started(&self) -> usize {
        self.started
    }

    /// How many runs a newer request for their plot stopped.
    #[must_use]
    pub const fn superseded(&self) -> usize {
        self.superseded
    }

    /// Stop the plot's run and drop its regions.
    pub fn forget(&mut self, plot_path: &str) {
        if let Some(flight) = self.in_flight.remove(plot_path) {
            flight.stop();
        }
        self.answered.remove(plot_path);
        self.asked.remove(plot_path);
    }

    /// Signal every run under way to stop, without waiting for them to end:
    /// each gives up at its next step, and a query under way is interrupted.
    /// Their plots are asked again by the next [`Self::request_for`].
    pub fn stop_all(&mut self) {
        for (path, flight) in std::mem::take(&mut self.in_flight) {
            flight.stop();
            self.asked.remove(&path);
        }
    }
}

impl Drop for RegionsWorker {
    fn drop(&mut self) {
        self.stop_all();
    }
}

/// The selection a click at `local` (pixels inside `plot`'s own rect) makes:
/// the cells of the region drawn under it, over the extent that region was
/// found on, or `None` where no region is drawn.
///
/// The answer read is the one drawn now ([`RegionsWorker::drawn_on`]), so a click
/// made while a zoom's regions are still being found is a plain click, not a
/// choice of cells from a view no longer shown. The point is inverted through the
/// displayed scales without unprojecting, because the grid is laid over those
/// scales' domains ([`displayed_extent`]).
#[must_use]
pub fn region_clause_at(
    worker: &RegionsWorker,
    plot: &PlotHandle,
    local: (f64, f64),
) -> Option<Predicate> {
    let answer = worker.drawn_on(plot)?;
    let x = plot.scales.get(Channel::X)?.inverse_f64(local.0)?;
    let y = plot.scales.get(Channel::Y)?.inverse_f64(local.1)?;
    let region = &answer.regions[region_at(&answer.grid, &answer.regions, x, y)?];
    let cells = region
        .cells
        .iter()
        .map(|&c| u32::try_from(c).ok())
        .collect::<Option<Vec<u32>>>()?;
    Predicate::cells(&answer.x, &answer.y, answer.extent.clone(), cells)
}

/// The region of `regions` that `held` selects: the one whose cells it holds,
/// over the same columns and extent. `None` for any other clause, and for
/// cells chosen over another extent, whose region is not among these.
#[must_use]
pub fn region_held(regions: &PlotRegions, held: &Predicate) -> Option<usize> {
    let Predicate::Cells {
        x,
        y,
        extent,
        cells,
    } = held
    else {
        return None;
    };
    if *x != regions.x || *y != regions.y || *extent != regions.extent {
        return None;
    }
    regions.regions.iter().position(|r| {
        r.cells.len() == cells.len()
            && r.cells
                .iter()
                .zip(cells)
                .all(|(&a, &b)| u32::try_from(a).is_ok_and(|a| a == b))
    })
}

/// The region drawn on `plot` that the plot's own gesture holds in a
/// selection, read as `pipeline::plot_selection` reads the plot's own clause:
/// keyed on the contributor, which is the plot's path.
fn held_on(session: &Session, regions: &PlotRegions, plot: &PlotHandle) -> Option<usize> {
    session
        .current_selections()
        .values()
        .flatten()
        .filter(|(contributor, _)| contributor.0 == plot.path)
        .find_map(|(_, predicate)| region_held(regions, predicate))
}

/// [`held_on`] for the answer drawn on `plot` now.
#[must_use]
pub fn held_region(worker: &RegionsWorker, session: &Session, plot: &PlotHandle) -> Option<usize> {
    held_on(session, worker.drawn_on(plot)?, plot)
}

/// A name placed on a plot: its region, its text, the box kept clear around it
/// and the point its baseline is centred on, in the plot's own pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedName {
    pub region: usize,
    pub text: String,
    pub rect: Rect,
    pub baseline: (f64, f64),
}

/// The names of `regions` that fit on `plot`, placed in order of region share.
///
/// Each name is centred on its region's anchor. A name whose box overlaps one
/// already placed, or does not fit inside the plot area, is left out; its
/// region keeps its outline.
#[must_use]
pub fn place_names(plot: &PlotHandle, regions: &PlotRegions) -> Vec<PlacedName> {
    let (Some(sx), Some(sy)) = (plot.scales.get(Channel::X), plot.scales.get(Channel::Y)) else {
        return Vec::new();
    };
    let layout = &plot.layout;
    let area = Rect::new(
        layout.plot_x_start(),
        layout.plot_y_start(),
        layout.plot_x_end(),
        layout.plot_y_end(),
    );
    let mut order: Vec<usize> = (0..regions.regions.len()).collect();
    order.sort_by(|&a, &b| {
        regions.regions[b]
            .share
            .total_cmp(&regions.regions[a].share)
            .then(a.cmp(&b))
    });
    let size = f64::from(LABEL_SIZE);
    let mut placed: Vec<PlacedName> = Vec::new();
    for region in order {
        let (Some(Some(text)), Some(&(ax, ay))) =
            (regions.names.get(region), regions.anchors.get(region))
        else {
            continue;
        };
        let (cx, cy) = (sx.map_f64(ax), sy.map_f64(ay));
        let half = measure_width(text, LABEL_SIZE) / 2.0 + NAME_PAD;
        let rect = Rect::new(
            cx - half,
            cy - size / 2.0 - NAME_PAD,
            cx + half,
            cy + size / 2.0 + NAME_PAD,
        );
        let inside =
            rect.x0 >= area.x0 && rect.y0 >= area.y0 && rect.x1 <= area.x1 && rect.y1 <= area.y1;
        let crowded = placed.iter().any(|p| {
            p.rect.x0 < rect.x1 && rect.x0 < p.rect.x1 && p.rect.y0 < rect.y1 && rect.y0 < p.rect.y1
        });
        if inside && !crowded {
            placed.push(PlacedName {
                region,
                text: text.clone(),
                rect,
                // The baseline sits below the centre by about half the
                // x-height, so the letters, not the line, are centred.
                baseline: (cx, cy + size * 0.35),
            });
        }
    }
    placed
}

/// Draw each plot's regions over the composed page: the outlines, then the
/// names with knockout ink so they read over dense points, inside the plot
/// area. A plot whose answer is for another extent, or that has none yet, is
/// drawn without regions. The region the plot's own selection holds is
/// outlined heavier, and only while the extent it was selected on is the one
/// drawn: after a zoom the selection keeps its cells, and the regions drawn
/// over the new view are new regions, none of them held.
pub(crate) fn ink_regions(composed: &mut Composed, worker: &RegionsWorker, session: &Session) {
    for plot in &composed.plots {
        let Some(regions) = worker.drawn_on(plot) else {
            continue;
        };
        if regions.regions.is_empty() {
            continue;
        }
        let held = held_on(session, regions, plot);
        let mut overlay = Scene::new();
        draw_regions(&mut overlay, plot, regions, held);
        composed.scene.append(
            &overlay,
            Some(Affine::translate((plot.rect.x, plot.rect.y))),
        );
    }
}

fn draw_regions(scene: &mut Scene, plot: &PlotHandle, regions: &PlotRegions, held: Option<usize>) {
    let (Some(sx), Some(sy)) = (plot.scales.get(Channel::X), plot.scales.get(Channel::Y)) else {
        return;
    };
    let layout = &plot.layout;
    let clip = Rect::new(
        layout.plot_x_start(),
        layout.plot_y_start(),
        layout.plot_x_end(),
        layout.plot_y_end(),
    );
    // From the plot's own ink tokens, as the committed-selection band takes
    // its inks: the outline and the names in the primary ink titles use, the
    // knockout in the chart surface.
    let ink = plot.scales.ink();
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &clip);
    let (stroke, heavy) = (Stroke::new(OUTLINE_WIDTH), Stroke::new(HELD_OUTLINE_WIDTH));
    for (region, rings) in regions.outlines.iter().enumerate() {
        let mut path = BezPath::new();
        for ring in rings {
            let mut corners = ring.iter().map(|&(x, y)| (sx.map_f64(x), sy.map_f64(y)));
            let Some(first) = corners.next() else {
                continue;
            };
            path.move_to(first);
            for corner in corners {
                path.line_to(corner);
            }
            path.close_path();
        }
        let stroke = if held == Some(region) {
            &heavy
        } else {
            &stroke
        };
        scene.stroke(stroke, Affine::IDENTITY, ink.title, None, &path);
    }
    for name in place_names(plot, regions) {
        let (x, y) = name.baseline;
        for (dx, dy) in [
            (-1.0, -1.0),
            (0.0, -1.0),
            (1.0, -1.0),
            (-1.0, 0.0),
            (1.0, 0.0),
            (-1.0, 1.0),
            (0.0, 1.0),
            (1.0, 1.0),
        ] {
            draw_text(
                scene,
                &name.text,
                x + dx * KNOCKOUT,
                y + dy * KNOCKOUT,
                LABEL_SIZE,
                ink.background,
                TextAnchor::Middle,
            );
        }
        draw_text(
            scene,
            &name.text,
            x,
            y,
            LABEL_SIZE,
            ink.title,
            TextAnchor::Middle,
        );
    }
    scene.pop_layer();
}
