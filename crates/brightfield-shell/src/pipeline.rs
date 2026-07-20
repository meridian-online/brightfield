//! Spec → composited Vello scene (gpui-free).
//!
//! A focused port of the app's `build_everything` plot-composition path, using
//! only the framework-free crates (`brightfield-spec` / `-engine` / `-sql` /
//! `-render`). It parses a Mosaic spec, executes each mark's query on the
//! engine, builds one Vello scene per plot (its own axes/scales/legend, titles
//! and axis insets resolved via the same public helpers the app uses), and
//! composites them into a single dashboard scene the egui host presents.
//!
//! Scope for the loop-first phase: colour-scheme / projection / highlight /
//! explicit colorDomain and standalone-legend relocation are NOT ported (the
//! golden `dashboard.yaml` and the simple examples use none of them). Marks are
//! taken from their first result batch (examples fit one ~2048-row chunk); the
//! app's multi-chunk concat is the production path.

use std::path::Path;

use arrow::record_batch::RecordBatch;
use brightfield_engine::Engine;
use brightfield_render::channel::ChannelMap;
use brightfield_render::inset::{resolve_insets_for_marks, DEFAULT_SCALE_INSET};
use brightfield_render::layout::{ChartLayout, Margins};
use brightfield_render::mark::{default_renderers, find_renderer, MarkRenderer};
use brightfield_render::scene::{build_multi_mark_scene, compose_dashboard, ChartData};
use brightfield_render::{grow_margins, resolve_titles};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::layout::{collect_plot_nodes, placed_plots, resolve_plot_insets, Rect};
use brightfield_spec::parse_spec_path;
use brightfield_sql::{collect_marks, collect_plot_groups};
use vello::Scene;

/// One composited dashboard ready to present: the merged Vello scene, its
/// logical bounding size, and the spec's declared title (for the window chrome).
pub struct Composed {
    /// The single composited Vello scene (all plots placed on the page plane).
    pub scene: Scene,
    /// Dashboard width in logical pixels.
    pub width: u32,
    /// Dashboard height in logical pixels.
    pub height: u32,
    /// The spec's `meta.title`, if declared.
    pub title: Option<String>,
}

/// Run a Mosaic spec at `spec_path` through parse → analyse → engine → execute →
/// per-plot scene → composite, returning the [`Composed`] dashboard.
///
/// # Errors
///
/// Returns a human-readable message if any pipeline stage fails or no mark
/// renders.
pub fn compose_spec(spec_path: &str) -> Result<Composed, String> {
    let parsed = parse_spec_path(spec_path).map_err(|e| format!("parse error: {e}"))?;
    let spec = parsed.spec;

    let analysis = analyse_spec(&spec).map_err(|e| format!("analysis error: {e}"))?;

    let engine = Engine::new();
    let spec_dir = Path::new(spec_path).parent();
    let load = engine
        .load_spec(spec.clone(), analysis, spec_dir)
        .map_err(|e| format!("engine error: {e}"))?;
    let mut session = load.session;

    // Execute every mark; keep its first result batch (examples fit one chunk).
    let results = session.execute_all();
    let marks = collect_marks(&spec);
    let mut batches: Vec<Option<RecordBatch>> = Vec::with_capacity(marks.len());
    let mut channel_maps: Vec<ChannelMap> = Vec::with_capacity(marks.len());
    let mut kinds = Vec::with_capacity(marks.len());
    for (i, result) in results.into_iter().enumerate() {
        let batch = match result {
            Ok(bs) => bs.into_iter().next(),
            Err(e) => {
                eprintln!("warning: skipping mark {i}: {e}");
                None
            }
        };
        batches.push(batch);
        channel_maps.push(ChannelMap::from_mark(marks[i]));
        kinds.push(marks[i].kind);
    }

    let placed = placed_plots(&spec, Rect::new(0.0, 0.0, 0.0, 0.0));
    let groups = collect_plot_groups(&spec);
    let plot_nodes = collect_plot_nodes(&spec);
    let registry = default_renderers();

    // Own each plot's scene; place them below.
    let mut placements: Vec<(f64, f64, Scene)> = Vec::new();
    for plot in &placed {
        let Some(group) = groups.iter().find(|g| g.plot_path == plot.path) else {
            continue;
        };

        let mut chart_data: Vec<ChartData<'_>> = Vec::new();
        for &mi in &group.mark_indices {
            let Some(batch) = batches.get(mi).and_then(|b| b.as_ref()) else {
                continue;
            };
            let renderer: &dyn MarkRenderer = match find_renderer(&registry, kinds[mi]) {
                Some(r) => r,
                None => {
                    eprintln!("warning: no renderer for mark {mi} — skipping");
                    continue;
                }
            };
            chart_data.push(ChartData {
                batch,
                channel_map: &channel_maps[mi],
                renderer,
                layout: ChartLayout::new(plot.rect.width, plot.rect.height),
                view_extent: None,
                highlight: None,
            });
        }
        if chart_data.is_empty() {
            continue;
        }

        // Axis + plot titles, then grow the margins to reserve their band.
        let title_maps: Vec<&ChannelMap> = chart_data.iter().map(|d| d.channel_map).collect();
        let titles = plot_nodes
            .iter()
            .find(|(p, _)| *p == plot.path)
            .map(|(_, node)| resolve_titles(node, &title_maps))
            .unwrap_or_default();
        drop(title_maps);

        // Axis insets so edge marks render whole inside the frame clip.
        let explicit_insets = plot_nodes
            .iter()
            .find(|(p, _)| *p == plot.path)
            .map(|(_, node)| resolve_plot_insets(node))
            .unwrap_or_default();
        let inset_entries: Vec<_> = chart_data
            .iter()
            .map(|d| (d.batch, d.channel_map, d.renderer))
            .collect();
        let insets = resolve_insets_for_marks(explicit_insets, &inset_entries, DEFAULT_SCALE_INSET);
        drop(inset_entries);

        let margins = grow_margins(Margins::default(), &titles);
        let layout = ChartLayout::with_margins_and_insets(
            plot.rect.width,
            plot.rect.height,
            margins,
            insets,
        );
        for d in &mut chart_data {
            d.layout = layout;
        }

        let refs: Vec<&ChartData<'_>> = chart_data.iter().collect();
        let (scene, _scales) = build_multi_mark_scene(&refs, true, &titles);
        drop(refs);
        drop(chart_data);
        placements.push((plot.rect.x, plot.rect.y, scene));
    }

    if placements.is_empty() {
        return Err("no marks rendered successfully".to_string());
    }

    let width = placed
        .iter()
        .map(|p| p.rect.x + p.rect.width)
        .fold(0.0_f64, f64::max)
        .ceil() as u32;
    let height = placed
        .iter()
        .map(|p| p.rect.y + p.rect.height)
        .fold(0.0_f64, f64::max)
        .ceil() as u32;

    let refs2: Vec<(f64, f64, &Scene)> = placements.iter().map(|(x, y, s)| (*x, *y, s)).collect();
    let scene = compose_dashboard(f64::from(width), f64::from(height), &refs2);

    let title = spec.meta.as_ref().and_then(|m| m.title.clone());
    Ok(Composed {
        scene,
        width,
        height,
        title,
    })
}
